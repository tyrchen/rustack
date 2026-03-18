//! DynamoDB configuration.

use std::env;

/// DynamoDB service configuration.
#[derive(Debug, Clone)]
pub struct DynamoDBConfig {
    /// Skip signature validation (default: true for local dev).
    pub skip_signature_validation: bool,
    /// Default AWS region.
    pub default_region: String,
}

impl DynamoDBConfig {
    /// Create configuration from environment variables.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            skip_signature_validation: env_bool("DYNAMODB_SKIP_SIGNATURE_VALIDATION", true),
            default_region: env::var("DEFAULT_REGION").unwrap_or_else(|_| "us-east-1".to_owned()),
        }
    }
}

impl Default for DynamoDBConfig {
    fn default() -> Self {
        Self {
            skip_signature_validation: true,
            default_region: "us-east-1".to_owned(),
        }
    }
}

fn env_bool(key: &str, default: bool) -> bool {
    env::var(key).map_or(default, |v| {
        matches!(v.as_str(), "1" | "true" | "yes" | "TRUE" | "YES")
    })
}
