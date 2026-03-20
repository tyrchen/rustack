//! API Gateway v2 service configuration.

use std::env;

/// API Gateway v2 service configuration.
#[derive(Debug, Clone)]
pub struct ApiGatewayV2Config {
    /// Skip signature validation (default: true for local dev).
    pub skip_signature_validation: bool,
    /// Default AWS region.
    pub default_region: String,
    /// Default AWS account ID.
    pub account_id: String,
    /// Host for URL generation.
    pub host: String,
    /// Port for URL generation.
    pub port: u16,
}

impl ApiGatewayV2Config {
    /// Create configuration from environment variables.
    ///
    /// Reads from:
    /// - `APIGATEWAYV2_SKIP_SIGNATURE_VALIDATION` (default: `true`)
    /// - `DEFAULT_REGION` (default: `us-east-1`)
    /// - `DEFAULT_ACCOUNT_ID` (default: `000000000000`)
    /// - `GATEWAY_HOST` (default: `localhost`)
    /// - `GATEWAY_PORT` (default: `4566`)
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            skip_signature_validation: env_bool("APIGATEWAYV2_SKIP_SIGNATURE_VALIDATION", true),
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

impl Default for ApiGatewayV2Config {
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
        v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes") || v == "1"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_create_default_config() {
        let config = ApiGatewayV2Config::default();
        assert!(config.skip_signature_validation);
        assert_eq!(config.default_region, "us-east-1");
        assert_eq!(config.account_id, "000000000000");
        assert_eq!(config.host, "localhost");
        assert_eq!(config.port, 4566);
    }
}
