//! DynamoDB configuration.

use std::env;

/// DynamoDB service configuration.
#[derive(Debug, Clone)]
pub struct DynamoDBConfig {
    /// Default AWS region.
    pub default_region: String,
}

impl DynamoDBConfig {
    /// Create configuration from environment variables.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            default_region: env::var("DEFAULT_REGION").unwrap_or_else(|_| "us-east-1".to_owned()),
        }
    }
}

impl Default for DynamoDBConfig {
    fn default() -> Self {
        Self {
            default_region: "us-east-1".to_owned(),
        }
    }
}
