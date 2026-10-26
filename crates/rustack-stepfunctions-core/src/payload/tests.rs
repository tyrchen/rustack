use std::error::Error;

use rustack_core::json::JsonError;
use serde_json::{Value, json};

use crate::{AslError, EvaluationContext, MAX_PAYLOAD_BYTES, Path, Payload};

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn test_should_reject_borrowed_extreme_depth_without_recursive_owned_teardown() {
    let mut value = Value::Null;
    for _ in 0..50_000 {
        value = Value::Array(vec![value]);
    }
    let payload = Payload::from_value(&value);
    let context = EvaluationContext::from_value(&value);
    // The caller retains and safely disposes its own rejected tree.
    let mut owned = vec![value];
    while let Some(value) = owned.pop() {
        if let Value::Array(mut children) = value {
            owned.append(&mut children);
        }
    }
    assert!(matches!(payload, Err(AslError::Json(JsonError::Depth))));
    assert!(matches!(context, Err(AslError::Json(JsonError::Depth))));
}

#[test]
fn test_should_embed_maximum_payload_bytes_without_reducing_output_limit() -> TestResult {
    let payload = Payload::parse(format!("\"{}\"", "x".repeat(MAX_PAYLOAD_BYTES - 2)).as_bytes())?;
    let context = EvaluationContext::from_value(&json!({
        "Execution":{"Input":payload.value(),"Name":"run","Id":"execution-id"},
        "StateMachine":{"Name":"machine"},"State":{"Name":"current"}
    }))?;
    let path: Path = "$$.Execution.Input".parse()?;
    let result = path.select(&payload, &context)?.to_payload()?;
    assert_eq!(result.original_bytes().len(), MAX_PAYLOAD_BYTES);
    assert_eq!(result.value(), payload.value());
    assert!(matches!(
        "$$".parse::<Path>()?
            .select(&payload, &context)?
            .to_payload(),
        Err(AslError::DataLimit)
    ));
    assert_eq!(format!("{context:?}"), "EvaluationContext(<redacted>)");
    Ok(())
}

#[test]
fn test_should_embed_maximum_payload_depth_and_member_count_with_metadata() -> TestResult {
    let raw = format!("{}0{}", "[".repeat(64), "]".repeat(64));
    let deep = Payload::parse(raw.as_bytes())?;
    let context = EvaluationContext::from_value(&json!({"Execution":{"Input":deep.value()}}))?;
    let selected = "$$.Execution.Input"
        .parse::<Path>()?
        .select(&deep, &context)?
        .to_payload()?;
    assert_eq!(selected.value(), deep.value());
    assert!(matches!(
        "$$".parse::<Path>()?.select(&deep, &context)?.to_payload(),
        Err(AslError::Json(JsonError::Depth))
    ));
    let wide = Payload::from_value(&json!(vec![0; 65_536]))?;
    let context = EvaluationContext::from_value(&json!({
        "Execution":{"Input":wide.value()},
        "Map":{"Item":{"Value":wide.value(),"Index":0,"Source":"STATE_DATA"}}
    }))?;
    assert_eq!(
        "$$.Execution.Input"
            .parse::<Path>()?
            .select(&wide, &context)?
            .to_payload()?
            .value(),
        wide.value()
    );
    Ok(())
}

#[test]
fn test_should_enforce_independent_context_byte_depth_member_and_string_limits() {
    let value = json!(vec!["x".repeat(MAX_PAYLOAD_BYTES); 16]);
    assert!(matches!(
        EvaluationContext::from_value(&value),
        Err(AslError::DataLimit)
    ));
    let mut value = Value::Null;
    for _ in 0..68 {
        value = json!([value]);
    }
    assert!(EvaluationContext::from_value(&value).is_ok());
    assert!(matches!(
        EvaluationContext::from_value(&json!([value])),
        Err(AslError::Json(JsonError::Depth))
    ));
    assert!(matches!(
        EvaluationContext::from_value(&json!(vec![0; 131_585])),
        Err(AslError::Json(JsonError::Members))
    ));
    assert!(matches!(
        EvaluationContext::from_value(&json!("x".repeat(MAX_PAYLOAD_BYTES + 1))),
        Err(AslError::Json(JsonError::String))
    ));
}

#[test]
fn test_should_allow_numeric_normalization_expansion_inside_internal_context() -> TestResult {
    let input = format!("[{}]", vec!["1e9"; 65_535].join(","));
    assert_eq!(input.len(), 262_141);
    let input = Payload::parse(input.as_bytes())?;
    let item = Payload::parse(format!("\"{}\"", "x".repeat(MAX_PAYLOAD_BYTES - 2)).as_bytes())?;
    let context_value = json!({
        "Execution":{"Input":input.value()},
        "Map":{"Item":{"Value":item.value(),"Index":0,"Source":"STATE_DATA"}}
    });
    assert!(serde_json::to_vec(&context_value)?.len() > 1024 * 1024);
    let context = EvaluationContext::from_value(&context_value)?;
    assert_eq!(
        "$$.Map.Item.Value"
            .parse::<Path>()?
            .select(&input, &context)?
            .to_payload()?
            .value(),
        item.value()
    );
    EvaluationContext::from_value(&json!({
        "Execution":{"Input":input.value()},
        "Map":{"Item":{"Value":input.value(),"Index":0}}
    }))?;
    assert!(matches!(
        "$$.Execution.Input"
            .parse::<Path>()?
            .select(&item, &context)?
            .to_payload(),
        Err(AslError::DataLimit)
    ));
    Ok(())
}

#[test]
fn test_should_bound_exact_internal_context_encoding_bytes() -> TestResult {
    let mut parts = vec![Value::String("x".repeat(262_141)); 16];
    parts.pop();
    parts.push(Value::String("x".repeat(262_140)));
    let exact = Value::Array(parts);
    assert_eq!(serde_json::to_vec(&exact)?.len(), 4 * 1024 * 1024);
    EvaluationContext::from_value(&exact)?;
    let mut excess = exact;
    if let Some(Value::String(last)) = excess.as_array_mut().and_then(|array| array.last_mut()) {
        last.push('x');
    }
    assert!(matches!(
        EvaluationContext::from_value(&excess),
        Err(AslError::DataLimit)
    ));
    Ok(())
}
