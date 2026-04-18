//! ARN helpers for CloudFront.
//!
//! CloudFront is a global service: its ARNs omit the region segment.
//! `arn:aws:cloudfront::{account}:distribution/{id}`

/// Construct a distribution ARN.
#[must_use]
pub fn distribution_arn(account_id: &str, distribution_id: &str) -> String {
    format!("arn:aws:cloudfront::{account_id}:distribution/{distribution_id}")
}

/// Construct an OAC ARN.
#[must_use]
pub fn origin_access_control_arn(account_id: &str, id: &str) -> String {
    format!("arn:aws:cloudfront::{account_id}:origin-access-control/{id}")
}

/// Construct an OAI ARN (same shape for identifying).
#[must_use]
pub fn oai_arn(account_id: &str, id: &str) -> String {
    format!("arn:aws:cloudfront::{account_id}:origin-access-identity/{id}")
}

/// Construct a cache policy ARN.
#[must_use]
pub fn cache_policy_arn(account_id: &str, id: &str) -> String {
    format!("arn:aws:cloudfront::{account_id}:cache-policy/{id}")
}

/// Construct a response-headers-policy ARN.
#[must_use]
pub fn response_headers_policy_arn(account_id: &str, id: &str) -> String {
    format!("arn:aws:cloudfront::{account_id}:response-headers-policy/{id}")
}

/// Construct an origin-request-policy ARN.
#[must_use]
pub fn origin_request_policy_arn(account_id: &str, id: &str) -> String {
    format!("arn:aws:cloudfront::{account_id}:origin-request-policy/{id}")
}

/// Construct a function ARN.
#[must_use]
pub fn function_arn(account_id: &str, name: &str) -> String {
    format!("arn:aws:cloudfront::{account_id}:function/{name}")
}

/// Construct a key-group ARN.
#[must_use]
pub fn key_group_arn(account_id: &str, id: &str) -> String {
    format!("arn:aws:cloudfront::{account_id}:key-group/{id}")
}

/// Construct a public-key ARN.
#[must_use]
pub fn public_key_arn(account_id: &str, id: &str) -> String {
    format!("arn:aws:cloudfront::{account_id}:public-key/{id}")
}

/// Construct a realtime-log ARN.
#[must_use]
pub fn realtime_log_arn(account_id: &str, name: &str) -> String {
    format!("arn:aws:cloudfront::{account_id}:realtime-log-config/{name}")
}

/// Construct a KVS ARN.
#[must_use]
pub fn kvs_arn(account_id: &str, id: &str) -> String {
    format!("arn:aws:cloudfront::{account_id}:key-value-store/{id}")
}

/// Extract the distribution ID from a distribution ARN.
#[must_use]
pub fn extract_distribution_id(arn: &str) -> Option<&str> {
    arn.rsplit_once("distribution/").map(|(_, id)| id)
}
