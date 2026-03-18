//! Data model types for the EventBridge pattern matching engine.
//!
//! These types represent a parsed event pattern that is ready for matching
//! against JSON events. The pattern tree mirrors the structure of EventBridge
//! event patterns with typed condition variants for each supported operator.

use ipnet::IpNet;

/// A parsed event pattern, ready for matching.
#[derive(Debug, Clone)]
pub struct EventPattern {
    /// Top-level field matchers (AND-ed together).
    pub fields: Vec<FieldMatcher>,
    /// Explicit `$or` conditions: each inner `Vec` is OR-ed, elements within
    /// each inner `Vec` are AND-ed.
    pub or_conditions: Vec<Vec<FieldMatcher>>,
}

/// A matcher for a single field path.
#[derive(Debug, Clone)]
pub struct FieldMatcher {
    /// Field path segments from root (e.g., `["detail", "status"]`).
    pub path: Vec<String>,
    /// Match node: either a leaf with conditions or a nested object.
    pub node: PatternNode,
}

/// A node in the pattern tree.
#[derive(Debug, Clone)]
pub enum PatternNode {
    /// Leaf: array of conditions (OR-ed together).
    Leaf(Vec<MatchCondition>),
    /// Nested object: recurse into sub-fields (AND-ed).
    Object {
        /// Field matchers within this nested object.
        fields: Vec<FieldMatcher>,
        /// `$or` conditions within this nested object.
        or_conditions: Vec<Vec<FieldMatcher>>,
    },
}

/// A single match condition within a leaf array.
#[derive(Debug, Clone)]
pub enum MatchCondition {
    /// Exact string match.
    ExactString(String),
    /// Exact numeric match (stored as f64 for IEEE 754 comparison).
    ExactNumeric(f64),
    /// Exact null match.
    ExactNull,
    /// Prefix match.
    Prefix(String),
    /// Prefix match (case-insensitive).
    PrefixIgnoreCase(String),
    /// Suffix match.
    Suffix(String),
    /// Suffix match (case-insensitive).
    SuffixIgnoreCase(String),
    /// Equals-ignore-case.
    EqualsIgnoreCase(String),
    /// Wildcard (shell-style glob, `*` matches any sequence of characters).
    Wildcard(String),
    /// Anything-but: inverted match.
    AnythingBut(AnythingButCondition),
    /// Numeric comparison.
    Numeric(NumericCondition),
    /// Field existence check.
    Exists(bool),
    /// CIDR block match.
    Cidr(IpNet),
}

/// The inner condition for anything-but matching.
#[derive(Debug, Clone)]
pub enum AnythingButCondition {
    /// Not equal to any of these strings.
    Strings(Vec<String>),
    /// Not equal to any of these numbers.
    Numbers(Vec<f64>),
    /// Does not match prefix.
    Prefix(String),
    /// Does not match suffix.
    Suffix(String),
    /// Does not match (case-insensitive).
    EqualsIgnoreCase(String),
    /// Does not match any in list (case-insensitive).
    EqualsIgnoreCaseList(Vec<String>),
    /// Does not match wildcard pattern.
    Wildcard(String),
}

/// Numeric comparison condition.
///
/// Supports lower bound, upper bound, exact equality, or range (lower + upper).
#[derive(Debug, Clone)]
pub struct NumericCondition {
    /// Lower bound (inclusive or exclusive).
    pub lower: Option<NumericBound>,
    /// Upper bound (inclusive or exclusive).
    pub upper: Option<NumericBound>,
    /// Exact equality.
    pub equals: Option<f64>,
}

/// A numeric bound with inclusivity flag.
#[derive(Debug, Clone)]
pub struct NumericBound {
    /// The boundary value.
    pub value: f64,
    /// Whether the bound is inclusive (`<=` / `>=`) or exclusive (`<` / `>`).
    pub inclusive: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_construct_event_pattern() {
        let pattern = EventPattern {
            fields: vec![FieldMatcher {
                path: vec!["source".to_string()],
                node: PatternNode::Leaf(vec![MatchCondition::ExactString("my.app".to_string())]),
            }],
            or_conditions: vec![],
        };
        assert_eq!(pattern.fields.len(), 1);
        assert_eq!(pattern.fields[0].path, vec!["source"]);
    }

    #[test]
    fn test_should_construct_nested_pattern() {
        let pattern = EventPattern {
            fields: vec![FieldMatcher {
                path: vec!["detail".to_string()],
                node: PatternNode::Object {
                    fields: vec![FieldMatcher {
                        path: vec!["status".to_string()],
                        node: PatternNode::Leaf(vec![MatchCondition::ExactString(
                            "active".to_string(),
                        )]),
                    }],
                    or_conditions: vec![],
                },
            }],
            or_conditions: vec![],
        };

        if let PatternNode::Object { fields, .. } = &pattern.fields[0].node {
            assert_eq!(fields.len(), 1);
            assert_eq!(fields[0].path, vec!["status"]);
        } else {
            panic!("Expected Object node");
        }
    }

    #[test]
    fn test_should_construct_all_match_condition_variants() {
        let conditions: Vec<MatchCondition> = vec![
            MatchCondition::ExactString("hello".to_string()),
            MatchCondition::ExactNumeric(42.0),
            MatchCondition::ExactNull,
            MatchCondition::Prefix("pre".to_string()),
            MatchCondition::PrefixIgnoreCase("PRE".to_string()),
            MatchCondition::Suffix("suf".to_string()),
            MatchCondition::SuffixIgnoreCase("SUF".to_string()),
            MatchCondition::EqualsIgnoreCase("hello".to_string()),
            MatchCondition::Wildcard("*.txt".to_string()),
            MatchCondition::AnythingBut(AnythingButCondition::Strings(vec!["bad".to_string()])),
            MatchCondition::Numeric(NumericCondition {
                lower: Some(NumericBound {
                    value: 0.0,
                    inclusive: true,
                }),
                upper: None,
                equals: None,
            }),
            MatchCondition::Exists(true),
            MatchCondition::Cidr("10.0.0.0/24".parse().unwrap()),
        ];
        assert_eq!(conditions.len(), 13);
    }

    #[test]
    fn test_should_construct_all_anything_but_variants() {
        let variants: Vec<AnythingButCondition> = vec![
            AnythingButCondition::Strings(vec!["a".to_string()]),
            AnythingButCondition::Numbers(vec![1.0, 2.0]),
            AnythingButCondition::Prefix("pre".to_string()),
            AnythingButCondition::Suffix("suf".to_string()),
            AnythingButCondition::EqualsIgnoreCase("val".to_string()),
            AnythingButCondition::EqualsIgnoreCaseList(vec!["a".to_string(), "b".to_string()]),
            AnythingButCondition::Wildcard("*.tmp".to_string()),
        ];
        assert_eq!(variants.len(), 7);
    }
}
