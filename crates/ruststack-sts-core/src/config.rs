//! STS service configuration.

use std::env;

/// STS service configuration.
#[derive(Debug, Clone)]
pub struct StsConfig {
    /// Skip signature validation (default: true for local dev).
    pub skip_signature_validation: bool,
    /// Default AWS region.
    pub default_region: String,
    /// Default AWS account ID.
    pub default_account_id: String,
    /// Default access key ID (maps to root of default account).
    pub default_access_key: String,
    /// Default secret access key.
    pub default_secret_key: String,
}

impl StsConfig {
    /// Create configuration from environment variables.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            skip_signature_validation: env_bool("STS_SKIP_SIGNATURE_VALIDATION", true),
            default_region: env::var("DEFAULT_REGION").unwrap_or_else(|_| "us-east-1".to_owned()),
            default_account_id: env::var("DEFAULT_ACCOUNT_ID")
                .unwrap_or_else(|_| "000000000000".to_owned()),
            default_access_key: env::var("AWS_ACCESS_KEY_ID").unwrap_or_else(|_| "test".to_owned()),
            default_secret_key: env::var("AWS_SECRET_ACCESS_KEY")
                .unwrap_or_else(|_| "test".to_owned()),
        }
    }
}

impl Default for StsConfig {
    fn default() -> Self {
        Self {
            skip_signature_validation: true,
            default_region: "us-east-1".to_owned(),
            default_account_id: "000000000000".to_owned(),
            default_access_key: "test".to_owned(),
            default_secret_key: "test".to_owned(),
        }
    }
}

fn env_bool(key: &str, default: bool) -> bool {
    env::var(key).map_or(default, |v| {
        v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes") || v == "1"
    })
}
