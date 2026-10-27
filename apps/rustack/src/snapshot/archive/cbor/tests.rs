//! Profile, resource and before-deserialization barriers for snapshot imports.

use std::{cell::Cell, collections::BTreeMap, error::Error};

use ciborium::Value;
use proptest::{collection, prelude::*};
use rstest::rstest;
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::IgnoredAny,
    ser::{Error as _, SerializeSeq},
};

use super::{
    CappedWriter, Failure, LIMITS, Limits, Scan, decode, decode_with_limits, encode,
    encode_with_limits,
};

type TestResult = Result<(), Box<dyn Error>>;

thread_local! {
    static VISITS: Cell<usize> = const { Cell::new(0) };
}

#[derive(Debug)]
struct Sentinel;
impl<'de> Deserialize<'de> for Sentinel {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        VISITS.set(VISITS.get().saturating_add(1));
        IgnoredAny::deserialize(decoder)?;
        Ok(Self)
    }
}

fn rejected(bytes: &[u8], limits: Limits, expected: Failure) {
    VISITS.set(0);
    assert_eq!(
        decode_with_limits::<Sentinel>(bytes, limits).err(),
        Some(expected)
    );
    assert_eq!(VISITS.get(), 0, "typed allocation must follow preflight");
}

#[rstest]
#[case(&[], Failure::Syntax)]
#[case(&[0xff], Failure::Syntax)]
#[case(&[0x1f], Failure::Syntax)]
#[case(&[0x3f], Failure::Syntax)]
#[case(&[0x1c], Failure::Syntax)]
#[case(&[0x1d], Failure::Syntax)]
#[case(&[0x1e], Failure::Syntax)]
#[case(&[0x18], Failure::Syntax)]
#[case(&[0x19, 0x01], Failure::Syntax)]
#[case(&[0x1a, 0x00, 0x00, 0x00], Failure::Syntax)]
#[case(&[0x1b, 0x00, 0x00, 0x00, 0x00], Failure::Syntax)]
#[case(&[0x61], Failure::Syntax)]
#[case(&[0x81], Failure::Syntax)]
#[case(&[0xa1, 0x00], Failure::Syntax)]
#[case(&[0x9f], Failure::Syntax)]
#[case(&[0xbf, 0x61, b'a', 0xff], Failure::Syntax)]
#[case(&[0x81, 0xff], Failure::Syntax)]
#[case(&[0xa1, 0xff, 0x00], Failure::Profile)]
#[case(&[0x62, 0xc0, 0xaf], Failure::Syntax)]
#[case(&[0xa1, 0x61, 0xff, 0x00], Failure::Syntax)]
#[case(&[0xf8, 0x00], Failure::Syntax)]
#[case(&[0xf8, 0x1f], Failure::Syntax)]
#[case(&[0xfc], Failure::Syntax)]
#[case(&[0xfd], Failure::Syntax)]
#[case(&[0xfe], Failure::Syntax)]
#[case(&[0xc0, 0x60], Failure::Profile)]
#[case(&[0x5f, 0xff], Failure::Profile)]
#[case(&[0x7f, 0xff], Failure::Profile)]
#[case(&[0xa1, 0x80, 0x00], Failure::Profile)]
#[case(&[0xa1, 0x40, 0x00], Failure::Profile)]
#[case(&[0xa1, 0xf5, 0x00], Failure::Profile)]
#[case(&[0xa1, 0xf9, 0x00, 0x00, 0x00], Failure::Profile)]
#[case(&[0x00, 0x00], Failure::Trailing)]
#[case(&[0x00, 0xff], Failure::Trailing)]
#[case(&[0x80, 0x61, b'a'], Failure::Trailing)]
fn test_should_reject_hostile_structure_before_typed_visitors(
    #[case] bytes: &[u8],
    #[case] expected: Failure,
) {
    rejected(bytes, LIMITS, expected);
}

#[rstest]
#[case(&[0xa2, 0x61, b'x', 0x00, 0x61, b'x', 0x01])]
#[case(&[0xa2, 0x61, b'x', 0x00, 0x78, 0x01, b'x', 0x01])]
#[case(&[0xa2, 0x01, 0x00, 0x18, 0x01, 0x01])]
#[case(&[0xa2, 0x20, 0x00, 0x38, 0x00, 0x01])]
#[case(&[0xbf, 0x61, b'x', 0x00, 0x61, b'x', 0x01, 0xff])]
fn test_should_reject_semantically_duplicate_keys(#[case] bytes: &[u8]) {
    rejected(bytes, LIMITS, Failure::Duplicate);
}

fn numeric_header(major: u8, width: usize, value: u8) -> Vec<u8> {
    match width {
        0 => vec![(major << 5) | value],
        1 => vec![(major << 5) | 0x18, value],
        2 => vec![(major << 5) | 0x19, 0, value],
        4 => vec![(major << 5) | 0x1a, 0, 0, 0, value],
        8 => vec![(major << 5) | 0x1b, 0, 0, 0, 0, 0, 0, 0, value],
        _ => Vec::new(),
    }
}

#[rstest]
#[case(0)]
#[case(1)]
#[case(2)]
#[case(4)]
#[case(8)]
fn test_should_accept_every_header_width_without_requiring_canonical_order(
    #[case] width: usize,
) -> TestResult {
    assert_eq!(decode::<u64>(&numeric_header(0, width, 1))?, 1);
    assert_eq!(decode::<i64>(&numeric_header(1, width, 1))?, -2);
    let mut text = numeric_header(3, width, 1);
    text.push(b'a');
    assert_eq!(decode::<String>(&text)?, "a");
    let mut bytes = numeric_header(2, width, 1);
    bytes.push(0xff);
    assert_eq!(decode::<Value>(&bytes)?, Value::Bytes(vec![0xff]));
    let mut array = numeric_header(4, width, 1);
    array.push(0x00);
    assert_eq!(decode::<Vec<u8>>(&array)?, vec![0]);
    let mut map = numeric_header(5, width, 1);
    map.extend_from_slice(&[0x61, b'a', 0x00]);
    assert_eq!(decode::<BTreeMap<String, u8>>(&map)?.get("a"), Some(&0));
    Ok(())
}

#[rstest]
#[case(&[0xf4], Value::Bool(false))]
#[case(&[0xf5], Value::Bool(true))]
#[case(&[0xf6], Value::Null)]
#[case(&[0x40], Value::Bytes(vec![]))]
#[case(&[0x60], Value::Text(String::new()))]
#[case(&[0x80], Value::Array(vec![]))]
#[case(&[0xa0], Value::Map(vec![]))]
#[case(&[0x9f, 0x00, 0x01, 0xff], Value::Array(vec![0.into(), 1.into()]))]
#[case(&[0xbf, 0x00, 0x00, 0xff], Value::Map(vec![(0.into(), 0.into())]))]
#[case(&[0xf9, 0x3e, 0x00], Value::Float(1.5))]
#[case(&[0xfa, 0x3f, 0xc0, 0x00, 0x00], Value::Float(1.5))]
#[case(&[0xfb, 0x3f, 0xf8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], Value::Float(1.5))]
fn test_should_preserve_supported_cbor_values(
    #[case] bytes: &[u8],
    #[case] expected: Value,
) -> TestResult {
    assert_eq!(decode::<Value>(bytes)?, expected);
    Ok(())
}

#[test]
fn test_should_allow_integer_keyed_versions_and_nested_key_reuse() -> TestResult {
    let value = BTreeMap::from([
        (0_u64, BTreeMap::from([("same".to_owned(), vec![0, 1, 2])])),
        (
            u64::MAX,
            BTreeMap::from([("same".to_owned(), vec![3, 4, 5])]),
        ),
    ]);
    assert_eq!(
        decode::<BTreeMap<u64, BTreeMap<String, Vec<i32>>>>(&encode(&value)?)?,
        value
    );
    let negative: BTreeMap<i64, u8> = decode(&[0xa2, 0x20, 0x01, 0x00, 0x02])?;
    assert_eq!(negative, BTreeMap::from([(-1, 1), (0, 2)]));
    Ok(())
}

#[test]
fn test_should_bound_section_bytes_at_the_exact_boundary() -> TestResult {
    let bytes = [0x61, b'a'];
    let limits = Limits { bytes: 2, ..LIMITS };
    assert_eq!(decode_with_limits::<String>(&bytes, limits)?, "a");
    assert_eq!(encode_with_limits(&"a", limits)?, bytes);
    rejected(&bytes, Limits { bytes: 1, ..limits }, Failure::Bytes);
    assert_eq!(
        encode_with_limits(&"a", Limits { bytes: 1, ..limits }),
        Err(Failure::Bytes)
    );
    Ok(())
}

#[test]
fn test_should_bound_all_aggregate_nodes_including_keys_and_containers() -> TestResult {
    // The root map, key, array and two scalar elements occupy five nodes.
    let bytes = [0xa1, 0x61, b'k', 0x82, 0x00, 0x01];
    let limits = Limits { nodes: 5, ..LIMITS };
    assert!(decode_with_limits::<Value>(&bytes, limits).is_ok());
    rejected(&bytes, Limits { nodes: 4, ..limits }, Failure::Nodes);
    rejected(&[0x00], Limits { nodes: 0, ..limits }, Failure::Nodes);
    Scan::validate(&[0x00], Limits { nodes: 1, ..limits })?;
    Ok(())
}

#[rstest]
#[case(&[0x82, 0x00, 0x01])]
#[case(&[0x9f, 0x00, 0x01, 0xff])]
#[case(&[0xa2, 0x00, 0x00, 0x01, 0x01])]
#[case(&[0xbf, 0x00, 0x00, 0x01, 0x01, 0xff])]
fn test_should_bound_definite_and_indefinite_collection_counts(#[case] bytes: &[u8]) {
    let limits = Limits {
        collection: 2,
        ..LIMITS
    };
    assert!(decode_with_limits::<Value>(bytes, limits).is_ok());
    rejected(
        bytes,
        Limits {
            collection: 1,
            ..limits
        },
        Failure::Collection,
    );
}

#[test]
fn test_should_bound_actual_container_depth_without_counting_scalars() -> TestResult {
    Scan::validate(&[0x00], Limits { depth: 0, ..LIMITS })?;
    rejected(&[0x80], Limits { depth: 0, ..LIMITS }, Failure::Depth);
    let limits = Limits { depth: 2, ..LIMITS };
    Scan::validate(&[0x81, 0x81, 0x00], limits)?;
    rejected(&[0x81, 0x81, 0x80], limits, Failure::Depth);
    let mut bytes = vec![0x81; LIMITS.depth];
    bytes.push(0x00);
    let _: Value = decode(&bytes)?;
    bytes.insert(0, 0x81);
    rejected(&bytes, LIMITS, Failure::Depth);
    Ok(())
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum Tree {
    Leaf,
    Seq(Vec<Self>),
}

#[test]
fn test_should_round_trip_derived_enums_at_exact_wire_depth_with_typed_headroom() -> TestResult {
    let mut value = Tree::Leaf;
    for _ in 0..64 {
        value = Tree::Seq(vec![value]);
    }
    let bytes = encode(&value)?;
    Scan::validate(&bytes, LIMITS)?;
    // W190: a scalar Leaf also consumes the typed decoder's enum guard.
    assert!(
        ciborium::de::from_reader_with_recursion_limit::<Tree, _>(bytes.as_slice(), 128).is_err()
    );
    assert_eq!(decode::<Tree>(&bytes)?, value);
    Ok(())
}

#[rstest]
#[case(&[0x63, b'a', b'b', b'c'])]
#[case(&[0x43, 0x00, 0x01, 0xff])]
fn test_should_bound_definite_scalar_lengths_before_content(#[case] bytes: &[u8]) -> TestResult {
    Scan::validate(
        bytes,
        Limits {
            scalar: 3,
            ..LIMITS
        },
    )?;
    rejected(
        bytes,
        Limits {
            scalar: 2,
            ..LIMITS
        },
        Failure::Scalar,
    );
    let claim = numeric_header(3, 8, 3);
    rejected(
        &claim,
        Limits {
            scalar: 2,
            ..LIMITS
        },
        Failure::Scalar,
    );
    Ok(())
}

#[test]
fn test_should_bound_decoded_text_keys_in_bytes() -> TestResult {
    // The Unicode key contains two bytes, one character.
    let bytes = [0xa1, 0x62, 0xc3, 0xa9, 0x00];
    Scan::validate(&bytes, Limits { key: 2, ..LIMITS })?;
    rejected(&bytes, Limits { key: 1, ..LIMITS }, Failure::Key);
    let claim = [0xa1, 0x62, 0x00];
    rejected(&claim, Limits { key: 1, ..LIMITS }, Failure::Key);
    Ok(())
}

#[test]
fn test_should_reject_extreme_external_lengths_without_allocation() {
    for (major, expected) in [
        (2, Failure::Scalar),
        (3, Failure::Scalar),
        (4, Failure::Collection),
        (5, Failure::Collection),
    ] {
        let mut bytes = vec![(major << 5) | 0x1b];
        bytes.extend_from_slice(&u64::MAX.to_be_bytes());
        rejected(&bytes, LIMITS, expected);
    }
}

#[test]
fn test_should_reject_all_truncated_prefixes_of_a_nested_value() -> TestResult {
    let value = BTreeMap::from([("outer", vec![vec!["hello", "world"], vec!["é"]])]);
    let bytes = encode(&value)?;
    for len in 0..bytes.len() {
        let prefix = bytes.get(..len).ok_or("invalid test prefix")?;
        VISITS.set(0);
        assert!(decode::<Sentinel>(prefix).is_err());
        assert_eq!(VISITS.get(), 0);
    }
    Ok(())
}

#[test]
fn test_should_remove_attacker_data_from_schema_errors_and_sources() {
    let secret = "private-snapshot-content";
    let mut raw = Vec::new();
    assert!(ciborium::into_writer(&secret, &mut raw).is_ok());
    let error = decode::<u64>(&raw).err();
    assert_eq!(error, Some(Failure::Schema));
    assert!(!format!("{error:?}").contains(secret));
    if let Some(error) = error {
        assert!(error.source().is_none());
        assert_eq!(error.to_string(), "invalid snapshot CBOR schema");
    }
}

struct FailingSerializer;
impl Serialize for FailingSerializer {
    fn serialize<S: Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
        Err(S::Error::custom("private-serialization-content"))
    }
}

#[test]
fn test_should_redact_serializer_failures() {
    let error = encode(&FailingSerializer).err();
    assert_eq!(error, Some(Failure::Encoding));
    assert!(!format!("{error:?}").contains("private-serialization-content"));
}

struct ManyValues;
impl Serialize for ManyValues {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(None)?;
        for _ in 0..1_000_000 {
            VISITS.set(VISITS.get().saturating_add(1));
            sequence.serialize_element(&0_u8)?;
        }
        sequence.end()
    }
}

#[test]
fn test_should_stop_oversized_serialization_before_finishing_the_input() {
    VISITS.set(0);
    assert_eq!(
        encode_with_limits(&ManyValues, Limits { bytes: 8, ..LIMITS }),
        Err(Failure::Bytes)
    );
    assert!(VISITS.get() <= 8);
}

#[test]
fn test_should_keep_capped_writer_bytes_unchanged_on_oversize() -> TestResult {
    let mut writer = CappedWriter {
        bytes: Vec::new(),
        limit: 3,
        failure: None,
    };
    assert_eq!(writer.append(&[0, 1])?, 2);
    assert_eq!(writer.append(&[2, 3]), Err(Failure::Bytes));
    assert_eq!(writer.bytes, [0, 1]);
    assert_eq!(writer.append(&[2])?, 1);
    assert_eq!(writer.bytes, [0, 1, 2]);
    Ok(())
}

#[test]
fn test_should_apply_profile_checks_to_newly_encoded_values() {
    assert_eq!(
        encode(&Value::Tag(0, Box::new(Value::Text("date".into())))),
        Err(Failure::Profile)
    );
    assert_eq!(
        encode(&Value::Map(vec![(Value::Bool(true), Value::Null)])),
        Err(Failure::Profile)
    );
    assert_eq!(
        encode(&Value::Map(vec![
            (Value::Text("x".into()), Value::Null),
            (Value::Text("x".into()), Value::Null)
        ])),
        Err(Failure::Duplicate)
    );
    assert_eq!(
        encode_with_limits(
            &vec![0; 3],
            Limits {
                collection: 2,
                ..LIMITS
            }
        ),
        Err(Failure::Collection)
    );
    assert_eq!(
        encode_with_limits(
            &"abc",
            Limits {
                scalar: 2,
                ..LIMITS
            }
        ),
        Err(Failure::Scalar)
    );
}

struct Early;
impl<'de> Deserialize<'de> for Early {
    fn deserialize<D: Deserializer<'de>>(_decoder: D) -> Result<Self, D::Error> {
        Ok(Self)
    }
}

#[test]
fn test_should_require_the_typed_reader_to_consume_the_single_root() {
    assert!(matches!(decode::<Early>(&[0x00]), Err(Failure::Trailing)));
}

#[test]
fn test_should_demonstrate_preflight_is_required_for_trailing_and_duplicate_input() -> TestResult {
    assert_eq!(ciborium::from_reader::<u8, _>([0x00, 0x01].as_slice())?, 0);
    rejected(&[0x00, 0x01], LIMITS, Failure::Trailing);
    let duplicate = [0xa2, 0x61, b'x', 0x00, 0x61, b'x', 0x01];
    assert_eq!(
        ciborium::from_reader::<BTreeMap<String, u8>, _>(duplicate.as_slice())?.get("x"),
        Some(&1)
    );
    rejected(&duplicate, LIMITS, Failure::Duplicate);
    Ok(())
}

fn profile_values() -> impl Strategy<Value = Value> {
    prop_oneof![
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(|v| Value::Integer(v.into())),
        "[a-zé水0-9]{0,32}".prop_map(Value::Text),
        collection::vec(any::<u8>(), 0..64).prop_map(Value::Bytes),
        Just(Value::Null),
    ]
    .prop_recursive(6, 128, 12, |inner| {
        prop_oneof![
            collection::vec(inner.clone(), 0..12).prop_map(Value::Array),
            collection::btree_map("[a-zA-Z0-9]{0,16}", inner, 0..8).prop_map(|map| Value::Map(
                map.into_iter()
                    .map(|(key, value)| (Value::Text(key), value))
                    .collect()
            )),
        ]
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn test_should_round_trip_bounded_nested_profile_values(value in profile_values()) {
        let encoded = encode(&value);
        prop_assert!(encoded.is_ok());
        if let Ok(bytes) = encoded {
            let mut original = Vec::new();
            let original_result = ciborium::into_writer(&value, &mut original);
            prop_assert!(original_result.is_ok());
            prop_assert_eq!(bytes.as_slice(), original.as_slice());
            let decoded = decode::<Value>(&bytes);
            prop_assert!(decoded.is_ok());
            if let Ok(decoded) = decoded {
                prop_assert_eq!(decoded, value);
            }
        }
    }

    #[test]
    fn test_should_never_deserialize_rejected_arbitrary_bytes(bytes in collection::vec(any::<u8>(), 0..4096)) {
        let rejected = Scan::validate(&bytes, LIMITS).is_err();
        VISITS.set(0);
        let _result = decode::<Sentinel>(&bytes);
        if rejected {
            prop_assert_eq!(VISITS.get(), 0);
        }
    }
}
