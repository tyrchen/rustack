//! Configuration management for Rustack services.
//!
//! All configuration is driven by environment variables, matching `LocalStack` conventions.

use crate::types::AwsRegion;

/// Global configuration for Rustack.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RustackConfig {
    /// Bind address for the gateway.
    pub gateway_listen: String,
    /// Default AWS region.
    pub default_region: AwsRegion,
    /// Log level.
    pub log_level: String,
    /// Whether persistence is enabled.
    pub persistence: bool,
    /// Data directory for persistence.
    pub data_dir: String,
}

impl Default for RustackConfig {
    fn default() -> Self {
        Self {
            gateway_listen: "0.0.0.0:4566".to_owned(),
            default_region: AwsRegion::default(),
            log_level: "info".to_owned(),
            persistence: false,
            data_dir: "/var/lib/localstack".to_owned(),
        }
    }
}

impl RustackConfig {
    /// Load configuration from environment variables.
    #[must_use]
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(v) = std::env::var("GATEWAY_LISTEN") {
            config.gateway_listen = v;
        }
        if let Ok(v) = std::env::var("DEFAULT_REGION") {
            config.default_region = AwsRegion::new(v);
        }
        if let Ok(v) = std::env::var("LOG_LEVEL") {
            config.log_level = v;
        }
        if let Ok(v) = std::env::var("PERSISTENCE") {
            config.persistence = v == "1" || v.eq_ignore_ascii_case("true");
        }
        if let Ok(v) = std::env::var("DATA_DIR") {
            config.data_dir = v;
        }

        config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_create_default_config() {
        let config = RustackConfig::default();
        assert_eq!(config.gateway_listen, "0.0.0.0:4566");
        assert_eq!(config.default_region.as_str(), "us-east-1");
        assert!(!config.persistence);
    }
}
