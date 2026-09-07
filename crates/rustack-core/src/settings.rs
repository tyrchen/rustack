//! Validated, immutable YAML and environment configuration shared by services.
//!
//! The binary installs settings once before constructing providers. Library users
//! may use [`ValidatedSettings::parse`] without modifying process globals. Values
//! from the process environment override YAML, and diagnostics never print values.

pub use std::env::VarError;
use std::{collections::BTreeMap, env, fmt, net::SocketAddr, sync::OnceLock, time::Duration};

use config::{Config, File, FileFormat};
use serde::Deserialize;
use serde_json::Value;
use tokio::io::AsyncReadExt;
use url::Url;

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const SERVICE_NAMES: &[&str] = &[
    "S3",
    "DYNAMODB",
    "DYNAMODBSTREAMS",
    "SQS",
    "SSM",
    "SNS",
    "LAMBDA",
    "EVENTS",
    "LOGS",
    "KMS",
    "KINESIS",
    "SECRETSMANAGER",
    "SES",
    "APIGATEWAYV2",
    "CLOUDWATCH",
    "IAM",
    "STS",
    "CLOUDFRONT",
];
const KEYS: &[&str] = &[
    "GATEWAY_LISTEN",
    "GATEWAY_HOST",
    "GATEWAY_PORT",
    "RUSTACK_ADVERTISED_ENDPOINT",
    "SERVICES",
    "DEFAULT_REGION",
    "AWS_DEFAULT_REGION",
    "DEFAULT_ACCOUNT_ID",
    "ACCOUNT_ID",
    "ACCESS_KEY",
    "SECRET_KEY",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "LOG_LEVEL",
    "RUST_LOG",
    "PERSISTENCE",
    "DATA_DIR",
    "S3_VIRTUAL_HOSTING",
    "S3_DOMAIN",
    "S3_MAX_MEMORY_OBJECT_SIZE",
    "LAMBDA_DOCKER_ENABLED",
    "LAMBDA_EXECUTOR",
    "LAMBDA_MAX_WARM_INSTANCES",
    "LAMBDA_IDLE_TIMEOUT_SECS",
    "LAMBDA_INIT_TIMEOUT_SECS",
    "LAMBDA_SQUIB_INSTANCE_ID",
    "LAMBDA_SQUIB_CONFIG_FILE",
    "LAMBDA_SQUIB_VSOCK_PATH",
    "LAMBDA_SQUIB_STAGE_PORT",
    "LAMBDA_SQUIB_CONNECT_TIMEOUT_MS",
    "LAMBDA_SQUIB_RESPONSE_LIMIT_BYTES",
    "LAMBDA_SQUIB_RUN_BUDGET_SECS",
    "LAMBDA_SQUIB_SHUTDOWN_TIMEOUT_MS",
    "RUSTACK_WORKSPACE_ROOT",
    "RUSTACK_SNAPSHOT_DIR",
    "RUSTACK_SNAPSHOT_PERF_FILE",
    "EVENTS_HOST",
    "EVENTS_PORT",
    "LOGS_HOST",
    "LOGS_PORT",
    "CLOUDFRONT_ACCOUNT_ID",
    "CLOUDFRONT_DOMAIN_SUFFIX",
    "CLOUDFRONT_DISTRIBUTION_PROPAGATION_MS",
    "CLOUDFRONT_INVALIDATION_PROPAGATION_MS",
    "CLOUDFRONT_DETERMINISTIC_IDS",
    "CLOUDFRONT_FAIL_ON_FUNCTION",
    "CLOUDFRONT_FORWARD_USER_METADATA",
    "CLOUDFRONT_MAX_UPSTREAM_BODY_BYTES",
    "CLOUDFRONT_HTTP_ORIGIN_TIMEOUT_MS",
    "CLOUDFRONT_DIVERGENCE_LOG_INTERVAL_MS",
    "CLOUDWATCH_MAX_RETENTION_SECONDS",
    "CLOUDWATCH_MAX_POINTS_PER_SERIES",
    "DYNAMODBSTREAMS_MAX_RECORDS_PER_SHARD",
    "DYNAMODBSTREAMS_MAX_RECORD_AGE_SECONDS",
    "SES_REQUIRE_VERIFIED_IDENTITY",
    "SES_MAX_24_HOUR_SEND",
    "SES_MAX_SEND_RATE",
];

static SETTINGS: OnceLock<ValidatedSettings> = OnceLock::new();

/// Positive, bounded runtime resource limits. YAML names are camelCase.
#[derive(Debug, Clone, Deserialize, typed_builder::TypedBuilder)]
#[non_exhaustive]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeBudgets {
    /// Maximum live HTTP connections.
    pub connections: usize,
    /// Maximum concurrently active business requests, including response bodies.
    pub requests: usize,
    /// HTTP header deadline in seconds.
    pub header_seconds: u64,
    /// Control-plane operation deadline in seconds.
    pub request_seconds: u64,
    /// Synchronous Lambda execution deadline in seconds (may exceed 900 while an Invoke runs).
    pub lambda_invoke_seconds: u64,
    /// Total graceful shutdown deadline in seconds.
    pub shutdown_seconds: u64,
    /// Maximum aggregate control-plane body bytes.
    pub control_body_bytes: u64,
    /// Maximum Lambda ZIP JSON envelope bytes.
    pub lambda_code_body_bytes: u64,
    /// Maximum upstream response bytes.
    pub upstream_body_bytes: u64,
    /// Body inactivity deadline in seconds.
    pub body_idle_seconds: u64,
    /// Control-plane body total deadline in seconds.
    pub body_total_seconds: u64,
    /// S3 streaming body total deadline in seconds.
    pub s3_body_total_seconds: u64,
    /// Maximum S3 object bytes, without aggregate collection.
    pub s3_object_body_bytes: u64,
}

impl Default for RuntimeBudgets {
    fn default() -> Self {
        Self {
            connections: 256,
            requests: 128,
            header_seconds: 5,
            request_seconds: 30,
            lambda_invoke_seconds: 930,
            shutdown_seconds: 30,
            control_body_bytes: 16 * 1024 * 1024,
            lambda_code_body_bytes: 96 * 1024 * 1024,
            upstream_body_bytes: 64 * 1024 * 1024,
            body_idle_seconds: 5,
            body_total_seconds: 30,
            s3_body_total_seconds: 3600,
            s3_object_body_bytes: 5 * 1024 * 1024 * 1024,
        }
    }
}

impl RuntimeBudgets {
    /// Validate budgets before allocating semaphore capacity or timers.
    ///
    /// # Errors
    /// Rejects zero, excessive, or inconsistent resource deadlines and sizes.
    pub fn validate(&self) -> Result<(), SettingsError> {
        for (key, value, max) in [
            ("budgets.connections", self.connections as u64, 65_536),
            ("budgets.requests", self.requests as u64, 65_536),
            ("budgets.headerSeconds", self.header_seconds, 300),
            ("budgets.requestSeconds", self.request_seconds, 3600),
            (
                "budgets.lambdaInvokeSeconds",
                self.lambda_invoke_seconds,
                3600,
            ),
            ("budgets.shutdownSeconds", self.shutdown_seconds, 3600),
            (
                "budgets.controlBodyBytes",
                self.control_body_bytes,
                16 * 1024 * 1024,
            ),
            (
                "budgets.lambdaCodeBodyBytes",
                self.lambda_code_body_bytes,
                96 * 1024 * 1024,
            ),
            (
                "budgets.upstreamBodyBytes",
                self.upstream_body_bytes,
                64 * 1024 * 1024,
            ),
            ("budgets.bodyIdleSeconds", self.body_idle_seconds, 300),
            ("budgets.bodyTotalSeconds", self.body_total_seconds, 3600),
            (
                "budgets.s3BodyTotalSeconds",
                self.s3_body_total_seconds,
                3600,
            ),
            (
                "budgets.s3ObjectBodyBytes",
                self.s3_object_body_bytes,
                5 * 1024 * 1024 * 1024,
            ),
        ] {
            if value == 0 || value > max {
                return Err(invalid(key, "outside supported nonzero range"));
            }
        }
        if self.body_idle_seconds > self.body_total_seconds
            || self.body_idle_seconds > self.s3_body_total_seconds
        {
            return Err(invalid(
                "budgets.bodyIdleSeconds",
                "idle deadline must not exceed total deadline",
            ));
        }
        Ok(())
    }
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct Document {
    environment: BTreeMap<String, Value>,
    budgets: RuntimeBudgets,
    advertised_endpoint: Option<String>,
}

/// Configuration failure without secret-bearing value diagnostics.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    /// Invalid or unknown operator setting.
    #[error("invalid configuration key {key}: {reason}")]
    Invalid {
        /// Name of the invalid key, never its value.
        key: String,
        /// Non-sensitive explanation.
        reason: &'static str,
    },
    /// YAML cannot be parsed according to the configuration schema.
    #[error("invalid YAML configuration schema (values redacted)")]
    Document,
    /// Configuration source IO failed.
    #[error("cannot read configuration source")]
    Io(#[source] std::io::Error),
    /// Config loading exceeded its deadline.
    #[error("configuration read deadline exceeded")]
    Timeout,
    /// Reinitializing the process configuration is not supported.
    #[error("runtime configuration was already installed")]
    AlreadyInstalled,
}

fn invalid(key: &str, reason: &'static str) -> SettingsError {
    SettingsError::Invalid {
        key: key.to_owned(),
        reason,
    }
}

/// Fully validated settings. Debug prints keys, not potentially secret values.
#[derive(Clone)]
pub struct ValidatedSettings {
    values: BTreeMap<String, String>,
    budgets: RuntimeBudgets,
    listen: SocketAddr,
    advertised_endpoint: String,
}

impl fmt::Debug for ValidatedSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ValidatedSettings")
            .field("keys", &self.values.keys().collect::<Vec<_>>())
            .field("budgets", &self.budgets)
            .field("listen", &self.listen)
            .finish_non_exhaustive()
    }
}

impl ValidatedSettings {
    /// Parse YAML and explicit environment overrides, without touching globals.
    ///
    /// # Errors
    /// Rejects unknown YAML keys, invalid scalars, unsafe ranges, and invalid addresses.
    pub fn parse(yaml: &str, overrides: &BTreeMap<String, String>) -> Result<Self, SettingsError> {
        let document: Document = if yaml.trim().is_empty() {
            Document::default()
        } else {
            Config::builder()
                .add_source(File::from_str(yaml, FileFormat::Yaml))
                .build()
                .map_err(|_| SettingsError::Document)?
                .try_deserialize()
                .map_err(|_| SettingsError::Document)?
        };
        document.budgets.validate()?;
        let mut values = BTreeMap::new();
        for (key, value) in document.environment {
            if !known_key(&key) {
                return Err(invalid(&key, "unknown setting"));
            }
            let text = match value {
                Value::String(text) => text,
                Value::Bool(value) => value.to_string(),
                Value::Number(value) => value.to_string(),
                _ => return Err(invalid(&key, "expected a scalar value")),
            };
            values.insert(key, text);
        }
        for (key, value) in overrides {
            if known_key(key) {
                values.insert(key.clone(), value.clone());
            }
        }
        for (key, value) in &mut values {
            validate_value(key, value)?;
        }
        normalize_aliases(
            &mut values,
            &["DEFAULT_REGION", "AWS_DEFAULT_REGION"],
            Some("us-east-1"),
        )?;
        normalize_aliases(
            &mut values,
            &["DEFAULT_ACCOUNT_ID", "ACCOUNT_ID", "CLOUDFRONT_ACCOUNT_ID"],
            Some("000000000000"),
        )?;
        normalize_aliases(&mut values, &["ACCESS_KEY", "AWS_ACCESS_KEY_ID"], None)?;
        normalize_aliases(&mut values, &["SECRET_KEY", "AWS_SECRET_ACCESS_KEY"], None)?;
        let listen: SocketAddr = values
            .get("GATEWAY_LISTEN")
            .map_or("127.0.0.1:4566", String::as_str)
            .parse()
            .map_err(|_| invalid("GATEWAY_LISTEN", "expected IP address and nonzero port"))?;
        if listen.port() == 0 {
            return Err(invalid("GATEWAY_LISTEN", "port must not be zero"));
        }
        values
            .entry("GATEWAY_LISTEN".to_owned())
            .or_insert_with(|| listen.to_string());
        let default_host = if listen.ip().is_unspecified() {
            "localhost".to_owned()
        } else {
            listen.ip().to_string()
        };
        let host = values
            .entry("GATEWAY_HOST".to_owned())
            .or_insert(default_host)
            .clone();
        let port = values
            .entry("GATEWAY_PORT".to_owned())
            .or_insert_with(|| listen.port().to_string())
            .clone();
        let host = if host.contains(':') && !host.starts_with('[') {
            format!("[{host}]")
        } else {
            host
        };
        values.insert("GATEWAY_HOST".to_owned(), host.clone());
        let advertised_endpoint = values
            .get("RUSTACK_ADVERTISED_ENDPOINT")
            .cloned()
            .or(document.advertised_endpoint)
            .unwrap_or_else(|| format!("http://{host}:{port}"));
        let endpoint = validate_endpoint(&advertised_endpoint)?;
        let endpoint_host = endpoint
            .host_str()
            .ok_or_else(|| invalid("advertisedEndpoint", "missing host"))?;
        let endpoint_port = endpoint
            .port_or_known_default()
            .ok_or_else(|| invalid("advertisedEndpoint", "missing port"))?;
        values.insert("GATEWAY_HOST".to_owned(), endpoint_host.to_owned());
        values.insert("GATEWAY_PORT".to_owned(), endpoint_port.to_string());
        let advertised_endpoint = endpoint.origin().ascii_serialization();
        Ok(Self {
            values,
            budgets: document.budgets,
            listen,
            advertised_endpoint,
        })
    }

    /// Effective immutable resource budgets.
    #[must_use]
    pub fn budgets(&self) -> &RuntimeBudgets {
        &self.budgets
    }

    /// Effective bind address, including the validated nonzero port.
    #[must_use]
    pub const fn listen(&self) -> SocketAddr {
        self.listen
    }

    /// Explicit local public endpoint, never derived from an incoming Host header.
    #[must_use]
    pub fn advertised_endpoint(&self) -> &str {
        &self.advertised_endpoint
    }

    /// Lookup a validated value for provider construction.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    /// Reject strict authentication without a complete static credential pair.
    ///
    /// # Errors
    /// Returns a redacted error if any enabled strict service lacks credentials.
    pub fn validate_credentials(&self, enabled: &[String]) -> Result<(), SettingsError> {
        let strict = enabled.iter().any(|service| {
            self.get(&format!(
                "{}_SKIP_SIGNATURE_VALIDATION",
                service.to_ascii_uppercase()
            )) == Some("false")
        });
        if strict {
            let access = self
                .get("ACCESS_KEY")
                .or_else(|| self.get("AWS_ACCESS_KEY_ID"));
            let secret = self
                .get("SECRET_KEY")
                .or_else(|| self.get("AWS_SECRET_ACCESS_KEY"));
            if access.is_none_or(str::is_empty) || secret.is_none_or(str::is_empty) {
                return Err(invalid(
                    "credentials",
                    "strict signature validation requires both access and secret keys",
                ));
            }
        }
        Ok(())
    }
}

fn known_key(key: &str) -> bool {
    KEYS.contains(&key)
        || key
            .strip_suffix("_SKIP_SIGNATURE_VALIDATION")
            .is_some_and(|prefix| SERVICE_NAMES.contains(&prefix))
}

fn validate_value(key: &str, value: &mut String) -> Result<(), SettingsError> {
    if value.is_empty() && key != "SERVICES" {
        return Err(invalid(key, "must not be empty"));
    }
    if value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(invalid(
            key,
            "exceeds byte limit or contains control characters",
        ));
    }
    let boolean = key.ends_with("_SKIP_SIGNATURE_VALIDATION")
        || matches!(
            key,
            "PERSISTENCE"
                | "S3_VIRTUAL_HOSTING"
                | "LAMBDA_DOCKER_ENABLED"
                | "CLOUDFRONT_DETERMINISTIC_IDS"
                | "CLOUDFRONT_FAIL_ON_FUNCTION"
                | "CLOUDFRONT_FORWARD_USER_METADATA"
                | "SES_REQUIRE_VERIFIED_IDENTITY"
        );
    if boolean {
        *value = match value.to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "1" => "true".to_owned(),
            "false" | "no" | "off" | "0" => "false".to_owned(),
            _ => return Err(invalid(key, "expected boolean")),
        };
    } else if key == "LAMBDA_EXECUTOR" {
        if !matches!(
            value.as_str(),
            "disabled" | "native" | "auto" | "docker" | "squib"
        ) {
            return Err(invalid(key, "unknown executor"));
        }
    } else if key.ends_with("ACCOUNT_ID") || key == "ACCOUNT_ID" {
        if value.len() != 12 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(invalid(key, "expected 12 ASCII digits"));
        }
    } else if key.ends_with("REGION") {
        if value.len() > 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(invalid(key, "invalid region identifier"));
        }
    } else if key.ends_with("_PORT") {
        let port = value
            .parse::<u16>()
            .map_err(|_| invalid(key, "expected valid port"))?;
        if port == 0 {
            return Err(invalid(key, "port must not be zero"));
        }
    } else if key == "SES_MAX_24_HOUR_SEND" || key == "SES_MAX_SEND_RATE" {
        let number = value
            .parse::<f64>()
            .map_err(|_| invalid(key, "expected finite positive quota"))?;
        if !number.is_finite() || number <= 0.0 || number > 1_000_000_000.0 {
            return Err(invalid(key, "outside supported quota range"));
        }
    } else if key.ends_with("_MS")
        || key.ends_with("_SECS")
        || key.ends_with("_SECONDS")
        || key.contains("_MAX_")
        || key.ends_with("_LIMIT_BYTES")
    {
        let number = value
            .parse::<u64>()
            .map_err(|_| invalid(key, "expected bounded nonnegative integer"))?;
        let zero_allowed = key.contains("PROPAGATION") || key == "S3_MAX_MEMORY_OBJECT_SIZE";
        let max = if key.ends_with("_MS") {
            86_400_000
        } else if key.ends_with("_SECS") || key.ends_with("_SECONDS") {
            31_536_000
        } else {
            5 * 1024 * 1024 * 1024
        };
        if (!zero_allowed && number == 0) || number > max {
            return Err(invalid(key, "outside supported range"));
        }
    } else if key.ends_with("_HOST") || key.ends_with("_DOMAIN") || key.ends_with("_DOMAIN_SUFFIX")
    {
        if value.len() > 253
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b".-:[]".contains(&byte))
        {
            return Err(invalid(key, "invalid hostname or address"));
        }
    }
    Ok(())
}

fn normalize_aliases(
    values: &mut BTreeMap<String, String>,
    keys: &[&str],
    default: Option<&str>,
) -> Result<(), SettingsError> {
    let mut selected = None;
    for key in keys {
        if let Some(value) = values.get(*key) {
            if selected.as_ref().is_some_and(|current| current != value) {
                return Err(invalid(key, "conflicting aliases"));
            }
            selected = Some(value.clone());
        }
    }
    if let Some(selected) = selected.or_else(|| default.map(str::to_owned)) {
        for key in keys {
            values.insert((*key).to_owned(), selected.clone());
        }
    }
    Ok(())
}

fn validate_endpoint(endpoint: &str) -> Result<Url, SettingsError> {
    if endpoint.len() > 300
        || endpoint
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b' ')
    {
        return Err(invalid(
            "advertisedEndpoint",
            "invalid URL length or characters",
        ));
    }
    let parsed = Url::parse(endpoint).map_err(|_| invalid("advertisedEndpoint", "invalid URL"))?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || parsed.port() == Some(0)
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(invalid(
            "advertisedEndpoint",
            "expected http(s) authority without credentials, path, query or fragment",
        ));
    }
    Ok(parsed)
}

/// Read bounded YAML and freeze process configuration before providers start.
///
/// # Errors
/// Returns configuration, IO, timeout, or duplicate-install errors.
pub async fn initialize() -> Result<&'static ValidatedSettings, SettingsError> {
    let yaml = match env::var("RUSTACK_CONFIG") {
        Ok(path) => {
            let read = async {
                let file = tokio::fs::File::open(path)
                    .await
                    .map_err(SettingsError::Io)?;
                let mut text = String::new();
                file.take(MAX_CONFIG_BYTES + 1)
                    .read_to_string(&mut text)
                    .await
                    .map_err(SettingsError::Io)?;
                if text.len() as u64 > MAX_CONFIG_BYTES {
                    return Err(invalid("RUSTACK_CONFIG", "configuration exceeds 1 MiB"));
                }
                Ok(text)
            };
            tokio::time::timeout(Duration::from_secs(5), read)
                .await
                .map_err(|_| SettingsError::Timeout)??
        }
        Err(VarError::NotPresent) => String::new(),
        Err(_) => return Err(invalid("RUSTACK_CONFIG", "expected UTF-8 path")),
    };
    let mut overrides = BTreeMap::new();
    for key in KEYS.iter().map(|key| (*key).to_owned()).chain(
        SERVICE_NAMES
            .iter()
            .map(|key| format!("{key}_SKIP_SIGNATURE_VALIDATION")),
    ) {
        match env::var(&key) {
            Ok(value) => {
                overrides.insert(key, value);
            }
            Err(VarError::NotPresent) => {}
            Err(_) => return Err(invalid(&key, "expected UTF-8 value")),
        }
    }
    let settings = ValidatedSettings::parse(&yaml, &overrides)?;
    SETTINGS
        .set(settings)
        .map_err(|_| SettingsError::AlreadyInstalled)?;
    SETTINGS.get().ok_or(SettingsError::AlreadyInstalled)
}

/// Get an effective value; library use without installation reads the environment.
///
/// # Errors
/// Matches `std::env::var` for absent or non-Unicode values.
pub fn var(key: &str) -> Result<String, VarError> {
    match SETTINGS.get() {
        Some(settings) => settings
            .get(key)
            .map(str::to_owned)
            .ok_or(VarError::NotPresent),
        None => env::var(key),
    }
}

/// Effective budgets, or defaults for independently constructed library services.
#[must_use]
pub fn budgets() -> RuntimeBudgets {
    SETTINGS
        .get()
        .map_or_else(RuntimeBudgets::default, |settings| settings.budgets.clone())
}

/// Installed public endpoint, when configured by the application.
#[must_use]
pub fn advertised_endpoint() -> Option<&'static str> {
    SETTINGS.get().map(ValidatedSettings::advertised_endpoint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_apply_environment_over_yaml_and_derive_ipv6_endpoint()
    -> Result<(), SettingsError> {
        let overrides = BTreeMap::from([("GATEWAY_LISTEN".to_owned(), "[::1]:4567".to_owned())]);
        let config = ValidatedSettings::parse(
            "environment:\n  GATEWAY_LISTEN: 127.0.0.1:4566\n",
            &overrides,
        )?;
        assert_eq!(config.get("GATEWAY_PORT"), Some("4567"));
        assert_eq!(config.advertised_endpoint(), "http://[::1]:4567");
        Ok(())
    }

    #[test]
    fn test_should_reject_unknown_keys_bad_values_and_zero_budgets() {
        for yaml in [
            "environment:\n  LAMBDA_EXECUTOR: disable",
            "environment:\n  GATEWAY_PORT: 0",
            "environment:\n  S3_SKIP_SIGNATURE_VALIDATION: maybe",
            "environment:\n  DYNAMDB_SKIP_SIGNATURE_VALIDATION: true",
            "budgets:\n  requests: 0",
            "unknown: true",
        ] {
            assert!(ValidatedSettings::parse(yaml, &BTreeMap::new()).is_err());
        }
    }

    #[test]
    fn test_should_fail_closed_without_credentials_and_redact_debug() -> Result<(), SettingsError> {
        let config = ValidatedSettings::parse(
            "environment:\n  SQS_SKIP_SIGNATURE_VALIDATION: false\n  ACCESS_KEY: sample\n  \
             SECRET_KEY: sensitive-value",
            &BTreeMap::new(),
        )?;
        config.validate_credentials(&["sqs".to_owned()])?;
        assert!(!format!("{config:?}").contains("sensitive-value"));
        let missing = ValidatedSettings::parse(
            "environment:\n  SQS_SKIP_SIGNATURE_VALIDATION: false",
            &BTreeMap::new(),
        )?;
        assert!(missing.validate_credentials(&["sqs".to_owned()]).is_err());
        Ok(())
    }
}
