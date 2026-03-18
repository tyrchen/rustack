//! SQS service configuration.

use std::env;

/// SQS service configuration.
#[derive(Debug, Clone)]
pub struct SqsConfig {
    /// Skip signature validation (default: true for local dev).
    pub skip_signature_validation: bool,
    /// Default AWS region.
    pub default_region: String,
    /// Default AWS account ID for queue URLs.
    pub account_id: String,
    /// Host for queue URL generation.
    pub host: String,
    /// Port for queue URL generation.
    pub port: u16,
}

impl SqsConfig {
    /// Create configuration from environment variables.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            skip_signature_validation: env_bool("SQS_SKIP_SIGNATURE_VALIDATION", true),
            default_region: env::var("DEFAULT_REGION").unwrap_or_else(|_| "us-east-1".to_owned()),
            account_id: env::var("DEFAULT_ACCOUNT_ID")
                .unwrap_or_else(|_| "000000000000".to_owned()),
            host: env::var("GATEWAY_HOST").unwrap_or_else(|_| "localhost".to_owned()),
            port: env::var("GATEWAY_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(4566),
        }
    }
}

impl Default for SqsConfig {
    fn default() -> Self {
        Self {
            skip_signature_validation: true,
            default_region: "us-east-1".to_owned(),
            account_id: "000000000000".to_owned(),
            host: "localhost".to_owned(),
            port: 4566,
        }
    }
}

fn env_bool(key: &str, default: bool) -> bool {
    env::var(key).map_or(default, |v| {
        matches!(v.as_str(), "1" | "true" | "yes" | "TRUE" | "YES")
    })
}
