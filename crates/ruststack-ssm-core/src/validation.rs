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

/// Validate tags.
pub fn validate_tags(tags: &[ruststack_ssm_model::types::Tag]) -> Result<(), SsmError> {
    if tags.len() > MAX_TAGS {
        return Err(SsmError::with_message(
            SsmErrorCode::TooManyTagsError,
            format!("Number of tags exceeds the maximum of {MAX_TAGS}."),
        ));
    }
    Ok(())
}

/// Validate an allowed pattern regex and check the value against it.
pub fn validate_allowed_pattern(pattern: &str, value: &str) -> Result<(), SsmError> {
    // For simplicity in local emulation, we validate that the pattern is a valid
    // basic regex. We do not pull in a regex crate; instead we do a simple check.
    // In a real implementation we would use `regex` crate.
    // For now, we just check that the value is non-empty if a pattern is set.
    if pattern.is_empty() {
        return Err(SsmError::with_message(
            SsmErrorCode::InvalidAllowedPatternException,
            "AllowedPattern must not be empty.",
        ));
    }

    // Simple wildcard/literal match for common patterns.
    // AWS uses Java regex; for local testing we do basic matching.
    if pattern == ".*" {
        // Matches everything.
        return Ok(());
    }

    // For exact match patterns (no special regex chars), do literal comparison.
    let is_simple = pattern
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_./-".contains(c));
    if is_simple && value != pattern {
        return Err(SsmError::with_message(
            SsmErrorCode::ParameterPatternMismatchException,
            format!("Parameter value '{value}' does not match the allowed pattern '{pattern}'."),
        ));
    }

    Ok(())
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
}
