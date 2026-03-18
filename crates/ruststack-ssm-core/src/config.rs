//! SSM service configuration.

use std::env;

/// SSM service configuration.
#[derive(Debug, Clone)]
pub struct SsmConfig {
    /// Default AWS region.
    pub default_region: String,
    /// Default AWS account ID.
    pub default_account_id: String,
}

impl SsmConfig {
    /// Create configuration from environment variables.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            default_region: env::var("DEFAULT_REGION").unwrap_or_else(|_| "us-east-1".to_owned()),
            default_account_id: env::var("DEFAULT_ACCOUNT_ID")
                .unwrap_or_else(|_| "000000000000".to_owned()),
        }
    }
}

impl Default for SsmConfig {
    fn default() -> Self {
        Self {
            default_region: "us-east-1".to_owned(),
            default_account_id: "000000000000".to_owned(),
        }
    }
}
