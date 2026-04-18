//! AWS-managed CloudFront policies seeded at provider construction.
//!
//! The IDs and shapes mirror the real AWS managed policies so Terraform /
//! CDK templates referencing them resolve. Only a subset is modelled — enough
//! to cover the ~95% of real-world configs.

use chrono::{TimeZone, Utc};
use rustack_cloudfront_model::{
    CachePolicy, CachePolicyConfig, CachePolicyCookiesConfig, CachePolicyHeadersConfig,
    CachePolicyQueryStringsConfig, OriginRequestPolicy, OriginRequestPolicyConfig,
    OriginRequestPolicyCookiesConfig, OriginRequestPolicyHeadersConfig,
    OriginRequestPolicyQueryStringsConfig, ParamsInCacheKey, ResponseHeadersPolicy,
    ResponseHeadersPolicyConfig,
};

/// Build a fixed AWS epoch timestamp for managed policy `LastModifiedTime`.
fn managed_timestamp() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2020, 5, 31, 0, 0, 0).unwrap()
}

/// Managed cache policies seeded into the store.
#[must_use]
pub fn managed_cache_policies() -> Vec<CachePolicy> {
    vec![
        managed_cache_policy(
            "658327ea-f89d-4fab-a63d-7e88639e58f6",
            "Managed-CachingOptimized",
            "Policy with caching enabled. Supports Gzip and Brotli compression.",
            86400,
            31_536_000,
            1,
        ),
        managed_cache_policy(
            "4135ea2d-6df8-44a3-9df3-4b5a84be39ad",
            "Managed-CachingDisabled",
            "Policy with caching disabled. All requests are sent to the origin.",
            0,
            0,
            0,
        ),
        managed_cache_policy(
            "83da9c7e-98b4-4e11-a168-04f0df8e2c65",
            "Managed-CachingOptimizedForUncompressedObjects",
            "Policy with caching enabled for uncompressed objects.",
            86400,
            31_536_000,
            1,
        ),
        managed_cache_policy(
            "08627262-05a9-4f76-9ded-b50ca2e3a84f",
            "Managed-Elemental-MediaPackage",
            "Policy for use with AWS Elemental MediaPackage.",
            0,
            86400,
            0,
        ),
    ]
}

fn managed_cache_policy(
    id: &str,
    name: &str,
    comment: &str,
    default_ttl: i64,
    max_ttl: i64,
    min_ttl: i64,
) -> CachePolicy {
    CachePolicy {
        id: id.to_owned(),
        last_modified_time: managed_timestamp(),
        config: CachePolicyConfig {
            comment: comment.to_owned(),
            name: name.to_owned(),
            default_ttl,
            max_ttl,
            min_ttl,
            parameters_in_cache_key_and_forwarded_to_origin: ParamsInCacheKey {
                enable_accept_encoding_gzip: true,
                enable_accept_encoding_brotli: true,
                headers_config: CachePolicyHeadersConfig {
                    header_behavior: "none".to_owned(),
                    headers: Vec::new(),
                },
                cookies_config: CachePolicyCookiesConfig {
                    cookie_behavior: "none".to_owned(),
                    cookies: Vec::new(),
                },
                query_strings_config: CachePolicyQueryStringsConfig {
                    query_string_behavior: "none".to_owned(),
                    query_strings: Vec::new(),
                },
            },
        },
        etag: "MANAGED_CACHE_POLICY_ETAG".to_owned(),
        managed: true,
    }
}

/// Managed origin request policies.
#[must_use]
pub fn managed_origin_request_policies() -> Vec<OriginRequestPolicy> {
    vec![
        managed_orp(
            "216adef6-5c7f-47e4-b989-5492eafa07d3",
            "Managed-AllViewer",
            "Forwards all values from the viewer to the origin.",
            "allViewer",
            "all",
            "all",
        ),
        managed_orp(
            "b689b0a8-53d0-40ab-baf2-68738e2966ac",
            "Managed-AllViewerAndCloudFrontHeaders-2022-06",
            "Forwards all values plus CloudFront-specific headers.",
            "allViewerAndWhitelistCloudFront",
            "all",
            "all",
        ),
        managed_orp(
            "59781a5b-3903-41f3-afcb-af62929ccde1",
            "Managed-CORS-CustomOrigin",
            "Policy that forwards Origin header for CORS.",
            "whitelist",
            "none",
            "none",
        ),
        managed_orp(
            "88a5eaf4-2fd4-4709-b370-b4c650ea3fcf",
            "Managed-CORS-S3Origin",
            "Policy forwarding CORS origin-access headers to S3.",
            "whitelist",
            "none",
            "none",
        ),
        managed_orp(
            "33f36d7e-f396-46d9-90e0-52428a34d9dc",
            "Managed-UserAgentRefererHeaders",
            "Forwards User-Agent and Referer headers.",
            "whitelist",
            "none",
            "none",
        ),
    ]
}

fn managed_orp(
    id: &str,
    name: &str,
    comment: &str,
    header_behavior: &str,
    cookie_behavior: &str,
    query_string_behavior: &str,
) -> OriginRequestPolicy {
    OriginRequestPolicy {
        id: id.to_owned(),
        last_modified_time: managed_timestamp(),
        config: OriginRequestPolicyConfig {
            comment: comment.to_owned(),
            name: name.to_owned(),
            headers_config: OriginRequestPolicyHeadersConfig {
                header_behavior: header_behavior.to_owned(),
                headers: Vec::new(),
            },
            cookies_config: OriginRequestPolicyCookiesConfig {
                cookie_behavior: cookie_behavior.to_owned(),
                cookies: Vec::new(),
            },
            query_strings_config: OriginRequestPolicyQueryStringsConfig {
                query_string_behavior: query_string_behavior.to_owned(),
                query_strings: Vec::new(),
            },
        },
        etag: "MANAGED_ORIGIN_REQUEST_POLICY_ETAG".to_owned(),
        managed: true,
    }
}

/// Managed response headers policies.
#[must_use]
pub fn managed_response_headers_policies() -> Vec<ResponseHeadersPolicy> {
    // Include the core handful: SimpleCORS, CORS-With-Preflight, SecurityHeadersPolicy,
    // CORS-and-SecurityHeadersPolicy.
    vec![
        ResponseHeadersPolicy {
            id: "60669652-455b-4ae9-85a4-c4c02393f86c".to_owned(),
            last_modified_time: managed_timestamp(),
            config: ResponseHeadersPolicyConfig {
                comment: "Managed SimpleCORS policy.".to_owned(),
                name: "Managed-SimpleCORS".to_owned(),
                ..Default::default()
            },
            etag: "MANAGED_RESPONSE_HEADERS_POLICY_ETAG".to_owned(),
            managed: true,
        },
        ResponseHeadersPolicy {
            id: "eaab4381-ed33-4a86-88ca-d9558dc6cd63".to_owned(),
            last_modified_time: managed_timestamp(),
            config: ResponseHeadersPolicyConfig {
                comment: "Managed CORS-with-preflight policy.".to_owned(),
                name: "Managed-CORS-With-Preflight".to_owned(),
                ..Default::default()
            },
            etag: "MANAGED_RESPONSE_HEADERS_POLICY_ETAG".to_owned(),
            managed: true,
        },
        ResponseHeadersPolicy {
            id: "67f7725c-6f97-4210-82d7-5512b31e9d03".to_owned(),
            last_modified_time: managed_timestamp(),
            config: ResponseHeadersPolicyConfig {
                comment: "Managed SecurityHeadersPolicy.".to_owned(),
                name: "Managed-SecurityHeadersPolicy".to_owned(),
                ..Default::default()
            },
            etag: "MANAGED_RESPONSE_HEADERS_POLICY_ETAG".to_owned(),
            managed: true,
        },
    ]
}
