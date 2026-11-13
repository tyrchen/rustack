//! DynamoDB configuration.

use rustack_core::settings as env;

/// DynamoDB service configuration.
#[derive(Debug, Clone)]
pub struct DynamoDBConfig {
    /// Skip signature validation (default: true for local dev).
    pub skip_signature_validation: bool,
    /// Default AWS region.
    pub default_region: String,
    /// Maximum `ClientRequestToken`s of `TransactWriteItems` remembered within the
    /// ten-minute idempotency window; a new token beyond it is rejected.
    pub max_transaction_tokens: usize,
}

/// Default for [`DynamoDBConfig::max_transaction_tokens`]: about a hundred
/// transactions a second sustained over the idempotency window.
pub const DEFAULT_MAX_TRANSACTION_TOKENS: usize = 65_536;

impl DynamoDBConfig {
    /// Create configuration from environment variables.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            skip_signature_validation: env_bool("DYNAMODB_SKIP_SIGNATURE_VALIDATION", true),
            default_region: env::var("DEFAULT_REGION").unwrap_or_else(|_| "us-east-1".to_owned()),
            max_transaction_tokens: env::var("DYNAMODB_MAX_TRANSACTION_TOKENS")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|&n| n > 0)
                .unwrap_or(DEFAULT_MAX_TRANSACTION_TOKENS),
        }
    }
}

impl Default for DynamoDBConfig {
    fn default() -> Self {
        Self {
            skip_signature_validation: true,
            default_region: "us-east-1".to_owned(),
            max_transaction_tokens: DEFAULT_MAX_TRANSACTION_TOKENS,
        }
    }
}

fn env_bool(key: &str, default: bool) -> bool {
    env::var(key).map_or(default, |v| {
        matches!(v.as_str(), "1" | "true" | "yes" | "TRUE" | "YES")
    })
}
