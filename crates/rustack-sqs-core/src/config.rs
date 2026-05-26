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
        let (listen_host, listen_port) = env::var("GATEWAY_LISTEN")
            .ok()
            .and_then(|value| host_port_from_gateway_listen(&value))
            .unwrap_or_else(|| ("localhost".to_owned(), 4566));

        Self {
            skip_signature_validation: env_bool("SQS_SKIP_SIGNATURE_VALIDATION", true),
            default_region: env::var("DEFAULT_REGION").unwrap_or_else(|_| "us-east-1".to_owned()),
            account_id: env::var("DEFAULT_ACCOUNT_ID")
                .unwrap_or_else(|_| "000000000000".to_owned()),
            host: env::var("GATEWAY_HOST").unwrap_or(listen_host),
            port: env::var("GATEWAY_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(listen_port),
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

fn host_port_from_gateway_listen(value: &str) -> Option<(String, u16)> {
    let (host, port) = value.rsplit_once(':')?;
    let port = port.parse().ok()?;
    let host = host.trim().trim_start_matches('[').trim_end_matches(']');
    let host = match host {
        "" | "0.0.0.0" | "::" => "localhost",
        value => value,
    };

    Some((host.to_owned(), port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_parse_gateway_listen_host_and_port() {
        assert_eq!(
            host_port_from_gateway_listen("127.0.0.1:4567"),
            Some(("127.0.0.1".to_owned(), 4567))
        );
        assert_eq!(
            host_port_from_gateway_listen("localhost:4570"),
            Some(("localhost".to_owned(), 4570))
        );
    }

    #[test]
    fn should_map_wildcard_gateway_listen_to_localhost() {
        assert_eq!(
            host_port_from_gateway_listen("0.0.0.0:4568"),
            Some(("localhost".to_owned(), 4568))
        );
        assert_eq!(
            host_port_from_gateway_listen("[::]:4569"),
            Some(("localhost".to_owned(), 4569))
        );
    }
}
