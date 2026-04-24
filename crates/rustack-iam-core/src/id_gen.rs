//! IAM entity ID and credential generation.
//!
//! Generates unique identifiers matching the AWS IAM format:
//! - Entity IDs: 4-char prefix + 17 alphanumeric uppercase characters
//! - Access key IDs: `AKIA` + 16 uppercase alphanumeric characters
//! - Secret access keys: 40 mixed-case alphanumeric + special characters

use rand::RngExt;

/// Uppercase alphanumeric character set for IAM IDs.
const UPPER_ALPHANUM: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// Mixed character set for secret access keys.
const SECRET_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Generate an IAM entity ID with the given prefix.
///
/// Format: `{prefix}{17 random uppercase alphanumeric chars}`
///
/// Common prefixes:
/// - `AIDA` for users
/// - `AROA` for roles
/// - `AGPA` for groups
/// - `ANPA` for managed policies
/// - `AIPA` for instance profiles
///
/// # Examples
///
/// ```
/// let id = rustack_iam_core::id_gen::generate_iam_id("AIDA");
/// assert!(id.starts_with("AIDA"));
/// assert_eq!(id.len(), 21); // 4 + 17
/// ```
#[must_use]
pub fn generate_iam_id(prefix: &str) -> String {
    let mut rng = rand::rng();
    let mut id = String::with_capacity(prefix.len() + 17);
    id.push_str(prefix);
    for _ in 0..17 {
        let idx = rng.random_range(0..UPPER_ALPHANUM.len());
        id.push(UPPER_ALPHANUM[idx] as char);
    }
    id
}

/// Generate an IAM access key ID.
///
/// Format: `AKIA` + 16 uppercase alphanumeric characters.
///
/// # Examples
///
/// ```
/// let key = rustack_iam_core::id_gen::generate_access_key_id();
/// assert!(key.starts_with("AKIA"));
/// assert_eq!(key.len(), 20);
/// ```
#[must_use]
pub fn generate_access_key_id() -> String {
    let mut rng = rand::rng();
    let mut id = String::with_capacity(20);
    id.push_str("AKIA");
    for _ in 0..16 {
        let idx = rng.random_range(0..UPPER_ALPHANUM.len());
        id.push(UPPER_ALPHANUM[idx] as char);
    }
    id
}

/// Generate an IAM secret access key.
///
/// Format: 40 characters from a mixed alphanumeric + `+/` character set.
///
/// # Examples
///
/// ```
/// let secret = rustack_iam_core::id_gen::generate_secret_access_key();
/// assert_eq!(secret.len(), 40);
/// ```
#[must_use]
pub fn generate_secret_access_key() -> String {
    let mut rng = rand::rng();
    let mut key = String::with_capacity(40);
    for _ in 0..40 {
        let idx = rng.random_range(0..SECRET_CHARS.len());
        key.push(SECRET_CHARS[idx] as char);
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_generate_iam_id_with_correct_prefix_and_length() {
        let id = generate_iam_id("AIDA");
        assert!(id.starts_with("AIDA"));
        assert_eq!(id.len(), 21);
        assert!(
            id[4..]
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        );
    }

    #[test]
    fn test_should_generate_access_key_id_with_akia_prefix() {
        let key = generate_access_key_id();
        assert!(key.starts_with("AKIA"));
        assert_eq!(key.len(), 20);
    }

    #[test]
    fn test_should_generate_secret_access_key_with_correct_length() {
        let secret = generate_secret_access_key();
        assert_eq!(secret.len(), 40);
    }
}
