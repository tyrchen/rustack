//! S3-specific configuration.
//!
//! Provides [`S3Config`] for configuring the RustStack S3 service.
//! Configuration values are loaded from environment variables, matching
//! LocalStack conventions for S3-specific settings.

use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

/// S3 service configuration.
///
/// All fields have sensible defaults matching LocalStack behavior. Configuration
/// can be loaded from environment variables via [`S3Config::from_env`].
///
/// # Examples
///
/// ```
/// use ruststack_s3_core::config::S3Config;
///
/// let config = S3Config::default();
/// assert_eq!(config.gateway_listen, "0.0.0.0:4566");
/// assert!(config.s3_virtual_hosting);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, TypedBuilder)]
#[serde(rename_all = "camelCase")]
pub struct S3Config {
    /// Bind address for the gateway (e.g. `"0.0.0.0:4566"`).
    #[builder(default = String::from("0.0.0.0:4566"))]
    pub gateway_listen: String,

    /// Whether S3 virtual-hosted-style addressing is enabled.
    #[builder(default = true)]
    pub s3_virtual_hosting: bool,

    /// Domain for S3 virtual hosting resolution.
    #[builder(default = String::from("s3.localhost.localstack.cloud"))]
    pub s3_domain: String,

    /// Whether to skip signature validation on incoming requests.
    #[builder(default = true)]
    pub s3_skip_signature_validation: bool,

    /// Maximum object size (in bytes) kept entirely in memory before spilling to disk.
    #[builder(default = 524_288)]
    pub s3_max_memory_object_size: usize,

    /// Default AWS region for this S3 service instance.
    #[builder(default = String::from("us-east-1"))]
    pub default_region: String,

    /// Log level filter string (e.g. `"info"`, `"debug"`).
    #[builder(default = String::from("info"))]
    pub log_level: String,

    /// Whether persistence (durable storage) is enabled.
    #[builder(default = false)]
    pub persistence: bool,

    /// Data directory used when persistence is enabled.
    #[builder(default = String::from("/var/lib/localstack"))]
    pub data_dir: String,
}

impl Default for S3Config {
    fn default() -> Self {
        Self {
            gateway_listen: String::from("0.0.0.0:4566"),
            s3_virtual_hosting: true,
            s3_domain: String::from("s3.localhost.localstack.cloud"),
            s3_skip_signature_validation: true,
            s3_max_memory_object_size: 524_288,
            default_region: String::from("us-east-1"),
            log_level: String::from("info"),
            persistence: false,
            data_dir: String::from("/var/lib/localstack"),
        }
    }
}

impl S3Config {
    /// Load configuration from environment variables.
    ///
    /// Reads the following environment variables (falling back to defaults):
    ///
    /// | Variable | Default |
    /// |----------|---------|
    /// | `GATEWAY_LISTEN` | `0.0.0.0:4566` |
    /// | `S3_VIRTUAL_HOSTING` | `true` |
    /// | `S3_DOMAIN` | `s3.localhost.localstack.cloud` |
    /// | `S3_SKIP_SIGNATURE_VALIDATION` | `true` |
    /// | `S3_MAX_MEMORY_OBJECT_SIZE` | `524288` |
    /// | `DEFAULT_REGION` | `us-east-1` |
    /// | `LOG_LEVEL` | `info` |
    /// | `PERSISTENCE` | `false` |
    /// | `DATA_DIR` | `/var/lib/localstack` |
    ///
    /// # Examples
    ///
    /// ```
    /// use ruststack_s3_core::config::S3Config;
    ///
    /// let config = S3Config::from_env();
    /// assert!(!config.gateway_listen.is_empty());
    /// ```
    #[must_use]
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(v) = std::env::var("GATEWAY_LISTEN") {
            config.gateway_listen = v;
        }
        if let Ok(v) = std::env::var("S3_VIRTUAL_HOSTING") {
            config.s3_virtual_hosting = parse_bool(&v);
        }
        if let Ok(v) = std::env::var("S3_DOMAIN") {
            config.s3_domain = v;
        }
        if let Ok(v) = std::env::var("S3_SKIP_SIGNATURE_VALIDATION") {
            config.s3_skip_signature_validation = parse_bool(&v);
        }
        if let Ok(v) = std::env::var("S3_MAX_MEMORY_OBJECT_SIZE") {
            if let Ok(n) = v.parse::<usize>() {
                config.s3_max_memory_object_size = n;
            }
        }
        if let Ok(v) = std::env::var("DEFAULT_REGION") {
            config.default_region = v;
        }
        if let Ok(v) = std::env::var("LOG_LEVEL") {
            config.log_level = v;
        }
        if let Ok(v) = std::env::var("PERSISTENCE") {
            config.persistence = parse_bool(&v);
        }
        if let Ok(v) = std::env::var("DATA_DIR") {
            config.data_dir = v;
        }

        config
    }
}

/// Parse a string as a boolean, accepting `"1"` and `"true"` (case-insensitive).
fn parse_bool(value: &str) -> bool {
    value == "1" || value.eq_ignore_ascii_case("true")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_create_default_config() {
        let config = S3Config::default();
        assert_eq!(config.gateway_listen, "0.0.0.0:4566");
        assert!(config.s3_virtual_hosting);
        assert_eq!(config.s3_domain, "s3.localhost.localstack.cloud");
        assert!(config.s3_skip_signature_validation);
        assert_eq!(config.s3_max_memory_object_size, 524_288);
        assert_eq!(config.default_region, "us-east-1");
        assert_eq!(config.log_level, "info");
        assert!(!config.persistence);
        assert_eq!(config.data_dir, "/var/lib/localstack");
    }

    #[test]
    fn test_should_load_from_env() {
        let config = S3Config::from_env();
        assert!(!config.gateway_listen.is_empty());
    }

    #[test]
    fn test_should_build_with_typed_builder() {
        let config = S3Config::builder()
            .gateway_listen("127.0.0.1:9999".into())
            .s3_virtual_hosting(false)
            .s3_domain("custom.domain".into())
            .s3_skip_signature_validation(false)
            .s3_max_memory_object_size(1024)
            .default_region("eu-west-1".into())
            .log_level("debug".into())
            .persistence(true)
            .data_dir("/tmp/data".into())
            .build();

        assert_eq!(config.gateway_listen, "127.0.0.1:9999");
        assert!(!config.s3_virtual_hosting);
        assert_eq!(config.s3_domain, "custom.domain");
        assert!(!config.s3_skip_signature_validation);
        assert_eq!(config.s3_max_memory_object_size, 1024);
        assert_eq!(config.default_region, "eu-west-1");
        assert_eq!(config.log_level, "debug");
        assert!(config.persistence);
        assert_eq!(config.data_dir, "/tmp/data");
    }

    #[test]
    fn test_should_serialize_to_camel_case_json() {
        let config = S3Config::default();
        let json = serde_json::to_string(&config).expect("test serialization");
        assert!(json.contains("gatewayListen"));
        assert!(json.contains("s3VirtualHosting"));
    }

    #[test]
    fn test_should_parse_bool_values() {
        assert!(parse_bool("1"));
        assert!(parse_bool("true"));
        assert!(parse_bool("TRUE"));
        assert!(parse_bool("True"));
        assert!(!parse_bool("0"));
        assert!(!parse_bool("false"));
        assert!(!parse_bool(""));
    }
}
