//! AWS chunked transfer encoding decoder.
//!
//! When S3 clients (e.g. minio-go) send PutObject with `Content-Encoding:
//! aws-chunked`, the body uses a proprietary chunked format:
//!
//! ```text
//! <hex-size>;chunk-signature=<sig>\r\n
//! <data>\r\n
//! 0;chunk-signature=<sig>\r\n
//! <trailing-headers>\r\n
//! \r\n
//! ```
//!
//! This module detects and decodes that framing so the server stores the raw
//! object data rather than the chunk envelope. Trailing headers (e.g. checksum
//! values sent after the body) are extracted and returned separately.

use std::collections::HashMap;

use bytes::{Bytes, BytesMut};
use http::header::HeaderMap;
use ruststack_s3_model::error::{S3Error, S3ErrorCode};

/// Result of decoding an AWS-chunked body.
#[derive(Debug)]
pub struct AwsChunkedResult {
    /// The decoded body data.
    pub body: Bytes,
    /// Trailing headers extracted from the chunked stream.
    ///
    /// Keys are lowercased header names, values are trimmed header values.
    /// The `x-amz-trailer-signature` header is excluded.
    pub trailing_headers: HashMap<String, String>,
}

/// Return `true` if the request uses AWS chunked transfer encoding.
///
/// Detection checks:
/// - `Content-Encoding` header contains `aws-chunked`, OR
/// - `x-amz-content-sha256` starts with `STREAMING-`
pub fn is_aws_chunked(parts: &http::request::Parts) -> bool {
    if let Some(ce) = parts.headers.get(http::header::CONTENT_ENCODING) {
        if let Ok(s) = ce.to_str() {
            if s.contains("aws-chunked") {
                return true;
            }
        }
    }

    if let Some(sha) = parts.headers.get("x-amz-content-sha256") {
        if let Ok(s) = sha.to_str() {
            if s.starts_with("STREAMING-") {
                return true;
            }
        }
    }

    false
}

/// Decode an AWS-chunked body, extracting both the raw payload and trailing
/// headers.
///
/// After the terminal `0`-sized chunk, any trailing headers (e.g.
/// `x-amz-checksum-crc32`) are parsed and returned in
/// [`AwsChunkedResult::trailing_headers`]. The `x-amz-trailer-signature`
/// header is silently skipped.
///
/// # Errors
///
/// Returns an error if the chunked framing is malformed (missing size line,
/// invalid hex size, or truncated data).
pub fn decode_aws_chunked(body: &[u8]) -> Result<AwsChunkedResult, S3Error> {
    let mut output = BytesMut::new();
    let mut pos = 0;

    loop {
        // Find the end of the size line (\r\n).
        let line_end = find_crlf(body, pos).ok_or_else(|| {
            S3Error::with_message(
                S3ErrorCode::InvalidArgument,
                "Malformed aws-chunked body: missing chunk size line",
            )
        })?;

        let size_line = &body[pos..line_end];

        // The size line format is: <hex-size>[;chunk-signature=<sig>][;other-ext]
        // Extract the hex size (everything before the first `;`).
        let hex_part = if let Some(semi) = size_line.iter().position(|&b| b == b';') {
            &size_line[..semi]
        } else {
            size_line
        };

        let hex_str = std::str::from_utf8(hex_part).map_err(|_| {
            S3Error::with_message(
                S3ErrorCode::InvalidArgument,
                "Malformed aws-chunked body: invalid chunk size encoding",
            )
        })?;

        let chunk_size = usize::from_str_radix(hex_str.trim(), 16).map_err(|_| {
            S3Error::with_message(
                S3ErrorCode::InvalidArgument,
                format!("Malformed aws-chunked body: invalid chunk size '{hex_str}'"),
            )
        })?;

        // Skip past the size line CRLF.
        pos = line_end + 2;

        if chunk_size == 0 {
            // Terminal chunk — parse trailing headers from remainder.
            let trailing_headers = parse_trailing_headers(&body[pos..])?;
            return Ok(AwsChunkedResult {
                body: output.freeze(),
                trailing_headers,
            });
        }

        // Read exactly `chunk_size` bytes of data.
        if pos + chunk_size > body.len() {
            return Err(S3Error::with_message(
                S3ErrorCode::InvalidArgument,
                "Malformed aws-chunked body: chunk data truncated",
            ));
        }

        output.extend_from_slice(&body[pos..pos + chunk_size]);
        pos += chunk_size;

        // Expect trailing CRLF after the chunk data.
        if pos + 2 > body.len() || body[pos] != b'\r' || body[pos + 1] != b'\n' {
            return Err(S3Error::with_message(
                S3ErrorCode::InvalidArgument,
                "Malformed aws-chunked body: missing CRLF after chunk data",
            ));
        }
        pos += 2;
    }
}

/// Parse trailing headers from the remainder of an AWS-chunked stream.
///
/// Format: `header-name:value\r\n` repeated, terminated by an empty `\r\n`.
/// The `x-amz-trailer-signature` header is silently skipped since we do not
/// validate trailer signatures.
fn parse_trailing_headers(data: &[u8]) -> Result<HashMap<String, String>, S3Error> {
    let mut headers = HashMap::new();

    let text = std::str::from_utf8(data).map_err(|_| {
        S3Error::with_message(
            S3ErrorCode::InvalidArgument,
            "Malformed aws-chunked body: trailing headers are not valid UTF-8",
        )
    })?;

    for line in text.split("\r\n") {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Skip trailer signature lines.
        if line.starts_with("x-amz-trailer-signature") {
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            headers.insert(key.trim().to_lowercase(), value.trim().to_owned());
        }
    }

    Ok(headers)
}

/// Remove `aws-chunked` from the `Content-Encoding` header.
///
/// If the header becomes empty after removal, the entire header is deleted.
pub fn strip_aws_chunked_encoding(headers: &mut HeaderMap) {
    let Some(ce) = headers.get(http::header::CONTENT_ENCODING) else {
        return;
    };

    let Ok(value) = ce.to_str() else {
        return;
    };

    let remaining: Vec<&str> = value
        .split(',')
        .map(str::trim)
        .filter(|&v| !v.eq_ignore_ascii_case("aws-chunked"))
        .collect();

    if remaining.is_empty() {
        headers.remove(http::header::CONTENT_ENCODING);
    } else if let Ok(new_val) = http::header::HeaderValue::from_str(&remaining.join(", ")) {
        headers.insert(http::header::CONTENT_ENCODING, new_val);
    }
}

/// Find the position of the next `\r\n` starting from `start`.
fn find_crlf(data: &[u8], start: usize) -> Option<usize> {
    if data.len() < start + 2 {
        return None;
    }
    data[start..]
        .windows(2)
        .position(|w| w == b"\r\n")
        .map(|p| start + p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_parts(content_encoding: Option<&str>, sha256: Option<&str>) -> http::request::Parts {
        let mut builder = http::Request::builder().method("PUT").uri("/test");
        if let Some(ce) = content_encoding {
            builder = builder.header("content-encoding", ce);
        }
        if let Some(sha) = sha256 {
            builder = builder.header("x-amz-content-sha256", sha);
        }
        let (parts, ()) = builder.body(()).expect("valid request").into_parts();
        parts
    }

    #[test]
    fn test_should_detect_aws_chunked_content_encoding() {
        let parts = make_parts(Some("aws-chunked"), None);
        assert!(is_aws_chunked(&parts));
    }

    #[test]
    fn test_should_detect_streaming_sha256() {
        let parts = make_parts(None, Some("STREAMING-AWS4-HMAC-SHA256-PAYLOAD"));
        assert!(is_aws_chunked(&parts));
    }

    #[test]
    fn test_should_not_detect_plain_request() {
        let parts = make_parts(None, Some("UNSIGNED-PAYLOAD"));
        assert!(!is_aws_chunked(&parts));
    }

    #[test]
    fn test_should_not_detect_no_headers() {
        let parts = make_parts(None, None);
        assert!(!is_aws_chunked(&parts));
    }

    #[test]
    fn test_should_decode_single_chunk() {
        let body = b"5;chunk-signature=abc123\r\nhello\r\n0;chunk-signature=def456\r\n\r\n";
        let result = decode_aws_chunked(body).expect("should decode");
        assert_eq!(result.body.as_ref(), b"hello");
        assert!(result.trailing_headers.is_empty());
    }

    #[test]
    fn test_should_decode_multiple_chunks() {
        let body =
            b"5;chunk-signature=aaa\r\nhello\r\n6;chunk-signature=bbb\r\n world\r\n0;chunk-signature=ccc\r\n\r\n";
        let result = decode_aws_chunked(body).expect("should decode");
        assert_eq!(result.body.as_ref(), b"hello world");
        assert!(result.trailing_headers.is_empty());
    }

    #[test]
    fn test_should_decode_empty_body() {
        let body = b"0;chunk-signature=abc\r\n\r\n";
        let result = decode_aws_chunked(body).expect("should decode");
        assert!(result.body.is_empty());
        assert!(result.trailing_headers.is_empty());
    }

    #[test]
    fn test_should_reject_malformed_no_crlf() {
        let body = b"5;chunk-signature=abc";
        assert!(decode_aws_chunked(body).is_err());
    }

    #[test]
    fn test_should_reject_truncated_data() {
        let body = b"10;chunk-signature=abc\r\nshort\r\n";
        assert!(decode_aws_chunked(body).is_err());
    }

    #[test]
    fn test_should_extract_trailing_headers() {
        let body = b"5;chunk-signature=aaa\r\nhello\r\n0;chunk-signature=bbb\r\nx-amz-checksum-crc32:AAAAAA==\r\n\r\n";
        let result = decode_aws_chunked(body).expect("should decode");
        assert_eq!(result.body.as_ref(), b"hello");
        assert_eq!(
            result.trailing_headers.get("x-amz-checksum-crc32"),
            Some(&"AAAAAA==".to_owned()),
        );
    }

    #[test]
    fn test_should_skip_trailer_signature_in_trailing_headers() {
        let body = b"3;chunk-signature=aaa\r\nabc\r\n0;chunk-signature=bbb\r\nx-amz-checksum-sha256:abc123\r\nx-amz-trailer-signature:sigvalue\r\n\r\n";
        let result = decode_aws_chunked(body).expect("should decode");
        assert_eq!(result.body.as_ref(), b"abc");
        assert_eq!(
            result.trailing_headers.get("x-amz-checksum-sha256"),
            Some(&"abc123".to_owned()),
        );
        assert!(
            !result
                .trailing_headers
                .contains_key("x-amz-trailer-signature")
        );
    }

    #[test]
    fn test_should_extract_multiple_trailing_headers() {
        let body = b"2;chunk-signature=aaa\r\nhi\r\n0;chunk-signature=bbb\r\nx-amz-checksum-crc32:AAA=\r\nx-amz-checksum-crc32c:BBB=\r\n\r\n";
        let result = decode_aws_chunked(body).expect("should decode");
        assert_eq!(result.body.as_ref(), b"hi");
        assert_eq!(result.trailing_headers.len(), 2);
        assert_eq!(
            result.trailing_headers.get("x-amz-checksum-crc32"),
            Some(&"AAA=".to_owned()),
        );
        assert_eq!(
            result.trailing_headers.get("x-amz-checksum-crc32c"),
            Some(&"BBB=".to_owned()),
        );
    }

    #[test]
    fn test_should_strip_aws_chunked_encoding_only() {
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::CONTENT_ENCODING,
            "aws-chunked".parse().unwrap(),
        );
        strip_aws_chunked_encoding(&mut headers);
        assert!(headers.get(http::header::CONTENT_ENCODING).is_none());
    }

    #[test]
    fn test_should_strip_aws_chunked_keep_other() {
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::CONTENT_ENCODING,
            "gzip, aws-chunked".parse().unwrap(),
        );
        strip_aws_chunked_encoding(&mut headers);
        assert_eq!(
            headers
                .get(http::header::CONTENT_ENCODING)
                .unwrap()
                .to_str()
                .unwrap(),
            "gzip"
        );
    }

    #[test]
    fn test_should_decode_chunk_without_signature_extension() {
        let body = b"3\r\nabc\r\n0\r\n\r\n";
        let result = decode_aws_chunked(body).expect("should decode");
        assert_eq!(result.body.as_ref(), b"abc");
    }
}
