//! S3 request routing: virtual hosting resolution and operation identification.
//!
//! The [`S3Router`] maps incoming HTTP requests to S3 operations by examining:
//!
//! - The HTTP method (GET, PUT, DELETE, POST, HEAD)
//! - Whether a bucket name is present (from the Host header or path)
//! - Whether an object key is present (from the URI path)
//! - Query parameters that identify sub-resources (e.g., `?versioning`, `?tagging`)
//! - Specific headers (e.g., `x-amz-copy-source` to distinguish PutObject from CopyObject)
//!
//! Virtual hosting is supported: the bucket name can come from either the `Host` header
//! (e.g., `mybucket.s3.localhost`) or from the first path segment (path-style).

use http::Method;
use percent_encoding::percent_decode_str;
use rustack_s3_model::{
    error::{S3Error, S3ErrorCode},
    operations::S3Operation,
};

/// Configuration for S3 request routing.
#[derive(Debug, Clone)]
pub struct S3Router {
    /// The base domain for virtual-hosted-style requests (e.g., `s3.localhost`).
    pub domain: String,
    /// Whether to enable virtual-hosted-style bucket addressing.
    pub virtual_hosting: bool,
}

/// The result of routing an HTTP request to an S3 operation.
#[derive(Debug, Clone)]
pub struct RoutingContext {
    /// The resolved bucket name, if any.
    pub bucket: Option<String>,
    /// The resolved object key, if any.
    pub key: Option<String>,
    /// The identified S3 operation.
    pub operation: S3Operation,
    /// Parsed query parameters from the request URI.
    pub query_params: Vec<(String, String)>,
}

impl S3Router {
    /// Create a new router with the given domain and virtual hosting setting.
    #[must_use]
    pub fn new(domain: impl Into<String>, virtual_hosting: bool) -> Self {
        Self {
            domain: domain.into(),
            virtual_hosting,
        }
    }

    /// Resolve an HTTP request to a routing context containing the identified S3 operation.
    ///
    /// This performs:
    /// 1. Virtual hosting resolution (extract bucket from Host header if applicable)
    /// 2. Path parsing (extract bucket and key from URI path)
    /// 3. Query parameter parsing
    /// 4. Operation identification from method + path structure + query params + headers
    ///
    /// # Errors
    ///
    /// Returns an `S3Error` if the request cannot be routed to a valid operation
    /// (e.g., unsupported HTTP method).
    pub fn resolve<B>(&self, req: &http::Request<B>) -> Result<RoutingContext, S3Error> {
        let method = req.method();
        let uri = req.uri();
        let headers = req.headers();

        // Parse query parameters.
        let query_params = parse_query_params(uri.query().unwrap_or(""));

        // Extract bucket from virtual hosting (Host header).
        let virtual_bucket = if self.virtual_hosting {
            extract_virtual_host_bucket(headers, &self.domain)
        } else {
            None
        };

        // Parse path to extract bucket and key.
        let path = uri.path();
        let (path_bucket, path_key) = parse_path(path);

        // Combine virtual host bucket with path-based bucket/key.
        let (bucket, key) = if let Some(vhost_bucket) = virtual_bucket {
            // Virtual hosting: bucket comes from Host, entire path is the key.
            let key = if path == "/" || path.is_empty() {
                None
            } else {
                // Strip leading "/" and decode.
                let raw_key = &path[1..];
                if raw_key.is_empty() {
                    None
                } else {
                    Some(decode_uri_component(raw_key))
                }
            };
            (Some(vhost_bucket), key)
        } else {
            // Path-style: bucket is first path segment, rest is key.
            (path_bucket, path_key)
        };

        // Identify the operation.
        let operation = identify_operation(
            method,
            bucket.as_ref(),
            key.as_ref(),
            &query_params,
            headers,
        )?;

        Ok(RoutingContext {
            bucket,
            key,
            operation,
            query_params,
        })
    }
}

/// Extract the bucket name from a virtual-hosted-style Host header.
///
/// For example, if the domain is `s3.localhost` and the Host header is
/// `mybucket.s3.localhost:4566`, this returns `Some("mybucket")`.
fn extract_virtual_host_bucket(headers: &http::HeaderMap, domain: &str) -> Option<String> {
    let host = headers
        .get(http::header::HOST)
        .and_then(|v| v.to_str().ok())?;

    // Strip port if present.
    let host_without_port = host.split(':').next().unwrap_or(host);

    // Check if the host ends with our domain and has a bucket prefix.
    let suffix = format!(".{domain}");
    if host_without_port.ends_with(&suffix) && host_without_port.len() > suffix.len() {
        let bucket = &host_without_port[..host_without_port.len() - suffix.len()];
        if !bucket.is_empty() {
            return Some(bucket.to_owned());
        }
    }

    None
}

/// Parse the URI path into an optional bucket and optional key.
///
/// Path format: `/{bucket}` or `/{bucket}/{key...}`
fn parse_path(path: &str) -> (Option<String>, Option<String>) {
    let trimmed = path.strip_prefix('/').unwrap_or(path);
    if trimmed.is_empty() {
        return (None, None);
    }

    if let Some(pos) = trimmed.find('/') {
        let bucket = decode_uri_component(&trimmed[..pos]);
        let key_raw = &trimmed[pos + 1..];
        let key = if key_raw.is_empty() {
            None
        } else {
            Some(decode_uri_component(key_raw))
        };
        (Some(bucket), key)
    } else {
        (Some(decode_uri_component(trimmed)), None)
    }
}

/// Decode a percent-encoded URI component.
fn decode_uri_component(s: &str) -> String {
    percent_decode_str(s).decode_utf8_lossy().into_owned()
}

/// Parse a query string into key-value pairs.
fn parse_query_params(query: &str) -> Vec<(String, String)> {
    if query.is_empty() {
        return Vec::new();
    }

    query
        .split('&')
        .filter(|s| !s.is_empty())
        .map(|pair| {
            if let Some(pos) = pair.find('=') {
                let key = decode_uri_component(&pair[..pos]);
                let value = decode_uri_component(&pair[pos + 1..]);
                (key, value)
            } else {
                (decode_uri_component(pair), String::new())
            }
        })
        .collect()
}

/// Look up a query parameter by name.
fn query_has_key(params: &[(String, String)], key: &str) -> bool {
    params.iter().any(|(k, _)| k == key)
}

/// Get the value of a query parameter by name.
fn query_value<'a>(params: &'a [(String, String)], key: &str) -> Option<&'a str> {
    params
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// Identify the S3 operation from the HTTP method, path structure, query params, and headers.
///
/// This is the core routing logic that maps the HTTP request characteristics to a
/// specific S3 operation variant.
fn identify_operation(
    method: &Method,
    bucket: Option<&String>,
    key: Option<&String>,
    query_params: &[(String, String)],
    headers: &http::HeaderMap,
) -> Result<S3Operation, S3Error> {
    let has_bucket = bucket.is_some();
    let has_key = key.is_some();

    match (method, has_bucket, has_key) {
        // No bucket: only ListBuckets is valid.
        (&Method::GET, false, false) => Ok(S3Operation::ListBuckets),

        // Bucket-level operations (no key).
        (method, true, false) => identify_bucket_operation(method, query_params, headers),

        // Object-level operations (bucket + key).
        (method, true, true) => identify_object_operation(method, query_params, headers),

        // Invalid: key without bucket should not occur.
        (_, false, true) => Err(S3Error::with_message(
            S3ErrorCode::InvalidRequest,
            "Object key specified without bucket",
        )),

        // No bucket and method other than GET: invalid.
        (_, false, false) => Err(S3Error::with_message(
            S3ErrorCode::MethodNotAllowed,
            "Only GET is allowed at the service level",
        )),
    }
}

/// Identify a bucket-level operation (bucket present, no key).
fn identify_bucket_operation(
    method: &Method,
    params: &[(String, String)],
    headers: &http::HeaderMap,
) -> Result<S3Operation, S3Error> {
    match *method {
        Method::GET => Ok(identify_bucket_get(params)),
        Method::PUT => Ok(identify_bucket_put(params, headers)),
        Method::DELETE => Ok(identify_bucket_delete(params)),
        Method::HEAD => Ok(S3Operation::HeadBucket),
        Method::POST => Ok(identify_bucket_post(params)),
        _ => Err(S3Error::method_not_allowed(method.as_str())),
    }
}

/// Identify a GET operation on a bucket.
fn identify_bucket_get(params: &[(String, String)]) -> S3Operation {
    // Check sub-resource query parameters in specificity order.
    // Parameters with specific values first, then presence-only checks.
    if query_has_key(params, "list-type") && query_value(params, "list-type") == Some("2") {
        return S3Operation::ListObjectsV2;
    }

    if query_has_key(params, "location") {
        return S3Operation::GetBucketLocation;
    }
    if query_has_key(params, "versioning") {
        return S3Operation::GetBucketVersioning;
    }
    if query_has_key(params, "encryption") {
        return S3Operation::GetBucketEncryption;
    }
    if query_has_key(params, "cors") {
        return S3Operation::GetBucketCors;
    }
    if query_has_key(params, "lifecycle") {
        return S3Operation::GetBucketLifecycleConfiguration;
    }
    if query_has_key(params, "policy") {
        return S3Operation::GetBucketPolicy;
    }
    if query_has_key(params, "tagging") {
        return S3Operation::GetBucketTagging;
    }
    if query_has_key(params, "notification") {
        return S3Operation::GetBucketNotificationConfiguration;
    }
    if query_has_key(params, "logging") {
        return S3Operation::GetBucketLogging;
    }
    if query_has_key(params, "publicAccessBlock") {
        return S3Operation::GetPublicAccessBlock;
    }
    if query_has_key(params, "ownershipControls") {
        return S3Operation::GetBucketOwnershipControls;
    }
    if query_has_key(params, "object-lock") {
        return S3Operation::GetObjectLockConfiguration;
    }
    if query_has_key(params, "accelerate") {
        return S3Operation::GetBucketAccelerateConfiguration;
    }
    if query_has_key(params, "requestPayment") {
        return S3Operation::GetBucketRequestPayment;
    }
    if query_has_key(params, "website") {
        return S3Operation::GetBucketWebsite;
    }
    if query_has_key(params, "acl") {
        return S3Operation::GetBucketAcl;
    }
    if query_has_key(params, "policyStatus") {
        return S3Operation::GetBucketPolicyStatus;
    }
    if query_has_key(params, "uploads") {
        return S3Operation::ListMultipartUploads;
    }
    if query_has_key(params, "versions") {
        return S3Operation::ListObjectVersions;
    }

    // Default: ListObjects (v1).
    S3Operation::ListObjects
}

/// Identify a PUT operation on a bucket.
fn identify_bucket_put(params: &[(String, String)], _headers: &http::HeaderMap) -> S3Operation {
    if query_has_key(params, "versioning") {
        return S3Operation::PutBucketVersioning;
    }
    if query_has_key(params, "encryption") {
        return S3Operation::PutBucketEncryption;
    }
    if query_has_key(params, "cors") {
        return S3Operation::PutBucketCors;
    }
    if query_has_key(params, "lifecycle") {
        return S3Operation::PutBucketLifecycleConfiguration;
    }
    if query_has_key(params, "policy") {
        return S3Operation::PutBucketPolicy;
    }
    if query_has_key(params, "tagging") {
        return S3Operation::PutBucketTagging;
    }
    if query_has_key(params, "notification") {
        return S3Operation::PutBucketNotificationConfiguration;
    }
    if query_has_key(params, "logging") {
        return S3Operation::PutBucketLogging;
    }
    if query_has_key(params, "publicAccessBlock") {
        return S3Operation::PutPublicAccessBlock;
    }
    if query_has_key(params, "ownershipControls") {
        return S3Operation::PutBucketOwnershipControls;
    }
    if query_has_key(params, "object-lock") {
        return S3Operation::PutObjectLockConfiguration;
    }
    if query_has_key(params, "accelerate") {
        return S3Operation::PutBucketAccelerateConfiguration;
    }
    if query_has_key(params, "requestPayment") {
        return S3Operation::PutBucketRequestPayment;
    }
    if query_has_key(params, "website") {
        return S3Operation::PutBucketWebsite;
    }
    if query_has_key(params, "acl") {
        return S3Operation::PutBucketAcl;
    }

    // Default: CreateBucket.
    S3Operation::CreateBucket
}

/// Identify a DELETE operation on a bucket.
fn identify_bucket_delete(params: &[(String, String)]) -> S3Operation {
    if query_has_key(params, "encryption") {
        return S3Operation::DeleteBucketEncryption;
    }
    if query_has_key(params, "cors") {
        return S3Operation::DeleteBucketCors;
    }
    if query_has_key(params, "lifecycle") {
        return S3Operation::DeleteBucketLifecycle;
    }
    if query_has_key(params, "policy") {
        return S3Operation::DeleteBucketPolicy;
    }
    if query_has_key(params, "tagging") {
        return S3Operation::DeleteBucketTagging;
    }
    if query_has_key(params, "publicAccessBlock") {
        return S3Operation::DeletePublicAccessBlock;
    }
    if query_has_key(params, "ownershipControls") {
        return S3Operation::DeleteBucketOwnershipControls;
    }
    if query_has_key(params, "website") {
        return S3Operation::DeleteBucketWebsite;
    }

    // Default: DeleteBucket.
    S3Operation::DeleteBucket
}

/// Identify a POST operation on a bucket.
fn identify_bucket_post(params: &[(String, String)]) -> S3Operation {
    if query_has_key(params, "delete") {
        return S3Operation::DeleteObjects;
    }

    // POST to a bucket without ?delete is a PostObject (browser-based upload).
    // The PostObject handler validates that the body is multipart/form-data.
    S3Operation::PostObject
}

/// Identify an object-level operation (bucket + key present).
fn identify_object_operation(
    method: &Method,
    params: &[(String, String)],
    headers: &http::HeaderMap,
) -> Result<S3Operation, S3Error> {
    match *method {
        Method::GET => Ok(identify_object_get(params)),
        Method::PUT => Ok(identify_object_put(params, headers)),
        Method::DELETE => Ok(identify_object_delete(params)),
        Method::HEAD => Ok(S3Operation::HeadObject),
        Method::POST => identify_object_post(params),
        _ => Err(S3Error::method_not_allowed(method.as_str())),
    }
}

/// Identify a GET operation on an object.
fn identify_object_get(params: &[(String, String)]) -> S3Operation {
    if query_has_key(params, "tagging") {
        return S3Operation::GetObjectTagging;
    }
    if query_has_key(params, "acl") {
        return S3Operation::GetObjectAcl;
    }
    if query_has_key(params, "retention") {
        return S3Operation::GetObjectRetention;
    }
    if query_has_key(params, "legal-hold") {
        return S3Operation::GetObjectLegalHold;
    }
    if query_has_key(params, "attributes") {
        return S3Operation::GetObjectAttributes;
    }
    if query_has_key(params, "uploadId") {
        return S3Operation::ListParts;
    }

    // Default: GetObject.
    S3Operation::GetObject
}

/// Identify a PUT operation on an object.
fn identify_object_put(params: &[(String, String)], headers: &http::HeaderMap) -> S3Operation {
    let has_copy_source = headers.contains_key("x-amz-copy-source");

    if query_has_key(params, "tagging") {
        return S3Operation::PutObjectTagging;
    }
    if query_has_key(params, "acl") {
        return S3Operation::PutObjectAcl;
    }
    if query_has_key(params, "retention") {
        return S3Operation::PutObjectRetention;
    }
    if query_has_key(params, "legal-hold") {
        return S3Operation::PutObjectLegalHold;
    }

    // UploadPart / UploadPartCopy: must have both partNumber and uploadId.
    if query_has_key(params, "partNumber") && query_has_key(params, "uploadId") {
        return if has_copy_source {
            S3Operation::UploadPartCopy
        } else {
            S3Operation::UploadPart
        };
    }

    // CopyObject vs PutObject.
    if has_copy_source {
        return S3Operation::CopyObject;
    }

    S3Operation::PutObject
}

/// Identify a DELETE operation on an object.
fn identify_object_delete(params: &[(String, String)]) -> S3Operation {
    if query_has_key(params, "tagging") {
        return S3Operation::DeleteObjectTagging;
    }
    if query_has_key(params, "uploadId") {
        return S3Operation::AbortMultipartUpload;
    }

    // Default: DeleteObject.
    S3Operation::DeleteObject
}

/// Identify a POST operation on an object.
fn identify_object_post(params: &[(String, String)]) -> Result<S3Operation, S3Error> {
    if query_has_key(params, "uploads") {
        return Ok(S3Operation::CreateMultipartUpload);
    }
    if query_has_key(params, "uploadId") {
        return Ok(S3Operation::CompleteMultipartUpload);
    }

    Err(S3Error::with_message(
        S3ErrorCode::MethodNotAllowed,
        "The specified method is not allowed against this resource",
    ))
}

#[cfg(test)]
mod tests {
    use http::Request;

    use super::*;

    fn router() -> S3Router {
        S3Router::new("s3.localhost", true)
    }

    fn path_style_router() -> S3Router {
        S3Router::new("s3.localhost", false)
    }

    fn get_request(uri: &str) -> Request<()> {
        Request::builder()
            .method(Method::GET)
            .uri(uri)
            .header("Host", "s3.localhost:4566")
            .body(())
            .expect("valid request")
    }

    fn vhost_request(method: &Method, host: &str, uri: &str) -> Request<()> {
        Request::builder()
            .method(method.clone())
            .uri(uri)
            .header("Host", host)
            .body(())
            .expect("valid request")
    }

    // --- Virtual hosting tests ---

    #[test]
    fn test_should_extract_bucket_from_virtual_host() {
        let req = vhost_request(&Method::GET, "mybucket.s3.localhost:4566", "/");
        let ctx = router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.bucket.as_deref(), Some("mybucket"));
        assert!(ctx.key.is_none());
        assert_eq!(ctx.operation, S3Operation::ListObjects);
    }

    #[test]
    fn test_should_extract_bucket_and_key_from_virtual_host() {
        let req = vhost_request(&Method::GET, "mybucket.s3.localhost:4566", "/mykey/subpath");
        let ctx = router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.bucket.as_deref(), Some("mybucket"));
        assert_eq!(ctx.key.as_deref(), Some("mykey/subpath"));
        assert_eq!(ctx.operation, S3Operation::GetObject);
    }

    #[test]
    fn test_should_ignore_virtual_host_when_disabled() {
        let req = vhost_request(&Method::GET, "mybucket.s3.localhost:4566", "/");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        // With virtual hosting disabled, no bucket extracted from host.
        assert!(ctx.bucket.is_none());
        assert_eq!(ctx.operation, S3Operation::ListBuckets);
    }

    // --- Path-style routing tests ---

    #[test]
    fn test_should_route_list_buckets() {
        let req = get_request("/");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert!(ctx.bucket.is_none());
        assert_eq!(ctx.operation, S3Operation::ListBuckets);
    }

    #[test]
    fn test_should_route_list_objects_from_path() {
        let req = get_request("/mybucket");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.bucket.as_deref(), Some("mybucket"));
        assert!(ctx.key.is_none());
        assert_eq!(ctx.operation, S3Operation::ListObjects);
    }

    #[test]
    fn test_should_route_get_object_from_path() {
        let req = get_request("/mybucket/my/key");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.bucket.as_deref(), Some("mybucket"));
        assert_eq!(ctx.key.as_deref(), Some("my/key"));
        assert_eq!(ctx.operation, S3Operation::GetObject);
    }

    // --- Bucket-level operation routing ---

    #[test]
    fn test_should_route_create_bucket() {
        let req = Request::builder()
            .method(Method::PUT)
            .uri("/mybucket")
            .header("Host", "s3.localhost:4566")
            .body(())
            .expect("valid request");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::CreateBucket);
    }

    #[test]
    fn test_should_route_delete_bucket() {
        let req = Request::builder()
            .method(Method::DELETE)
            .uri("/mybucket")
            .header("Host", "s3.localhost:4566")
            .body(())
            .expect("valid request");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::DeleteBucket);
    }

    #[test]
    fn test_should_route_head_bucket() {
        let req = Request::builder()
            .method(Method::HEAD)
            .uri("/mybucket")
            .header("Host", "s3.localhost:4566")
            .body(())
            .expect("valid request");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::HeadBucket);
    }

    #[test]
    fn test_should_route_get_bucket_versioning() {
        let req = get_request("/mybucket?versioning");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::GetBucketVersioning);
    }

    #[test]
    fn test_should_route_get_bucket_location() {
        let req = get_request("/mybucket?location");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::GetBucketLocation);
    }

    #[test]
    fn test_should_route_get_bucket_encryption() {
        let req = get_request("/mybucket?encryption");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::GetBucketEncryption);
    }

    #[test]
    fn test_should_route_get_bucket_cors() {
        let req = get_request("/mybucket?cors");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::GetBucketCors);
    }

    #[test]
    fn test_should_route_get_bucket_tagging() {
        let req = get_request("/mybucket?tagging");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::GetBucketTagging);
    }

    #[test]
    fn test_should_route_list_objects_v2() {
        let req = get_request("/mybucket?list-type=2");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::ListObjectsV2);
    }

    #[test]
    fn test_should_route_list_object_versions() {
        let req = get_request("/mybucket?versions");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::ListObjectVersions);
    }

    #[test]
    fn test_should_route_list_multipart_uploads() {
        let req = get_request("/mybucket?uploads");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::ListMultipartUploads);
    }

    #[test]
    fn test_should_route_delete_objects() {
        let req = Request::builder()
            .method(Method::POST)
            .uri("/mybucket?delete")
            .header("Host", "s3.localhost:4566")
            .body(())
            .expect("valid request");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::DeleteObjects);
    }

    #[test]
    fn test_should_route_put_bucket_versioning() {
        let req = Request::builder()
            .method(Method::PUT)
            .uri("/mybucket?versioning")
            .header("Host", "s3.localhost:4566")
            .body(())
            .expect("valid request");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::PutBucketVersioning);
    }

    #[test]
    fn test_should_route_delete_bucket_encryption() {
        let req = Request::builder()
            .method(Method::DELETE)
            .uri("/mybucket?encryption")
            .header("Host", "s3.localhost:4566")
            .body(())
            .expect("valid request");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::DeleteBucketEncryption);
    }

    // --- Object-level operation routing ---

    #[test]
    fn test_should_route_put_object() {
        let req = Request::builder()
            .method(Method::PUT)
            .uri("/mybucket/mykey")
            .header("Host", "s3.localhost:4566")
            .body(())
            .expect("valid request");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::PutObject);
    }

    #[test]
    fn test_should_route_copy_object() {
        let req = Request::builder()
            .method(Method::PUT)
            .uri("/mybucket/mykey")
            .header("Host", "s3.localhost:4566")
            .header("x-amz-copy-source", "/srcbucket/srckey")
            .body(())
            .expect("valid request");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::CopyObject);
    }

    #[test]
    fn test_should_route_head_object() {
        let req = Request::builder()
            .method(Method::HEAD)
            .uri("/mybucket/mykey")
            .header("Host", "s3.localhost:4566")
            .body(())
            .expect("valid request");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::HeadObject);
    }

    #[test]
    fn test_should_route_delete_object() {
        let req = Request::builder()
            .method(Method::DELETE)
            .uri("/mybucket/mykey")
            .header("Host", "s3.localhost:4566")
            .body(())
            .expect("valid request");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::DeleteObject);
    }

    #[test]
    fn test_should_route_get_object_tagging() {
        let req = get_request("/mybucket/mykey?tagging");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::GetObjectTagging);
    }

    #[test]
    fn test_should_route_put_object_tagging() {
        let req = Request::builder()
            .method(Method::PUT)
            .uri("/mybucket/mykey?tagging")
            .header("Host", "s3.localhost:4566")
            .body(())
            .expect("valid request");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::PutObjectTagging);
    }

    #[test]
    fn test_should_route_delete_object_tagging() {
        let req = Request::builder()
            .method(Method::DELETE)
            .uri("/mybucket/mykey?tagging")
            .header("Host", "s3.localhost:4566")
            .body(())
            .expect("valid request");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::DeleteObjectTagging);
    }

    #[test]
    fn test_should_route_get_object_acl() {
        let req = get_request("/mybucket/mykey?acl");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::GetObjectAcl);
    }

    #[test]
    fn test_should_route_get_object_attributes() {
        let req = get_request("/mybucket/mykey?attributes");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::GetObjectAttributes);
    }

    // --- Multipart operations ---

    #[test]
    fn test_should_route_create_multipart_upload() {
        let req = Request::builder()
            .method(Method::POST)
            .uri("/mybucket/mykey?uploads")
            .header("Host", "s3.localhost:4566")
            .body(())
            .expect("valid request");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::CreateMultipartUpload);
    }

    #[test]
    fn test_should_route_upload_part() {
        let req = Request::builder()
            .method(Method::PUT)
            .uri("/mybucket/mykey?partNumber=1&uploadId=abc123")
            .header("Host", "s3.localhost:4566")
            .body(())
            .expect("valid request");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::UploadPart);
    }

    #[test]
    fn test_should_route_upload_part_copy() {
        let req = Request::builder()
            .method(Method::PUT)
            .uri("/mybucket/mykey?partNumber=1&uploadId=abc123")
            .header("Host", "s3.localhost:4566")
            .header("x-amz-copy-source", "/srcbucket/srckey")
            .body(())
            .expect("valid request");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::UploadPartCopy);
    }

    #[test]
    fn test_should_route_complete_multipart_upload() {
        let req = Request::builder()
            .method(Method::POST)
            .uri("/mybucket/mykey?uploadId=abc123")
            .header("Host", "s3.localhost:4566")
            .body(())
            .expect("valid request");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::CompleteMultipartUpload);
    }

    #[test]
    fn test_should_route_abort_multipart_upload() {
        let req = Request::builder()
            .method(Method::DELETE)
            .uri("/mybucket/mykey?uploadId=abc123")
            .header("Host", "s3.localhost:4566")
            .body(())
            .expect("valid request");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::AbortMultipartUpload);
    }

    #[test]
    fn test_should_route_list_parts() {
        let req = get_request("/mybucket/mykey?uploadId=abc123");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::ListParts);
    }

    // --- Edge cases ---

    #[test]
    fn test_should_decode_percent_encoded_key() {
        let req = get_request("/mybucket/my%20key%2Fwith%2Fslashes");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.key.as_deref(), Some("my key/with/slashes"));
    }

    #[test]
    fn test_should_parse_query_params_correctly() {
        let params = parse_query_params("prefix=test&max-keys=100&delimiter=%2F");
        assert_eq!(params.len(), 3);
        assert_eq!(query_value(&params, "prefix"), Some("test"));
        assert_eq!(query_value(&params, "max-keys"), Some("100"));
        assert_eq!(query_value(&params, "delimiter"), Some("/"));
    }

    #[test]
    fn test_should_handle_empty_query_string() {
        let params = parse_query_params("");
        assert_eq!(params.len(), 0);
    }

    #[test]
    fn test_should_handle_key_only_query_params() {
        let params = parse_query_params("versioning");
        assert_eq!(params.len(), 1);
        assert!(query_has_key(&params, "versioning"));
        assert_eq!(query_value(&params, "versioning"), Some(""));
    }

    #[test]
    fn test_should_route_get_bucket_acl() {
        let req = get_request("/mybucket?acl");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::GetBucketAcl);
    }

    #[test]
    fn test_should_route_get_bucket_policy_status() {
        let req = get_request("/mybucket?policyStatus");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::GetBucketPolicyStatus);
    }

    #[test]
    fn test_should_route_get_object_retention() {
        let req = get_request("/mybucket/mykey?retention");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::GetObjectRetention);
    }

    #[test]
    fn test_should_route_get_object_legal_hold() {
        let req = get_request("/mybucket/mykey?legal-hold");
        let ctx = path_style_router().resolve(&req).expect("should resolve");
        assert_eq!(ctx.operation, S3Operation::GetObjectLegalHold);
    }

    #[test]
    fn test_should_reject_unsupported_method() {
        let req = Request::builder()
            .method(Method::PATCH)
            .uri("/mybucket")
            .header("Host", "s3.localhost:4566")
            .body(())
            .expect("valid request");
        let err = path_style_router().resolve(&req).unwrap_err();
        assert_eq!(err.code, S3ErrorCode::MethodNotAllowed);
    }
}
