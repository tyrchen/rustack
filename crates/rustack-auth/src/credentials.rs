//! Credential provider trait and implementations.
//!
//! This module defines the [`CredentialProvider`] trait for resolving secret access keys
//! from access key IDs, along with a [`StaticCredentialProvider`] for testing and
//! development use cases.

use std::collections::HashMap;

use crate::error::AuthError;

/// Trait for looking up secret access keys by access key ID.
///
/// Implementations may back this with a database, configuration file,
/// or any other credential store.
pub trait CredentialProvider: Send + Sync {
    /// Retrieve the secret access key for the given access key ID.
    ///
    /// # Errors
    ///
    /// Returns [`AuthError::AccessKeyNotFound`] if the access key ID is not recognized.
    fn get_secret_key(&self, access_key_id: &str) -> Result<String, AuthError>;
}

/// A simple in-memory credential provider backed by a `HashMap`.
///
/// Suitable for testing and development environments. For production use,
/// implement [`CredentialProvider`] with a secure credential store.
///
/// # Examples
///
/// ```
/// use rustack_auth::credentials::{CredentialProvider, StaticCredentialProvider};
///
/// let provider = StaticCredentialProvider::new(vec![
///     ("AKIAIOSFODNN7EXAMPLE".to_owned(), "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_owned()),
/// ]);
///
/// let secret = provider.get_secret_key("AKIAIOSFODNN7EXAMPLE").unwrap();
/// assert_eq!(secret, "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY");
/// ```
#[derive(Debug, Clone)]
pub struct StaticCredentialProvider {
    credentials: HashMap<String, String>,
}

impl StaticCredentialProvider {
    /// Create a new `StaticCredentialProvider` from an iterable of (`access_key_id`, `secret_key`)
    /// pairs.
    pub fn new(credentials: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            credentials: credentials.into_iter().collect(),
        }
    }
}

impl CredentialProvider for StaticCredentialProvider {
    fn get_secret_key(&self, access_key_id: &str) -> Result<String, AuthError> {
        self.credentials
            .get(access_key_id)
            .cloned()
            .ok_or_else(|| AuthError::AccessKeyNotFound(access_key_id.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_return_secret_key_for_known_access_key() {
        let provider =
            StaticCredentialProvider::new(vec![("AKID".to_owned(), "secret".to_owned())]);

        let result = provider.get_secret_key("AKID");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "secret");
    }

    #[test]
    fn test_should_return_error_for_unknown_access_key() {
        let provider = StaticCredentialProvider::new(vec![]);

        let result = provider.get_secret_key("UNKNOWN");
        assert!(matches!(result, Err(AuthError::AccessKeyNotFound(_))));
    }
}
