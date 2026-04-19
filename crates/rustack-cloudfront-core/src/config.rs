//! Runtime configuration for the CloudFront service.
//!
//! All configuration is env-driven to match the pattern used by other Rustack
//! services; no hot-reload.

use std::time::Duration;

/// Config for the CloudFront management plane.
#[derive(Debug, Clone)]
pub struct CloudFrontConfig {
    /// Whether to skip SigV4 signature validation on incoming requests.
    pub skip_signature_validation: bool,
    /// Region to report in ARNs (CloudFront is global but the SDK still signs
    /// with a region — we accept any).
    pub default_region: String,
    /// 12-digit account ID used in ARNs.
    pub account_id: String,
    /// Host suffix emitted in distribution `DomainName` fields.
    ///
    /// Defaults to `cloudfront.net`. Override with `CLOUDFRONT_DOMAIN_SUFFIX`.
    pub domain_suffix: String,
    /// Simulated propagation delay for `CreateDistribution` / `UpdateDistribution`
    /// (`InProgress` → `Deployed`).
    pub distribution_propagation: Duration,
    /// Simulated propagation delay for invalidations (`InProgress` → `Completed`).
    pub invalidation_propagation: Duration,
    /// Whether resource IDs are derived from a hash of their input rather than
    /// random. Useful for snapshot tests.
    pub deterministic_ids: bool,
}

impl Default for CloudFrontConfig {
    fn default() -> Self {
        Self {
            skip_signature_validation: true,
            default_region: "us-east-1".to_owned(),
            account_id: "000000000000".to_owned(),
            domain_suffix: "cloudfront.net".to_owned(),
            distribution_propagation: Duration::from_millis(0),
            invalidation_propagation: Duration::from_millis(0),
            deterministic_ids: false,
        }
    }
}

impl CloudFrontConfig {
    /// Load config from environment variables, falling back to defaults.
    #[must_use]
    pub fn from_env() -> Self {
        let mut cfg = Self::default();

        if let Ok(v) = std::env::var("CLOUDFRONT_SKIP_SIGNATURE_VALIDATION") {
            cfg.skip_signature_validation = parse_bool(&v).unwrap_or(cfg.skip_signature_validation);
        }
        if let Ok(v) = std::env::var("AWS_DEFAULT_REGION") {
            cfg.default_region = v;
        }
        if let Ok(v) =
            std::env::var("ACCOUNT_ID").or_else(|_| std::env::var("CLOUDFRONT_ACCOUNT_ID"))
        {
            cfg.account_id = v;
        }
        if let Ok(v) = std::env::var("CLOUDFRONT_DOMAIN_SUFFIX") {
            cfg.domain_suffix = v;
        }
        if let Ok(v) = std::env::var("CLOUDFRONT_DISTRIBUTION_PROPAGATION_MS") {
            if let Ok(ms) = v.parse::<u64>() {
                cfg.distribution_propagation = Duration::from_millis(ms);
            }
        }
        if let Ok(v) = std::env::var("CLOUDFRONT_INVALIDATION_PROPAGATION_MS") {
            if let Ok(ms) = v.parse::<u64>() {
                cfg.invalidation_propagation = Duration::from_millis(ms);
            }
        }
        if let Ok(v) = std::env::var("CLOUDFRONT_DETERMINISTIC_IDS") {
            cfg.deterministic_ids = parse_bool(&v).unwrap_or(false);
        }

        cfg
    }
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}
