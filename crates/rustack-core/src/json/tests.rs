use std::collections::BTreeMap;

use proptest::{
    arbitrary::any,
    collection::{btree_map, vec},
    prelude::{ProptestConfig, Strategy},
    prop_assert, prop_assert_eq, prop_oneof, proptest,
};
use rstest::rstest;
use serde::Deserialize;
use serde_json::{Value, json};

use super::*;

fn limits() -> JsonLimits {
    JsonLimits::new(16 * 1024, 16, 1024, 256, 32).unwrap()
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Model {
    count: u32,
}

#[test]
fn test_should_validate_before_typed_schema_and_redact_schema_errors() {
    assert_eq!(
        from_slice::<Model>(br#"{"count":7}"#, limits()).unwrap(),
        Model { count: 7 }
    );
    let error = from_slice::<Model>(br#"{"count":"secret-schema-value"}"#, limits()).unwrap_err();
    assert_eq!(error, JsonError::Schema);
    assert!(!format!("{error:?} {error}").contains("secret-schema-value"));
    assert_eq!(
        from_slice::<Model>(br#"{"count":7,"unknown":1,"unknown":2}"#, limits()),
        Err(JsonError::Duplicate)
    );
}

#[rstest]
#[case(br#"{"a":1,"a":2}"#, JsonError::Duplicate)]
#[case(br#"{"a":1,"\u0061":2}"#, JsonError::Duplicate)]
#[case(br#"{"x":{"a":1,"a":2}}"#, JsonError::Duplicate)]
#[case(b"{", JsonError::Syntax)]
#[case(b"[1,]", JsonError::Syntax)]
#[case(b"null true", JsonError::Syntax)]
#[case(b"", JsonError::Syntax)]
#[case(b"01", JsonError::Syntax)]
#[case(b"1e99999", JsonError::Syntax)]
#[case(br#""\ud800""#, JsonError::Syntax)]
#[case(br#""\x00""#, JsonError::Syntax)]
#[case(&[b'"', 0xff, b'"'], JsonError::Syntax)]
fn test_should_reject_duplicates_and_malformed_syntax(
    #[case] input: &[u8],
    #[case] expected: JsonError,
) {
    assert_eq!(validate(input, limits()), Err(expected));
}

#[test]
fn test_should_scope_duplicate_keys_to_their_own_object() {
    validate(br#"{"a":1,"child":{"a":2}}"#, limits()).unwrap();
    validate(br#"[{"a":1},{"a":2}]"#, limits()).unwrap();
    validate(br#"{"\u00e9":1,"e\u0301":2}"#, limits()).unwrap();
}

#[test]
fn test_should_bound_container_depth_without_rejecting_scalar_children() {
    let shallow = JsonLimits::new(1024, 1, 64, 256, 32).unwrap();
    for input in [
        b"{}".as_slice(),
        b"[]",
        b"[1,true,null]",
        br#"{"a":1}"#,
        b"false",
    ] {
        validate(input, shallow).unwrap();
    }
    for input in [br#"{"a":{}}"#.as_slice(), b"[[]]", br#"[{"a":1}]"#] {
        assert_eq!(validate(input, shallow), Err(JsonError::Depth));
    }
    let deeply_nested = format!("{}0{}", "[".repeat(4000), "]".repeat(4000));
    let bounded = JsonLimits::new(16 * 1024, 64, 8192, 256, 32).unwrap();
    assert_eq!(
        validate(deeply_nested.as_bytes(), bounded),
        Err(JsonError::Depth)
    );
}

#[test]
fn test_should_count_all_object_pairs_and_array_elements_in_one_budget() {
    let three = JsonLimits::new(1024, 8, 3, 256, 32).unwrap();
    validate(br#"{"a":[1,2]}"#, three).unwrap();
    assert_eq!(
        validate(br#"{"a":[1,2,3]}"#, three),
        Err(JsonError::Members)
    );
    validate(br#"{"a":{"b":1},"c":2}"#, three).unwrap();
    assert_eq!(
        validate(br#"{"a":{"b":1},"c":2,"d":3}"#, three),
        Err(JsonError::Members)
    );
    let one = JsonLimits::new(1024, 8, 1, 256, 32).unwrap();
    validate(br#"{"a":[]}"#, one).unwrap();
    validate(br#"{"a":{}}"#, one).unwrap();
    validate(b"[]", one).unwrap();
    assert_eq!(validate(br#"{"a":[1]}"#, one), Err(JsonError::Members));
}

#[test]
fn test_should_bound_strings_and_keys_by_decoded_utf8_bytes_and_encoded_work() {
    let small = JsonLimits::new(1024, 8, 64, 2, 1).unwrap();
    validate("\"é\"".as_bytes(), small).unwrap();
    validate(br#""\u00e9""#, small).unwrap();
    assert_eq!(validate("\"界\"".as_bytes(), small), Err(JsonError::String));
    assert_eq!(validate(br#""\u754c""#, small), Err(JsonError::String));
    assert_eq!(validate("{\"é\":1}".as_bytes(), small), Err(JsonError::Key));
    assert_eq!(validate(br#"{"\u00e9":1}"#, small), Err(JsonError::Key));
    let many_escapes = format!("\"{}\"", "\\u0061".repeat(100));
    assert_eq!(
        validate(many_escapes.as_bytes(), small),
        Err(JsonError::String)
    );
    validate(br#""\\""#, small).unwrap();
    validate(br#""\"""#, small).unwrap();
    let raised_key_limit = JsonLimits::new(1024, 1, 1, 512, 512).unwrap();
    let long_key = format!("{{\"{}\":0}}", "k".repeat(300));
    validate(long_key.as_bytes(), raised_key_limit).unwrap();
    assert_eq!(validate(long_key.as_bytes(), limits()), Err(JsonError::Key));
}

#[test]
fn test_should_enforce_input_bytes_before_lexical_processing() {
    let tiny = JsonLimits::new(4, 1, 1, 4, 4).unwrap();
    validate(b"null", tiny).unwrap();
    assert_eq!(validate(b"null ", tiny), Err(JsonError::Bytes));
    assert_eq!(validate(&[0xff; 5], tiny), Err(JsonError::Bytes));
}

#[test]
fn test_should_reject_invalid_limit_relationships_and_ranges() {
    for args in [
        (0, 1, 1, 1, 1),
        (1, 0, 1, 1, 1),
        (1, 1, 0, 1, 1),
        (1, 1, 1, 0, 1),
        (1, 1, 1, 1, 0),
        (16 * 1024 * 1024 + 1, 1, 1, 1, 1),
        (1024, 65, 1, 1, 1),
        (1024, 1, 65_537, 1, 1),
        (2 * 1024 * 1024, 1, 1, 1024 * 1024 + 1, 1),
        (1024, 1, 1, 256, 257),
        (1024, 1, 1, 1, 2),
        (1, 1, 1, 2, 1),
    ] {
        assert!(matches!(
            JsonLimits::new(args.0, args.1, args.2, args.3, args.4),
            Err(JsonError::InvalidLimits)
        ));
    }
}

fn values() -> impl Strategy<Value = Value> {
    prop_oneof![
        any::<bool>().prop_map(Value::Bool),
        any::<i32>().prop_map(|value| json!(value)),
        "[a-zA-Z0-9é界\\\"\n]{0,16}".prop_map(Value::String),
        proptest::strategy::Just(Value::Null),
    ]
    .prop_recursive(3, 64, 4, |inner| {
        prop_oneof![
            vec(inner.clone(), 0..4).prop_map(Value::Array),
            btree_map("[a-z]{1,8}", inner, 0..4)
                .prop_map(|map: BTreeMap<String, Value>| Value::Object(map.into_iter().collect())),
        ]
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn test_should_never_panic_on_bounded_malformed_json(input in vec(any::<u8>(), 0..4096)) {
        let result = validate(&input, JsonLimits::new(4096, 8, 64, 64, 32).unwrap());
        if result.is_ok() { prop_assert!(serde_json::from_slice::<Value>(&input).is_ok()); }
    }

    #[test]
    fn test_should_preserve_bounded_recursive_values(value in values()) {
        let bytes = serde_json::to_vec(&value).unwrap();
        prop_assert!(bytes.len() <= 16 * 1024);
        let parsed: Value = from_slice(&bytes, limits()).unwrap();
        prop_assert_eq!(parsed, value);
    }
}
