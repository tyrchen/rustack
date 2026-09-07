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
    /// - `LAMBDA_EXECUTOR` (default: `disabled`; unsupported Docker settings fail explicitly).
    ///   Accepts `disabled`, `auto`, `native`, `docker`, `squib`. Auto uses Squib for Zip functions
    ///   on macOS and native execution otherwise. The native backend runs `provided.*` bootstraps
    ///   (Rust / Go / C++) directly on the host with no Docker requirement.
    /// - `LAMBDA_MAX_WARM_INSTANCES` (default: `1`)
    /// - `LAMBDA_IDLE_TIMEOUT_SECS` (default: `600`)
    /// - `LAMBDA_INIT_TIMEOUT_SECS` (default: `5`)
    /// - `LAMBDA_SQUIB_*` variables documented by [`SquibExecutorConfig`].
    pub fn from_env() -> Result<Self, crate::error::LambdaServiceError> {
        Self::from_lookup(|key| match rustack_core::settings::var(key) {
            Ok(value) => Some(value),
            Err(env::VarError::NotPresent) => None,
            Err(_) => Some(String::new()),
        })
    }

    /// Parse an explicit environment lookup without changing process globals.
    pub fn from_lookup(
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, crate::error::LambdaServiceError> {
        let error = |key: &str| crate::error::LambdaServiceError::InvalidParameter {
            message: format!("Invalid Lambda configuration: {key}"),
        };
        let boolean =
            |key: &str, default: bool| -> Result<bool, crate::error::LambdaServiceError> {
                match lookup(key).as_deref() {
                    None => Ok(default),
                    Some("true" | "1" | "yes") => Ok(true),
                    Some("false" | "0" | "no") => Ok(false),
                    Some(_) => Err(error(key)),
                }
            };
        let number =
            |key: &str, default: u64, max: u64| -> Result<u64, crate::error::LambdaServiceError> {
                let value = match lookup(key) {
                    None => default,
                    Some(raw) => raw.parse().map_err(|_| error(key))?,
                };
                if value == 0 || value > max {
                    return Err(error(key));
                }
                Ok(value)
            };
        let docker_enabled = boolean("LAMBDA_DOCKER_ENABLED", false)?;
        let executor = match lookup("LAMBDA_EXECUTOR") {
            Some(raw) => raw.parse().map_err(|_| error("LAMBDA_EXECUTOR"))?,
            None if docker_enabled => ExecutorBackend::Docker,
            None => ExecutorBackend::Disabled,
        };
        if docker_enabled && executor != ExecutorBackend::Docker {
            return Err(error("conflicting Docker/executor settings"));
        }
        if executor == ExecutorBackend::Docker {
            return Err(error("Docker execution is not supported by this build"));
        }
        let mut config = Self {
            executor,
            docker_enabled,
            skip_signature_validation: boolean("LAMBDA_SKIP_SIGNATURE_VALIDATION", true)?,
            port: u16::try_from(number("GATEWAY_PORT", 4566, 65_535)?)
                .map_err(|_| error("GATEWAY_PORT"))?,
            max_warm_instances: usize::try_from(number("LAMBDA_MAX_WARM_INSTANCES", 1, 1)?)
                .map_err(|_| error("LAMBDA_MAX_WARM_INSTANCES"))?,
            idle_timeout: Duration::from_secs(number("LAMBDA_IDLE_TIMEOUT_SECS", 600, 86_400)?),
            init_timeout: Duration::from_secs(number("LAMBDA_INIT_TIMEOUT_SECS", 5, 900)?),
            ..Self::default()
        };
        if let Some(value) = lookup("DEFAULT_REGION") {
            if value.is_empty()
                || value.len() > 64
                || !value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            {
                return Err(error("DEFAULT_REGION"));
            }
            config.default_region = value;
        }
        if let Some(value) = lookup("DEFAULT_ACCOUNT_ID") {
            if value.len() != 12 || !value.bytes().all(|b| b.is_ascii_digit()) {
                return Err(error("DEFAULT_ACCOUNT_ID"));
            }
            config.account_id = value;
        }
        if let Some(value) = lookup("GATEWAY_HOST") {
            if value.is_empty()
                || value.len() > 253
                || !value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-:[]".contains(&b))
            {
                return Err(error("GATEWAY_HOST"));
            }
            config.host = value;
        }
        config.squib = SquibExecutorConfig::from_env_reader(lookup)?;
        Ok(config)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_reject_invalid_configuration_without_fallback() {
        for (key, value) in [
            ("LAMBDA_EXECUTOR", "disable"),
            ("LAMBDA_SKIP_SIGNATURE_VALIDATION", "maybe"),
            ("GATEWAY_PORT", "0"),
            ("GATEWAY_PORT", "65536"),
            ("LAMBDA_INIT_TIMEOUT_SECS", "-1"),
            ("LAMBDA_MAX_WARM_INSTANCES", "999"),
            ("LAMBDA_SQUIB_STAGE_PORT", "oops"),
            ("LAMBDA_SQUIB_CONFIG_FILE", ""),
            ("DEFAULT_ACCOUNT_ID", "../account"),
        ] {
            assert!(
                LambdaConfig::from_lookup(|name| (name == key).then(|| value.into())).is_err(),
                "{key}"
            );
        }
        assert_eq!(
            LambdaConfig::from_lookup(|_| None).unwrap().executor,
            ExecutorBackend::Disabled
        );
        assert!(
            LambdaConfig::from_lookup(|key| match key {
                "LAMBDA_EXECUTOR" => Some("native".into()),
                "LAMBDA_DOCKER_ENABLED" => Some("true".into()),
                _ => None,
            })
            .is_err()
        );
    }

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
