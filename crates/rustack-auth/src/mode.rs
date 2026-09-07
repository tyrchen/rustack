//! Fail-closed resolution of legacy HTTP authentication configuration.

use crate::{AuthError, AuthResult, CredentialProvider, verify_sigv4};

/// Valid authentication states at an HTTP trust boundary.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub enum AuthMode<'a> {
    /// Explicit local development mode; no signature validation.
    Development,
    /// Every request must authenticate using this credential provider.
    Required(&'a dyn CredentialProvider),
}

impl std::fmt::Debug for AuthMode<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Development => "Development",
            Self::Required(_) => "Required([REDACTED])",
        })
    }
}

impl<'a> AuthMode<'a> {
    /// Resolve legacy service configuration without permitting strict-mode fallback.
    ///
    /// # Errors
    /// Returns an error if strict mode has no credential provider.
    pub fn resolve(
        skip_signature_validation: bool,
        provider: Option<&'a dyn CredentialProvider>,
    ) -> Result<Self, AuthError> {
        if skip_signature_validation {
            Ok(Self::Development)
        } else {
            provider
                .map(Self::Required)
                .ok_or(AuthError::MissingCredentialProvider)
        }
    }

    /// Verify an ordinary, fully received request in the selected mode.
    ///
    /// # Errors
    /// Returns the signature or payload validation failure in required mode.
    pub fn verify(
        self,
        parts: &http::request::Parts,
        actual_body_hash: &str,
    ) -> Result<Option<AuthResult>, AuthError> {
        match self {
            Self::Development => Ok(None),
            Self::Required(provider) => verify_sigv4(parts, actual_body_hash, provider).map(Some),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_reject_strict_without_provider() {
        assert!(matches!(
            AuthMode::resolve(false, None),
            Err(AuthError::MissingCredentialProvider)
        ));
        assert!(matches!(
            AuthMode::resolve(true, None),
            Ok(AuthMode::Development)
        ));
    }
}
