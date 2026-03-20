//! DynamoDB Streams configuration.

use std::env;

/// DynamoDB Streams service configuration.
#[derive(Debug, Clone)]
pub struct DynamoDBStreamsConfig {
    /// Skip signature validation (default: true for local dev).
    pub skip_signature_validation: bool,
    /// Default AWS region.
    pub default_region: String,
    /// Default account ID for ARNs.
    pub default_account_id: String,
    /// Maximum records per shard (0 = unlimited).
    pub max_records_per_shard: usize,
    /// Maximum record age in seconds (0 = unlimited).
    pub max_record_age_seconds: u64,
}

impl DynamoDBStreamsConfig {
    /// Create configuration from environment variables.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            skip_signature_validation: env_bool("DYNAMODBSTREAMS_SKIP_SIGNATURE_VALIDATION", true),
            default_region: env::var("DEFAULT_REGION").unwrap_or_else(|_| "us-east-1".to_owned()),
            default_account_id: env::var("DEFAULT_ACCOUNT_ID")
                .unwrap_or_else(|_| "000000000000".to_owned()),
            max_records_per_shard: env::var("DYNAMODBSTREAMS_MAX_RECORDS_PER_SHARD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            max_record_age_seconds: env::var("DYNAMODBSTREAMS_MAX_RECORD_AGE_SECONDS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
        }
    }
}

impl Default for DynamoDBStreamsConfig {
    fn default() -> Self {
        Self {
            skip_signature_validation: true,
            default_region: "us-east-1".to_owned(),
            default_account_id: "000000000000".to_owned(),
            max_records_per_shard: 0,
            max_record_age_seconds: 0,
        }
    }
}

fn env_bool(key: &str, default: bool) -> bool {
    env::var(key).map_or(default, |v| {
        matches!(v.as_str(), "1" | "true" | "yes" | "TRUE" | "YES")
    })
}
