//! Common AWS type definitions shared across services.

use std::fmt;

/// AWS Account ID (12-digit string).
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct AccountId(String);

impl AccountId {
    /// Default account ID used by `LocalStack`.
    pub const DEFAULT: &str = "000000000000";

    /// Create a new account ID from a string.
    ///
    /// # Errors
    /// Returns an error if the account ID is not a 12-digit numeric string.
    pub fn new(id: impl Into<String>) -> Result<Self, crate::RustackError> {
        let id = id.into();
        if id.len() != 12 || !id.chars().all(|c| c.is_ascii_digit()) {
            return Err(crate::RustackError::InvalidAccountId(id));
        }
        Ok(Self(id))
    }

    /// Get the account ID as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for AccountId {
    fn default() -> Self {
        Self(Self::DEFAULT.to_owned())
    }
}

impl fmt::Display for AccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// AWS Region identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct AwsRegion(String);

impl AwsRegion {
    /// Default region used by `LocalStack`.
    pub const DEFAULT: &str = "us-east-1";

    /// Create a new region.
    #[must_use]
    pub fn new(region: impl Into<String>) -> Self {
        Self(region.into())
    }

    /// Get the region as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for AwsRegion {
    fn default() -> Self {
        Self(Self::DEFAULT.to_owned())
    }
}

impl fmt::Display for AwsRegion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_create_valid_account_id() {
        let id = AccountId::new("123456789012").unwrap();
        assert_eq!(id.as_str(), "123456789012");
    }

    #[test]
    fn test_should_reject_invalid_account_id() {
        assert!(AccountId::new("12345").is_err());
        assert!(AccountId::new("abcdefghijkl").is_err());
        assert!(AccountId::new("1234567890123").is_err());
    }

    #[test]
    fn test_should_use_default_account_id() {
        let id = AccountId::default();
        assert_eq!(id.as_str(), "000000000000");
    }

    #[test]
    fn test_should_create_region() {
        let region = AwsRegion::new("eu-west-1");
        assert_eq!(region.as_str(), "eu-west-1");
    }

    #[test]
    fn test_should_use_default_region() {
        let region = AwsRegion::default();
        assert_eq!(region.as_str(), "us-east-1");
    }
}
