//! Filter policy evaluation engine for SNS subscription message filtering.
//!
//! Supports the following filter operators:
//!
//! - **Exact match**: `["value1", "value2"]` or `[123]`
//! - **Prefix**: `[{"prefix": "order."}]`
//! - **Suffix**: `[{"suffix": ".created"}]`
//! - **Numeric**: `[{"numeric": [">=", 100, "<=", 200]}]`
//! - **Exists**: `[{"exists": true}]` / `[{"exists": false}]`
//! - **Anything-but**: `[{"anything-but": "value"}]` or `[{"anything-but": ["a","b"]}]`
//!
//! Policy semantics: AND across keys, OR across conditions within a key.

use std::collections::HashMap;

use ruststack_sns_model::error::SnsError;
use ruststack_sns_model::types::MessageAttributeValue;

use crate::subscription::FilterPolicyScope;

/// Evaluate a filter policy against message attributes or message body.
///
/// Returns `true` if the message matches the filter policy (should be delivered),
/// or `false` if it should be filtered out.
///
/// # Errors
///
/// Returns an `SnsError` if the filter policy JSON is malformed or the
/// message body cannot be parsed when `scope` is `MessageBody`.
pub fn evaluate_filter_policy<S: ::std::hash::BuildHasher>(
    filter_json: &str,
    scope: &FilterPolicyScope,
    message_attributes: &HashMap<String, MessageAttributeValue, S>,
    message_body: &str,
) -> Result<bool, SnsError> {
    let policy: serde_json::Value = serde_json::from_str(filter_json)
        .map_err(|_| SnsError::invalid_parameter("Invalid filter policy: failed to parse JSON"))?;

    let policy_map = policy.as_object().ok_or_else(|| {
        SnsError::invalid_parameter("Invalid filter policy: must be a JSON object")
    })?;

    match scope {
        FilterPolicyScope::MessageAttributes => {
            evaluate_against_attributes(policy_map, message_attributes)
        }
        FilterPolicyScope::MessageBody => {
            let body_value: serde_json::Value =
                serde_json::from_str(message_body).map_err(|_| {
                    SnsError::invalid_parameter(
                        "Message body is not valid JSON for MessageBody filter scope",
                    )
                })?;

            let body_map = body_value.as_object().ok_or_else(|| {
                SnsError::invalid_parameter(
                    "Message body must be a JSON object for MessageBody filter scope",
                )
            })?;

            evaluate_against_body(policy_map, body_map)
        }
    }
}

/// Evaluate the filter policy against message attributes.
///
/// AND across keys: every key in the policy must match.
/// OR across conditions: at least one condition per key must match.
fn evaluate_against_attributes<S: ::std::hash::BuildHasher>(
    policy: &serde_json::Map<String, serde_json::Value>,
    attributes: &HashMap<String, MessageAttributeValue, S>,
) -> Result<bool, SnsError> {
    for (key, conditions) in policy {
        let conditions_array = conditions.as_array().ok_or_else(|| {
            SnsError::invalid_parameter(format!(
                "Invalid filter policy: conditions for key '{key}' must be an array"
            ))
        })?;

        let attr = attributes.get(key);
        if !evaluate_conditions_for_attribute(conditions_array, attr)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Evaluate the filter policy against a parsed message body.
fn evaluate_against_body(
    policy: &serde_json::Map<String, serde_json::Value>,
    body: &serde_json::Map<String, serde_json::Value>,
) -> Result<bool, SnsError> {
    for (key, conditions) in policy {
        let conditions_array = conditions.as_array().ok_or_else(|| {
            SnsError::invalid_parameter(format!(
                "Invalid filter policy: conditions for key '{key}' must be an array"
            ))
        })?;

        let body_value = body.get(key);
        if !evaluate_conditions_for_body_value(conditions_array, body_value)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Evaluate conditions for a single attribute key (OR semantics).
fn evaluate_conditions_for_attribute(
    conditions: &[serde_json::Value],
    attr: Option<&MessageAttributeValue>,
) -> Result<bool, SnsError> {
    for condition in conditions {
        if evaluate_single_condition_attribute(condition, attr)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Evaluate a single condition against a message attribute.
fn evaluate_single_condition_attribute(
    condition: &serde_json::Value,
    attr: Option<&MessageAttributeValue>,
) -> Result<bool, SnsError> {
    match condition {
        // Exact string match: "value"
        serde_json::Value::String(expected) => {
            let Some(attr) = attr else {
                return Ok(false);
            };
            let actual = attr.string_value.as_deref().unwrap_or("");
            Ok(actual == expected)
        }
        // Exact numeric match: 123 or 1.5
        serde_json::Value::Number(expected) => {
            let Some(attr) = attr else {
                return Ok(false);
            };
            let Some(ref attr_val) = attr.string_value else {
                return Ok(false);
            };
            let actual: f64 = attr_val.parse().unwrap_or(f64::NAN);
            let expected_f64 = expected.as_f64().unwrap_or(f64::NAN);
            Ok((actual - expected_f64).abs() < f64::EPSILON)
        }
        // Object condition: prefix, suffix, numeric, exists, anything-but
        serde_json::Value::Object(obj) => evaluate_object_condition_attribute(obj, attr),
        _ => Ok(false),
    }
}

/// Evaluate an object-based condition against a message attribute.
fn evaluate_object_condition_attribute(
    obj: &serde_json::Map<String, serde_json::Value>,
    attr: Option<&MessageAttributeValue>,
) -> Result<bool, SnsError> {
    if let Some(serde_json::Value::Bool(exists)) = obj.get("exists") {
        let is_present = attr.is_some();
        return Ok(is_present == *exists);
    }

    // All other operators require the attribute to be present.
    let Some(attr) = attr else {
        return Ok(false);
    };
    let attr_str = attr.string_value.as_deref().unwrap_or("");

    if let Some(prefix_val) = obj.get("prefix") {
        let prefix = prefix_val.as_str().unwrap_or("");
        return Ok(attr_str.starts_with(prefix));
    }

    if let Some(suffix_val) = obj.get("suffix") {
        let suffix = suffix_val.as_str().unwrap_or("");
        return Ok(attr_str.ends_with(suffix));
    }

    if let Some(numeric_val) = obj.get("numeric") {
        return evaluate_numeric_condition(numeric_val, attr_str);
    }

    if let Some(anything_but_val) = obj.get("anything-but") {
        return evaluate_anything_but(anything_but_val, attr_str);
    }

    Ok(false)
}

/// Evaluate conditions for a single body key (OR semantics).
fn evaluate_conditions_for_body_value(
    conditions: &[serde_json::Value],
    body_value: Option<&serde_json::Value>,
) -> Result<bool, SnsError> {
    for condition in conditions {
        if evaluate_single_condition_body(condition, body_value)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Evaluate a single condition against a body field value.
fn evaluate_single_condition_body(
    condition: &serde_json::Value,
    body_value: Option<&serde_json::Value>,
) -> Result<bool, SnsError> {
    match condition {
        // Exact string match
        serde_json::Value::String(expected) => {
            let Some(val) = body_value else {
                return Ok(false);
            };
            match val {
                serde_json::Value::String(s) => Ok(s == expected),
                _ => Ok(false),
            }
        }
        // Exact numeric match
        serde_json::Value::Number(expected) => {
            let Some(val) = body_value else {
                return Ok(false);
            };
            match val {
                serde_json::Value::Number(n) => {
                    let actual = n.as_f64().unwrap_or(f64::NAN);
                    let expected_f64 = expected.as_f64().unwrap_or(f64::NAN);
                    Ok((actual - expected_f64).abs() < f64::EPSILON)
                }
                _ => Ok(false),
            }
        }
        // Object condition
        serde_json::Value::Object(obj) => evaluate_object_condition_body(obj, body_value),
        _ => Ok(false),
    }
}

/// Evaluate an object-based condition against a body field value.
fn evaluate_object_condition_body(
    obj: &serde_json::Map<String, serde_json::Value>,
    body_value: Option<&serde_json::Value>,
) -> Result<bool, SnsError> {
    if let Some(serde_json::Value::Bool(exists)) = obj.get("exists") {
        let is_present = body_value.is_some();
        return Ok(is_present == *exists);
    }

    let Some(val) = body_value else {
        return Ok(false);
    };

    if let Some(prefix_val) = obj.get("prefix") {
        let prefix = prefix_val.as_str().unwrap_or("");
        return match val {
            serde_json::Value::String(s) => Ok(s.starts_with(prefix)),
            _ => Ok(false),
        };
    }

    if let Some(suffix_val) = obj.get("suffix") {
        let suffix = suffix_val.as_str().unwrap_or("");
        return match val {
            serde_json::Value::String(s) => Ok(s.ends_with(suffix)),
            _ => Ok(false),
        };
    }

    if let Some(numeric_val) = obj.get("numeric") {
        return match val {
            serde_json::Value::Number(n) => {
                let val_str = n.to_string();
                evaluate_numeric_condition(numeric_val, &val_str)
            }
            serde_json::Value::String(s) => evaluate_numeric_condition(numeric_val, s),
            _ => Ok(false),
        };
    }

    if let Some(anything_but_val) = obj.get("anything-but") {
        return match val {
            serde_json::Value::String(s) => evaluate_anything_but(anything_but_val, s),
            serde_json::Value::Number(n) => {
                let val_str = n.to_string();
                evaluate_anything_but(anything_but_val, &val_str)
            }
            _ => Ok(false),
        };
    }

    Ok(false)
}

/// Evaluate a numeric condition: `[">", 100]` or `[">=", 100, "<=", 200]`.
fn evaluate_numeric_condition(
    numeric_spec: &serde_json::Value,
    value_str: &str,
) -> Result<bool, SnsError> {
    let arr = numeric_spec.as_array().ok_or_else(|| {
        SnsError::invalid_parameter("Invalid filter policy: 'numeric' must be an array")
    })?;

    let actual: f64 = value_str.parse().unwrap_or(f64::NAN);
    if actual.is_nan() {
        return Ok(false);
    }

    let mut i = 0;
    while i < arr.len() {
        let Some(op) = arr[i].as_str() else {
            return Err(SnsError::invalid_parameter(
                "Invalid filter policy: numeric operator must be a string",
            ));
        };

        i += 1;
        if i >= arr.len() {
            return Err(SnsError::invalid_parameter(
                "Invalid filter policy: numeric operator missing operand",
            ));
        }

        let threshold = arr[i].as_f64().ok_or_else(|| {
            SnsError::invalid_parameter("Invalid filter policy: numeric operand must be a number")
        })?;

        let passes = match op {
            "=" => (actual - threshold).abs() < f64::EPSILON,
            ">" => actual > threshold,
            ">=" => actual >= threshold - f64::EPSILON,
            "<" => actual < threshold,
            "<=" => actual <= threshold + f64::EPSILON,
            _ => {
                return Err(SnsError::invalid_parameter(format!(
                    "Invalid filter policy: unknown numeric operator '{op}'"
                )));
            }
        };

        if !passes {
            return Ok(false);
        }

        i += 1;
    }

    Ok(true)
}

/// Evaluate an `anything-but` condition.
///
/// `anything-but` can be a single string, a single number, or an array of strings/numbers.
fn evaluate_anything_but(
    anything_but_val: &serde_json::Value,
    actual: &str,
) -> Result<bool, SnsError> {
    match anything_but_val {
        serde_json::Value::String(s) => Ok(actual != s),
        serde_json::Value::Number(n) => {
            let n_str = n.to_string();
            Ok(actual != n_str)
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                match item {
                    serde_json::Value::String(s) if actual == s => return Ok(false),
                    serde_json::Value::Number(n) if actual == n.to_string() => return Ok(false),
                    _ => {}
                }
            }
            Ok(true)
        }
        _ => Err(SnsError::invalid_parameter(
            "Invalid filter policy: 'anything-but' value must be a string, number, or array",
        )),
    }
}

/// Resolve the effective message for a specific protocol from a `MessageStructure=json` message.
///
/// When `message_structure` is `"json"`, the message body is a JSON object with
/// protocol-specific keys. This function resolves the message for the given protocol,
/// falling back to the `"default"` key.
///
/// # Errors
///
/// Returns an `SnsError` if the message is not valid JSON or missing the `"default"` key.
pub fn resolve_protocol_message(message: &str, protocol: &str) -> Result<String, SnsError> {
    let parsed: serde_json::Value = serde_json::from_str(message).map_err(|_| {
        SnsError::invalid_parameter(
            "Invalid parameter: Message Reason: \
             When MessageStructure is 'json', the message must be valid JSON",
        )
    })?;

    let obj = parsed.as_object().ok_or_else(|| {
        SnsError::invalid_parameter(
            "Invalid parameter: Message Reason: \
             When MessageStructure is 'json', the message must be a JSON object",
        )
    })?;

    // Try protocol-specific key first, then fall back to "default".
    if let Some(serde_json::Value::String(s)) = obj.get(protocol) {
        return Ok(s.clone());
    }

    obj.get("default")
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| {
            SnsError::invalid_parameter(
                "Invalid parameter: Message Reason: \
                 When MessageStructure is 'json', the message must contain a 'default' key",
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_attr(data_type: &str, string_value: &str) -> MessageAttributeValue {
        MessageAttributeValue {
            data_type: data_type.to_owned(),
            string_value: Some(string_value.to_owned()),
            binary_value: None,
        }
    }

    fn make_attrs(pairs: &[(&str, &str, &str)]) -> HashMap<String, MessageAttributeValue> {
        pairs
            .iter()
            .map(|(name, dt, val)| ((*name).to_owned(), make_attr(dt, val)))
            .collect()
    }

    // -- Exact match tests --

    #[test]
    fn test_should_match_exact_string() {
        let policy = r#"{"color": ["red", "blue"]}"#;
        let attrs = make_attrs(&[("color", "String", "red")]);
        let result =
            evaluate_filter_policy(policy, &FilterPolicyScope::MessageAttributes, &attrs, "")
                .unwrap();
        assert!(result);
    }

    #[test]
    fn test_should_not_match_exact_string() {
        let policy = r#"{"color": ["red", "blue"]}"#;
        let attrs = make_attrs(&[("color", "String", "green")]);
        let result =
            evaluate_filter_policy(policy, &FilterPolicyScope::MessageAttributes, &attrs, "")
                .unwrap();
        assert!(!result);
    }

    #[test]
    fn test_should_not_match_missing_attribute() {
        let policy = r#"{"color": ["red"]}"#;
        let attrs = HashMap::new();
        let result =
            evaluate_filter_policy(policy, &FilterPolicyScope::MessageAttributes, &attrs, "")
                .unwrap();
        assert!(!result);
    }

    // -- Numeric match tests --

    #[test]
    fn test_should_match_exact_numeric() {
        let policy = r#"{"price": [100]}"#;
        let attrs = make_attrs(&[("price", "Number", "100")]);
        let result =
            evaluate_filter_policy(policy, &FilterPolicyScope::MessageAttributes, &attrs, "")
                .unwrap();
        assert!(result);
    }

    #[test]
    fn test_should_match_numeric_range() {
        let policy = r#"{"price": [{"numeric": [">=", 100, "<=", 200]}]}"#;
        let attrs = make_attrs(&[("price", "Number", "150")]);
        let result =
            evaluate_filter_policy(policy, &FilterPolicyScope::MessageAttributes, &attrs, "")
                .unwrap();
        assert!(result);
    }

    #[test]
    fn test_should_not_match_numeric_range_out_of_bounds() {
        let policy = r#"{"price": [{"numeric": [">=", 100, "<=", 200]}]}"#;
        let attrs = make_attrs(&[("price", "Number", "250")]);
        let result =
            evaluate_filter_policy(policy, &FilterPolicyScope::MessageAttributes, &attrs, "")
                .unwrap();
        assert!(!result);
    }

    #[test]
    fn test_should_match_numeric_greater_than() {
        let policy = r#"{"price": [{"numeric": [">", 0]}]}"#;
        let attrs = make_attrs(&[("price", "Number", "42")]);
        let result =
            evaluate_filter_policy(policy, &FilterPolicyScope::MessageAttributes, &attrs, "")
                .unwrap();
        assert!(result);
    }

    // -- Prefix/suffix tests --

    #[test]
    fn test_should_match_prefix() {
        let policy = r#"{"event": [{"prefix": "order."}]}"#;
        let attrs = make_attrs(&[("event", "String", "order.created")]);
        let result =
            evaluate_filter_policy(policy, &FilterPolicyScope::MessageAttributes, &attrs, "")
                .unwrap();
        assert!(result);
    }

    #[test]
    fn test_should_not_match_prefix() {
        let policy = r#"{"event": [{"prefix": "order."}]}"#;
        let attrs = make_attrs(&[("event", "String", "payment.created")]);
        let result =
            evaluate_filter_policy(policy, &FilterPolicyScope::MessageAttributes, &attrs, "")
                .unwrap();
        assert!(!result);
    }

    #[test]
    fn test_should_match_suffix() {
        let policy = r#"{"event": [{"suffix": ".created"}]}"#;
        let attrs = make_attrs(&[("event", "String", "order.created")]);
        let result =
            evaluate_filter_policy(policy, &FilterPolicyScope::MessageAttributes, &attrs, "")
                .unwrap();
        assert!(result);
    }

    // -- Exists tests --

    #[test]
    fn test_should_match_exists_true() {
        let policy = r#"{"color": [{"exists": true}]}"#;
        let attrs = make_attrs(&[("color", "String", "red")]);
        let result =
            evaluate_filter_policy(policy, &FilterPolicyScope::MessageAttributes, &attrs, "")
                .unwrap();
        assert!(result);
    }

    #[test]
    fn test_should_match_exists_false() {
        let policy = r#"{"color": [{"exists": false}]}"#;
        let attrs = HashMap::new();
        let result =
            evaluate_filter_policy(policy, &FilterPolicyScope::MessageAttributes, &attrs, "")
                .unwrap();
        assert!(result);
    }

    #[test]
    fn test_should_not_match_exists_true_when_missing() {
        let policy = r#"{"color": [{"exists": true}]}"#;
        let attrs = HashMap::new();
        let result =
            evaluate_filter_policy(policy, &FilterPolicyScope::MessageAttributes, &attrs, "")
                .unwrap();
        assert!(!result);
    }

    // -- Anything-but tests --

    #[test]
    fn test_should_match_anything_but_string() {
        let policy = r#"{"color": [{"anything-but": "red"}]}"#;
        let attrs = make_attrs(&[("color", "String", "blue")]);
        let result =
            evaluate_filter_policy(policy, &FilterPolicyScope::MessageAttributes, &attrs, "")
                .unwrap();
        assert!(result);
    }

    #[test]
    fn test_should_not_match_anything_but_string() {
        let policy = r#"{"color": [{"anything-but": "red"}]}"#;
        let attrs = make_attrs(&[("color", "String", "red")]);
        let result =
            evaluate_filter_policy(policy, &FilterPolicyScope::MessageAttributes, &attrs, "")
                .unwrap();
        assert!(!result);
    }

    #[test]
    fn test_should_match_anything_but_array() {
        let policy = r#"{"color": [{"anything-but": ["red", "blue"]}]}"#;
        let attrs = make_attrs(&[("color", "String", "green")]);
        let result =
            evaluate_filter_policy(policy, &FilterPolicyScope::MessageAttributes, &attrs, "")
                .unwrap();
        assert!(result);
    }

    #[test]
    fn test_should_not_match_anything_but_array() {
        let policy = r#"{"color": [{"anything-but": ["red", "blue"]}]}"#;
        let attrs = make_attrs(&[("color", "String", "red")]);
        let result =
            evaluate_filter_policy(policy, &FilterPolicyScope::MessageAttributes, &attrs, "")
                .unwrap();
        assert!(!result);
    }

    // -- AND across keys --

    #[test]
    fn test_should_require_all_keys_to_match() {
        let policy = r#"{"color": ["red"], "size": ["large"]}"#;
        let attrs = make_attrs(&[("color", "String", "red"), ("size", "String", "small")]);
        let result =
            evaluate_filter_policy(policy, &FilterPolicyScope::MessageAttributes, &attrs, "")
                .unwrap();
        assert!(!result);
    }

    #[test]
    fn test_should_match_all_keys() {
        let policy = r#"{"color": ["red"], "size": ["large"]}"#;
        let attrs = make_attrs(&[("color", "String", "red"), ("size", "String", "large")]);
        let result =
            evaluate_filter_policy(policy, &FilterPolicyScope::MessageAttributes, &attrs, "")
                .unwrap();
        assert!(result);
    }

    // -- MessageBody scope tests --

    #[test]
    fn test_should_match_body_exact_string() {
        let policy = r#"{"color": ["red"]}"#;
        let body = r#"{"color": "red"}"#;
        let result = evaluate_filter_policy(
            policy,
            &FilterPolicyScope::MessageBody,
            &HashMap::new(),
            body,
        )
        .unwrap();
        assert!(result);
    }

    #[test]
    fn test_should_not_match_body_wrong_value() {
        let policy = r#"{"color": ["red"]}"#;
        let body = r#"{"color": "green"}"#;
        let result = evaluate_filter_policy(
            policy,
            &FilterPolicyScope::MessageBody,
            &HashMap::new(),
            body,
        )
        .unwrap();
        assert!(!result);
    }

    #[test]
    fn test_should_match_body_numeric() {
        let policy = r#"{"price": [{"numeric": [">=", 100]}]}"#;
        let body = r#"{"price": 150}"#;
        let result = evaluate_filter_policy(
            policy,
            &FilterPolicyScope::MessageBody,
            &HashMap::new(),
            body,
        )
        .unwrap();
        assert!(result);
    }

    #[test]
    fn test_should_match_body_exists_true() {
        let policy = r#"{"color": [{"exists": true}]}"#;
        let body = r#"{"color": "red"}"#;
        let result = evaluate_filter_policy(
            policy,
            &FilterPolicyScope::MessageBody,
            &HashMap::new(),
            body,
        )
        .unwrap();
        assert!(result);
    }

    #[test]
    fn test_should_match_body_exists_false() {
        let policy = r#"{"color": [{"exists": false}]}"#;
        let body = r#"{"other": "value"}"#;
        let result = evaluate_filter_policy(
            policy,
            &FilterPolicyScope::MessageBody,
            &HashMap::new(),
            body,
        )
        .unwrap();
        assert!(result);
    }

    // -- resolve_protocol_message tests --

    #[test]
    fn test_should_resolve_protocol_specific_message() {
        let message = r#"{"default": "fallback", "sqs": "sqs-specific"}"#;
        let result = resolve_protocol_message(message, "sqs").unwrap();
        assert_eq!(result, "sqs-specific");
    }

    #[test]
    fn test_should_fall_back_to_default() {
        let message = r#"{"default": "fallback", "sqs": "sqs-specific"}"#;
        let result = resolve_protocol_message(message, "http").unwrap();
        assert_eq!(result, "fallback");
    }

    #[test]
    fn test_should_error_on_missing_default() {
        let message = r#"{"sqs": "sqs-specific"}"#;
        let result = resolve_protocol_message(message, "http");
        assert!(result.is_err());
    }

    #[test]
    fn test_should_error_on_invalid_json() {
        let result = resolve_protocol_message("not-json", "sqs");
        assert!(result.is_err());
    }

    // -- Empty policy --

    #[test]
    fn test_should_match_empty_policy() {
        let policy = "{}";
        let attrs = make_attrs(&[("color", "String", "red")]);
        let result =
            evaluate_filter_policy(policy, &FilterPolicyScope::MessageAttributes, &attrs, "")
                .unwrap();
        assert!(result);
    }
}
