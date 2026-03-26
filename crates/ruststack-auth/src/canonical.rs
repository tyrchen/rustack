//! Canonical request construction for AWS Signature Version 4.
//!
//! This module implements the canonical request format as specified by AWS:
//!
//! ```text
//! HTTPRequestMethod\n
//! CanonicalURI\n
//! CanonicalQueryString\n
//! CanonicalHeaders\n\n
//! SignedHeaders\n
//! HashedPayload
//! ```
//!
//! Each component is normalized according to the AWS specification to ensure
//! deterministic signature computation.

use std::collections::BTreeMap;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};

/// The set of characters that must be percent-encoded in URI path segments.
///
/// Per AWS SigV4 spec, all characters except unreserved characters
/// (A-Z, a-z, 0-9, `-`, `_`, `.`, `~`) must be encoded.
/// Forward slashes in the path are preserved (not encoded).
const URI_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// Build the full canonical request string from its components.
///
/// The result is a newline-separated string of:
/// 1. HTTP method
/// 2. Canonical URI
/// 3. Canonical query string
/// 4. Canonical headers (terminated by an extra newline)
/// 5. Signed headers
/// 6. Hashed payload
///
/// # Examples
///
/// ```
/// use ruststack_auth::canonical::build_canonical_request;
///
/// let canonical = build_canonical_request(
///     "GET",
///     "/test.txt",
///     "",
///     &[("host", "examplebucket.s3.amazonaws.com")],
///     &["host"],
///     "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
/// );
/// assert!(canonical.starts_with("GET\n/test.txt\n"));
/// ```
#[must_use]
pub fn build_canonical_request(
    method: &str,
    uri: &str,
    query_string: &str,
    headers: &[(&str, &str)],
    signed_headers: &[&str],
    payload_hash: &str,
) -> String {
    let canonical_uri = build_canonical_uri(uri);
    let canonical_query = build_canonical_query_string(query_string);
    let canonical_headers = build_canonical_headers(headers, signed_headers);
    let signed_headers_str = build_signed_headers_string(signed_headers);

    format!(
        "{method}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n\\
         n{signed_headers_str}\n{payload_hash}"
    )
}

/// Build the canonical URI by URI-encoding each path segment individually.
///
/// Forward slashes (`/`) are preserved. Empty paths are normalized to `/`.
/// Each segment is percent-encoded according to RFC 3986 unreserved characters.
///
/// # Examples
///
/// ```
/// use ruststack_auth::canonical::build_canonical_uri;
///
/// assert_eq!(build_canonical_uri("/test.txt"), "/test.txt");
/// assert_eq!(build_canonical_uri("/"), "/");
/// assert_eq!(build_canonical_uri(""), "/");
/// ```
#[must_use]
pub fn build_canonical_uri(path: &str) -> String {
    if path.is_empty() || path == "/" {
        return "/".to_owned();
    }

    let segments: Vec<&str> = path.split('/').collect();
    let encoded_segments: Vec<String> = segments
        .iter()
        .map(|segment| {
            // Decode first to normalize, then re-encode to produce consistent canonical form.
            // This prevents double-encoding when the path is already percent-encoded.
            let decoded = percent_decode_str(segment).decode_utf8_lossy();
            uri_encode(&decoded)
        })
        .collect();

    encoded_segments.join("/")
}

/// Build the canonical query string by sorting parameters.
///
/// Parameters are sorted by key name first, then by value for duplicate keys.
/// The raw query string values are preserved as-is (no decode/re-encode) because
/// different clients use different encoding rules when signing. For example,
/// AWS SDKs percent-encode `:` and `*` but minio-java (via OkHttp) leaves them
/// raw. The server must use the exact same encoding the client used for signing,
/// which is whatever appears in the HTTP request.
///
/// # Examples
///
/// ```
/// use ruststack_auth::canonical::build_canonical_query_string;
///
/// assert_eq!(build_canonical_query_string(""), "");
/// assert_eq!(
///     build_canonical_query_string("b=2&a=1"),
///     "a=1&b=2"
/// );
/// ```
#[must_use]
pub fn build_canonical_query_string(query: &str) -> String {
    if query.is_empty() {
        return String::new();
    }

    let mut params: Vec<(&str, &str)> = query
        .split('&')
        .filter(|s| !s.is_empty())
        .map(|param| param.split_once('=').unwrap_or((param, "")))
        .collect();

    params.sort_unstable();

    params
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

/// Build the canonical headers string from the request headers.
///
/// Only headers listed in `signed_headers` are included. Header names are lowercased,
/// values are trimmed of leading/trailing whitespace and consecutive spaces are collapsed
/// to a single space. Headers are sorted by name.
///
/// The result does NOT include a trailing newline; the caller adds that as part of
/// the canonical request format (the double newline between headers and signed headers).
///
/// # Examples
///
/// ```
/// use ruststack_auth::canonical::build_canonical_headers;
///
/// let headers = vec![
///     ("Host", "example.com"),
///     ("X-Amz-Date", "20130524T000000Z"),
/// ];
/// let signed = vec!["host", "x-amz-date"];
/// let result = build_canonical_headers(
///     &headers.iter().map(|(k, v)| (*k, *v)).collect::<Vec<_>>(),
///     &signed.iter().map(|s| *s).collect::<Vec<_>>(),
/// );
/// assert!(result.contains("host:example.com"));
/// ```
#[must_use]
pub fn build_canonical_headers(headers: &[(&str, &str)], signed_headers: &[&str]) -> String {
    // Collect headers into a sorted map, keyed by lowercase name.
    // If multiple headers share the same name, their values are concatenated with commas.
    let mut header_map: BTreeMap<String, String> = BTreeMap::new();
    for (name, value) in headers {
        let lower_name = name.to_lowercase();
        let trimmed_value = collapse_whitespace(value.trim());
        header_map
            .entry(lower_name)
            .and_modify(|existing| {
                existing.push(',');
                existing.push_str(&trimmed_value);
            })
            .or_insert(trimmed_value);
    }

    // Build the canonical headers string using only the signed headers, in sorted order.
    let mut sorted_signed: Vec<&str> = signed_headers.to_vec();
    sorted_signed.sort_unstable();

    sorted_signed
        .iter()
        .filter_map(|name| header_map.get(*name).map(|value| format!("{name}:{value}")))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Build the signed headers string as a semicolon-separated list of lowercase header names.
///
/// The header names are sorted lexicographically.
///
/// # Examples
///
/// ```
/// use ruststack_auth::canonical::build_signed_headers_string;
///
/// assert_eq!(
///     build_signed_headers_string(&["x-amz-date", "host"]),
///     "host;x-amz-date"
/// );
/// ```
#[must_use]
pub fn build_signed_headers_string(signed_headers: &[&str]) -> String {
    let mut sorted: Vec<&str> = signed_headers.to_vec();
    sorted.sort_unstable();
    sorted.join(";")
}

/// URI-encode a single path segment using the AWS SigV4 encoding rules.
fn uri_encode(input: &str) -> String {
    utf8_percent_encode(input, URI_ENCODE_SET).to_string()
}

/// Collapse consecutive whitespace characters in a string to a single space.
fn collapse_whitespace(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut prev_was_space = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !prev_was_space {
                result.push(' ');
                prev_was_space = true;
            }
        } else {
            result.push(ch);
            prev_was_space = false;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_build_canonical_uri_for_simple_path() {
        assert_eq!(build_canonical_uri("/test.txt"), "/test.txt");
    }

    #[test]
    fn test_should_normalize_empty_path_to_slash() {
        assert_eq!(build_canonical_uri(""), "/");
        assert_eq!(build_canonical_uri("/"), "/");
    }

    #[test]
    fn test_should_encode_special_characters_in_path() {
        assert_eq!(build_canonical_uri("/hello world"), "/hello%20world");
    }

    #[test]
    fn test_should_sort_query_parameters() {
        assert_eq!(build_canonical_query_string("b=2&a=1&c=3"), "a=1&b=2&c=3");
    }

    #[test]
    fn test_should_return_empty_for_empty_query() {
        assert_eq!(build_canonical_query_string(""), "");
    }

    #[test]
    fn test_should_preserve_raw_query_parameter_values() {
        // Raw values are preserved as-is — no re-encoding is applied.
        assert_eq!(
            build_canonical_query_string("key=hello%20world"),
            "key=hello%20world"
        );
    }

    #[test]
    fn test_should_build_canonical_headers_sorted_and_lowercased() {
        let headers = [
            ("Host", "examplebucket.s3.amazonaws.com"),
            ("Range", "bytes=0-9"),
            (
                "x-amz-content-sha256",
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            ),
            ("x-amz-date", "20130524T000000Z"),
        ];
        let signed = ["host", "range", "x-amz-content-sha256", "x-amz-date"];
        let result = build_canonical_headers(
            &headers.iter().map(|(k, v)| (*k, *v)).collect::<Vec<_>>(),
            &signed,
        );
        let expected = "host:examplebucket.s3.amazonaws.com\nrange:bytes=0-9\\
                        \
                        nx-amz-content-sha256:\
                        e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\\
                        nx-amz-date:20130524T000000Z";
        assert_eq!(result, expected);
    }

    #[test]
    fn test_should_build_signed_headers_string_sorted() {
        assert_eq!(
            build_signed_headers_string(&["x-amz-date", "host", "range"]),
            "host;range;x-amz-date"
        );
    }

    #[test]
    fn test_should_collapse_whitespace_in_header_values() {
        let headers = [("Host", "  example.com  "), ("X-Custom", "a   b   c")];
        let signed = ["host", "x-custom"];
        let result = build_canonical_headers(
            &headers.iter().map(|(k, v)| (*k, *v)).collect::<Vec<_>>(),
            &signed,
        );
        assert_eq!(result, "host:example.com\nx-custom:a b c");
    }

    #[test]
    fn test_should_build_canonical_request_matching_aws_example() {
        use sha2::{Digest, Sha256};

        // AWS test vector: GET /test.txt from examplebucket
        let headers = vec![
            ("host", "examplebucket.s3.amazonaws.com"),
            ("range", "bytes=0-9"),
            (
                "x-amz-content-sha256",
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            ),
            ("x-amz-date", "20130524T000000Z"),
        ];
        let signed_headers = vec!["host", "range", "x-amz-content-sha256", "x-amz-date"];

        let canonical = build_canonical_request(
            "GET",
            "/test.txt",
            "",
            &headers,
            &signed_headers,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        );

        let expected = "GET\n/test.txt\n\nhost:examplebucket.s3.amazonaws.com\nrange:bytes=0-9\\
                        \
                        nx-amz-content-sha256:\
                        e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\\
                        \
                        nx-amz-date:20130524T000000Z\n\nhost;range;x-amz-content-sha256;\
                        x-amz-date\\
                        ne3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(canonical, expected);

        // Verify the hash of the canonical request matches the AWS test vector
        let hash = hex::encode(Sha256::digest(canonical.as_bytes()));
        assert_eq!(
            hash,
            "7344ae5b7ee6c3e7e6b0fe0640412a37625d1fbfff95c48bbb2dc43964946972"
        );
    }

    #[test]
    fn test_should_handle_presigned_url_query_string() {
        let query = "X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%\
                     2F20130524%2Fus-east-1%2Fs3%2Faws4_request&X-Amz-Date=20130524T000000Z&\
                     X-Amz-Expires=86400&X-Amz-SignedHeaders=host";
        let result = build_canonical_query_string(query);
        // Should be sorted, raw values preserved
        assert!(result.contains("X-Amz-Algorithm=AWS4-HMAC-SHA256"));
        assert!(result.contains("X-Amz-Expires=86400"));
        // %2F should be preserved, not double-encoded to %252F
        assert!(result.contains("AKIAIOSFODNN7EXAMPLE%2F20130524%2Fus-east-1%2Fs3%2Faws4_request"));
    }

    #[test]
    fn test_should_preserve_percent_encoded_query_parameters() {
        // Percent-encoded values are preserved as-is.
        let query = "events=s3%3AObjectCreated%3A%2A&prefix=test";
        let result = build_canonical_query_string(query);
        assert_eq!(result, "events=s3%3AObjectCreated%3A%2A&prefix=test");
    }

    #[test]
    fn test_should_preserve_raw_special_characters_in_query() {
        // Raw (unencoded) special characters are preserved as-is.
        // This matches minio-java behavior which uses OkHttp's encoding
        // that leaves `:` and `*` unencoded in query strings.
        let raw = "events=s3:ObjectCreated:*&prefix=test";
        let result = build_canonical_query_string(raw);
        assert_eq!(result, "events=s3:ObjectCreated:*&prefix=test");
    }

    #[test]
    fn test_should_sort_duplicate_query_keys() {
        // Duplicate keys should be sorted by value.
        let query = "events=s3:ObjectCreated:*&events=s3:ObjectAccessed:*&prefix=p";
        let result = build_canonical_query_string(query);
        assert_eq!(
            result,
            "events=s3:ObjectAccessed:*&events=s3:ObjectCreated:*&prefix=p"
        );
    }

    #[test]
    fn test_should_not_double_encode_uri_path() {
        // Path with already percent-encoded space
        assert_eq!(build_canonical_uri("/hello%20world"), "/hello%20world");
        // Raw path should produce the same result
        assert_eq!(
            build_canonical_uri("/hello world"),
            build_canonical_uri("/hello%20world")
        );
    }
}
