//! SES service configuration.

use rustack_core::settings as env;

/// SES service configuration.
#[derive(Debug, Clone)]
pub struct SesConfig {
    /// Skip signature validation (default: true for local dev).
    pub skip_signature_validation: bool,
    /// Default AWS region.
    pub default_region: String,
    /// Default AWS account ID.
    pub default_account_id: String,
    /// Whether to require verified identities for sending.
    /// When false (default), any source address is accepted.
    /// When true, source must be verified via `VerifyEmailIdentity`/`VerifyDomainIdentity`.
    pub require_verified_identity: bool,
    /// Max sends per 24 hours (for `GetSendQuota`). Default: 200 (sandbox).
    pub max_24_hour_send: f64,
    /// Max send rate per second (for `GetSendQuota`). Default: 1.0 (sandbox).
    pub max_send_rate: f64,
}

impl SesConfig {
    /// Create configuration from environment variables.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            skip_signature_validation: env_bool("SES_SKIP_SIGNATURE_VALIDATION", true),
            default_region: env::var("DEFAULT_REGION").unwrap_or_else(|_| "us-east-1".to_owned()),
            default_account_id: env::var("DEFAULT_ACCOUNT_ID")
                .unwrap_or_else(|_| "000000000000".to_owned()),
            require_verified_identity: env_bool("SES_REQUIRE_VERIFIED_IDENTITY", false),
            max_24_hour_send: env_f64("SES_MAX_24_HOUR_SEND", 200.0),
            max_send_rate: env_f64("SES_MAX_SEND_RATE", 1.0),
        }
    }
}

impl Default for SesConfig {
    fn default() -> Self {
        Self {
            skip_signature_validation: true,
            default_region: "us-east-1".to_owned(),
            default_account_id: "000000000000".to_owned(),
            require_verified_identity: false,
            max_24_hour_send: 200.0,
            max_send_rate: 1.0,
        }
    }
}

fn env_bool(key: &str, default: bool) -> bool {
    env::var(key).map_or(default, |v| {
        v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes") || v == "1"
    })
}

fn env_f64(key: &str, default: f64) -> f64 {
    env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_create_default_config() {
        let config = SesConfig::default();
        assert!(config.skip_signature_validation);
        assert_eq!(config.default_region, "us-east-1");
        assert_eq!(config.default_account_id, "000000000000");
        assert!(!config.require_verified_identity);
        assert!((config.max_24_hour_send - 200.0).abs() < f64::EPSILON);
        assert!((config.max_send_rate - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_should_parse_env_bool_values() {
        assert!(env_bool("NONEXISTENT_VAR_12345", true));
        assert!(!env_bool("NONEXISTENT_VAR_12345", false));
    }

    #[test]
    fn test_should_parse_env_f64_values() {
        assert!((env_f64("NONEXISTENT_VAR_12345", 42.0) - 42.0).abs() < f64::EPSILON);
    }
}
