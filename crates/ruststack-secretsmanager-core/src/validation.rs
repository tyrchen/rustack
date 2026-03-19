//! Validation rules for Secrets Manager resources.

use ruststack_secretsmanager_model::error::{SecretsManagerError, SecretsManagerErrorCode};
use ruststack_secretsmanager_model::types::Tag;

/// Maximum secret name length.
const MAX_NAME_LENGTH: usize = 512;

/// Maximum secret value size (string or binary), 65536 bytes.
pub const MAX_VALUE_SIZE: usize = 65_536;

/// Maximum description length.
const MAX_DESCRIPTION_LENGTH: usize = 2048;

/// Maximum number of tags per secret.
pub const MAX_TAGS: usize = 50;

/// Maximum tag key length.
const MAX_TAG_KEY_LENGTH: usize = 128;

/// Maximum tag value length.
const MAX_TAG_VALUE_LENGTH: usize = 256;

/// Validate a secret name.
///
/// Secret names must be 1-512 characters, containing only ASCII letters,
/// numbers, and the characters `/_+=.@-`.
pub fn validate_secret_name(name: &str) -> Result<(), SecretsManagerError> {
    if name.is_empty() || name.len() > MAX_NAME_LENGTH {
        return Err(SecretsManagerError::with_message(
            SecretsManagerErrorCode::InvalidParameterException,
            format!("The parameter Name must be between 1 and {MAX_NAME_LENGTH} characters long."),
        ));
    }

    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/_+=.@-".contains(c))
    {
        return Err(SecretsManagerError::with_message(
            SecretsManagerErrorCode::InvalidParameterException,
            format!(
                "The parameter Name \"{name}\" contains invalid characters. \
                 Only ASCII letters, digits, and /_+=.@- are allowed."
            ),
        ));
    }

    Ok(())
}

/// Validate a client request token (version ID).
///
/// Must be 32-64 characters, matching `[a-zA-Z0-9-]+`.
pub fn validate_client_request_token(token: &str) -> Result<(), SecretsManagerError> {
    if token.len() < 32 || token.len() > 64 {
        return Err(SecretsManagerError::with_message(
            SecretsManagerErrorCode::InvalidParameterException,
            "ClientRequestToken must be 32-64 characters long.",
        ));
    }

    if !token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(SecretsManagerError::with_message(
            SecretsManagerErrorCode::InvalidParameterException,
            "ClientRequestToken must contain only alphanumeric characters and hyphens.",
        ));
    }

    Ok(())
}

/// Validate tags (count, key length, value length).
pub fn validate_tags(tags: &[Tag]) -> Result<(), SecretsManagerError> {
    if tags.len() > MAX_TAGS {
        return Err(SecretsManagerError::with_message(
            SecretsManagerErrorCode::InvalidParameterException,
            format!("Number of tags exceeds the maximum of {MAX_TAGS}."),
        ));
    }
    for tag in tags {
        let key = tag.key.as_deref().unwrap_or("");
        let value = tag.value.as_deref().unwrap_or("");
        if key.is_empty() || key.len() > MAX_TAG_KEY_LENGTH {
            return Err(SecretsManagerError::with_message(
                SecretsManagerErrorCode::InvalidParameterException,
                format!("Tag key must be between 1 and {MAX_TAG_KEY_LENGTH} characters."),
            ));
        }
        if value.len() > MAX_TAG_VALUE_LENGTH {
            return Err(SecretsManagerError::with_message(
                SecretsManagerErrorCode::InvalidParameterException,
                format!("Tag value must not exceed {MAX_TAG_VALUE_LENGTH} characters."),
            ));
        }
    }
    Ok(())
}

/// Validate a secret description.
pub fn validate_description(description: &str) -> Result<(), SecretsManagerError> {
    if description.len() > MAX_DESCRIPTION_LENGTH {
        return Err(SecretsManagerError::with_message(
            SecretsManagerErrorCode::InvalidParameterException,
            format!(
                "Description exceeds the maximum length of {MAX_DESCRIPTION_LENGTH} characters."
            ),
        ));
    }
    Ok(())
}

/// Validate that exactly one of `secret_string` or `secret_binary` is provided.
pub fn validate_secret_value(
    secret_string: Option<&str>,
    secret_binary: Option<&bytes::Bytes>,
) -> Result<(), SecretsManagerError> {
    match (secret_string, secret_binary) {
        (Some(_), Some(_)) => Err(SecretsManagerError::with_message(
            SecretsManagerErrorCode::InvalidParameterException,
            "You can't specify both SecretString and SecretBinary.",
        )),
        (None, None) => Err(SecretsManagerError::with_message(
            SecretsManagerErrorCode::InvalidParameterException,
            "You must specify either SecretString or SecretBinary.",
        )),
        (Some(s), None) => {
            if s.len() > MAX_VALUE_SIZE {
                return Err(SecretsManagerError::with_message(
                    SecretsManagerErrorCode::InvalidParameterException,
                    format!("SecretString length exceeds the maximum of {MAX_VALUE_SIZE} bytes."),
                ));
            }
            Ok(())
        }
        (None, Some(b)) => {
            if b.len() > MAX_VALUE_SIZE {
                return Err(SecretsManagerError::with_message(
                    SecretsManagerErrorCode::InvalidParameterException,
                    format!("SecretBinary length exceeds the maximum of {MAX_VALUE_SIZE} bytes."),
                ));
            }
            Ok(())
        }
    }
}

/// Validate the recovery window for `DeleteSecret`.
pub fn validate_recovery_window(days: i64) -> Result<(), SecretsManagerError> {
    if !(7..=30).contains(&days) {
        return Err(SecretsManagerError::with_message(
            SecretsManagerErrorCode::InvalidParameterException,
            "RecoveryWindowInDays must be between 7 and 30 days.",
        ));
    }
    Ok(())
}
