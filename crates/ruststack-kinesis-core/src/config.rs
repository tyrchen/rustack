//! Kinesis service configuration.

use std::env;

/// Kinesis service configuration.
#[derive(Debug, Clone)]
pub struct KinesisConfig {
    /// Skip signature validation (default: true for local dev).
    pub skip_signature_validation: bool,
    /// Default AWS region.
    pub default_region: String,
    /// Default AWS account ID.
    pub default_account_id: String,
    /// Gateway host.
    pub host: String,
    /// Gateway port.
    pub port: u16,
    /// Default number of shards for new streams.
    pub default_shard_count: u32,
    /// Default retention period in hours.
    pub default_retention_hours: u32,
}

impl KinesisConfig {
    /// Create configuration from environment variables.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            skip_signature_validation: env_bool("KINESIS_SKIP_SIGNATURE_VALIDATION", true),
            default_region: env::var("DEFAULT_REGION").unwrap_or_else(|_| "us-east-1".to_owned()),
            default_account_id: env::var("DEFAULT_ACCOUNT_ID")
                .unwrap_or_else(|_| "000000000000".to_owned()),
            host: env::var("GATEWAY_HOST").unwrap_or_else(|_| "localhost".to_owned()),
            port: env::var("GATEWAY_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(4566),
            default_shard_count: 4,
            default_retention_hours: 24,
        }
    }
}

impl Default for KinesisConfig {
    fn default() -> Self {
        Self {
            skip_signature_validation: true,
            default_region: "us-east-1".to_owned(),
            default_account_id: "000000000000".to_owned(),
            host: "localhost".to_owned(),
            port: 4566,
            default_shard_count: 4,
            default_retention_hours: 24,
        }
    }
}

fn env_bool(key: &str, default: bool) -> bool {
    env::var(key).map_or(default, |v| {
        matches!(v.as_str(), "1" | "true" | "yes" | "TRUE" | "YES")
    })
}
