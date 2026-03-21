//! Input validation for STS operations.

use ruststack_sts_model::error::StsError;

/// Validate a role ARN format.
///
/// Must match `arn:aws:iam::\d{12}:role/.+`
pub fn validate_role_arn(arn: &str) -> Result<(), StsError> {
    if !arn.starts_with("arn:aws:iam::") {
        return Err(StsError::invalid_parameter_value(format!(
            "Invalid role ARN: {arn}"
        )));
    }
    let rest = &arn[13..]; // after "arn:aws:iam::"
    let parts: Vec<&str> = rest.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err(StsError::invalid_parameter_value(format!(
            "Invalid role ARN: {arn}"
        )));
    }
    let account_id = parts[0];
    let resource = parts[1];

    if account_id.len() != 12 || !account_id.chars().all(|c| c.is_ascii_digit()) {
        return Err(StsError::invalid_parameter_value(format!(
            "Invalid account ID in role ARN: {arn}"
        )));
    }

    if !resource.starts_with("role/") || resource.len() <= 5 {
        return Err(StsError::invalid_parameter_value(format!(
            "Invalid role resource in ARN: {arn}"
        )));
    }

    Ok(())
}

/// Parse the account ID and role name from a validated role ARN.
pub fn parse_role_arn(arn: &str) -> Result<(String, String), StsError> {
    // arn:aws:iam::ACCOUNT_ID:role/ROLE_NAME[/path]
    let rest = &arn[13..]; // after "arn:aws:iam::"
    let parts: Vec<&str> = rest.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err(StsError::invalid_parameter_value(format!(
            "Cannot parse role ARN: {arn}"
        )));
    }
    let account_id = parts[0].to_owned();
    let resource = parts[1];

    // Extract role name (handle path-based roles like "role/path/to/RoleName")
    let role_path = resource.strip_prefix("role/").ok_or_else(|| {
        StsError::invalid_parameter_value(format!("Cannot parse role ARN: {arn}"))
    })?;

    // Use the last segment as the role name if path contains slashes
    let role_name = role_path.rsplit('/').next().unwrap_or(role_path).to_owned();

    Ok((account_id, role_name))
}

/// Validate a role session name.
///
/// Must be 2-64 characters, pattern `[a-zA-Z_0-9+=,.@-]+`.
pub fn validate_session_name(name: &str) -> Result<(), StsError> {
    if name.len() < 2 || name.len() > 64 {
        return Err(StsError::invalid_parameter_value(format!(
            "RoleSessionName must be between 2 and 64 characters, got {}",
            name.len()
        )));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_+=,.@-".contains(c))
    {
        return Err(StsError::invalid_parameter_value(format!(
            "RoleSessionName contains invalid characters: {name}"
        )));
    }
    Ok(())
}

/// Validate a federated user name.
///
/// Must be 2-32 characters, pattern `[a-zA-Z_0-9+=,.@-]+`.
pub fn validate_federated_name(name: &str) -> Result<(), StsError> {
    if name.len() < 2 || name.len() > 32 {
        return Err(StsError::invalid_parameter_value(format!(
            "Name must be between 2 and 32 characters, got {}",
            name.len()
        )));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_+=,.@-".contains(c))
    {
        return Err(StsError::invalid_parameter_value(format!(
            "Name contains invalid characters: {name}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_accept_valid_role_arn() {
        assert!(validate_role_arn("arn:aws:iam::123456789012:role/TestRole").is_ok());
        assert!(validate_role_arn("arn:aws:iam::123456789012:role/path/to/Role").is_ok());
    }

    #[test]
    fn test_should_reject_invalid_role_arn() {
        assert!(validate_role_arn("not-an-arn").is_err());
        assert!(validate_role_arn("arn:aws:iam::short:role/R").is_err());
        assert!(validate_role_arn("arn:aws:iam::123456789012:user/Bob").is_err());
    }

    #[test]
    fn test_should_parse_role_arn() {
        let (account, role) = parse_role_arn("arn:aws:iam::123456789012:role/TestRole").unwrap();
        assert_eq!(account, "123456789012");
        assert_eq!(role, "TestRole");
    }

    #[test]
    fn test_should_parse_path_role_arn() {
        let (account, role) =
            parse_role_arn("arn:aws:iam::123456789012:role/path/to/MyRole").unwrap();
        assert_eq!(account, "123456789012");
        assert_eq!(role, "MyRole");
    }

    #[test]
    fn test_should_accept_valid_session_name() {
        assert!(validate_session_name("my-session").is_ok());
        assert!(validate_session_name("ab").is_ok());
        assert!(validate_session_name("a_b+c=d,e.f@g-h").is_ok());
    }

    #[test]
    fn test_should_reject_invalid_session_name() {
        assert!(validate_session_name("x").is_err()); // too short
        assert!(validate_session_name(&"a".repeat(65)).is_err()); // too long
        assert!(validate_session_name("ab cd").is_err()); // space not allowed
    }

    #[test]
    fn test_should_accept_valid_federated_name() {
        assert!(validate_federated_name("bob").is_ok());
        assert!(validate_federated_name("ab").is_ok());
    }

    #[test]
    fn test_should_reject_invalid_federated_name() {
        assert!(validate_federated_name("x").is_err()); // too short
        assert!(validate_federated_name(&"a".repeat(33)).is_err()); // too long
    }
}
