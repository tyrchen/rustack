//! IAM service configuration.

use rustack_core::settings as env;

/// IAM service configuration.
///
/// IAM is a global AWS service (no region) so this only tracks account-level
/// settings.
#[derive(Debug, Clone)]
pub struct IamConfig {
    /// Skip AWS signature validation (default: true for local dev).
    pub skip_signature_validation: bool,
    /// The AWS account ID used for ARN generation.
    pub account_id: String,
}

impl IamConfig {
    /// Create configuration from environment variables.
    ///
    /// | Variable | Default |
    /// |---|---|
    /// | `IAM_SKIP_SIGNATURE_VALIDATION` | `true` |
    /// | `DEFAULT_ACCOUNT_ID` | `000000000000` |
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            skip_signature_validation: env_bool("IAM_SKIP_SIGNATURE_VALIDATION", true),
            account_id: env::var("DEFAULT_ACCOUNT_ID")
                .unwrap_or_else(|_| "000000000000".to_owned()),
        }
    }
}

impl Default for IamConfig {
    fn default() -> Self {
        Self {
            skip_signature_validation: true,
            account_id: "000000000000".to_owned(),
        }
    }
}

/// Read a boolean from an environment variable.
fn env_bool(key: &str, default: bool) -> bool {
    env::var(key).map_or(default, |v| {
        v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes") || v == "1"
    })
}
