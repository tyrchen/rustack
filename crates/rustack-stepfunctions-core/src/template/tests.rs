use std::error::Error;

use proptest::prelude::*;
use rstest::rstest;
use serde_json::{Map, Value, json};
use uuid::{Builder, Uuid};

use crate::{AslError, EvaluationContext, Payload, Template, UuidSource};

type TestResult = Result<(), Box<dyn Error>>;

#[derive(Default)]
struct Ids(u8);
impl UuidSource for Ids {
    fn next_uuid(&mut self) -> Result<Uuid, AslError> {
        self.0 = self.0.checked_add(1).ok_or(AslError::WorkLimit)?;
        Ok(Builder::from_random_bytes([self.0; 16]).into_uuid())
    }
}

#[test]
fn test_should_expand_nested_arrays_context_and_intrinsics_without_reinterpreting_literals()
-> TestResult {
    let original = Payload::from_value(&json!({"n":3}))?;
    let context = EvaluationContext::from_value(&json!({"Execution":{"Name":"run"}}))?;
    let template = Template::compile(&json!({
        "literal":"$.n","code":"States.UUID()",
        "nested":[{"n.$":"States.MathAdd($.n,1)","run.$":"$$.Execution.Name"},"$.n"],
        "name.$.$":"$.n","a0":1,"a.$":"$.n","a-":2
    }))?;
    let output = template.evaluate(&original, &context)?;
    assert_eq!(
        output.value(),
        &json!({"literal":"$.n","code":"States.UUID()","nested":[{"n":4,"run":"run"},"$.n"],"name.$":3,"a0":1,"a":3,"a-":2})
    );
    assert!(
        output
            .original_bytes()
            .starts_with(br#"{"a":3,"a-":2,"a0":1"#)
    );
    assert_eq!(original.value(), &json!({"n":3}));
    assert_eq!(format!("{template:?}"), "Template(<redacted>)");
    Ok(())
}

#[rstest]
#[case(json!([]))]
#[case(json!(null))]
#[case(json!({"a":1,"a.$":"$.x"}))]
#[case(json!({"nested":[{"a":1,"a.$":"$.x"}]}))]
#[case(json!({"a.$":null}))]
#[case(json!({"a.$":1}))]
#[case(json!({"a.$":"literal"}))]
fn test_should_reject_invalid_template_roots_dynamic_values_and_normalized_collisions(
    #[case] value: Value,
) {
    assert!(Template::compile(&value).is_err());
}

#[test]
fn test_should_apply_one_shared_source_to_all_dynamic_fields() -> TestResult {
    let template = Template::compile(&json!({"b.$":"States.UUID()","a.$":"States.UUID()"}))?;
    let input = Payload::parse(b"null")?;
    let mut ids = Ids::default();
    let output =
        template.evaluate_with_source(&input, &EvaluationContext::from(&input), &mut ids)?;
    assert_eq!(ids.0, 2);
    assert_eq!(
        output.value()["a"],
        json!(Builder::from_random_bytes([1; 16]).into_uuid().to_string())
    );
    assert_eq!(
        output.value()["b"],
        json!(Builder::from_random_bytes([2; 16]).into_uuid().to_string())
    );
    Ok(())
}

#[test]
fn test_should_stream_oversized_constant_and_dynamic_results_before_owning_output() -> TestResult {
    let input = Payload::from_value(&json!({"x":"a".repeat(140_000)}))?;
    let context = EvaluationContext::from(&input);
    let dynamic = Template::compile(&json!({"a.$":"$.x","b.$":"$.x"}))?;
    assert!(matches!(
        dynamic.evaluate(&input, &context),
        Err(AslError::DataLimit)
    ));
    let constants = Template::compile(&json!({"a":"a".repeat(150_000),"b":"b".repeat(150_000)}))?;
    assert!(matches!(
        constants.evaluate(&input, &context),
        Err(AslError::DataLimit)
    ));
    assert_eq!(
        input.value()["x"].as_str().ok_or("missing x")?.len(),
        140_000
    );
    Ok(())
}

#[test]
fn test_should_share_work_across_fields_and_charge_embedded_expression_nodes() -> TestResult {
    let input = Payload::from_value(&json!({"items":vec![0;6000]}))?;
    let context = EvaluationContext::from(&input);
    let one = Template::compile(&json!({"a.$":"States.ArrayLength($.items[*])"}))?;
    assert_eq!(one.evaluate(&input, &context)?.value(), &json!({"a":6000}));
    let two = Template::compile(
        &json!({"a.$":"States.ArrayLength($.items[*])","b.$":"States.ArrayLength($.items[*])"}),
    )?;
    assert!(matches!(
        two.evaluate(&input, &context),
        Err(AslError::WorkLimit)
    ));
    let group = format!("States.Array({})", vec!["0"; 127].join(","));
    let four = format!("States.Array({})", vec![group; 8].join(","));
    // This expression has1,025 nodes and is independently invalid.
    assert!(Template::compile(&json!({"a.$":four})).is_err());
    let expression = format!("States.Array({})", vec!["0"; 128].join(","));
    let mut fields = Map::new();
    for index in 0..32 {
        fields.insert(format!("k{index}.$"), json!(expression));
    }
    assert!(matches!(
        Template::compile(&Value::Object(fields)),
        Err(AslError::InvalidTemplate)
    ));
    let mut exact = Map::new();
    for index in 0..31 {
        exact.insert(format!("k{index}.$"), json!(expression));
    }
    exact.insert(
        "last.$".into(),
        json!(format!("States.Array({})", vec!["0"; 63].join(","))),
    );
    let valid = Template::compile(&Value::Object(exact.clone()))?;
    valid.evaluate(&input, &context)?;
    exact.insert(
        "last.$".into(),
        json!(format!("States.Array({})", vec!["0"; 64].join(","))),
    );
    assert!(matches!(
        Template::compile(&Value::Object(exact)),
        Err(AslError::InvalidTemplate)
    ));
    Ok(())
}

#[test]
fn test_should_enforce_template_depth_and_reject_borrowed_extreme_trees_safely() -> TestResult {
    let mut nested = Value::Null;
    for _ in 0..31 {
        nested = json!([nested]);
    }
    Template::compile(&json!({"value":nested}))?;
    assert!(matches!(
        Template::compile(&json!({"value":[nested]})),
        Err(AslError::InvalidTemplate)
    ));
    let mut tree = Value::Null;
    for _ in 0..50_000 {
        tree = Value::Array(vec![tree]);
    }
    let rejected = Template::compile(&tree);
    let mut owned = vec![tree];
    while let Some(value) = owned.pop() {
        if let Value::Array(mut values) = value {
            owned.append(&mut values);
        }
    }
    assert!(rejected.is_err());
    assert!(Template::parse(br#"{"a":1,"a":2}"#).is_err());
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]
    #[test]
    fn test_should_strip_exactly_one_dynamic_suffix_for_arbitrary_business_keys(name in ".{0,20}", n in -1_000_000_i32..1_000_000) {
        let template=Template::compile(&json!({format!("{name}.$"):"States.MathAdd($.n,1)"})).map_err(|e|TestCaseError::fail(e.to_string()))?;
        let input=Payload::from_value(&json!({"n":n})).map_err(|e|TestCaseError::fail(e.to_string()))?;
        let output=template.evaluate(&input,&EvaluationContext::from(&input)).map_err(|e|TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(output.value(),&json!({name:n+1}));
    }
}
