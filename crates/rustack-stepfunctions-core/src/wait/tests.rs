use std::{error::Error, time::Duration};

use chrono::{DateTime, Utc};
use proptest::prelude::*;
use rstest::rstest;
use rustack_core::json::JsonError;
use serde_json::{Value, json};

use crate::{AslError, EvaluationContext, Payload, WaitSource};

type TestResult = Result<(), Box<dyn Error>>;

fn now() -> Result<DateTime<Utc>, AslError> {
    DateTime::from_timestamp(0, 0).ok_or(AslError::WaitValue)
}

#[rstest]
#[case(json!({"Seconds":0}),json!({}),0)]
#[case(json!({"Seconds":5}),json!({}),5)]
#[case(json!({"Seconds":31_536_000}),json!({}),31_536_000)]
#[case(json!({"SecondsPath":"$.delay"}),json!({"delay":7}),7)]
#[case(json!({"Timestamp":"1970-01-01T00:00:05Z"}),json!({}),5)]
#[case(json!({"Timestamp":"1970-01-01T00:00:05.999999999999Z"}),json!({}),5)]
#[case(json!({"Timestamp":"1970-01-01T01:00:05+01:00"}),json!({}),5)]
#[case(json!({"Timestamp":"1970-01-01T00:00:00Z"}),json!({}),0)]
#[case(json!({"Timestamp":"1969-12-31T23:59:59Z"}),json!({}),0)]
#[case(json!({"Timestamp":"1971-01-01T00:00:00Z"}),json!({}),31_536_000)]
#[case(json!({"TimestampPath":"$.date"}),json!({"date":"1970-01-01T00:00:05Z"}),5)]
fn test_should_resolve_all_wait_source_forms(
    #[case] source: Value,
    #[case] input: Value,
    #[case] seconds: u64,
) -> TestResult {
    let source = WaitSource::compile(&source)?;
    let input = Payload::from_value(&input)?;
    assert_eq!(
        source.resolve(&input, &EvaluationContext::from(&input), now()?)?,
        Duration::from_secs(seconds)
    );
    assert_eq!(format!("{source:?}"), "WaitSource(<redacted>)");
    Ok(())
}

#[rstest]
#[case(json!({}))]
#[case(json!(null))]
#[case(json!({"Unknown":1}))]
#[case(json!({"Seconds":1,"Timestamp":"1970-01-01T00:00:00Z"}))]
#[case(json!({"Seconds":1,"Comment":"unknown component field"}))]
#[case(json!({"Seconds":1.0}))]
#[case(json!({"Seconds":-1}))]
#[case(json!({"Seconds":true}))]
#[case(json!({"Seconds":"1"}))]
#[case(json!({"Seconds":31_536_001}))]
#[case(json!({"Seconds":99_999_999}))]
#[case(json!({"Timestamp":"1970-01-01"}))]
#[case(json!({"Timestamp":"1970-01-01t00:00:00Z"}))]
#[case(json!({"Timestamp":"1970-01-01T00:00:00z"}))]
#[case(json!({"Timestamp":"2016-12-31T23:59:60Z"}))]
#[case(json!({"Timestamp":"2020-02-30T00:00:00Z"}))]
#[case(json!({"Timestamp":0}))]
#[case(json!({"SecondsPath":0}))]
fn test_should_reject_ambiguous_or_invalid_static_wait_sources(#[case] value: Value) {
    assert!(matches!(
        WaitSource::compile(&value),
        Err(AslError::InvalidWait)
    ));
}

#[rstest]
#[case("SecondsPath",json!({"value":null}))]
#[case("SecondsPath",json!({"value":"5"}))]
#[case("SecondsPath",json!({"value":1.0}))]
#[case("SecondsPath",json!({"value":-1}))]
#[case("SecondsPath",json!({"value":31_536_001}))]
#[case("TimestampPath",json!({"value":5}))]
#[case("TimestampPath",json!({"value":"invalid"}))]
#[case("TimestampPath",json!({"value":"1970-01-01t00:00:00Z"}))]
#[case("TimestampPath",json!({"value":"1971-01-01T00:00:01Z"}))]
fn test_should_fail_invalid_dynamic_values_without_defaulting(
    #[case] field: &str,
    #[case] input: Value,
) -> TestResult {
    let source = WaitSource::compile(&json!({field:"$.value"}))?;
    let input = Payload::from_value(&input)?;
    assert!(matches!(
        source.resolve(&input, &EvaluationContext::from(&input), now()?),
        Err(AslError::WaitValue)
    ));
    Ok(())
}

#[test]
fn test_should_preserve_missing_reference_class_and_validate_paths() -> TestResult {
    let input = Payload::parse(b"{}")?;
    for field in ["SecondsPath", "TimestampPath"] {
        let source = WaitSource::compile(&json!({field:"$.missing"}))?;
        assert!(matches!(
            source.resolve(&input, &EvaluationContext::from(&input), now()?),
            Err(AslError::Missing)
        ));
        assert!(matches!(
            WaitSource::compile(&json!({field:"$[*]"})),
            Err(AslError::InvalidReference)
        ));
    }
    let context = EvaluationContext::from_value(&json!({"State":{"delay":9}}))?;
    let source = WaitSource::compile(&json!({"SecondsPath":"$$.State.delay"}))?;
    assert_eq!(
        source.resolve(&input, &context, now()?)?,
        Duration::from_secs(9)
    );
    assert!(matches!(
        WaitSource::parse(br#"{"Seconds":1,"Seconds":2}"#),
        Err(AslError::Json(JsonError::Duplicate))
    ));
    assert!(matches!(
        WaitSource::parse(br#"{"Seconds":1e0}"#),
        Err(AslError::InvalidWait)
    ));
    Ok(())
}

#[test]
fn test_should_truncate_only_the_target_preserving_current_fraction_and_supplied_clock()
-> TestResult {
    let source = WaitSource::parse(br#"{"Timestamp":"1970-01-01T00:00:10.999999999Z"}"#)?;
    let input = Payload::parse(b"{}")?;
    let context = EvaluationContext::from(&input);
    let sample = DateTime::from_timestamp(9, 250_000_000).ok_or("invalid test clock")?;
    assert_eq!(
        source.resolve(&input, &context, sample)?,
        Duration::from_millis(750)
    );
    let target_past = DateTime::from_timestamp(10, 1).ok_or("invalid test clock")?;
    assert_eq!(
        source.resolve(&input, &context, target_past)?,
        Duration::ZERO
    );
    let overlong = WaitSource::parse(br#"{"Timestamp":"1971-01-01T00:00:01Z"}"#)?;
    assert!(matches!(
        overlong.resolve(&input, &context, now()?),
        Err(AslError::WaitValue)
    ));
    // A new entry uses its new sample, without the component reading a clock.
    assert_eq!(
        source.resolve(&input, &context, now()?)?,
        Duration::from_secs(10)
    );
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]
    #[test]
    fn test_should_resolve_static_and_dynamic_seconds_equivalently(seconds in 0_u64..=31_536_000) {
        let input=Payload::from_value(&json!({"seconds":seconds})).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let context=EvaluationContext::from(&input);
        let clock=now().map_err(|e| TestCaseError::fail(e.to_string()))?;
        let static_source=WaitSource::compile(&json!({"Seconds":seconds})).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let dynamic_source=WaitSource::compile(&json!({"SecondsPath":"$.seconds"})).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(static_source.resolve(&input,&context,clock).map_err(|e| TestCaseError::fail(e.to_string()))?,dynamic_source.resolve(&input,&context,clock).map_err(|e| TestCaseError::fail(e.to_string()))?);
    }
}
