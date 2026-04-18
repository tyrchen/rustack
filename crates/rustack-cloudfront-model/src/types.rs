//! Domain types for CloudFront resources.
//!
//! The types here form a minimal but faithful representation of the wire
//! schema. Each resource record is a single struct that owns its configuration
//! plus bookkeeping (ETag, timestamps, status, etc.). Optional fields that
//! CloudFront always emits (even as `<Quantity>0</Quantity>`) are modelled as
//! `Vec<T>` rather than `Option<Vec<T>>` so the XML renderer can use the same
//! code path for "present but empty" and "present with items".

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::tags::TagSet;

// ---------------------------------------------------------------------------
// Common enums
// ---------------------------------------------------------------------------

/// Distribution / invalidation lifecycle states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ResourceStatus {
    /// Change is propagating.
    #[default]
    InProgress,
    /// Deployed to the edge.
    Deployed,
    /// Invalidation has finished.
    Completed,
}

impl ResourceStatus {
    /// Wire-format string.
    #[must_use]
    pub fn as_wire(self) -> &'static str {
        match self {
            Self::InProgress => "InProgress",
            Self::Deployed => "Deployed",
            Self::Completed => "Completed",
        }
    }
}

// ---------------------------------------------------------------------------
// Distribution
// ---------------------------------------------------------------------------

/// Full distribution record persisted in the store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Distribution {
    /// CloudFront-assigned distribution ID (14 chars).
    pub id: String,
    /// ARN — for CloudFront, region is empty: `arn:aws:cloudfront::{account}:distribution/{id}`.
    pub arn: String,
    /// Current lifecycle status.
    pub status: ResourceStatus,
    /// Last modification wall-clock time.
    pub last_modified_time: DateTime<Utc>,
    /// CloudFront-assigned `{id}.cloudfront.net` FQDN.
    pub domain_name: String,
    /// Number of invalidation batches currently running for this distribution.
    pub in_progress_invalidation_batches: i32,
    /// Active trusted signers (always disabled in Rustack; stored for echo).
    pub active_trusted_signers_enabled: bool,
    /// Active trusted key groups (always disabled in Rustack; stored for echo).
    pub active_trusted_key_groups_enabled: bool,
    /// Distribution configuration (echoed back in GET/Create responses).
    pub config: DistributionConfig,
    /// Tags (for Tag-enabled distributions).
    pub tags: TagSet,
    /// ETag (monotonic version token).
    pub etag: String,
    /// A/B testing weight — stored for completeness, unused in data plane.
    pub alias_icp_recordal: Vec<AliasIcpRecordal>,
}

/// Alternate-domain ICP recordal entry (PRC compliance).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AliasIcpRecordal {
    /// Alias FQDN.
    pub cname: String,
    /// Status, e.g. `APPROVED`.
    pub icp_recordal_status: String,
}

/// Distribution configuration.
///
/// Mirrors AWS `DistributionConfig`. Optional subsections that AWS always emits
/// (like `Aliases`, `CustomErrorResponses`, `Restrictions`) are modelled as
/// default-constructible structs rather than `Option<T>`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DistributionConfig {
    /// Idempotency token supplied by the caller.
    pub caller_reference: String,
    /// Alternate CNAMEs.
    pub aliases: Vec<String>,
    /// Default root object (e.g. `index.html`).
    pub default_root_object: String,
    /// Origin list.
    pub origins: Vec<Origin>,
    /// Origin groups (failover pairs).
    pub origin_groups: Vec<OriginGroup>,
    /// Catch-all cache behavior.
    pub default_cache_behavior: CacheBehavior,
    /// Ordered prefixed cache behaviors.
    pub cache_behaviors: Vec<CacheBehavior>,
    /// Custom error response overrides.
    pub custom_error_responses: Vec<CustomErrorResponse>,
    /// Optional comment.
    pub comment: String,
    /// Logging settings.
    pub logging: LoggingConfig,
    /// `PriceClass_All`, `PriceClass_100`, `PriceClass_200`.
    pub price_class: String,
    /// Whether the distribution is enabled.
    pub enabled: bool,
    /// Viewer certificate settings.
    pub viewer_certificate: ViewerCertificate,
    /// Geo restrictions.
    pub restrictions: Restrictions,
    /// ARN of WAF WebACL (stored only).
    pub web_acl_id: String,
    /// HTTP version (`http1.1`, `http2`, `http2and3`, `http3`).
    pub http_version: String,
    /// Whether IPv6 is enabled.
    pub is_ipv6_enabled: bool,
    /// Continuous deployment policy ID (stored only).
    pub continuous_deployment_policy_id: String,
    /// Whether this is a staging distribution.
    pub staging: bool,
    /// Anycast IP list ID (stored only).
    pub anycast_ip_list_id: String,
    /// Connection mode: `direct` or `tenant-only`.
    pub connection_mode: String,
    /// TenantConfig parameter definitions (stored only).
    pub tenant_config_parameters: Vec<TenantConfigParameter>,
}

/// CloudFront origin.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Origin {
    /// Caller-supplied ID, referenced by `TargetOriginId`.
    pub id: String,
    /// Origin FQDN (e.g. `my-bucket.s3.us-east-1.amazonaws.com`).
    pub domain_name: String,
    /// Optional directory prefix prepended to every request.
    pub origin_path: String,
    /// Headers appended to every origin request.
    pub custom_headers: Vec<CustomHeader>,
    /// Present if this is an S3 origin.
    pub s3_origin_config: Option<S3OriginConfig>,
    /// Present if this is a custom HTTP origin.
    pub custom_origin_config: Option<CustomOriginConfig>,
    /// Connection attempts (default 3).
    pub connection_attempts: i32,
    /// Connection timeout in seconds (default 10).
    pub connection_timeout: i32,
    /// Origin Shield configuration.
    pub origin_shield: Option<OriginShield>,
    /// Origin Access Control ID (OAC — modern).
    pub origin_access_control_id: String,
    /// VPC origin configuration.
    pub vpc_origin_config: Option<VpcOriginConfig>,
}

/// S3 origin configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct S3OriginConfig {
    /// Origin Access Identity (OAI — legacy), format `origin-access-identity/cloudfront/{Id}` or
    /// empty.
    pub origin_access_identity: String,
}

/// Custom (HTTP) origin configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CustomOriginConfig {
    /// HTTP port.
    pub http_port: i32,
    /// HTTPS port.
    pub https_port: i32,
    /// `http-only`, `https-only`, `match-viewer`.
    pub origin_protocol_policy: String,
    /// List of allowed SSL/TLS versions.
    pub origin_ssl_protocols: Vec<String>,
    /// Read timeout in seconds.
    pub origin_read_timeout: i32,
    /// Keep-alive timeout in seconds.
    pub origin_keepalive_timeout: i32,
}

/// VPC origin configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VpcOriginConfig {
    /// VPC origin ID.
    pub vpc_origin_id: String,
    /// Read timeout in seconds.
    pub origin_read_timeout: i32,
    /// Keep-alive timeout in seconds.
    pub origin_keepalive_timeout: i32,
}

/// Origin Shield configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OriginShield {
    /// Whether Origin Shield is enabled.
    pub enabled: bool,
    /// Origin Shield region.
    pub origin_shield_region: String,
}

/// Origin group (primary/failover).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OriginGroup {
    /// Group ID (referenced by `TargetOriginId`).
    pub id: String,
    /// Failover trigger status codes.
    pub failover_status_codes: Vec<i32>,
    /// Member origin IDs in priority order.
    pub member_origins: Vec<String>,
    /// Selection criteria: `default`, `media-quality-based`.
    pub selection_criteria: String,
}

/// Arbitrary HTTP header `(name, value)` pair.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CustomHeader {
    /// Header name.
    pub header_name: String,
    /// Header value.
    pub header_value: String,
}

/// Cache behavior (default or prefixed).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CacheBehavior {
    /// Glob path pattern (empty for `DefaultCacheBehavior`).
    pub path_pattern: String,
    /// Referenced `Origin.id`.
    pub target_origin_id: String,
    /// `allow-all`, `https-only`, `redirect-to-https`.
    pub viewer_protocol_policy: String,
    /// Methods the distribution accepts.
    pub allowed_methods: Vec<String>,
    /// Subset of `allowed_methods` whose responses are cached.
    pub cached_methods: Vec<String>,
    /// Whether SmoothStreaming is enabled.
    pub smooth_streaming: bool,
    /// Whether to compress responses.
    pub compress: bool,
    /// Field-level encryption config ID.
    pub field_level_encryption_id: String,
    /// Realtime log config ARN.
    pub realtime_log_config_arn: String,
    /// Cache policy ID.
    pub cache_policy_id: String,
    /// Origin request policy ID.
    pub origin_request_policy_id: String,
    /// Response headers policy ID.
    pub response_headers_policy_id: String,
    /// Grpc config.
    pub grpc_enabled: bool,
    /// Trusted signers (legacy).
    pub trusted_signers: Vec<String>,
    /// Trusted signers enabled flag.
    pub trusted_signers_enabled: bool,
    /// Trusted key groups (modern).
    pub trusted_key_groups: Vec<String>,
    /// Trusted key groups enabled flag.
    pub trusted_key_groups_enabled: bool,
    /// Lambda@Edge associations.
    pub lambda_function_associations: Vec<LambdaFunctionAssociation>,
    /// CloudFront Function associations.
    pub function_associations: Vec<FunctionAssociation>,
    /// Legacy forwarded values (when no CachePolicy is set).
    pub forwarded_values: Option<ForwardedValues>,
    /// Legacy MinTTL (when no CachePolicy is set).
    pub min_ttl: i64,
    /// Legacy DefaultTTL.
    pub default_ttl: i64,
    /// Legacy MaxTTL.
    pub max_ttl: i64,
}

/// Lambda@Edge association entry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LambdaFunctionAssociation {
    /// Lambda function version ARN.
    pub lambda_function_arn: String,
    /// `viewer-request`, `viewer-response`, `origin-request`, `origin-response`.
    pub event_type: String,
    /// Whether the function receives the body.
    pub include_body: bool,
}

/// CloudFront Function association entry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FunctionAssociation {
    /// Function ARN.
    pub function_arn: String,
    /// `viewer-request` or `viewer-response`.
    pub event_type: String,
}

/// Legacy forwarded-values configuration (pre-CachePolicy).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ForwardedValues {
    /// Whether to forward query strings as cache keys.
    pub query_string: bool,
    /// Cookie forwarding.
    pub cookies: CookiePreference,
    /// Headers to forward.
    pub headers: Vec<String>,
    /// Whitelisted query string names.
    pub query_string_cache_keys: Vec<String>,
}

/// Cookie forwarding configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CookiePreference {
    /// `none`, `whitelist`, `all`, `allExcept`.
    pub forward: String,
    /// Whitelisted cookie names.
    pub whitelisted_names: Vec<String>,
}

/// Custom error response override.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CustomErrorResponse {
    /// HTTP error code to match.
    pub error_code: i32,
    /// Path to the replacement response page.
    pub response_page_path: String,
    /// Replacement response status code.
    pub response_code: String,
    /// Min TTL for caching error responses.
    pub error_caching_min_ttl: i64,
}

/// Access-log configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Whether logging is enabled.
    pub enabled: bool,
    /// Whether to include cookies in the log.
    pub include_cookies: bool,
    /// S3 bucket (`mylog-bucket.s3.amazonaws.com`).
    pub bucket: String,
    /// Prefix within the bucket.
    pub prefix: String,
}

/// Viewer certificate settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ViewerCertificate {
    /// Whether to use the default CloudFront certificate.
    pub cloud_front_default_certificate: bool,
    /// ACM certificate ARN.
    pub acm_certificate_arn: String,
    /// IAM certificate ID.
    pub iam_certificate_id: String,
    /// Minimum TLS protocol version (e.g. `TLSv1.2_2021`).
    pub minimum_protocol_version: String,
    /// `sni-only`, `vip`, `static-ip`.
    pub ssl_support_method: String,
    /// Legacy fields.
    pub certificate: String,
    /// Legacy source: `cloudfront` | `iam` | `acm`.
    pub certificate_source: String,
}

/// Geo restriction settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Restrictions {
    /// Geo restriction block.
    pub geo_restriction: GeoRestriction,
}

/// Geo restriction details.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GeoRestriction {
    /// `blacklist`, `whitelist`, `none`.
    pub restriction_type: String,
    /// Country codes.
    pub locations: Vec<String>,
}

/// Tenant config parameter (for tenant-only distributions).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TenantConfigParameter {
    /// Parameter name.
    pub name: String,
}

// ---------------------------------------------------------------------------
// Invalidation
// ---------------------------------------------------------------------------

/// Invalidation record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invalidation {
    /// CloudFront-assigned invalidation ID.
    pub id: String,
    /// Status (starts `InProgress`, becomes `Completed`).
    pub status: ResourceStatus,
    /// Create time.
    pub create_time: DateTime<Utc>,
    /// Parent distribution ID.
    pub distribution_id: String,
    /// Invalidation batch input.
    pub batch: InvalidationBatch,
}

/// Invalidation batch input.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InvalidationBatch {
    /// Paths to invalidate.
    pub paths: Vec<String>,
    /// Idempotency token.
    pub caller_reference: String,
}

// ---------------------------------------------------------------------------
// Origin Access Control (OAC)
// ---------------------------------------------------------------------------

/// Origin Access Control record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OriginAccessControl {
    /// CloudFront-assigned OAC ID.
    pub id: String,
    /// Configuration (echoed on GET).
    pub config: OriginAccessControlConfig,
    /// ETag.
    pub etag: String,
}

/// Origin Access Control configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OriginAccessControlConfig {
    /// OAC name (required, unique per account).
    pub name: String,
    /// Description (optional).
    pub description: String,
    /// Signing protocol: `sigv4`.
    pub signing_protocol: String,
    /// Signing behavior: `always`, `never`, `no-override`.
    pub signing_behavior: String,
    /// Origin type: `s3`, `mediastore`, `lambda`, `mediapackagev2`.
    pub origin_access_control_origin_type: String,
}

// ---------------------------------------------------------------------------
// Origin Access Identity (OAI, legacy)
// ---------------------------------------------------------------------------

/// CloudFront Origin Access Identity (legacy — still used by Terraform).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudFrontOriginAccessIdentity {
    /// OAI ID (14 chars, E-prefixed).
    pub id: String,
    /// Canonical user ID used in S3 bucket policies.
    pub s3_canonical_user_id: String,
    /// Configuration.
    pub config: CloudFrontOriginAccessIdentityConfig,
    /// ETag.
    pub etag: String,
}

/// OAI configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CloudFrontOriginAccessIdentityConfig {
    /// Caller reference.
    pub caller_reference: String,
    /// Comment.
    pub comment: String,
}

// ---------------------------------------------------------------------------
// Cache / OriginRequest / ResponseHeaders Policies
// ---------------------------------------------------------------------------

/// Cache policy record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachePolicy {
    /// Policy ID.
    pub id: String,
    /// Last-modified timestamp.
    pub last_modified_time: DateTime<Utc>,
    /// Configuration.
    pub config: CachePolicyConfig,
    /// ETag.
    pub etag: String,
    /// Whether this is an AWS-managed policy (immutable).
    pub managed: bool,
}

/// Cache policy configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachePolicyConfig {
    /// Optional comment.
    pub comment: String,
    /// Policy name.
    pub name: String,
    /// Default TTL in seconds.
    pub default_ttl: i64,
    /// Maximum TTL in seconds.
    pub max_ttl: i64,
    /// Minimum TTL in seconds.
    pub min_ttl: i64,
    /// Parameters contributing to the cache key.
    pub parameters_in_cache_key_and_forwarded_to_origin: ParamsInCacheKey,
}

/// Parameters controlling cache-key composition.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ParamsInCacheKey {
    /// Whether gzip is permitted in the cache key.
    pub enable_accept_encoding_gzip: bool,
    /// Whether brotli is permitted in the cache key.
    pub enable_accept_encoding_brotli: bool,
    /// Headers forwarded to origin and included in key.
    pub headers_config: CachePolicyHeadersConfig,
    /// Cookies forwarded/included.
    pub cookies_config: CachePolicyCookiesConfig,
    /// Query strings forwarded/included.
    pub query_strings_config: CachePolicyQueryStringsConfig,
}

/// Header cache-key configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachePolicyHeadersConfig {
    /// `none`, `whitelist`.
    pub header_behavior: String,
    /// Whitelisted header names.
    pub headers: Vec<String>,
}

/// Cookie cache-key configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachePolicyCookiesConfig {
    /// `none`, `whitelist`, `allExcept`, `all`.
    pub cookie_behavior: String,
    /// Cookie name list.
    pub cookies: Vec<String>,
}

/// Query-string cache-key configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachePolicyQueryStringsConfig {
    /// `none`, `whitelist`, `allExcept`, `all`.
    pub query_string_behavior: String,
    /// Whitelist.
    pub query_strings: Vec<String>,
}

/// Origin request policy record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OriginRequestPolicy {
    /// Policy ID.
    pub id: String,
    /// Last-modified timestamp.
    pub last_modified_time: DateTime<Utc>,
    /// Configuration.
    pub config: OriginRequestPolicyConfig,
    /// ETag.
    pub etag: String,
    /// AWS-managed flag.
    pub managed: bool,
}

/// Origin request policy config.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OriginRequestPolicyConfig {
    /// Comment.
    pub comment: String,
    /// Policy name.
    pub name: String,
    /// Headers forwarded to origin.
    pub headers_config: OriginRequestPolicyHeadersConfig,
    /// Cookies forwarded to origin.
    pub cookies_config: OriginRequestPolicyCookiesConfig,
    /// Query strings forwarded to origin.
    pub query_strings_config: OriginRequestPolicyQueryStringsConfig,
}

/// Origin request headers config.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OriginRequestPolicyHeadersConfig {
    /// `none`, `whitelist`, `allViewer`, `allViewerAndWhitelistCloudFront`, `allExcept`.
    pub header_behavior: String,
    /// List.
    pub headers: Vec<String>,
}

/// Origin request cookies config.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OriginRequestPolicyCookiesConfig {
    /// `none`, `whitelist`, `all`, `allExcept`.
    pub cookie_behavior: String,
    /// List.
    pub cookies: Vec<String>,
}

/// Origin request query-strings config.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OriginRequestPolicyQueryStringsConfig {
    /// `none`, `whitelist`, `all`, `allExcept`.
    pub query_string_behavior: String,
    /// List.
    pub query_strings: Vec<String>,
}

/// Response headers policy record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseHeadersPolicy {
    /// Policy ID.
    pub id: String,
    /// Last-modified timestamp.
    pub last_modified_time: DateTime<Utc>,
    /// Configuration.
    pub config: ResponseHeadersPolicyConfig,
    /// ETag.
    pub etag: String,
    /// AWS-managed flag.
    pub managed: bool,
}

/// Response headers policy configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseHeadersPolicyConfig {
    /// Comment.
    pub comment: String,
    /// Policy name.
    pub name: String,
    /// CORS configuration.
    pub cors_config: Option<ResponseHeadersPolicyCorsConfig>,
    /// Security header configuration.
    pub security_headers_config: Option<ResponseHeadersPolicySecurityHeadersConfig>,
    /// Server-Timing.
    pub server_timing_headers_config: Option<ServerTimingHeadersConfig>,
    /// Custom user-supplied headers.
    pub custom_headers_config: Vec<ResponseHeaderOverride>,
    /// Headers to strip from the upstream response.
    pub remove_headers_config: Vec<String>,
}

/// CORS config portion of a response headers policy.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseHeadersPolicyCorsConfig {
    /// `Access-Control-Allow-Credentials`.
    pub access_control_allow_credentials: bool,
    /// Allowed origins.
    pub access_control_allow_origins: Vec<String>,
    /// Allowed headers.
    pub access_control_allow_headers: Vec<String>,
    /// Allowed methods.
    pub access_control_allow_methods: Vec<String>,
    /// Exposed headers.
    pub access_control_expose_headers: Vec<String>,
    /// Max-age in seconds.
    pub access_control_max_age_sec: i64,
    /// Whether these override upstream headers.
    pub origin_override: bool,
}

/// Security header configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseHeadersPolicySecurityHeadersConfig {
    /// XSS protection header settings.
    pub xss_protection: Option<XssProtection>,
    /// Frame options.
    pub frame_options: Option<FrameOptions>,
    /// Referrer policy.
    pub referrer_policy: Option<ReferrerPolicy>,
    /// Content security policy.
    pub content_security_policy: Option<ContentSecurityPolicy>,
    /// `X-Content-Type-Options: nosniff`.
    pub content_type_options: Option<ContentTypeOptions>,
    /// Strict-Transport-Security.
    pub strict_transport_security: Option<StrictTransportSecurity>,
}

/// XSS protection header.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct XssProtection {
    /// `X-XSS-Protection` value `1`.
    pub protection: bool,
    /// `mode=block`.
    pub mode_block: bool,
    /// Whether to override upstream.
    pub override_upstream: bool,
    /// Optional report URI.
    pub report_uri: String,
}

/// `X-Frame-Options`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FrameOptions {
    /// `DENY` or `SAMEORIGIN`.
    pub frame_option: String,
    /// Whether to override upstream.
    pub override_upstream: bool,
}

/// `Referrer-Policy`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReferrerPolicy {
    /// Referrer-Policy value.
    pub referrer_policy: String,
    /// Whether to override upstream.
    pub override_upstream: bool,
}

/// `Content-Security-Policy`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContentSecurityPolicy {
    /// The policy.
    pub content_security_policy: String,
    /// Whether to override upstream.
    pub override_upstream: bool,
}

/// `X-Content-Type-Options`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContentTypeOptions {
    /// Whether to override upstream.
    pub override_upstream: bool,
}

/// `Strict-Transport-Security`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StrictTransportSecurity {
    /// Whether to override upstream.
    pub override_upstream: bool,
    /// `includeSubDomains` directive.
    pub include_subdomains: bool,
    /// `preload` directive.
    pub preload: bool,
    /// Max-age seconds.
    pub access_control_max_age_sec: i64,
}

/// Server-Timing header config.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerTimingHeadersConfig {
    /// Whether enabled.
    pub enabled: bool,
    /// Sampling rate 0.0–100.0.
    pub sampling_rate: f64,
}

/// Single custom header override entry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseHeaderOverride {
    /// Header name.
    pub header: String,
    /// Header value.
    pub value: String,
    /// Whether to override any value from origin.
    pub override_upstream: bool,
}

// ---------------------------------------------------------------------------
// Key material
// ---------------------------------------------------------------------------

/// Key group record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyGroup {
    /// Key group ID.
    pub id: String,
    /// Last-modified timestamp.
    pub last_modified_time: DateTime<Utc>,
    /// Configuration.
    pub config: KeyGroupConfig,
    /// ETag.
    pub etag: String,
}

/// Key group configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KeyGroupConfig {
    /// Key group name.
    pub name: String,
    /// List of public-key IDs in this group.
    pub items: Vec<String>,
    /// Optional comment.
    pub comment: String,
}

/// Public key record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicKey {
    /// Public key ID.
    pub id: String,
    /// Create time.
    pub created_time: DateTime<Utc>,
    /// Configuration.
    pub config: PublicKeyConfig,
    /// ETag.
    pub etag: String,
}

/// Public key configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PublicKeyConfig {
    /// Caller reference.
    pub caller_reference: String,
    /// Name.
    pub name: String,
    /// PEM-encoded public key.
    pub encoded_key: String,
    /// Comment.
    pub comment: String,
}

// ---------------------------------------------------------------------------
// Functions
// ---------------------------------------------------------------------------

/// CloudFront Function record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudFrontFunction {
    /// Function name.
    pub name: String,
    /// Function ARN.
    pub arn: String,
    /// Last-modified.
    pub last_modified_time: DateTime<Utc>,
    /// `DEVELOPMENT` or `LIVE`.
    pub stage: String,
    /// Metadata.
    pub metadata: FunctionMetadata,
    /// Configuration.
    pub config: FunctionConfig,
    /// Source code (stored as opaque bytes).
    pub code: Vec<u8>,
    /// ETag.
    pub etag: String,
    /// Status text.
    pub status: String,
}

/// Function configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FunctionConfig {
    /// Comment.
    pub comment: String,
    /// Runtime (e.g. `cloudfront-js-1.0`).
    pub runtime: String,
    /// KeyValueStore associations.
    pub key_value_store_associations: Vec<String>,
}

/// Function metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionMetadata {
    /// Function ARN.
    pub function_arn: String,
    /// `DEVELOPMENT` or `LIVE`.
    pub stage: String,
    /// Created time.
    pub created_time: DateTime<Utc>,
    /// Last-modified time.
    pub last_modified_time: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// FLE (Field-Level Encryption)
// ---------------------------------------------------------------------------

/// FLE configuration record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldLevelEncryption {
    /// Config ID.
    pub id: String,
    /// Last-modified.
    pub last_modified_time: DateTime<Utc>,
    /// Config.
    pub config: FieldLevelEncryptionConfig,
    /// ETag.
    pub etag: String,
}

/// FLE config.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FieldLevelEncryptionConfig {
    /// Caller reference.
    pub caller_reference: String,
    /// Comment.
    pub comment: String,
    /// Query-argument profile config.
    pub query_arg_profile_config_enabled: bool,
    /// Content-type profile config.
    pub content_type_profile_config_enabled: bool,
}

/// FLE profile record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldLevelEncryptionProfile {
    /// Profile ID.
    pub id: String,
    /// Last-modified.
    pub last_modified_time: DateTime<Utc>,
    /// Config.
    pub config: FieldLevelEncryptionProfileConfig,
    /// ETag.
    pub etag: String,
}

/// FLE profile config.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FieldLevelEncryptionProfileConfig {
    /// Name.
    pub name: String,
    /// Caller reference.
    pub caller_reference: String,
    /// Comment.
    pub comment: String,
}

// ---------------------------------------------------------------------------
// Monitoring / KVStore / RealtimeLog
// ---------------------------------------------------------------------------

/// Monitoring subscription record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringSubscription {
    /// Distribution ID.
    pub distribution_id: String,
    /// Whether realtime metrics are enabled.
    pub realtime_metrics_subscription_status: String,
}

/// KVStore record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyValueStore {
    /// KVS ID.
    pub id: String,
    /// KVS name.
    pub name: String,
    /// ARN.
    pub arn: String,
    /// Comment.
    pub comment: String,
    /// Status.
    pub status: String,
    /// Last-modified.
    pub last_modified_time: DateTime<Utc>,
    /// ETag.
    pub etag: String,
}

/// Realtime log config record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RealtimeLogConfig {
    /// ARN.
    pub arn: String,
    /// Name.
    pub name: String,
    /// Sampling rate 1..=100.
    pub sampling_rate: i64,
    /// Kinesis endpoint ARN.
    pub end_points: Vec<EndPoint>,
    /// Fields logged.
    pub fields: Vec<String>,
}

/// Endpoint for realtime log shipping.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EndPoint {
    /// `Kinesis`.
    pub stream_type: String,
    /// Kinesis stream ARN + role ARN.
    pub kinesis_stream_config: KinesisStreamConfig,
}

/// Kinesis stream config for realtime logging.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KinesisStreamConfig {
    /// Role ARN assumed by CloudFront.
    pub role_arn: String,
    /// Target Kinesis stream ARN.
    pub stream_arn: String,
}
