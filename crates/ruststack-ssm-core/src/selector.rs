//! Parameter name selector parsing.
//!
//! SSM `GetParameter` and `GetParameters` support selector syntax in the
//! parameter name:
//!
//! - `"/my/param"` - no selector (latest version)
//! - `"/my/param:3"` - select version 3
//! - `"/my/param:my-label"` - select by label

use ruststack_ssm_model::error::SsmError;

/// A parsed parameter selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParameterSelector {
    /// Select a specific version by number.
    Version(u64),
    /// Select a version by label.
    Label(String),
}

/// The result of parsing a parameter name with an optional selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedName {
    /// The base parameter name (without selector).
    pub name: String,
    /// The optional selector (version or label).
    pub selector: Option<ParameterSelector>,
}

/// Parse a parameter name that may contain a `:version` or `:label` selector.
///
/// # Examples
///
/// ```
/// use ruststack_ssm_core::selector::{parse_name_with_selector, ParameterSelector};
///
/// let parsed = parse_name_with_selector("/my/param").unwrap();
/// assert_eq!(parsed.name, "/my/param");
/// assert_eq!(parsed.selector, None);
///
/// let parsed = parse_name_with_selector("/my/param:3").unwrap();
/// assert_eq!(parsed.name, "/my/param");
/// assert_eq!(parsed.selector, Some(ParameterSelector::Version(3)));
///
/// let parsed = parse_name_with_selector("/my/param:prod").unwrap();
/// assert_eq!(parsed.name, "/my/param");
/// assert_eq!(parsed.selector, Some(ParameterSelector::Label("prod".to_owned())));
/// ```
pub fn parse_name_with_selector(name: &str) -> Result<ParsedName, SsmError> {
    // Find the last `:` that is a selector delimiter.
    // Parameter names can contain `/`, `.`, `-`, `_` but selectors are appended
    // after the final `:`.
    if let Some(colon_pos) = name.rfind(':') {
        let base = &name[..colon_pos];
        let selector_str = &name[colon_pos + 1..];

        // Empty selector is invalid.
        if selector_str.is_empty() {
            return Err(SsmError::validation(format!(
                "Invalid parameter name: {name}. Parameter name must not end with ':'."
            )));
        }

        // If the selector is all digits, it is a version number.
        if selector_str.chars().all(|c| c.is_ascii_digit()) {
            let version: u64 = selector_str.parse().map_err(|_| {
                SsmError::validation(format!("Invalid version selector in: {name}"))
            })?;
            if version == 0 {
                return Err(SsmError::with_message(
                    ruststack_ssm_model::error::SsmErrorCode::ParameterVersionNotFound,
                    format!("Version 0 not found for parameter {base}"),
                ));
            }
            Ok(ParsedName {
                name: base.to_owned(),
                selector: Some(ParameterSelector::Version(version)),
            })
        } else {
            // It is a label selector.
            Ok(ParsedName {
                name: base.to_owned(),
                selector: Some(ParameterSelector::Label(selector_str.to_owned())),
            })
        }
    } else {
        Ok(ParsedName {
            name: name.to_owned(),
            selector: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_parse_plain_name() {
        let parsed = parse_name_with_selector("/my/param").expect("should parse");
        assert_eq!(parsed.name, "/my/param");
        assert_eq!(parsed.selector, None);
    }

    #[test]
    fn test_should_parse_version_selector() {
        let parsed = parse_name_with_selector("/my/param:5").expect("should parse");
        assert_eq!(parsed.name, "/my/param");
        assert_eq!(parsed.selector, Some(ParameterSelector::Version(5)));
    }

    #[test]
    fn test_should_parse_label_selector() {
        let parsed = parse_name_with_selector("/my/param:prod").expect("should parse");
        assert_eq!(parsed.name, "/my/param");
        assert_eq!(
            parsed.selector,
            Some(ParameterSelector::Label("prod".to_owned()))
        );
    }

    #[test]
    fn test_should_reject_trailing_colon() {
        let result = parse_name_with_selector("/my/param:");
        assert!(result.is_err());
    }

    #[test]
    fn test_should_reject_version_zero() {
        let result = parse_name_with_selector("/my/param:0");
        assert!(result.is_err());
    }

    #[test]
    fn test_should_parse_name_without_leading_slash() {
        let parsed = parse_name_with_selector("my-param").expect("should parse");
        assert_eq!(parsed.name, "my-param");
        assert_eq!(parsed.selector, None);
    }
}
