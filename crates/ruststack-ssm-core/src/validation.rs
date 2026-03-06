//! Parameter validation rules.
//!
//! Implements AWS SSM validation constraints for parameter names, values,
//! descriptions, hierarchy depth, and allowed patterns.

use ruststack_ssm_model::error::{SsmError, SsmErrorCode};
use ruststack_ssm_model::types::ParameterTier;

/// Maximum parameter name length.
const MAX_NAME_LENGTH: usize = 2048;

/// Maximum hierarchy depth (number of `/` separators).
const MAX_HIERARCHY_DEPTH: usize = 15;

/// Maximum description length.
const MAX_DESCRIPTION_LENGTH: usize = 1024;

/// Maximum value size for Standard tier (4 KB).
const MAX_STANDARD_VALUE_SIZE: usize = 4096;

/// Maximum value size for Advanced tier (8 KB).
const MAX_ADVANCED_VALUE_SIZE: usize = 8192;

/// Maximum number of tags per resource.
const MAX_TAGS: usize = 50;

/// Maximum number of versions per parameter.
pub const MAX_VERSIONS: usize = 100;

/// Maximum number of parameters in a batch get/delete.
pub const MAX_BATCH_SIZE: usize = 10;

/// Maximum number of labels per parameter version.
pub const MAX_LABELS_PER_VERSION: usize = 10;

/// Maximum label length.
const MAX_LABEL_LENGTH: usize = 100;

/// Validate a parameter name.
pub fn validate_name(name: &str) -> Result<(), SsmError> {
    if name.is_empty() || name.len() > MAX_NAME_LENGTH {
        return Err(SsmError::validation(format!(
            "Parameter name must be between 1 and {MAX_NAME_LENGTH} characters."
        )));
    }

    // Validate characters: [a-zA-Z0-9_./-]
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_./-".contains(c))
    {
        return Err(SsmError::validation(format!(
            "Parameter name '{name}' contains invalid characters. \
             Only [a-zA-Z0-9_./-] are allowed."
        )));
    }

    // Cannot prefix with `aws` or `ssm` (case-insensitive).
    let lower = name.to_lowercase();
    // Strip leading slashes for prefix check.
    let check_name = lower.trim_start_matches('/');
    if check_name.starts_with("aws") || check_name.starts_with("ssm") {
        return Err(SsmError::validation(format!(
            "Parameter name '{name}' is not allowed. \
             Names beginning with 'aws' or 'ssm' (case-insensitive) are reserved."
        )));
    }

    // Validate hierarchy depth.
    let depth = name.matches('/').count();
    if depth > MAX_HIERARCHY_DEPTH {
        return Err(SsmError::with_message(
            SsmErrorCode::HierarchyLevelLimitExceeded,
            format!(
                "Parameter name '{name}' exceeds the maximum hierarchy depth \
                 of {MAX_HIERARCHY_DEPTH} levels."
            ),
        ));
    }

    Ok(())
}

/// Validate a parameter value against tier size limits.
pub fn validate_value(value: &str, tier: &ParameterTier) -> Result<(), SsmError> {
    let max_size = match tier {
        ParameterTier::Standard => MAX_STANDARD_VALUE_SIZE,
        ParameterTier::Advanced | ParameterTier::IntelligentTiering => MAX_ADVANCED_VALUE_SIZE,
    };

    if value.len() > max_size {
        return Err(SsmError::validation(format!(
            "Parameter value exceeds the maximum size of {max_size} bytes for \
             {tier} tier."
        )));
    }

    if value.is_empty() {
        return Err(SsmError::validation("Parameter value must not be empty."));
    }

    Ok(())
}

/// Validate a parameter description.
pub fn validate_description(description: &str) -> Result<(), SsmError> {
    if description.len() > MAX_DESCRIPTION_LENGTH {
        return Err(SsmError::validation(format!(
            "Description exceeds the maximum length of {MAX_DESCRIPTION_LENGTH} characters."
        )));
    }
    Ok(())
}

/// Maximum tag key length.
const MAX_TAG_KEY_LENGTH: usize = 128;

/// Maximum tag value length.
const MAX_TAG_VALUE_LENGTH: usize = 256;

/// Validate tags (count, key length, value length).
pub fn validate_tags(tags: &[ruststack_ssm_model::types::Tag]) -> Result<(), SsmError> {
    if tags.len() > MAX_TAGS {
        return Err(SsmError::with_message(
            SsmErrorCode::TooManyTagsError,
            format!("Number of tags exceeds the maximum of {MAX_TAGS}."),
        ));
    }
    for tag in tags {
        if tag.key.is_empty() || tag.key.len() > MAX_TAG_KEY_LENGTH {
            return Err(SsmError::validation(format!(
                "Tag key must be between 1 and {MAX_TAG_KEY_LENGTH} characters."
            )));
        }
        if tag.value.len() > MAX_TAG_VALUE_LENGTH {
            return Err(SsmError::validation(format!(
                "Tag value must not exceed {MAX_TAG_VALUE_LENGTH} characters."
            )));
        }
    }
    Ok(())
}

/// Validate an allowed pattern regex and check the value against it.
///
/// AWS SSM uses Java regex patterns. We use Rust's `regex` crate which covers
/// most common patterns. The pattern is compiled and must match the full value.
pub fn validate_allowed_pattern(pattern: &str, value: &str) -> Result<(), SsmError> {
    if pattern.is_empty() {
        return Err(SsmError::with_message(
            SsmErrorCode::InvalidAllowedPatternException,
            "AllowedPattern must not be empty.",
        ));
    }

    // Compile the pattern as a regex.
    let re = regex::Regex::new(pattern).map_err(|_| {
        SsmError::with_message(
            SsmErrorCode::InvalidAllowedPatternException,
            format!("Invalid AllowedPattern: {pattern}"),
        )
    })?;

    // AWS requires a full match (anchored). Wrap in ^ and $ if not already.
    let full_match = if pattern.starts_with('^') && pattern.ends_with('$') {
        re.is_match(value)
    } else {
        let anchored = format!("^(?:{pattern})$");
        let re_full = regex::Regex::new(&anchored).map_err(|_| {
            SsmError::with_message(
                SsmErrorCode::InvalidAllowedPatternException,
                format!("Invalid AllowedPattern: {pattern}"),
            )
        })?;
        re_full.is_match(value)
    };

    if !full_match {
        return Err(SsmError::with_message(
            SsmErrorCode::ParameterPatternMismatchException,
            format!("Parameter value failed to satisfy the AllowedPattern: {pattern}"),
        ));
    }

    Ok(())
}

/// Validate a parameter version label.
///
/// Labels must be 1-100 characters, contain only `[a-zA-Z0-9_.-]`,
/// and cannot start with `aws`, `ssm` (case-insensitive), or a digit.
///
/// Returns `true` if the label is valid, `false` otherwise.
#[must_use]
pub fn is_valid_label(label: &str) -> bool {
    if label.is_empty() || label.len() > MAX_LABEL_LENGTH {
        return false;
    }

    // Must contain only [a-zA-Z0-9_.-]
    if !label
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
    {
        return false;
    }

    // Cannot start with a digit.
    if label.starts_with(|c: char| c.is_ascii_digit()) {
        return false;
    }

    // Cannot start with `aws` or `ssm` (case-insensitive).
    let lower = label.to_lowercase();
    if lower.starts_with("aws") || lower.starts_with("ssm") {
        return false;
    }

    true
}

/// Parse a tier string into a `ParameterTier`.
pub fn parse_tier(tier: &str) -> Result<ParameterTier, SsmError> {
    match tier {
        "Standard" => Ok(ParameterTier::Standard),
        "Advanced" => Ok(ParameterTier::Advanced),
        "Intelligent-Tiering" => Ok(ParameterTier::IntelligentTiering),
        _ => Err(SsmError::validation(format!(
            "Unsupported tier: {tier}. Valid values: Standard, Advanced, Intelligent-Tiering."
        ))),
    }
}

/// Parse a parameter type string.
pub fn parse_parameter_type(
    type_str: &str,
) -> Result<ruststack_ssm_model::types::ParameterType, SsmError> {
    match type_str {
        "String" => Ok(ruststack_ssm_model::types::ParameterType::String),
        "StringList" => Ok(ruststack_ssm_model::types::ParameterType::StringList),
        "SecureString" => Ok(ruststack_ssm_model::types::ParameterType::SecureString),
        _ => Err(SsmError::with_message(
            SsmErrorCode::UnsupportedParameterType,
            format!("Unsupported parameter type: {type_str}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_validate_valid_name() {
        assert!(validate_name("/my/param").is_ok());
        assert!(validate_name("/my/param-name").is_ok());
        assert!(validate_name("/my/param_name").is_ok());
        assert!(validate_name("/my/param.name").is_ok());
        assert!(validate_name("param").is_ok());
    }

    #[test]
    fn test_should_reject_empty_name() {
        assert!(validate_name("").is_err());
    }

    #[test]
    fn test_should_reject_reserved_prefix() {
        assert!(validate_name("/aws/param").is_err());
        assert!(validate_name("/ssm/param").is_err());
        assert!(validate_name("aws-param").is_err());
    }

    #[test]
    fn test_should_reject_invalid_chars() {
        assert!(validate_name("/my/param!").is_err());
        assert!(validate_name("/my/param@name").is_err());
    }

    #[test]
    fn test_should_reject_deep_hierarchy() {
        let deep = "/a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p";
        assert!(validate_name(deep).is_err());
    }

    #[test]
    fn test_should_validate_value_standard() {
        let value = "a".repeat(4096);
        assert!(validate_value(&value, &ParameterTier::Standard).is_ok());

        let value = "a".repeat(4097);
        assert!(validate_value(&value, &ParameterTier::Standard).is_err());
    }

    #[test]
    fn test_should_validate_value_advanced() {
        let value = "a".repeat(8192);
        assert!(validate_value(&value, &ParameterTier::Advanced).is_ok());

        let value = "a".repeat(8193);
        assert!(validate_value(&value, &ParameterTier::Advanced).is_err());
    }

    #[test]
    fn test_should_reject_empty_value() {
        assert!(validate_value("", &ParameterTier::Standard).is_err());
    }

    #[test]
    fn test_should_validate_description() {
        let desc = "a".repeat(1024);
        assert!(validate_description(&desc).is_ok());

        let desc = "a".repeat(1025);
        assert!(validate_description(&desc).is_err());
    }

    #[test]
    fn test_should_parse_tier() {
        assert_eq!(parse_tier("Standard").expect("ok"), ParameterTier::Standard);
        assert_eq!(parse_tier("Advanced").expect("ok"), ParameterTier::Advanced);
        assert_eq!(
            parse_tier("Intelligent-Tiering").expect("ok"),
            ParameterTier::IntelligentTiering
        );
        assert!(parse_tier("Unknown").is_err());
    }

    #[test]
    fn test_should_parse_parameter_type() {
        use ruststack_ssm_model::types::ParameterType;
        assert_eq!(
            parse_parameter_type("String").expect("ok"),
            ParameterType::String
        );
        assert_eq!(
            parse_parameter_type("StringList").expect("ok"),
            ParameterType::StringList
        );
        assert_eq!(
            parse_parameter_type("SecureString").expect("ok"),
            ParameterType::SecureString
        );
        assert!(parse_parameter_type("Invalid").is_err());
    }

    #[test]
    fn test_should_validate_valid_labels() {
        assert!(is_valid_label("release"));
        assert!(is_valid_label("my-label"));
        assert!(is_valid_label("my_label"));
        assert!(is_valid_label("my.label"));
        assert!(is_valid_label("Release-v1.0"));
        assert!(is_valid_label("a")); // minimum length
    }

    #[test]
    fn test_should_reject_invalid_labels() {
        // Empty.
        assert!(!is_valid_label(""));
        // Too long (101 chars).
        assert!(!is_valid_label(&"a".repeat(101)));
        // Starts with digit.
        assert!(!is_valid_label("1label"));
        // Starts with aws (case-insensitive).
        assert!(!is_valid_label("aws-reserved"));
        assert!(!is_valid_label("AWS-reserved"));
        // Starts with ssm (case-insensitive).
        assert!(!is_valid_label("ssm-reserved"));
        assert!(!is_valid_label("SSM-reserved"));
        // Invalid characters.
        assert!(!is_valid_label("label with spaces"));
        assert!(!is_valid_label("label@special"));
    }

    #[test]
    fn test_should_accept_max_length_label() {
        // Exactly 100 chars should be valid.
        let label = "a".repeat(100);
        assert!(is_valid_label(&label));
    }
}
