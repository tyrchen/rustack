//! Event pattern matching engine.
//!
//! Evaluates a parsed `EventPattern` against a JSON event. The engine
//! implements AND semantics for multiple fields, OR within condition arrays,
//! and OR for `$or` groups. Array event fields are handled by matching if
//! any element satisfies the condition.

use serde_json::Value;

use super::operators::match_single_value;
use super::value::{EventPattern, FieldMatcher, MatchCondition, PatternNode};

/// Match an event pattern against a JSON event.
///
/// Returns `true` if the event satisfies all conditions in the pattern.
///
/// # Matching semantics
///
/// - Multiple field matchers at the same level are AND-ed.
/// - Multiple conditions within a leaf array are OR-ed.
/// - `$or` groups are OR-ed: at least one group must match.
/// - Within each `$or` group, field matchers are AND-ed.
/// - Array event fields match if any element satisfies the condition.
#[must_use]
pub fn matches(pattern: &EventPattern, event: &Value) -> bool {
    match_pattern_fields(&pattern.fields, &pattern.or_conditions, event)
}

/// Match a set of field matchers and `$or` conditions against an event.
fn match_pattern_fields(
    fields: &[FieldMatcher],
    or_conditions: &[Vec<FieldMatcher>],
    event: &Value,
) -> bool {
    // All field matchers must match (AND semantics)
    for matcher in fields {
        if !match_field(matcher, event) {
            return false;
        }
    }

    // If there are $or conditions, at least one group must match
    if !or_conditions.is_empty() {
        let any_or_matches = or_conditions
            .iter()
            .any(|group| group.iter().all(|matcher| match_field(matcher, event)));
        if !any_or_matches {
            return false;
        }
    }

    true
}

/// Match a single field matcher against an event.
fn match_field(matcher: &FieldMatcher, event: &Value) -> bool {
    match &matcher.node {
        PatternNode::Leaf(conditions) => {
            let field_value = navigate_path(event, &matcher.path);
            match_leaf(conditions, field_value)
        }
        PatternNode::Object {
            fields,
            or_conditions,
        } => {
            // Navigate to the nested object first, then match sub-fields
            let nested = navigate_path(event, &matcher.path);
            match nested {
                Some(nested_val) => match_pattern_fields(fields, or_conditions, nested_val),
                None => {
                    // If the nested object doesn't exist, check if all conditions
                    // are `exists: false` (which would match absent fields).
                    // Otherwise, the match fails since we can't navigate further.
                    all_fields_are_exists_false(fields) && or_conditions.is_empty()
                }
            }
        }
    }
}

/// Check if all field matchers in a list are `exists: false` leaves.
fn all_fields_are_exists_false(fields: &[FieldMatcher]) -> bool {
    fields.iter().all(|f| match &f.node {
        PatternNode::Leaf(conditions) => {
            conditions.len() == 1 && matches!(&conditions[0], MatchCondition::Exists(false))
        }
        PatternNode::Object {
            fields,
            or_conditions,
        } => all_fields_are_exists_false(fields) && or_conditions.is_empty(),
    })
}

/// Navigate a dot-path in the event JSON, returning the value at the end.
///
/// Returns `None` if any segment along the path is missing.
fn navigate_path<'a>(event: &'a Value, path: &[String]) -> Option<&'a Value> {
    let mut current = event;
    for segment in path {
        match current.get(segment.as_str()) {
            Some(next) => current = next,
            None => return None,
        }
    }
    Some(current)
}

/// Match a set of leaf conditions against a field value.
///
/// Conditions are OR-ed: at least one must match.
/// If the field is absent (`None`), only `Exists(false)` matches.
/// If the field value is an array, any element matching any condition is a match.
fn match_leaf(conditions: &[MatchCondition], field_value: Option<&Value>) -> bool {
    // Handle exists conditions specially
    let has_exists = conditions
        .iter()
        .any(|c| matches!(c, MatchCondition::Exists(_)));

    if has_exists {
        // For exists conditions, check field presence
        return conditions.iter().any(|c| match c {
            MatchCondition::Exists(expected) => {
                let is_present = field_value.is_some();
                *expected == is_present
            }
            // Non-exists conditions can also be in the same array (OR-ed)
            other => match field_value {
                Some(val) => match_value_or_array(other, val),
                None => false,
            },
        });
    }

    // For non-exists conditions, field must be present
    let Some(val) = field_value else {
        return false;
    };

    // OR semantics: any condition matching is a match
    conditions.iter().any(|c| match_value_or_array(c, val))
}

/// Match a condition against a value, handling arrays by checking any element.
fn match_value_or_array(condition: &MatchCondition, value: &Value) -> bool {
    if let Value::Array(arr) = value {
        // For array fields, any element matching is a match
        arr.iter().any(|elem| match_single_value(condition, elem))
    } else {
        match_single_value(condition, value)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::pattern::parse_event_pattern;

    /// Helper to check pattern matching.
    fn assert_matches(pattern_json: &str, event: &Value) {
        let pattern = parse_event_pattern(pattern_json).unwrap();
        assert!(
            matches(&pattern, event),
            "Expected pattern to match event.\nPattern: {pattern_json}\nEvent: {event}",
        );
    }

    /// Helper to check pattern non-matching.
    fn assert_no_match(pattern_json: &str, event: &Value) {
        let pattern = parse_event_pattern(pattern_json).unwrap();
        assert!(
            !matches(&pattern, event),
            "Expected pattern NOT to match event.\nPattern: {pattern_json}\nEvent: {event}",
        );
    }

    // -- Exact match tests --

    #[test]
    fn test_should_match_exact_string() {
        assert_matches(r#"{"source": ["my.app"]}"#, &json!({"source": "my.app"}));
    }

    #[test]
    fn test_should_not_match_different_string() {
        assert_no_match(r#"{"source": ["my.app"]}"#, &json!({"source": "other.app"}));
    }

    #[test]
    fn test_should_match_exact_numeric() {
        assert_matches(r#"{"count": [42]}"#, &json!({"count": 42}));
    }

    #[test]
    fn test_should_match_exact_null() {
        assert_matches(r#"{"value": [null]}"#, &json!({"value": null}));
    }

    #[test]
    fn test_should_match_multiple_exact_values_or() {
        assert_matches(r#"{"type": ["A", "B"]}"#, &json!({"type": "B"}));
    }

    #[test]
    fn test_should_match_multiple_fields_and() {
        assert_matches(
            r#"{"source": ["my.app"], "type": ["OrderPlaced"]}"#,
            &json!({"source": "my.app", "type": "OrderPlaced"}),
        );
    }

    #[test]
    fn test_should_not_match_if_one_field_fails() {
        assert_no_match(
            r#"{"source": ["my.app"], "type": ["OrderPlaced"]}"#,
            &json!({"source": "my.app", "type": "OrderCancelled"}),
        );
    }

    // -- Prefix tests --

    #[test]
    fn test_should_match_prefix() {
        assert_matches(
            r#"{"region": [{"prefix": "us-"}]}"#,
            &json!({"region": "us-east-1"}),
        );
    }

    #[test]
    fn test_should_not_match_wrong_prefix() {
        assert_no_match(
            r#"{"region": [{"prefix": "us-"}]}"#,
            &json!({"region": "eu-west-1"}),
        );
    }

    #[test]
    fn test_should_match_prefix_ignore_case() {
        assert_matches(
            r#"{"region": [{"prefix": {"equals-ignore-case": "US-"}}]}"#,
            &json!({"region": "us-east-1"}),
        );
    }

    // -- Suffix tests --

    #[test]
    fn test_should_match_suffix() {
        assert_matches(
            r#"{"file": [{"suffix": ".png"}]}"#,
            &json!({"file": "image.png"}),
        );
    }

    #[test]
    fn test_should_match_suffix_ignore_case() {
        assert_matches(
            r#"{"file": [{"suffix": {"equals-ignore-case": ".PNG"}}]}"#,
            &json!({"file": "image.png"}),
        );
    }

    // -- Equals-ignore-case tests --

    #[test]
    fn test_should_match_equals_ignore_case() {
        assert_matches(
            r#"{"name": [{"equals-ignore-case": "alice"}]}"#,
            &json!({"name": "ALICE"}),
        );
    }

    // -- Wildcard tests --

    #[test]
    fn test_should_match_wildcard() {
        assert_matches(
            r#"{"path": [{"wildcard": "dir/*.png"}]}"#,
            &json!({"path": "dir/image.png"}),
        );
    }

    #[test]
    fn test_should_not_match_wildcard() {
        assert_no_match(
            r#"{"path": [{"wildcard": "dir/*.png"}]}"#,
            &json!({"path": "other/image.png"}),
        );
    }

    // -- Anything-but tests --

    #[test]
    fn test_should_match_anything_but_string() {
        assert_matches(
            r#"{"status": [{"anything-but": "cancelled"}]}"#,
            &json!({"status": "active"}),
        );
    }

    #[test]
    fn test_should_not_match_anything_but_string() {
        assert_no_match(
            r#"{"status": [{"anything-but": "cancelled"}]}"#,
            &json!({"status": "cancelled"}),
        );
    }

    #[test]
    fn test_should_match_anything_but_list() {
        assert_matches(
            r#"{"status": [{"anything-but": ["cancelled", "failed"]}]}"#,
            &json!({"status": "active"}),
        );
        assert_no_match(
            r#"{"status": [{"anything-but": ["cancelled", "failed"]}]}"#,
            &json!({"status": "failed"}),
        );
    }

    #[test]
    fn test_should_match_anything_but_prefix() {
        assert_matches(
            r#"{"source": [{"anything-but": {"prefix": "init"}}]}"#,
            &json!({"source": "complete"}),
        );
        assert_no_match(
            r#"{"source": [{"anything-but": {"prefix": "init"}}]}"#,
            &json!({"source": "initialize"}),
        );
    }

    #[test]
    fn test_should_match_anything_but_suffix() {
        assert_matches(
            r#"{"file": [{"anything-but": {"suffix": ".tmp"}}]}"#,
            &json!({"file": "data.txt"}),
        );
        assert_no_match(
            r#"{"file": [{"anything-but": {"suffix": ".tmp"}}]}"#,
            &json!({"file": "data.tmp"}),
        );
    }

    #[test]
    fn test_should_match_anything_but_wildcard() {
        assert_matches(
            r#"{"path": [{"anything-but": {"wildcard": "*/lib/*"}}]}"#,
            &json!({"path": "usr/bin/foo"}),
        );
        assert_no_match(
            r#"{"path": [{"anything-but": {"wildcard": "*/lib/*"}}]}"#,
            &json!({"path": "usr/lib/foo"}),
        );
    }

    // -- Numeric tests --

    #[test]
    fn test_should_match_numeric_greater_than() {
        assert_matches(
            r#"{"amount": [{"numeric": [">", 100]}]}"#,
            &json!({"amount": 150}),
        );
        assert_no_match(
            r#"{"amount": [{"numeric": [">", 100]}]}"#,
            &json!({"amount": 50}),
        );
    }

    #[test]
    fn test_should_match_numeric_range() {
        assert_matches(
            r#"{"amount": [{"numeric": [">=", 10, "<", 100]}]}"#,
            &json!({"amount": 50}),
        );
        assert_no_match(
            r#"{"amount": [{"numeric": [">=", 10, "<", 100]}]}"#,
            &json!({"amount": 100}),
        );
    }

    #[test]
    fn test_should_match_numeric_equals() {
        assert_matches(
            r#"{"count": [{"numeric": ["=", 42]}]}"#,
            &json!({"count": 42}),
        );
        assert_no_match(
            r#"{"count": [{"numeric": ["=", 42]}]}"#,
            &json!({"count": 43}),
        );
    }

    // -- Exists tests --

    #[test]
    fn test_should_match_exists_true() {
        assert_matches(
            r#"{"field": [{"exists": true}]}"#,
            &json!({"field": "any value"}),
        );
        assert_matches(r#"{"field": [{"exists": true}]}"#, &json!({"field": null}));
    }

    #[test]
    fn test_should_not_match_exists_true_when_absent() {
        assert_no_match(
            r#"{"field": [{"exists": true}]}"#,
            &json!({"other": "value"}),
        );
    }

    #[test]
    fn test_should_match_exists_false() {
        assert_matches(
            r#"{"field": [{"exists": false}]}"#,
            &json!({"other": "value"}),
        );
    }

    #[test]
    fn test_should_not_match_exists_false_when_present() {
        assert_no_match(
            r#"{"field": [{"exists": false}]}"#,
            &json!({"field": "present"}),
        );
    }

    // -- CIDR tests --

    #[test]
    fn test_should_match_cidr_ipv4() {
        assert_matches(
            r#"{"ip": [{"cidr": "10.0.0.0/24"}]}"#,
            &json!({"ip": "10.0.0.42"}),
        );
        assert_no_match(
            r#"{"ip": [{"cidr": "10.0.0.0/24"}]}"#,
            &json!({"ip": "10.0.1.1"}),
        );
    }

    #[test]
    fn test_should_match_cidr_ipv6() {
        assert_matches(
            r#"{"ip": [{"cidr": "2001:db8::/32"}]}"#,
            &json!({"ip": "2001:db8::1"}),
        );
        assert_no_match(
            r#"{"ip": [{"cidr": "2001:db8::/32"}]}"#,
            &json!({"ip": "2001:db9::1"}),
        );
    }

    // -- Nested field tests --

    #[test]
    fn test_should_match_nested_fields() {
        assert_matches(
            r#"{"detail": {"status": ["active"]}}"#,
            &json!({"detail": {"status": "active"}}),
        );
    }

    #[test]
    fn test_should_not_match_nested_field_wrong_value() {
        assert_no_match(
            r#"{"detail": {"status": ["active"]}}"#,
            &json!({"detail": {"status": "inactive"}}),
        );
    }

    #[test]
    fn test_should_not_match_missing_nested_object() {
        assert_no_match(
            r#"{"detail": {"status": ["active"]}}"#,
            &json!({"source": "my.app"}),
        );
    }

    #[test]
    fn test_should_match_deeply_nested() {
        assert_matches(
            r#"{"detail": {"order": {"amount": [{"numeric": [">", 100]}]}}}"#,
            &json!({"detail": {"order": {"amount": 150}}}),
        );
    }

    // -- Array field tests --

    #[test]
    fn test_should_match_array_field_any_element() {
        assert_matches(
            r#"{"tags": ["important"]}"#,
            &json!({"tags": ["normal", "important", "urgent"]}),
        );
    }

    #[test]
    fn test_should_not_match_array_field_no_element() {
        assert_no_match(
            r#"{"tags": ["critical"]}"#,
            &json!({"tags": ["normal", "important"]}),
        );
    }

    #[test]
    fn test_should_match_array_field_with_operator() {
        assert_matches(
            r#"{"values": [{"numeric": [">", 5]}]}"#,
            &json!({"values": [1, 3, 10]}),
        );
    }

    // -- $or tests --

    #[test]
    fn test_should_match_or_first_branch() {
        assert_matches(
            r#"{"$or": [{"source": ["a"]}, {"source": ["b"]}]}"#,
            &json!({"source": "a"}),
        );
    }

    #[test]
    fn test_should_match_or_second_branch() {
        assert_matches(
            r#"{"$or": [{"source": ["a"]}, {"source": ["b"]}]}"#,
            &json!({"source": "b"}),
        );
    }

    #[test]
    fn test_should_not_match_or_no_branch() {
        assert_no_match(
            r#"{"$or": [{"source": ["a"]}, {"source": ["b"]}]}"#,
            &json!({"source": "c"}),
        );
    }

    #[test]
    fn test_should_match_or_with_and_fields() {
        // source must be "my.app" AND (type = "A" OR type = "B")
        assert_matches(
            r#"{"source": ["my.app"], "$or": [{"type": ["A"]}, {"type": ["B"]}]}"#,
            &json!({"source": "my.app", "type": "A"}),
        );
        assert_matches(
            r#"{"source": ["my.app"], "$or": [{"type": ["A"]}, {"type": ["B"]}]}"#,
            &json!({"source": "my.app", "type": "B"}),
        );
        assert_no_match(
            r#"{"source": ["my.app"], "$or": [{"type": ["A"]}, {"type": ["B"]}]}"#,
            &json!({"source": "my.app", "type": "C"}),
        );
        assert_no_match(
            r#"{"source": ["my.app"], "$or": [{"type": ["A"]}, {"type": ["B"]}]}"#,
            &json!({"source": "other", "type": "A"}),
        );
    }

    // -- Complex pattern tests --

    #[test]
    fn test_should_match_complex_pattern() {
        let pattern = r#"{
            "source": ["my.app"],
            "detail-type": ["OrderPlaced", "OrderUpdated"],
            "detail": {
                "amount": [{"numeric": [">", 100]}],
                "status": [{"anything-but": "cancelled"}],
                "region": [{"prefix": "us-"}]
            }
        }"#;

        let event = json!({
            "source": "my.app",
            "detail-type": "OrderPlaced",
            "detail": {
                "amount": 150,
                "status": "active",
                "region": "us-east-1"
            }
        });

        assert_matches(pattern, &event);
    }

    #[test]
    fn test_should_not_match_complex_pattern_wrong_amount() {
        let pattern = r#"{
            "source": ["my.app"],
            "detail": {
                "amount": [{"numeric": [">", 100]}]
            }
        }"#;

        let event = json!({
            "source": "my.app",
            "detail": {
                "amount": 50
            }
        });

        assert_no_match(pattern, &event);
    }

    #[test]
    fn test_should_match_empty_pattern_any_event() {
        // An empty pattern matches any event
        assert_matches(r"{}", &json!({"anything": "goes"}));
    }

    #[test]
    fn test_should_match_event_with_extra_fields() {
        // Pattern only checks specified fields; extra fields are ignored
        assert_matches(
            r#"{"source": ["my.app"]}"#,
            &json!({"source": "my.app", "extra": "field", "more": 42}),
        );
    }

    #[test]
    fn test_should_not_match_missing_field() {
        assert_no_match(r#"{"source": ["my.app"]}"#, &json!({"other": "field"}));
    }

    #[test]
    fn test_should_match_exists_false_in_nested_absent_object() {
        assert_matches(
            r#"{"detail": {"missing_field": [{"exists": false}]}}"#,
            &json!({"detail": {"other": "value"}}),
        );
    }

    #[test]
    fn test_should_match_exists_false_when_parent_absent() {
        // When the parent object is absent, exists:false for sub-fields should match
        assert_matches(
            r#"{"detail": {"sub": [{"exists": false}]}}"#,
            &json!({"source": "my.app"}),
        );
    }

    #[test]
    fn test_should_match_empty_string() {
        assert_matches(r#"{"value": [""]}"#, &json!({"value": ""}));
        assert_no_match(r#"{"value": [""]}"#, &json!({"value": "notempty"}));
    }
}
