//! Required/optional numeric wire timestamps from the actual generated model.

use chrono::{DateTime, Utc};
use rstest::rstest;
use rustack_stepfunctions_model::{
    output::{DescribeExecutionOutput, StartExecutionOutput},
    types::{ExecutionStatus, HistoryEvent, HistoryEventType},
};
use serde_json::json;

#[rstest]
#[case("0", 0)]
#[case("1", 1000)]
#[case("1700000000.123", 1_700_000_000_123)]
#[case("-0.25",-250)]
#[case("-1.999",-1999)]
#[case("-1.999999",-1999)]
#[case("0.000999", 0)]
#[case("-0.000999", 0)]
#[case("1.23e2", 123_000)]
#[case("123e-3", 123)]
#[case("0.00123e3", 1230)]
#[case("1e-100", 0)]
#[case("-0e100", 0)]
#[case("0e2147483648", 0)]
#[case("-0e-2147483649", 0)]
#[case("1e-2147483649", 0)]
fn test_should_decode_and_roundtrip_generated_required_timestamp(
    #[case] number: &str,
    #[case] millis: i64,
) {
    let wire = format!(r#"{{"executionArn":"execution","startDate":{number}}}"#);
    let decoded: StartExecutionOutput = serde_json::from_str(&wire).unwrap();
    assert_eq!(decoded.start_date.timestamp_millis(), millis);
    let serialized = serde_json::to_string(&decoded).unwrap();
    assert!(!serialized.contains('Z'));
    let roundtrip: StartExecutionOutput = serde_json::from_str(&serialized).unwrap();
    assert_eq!(roundtrip.start_date, decoded.start_date);
}
#[rstest]
#[case("null")]
#[case("true")]
#[case("[]")]
#[case("{}")]
#[case("\"1700000000\"")]
#[case("\"2026-01-01T00:00:00Z\"")]
#[case("1e100")]
#[case("9223372036854775807")]
#[case("-9223372036854775808")]
#[case("NaN")]
fn test_should_reject_invalid_required_timestamp(#[case] value: &str) {
    let wire = format!(r#"{{"executionArn":"execution","startDate":{value}}}"#);
    assert!(serde_json::from_str::<StartExecutionOutput>(&wire).is_err());
}
#[test]
fn test_should_omit_optional_timestamp_and_accept_null_or_exponent_values() {
    let output = DescribeExecutionOutput::builder()
        .execution_arn("execution".into())
        .state_machine_arn("machine".into())
        .status(ExecutionStatus::Running)
        .start_date(DateTime::from_timestamp_millis(-250).unwrap())
        .build();
    let serialized = serde_json::to_string(&output).unwrap();
    assert!(!serialized.contains("stopDate"));
    let mut value = serde_json::to_value(&output).unwrap();
    value["stopDate"] = json!(null);
    let decoded: DescribeExecutionOutput = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(decoded.stop_date, None);
    value["stopDate"] = json!(-0.75);
    let decoded: DescribeExecutionOutput = serde_json::from_value(value).unwrap();
    assert_eq!(decoded.stop_date.unwrap().timestamp_millis(), -750);
    let wire = serialized.trim_end_matches('}').to_owned() + ",\"stopDate\":-2.5e-1}";
    assert_eq!(
        serde_json::from_str::<DescribeExecutionOutput>(&wire)
            .unwrap()
            .stop_date
            .unwrap()
            .timestamp_millis(),
        -250
    );
}
#[test]
fn test_should_serialize_exact_decimal_and_truncate_submillisecond_precision() {
    let output = HistoryEvent::builder()
        .id(1)
        .timestamp(DateTime::from_timestamp(-2, 765_432_100).unwrap())
        .r#type(HistoryEventType::ExecutionStarted)
        .build();
    let serialized = serde_json::to_string(&output).unwrap();
    assert!(serialized.contains(r#""timestamp":-1.234"#));
    let decoded: HistoryEvent = serde_json::from_str(&serialized).unwrap();
    assert_eq!(decoded.timestamp.timestamp_millis(), -1234);
}
#[test]
fn test_should_reject_leap_seconds_missing_required_fields_and_unknown_fields() {
    let mut output = StartExecutionOutput::default();
    output.start_date = DateTime::from_timestamp(59, 1_000_000_000).unwrap();
    assert!(serde_json::to_string(&output).is_err());
    assert!(
        serde_json::from_str::<StartExecutionOutput>(r#"{"executionArn":"execution"}"#).is_err()
    );
    assert!(
        serde_json::from_str::<StartExecutionOutput>(
            r#"{"executionArn":"execution","startDate":0,"extra":0}"#
        )
        .is_err()
    );
}
#[test]
fn test_should_preserve_chrono_range_millisecond_boundaries_without_float_rounding() {
    for value in [DateTime::<Utc>::MIN_UTC, DateTime::<Utc>::MAX_UTC] {
        let mut output = StartExecutionOutput::default();
        output.start_date = value;
        let serialized = serde_json::to_string(&output).unwrap();
        let decoded: StartExecutionOutput = serde_json::from_str(&serialized).unwrap();
        let total = i128::from(value.timestamp()) * 1_000_000_000
            + i128::from(value.timestamp_subsec_nanos());
        assert_eq!(
            i128::from(decoded.start_date.timestamp_millis()),
            total / 1_000_000
        );
    }
}
#[test]
fn test_should_reject_oversized_timestamp_lexeme_with_redacted_error() {
    let wire = format!(
        r#"{{"executionArn":"execution","startDate":0.{}}}"#,
        "1".repeat(129)
    );
    let error = serde_json::from_str::<StartExecutionOutput>(&wire)
        .unwrap_err()
        .to_string();
    assert!(error.contains("invalid epoch-second timestamp"));
    assert!(!error.contains(&"1".repeat(20)));
}

#[test]
fn test_should_accept_optional_oversized_exponents_only_when_value_normalizes_to_zero() {
    for number in ["0e2147483648", "-0e-2147483649", "1e-2147483649"] {
        let wire = format!(
            r#"{{"executionArn":"execution","stateMachineArn":"machine","startDate":0,"status":"RUNNING","stopDate":{number}}}"#
        );
        let decoded: DescribeExecutionOutput = serde_json::from_str(&wire).unwrap();
        assert_eq!(decoded.stop_date.unwrap().timestamp_millis(), 0);
    }
    let wire = r#"{"executionArn":"execution","startDate":1e2147483648}"#;
    assert!(serde_json::from_str::<StartExecutionOutput>(wire).is_err());
}
#[test]
fn test_should_redact_generated_inputs_nested_history_and_error_diagnostics() {
    use std::error::Error;

    use rustack_stepfunctions_model::{
        StepFunctionsError, StepFunctionsErrorCode, input::SendTaskSuccessInput,
        types::ExecutionStartedEventDetails,
    };
    let token = "secret-task-token";
    let input: SendTaskSuccessInput =
        serde_json::from_value(json!({"taskToken": token, "output": "secret-output"})).unwrap();
    let debug = format!("{input:?}");
    assert!(!debug.contains(token) && !debug.contains("secret-output"));
    let nested: ExecutionStartedEventDetails =
        serde_json::from_value(json!({"input":"secret-input"})).unwrap();
    let event = HistoryEvent::builder()
        .id(1)
        .timestamp(DateTime::from_timestamp_millis(0).unwrap())
        .r#type(HistoryEventType::ExecutionStarted)
        .execution_started_event_details(Some(nested))
        .build();
    assert!(!format!("{event:?}").contains("secret-input"));
    let mut error = StepFunctionsError::with_message(StepFunctionsErrorCode::InvalidToken, token);
    error.source = Some(Box::new(std::io::Error::other("secret-source")));
    assert_eq!(format!("{error:?}"), "StepFunctionsError(<redacted>)");
    assert_eq!(error.to_string(), "StepFunctionsError(<redacted>)");
    assert!(error.source().is_some());
}
#[test]
fn test_should_require_present_collections_and_keep_supported_operation_names_exact() {
    use rustack_stepfunctions_model::{
        StepFunctionsError, StepFunctionsErrorCode, StepFunctionsOperation, input::TagResourceInput,
    };
    assert!(serde_json::from_str::<TagResourceInput>(r#"{"resourceArn":"resource"}"#).is_err());
    assert!(
        serde_json::from_str::<TagResourceInput>(r#"{"resourceArn":"resource","tags":[]}"#).is_ok()
    );
    assert!(StepFunctionsOperation::from_name("StartExecution").is_some());
    assert_eq!(StepFunctionsOperation::from_name("startexecution"), None);
    assert_eq!(
        rustack_stepfunctions_model::TARGET_PREFIX,
        "AWSStepFunctions"
    );
    let error = StepFunctionsError::new(StepFunctionsErrorCode::UnsupportedOperation);
    assert_eq!(
        error.message,
        "Operation is outside the supported Step Functions profile"
    );
}
