//! Lambda service configuration.

use std::{env, time::Duration};

use crate::executor::{ExecutorBackend, SquibExecutorConfig};

/// Lambda service configuration.
#[derive(Debug, Clone)]
pub struct LambdaConfig {
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
    /// Whether Docker execution is enabled for Invoke (legacy alias for
    /// `executor = Docker`).
    pub docker_enabled: bool,
    /// Selected execution backend.
    pub executor: ExecutorBackend,
    /// Maximum number of warm instances kept per `(function, qualifier)` key.
    pub max_warm_instances: usize,
    /// Idle window before a warm instance is reaped.
    pub idle_timeout: Duration,
    /// Time the bootstrap has to call `/runtime/invocation/next` after spawn.
    pub init_timeout: Duration,
    /// Squib microVM executor configuration.
    pub squib: SquibExecutorConfig,
}

impl LambdaConfig {
    /// Create configuration from environment variables.
    ///
    /// Reads from:
    /// - `LAMBDA_SKIP_SIGNATURE_VALIDATION` (default: `true`)
    /// - `DEFAULT_REGION` (default: `us-east-1`)
    /// - `DEFAULT_ACCOUNT_ID` (default: `000000000000`)
    /// - `GATEWAY_HOST` (default: `localhost`)
    /// - `GATEWAY_PORT` (default: `4566`)
    /// - `LAMBDA_DOCKER_ENABLED` (default: `false` — legacy alias for `LAMBDA_EXECUTOR=docker`).
    /// - `LAMBDA_EXECUTOR` (default: `auto`; `docker` if `LAMBDA_DOCKER_ENABLED=true`). Accepts
    ///   `disabled`, `auto`, `native`, `docker`, `squib`. Auto uses Squib for Zip functions on
    ///   macOS and native execution otherwise. The native backend runs `provided.*` bootstraps
    ///   (Rust / Go / C++) directly on the host with no Docker requirement.
    /// - `LAMBDA_MAX_WARM_INSTANCES` (default: `1`)
    /// - `LAMBDA_IDLE_TIMEOUT_SECS` (default: `600`)
    /// - `LAMBDA_INIT_TIMEOUT_SECS` (default: `5`)
    /// - `LAMBDA_SQUIB_*` variables documented by [`SquibExecutorConfig`].
    #[must_use]
    pub fn from_env() -> Self {
        let docker_enabled = env_bool("LAMBDA_DOCKER_ENABLED", false);
        let executor = env::var("LAMBDA_EXECUTOR")
            .ok()
            .and_then(|raw| raw.parse::<ExecutorBackend>().ok())
            .unwrap_or(if docker_enabled {
                ExecutorBackend::Docker
            } else {
                ExecutorBackend::Auto
            });
        Self {
            skip_signature_validation: env_bool("LAMBDA_SKIP_SIGNATURE_VALIDATION", true),
            default_region: env::var("DEFAULT_REGION").unwrap_or_else(|_| "us-east-1".to_owned()),
            account_id: env::var("DEFAULT_ACCOUNT_ID")
                .unwrap_or_else(|_| "000000000000".to_owned()),
            host: env::var("GATEWAY_HOST").unwrap_or_else(|_| "localhost".to_owned()),
            port: env::var("GATEWAY_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(4566),
            docker_enabled,
            executor,
            max_warm_instances: env::var("LAMBDA_MAX_WARM_INSTANCES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1),
            idle_timeout: Duration::from_secs(
                env::var("LAMBDA_IDLE_TIMEOUT_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(600),
            ),
            init_timeout: Duration::from_secs(
                env::var("LAMBDA_INIT_TIMEOUT_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(5),
            ),
            squib: SquibExecutorConfig::from_env(),
        }
    }
}

impl Default for LambdaConfig {
    fn default() -> Self {
        Self {
            skip_signature_validation: true,
            default_region: "us-east-1".to_owned(),
            account_id: "000000000000".to_owned(),
            host: "localhost".to_owned(),
            port: 4566,
            docker_enabled: false,
            executor: ExecutorBackend::Disabled,
            max_warm_instances: 1,
            idle_timeout: Duration::from_mins(10),
            init_timeout: Duration::from_secs(5),
            squib: SquibExecutorConfig::default(),
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
        let config = LambdaConfig::default();
        assert!(config.skip_signature_validation);
        assert_eq!(config.default_region, "us-east-1");
        assert_eq!(config.account_id, "000000000000");
        assert_eq!(config.host, "localhost");
        assert_eq!(config.port, 4566);
        assert!(!config.docker_enabled);
        assert!(config.squib.config_file.is_some());
        assert!(config.squib.vsock_path.is_some());
    }
}
