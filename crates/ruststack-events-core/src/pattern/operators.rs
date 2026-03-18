//! Individual operator matching logic for EventBridge pattern conditions.
//!
//! Each operator implements its matching semantics against a `serde_json::Value`.
//! The `match_single_value` function dispatches to the appropriate operator.

use std::net::IpAddr;

use serde_json::Value;

use super::value::{AnythingButCondition, MatchCondition, NumericCondition};

/// A segment of a parsed wildcard pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WildcardSegment {
    /// A literal string that must be matched exactly.
    Literal(String),
    /// A `*` that matches any sequence of characters (including empty).
    Star,
}

/// Split a wildcard pattern into segments, handling `\*` as an escaped literal `*`.
pub fn split_wildcard_pattern(pattern: &str) -> Vec<WildcardSegment> {
    let mut segments = Vec::new();
    let mut current_literal = String::new();
    let mut chars = pattern.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            // Escaped character: the next character is literal
            if let Some(next) = chars.next() {
                current_literal.push(next);
            } else {
                // Trailing backslash: treat as literal
                current_literal.push('\\');
            }
        } else if ch == '*' {
            if !current_literal.is_empty() {
                segments.push(WildcardSegment::Literal(std::mem::take(
                    &mut current_literal,
                )));
            }
            segments.push(WildcardSegment::Star);
        } else {
            current_literal.push(ch);
        }
    }

    if !current_literal.is_empty() {
        segments.push(WildcardSegment::Literal(current_literal));
    }

    segments
}

/// Match a wildcard pattern against text.
///
/// The `*` character matches any sequence of characters (including empty).
/// Use `\*` to match a literal `*`.
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    let segments = split_wildcard_pattern(pattern);
    match_segments(&segments, text)
}

/// Recursively match wildcard segments against text.
fn match_segments(segments: &[WildcardSegment], text: &str) -> bool {
    if segments.is_empty() {
        return text.is_empty();
    }

    match &segments[0] {
        WildcardSegment::Literal(lit) => {
            if let Some(rest) = text.strip_prefix(lit.as_str()) {
                match_segments(&segments[1..], rest)
            } else {
                false
            }
        }
        WildcardSegment::Star => {
            // Star matches zero or more characters.
            // Try matching the remaining segments starting at each position.
            if segments.len() == 1 {
                // Trailing star matches everything
                return true;
            }

            let remaining = &segments[1..];
            // Try matching remaining segments at every position
            for i in 0..=text.len() {
                // Ensure we don't split a multi-byte char
                if text.is_char_boundary(i) && match_segments(remaining, &text[i..]) {
                    return true;
                }
            }
            false
        }
    }
}

/// Match an `anything-but` condition against a JSON value.
///
/// Returns `true` if the value does NOT match the condition (inverted match).
pub fn match_anything_but(ab: &AnythingButCondition, value: &Value) -> bool {
    match ab {
        AnythingButCondition::Strings(strings) => {
            if let Some(s) = value.as_str() {
                !strings.iter().any(|candidate| candidate == s)
            } else {
                // Non-string values never match string comparisons, so
                // "anything-but" is satisfied
                true
            }
        }
        AnythingButCondition::Numbers(numbers) => {
            if let Some(n) = value.as_f64() {
                !numbers
                    .iter()
                    .any(|candidate| (*candidate - n).abs() < f64::EPSILON)
            } else {
                true
            }
        }
        AnythingButCondition::Prefix(prefix) => {
            if let Some(s) = value.as_str() {
                !s.starts_with(prefix.as_str())
            } else {
                true
            }
        }
        AnythingButCondition::Suffix(suffix) => {
            if let Some(s) = value.as_str() {
                !s.ends_with(suffix.as_str())
            } else {
                true
            }
        }
        AnythingButCondition::EqualsIgnoreCase(target) => {
            if let Some(s) = value.as_str() {
                !s.eq_ignore_ascii_case(target)
            } else {
                true
            }
        }
        AnythingButCondition::EqualsIgnoreCaseList(targets) => {
            if let Some(s) = value.as_str() {
                !targets.iter().any(|t| s.eq_ignore_ascii_case(t))
            } else {
                true
            }
        }
        AnythingButCondition::Wildcard(pattern) => {
            if let Some(s) = value.as_str() {
                !wildcard_match(pattern, s)
            } else {
                true
            }
        }
    }
}

/// Match a numeric condition against a numeric value.
pub fn match_numeric(nc: &NumericCondition, value: f64) -> bool {
    if let Some(eq) = nc.equals {
        if (eq - value).abs() >= f64::EPSILON {
            return false;
        }
    }

    if let Some(ref lower) = nc.lower {
        if lower.inclusive {
            if value < lower.value {
                return false;
            }
        } else if value <= lower.value {
            return false;
        }
    }

    if let Some(ref upper) = nc.upper {
        if upper.inclusive {
            if value > upper.value {
                return false;
            }
        } else if value >= upper.value {
            return false;
        }
    }

    true
}

/// Match a single condition against a JSON value.
///
/// Dispatches to the appropriate operator logic based on the condition variant.
pub fn match_single_value(condition: &MatchCondition, value: &Value) -> bool {
    match condition {
        MatchCondition::ExactString(expected) => {
            value.as_str().is_some_and(|s| s == expected.as_str())
        }
        MatchCondition::ExactNumeric(expected) => value
            .as_f64()
            .is_some_and(|n| (n - expected).abs() < f64::EPSILON),
        MatchCondition::ExactNull => value.is_null(),
        MatchCondition::Prefix(prefix) => value
            .as_str()
            .is_some_and(|s| s.starts_with(prefix.as_str())),
        MatchCondition::PrefixIgnoreCase(prefix) => value
            .as_str()
            .is_some_and(|s| s.to_lowercase().starts_with(&prefix.to_lowercase())),
        MatchCondition::Suffix(suffix) => {
            value.as_str().is_some_and(|s| s.ends_with(suffix.as_str()))
        }
        MatchCondition::SuffixIgnoreCase(suffix) => value
            .as_str()
            .is_some_and(|s| s.to_lowercase().ends_with(&suffix.to_lowercase())),
        MatchCondition::EqualsIgnoreCase(expected) => value
            .as_str()
            .is_some_and(|s| s.eq_ignore_ascii_case(expected)),
        MatchCondition::Wildcard(pattern) => {
            value.as_str().is_some_and(|s| wildcard_match(pattern, s))
        }
        MatchCondition::AnythingBut(ab) => match_anything_but(ab, value),
        MatchCondition::Numeric(nc) => value.as_f64().is_some_and(|n| match_numeric(nc, n)),
        MatchCondition::Exists(_) => {
            // Exists is handled at the engine level, not here.
            // This should not be called for Exists conditions.
            // Return false as a safety measure.
            false
        }
        MatchCondition::Cidr(net) => value
            .as_str()
            .is_some_and(|s| s.parse::<IpAddr>().is_ok_and(|ip| net.contains(&ip))),
    }
}

#[cfg(test)]
mod tests {
    use ipnet::IpNet;
    use serde_json::json;

    use super::*;
    use crate::pattern::value::{NumericBound, NumericCondition};

    // -- Wildcard tests --

    #[test]
    fn test_should_match_wildcard_exact() {
        assert!(wildcard_match("hello", "hello"));
        assert!(!wildcard_match("hello", "world"));
    }

    #[test]
    fn test_should_match_wildcard_star_at_end() {
        assert!(wildcard_match("hello*", "hello"));
        assert!(wildcard_match("hello*", "hello world"));
        assert!(!wildcard_match("hello*", "hi"));
    }

    #[test]
    fn test_should_match_wildcard_star_at_start() {
        assert!(wildcard_match("*.png", "image.png"));
        assert!(wildcard_match("*.png", ".png"));
        assert!(!wildcard_match("*.png", "image.jpg"));
    }

    #[test]
    fn test_should_match_wildcard_star_in_middle() {
        assert!(wildcard_match("dir/*.png", "dir/image.png"));
        assert!(wildcard_match("dir/*.png", "dir/.png"));
        assert!(!wildcard_match("dir/*.png", "other/image.png"));
    }

    #[test]
    fn test_should_match_wildcard_multiple_stars() {
        assert!(wildcard_match("*/lib/*", "usr/lib/foo"));
        assert!(wildcard_match("*/lib/*", "/lib/"));
        assert!(!wildcard_match("*/lib/*", "usr/bin/foo"));
    }

    #[test]
    fn test_should_match_wildcard_star_only() {
        assert!(wildcard_match("*", ""));
        assert!(wildcard_match("*", "anything"));
    }

    #[test]
    fn test_should_match_wildcard_empty_pattern_empty_text() {
        assert!(wildcard_match("", ""));
        assert!(!wildcard_match("", "notempty"));
    }

    #[test]
    fn test_should_match_wildcard_escaped_star() {
        assert!(wildcard_match(r"hello\*world", "hello*world"));
        assert!(!wildcard_match(r"hello\*world", "helloXworld"));
    }

    #[test]
    fn test_should_match_wildcard_escaped_and_unescaped() {
        assert!(wildcard_match(r"\**", "*anything"));
        assert!(wildcard_match(r"\**", "*"));
        assert!(!wildcard_match(r"\**", "nope"));
    }

    #[test]
    fn test_should_split_wildcard_pattern() {
        let segments = split_wildcard_pattern("dir/*.png");
        assert_eq!(
            segments,
            vec![
                WildcardSegment::Literal("dir/".to_string()),
                WildcardSegment::Star,
                WildcardSegment::Literal(".png".to_string()),
            ]
        );
    }

    #[test]
    fn test_should_split_wildcard_with_escape() {
        let segments = split_wildcard_pattern(r"hello\*world");
        assert_eq!(
            segments,
            vec![WildcardSegment::Literal("hello*world".to_string())]
        );
    }

    // -- Numeric tests --

    #[test]
    fn test_should_match_numeric_greater_than() {
        let nc = NumericCondition {
            lower: Some(NumericBound {
                value: 100.0,
                inclusive: false,
            }),
            upper: None,
            equals: None,
        };
        assert!(match_numeric(&nc, 101.0));
        assert!(!match_numeric(&nc, 100.0));
        assert!(!match_numeric(&nc, 99.0));
    }

    #[test]
    fn test_should_match_numeric_greater_or_equal() {
        let nc = NumericCondition {
            lower: Some(NumericBound {
                value: 100.0,
                inclusive: true,
            }),
            upper: None,
            equals: None,
        };
        assert!(match_numeric(&nc, 100.0));
        assert!(match_numeric(&nc, 101.0));
        assert!(!match_numeric(&nc, 99.0));
    }

    #[test]
    fn test_should_match_numeric_less_than() {
        let nc = NumericCondition {
            lower: None,
            upper: Some(NumericBound {
                value: 50.0,
                inclusive: false,
            }),
            equals: None,
        };
        assert!(match_numeric(&nc, 49.0));
        assert!(!match_numeric(&nc, 50.0));
        assert!(!match_numeric(&nc, 51.0));
    }

    #[test]
    fn test_should_match_numeric_range() {
        let nc = NumericCondition {
            lower: Some(NumericBound {
                value: 10.0,
                inclusive: true,
            }),
            upper: Some(NumericBound {
                value: 20.0,
                inclusive: false,
            }),
            equals: None,
        };
        assert!(match_numeric(&nc, 10.0));
        assert!(match_numeric(&nc, 15.0));
        assert!(!match_numeric(&nc, 20.0));
        assert!(!match_numeric(&nc, 9.0));
    }

    #[test]
    fn test_should_match_numeric_equals() {
        let nc = NumericCondition {
            lower: None,
            upper: None,
            equals: Some(42.0),
        };
        assert!(match_numeric(&nc, 42.0));
        assert!(!match_numeric(&nc, 43.0));
    }

    // -- Anything-but tests --

    #[test]
    fn test_should_match_anything_but_strings() {
        let ab = AnythingButCondition::Strings(vec!["bad".to_string(), "worse".to_string()]);
        assert!(match_anything_but(&ab, &json!("good")));
        assert!(!match_anything_but(&ab, &json!("bad")));
        assert!(!match_anything_but(&ab, &json!("worse")));
        // Non-string values always pass
        assert!(match_anything_but(&ab, &json!(42)));
    }

    #[test]
    fn test_should_match_anything_but_numbers() {
        let ab = AnythingButCondition::Numbers(vec![404.0, 500.0]);
        assert!(match_anything_but(&ab, &json!(200)));
        assert!(!match_anything_but(&ab, &json!(404)));
        assert!(!match_anything_but(&ab, &json!(500)));
        // Non-number values always pass
        assert!(match_anything_but(&ab, &json!("text")));
    }

    #[test]
    fn test_should_match_anything_but_prefix() {
        let ab = AnythingButCondition::Prefix("init".to_string());
        assert!(match_anything_but(&ab, &json!("complete")));
        assert!(!match_anything_but(&ab, &json!("initialize")));
    }

    #[test]
    fn test_should_match_anything_but_suffix() {
        let ab = AnythingButCondition::Suffix(".tmp".to_string());
        assert!(match_anything_but(&ab, &json!("file.txt")));
        assert!(!match_anything_but(&ab, &json!("file.tmp")));
    }

    #[test]
    fn test_should_match_anything_but_equals_ignore_case() {
        let ab = AnythingButCondition::EqualsIgnoreCase("admin".to_string());
        assert!(match_anything_but(&ab, &json!("user")));
        assert!(!match_anything_but(&ab, &json!("Admin")));
        assert!(!match_anything_but(&ab, &json!("ADMIN")));
    }

    #[test]
    fn test_should_match_anything_but_equals_ignore_case_list() {
        let ab = AnythingButCondition::EqualsIgnoreCaseList(vec![
            "admin".to_string(),
            "root".to_string(),
        ]);
        assert!(match_anything_but(&ab, &json!("user")));
        assert!(!match_anything_but(&ab, &json!("Admin")));
        assert!(!match_anything_but(&ab, &json!("ROOT")));
    }

    #[test]
    fn test_should_match_anything_but_wildcard() {
        let ab = AnythingButCondition::Wildcard("*/lib/*".to_string());
        assert!(match_anything_but(&ab, &json!("usr/bin/foo")));
        assert!(!match_anything_but(&ab, &json!("usr/lib/foo")));
    }

    // -- match_single_value tests --

    #[test]
    fn test_should_match_exact_string() {
        let cond = MatchCondition::ExactString("hello".to_string());
        assert!(match_single_value(&cond, &json!("hello")));
        assert!(!match_single_value(&cond, &json!("world")));
        assert!(!match_single_value(&cond, &json!(42)));
    }

    #[test]
    fn test_should_match_exact_numeric() {
        let cond = MatchCondition::ExactNumeric(42.0);
        assert!(match_single_value(&cond, &json!(42)));
        assert!(!match_single_value(&cond, &json!(43)));
        assert!(!match_single_value(&cond, &json!("42")));
    }

    #[test]
    fn test_should_match_exact_null() {
        let cond = MatchCondition::ExactNull;
        assert!(match_single_value(&cond, &json!(null)));
        assert!(!match_single_value(&cond, &json!("")));
    }

    #[test]
    fn test_should_match_prefix() {
        let cond = MatchCondition::Prefix("us-".to_string());
        assert!(match_single_value(&cond, &json!("us-east-1")));
        assert!(!match_single_value(&cond, &json!("eu-west-1")));
    }

    #[test]
    fn test_should_match_prefix_ignore_case() {
        let cond = MatchCondition::PrefixIgnoreCase("US-".to_string());
        assert!(match_single_value(&cond, &json!("us-east-1")));
        assert!(match_single_value(&cond, &json!("US-EAST-1")));
        assert!(!match_single_value(&cond, &json!("eu-west-1")));
    }

    #[test]
    fn test_should_match_suffix() {
        let cond = MatchCondition::Suffix(".png".to_string());
        assert!(match_single_value(&cond, &json!("image.png")));
        assert!(!match_single_value(&cond, &json!("image.jpg")));
    }

    #[test]
    fn test_should_match_suffix_ignore_case() {
        let cond = MatchCondition::SuffixIgnoreCase(".PNG".to_string());
        assert!(match_single_value(&cond, &json!("image.png")));
        assert!(match_single_value(&cond, &json!("image.PNG")));
        assert!(!match_single_value(&cond, &json!("image.jpg")));
    }

    #[test]
    fn test_should_match_equals_ignore_case() {
        let cond = MatchCondition::EqualsIgnoreCase("alice".to_string());
        assert!(match_single_value(&cond, &json!("Alice")));
        assert!(match_single_value(&cond, &json!("ALICE")));
        assert!(!match_single_value(&cond, &json!("bob")));
    }

    #[test]
    fn test_should_match_wildcard_via_single_value() {
        let cond = MatchCondition::Wildcard("dir/*.png".to_string());
        assert!(match_single_value(&cond, &json!("dir/image.png")));
        assert!(!match_single_value(&cond, &json!("other/image.png")));
    }

    #[test]
    fn test_should_match_cidr() {
        let net: IpNet = "10.0.0.0/24".parse().unwrap();
        let cond = MatchCondition::Cidr(net);
        assert!(match_single_value(&cond, &json!("10.0.0.1")));
        assert!(match_single_value(&cond, &json!("10.0.0.255")));
        assert!(!match_single_value(&cond, &json!("10.0.1.1")));
        assert!(!match_single_value(&cond, &json!("not-an-ip")));
    }

    #[test]
    fn test_should_match_cidr_ipv6() {
        let net: IpNet = "2001:db8::/32".parse().unwrap();
        let cond = MatchCondition::Cidr(net);
        assert!(match_single_value(&cond, &json!("2001:db8::1")));
        assert!(!match_single_value(&cond, &json!("2001:db9::1")));
    }

    #[test]
    fn test_should_match_numeric_via_single_value() {
        let cond = MatchCondition::Numeric(NumericCondition {
            lower: Some(NumericBound {
                value: 0.0,
                inclusive: true,
            }),
            upper: Some(NumericBound {
                value: 100.0,
                inclusive: true,
            }),
            equals: None,
        });
        assert!(match_single_value(&cond, &json!(50)));
        assert!(match_single_value(&cond, &json!(0)));
        assert!(match_single_value(&cond, &json!(100)));
        assert!(!match_single_value(&cond, &json!(101)));
        assert!(!match_single_value(&cond, &json!("50")));
    }

    #[test]
    fn test_should_not_match_prefix_for_short_string() {
        let cond = MatchCondition::PrefixIgnoreCase("longprefix".to_string());
        assert!(!match_single_value(&cond, &json!("short")));
    }

    #[test]
    fn test_should_not_match_suffix_for_short_string() {
        let cond = MatchCondition::SuffixIgnoreCase("longsuffix".to_string());
        assert!(!match_single_value(&cond, &json!("sh")));
    }
}
