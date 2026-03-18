//! The main S3 HTTP service implementing hyper's `Service` trait.
//!
//! [`S3HttpService`] ties together routing, authentication, dispatch, and response
//! serialization into a single hyper-compatible service. It handles:
//!
//! 1. Health check interception (`GET /_localstack/health`)
//! 2. CORS preflight requests (`OPTIONS`)
//! 3. Request body collection
//! 4. S3 request routing via [`S3Router`]
//! 5. Optional SigV4 authentication
//! 6. Operation dispatch to the [`S3Handler`]
//! 7. Common response headers (`x-amz-request-id`, `Server`, `Date`)
//! 8. Error response formatting

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::service::Service;
use sha2::{Digest, Sha256};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use ruststack_auth::CredentialProvider;
use ruststack_s3_model::error::{S3Error, S3ErrorCode};

use crate::body::S3ResponseBody;
use crate::dispatch::{S3Handler, dispatch_operation};
use crate::response::error_to_response;
use crate::router::S3Router;

/// Configuration for the S3 HTTP service.
#[derive(Clone)]
pub struct S3HttpConfig {
    /// The base domain for virtual-hosted-style requests (e.g., `s3.localhost`).
    pub domain: String,
    /// Whether to enable virtual-hosted-style bucket addressing.
    pub virtual_hosting: bool,
    /// Whether to skip SigV4 signature validation (useful for development).
    pub skip_signature_validation: bool,
    /// The AWS region this service operates in.
    pub region: String,
    /// Optional credential provider for SigV4 and presigned URL verification.
    pub credential_provider: Option<Arc<dyn CredentialProvider>>,
}

impl std::fmt::Debug for S3HttpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3HttpConfig")
            .field("domain", &self.domain)
            .field("virtual_hosting", &self.virtual_hosting)
            .field("skip_signature_validation", &self.skip_signature_validation)
            .field("region", &self.region)
            .field(
                "credential_provider",
                &self.credential_provider.as_ref().map(|_| "..."),
            )
            .finish()
    }
}

impl Default for S3HttpConfig {
    fn default() -> Self {
        Self {
            domain: "s3.localhost".to_owned(),
            virtual_hosting: true,
            skip_signature_validation: true,
            region: "us-east-1".to_owned(),
            credential_provider: None,
        }
    }
}

/// The S3 HTTP service that implements hyper's `Service` trait.
///
/// This service processes incoming HTTP requests through the full S3 request lifecycle:
/// routing, authentication, dispatch to the handler, and response formatting.
///
/// # Type Parameters
///
/// - `H`: The business logic handler implementing [`S3Handler`].
#[derive(Debug)]
pub struct S3HttpService<H: S3Handler> {
    handler: Arc<H>,
    router: S3Router,
    config: Arc<S3HttpConfig>,
}

impl<H: S3Handler> S3HttpService<H> {
    /// Create a new S3 HTTP service with the given handler and configuration.
    #[must_use]
    pub fn new(handler: H, config: S3HttpConfig) -> Self {
        let router = S3Router::new(&config.domain, config.virtual_hosting);
        Self {
            handler: Arc::new(handler),
            router,
            config: Arc::new(config),
        }
    }

    /// Create a new S3 HTTP service from an `Arc<H>` handler and configuration.
    #[must_use]
    pub fn from_shared(handler: Arc<H>, config: S3HttpConfig) -> Self {
        let router = S3Router::new(&config.domain, config.virtual_hosting);
        Self {
            handler,
            router,
            config: Arc::new(config),
        }
    }
}

impl<H: S3Handler> Clone for S3HttpService<H> {
    fn clone(&self) -> Self {
        Self {
            handler: Arc::clone(&self.handler),
            router: self.router.clone(),
            config: Arc::clone(&self.config),
        }
    }
}

impl<H: S3Handler> Service<http::Request<Incoming>> for S3HttpService<H> {
    type Response = http::Response<S3ResponseBody>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, req: http::Request<Incoming>) -> Self::Future {
        let handler = Arc::clone(&self.handler);
        let router = self.router.clone();
        let config = Arc::clone(&self.config);

        Box::pin(async move {
            let request_id = Uuid::new_v4().to_string();

            // Process the request through the S3 pipeline.
            let response =
                process_request(req, handler.as_ref(), &router, &config, &request_id).await;

            // Add common response headers.
            let response = add_common_headers(response, &request_id);

            Ok(response)
        })
    }
}

/// Process an incoming HTTP request through the S3 pipeline.
#[allow(clippy::too_many_lines)]
async fn process_request<H: S3Handler>(
    req: http::Request<Incoming>,
    handler: &H,
    router: &S3Router,
    config: &S3HttpConfig,
    request_id: &str,
) -> http::Response<S3ResponseBody> {
    let method = req.method().clone();
    let uri = req.uri().clone();
    debug!(%method, %uri, request_id, "processing S3 request");

    // 1. Health check interception.
    if is_health_check(&method, uri.path()) {
        return health_check_response();
    }

    // 1b. Prometheus metrics endpoint.
    if is_metrics_endpoint(&method, uri.path()) {
        return prometheus_metrics_response();
    }

    // 2. CORS preflight.
    if method == http::Method::OPTIONS {
        return cors_preflight_response();
    }

    // 3. Route the request (needs the original request for headers).
    let routing_result = router.resolve(&req);
    let ctx = match routing_result {
        Ok(ctx) => ctx,
        Err(err) => {
            warn!(
                %method, %uri, error = %err, request_id,
                "failed to route S3 request"
            );
            return error_to_response(&err, request_id);
        }
    };

    info!(
        operation = %ctx.operation,
        bucket = ?ctx.bucket,
        key = ?ctx.key,
        request_id,
        "routed S3 request"
    );

    // 4. Collect body.
    let (mut parts, incoming) = req.into_parts();
    let mut body = match collect_body(incoming).await {
        Ok(body) => body,
        Err(err) => {
            error!(error = %err, request_id, "failed to collect request body");
            let s3_err =
                ruststack_s3_model::error::S3Error::internal_error("Failed to read request body");
            return error_to_response(&s3_err, request_id);
        }
    };

    // 4b. Validate X-Amz-Content-Sha256 header (independent of auth).
    if let Err(s3_err) = validate_content_sha256(&parts, &body) {
        warn!(error = %s3_err.message, request_id, "content SHA256 mismatch");
        return error_to_response(&s3_err, request_id);
    }

    // 4c. Decode AWS chunked transfer encoding.
    if crate::codec::is_aws_chunked(&parts) {
        match crate::codec::decode_aws_chunked(&body) {
            Ok(result) => {
                debug!(
                    raw_len = body.len(),
                    decoded_len = result.body.len(),
                    trailing_header_count = result.trailing_headers.len(),
                    request_id,
                    "decoded aws-chunked body"
                );
                body = result.body;
                crate::codec::strip_aws_chunked_encoding(&mut parts.headers);

                // Inject trailing headers (e.g. checksum values) into request
                // headers so downstream request parsing picks them up.
                for (key, value) in &result.trailing_headers {
                    if let Ok(hv) = http::header::HeaderValue::from_str(value) {
                        if let Ok(hn) = http::header::HeaderName::from_bytes(key.as_bytes()) {
                            // Only insert if not already present in the request headers.
                            parts.headers.entry(hn).or_insert(hv);
                        }
                    }
                }
            }
            Err(s3_err) => {
                warn!(error = %s3_err.message, request_id, "failed to decode aws-chunked body");
                return error_to_response(&s3_err, request_id);
            }
        }
    }

    // 5. Authentication.
    if !config.skip_signature_validation {
        if let Some(ref cred_provider) = config.credential_provider {
            let has_presigned = parts
                .uri
                .query()
                .is_some_and(|q| q.contains("X-Amz-Signature"));

            let auth_result = if has_presigned {
                ruststack_auth::verify_presigned(&parts, cred_provider.as_ref())
            } else if let Some(auth_header) = parts
                .headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
            {
                if ruststack_auth::is_sigv2(auth_header) {
                    ruststack_auth::verify_sigv2(&parts, cred_provider.as_ref())
                } else {
                    let body_hash = ruststack_auth::hash_payload(&body);
                    ruststack_auth::verify_sigv4(&parts, &body_hash, cred_provider.as_ref())
                }
            } else {
                // Anonymous request — allow through.
                Ok(ruststack_auth::AuthResult {
                    access_key_id: String::new(),
                    region: String::new(),
                    service: String::new(),
                    signed_headers: Vec::new(),
                })
            };

            if let Err(auth_err) = auth_result {
                warn!(error = %auth_err, request_id, "authentication failed");
                let s3_err = S3Error::with_message(S3ErrorCode::AccessDenied, auth_err.to_string());
                return error_to_response(&s3_err, request_id);
            }
        }
    }

    // 6. Dispatch to handler.
    match dispatch_operation(handler, parts, body, ctx).await {
        Ok(response) => response,
        Err(err) => {
            debug!(
                error = %err,
                request_id,
                "S3 operation returned error"
            );
            error_to_response(&err, request_id)
        }
    }
}

/// Collect the full body from a hyper `Incoming` stream into `Bytes`.
async fn collect_body(incoming: Incoming) -> Result<Bytes, hyper::Error> {
    let collected = incoming.collect().await?;
    Ok(collected.to_bytes())
}

/// Validate the `X-Amz-Content-Sha256` header against the request body.
///
/// This check runs independently of signature validation. If the header is
/// present and contains a concrete hex hash (i.e. not a streaming or unsigned
/// placeholder), we verify it matches the actual body content. An invalid or
/// mismatching value returns `XAmzContentSHA256Mismatch`.
fn validate_content_sha256(parts: &http::request::Parts, body: &[u8]) -> Result<(), S3Error> {
    let Some(header_value) = parts.headers.get("x-amz-content-sha256") else {
        return Ok(());
    };

    let hash_str = header_value.to_str().map_err(|_| {
        S3Error::with_message(
            S3ErrorCode::XAmzContentSHA256Mismatch,
            "Invalid X-Amz-Content-Sha256 header encoding",
        )
    })?;

    // Skip validation for streaming and unsigned payload placeholders.
    // AWS SDKs use various STREAMING-* prefixes (SigV4, SigV4a, CRT-based,
    // etc.) and UNSIGNED-PAYLOAD variants. Rather than maintaining an
    // exhaustive allowlist, accept any recognised placeholder pattern.
    if hash_str == "UNSIGNED-PAYLOAD"
        || hash_str.starts_with("STREAMING-")
        || hash_str.starts_with("UNSIGNED-PAYLOAD-")
    {
        return Ok(());
    }

    // The value must be a 64-character lowercase hex string (SHA-256 output).
    if hash_str.len() != 64 || !hash_str.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(S3Error::with_message(
            S3ErrorCode::XAmzContentSHA256Mismatch,
            format!("The provided 'x-amz-content-sha256' header is not valid: {hash_str}"),
        ));
    }

    // Compute the actual SHA-256 digest and compare.
    let actual = hex::encode(Sha256::digest(body));
    if actual != hash_str {
        return Err(S3Error::with_message(
            S3ErrorCode::XAmzContentSHA256Mismatch,
            "The provided 'x-amz-content-sha256' header does not match what was computed",
        ));
    }

    Ok(())
}

/// Check if the request is a health check probe.
fn is_health_check(method: &http::Method, path: &str) -> bool {
    *method == http::Method::GET
        && (path == "/_localstack/health"
            || path == "/_health"
            || path == "/health"
            || path == "/minio/health/live"
            || path == "/minio/health/ready"
            || path == "/minio/health/cluster")
}

/// Produce a health check response.
fn health_check_response() -> http::Response<S3ResponseBody> {
    http::Response::builder()
        .status(http::StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(S3ResponseBody::from_string(
            r#"{"status":"running","service":"s3"}"#,
        ))
        .expect("static health response should be valid")
}

/// Check if the request is a Prometheus metrics scrape.
fn is_metrics_endpoint(method: &http::Method, path: &str) -> bool {
    *method == http::Method::GET
        && (path == "/minio/v2/metrics/cluster"
            || path == "/minio/v2/metrics/node"
            || path == "/minio/v2/metrics/bucket"
            || path == "/minio/v2/metrics/resource"
            || path == "/minio/prometheus/metrics")
}

/// Produce a Prometheus-format metrics response.
fn prometheus_metrics_response() -> http::Response<S3ResponseBody> {
    let body = concat!(
        "# HELP s3_requests_total Total number of S3 requests.\n",
        "# TYPE s3_requests_total counter\n",
        "s3_requests_total 0\n",
    );
    http::Response::builder()
        .status(http::StatusCode::OK)
        .header("Content-Type", "text/plain; version=0.0.4; charset=utf-8")
        .body(S3ResponseBody::from_string(body))
        .expect("static metrics response should be valid")
}

/// Produce a CORS preflight response.
fn cors_preflight_response() -> http::Response<S3ResponseBody> {
    http::Response::builder()
        .status(http::StatusCode::OK)
        .header("Access-Control-Allow-Origin", "*")
        .header(
            "Access-Control-Allow-Methods",
            "GET, PUT, POST, DELETE, HEAD, OPTIONS",
        )
        .header(
            "Access-Control-Allow-Headers",
            "*, Authorization, Content-Type, x-amz-*",
        )
        .header("Access-Control-Max-Age", "86400")
        .body(S3ResponseBody::empty())
        .expect("static CORS response should be valid")
}

/// Add common response headers to every S3 response.
fn add_common_headers(
    mut response: http::Response<S3ResponseBody>,
    request_id: &str,
) -> http::Response<S3ResponseBody> {
    let headers = response.headers_mut();

    // x-amz-request-id
    if let Ok(hv) = http::header::HeaderValue::from_str(request_id) {
        headers.insert("x-amz-request-id", hv);
    }

    // x-amz-id-2 (extended request ID - typically Base64 in real S3)
    if let Ok(hv) = http::header::HeaderValue::from_str(request_id) {
        headers.insert("x-amz-id-2", hv);
    }

    // Server header
    headers.insert(
        "Server",
        http::header::HeaderValue::from_static("RustStackS3"),
    );

    // CORS headers for all responses
    headers.insert(
        "Access-Control-Allow-Origin",
        http::header::HeaderValue::from_static("*"),
    );
    headers.insert(
        "Access-Control-Expose-Headers",
        http::header::HeaderValue::from_static(
            "x-amz-request-id, x-amz-id-2, x-amz-version-id, ETag, x-amz-delete-marker",
        ),
    );

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_detect_health_check_paths() {
        assert!(is_health_check(&http::Method::GET, "/_localstack/health"));
        assert!(is_health_check(&http::Method::GET, "/_health"));
        assert!(is_health_check(&http::Method::GET, "/health"));
        assert!(!is_health_check(&http::Method::POST, "/_health"));
        assert!(!is_health_check(&http::Method::GET, "/mybucket"));
    }

    #[test]
    fn test_should_produce_health_check_response() {
        let resp = health_check_response();
        assert_eq!(resp.status(), http::StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get("Content-Type")
                .and_then(|v| v.to_str().ok()),
            Some("application/json"),
        );
    }

    #[test]
    fn test_should_detect_metrics_endpoints() {
        assert!(is_metrics_endpoint(
            &http::Method::GET,
            "/minio/v2/metrics/cluster"
        ));
        assert!(is_metrics_endpoint(
            &http::Method::GET,
            "/minio/prometheus/metrics"
        ));
        assert!(is_metrics_endpoint(
            &http::Method::GET,
            "/minio/v2/metrics/node"
        ));
        assert!(!is_metrics_endpoint(
            &http::Method::POST,
            "/minio/v2/metrics/cluster"
        ));
        assert!(!is_metrics_endpoint(&http::Method::GET, "/mybucket"));
    }

    #[test]
    fn test_should_produce_prometheus_metrics_response() {
        let resp = prometheus_metrics_response();
        assert_eq!(resp.status(), http::StatusCode::OK);
        assert!(
            resp.headers()
                .get("Content-Type")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.contains("text/plain"))
        );
    }

    #[test]
    fn test_should_produce_cors_preflight_response() {
        let resp = cors_preflight_response();
        assert_eq!(resp.status(), http::StatusCode::OK);
        assert!(resp.headers().contains_key("Access-Control-Allow-Origin"));
        assert!(resp.headers().contains_key("Access-Control-Allow-Methods"));
    }

    #[test]
    fn test_should_add_common_headers() {
        let resp = http::Response::builder()
            .status(http::StatusCode::OK)
            .body(S3ResponseBody::empty())
            .expect("valid response");
        let resp = add_common_headers(resp, "test-request-id");
        assert_eq!(
            resp.headers()
                .get("x-amz-request-id")
                .and_then(|v| v.to_str().ok()),
            Some("test-request-id"),
        );
        assert_eq!(
            resp.headers().get("Server").and_then(|v| v.to_str().ok()),
            Some("RustStackS3"),
        );
    }

    #[test]
    fn test_should_create_default_config() {
        let config = S3HttpConfig::default();
        assert_eq!(config.domain, "s3.localhost");
        assert!(config.virtual_hosting);
        assert!(config.skip_signature_validation);
        assert_eq!(config.region, "us-east-1");
        assert!(config.credential_provider.is_none());
    }

    #[test]
    fn test_should_debug_format_config() {
        let config = S3HttpConfig::default();
        let debug_str = format!("{config:?}");
        assert!(debug_str.contains("S3HttpConfig"));
        assert!(debug_str.contains("s3.localhost"));
    }

    // -----------------------------------------------------------------------
    // X-Amz-Content-Sha256 validation
    // -----------------------------------------------------------------------

    fn parts_with_sha256(header_value: &str) -> http::request::Parts {
        let (parts, ()) = http::Request::builder()
            .method(http::Method::PUT)
            .uri("/bucket/key")
            .header("x-amz-content-sha256", header_value)
            .body(())
            .expect("valid request")
            .into_parts();
        parts
    }

    fn parts_without_sha256() -> http::request::Parts {
        let (parts, ()) = http::Request::builder()
            .method(http::Method::PUT)
            .uri("/bucket/key")
            .body(())
            .expect("valid request")
            .into_parts();
        parts
    }

    #[test]
    fn test_should_accept_absent_content_sha256() {
        let parts = parts_without_sha256();
        assert!(validate_content_sha256(&parts, b"hello").is_ok());
    }

    #[test]
    fn test_should_accept_unsigned_payload() {
        let parts = parts_with_sha256("UNSIGNED-PAYLOAD");
        assert!(validate_content_sha256(&parts, b"hello").is_ok());
    }

    #[test]
    fn test_should_accept_streaming_payload() {
        let parts = parts_with_sha256("STREAMING-AWS4-HMAC-SHA256-PAYLOAD");
        assert!(validate_content_sha256(&parts, b"hello").is_ok());
    }

    #[test]
    fn test_should_accept_streaming_payload_trailer() {
        let parts = parts_with_sha256("STREAMING-AWS4-HMAC-SHA256-PAYLOAD-TRAILER");
        assert!(validate_content_sha256(&parts, b"hello").is_ok());
    }

    #[test]
    fn test_should_accept_streaming_sigv4a_payload() {
        let parts = parts_with_sha256("STREAMING-AWS4-ECDSA-P256-SHA256-PAYLOAD");
        assert!(validate_content_sha256(&parts, b"hello").is_ok());
    }

    #[test]
    fn test_should_accept_streaming_unsigned_payload_trailer() {
        let parts = parts_with_sha256("STREAMING-UNSIGNED-PAYLOAD-TRAILER");
        assert!(validate_content_sha256(&parts, b"hello").is_ok());
    }

    #[test]
    fn test_should_accept_unsigned_payload_trailer() {
        let parts = parts_with_sha256("UNSIGNED-PAYLOAD-TRAILER");
        assert!(validate_content_sha256(&parts, b"hello").is_ok());
    }

    #[test]
    fn test_should_accept_correct_content_sha256() {
        let body = b"hello";
        let hash = hex::encode(Sha256::digest(body));
        let parts = parts_with_sha256(&hash);
        assert!(validate_content_sha256(&parts, body).is_ok());
    }

    #[test]
    fn test_should_reject_invalid_content_sha256() {
        let parts = parts_with_sha256("invalid-sha256");
        let result = validate_content_sha256(&parts, b"hello");
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().code,
            S3ErrorCode::XAmzContentSHA256Mismatch
        );
    }

    #[test]
    fn test_should_reject_wrong_content_sha256() {
        let wrong_hash = hex::encode(Sha256::digest(b"wrong"));
        let parts = parts_with_sha256(&wrong_hash);
        let result = validate_content_sha256(&parts, b"hello");
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().code,
            S3ErrorCode::XAmzContentSHA256Mismatch
        );
    }
}
