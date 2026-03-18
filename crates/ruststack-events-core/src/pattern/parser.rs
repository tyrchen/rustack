//! Pattern parser that converts JSON event patterns into typed `EventPattern` structures.
//!
//! Handles all EventBridge pattern operators including prefix, suffix,
//! equals-ignore-case, wildcard, anything-but (all variants), numeric,
//! exists, and cidr. Also handles the `$or` logical combinator.

use serde_json::{Map, Value};

use super::value::{
    AnythingButCondition, EventPattern, FieldMatcher, MatchCondition, NumericBound,
    NumericCondition, PatternNode,
};

/// Errors that can occur during event pattern parsing.
#[derive(Debug, Clone, thiserror::Error)]
pub enum PatternParseError {
    /// The input string is not valid JSON.
    #[error("invalid JSON: {0}")]
    InvalidJson(String),
    /// The top-level value is not a JSON object.
    #[error("event pattern must be a JSON object")]
    NotAnObject,
    /// The `$or` key value is not an array.
    #[error("$or value must be an array")]
    OrNotArray,
    /// An item within `$or` array is not an object.
    #[error("each $or item must be a JSON object")]
    OrItemNotObject,
    /// A field value is not an array (leaf) or object (nested).
    #[error("field \"{field}\" value must be an array or object")]
    InvalidFieldValue {
        /// The field name.
        field: String,
    },
    /// An operator object has an unrecognized key.
    #[error("unknown operator: \"{operator}\"")]
    UnknownOperator {
        /// The operator name.
        operator: String,
    },
    /// An operator has an invalid value type.
    #[error("invalid value for operator \"{operator}\": {reason}")]
    InvalidOperatorValue {
        /// The operator name.
        operator: String,
        /// Description of why the value is invalid.
        reason: String,
    },
    /// A numeric condition has invalid syntax.
    #[error("invalid numeric condition: {0}")]
    InvalidNumericCondition(String),
    /// A wildcard pattern has consecutive unescaped `*` characters.
    #[error("wildcard pattern contains consecutive unescaped '*' characters")]
    ConsecutiveWildcardStars,
    /// A CIDR value could not be parsed.
    #[error("invalid CIDR: {0}")]
    InvalidCidr(String),
}

/// Parse a JSON event pattern string into an `EventPattern`.
///
/// # Errors
///
/// Returns `PatternParseError` if the pattern JSON is invalid or uses
/// unsupported syntax.
pub fn parse_event_pattern(pattern_json: &str) -> Result<EventPattern, PatternParseError> {
    let value: Value = serde_json::from_str(pattern_json)
        .map_err(|e| PatternParseError::InvalidJson(e.to_string()))?;

    let obj = value.as_object().ok_or(PatternParseError::NotAnObject)?;

    parse_object(obj)
}

fn parse_object(obj: &Map<String, Value>) -> Result<EventPattern, PatternParseError> {
    let mut fields = Vec::new();
    let mut or_conditions = Vec::new();

    for (key, value) in obj {
        if key == "$or" {
            let or_array = value.as_array().ok_or(PatternParseError::OrNotArray)?;
            for or_item in or_array {
                let or_obj = or_item
                    .as_object()
                    .ok_or(PatternParseError::OrItemNotObject)?;
                let sub_pattern = parse_object(or_obj)?;
                if sub_pattern.or_conditions.is_empty() {
                    // Simple case: no nested $or, push fields directly.
                    or_conditions.push(sub_pattern.fields);
                } else {
                    // Nested $or within $or: expand into multiple branches.
                    // Each nested $or branch is combined with the fields from
                    // this level to form a complete branch.
                    for nested_branch in &sub_pattern.or_conditions {
                        let mut combined = sub_pattern.fields.clone();
                        combined.extend(nested_branch.clone());
                        or_conditions.push(combined);
                    }
                }
            }
        } else {
            fields.push(parse_field(key, value)?);
        }
    }

    Ok(EventPattern {
        fields,
        or_conditions,
    })
}

fn parse_field(key: &str, value: &Value) -> Result<FieldMatcher, PatternParseError> {
    let path = vec![key.to_string()];
    let node = parse_node(key, value)?;
    Ok(FieldMatcher { path, node })
}

fn parse_node(field_name: &str, value: &Value) -> Result<PatternNode, PatternParseError> {
    match value {
        Value::Array(arr) => {
            let conditions = parse_conditions(arr)?;
            Ok(PatternNode::Leaf(conditions))
        }
        Value::Object(obj) => {
            let sub_pattern = parse_object(obj)?;
            Ok(PatternNode::Object {
                fields: sub_pattern.fields,
                or_conditions: sub_pattern.or_conditions,
            })
        }
        _ => Err(PatternParseError::InvalidFieldValue {
            field: field_name.to_string(),
        }),
    }
}

fn parse_conditions(arr: &[Value]) -> Result<Vec<MatchCondition>, PatternParseError> {
    let mut conditions = Vec::with_capacity(arr.len());
    for item in arr {
        conditions.push(parse_single_condition(item)?);
    }
    Ok(conditions)
}

fn parse_single_condition(value: &Value) -> Result<MatchCondition, PatternParseError> {
    match value {
        Value::String(s) => Ok(MatchCondition::ExactString(s.clone())),
        Value::Number(n) => {
            let f = n
                .as_f64()
                .ok_or_else(|| PatternParseError::InvalidOperatorValue {
                    operator: "exact-numeric".to_string(),
                    reason: "number cannot be represented as f64".to_string(),
                })?;
            Ok(MatchCondition::ExactNumeric(f))
        }
        Value::Null => Ok(MatchCondition::ExactNull),
        Value::Object(obj) => parse_operator_condition(obj),
        _ => Err(PatternParseError::InvalidOperatorValue {
            operator: "condition".to_string(),
            reason: "condition must be a string, number, null, or operator object".to_string(),
        }),
    }
}

fn parse_operator_condition(obj: &Map<String, Value>) -> Result<MatchCondition, PatternParseError> {
    if obj.len() != 1 {
        return Err(PatternParseError::InvalidOperatorValue {
            operator: "operator".to_string(),
            reason: "operator object must have exactly one key".to_string(),
        });
    }

    let (op_name, op_value) = obj.iter().next().expect("checked len == 1");

    match op_name.as_str() {
        "prefix" => parse_prefix_or_suffix(op_value, true),
        "suffix" => parse_prefix_or_suffix(op_value, false),
        "equals-ignore-case" => {
            let s = op_value
                .as_str()
                .ok_or_else(|| PatternParseError::InvalidOperatorValue {
                    operator: "equals-ignore-case".to_string(),
                    reason: "value must be a string".to_string(),
                })?;
            Ok(MatchCondition::EqualsIgnoreCase(s.to_string()))
        }
        "wildcard" => {
            let s = op_value
                .as_str()
                .ok_or_else(|| PatternParseError::InvalidOperatorValue {
                    operator: "wildcard".to_string(),
                    reason: "value must be a string".to_string(),
                })?;
            validate_wildcard_pattern(s)?;
            Ok(MatchCondition::Wildcard(s.to_string()))
        }
        "anything-but" => parse_anything_but(op_value),
        "numeric" => parse_numeric(op_value),
        "exists" => {
            let b = op_value
                .as_bool()
                .ok_or_else(|| PatternParseError::InvalidOperatorValue {
                    operator: "exists".to_string(),
                    reason: "value must be a boolean".to_string(),
                })?;
            Ok(MatchCondition::Exists(b))
        }
        "cidr" => {
            let s = op_value
                .as_str()
                .ok_or_else(|| PatternParseError::InvalidOperatorValue {
                    operator: "cidr".to_string(),
                    reason: "value must be a string".to_string(),
                })?;
            let net: ipnet::IpNet = s.parse().map_err(|e: ipnet::AddrParseError| {
                PatternParseError::InvalidCidr(e.to_string())
            })?;
            Ok(MatchCondition::Cidr(net))
        }
        other => Err(PatternParseError::UnknownOperator {
            operator: other.to_string(),
        }),
    }
}

/// Parse `prefix` or `suffix` operator. Value can be:
/// - A string: normal prefix/suffix
/// - An object with `equals-ignore-case` key: case-insensitive variant
fn parse_prefix_or_suffix(
    value: &Value,
    is_prefix: bool,
) -> Result<MatchCondition, PatternParseError> {
    let op_name = if is_prefix { "prefix" } else { "suffix" };
    match value {
        Value::String(s) => {
            if is_prefix {
                Ok(MatchCondition::Prefix(s.clone()))
            } else {
                Ok(MatchCondition::Suffix(s.clone()))
            }
        }
        Value::Object(obj) => {
            if let Some(ic_value) = obj.get("equals-ignore-case") {
                let s =
                    ic_value
                        .as_str()
                        .ok_or_else(|| PatternParseError::InvalidOperatorValue {
                            operator: op_name.to_string(),
                            reason: "equals-ignore-case value must be a string".to_string(),
                        })?;
                if is_prefix {
                    Ok(MatchCondition::PrefixIgnoreCase(s.to_string()))
                } else {
                    Ok(MatchCondition::SuffixIgnoreCase(s.to_string()))
                }
            } else {
                Err(PatternParseError::InvalidOperatorValue {
                    operator: op_name.to_string(),
                    reason: "object value must have 'equals-ignore-case' key".to_string(),
                })
            }
        }
        _ => Err(PatternParseError::InvalidOperatorValue {
            operator: op_name.to_string(),
            reason: "value must be a string or object with 'equals-ignore-case'".to_string(),
        }),
    }
}

/// Parse `anything-but` operator. Value can be:
/// - A string: not equal to that string
/// - A number: not equal to that number
/// - An array of strings or numbers
/// - An object with prefix/suffix/equals-ignore-case/wildcard
fn parse_anything_but(value: &Value) -> Result<MatchCondition, PatternParseError> {
    match value {
        Value::String(s) => Ok(MatchCondition::AnythingBut(AnythingButCondition::Strings(
            vec![s.clone()],
        ))),
        Value::Number(n) => {
            let f = n
                .as_f64()
                .ok_or_else(|| PatternParseError::InvalidOperatorValue {
                    operator: "anything-but".to_string(),
                    reason: "number cannot be represented as f64".to_string(),
                })?;
            Ok(MatchCondition::AnythingBut(AnythingButCondition::Numbers(
                vec![f],
            )))
        }
        Value::Array(arr) => parse_anything_but_array(arr),
        Value::Object(obj) => parse_anything_but_object(obj),
        _ => Err(PatternParseError::InvalidOperatorValue {
            operator: "anything-but".to_string(),
            reason: "value must be a string, number, array, or object".to_string(),
        }),
    }
}

fn parse_anything_but_array(arr: &[Value]) -> Result<MatchCondition, PatternParseError> {
    if arr.is_empty() {
        return Err(PatternParseError::InvalidOperatorValue {
            operator: "anything-but".to_string(),
            reason: "array must not be empty".to_string(),
        });
    }

    // Determine type from first element
    let first = &arr[0];
    if first.is_string() {
        let mut strings = Vec::with_capacity(arr.len());
        for item in arr {
            let s = item
                .as_str()
                .ok_or_else(|| PatternParseError::InvalidOperatorValue {
                    operator: "anything-but".to_string(),
                    reason: "all array elements must be the same type (expected string)"
                        .to_string(),
                })?;
            strings.push(s.to_string());
        }
        Ok(MatchCondition::AnythingBut(AnythingButCondition::Strings(
            strings,
        )))
    } else if first.is_number() {
        let mut numbers = Vec::with_capacity(arr.len());
        for item in arr {
            let n = item
                .as_f64()
                .ok_or_else(|| PatternParseError::InvalidOperatorValue {
                    operator: "anything-but".to_string(),
                    reason: "all array elements must be the same type (expected number)"
                        .to_string(),
                })?;
            numbers.push(n);
        }
        Ok(MatchCondition::AnythingBut(AnythingButCondition::Numbers(
            numbers,
        )))
    } else {
        Err(PatternParseError::InvalidOperatorValue {
            operator: "anything-but".to_string(),
            reason: "array elements must be strings or numbers".to_string(),
        })
    }
}

fn parse_anything_but_object(
    obj: &Map<String, Value>,
) -> Result<MatchCondition, PatternParseError> {
    if obj.len() != 1 {
        return Err(PatternParseError::InvalidOperatorValue {
            operator: "anything-but".to_string(),
            reason: "inner object must have exactly one key".to_string(),
        });
    }

    let (key, value) = obj.iter().next().expect("checked len == 1");

    match key.as_str() {
        "prefix" => {
            let s = value
                .as_str()
                .ok_or_else(|| PatternParseError::InvalidOperatorValue {
                    operator: "anything-but.prefix".to_string(),
                    reason: "value must be a string".to_string(),
                })?;
            Ok(MatchCondition::AnythingBut(AnythingButCondition::Prefix(
                s.to_string(),
            )))
        }
        "suffix" => {
            let s = value
                .as_str()
                .ok_or_else(|| PatternParseError::InvalidOperatorValue {
                    operator: "anything-but.suffix".to_string(),
                    reason: "value must be a string".to_string(),
                })?;
            Ok(MatchCondition::AnythingBut(AnythingButCondition::Suffix(
                s.to_string(),
            )))
        }
        "equals-ignore-case" => match value {
            Value::String(s) => Ok(MatchCondition::AnythingBut(
                AnythingButCondition::EqualsIgnoreCase(s.clone()),
            )),
            Value::Array(arr) => {
                let mut strings = Vec::with_capacity(arr.len());
                for item in arr {
                    let s =
                        item.as_str()
                            .ok_or_else(|| PatternParseError::InvalidOperatorValue {
                                operator: "anything-but.equals-ignore-case".to_string(),
                                reason: "all array elements must be strings".to_string(),
                            })?;
                    strings.push(s.to_string());
                }
                Ok(MatchCondition::AnythingBut(
                    AnythingButCondition::EqualsIgnoreCaseList(strings),
                ))
            }
            _ => Err(PatternParseError::InvalidOperatorValue {
                operator: "anything-but.equals-ignore-case".to_string(),
                reason: "value must be a string or array of strings".to_string(),
            }),
        },
        "wildcard" => {
            let s = value
                .as_str()
                .ok_or_else(|| PatternParseError::InvalidOperatorValue {
                    operator: "anything-but.wildcard".to_string(),
                    reason: "value must be a string".to_string(),
                })?;
            validate_wildcard_pattern(s)?;
            Ok(MatchCondition::AnythingBut(AnythingButCondition::Wildcard(
                s.to_string(),
            )))
        }
        other => Err(PatternParseError::UnknownOperator {
            operator: format!("anything-but.{other}"),
        }),
    }
}

/// Parse `numeric` operator. Value is an array of alternating operator strings
/// and numbers. Valid operators: `<`, `<=`, `>`, `>=`, `=`.
///
/// Examples:
/// - `[">", 100]` -> lower bound > 100
/// - `[">=", 0, "<", 100]` -> range [0, 100)
/// - `["=", 42]` -> exact equality
fn parse_numeric(value: &Value) -> Result<MatchCondition, PatternParseError> {
    let arr = value.as_array().ok_or_else(|| {
        PatternParseError::InvalidNumericCondition("numeric value must be an array".to_string())
    })?;

    if arr.is_empty() || arr.len() % 2 != 0 || arr.len() > 4 {
        return Err(PatternParseError::InvalidNumericCondition(
            "numeric array must have 2 or 4 elements (operator-value pairs)".to_string(),
        ));
    }

    let mut condition = NumericCondition {
        lower: None,
        upper: None,
        equals: None,
    };

    let mut i = 0;
    while i < arr.len() {
        let op = arr[i].as_str().ok_or_else(|| {
            PatternParseError::InvalidNumericCondition("operator must be a string".to_string())
        })?;
        let num = arr[i + 1].as_f64().ok_or_else(|| {
            PatternParseError::InvalidNumericCondition("operand must be a number".to_string())
        })?;

        match op {
            ">" => {
                condition.lower = Some(NumericBound {
                    value: num,
                    inclusive: false,
                });
            }
            ">=" => {
                condition.lower = Some(NumericBound {
                    value: num,
                    inclusive: true,
                });
            }
            "<" => {
                condition.upper = Some(NumericBound {
                    value: num,
                    inclusive: false,
                });
            }
            "<=" => {
                condition.upper = Some(NumericBound {
                    value: num,
                    inclusive: true,
                });
            }
            "=" => {
                condition.equals = Some(num);
            }
            other => {
                return Err(PatternParseError::InvalidNumericCondition(format!(
                    "unknown numeric operator: \"{other}\""
                )));
            }
        }
        i += 2;
    }

    Ok(MatchCondition::Numeric(condition))
}

/// Validate that a wildcard pattern does not contain consecutive unescaped `*`.
fn validate_wildcard_pattern(pattern: &str) -> Result<(), PatternParseError> {
    let mut prev_was_star = false;
    let mut chars = pattern.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            // Escaped character: skip the next char
            chars.next();
            prev_was_star = false;
        } else if ch == '*' {
            if prev_was_star {
                return Err(PatternParseError::ConsecutiveWildcardStars);
            }
            prev_was_star = true;
        } else {
            prev_was_star = false;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_parse_exact_string() {
        let pattern = parse_event_pattern(r#"{"source": ["my.app"]}"#).unwrap();
        assert_eq!(pattern.fields.len(), 1);
        assert_eq!(pattern.fields[0].path, vec!["source"]);
        if let PatternNode::Leaf(ref conds) = pattern.fields[0].node {
            assert_eq!(conds.len(), 1);
            assert!(matches!(&conds[0], MatchCondition::ExactString(s) if s == "my.app"));
        } else {
            panic!("Expected Leaf node");
        }
    }

    #[test]
    fn test_should_parse_exact_numeric() {
        let pattern = parse_event_pattern(r#"{"count": [42]}"#).unwrap();
        if let PatternNode::Leaf(ref conds) = pattern.fields[0].node {
            assert!(
                matches!(&conds[0], MatchCondition::ExactNumeric(n) if (*n - 42.0).abs() < f64::EPSILON)
            );
        } else {
            panic!("Expected Leaf node");
        }
    }

    #[test]
    fn test_should_parse_exact_null() {
        let pattern = parse_event_pattern(r#"{"value": [null]}"#).unwrap();
        if let PatternNode::Leaf(ref conds) = pattern.fields[0].node {
            assert!(matches!(&conds[0], MatchCondition::ExactNull));
        } else {
            panic!("Expected Leaf node");
        }
    }

    #[test]
    fn test_should_parse_prefix() {
        let pattern = parse_event_pattern(r#"{"source": [{"prefix": "my."}]}"#).unwrap();
        if let PatternNode::Leaf(ref conds) = pattern.fields[0].node {
            assert!(matches!(&conds[0], MatchCondition::Prefix(s) if s == "my."));
        } else {
            panic!("Expected Leaf node");
        }
    }

    #[test]
    fn test_should_parse_prefix_ignore_case() {
        let pattern =
            parse_event_pattern(r#"{"source": [{"prefix": {"equals-ignore-case": "MY."}}]}"#)
                .unwrap();
        if let PatternNode::Leaf(ref conds) = pattern.fields[0].node {
            assert!(matches!(&conds[0], MatchCondition::PrefixIgnoreCase(s) if s == "MY."));
        } else {
            panic!("Expected Leaf node");
        }
    }

    #[test]
    fn test_should_parse_suffix() {
        let pattern = parse_event_pattern(r#"{"file": [{"suffix": ".png"}]}"#).unwrap();
        if let PatternNode::Leaf(ref conds) = pattern.fields[0].node {
            assert!(matches!(&conds[0], MatchCondition::Suffix(s) if s == ".png"));
        } else {
            panic!("Expected Leaf node");
        }
    }

    #[test]
    fn test_should_parse_suffix_ignore_case() {
        let pattern =
            parse_event_pattern(r#"{"file": [{"suffix": {"equals-ignore-case": ".PNG"}}]}"#)
                .unwrap();
        if let PatternNode::Leaf(ref conds) = pattern.fields[0].node {
            assert!(matches!(&conds[0], MatchCondition::SuffixIgnoreCase(s) if s == ".PNG"));
        } else {
            panic!("Expected Leaf node");
        }
    }

    #[test]
    fn test_should_parse_equals_ignore_case() {
        let pattern =
            parse_event_pattern(r#"{"name": [{"equals-ignore-case": "alice"}]}"#).unwrap();
        if let PatternNode::Leaf(ref conds) = pattern.fields[0].node {
            assert!(matches!(&conds[0], MatchCondition::EqualsIgnoreCase(s) if s == "alice"));
        } else {
            panic!("Expected Leaf node");
        }
    }

    #[test]
    fn test_should_parse_wildcard() {
        let pattern = parse_event_pattern(r#"{"path": [{"wildcard": "dir/*.png"}]}"#).unwrap();
        if let PatternNode::Leaf(ref conds) = pattern.fields[0].node {
            assert!(matches!(&conds[0], MatchCondition::Wildcard(s) if s == "dir/*.png"));
        } else {
            panic!("Expected Leaf node");
        }
    }

    #[test]
    fn test_should_reject_consecutive_wildcard_stars() {
        let result = parse_event_pattern(r#"{"path": [{"wildcard": "dir/**.png"}]}"#);
        assert!(matches!(
            result,
            Err(PatternParseError::ConsecutiveWildcardStars)
        ));
    }

    #[test]
    fn test_should_allow_escaped_consecutive_stars() {
        // `\**` means literal `*` followed by wildcard `*` -- not consecutive unescaped
        let result = parse_event_pattern(r#"{"path": [{"wildcard": "dir/\\**.png"}]}"#);
        assert!(result.is_ok());
    }

    #[test]
    fn test_should_parse_anything_but_string() {
        let pattern =
            parse_event_pattern(r#"{"status": [{"anything-but": "cancelled"}]}"#).unwrap();
        if let PatternNode::Leaf(ref conds) = pattern.fields[0].node {
            assert!(
                matches!(&conds[0], MatchCondition::AnythingBut(AnythingButCondition::Strings(v)) if v == &["cancelled"])
            );
        } else {
            panic!("Expected Leaf node");
        }
    }

    #[test]
    fn test_should_parse_anything_but_number() {
        let pattern = parse_event_pattern(r#"{"code": [{"anything-but": 404}]}"#).unwrap();
        if let PatternNode::Leaf(ref conds) = pattern.fields[0].node {
            assert!(
                matches!(&conds[0], MatchCondition::AnythingBut(AnythingButCondition::Numbers(v)) if v.len() == 1 && (v[0] - 404.0).abs() < f64::EPSILON)
            );
        } else {
            panic!("Expected Leaf node");
        }
    }

    #[test]
    fn test_should_parse_anything_but_string_list() {
        let pattern =
            parse_event_pattern(r#"{"status": [{"anything-but": ["cancelled", "failed"]}]}"#)
                .unwrap();
        if let PatternNode::Leaf(ref conds) = pattern.fields[0].node {
            assert!(
                matches!(&conds[0], MatchCondition::AnythingBut(AnythingButCondition::Strings(v)) if v == &["cancelled", "failed"])
            );
        } else {
            panic!("Expected Leaf node");
        }
    }

    #[test]
    fn test_should_parse_anything_but_prefix() {
        let pattern =
            parse_event_pattern(r#"{"source": [{"anything-but": {"prefix": "init"}}]}"#).unwrap();
        if let PatternNode::Leaf(ref conds) = pattern.fields[0].node {
            assert!(
                matches!(&conds[0], MatchCondition::AnythingBut(AnythingButCondition::Prefix(s)) if s == "init")
            );
        } else {
            panic!("Expected Leaf node");
        }
    }

    #[test]
    fn test_should_parse_anything_but_suffix() {
        let pattern =
            parse_event_pattern(r#"{"file": [{"anything-but": {"suffix": ".tmp"}}]}"#).unwrap();
        if let PatternNode::Leaf(ref conds) = pattern.fields[0].node {
            assert!(
                matches!(&conds[0], MatchCondition::AnythingBut(AnythingButCondition::Suffix(s)) if s == ".tmp")
            );
        } else {
            panic!("Expected Leaf node");
        }
    }

    #[test]
    fn test_should_parse_anything_but_equals_ignore_case() {
        let pattern =
            parse_event_pattern(r#"{"name": [{"anything-but": {"equals-ignore-case": "admin"}}]}"#)
                .unwrap();
        if let PatternNode::Leaf(ref conds) = pattern.fields[0].node {
            assert!(
                matches!(&conds[0], MatchCondition::AnythingBut(AnythingButCondition::EqualsIgnoreCase(s)) if s == "admin")
            );
        } else {
            panic!("Expected Leaf node");
        }
    }

    #[test]
    fn test_should_parse_anything_but_equals_ignore_case_list() {
        let pattern = parse_event_pattern(
            r#"{"name": [{"anything-but": {"equals-ignore-case": ["admin", "root"]}}]}"#,
        )
        .unwrap();
        if let PatternNode::Leaf(ref conds) = pattern.fields[0].node {
            assert!(
                matches!(&conds[0], MatchCondition::AnythingBut(AnythingButCondition::EqualsIgnoreCaseList(v)) if v == &["admin", "root"])
            );
        } else {
            panic!("Expected Leaf node");
        }
    }

    #[test]
    fn test_should_parse_anything_but_wildcard() {
        let pattern =
            parse_event_pattern(r#"{"path": [{"anything-but": {"wildcard": "*/lib/*"}}]}"#)
                .unwrap();
        if let PatternNode::Leaf(ref conds) = pattern.fields[0].node {
            assert!(
                matches!(&conds[0], MatchCondition::AnythingBut(AnythingButCondition::Wildcard(s)) if s == "*/lib/*")
            );
        } else {
            panic!("Expected Leaf node");
        }
    }

    #[test]
    fn test_should_parse_numeric_greater_than() {
        let pattern = parse_event_pattern(r#"{"amount": [{"numeric": [">", 100]}]}"#).unwrap();
        if let PatternNode::Leaf(ref conds) = pattern.fields[0].node {
            if let MatchCondition::Numeric(ref nc) = conds[0] {
                assert!(nc.lower.is_some());
                let lb = nc.lower.as_ref().unwrap();
                assert!((lb.value - 100.0).abs() < f64::EPSILON);
                assert!(!lb.inclusive);
                assert!(nc.upper.is_none());
                assert!(nc.equals.is_none());
            } else {
                panic!("Expected Numeric condition");
            }
        } else {
            panic!("Expected Leaf node");
        }
    }

    #[test]
    fn test_should_parse_numeric_range() {
        let pattern =
            parse_event_pattern(r#"{"amount": [{"numeric": [">=", 10, "<", 100]}]}"#).unwrap();
        if let PatternNode::Leaf(ref conds) = pattern.fields[0].node {
            if let MatchCondition::Numeric(ref nc) = conds[0] {
                let lb = nc.lower.as_ref().unwrap();
                assert!((lb.value - 10.0).abs() < f64::EPSILON);
                assert!(lb.inclusive);
                let ub = nc.upper.as_ref().unwrap();
                assert!((ub.value - 100.0).abs() < f64::EPSILON);
                assert!(!ub.inclusive);
            } else {
                panic!("Expected Numeric condition");
            }
        } else {
            panic!("Expected Leaf node");
        }
    }

    #[test]
    fn test_should_parse_numeric_equals() {
        let pattern = parse_event_pattern(r#"{"count": [{"numeric": ["=", 42]}]}"#).unwrap();
        if let PatternNode::Leaf(ref conds) = pattern.fields[0].node {
            if let MatchCondition::Numeric(ref nc) = conds[0] {
                assert!(nc.equals.is_some());
                assert!((nc.equals.unwrap() - 42.0).abs() < f64::EPSILON);
            } else {
                panic!("Expected Numeric condition");
            }
        } else {
            panic!("Expected Leaf node");
        }
    }

    #[test]
    fn test_should_parse_exists_true() {
        let pattern = parse_event_pattern(r#"{"field": [{"exists": true}]}"#).unwrap();
        if let PatternNode::Leaf(ref conds) = pattern.fields[0].node {
            assert!(matches!(&conds[0], MatchCondition::Exists(true)));
        } else {
            panic!("Expected Leaf node");
        }
    }

    #[test]
    fn test_should_parse_exists_false() {
        let pattern = parse_event_pattern(r#"{"field": [{"exists": false}]}"#).unwrap();
        if let PatternNode::Leaf(ref conds) = pattern.fields[0].node {
            assert!(matches!(&conds[0], MatchCondition::Exists(false)));
        } else {
            panic!("Expected Leaf node");
        }
    }

    #[test]
    fn test_should_parse_cidr() {
        let pattern = parse_event_pattern(r#"{"ip": [{"cidr": "10.0.0.0/24"}]}"#).unwrap();
        if let PatternNode::Leaf(ref conds) = pattern.fields[0].node {
            assert!(matches!(&conds[0], MatchCondition::Cidr(_)));
        } else {
            panic!("Expected Leaf node");
        }
    }

    #[test]
    fn test_should_parse_nested_pattern() {
        let pattern = parse_event_pattern(
            r#"{"detail": {"status": ["active"], "region": [{"prefix": "us-"}]}}"#,
        )
        .unwrap();
        assert_eq!(pattern.fields.len(), 1);
        if let PatternNode::Object { ref fields, .. } = pattern.fields[0].node {
            assert_eq!(fields.len(), 2);
        } else {
            panic!("Expected Object node");
        }
    }

    #[test]
    fn test_should_parse_or_conditions() {
        let pattern =
            parse_event_pattern(r#"{"$or": [{"source": ["a"]}, {"source": ["b"]}]}"#).unwrap();
        assert!(pattern.fields.is_empty());
        assert_eq!(pattern.or_conditions.len(), 2);
    }

    #[test]
    fn test_should_parse_mixed_fields_and_or() {
        let pattern = parse_event_pattern(
            r#"{"source": ["my.app"], "$or": [{"detail-type": ["A"]}, {"detail-type": ["B"]}]}"#,
        )
        .unwrap();
        assert_eq!(pattern.fields.len(), 1);
        assert_eq!(pattern.or_conditions.len(), 2);
    }

    #[test]
    fn test_should_reject_invalid_json() {
        let result = parse_event_pattern("not json");
        assert!(matches!(result, Err(PatternParseError::InvalidJson(_))));
    }

    #[test]
    fn test_should_reject_non_object() {
        let result = parse_event_pattern("[1, 2, 3]");
        assert!(matches!(result, Err(PatternParseError::NotAnObject)));
    }

    #[test]
    fn test_should_reject_or_not_array() {
        let result = parse_event_pattern(r#"{"$or": "bad"}"#);
        assert!(matches!(result, Err(PatternParseError::OrNotArray)));
    }

    #[test]
    fn test_should_reject_or_item_not_object() {
        let result = parse_event_pattern(r#"{"$or": ["bad"]}"#);
        assert!(matches!(result, Err(PatternParseError::OrItemNotObject)));
    }

    #[test]
    fn test_should_reject_unknown_operator() {
        let result = parse_event_pattern(r#"{"field": [{"unknown-op": "val"}]}"#);
        assert!(matches!(
            result,
            Err(PatternParseError::UnknownOperator { .. })
        ));
    }

    #[test]
    fn test_should_reject_invalid_numeric() {
        let result = parse_event_pattern(r#"{"field": [{"numeric": [">", 1, "<"]}]}"#);
        assert!(matches!(
            result,
            Err(PatternParseError::InvalidNumericCondition(_))
        ));
    }

    #[test]
    fn test_should_reject_invalid_cidr() {
        let result = parse_event_pattern(r#"{"ip": [{"cidr": "not-a-cidr"}]}"#);
        assert!(matches!(result, Err(PatternParseError::InvalidCidr(_))));
    }

    #[test]
    fn test_should_parse_multiple_conditions_in_array() {
        let pattern = parse_event_pattern(r#"{"type": ["OrderPlaced", "OrderUpdated"]}"#).unwrap();
        if let PatternNode::Leaf(ref conds) = pattern.fields[0].node {
            assert_eq!(conds.len(), 2);
        } else {
            panic!("Expected Leaf node");
        }
    }

    #[test]
    fn test_should_parse_complex_pattern() {
        let pattern = parse_event_pattern(
            r#"{
                "source": ["my.app"],
                "detail-type": ["OrderPlaced", "OrderUpdated"],
                "detail": {
                    "amount": [{"numeric": [">", 100]}],
                    "status": [{"anything-but": "cancelled"}],
                    "region": [{"prefix": "us-"}]
                }
            }"#,
        )
        .unwrap();
        assert_eq!(pattern.fields.len(), 3);
    }

    #[test]
    fn test_should_reject_field_with_invalid_value_type() {
        let result = parse_event_pattern(r#"{"field": true}"#);
        assert!(matches!(
            result,
            Err(PatternParseError::InvalidFieldValue { .. })
        ));
    }

    #[test]
    fn test_should_reject_anything_but_empty_array() {
        let result = parse_event_pattern(r#"{"field": [{"anything-but": []}]}"#);
        assert!(matches!(
            result,
            Err(PatternParseError::InvalidOperatorValue { .. })
        ));
    }
}
