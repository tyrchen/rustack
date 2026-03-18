//! EventBridge service configuration.

use std::env;

/// EventBridge service configuration.
#[derive(Debug, Clone)]
pub struct EventsConfig {
    /// Default AWS region.
    pub default_region: String,
    /// Default AWS account ID.
    pub account_id: String,
    /// Host to bind to.
    pub host: String,
    /// Port to listen on.
    pub port: u16,
}

impl EventsConfig {
    /// Create configuration from environment variables.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            default_region: env::var("DEFAULT_REGION").unwrap_or_else(|_| "us-east-1".to_owned()),
            account_id: env::var("DEFAULT_ACCOUNT_ID")
                .unwrap_or_else(|_| "000000000000".to_owned()),
            host: env::var("EVENTS_HOST").unwrap_or_else(|_| "0.0.0.0".to_owned()),
            port: env::var("EVENTS_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(4510),
        }
    }
}

impl Default for EventsConfig {
    fn default() -> Self {
        Self {
            default_region: "us-east-1".to_owned(),
            account_id: "000000000000".to_owned(),
            host: "0.0.0.0".to_owned(),
            port: 4510,
        }
    }
}
