//! Data-plane runtime config.

use std::time::Duration;

/// Data-plane runtime config.
#[derive(Debug, Clone)]
pub struct DataPlaneConfig {
    /// Host suffix used for host-header routing (default `cloudfront.net`).
    pub domain_suffix: String,
    /// When true, requests that would invoke Lambda\@Edge / Function / signed
    /// URL validation hard-fail with 500 instead of emitting a warning and
    /// passing through.
    pub fail_on_function: bool,
    /// Include `x-amz-meta-*` S3 object headers on responses.
    pub forward_user_metadata: bool,
    /// Max upstream response body buffered before returning 413.
    pub max_upstream_body_bytes: usize,
    /// Default custom HTTP origin read timeout.
    pub http_origin_timeout: Duration,
    /// Minimum interval between duplicate divergence warnings per distribution.
    pub divergence_log_interval: Duration,
}

impl Default for DataPlaneConfig {
    fn default() -> Self {
        Self {
            domain_suffix: "cloudfront.net".to_owned(),
            fail_on_function: false,
            forward_user_metadata: false,
            max_upstream_body_bytes: 64 * 1024 * 1024,
            http_origin_timeout: Duration::from_secs(30),
            divergence_log_interval: Duration::from_secs(60),
        }
    }
}

impl DataPlaneConfig {
    /// Load config from environment variables.
    #[must_use]
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        if let Ok(v) = std::env::var("CLOUDFRONT_DOMAIN_SUFFIX") {
            cfg.domain_suffix = v;
        }
        if let Ok(v) = std::env::var("CLOUDFRONT_FAIL_ON_FUNCTION") {
            cfg.fail_on_function = matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            );
        }
        if let Ok(v) = std::env::var("CLOUDFRONT_FORWARD_USER_METADATA") {
            cfg.forward_user_metadata = matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            );
        }
        if let Ok(v) = std::env::var("CLOUDFRONT_MAX_UPSTREAM_BODY_BYTES") {
            if let Ok(n) = v.parse() {
                cfg.max_upstream_body_bytes = n;
            }
        }
        if let Ok(v) = std::env::var("CLOUDFRONT_HTTP_ORIGIN_TIMEOUT_MS") {
            if let Ok(ms) = v.parse() {
                cfg.http_origin_timeout = Duration::from_millis(ms);
            }
        }
        if let Ok(v) = std::env::var("CLOUDFRONT_DIVERGENCE_LOG_INTERVAL_MS") {
            if let Ok(ms) = v.parse() {
                cfg.divergence_log_interval = Duration::from_millis(ms);
            }
        }
        cfg
    }
}
