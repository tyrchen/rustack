//! Shared utilities for the S3 service.
//!
//! Provides ID generation, timestamp helpers, range-header parsing,
//! conditional-request matching, continuation-token encoding, and XML
//! escaping functions.

use base64::{Engine, engine::general_purpose::STANDARD as BASE64_STANDARD};
use chrono::Utc;
use rand::RngExt;
use uuid::Uuid;

use crate::error::S3ServiceError;

// ---------------------------------------------------------------------------
// ID generation
// ---------------------------------------------------------------------------

/// Generate a random version ID suitable for S3 versioned objects.
///
/// Produces a URL-safe base64 string of approximately 32 characters.
///
/// # Examples
///
/// ```
/// use rustack_s3_core::utils::generate_version_id;
///
/// let id = generate_version_id();
/// assert!(id.len() >= 20);
/// assert!(id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
/// ```
#[must_use]
pub fn generate_version_id() -> String {
    let mut rng = rand::rng();
    let mut buf = [0u8; 24];
    rng.fill(&mut buf);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

/// Generate a random upload ID for multipart uploads.
///
/// Produces a hex string of approximately 64 characters.
///
/// # Examples
///
/// ```
/// use rustack_s3_core::utils::generate_upload_id;
///
/// let id = generate_upload_id();
/// assert!(id.len() >= 32);
/// assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
/// ```
#[must_use]
pub fn generate_upload_id() -> String {
    let mut rng = rand::rng();
    let mut buf = [0u8; 32];
    rng.fill(&mut buf);
    hex::encode(buf)
}

/// Generate a unique request ID (UUID v4 without dashes).
///
/// # Examples
///
/// ```
/// use rustack_s3_core::utils::generate_request_id;
///
/// let id = generate_request_id();
/// assert_eq!(id.len(), 32);
/// assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
/// ```
#[must_use]
pub fn generate_request_id() -> String {
    Uuid::new_v4().simple().to_string()
}

// ---------------------------------------------------------------------------
// Timestamps
// ---------------------------------------------------------------------------

/// Return the current UTC time as milliseconds since the Unix epoch.
///
/// # Examples
///
/// ```
/// use rustack_s3_core::utils::timestamp_millis;
///
/// let ts = timestamp_millis();
/// assert!(ts > 0);
/// ```
#[must_use]
pub fn timestamp_millis() -> i64 {
    Utc::now().timestamp_millis()
}

/// Return the current UTC time formatted as an RFC 3339 string.
///
/// # Examples
///
/// ```
/// use rustack_s3_core::utils::timestamp_rfc3339;
///
/// let ts = timestamp_rfc3339();
/// assert!(ts.contains('T'));
/// ```
#[must_use]
pub fn timestamp_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

// ---------------------------------------------------------------------------
// Range header parsing
// ---------------------------------------------------------------------------

/// Parse an HTTP `Range` header value and return the inclusive byte range.
///
/// Supported formats:
/// - `bytes=0-499` -- first 500 bytes
/// - `bytes=-500` -- last 500 bytes
/// - `bytes=500-` -- from byte 500 to the end
/// - `bytes=0-` -- the entire content
///
/// Returns an inclusive `(start, end)` tuple.
///
/// # Errors
///
/// Returns [`S3ServiceError::InvalidRange`] if the range header is malformed
/// or specifies an unsatisfiable range.
///
/// # Examples
///
/// ```
/// use rustack_s3_core::utils::parse_range_header;
///
/// let (start, end) = parse_range_header("bytes=0-499", 1000).unwrap();
/// assert_eq!((start, end), (0, 499));
/// ```
pub fn parse_range_header(range: &str, content_length: u64) -> Result<(u64, u64), S3ServiceError> {
    let range = range
        .strip_prefix("bytes=")
        .ok_or(S3ServiceError::InvalidRange)?;

    if content_length == 0 {
        return Err(S3ServiceError::InvalidRange);
    }

    if let Some(suffix) = range.strip_prefix('-') {
        // bytes=-N  (last N bytes)
        let n: u64 = suffix.parse().map_err(|_| S3ServiceError::InvalidRange)?;
        if n == 0 || n > content_length {
            return Err(S3ServiceError::InvalidRange);
        }
        let start = content_length - n;
        Ok((start, content_length - 1))
    } else if let Some(prefix) = range.strip_suffix('-') {
        // bytes=N-  (from N to end)
        let start: u64 = prefix.parse().map_err(|_| S3ServiceError::InvalidRange)?;
        if start >= content_length {
            return Err(S3ServiceError::InvalidRange);
        }
        Ok((start, content_length - 1))
    } else {
        // bytes=N-M
        let parts: Vec<&str> = range.splitn(2, '-').collect();
        if parts.len() != 2 {
            return Err(S3ServiceError::InvalidRange);
        }
        let start: u64 = parts[0].parse().map_err(|_| S3ServiceError::InvalidRange)?;
        let end: u64 = parts[1].parse().map_err(|_| S3ServiceError::InvalidRange)?;
        if start > end || start >= content_length {
            return Err(S3ServiceError::InvalidRange);
        }
        // Clamp end to content_length - 1
        let end = end.min(content_length - 1);
        Ok((start, end))
    }
}

// ---------------------------------------------------------------------------
// Conditional request helpers
// ---------------------------------------------------------------------------

/// Check whether the given ETag satisfies an `If-Match` condition.
///
/// The `if_match` value may be `"*"` (matches any ETag) or a quoted ETag
/// value.
///
/// # Examples
///
/// ```
/// use rustack_s3_core::utils::is_valid_if_match;
///
/// assert!(is_valid_if_match("\"abc\"", "*"));
/// assert!(is_valid_if_match("\"abc\"", "\"abc\""));
/// assert!(!is_valid_if_match("\"abc\"", "\"xyz\""));
/// ```
#[must_use]
pub fn is_valid_if_match(etag: &str, if_match: &str) -> bool {
    if if_match == "*" {
        return true;
    }
    normalize_etag(etag) == normalize_etag(if_match)
}

/// Check whether the given ETag satisfies an `If-None-Match` condition.
///
/// Returns `true` if the object should be returned (i.e. the ETag does
/// *not* match). Returns `false` if the ETags match (meaning a 304 Not
/// Modified response is appropriate).
///
/// # Examples
///
/// ```
/// use rustack_s3_core::utils::is_valid_if_none_match;
///
/// assert!(!is_valid_if_none_match("\"abc\"", "*"));
/// assert!(!is_valid_if_none_match("\"abc\"", "\"abc\""));
/// assert!(is_valid_if_none_match("\"abc\"", "\"xyz\""));
/// ```
#[must_use]
pub fn is_valid_if_none_match(etag: &str, if_none_match: &str) -> bool {
    if if_none_match == "*" {
        return false;
    }
    normalize_etag(etag) != normalize_etag(if_none_match)
}

/// Normalize an ETag by stripping surrounding double quotes.
fn normalize_etag(etag: &str) -> &str {
    etag.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(etag)
}

// ---------------------------------------------------------------------------
// Continuation tokens
// ---------------------------------------------------------------------------

/// Encode an object key as a base64 continuation token.
///
/// # Examples
///
/// ```
/// use rustack_s3_core::utils::{encode_continuation_token, decode_continuation_token};
///
/// let token = encode_continuation_token("photos/2024/img.jpg");
/// let key = decode_continuation_token(&token).unwrap();
/// assert_eq!(key, "photos/2024/img.jpg");
/// ```
#[must_use]
pub fn encode_continuation_token(key: &str) -> String {
    BASE64_STANDARD.encode(key.as_bytes())
}

/// Decode a base64 continuation token back to an object key.
///
/// # Errors
///
/// Returns [`S3ServiceError::InvalidArgument`] if the token is not valid
/// base64 or does not decode to valid UTF-8.
pub fn decode_continuation_token(token: &str) -> Result<String, S3ServiceError> {
    let bytes = BASE64_STANDARD
        .decode(token)
        .map_err(|_| S3ServiceError::InvalidArgument {
            message: "Invalid continuation token".to_owned(),
        })?;
    String::from_utf8(bytes).map_err(|_| S3ServiceError::InvalidArgument {
        message: "Continuation token contains invalid UTF-8".to_owned(),
    })
}

// ---------------------------------------------------------------------------
// Copy source parsing
// ---------------------------------------------------------------------------

/// Parse the `x-amz-copy-source` header value into bucket, key, and optional
/// version ID components.
///
/// The copy source header uses the format `/bucket/key` or `bucket/key`, with
/// an optional `?versionId=<vid>` suffix. Percent-encoded characters in the
/// key are decoded.
///
/// # Errors
///
/// Returns [`S3ServiceError::InvalidArgument`] if the copy source string
/// is empty or malformed.
///
/// # Examples
///
/// ```
/// use rustack_s3_core::utils::parse_copy_source;
///
/// let (bucket, key, vid) = parse_copy_source("my-bucket/my-key").unwrap();
/// assert_eq!(bucket, "my-bucket");
/// assert_eq!(key, "my-key");
/// assert!(vid.is_none());
/// ```
pub fn parse_copy_source(source: &str) -> Result<(String, String, Option<String>), S3ServiceError> {
    // Strip leading slash if present.
    let source = source.strip_prefix('/').unwrap_or(source);

    // Split off the versionId query parameter if present.
    let (path, version_id) = if let Some((p, query)) = source.split_once('?') {
        let vid = query
            .split('&')
            .find_map(|param| param.strip_prefix("versionId="))
            .map(String::from);
        (p, vid)
    } else {
        (source, None)
    };

    // Split into bucket and key at the first '/'.
    let (bucket, key) = path
        .split_once('/')
        .ok_or_else(|| S3ServiceError::InvalidArgument {
            message: "Invalid copy source: must be in the format bucket/key".to_owned(),
        })?;

    if bucket.is_empty() || key.is_empty() {
        return Err(S3ServiceError::InvalidArgument {
            message: "Invalid copy source: bucket and key must not be empty".to_owned(),
        });
    }

    // URL-decode the key (copy source keys may be percent-encoded).
    let decoded_key = percent_encoding::percent_decode_str(key)
        .decode_utf8()
        .map_err(|_| S3ServiceError::InvalidArgument {
            message: "Invalid copy source: key contains invalid UTF-8".to_owned(),
        })?
        .into_owned();

    Ok((bucket.to_owned(), decoded_key, version_id))
}

// ---------------------------------------------------------------------------
// XML escaping
// ---------------------------------------------------------------------------

/// Escape a string for safe inclusion in XML content.
///
/// Replaces `&`, `<`, `>`, `"`, and `'` with their XML entity references.
///
/// # Examples
///
/// ```
/// use rustack_s3_core::utils::xml_escape;
///
/// assert_eq!(xml_escape("a<b>c"), "a&lt;b&gt;c");
/// assert_eq!(xml_escape("x&y"), "x&amp;y");
/// assert_eq!(xml_escape("hello"), "hello");
/// ```
#[must_use]
pub fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // ID generation
    // -----------------------------------------------------------------------

    #[test]
    fn test_should_generate_unique_version_ids() {
        let id1 = generate_version_id();
        let id2 = generate_version_id();
        assert_ne!(id1, id2);
        assert!(id1.len() >= 20);
    }

    #[test]
    fn test_should_generate_unique_upload_ids() {
        let id1 = generate_upload_id();
        let id2 = generate_upload_id();
        assert_ne!(id1, id2);
        assert_eq!(id1.len(), 64);
        assert!(id1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_should_generate_unique_request_ids() {
        let id1 = generate_request_id();
        let id2 = generate_request_id();
        assert_ne!(id1, id2);
        assert_eq!(id1.len(), 32);
    }

    // -----------------------------------------------------------------------
    // Timestamps
    // -----------------------------------------------------------------------

    #[test]
    fn test_should_return_positive_timestamp_millis() {
        assert!(timestamp_millis() > 0);
    }

    #[test]
    fn test_should_return_rfc3339_timestamp() {
        let ts = timestamp_rfc3339();
        assert!(ts.contains('T'));
        assert!(ts.contains('+') || ts.contains('Z'));
    }

    // -----------------------------------------------------------------------
    // Range parsing
    // -----------------------------------------------------------------------

    #[test]
    fn test_should_parse_range_start_end() {
        let (s, e) = parse_range_header("bytes=0-499", 1000).expect("test parse");
        assert_eq!((s, e), (0, 499));
    }

    #[test]
    fn test_should_parse_range_suffix() {
        let (s, e) = parse_range_header("bytes=-500", 1000).expect("test parse");
        assert_eq!((s, e), (500, 999));
    }

    #[test]
    fn test_should_parse_range_from_offset() {
        let (s, e) = parse_range_header("bytes=500-", 1000).expect("test parse");
        assert_eq!((s, e), (500, 999));
    }

    #[test]
    fn test_should_parse_range_from_zero() {
        let (s, e) = parse_range_header("bytes=0-", 1000).expect("test parse");
        assert_eq!((s, e), (0, 999));
    }

    #[test]
    fn test_should_clamp_range_end_to_content_length() {
        let (s, e) = parse_range_header("bytes=0-9999", 100).expect("test parse");
        assert_eq!((s, e), (0, 99));
    }

    #[test]
    fn test_should_reject_invalid_range_no_prefix() {
        assert!(parse_range_header("0-499", 1000).is_err());
    }

    #[test]
    fn test_should_reject_range_start_beyond_length() {
        assert!(parse_range_header("bytes=1000-", 1000).is_err());
    }

    #[test]
    fn test_should_reject_range_start_greater_than_end() {
        assert!(parse_range_header("bytes=500-100", 1000).is_err());
    }

    #[test]
    fn test_should_reject_range_on_empty_content() {
        assert!(parse_range_header("bytes=0-0", 0).is_err());
    }

    #[test]
    fn test_should_reject_suffix_range_zero() {
        assert!(parse_range_header("bytes=-0", 1000).is_err());
    }

    #[test]
    fn test_should_reject_suffix_range_exceeding_length() {
        assert!(parse_range_header("bytes=-2000", 1000).is_err());
    }

    // -----------------------------------------------------------------------
    // Conditional request matching
    // -----------------------------------------------------------------------

    #[test]
    fn test_should_match_if_match_wildcard() {
        assert!(is_valid_if_match("\"abc\"", "*"));
    }

    #[test]
    fn test_should_match_if_match_same_etag() {
        assert!(is_valid_if_match("\"abc\"", "\"abc\""));
    }

    #[test]
    fn test_should_not_match_if_match_different_etag() {
        assert!(!is_valid_if_match("\"abc\"", "\"xyz\""));
    }

    #[test]
    fn test_should_match_if_match_unquoted() {
        assert!(is_valid_if_match("abc", "abc"));
    }

    #[test]
    fn test_should_not_match_if_none_match_wildcard() {
        assert!(!is_valid_if_none_match("\"abc\"", "*"));
    }

    #[test]
    fn test_should_not_match_if_none_match_same_etag() {
        assert!(!is_valid_if_none_match("\"abc\"", "\"abc\""));
    }

    #[test]
    fn test_should_match_if_none_match_different_etag() {
        assert!(is_valid_if_none_match("\"abc\"", "\"xyz\""));
    }

    // -----------------------------------------------------------------------
    // Continuation tokens
    // -----------------------------------------------------------------------

    #[test]
    fn test_should_roundtrip_continuation_token() {
        let key = "photos/2024/image.jpg";
        let token = encode_continuation_token(key);
        let decoded = decode_continuation_token(&token).expect("test decode");
        assert_eq!(decoded, key);
    }

    #[test]
    fn test_should_roundtrip_empty_continuation_token() {
        let token = encode_continuation_token("");
        let decoded = decode_continuation_token(&token).expect("test decode");
        assert_eq!(decoded, "");
    }

    #[test]
    fn test_should_reject_invalid_continuation_token() {
        assert!(decode_continuation_token("!!!not-base64!!!").is_err());
    }

    // -----------------------------------------------------------------------
    // Copy source parsing
    // -----------------------------------------------------------------------

    #[test]
    fn test_should_parse_copy_source_simple() {
        let (bucket, key, vid) = parse_copy_source("my-bucket/my-key").unwrap();
        assert_eq!(bucket, "my-bucket");
        assert_eq!(key, "my-key");
        assert!(vid.is_none());
    }

    #[test]
    fn test_should_parse_copy_source_with_leading_slash() {
        let (bucket, key, vid) = parse_copy_source("/my-bucket/my-key").unwrap();
        assert_eq!(bucket, "my-bucket");
        assert_eq!(key, "my-key");
        assert!(vid.is_none());
    }

    #[test]
    fn test_should_parse_copy_source_with_version_id() {
        let (bucket, key, vid) = parse_copy_source("/my-bucket/my-key?versionId=abc123").unwrap();
        assert_eq!(bucket, "my-bucket");
        assert_eq!(key, "my-key");
        assert_eq!(vid.as_deref(), Some("abc123"));
    }

    #[test]
    fn test_should_parse_copy_source_with_nested_key() {
        let (bucket, key, vid) = parse_copy_source("bucket/path/to/key").unwrap();
        assert_eq!(bucket, "bucket");
        assert_eq!(key, "path/to/key");
        assert!(vid.is_none());
    }

    #[test]
    fn test_should_parse_copy_source_with_encoded_key() {
        let (bucket, key, vid) = parse_copy_source("bucket/path%20to/key%2B1").unwrap();
        assert_eq!(bucket, "bucket");
        assert_eq!(key, "path to/key+1");
        assert!(vid.is_none());
    }

    #[test]
    fn test_should_reject_copy_source_no_key() {
        assert!(parse_copy_source("bucket-only").is_err());
    }

    #[test]
    fn test_should_reject_copy_source_empty_bucket() {
        assert!(parse_copy_source("/").is_err());
    }

    #[test]
    fn test_should_reject_copy_source_empty_key() {
        assert!(parse_copy_source("bucket/").is_err());
    }

    // -----------------------------------------------------------------------
    // XML escaping
    // -----------------------------------------------------------------------

    #[test]
    fn test_should_escape_ampersand() {
        assert_eq!(xml_escape("a&b"), "a&amp;b");
    }

    #[test]
    fn test_should_escape_angle_brackets() {
        assert_eq!(xml_escape("<tag>"), "&lt;tag&gt;");
    }

    #[test]
    fn test_should_escape_quotes() {
        assert_eq!(xml_escape("he said \"hi\""), "he said &quot;hi&quot;");
    }

    #[test]
    fn test_should_escape_apostrophe() {
        assert_eq!(xml_escape("it's"), "it&apos;s");
    }

    #[test]
    fn test_should_not_escape_plain_text() {
        assert_eq!(xml_escape("hello world"), "hello world");
    }

    #[test]
    fn test_should_handle_empty_string() {
        assert_eq!(xml_escape(""), "");
    }
}
