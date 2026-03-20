//! CloudWatch service configuration.

use std::env;

/// CloudWatch service configuration.
#[derive(Debug, Clone)]
pub struct CloudWatchConfig {
    /// Skip signature validation (default: true for local dev).
    pub skip_signature_validation: bool,
    /// Default AWS region.
    pub default_region: String,
    /// Default AWS account ID.
    pub account_id: String,
    /// Maximum retention period in seconds (default: 86400 = 24h).
    pub max_retention_seconds: u64,
    /// Maximum data points per metric series (default: 100_000).
    pub max_points_per_series: usize,
}

impl CloudWatchConfig {
    /// Create configuration from environment variables.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            skip_signature_validation: env_bool("CLOUDWATCH_SKIP_SIGNATURE_VALIDATION", true),
            default_region: env::var("DEFAULT_REGION").unwrap_or_else(|_| "us-east-1".to_owned()),
            account_id: env::var("DEFAULT_ACCOUNT_ID")
                .unwrap_or_else(|_| "000000000000".to_owned()),
            max_retention_seconds: env::var("CLOUDWATCH_MAX_RETENTION_SECONDS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(86400),
            max_points_per_series: env::var("CLOUDWATCH_MAX_POINTS_PER_SERIES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(100_000),
        }
    }
}

impl Default for CloudWatchConfig {
    fn default() -> Self {
        Self {
            skip_signature_validation: true,
            default_region: "us-east-1".to_owned(),
            account_id: "000000000000".to_owned(),
            max_retention_seconds: 86400,
            max_points_per_series: 100_000,
        }
    }
}

fn env_bool(key: &str, default: bool) -> bool {
    env::var(key).map_or(default, |v| {
        v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes") || v == "1"
    })
}
