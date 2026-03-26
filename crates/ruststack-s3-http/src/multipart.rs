//! Multipart form data parser for S3 POST Object (browser-based upload).
//!
//! Parses `multipart/form-data` bodies into named fields and a single file payload.
//! This is a synchronous parser that works on the already-collected body bytes.

use std::collections::HashMap;

use bytes::Bytes;
use ruststack_s3_model::error::{S3Error, S3ErrorCode};

/// A parsed multipart form-data submission.
#[derive(Debug)]
pub struct MultipartForm {
    /// Non-file form fields (name → value).
    pub fields: HashMap<String, String>,
    /// The file field content (the uploaded object data).
    pub file_data: Bytes,
    /// The Content-Type of the file field, if specified.
    pub file_content_type: Option<String>,
}

/// Extract the boundary string from a `Content-Type: multipart/form-data; boundary=...` header.
///
/// # Errors
///
/// Returns an error if the Content-Type is missing, not multipart/form-data,
/// or the boundary parameter is absent.
pub fn extract_boundary(content_type: &str) -> Result<String, S3Error> {
    if !content_type
        .to_ascii_lowercase()
        .starts_with("multipart/form-data")
    {
        return Err(S3Error::with_message(
            S3ErrorCode::InvalidRequest,
            format!("POST requires Content-Type multipart/form-data, got: {content_type}"),
        ));
    }

    // Find "boundary=" in the Content-Type header.
    for part in content_type.split(';') {
        let trimmed = part.trim();
        if let Some(val) = trimmed.strip_prefix("boundary=") {
            let boundary = val.trim_matches('"').to_owned();
            if boundary.is_empty() {
                return Err(S3Error::with_message(
                    S3ErrorCode::InvalidRequest,
                    "Empty boundary in Content-Type",
                ));
            }
            return Ok(boundary);
        }
    }

    Err(S3Error::with_message(
        S3ErrorCode::InvalidRequest,
        "Missing boundary in Content-Type",
    ))
}

/// Parse a multipart/form-data body into form fields and file data.
///
/// Per the S3 POST Object spec, the `file` field must be the last field in the
/// multipart body. All fields before it are treated as form fields.
///
/// # Errors
///
/// Returns an error if the body cannot be parsed or required fields are missing.
pub fn parse_multipart(body: &[u8], boundary: &str) -> Result<MultipartForm, S3Error> {
    let delimiter = format!("--{boundary}");
    let end_delimiter = format!("--{boundary}--");

    let mut fields: HashMap<String, String> = HashMap::new();
    let mut file_data: Option<Bytes> = None;
    let mut file_content_type: Option<String> = None;

    // Split body by boundary delimiters.
    let parts = split_multipart_parts(body, delimiter.as_bytes(), end_delimiter.as_bytes());

    for part_bytes in parts {
        // Each part has headers separated from body by \r\n\r\n.
        let Some((headers_section, part_body)) = split_headers_body(part_bytes) else {
            continue;
        };

        // Parse Content-Disposition header to get the field name and optional filename.
        let disposition = parse_content_disposition(headers_section);
        let Some(field_name) = disposition.name else {
            continue;
        };

        if field_name == "file" || disposition.filename.is_some() {
            // This is the file field — extract content type and data.
            file_content_type = parse_part_content_type(headers_section);
            file_data = Some(Bytes::copy_from_slice(part_body));
        } else {
            // Regular form field — store as string.
            let value = String::from_utf8_lossy(part_body).into_owned();
            fields.insert(field_name, value);
        }
    }

    let file_data = file_data.ok_or_else(|| {
        S3Error::with_message(
            S3ErrorCode::InvalidRequest,
            "Missing file field in multipart form data",
        )
    })?;

    Ok(MultipartForm {
        fields,
        file_data,
        file_content_type,
    })
}

/// Split the multipart body into individual parts by boundary.
fn split_multipart_parts<'a>(
    body: &'a [u8],
    delimiter: &[u8],
    end_delimiter: &[u8],
) -> Vec<&'a [u8]> {
    let mut parts = Vec::new();
    let mut remaining = body;

    // Skip the preamble (everything before the first delimiter).
    if let Some(pos) = find_bytes(remaining, delimiter) {
        remaining = &remaining[pos + delimiter.len()..];
        // Skip the \r\n after the first delimiter.
        remaining = skip_crlf(remaining);
    } else {
        return parts;
    }

    loop {
        // Check for end delimiter.
        if remaining.starts_with(end_delimiter)
            || remaining
                .strip_prefix(b"\r\n")
                .is_some_and(|r| r.starts_with(end_delimiter))
        {
            break;
        }

        // Find the next delimiter.
        if let Some(pos) = find_bytes(remaining, delimiter) {
            // The part content is everything before the delimiter, minus trailing \r\n.
            let part = strip_trailing_crlf(&remaining[..pos]);
            parts.push(part);
            remaining = &remaining[pos + delimiter.len()..];
            remaining = skip_crlf(remaining);
        } else {
            // No more delimiters — treat rest as the last part.
            let part = strip_trailing_crlf(remaining);
            if !part.is_empty() {
                parts.push(part);
            }
            break;
        }
    }

    parts
}

/// Split a part into headers section and body at the first \r\n\r\n boundary.
fn split_headers_body(part: &[u8]) -> Option<(&[u8], &[u8])> {
    let separator = b"\r\n\r\n";
    find_bytes(part, separator).map(|pos| (&part[..pos], &part[pos + separator.len()..]))
}

/// Parsed Content-Disposition header fields.
struct ContentDisposition {
    name: Option<String>,
    filename: Option<String>,
}

/// Parse a Content-Disposition header from a headers section.
fn parse_content_disposition(headers: &[u8]) -> ContentDisposition {
    let headers_str = String::from_utf8_lossy(headers);
    let mut name = None;
    let mut filename = None;

    for line in headers_str.split("\r\n") {
        let lower = line.to_ascii_lowercase();
        if !lower.starts_with("content-disposition:") {
            continue;
        }

        // Extract name="..." parameter.
        if let Some(n) = extract_quoted_param(line, "name") {
            name = Some(n);
        }
        // Extract filename="..." parameter.
        if let Some(f) = extract_quoted_param(line, "filename") {
            filename = Some(f);
        }
    }

    ContentDisposition { name, filename }
}

/// Extract the Content-Type from a part's headers section.
fn parse_part_content_type(headers: &[u8]) -> Option<String> {
    let headers_str = String::from_utf8_lossy(headers);
    for line in headers_str.split("\r\n") {
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("content-type:") {
            return Some(rest.trim().to_owned());
        }
    }
    None
}

/// Extract a quoted parameter value from a header line.
///
/// Looks for `param_name="value"` and returns the unquoted value.
fn extract_quoted_param(header_line: &str, param_name: &str) -> Option<String> {
    // Build search patterns: `name="` and `name=` (unquoted).
    let quoted_pattern = format!("{param_name}=\"");
    let unquoted_pattern = format!("{param_name}=");

    let lower_line = header_line.to_ascii_lowercase();

    // Try quoted form first.
    if let Some(pos) = lower_line.find(&quoted_pattern) {
        let start = pos + quoted_pattern.len();
        let rest = &header_line[start..];
        if let Some(end) = rest.find('"') {
            return Some(rest[..end].to_owned());
        }
    }

    // Try unquoted form.
    if let Some(pos) = lower_line.find(&unquoted_pattern) {
        let start = pos + unquoted_pattern.len();
        let rest = &header_line[start..];
        // Value ends at ; or end of line.
        let end = rest.find(';').unwrap_or(rest.len());
        let val = rest[..end].trim().to_owned();
        if !val.is_empty() {
            return Some(val);
        }
    }

    None
}

/// Find the position of a needle in a haystack.
fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Skip leading \r\n.
fn skip_crlf(data: &[u8]) -> &[u8] {
    data.strip_prefix(b"\r\n").unwrap_or(data)
}

/// Strip trailing \r\n.
fn strip_trailing_crlf(data: &[u8]) -> &[u8] {
    data.strip_suffix(b"\r\n").unwrap_or(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_extract_boundary() {
        let ct = "multipart/form-data; boundary=----WebKitFormBoundary7MA4YWxkTrZu0gW";
        let b = extract_boundary(ct).expect("should extract boundary");
        assert_eq!(b, "----WebKitFormBoundary7MA4YWxkTrZu0gW");
    }

    #[test]
    fn test_should_extract_quoted_boundary() {
        let ct = r#"multipart/form-data; boundary="abc123""#;
        let b = extract_boundary(ct).expect("should extract boundary");
        assert_eq!(b, "abc123");
    }

    #[test]
    fn test_should_reject_non_multipart() {
        let result = extract_boundary("application/json");
        assert!(result.is_err());
    }

    #[test]
    fn test_should_parse_simple_multipart() {
        let boundary = "----boundary";
        let body = "------boundary\r\nContent-Disposition: form-data; \
                    name=\"key\"\r\n\r\nmy-object-key\r\n------boundary\r\nContent-Disposition: \
                    form-data; name=\"file\"; filename=\"test.txt\"\r\nContent-Type: \
                    text/plain\r\n\r\nhello world\r\n------boundary--\r\n";

        let result = parse_multipart(body.as_bytes(), boundary).expect("should parse");
        assert_eq!(
            result.fields.get("key").map(String::as_str),
            Some("my-object-key")
        );
        assert_eq!(result.file_data.as_ref(), b"hello world");
        assert_eq!(result.file_content_type.as_deref(), Some("text/plain"));
    }

    #[test]
    fn test_should_parse_multipart_with_policy_fields() {
        let boundary = "xyzzy";
        let body =
            "--xyzzy\r\nContent-Disposition: form-data; \
             name=\"key\"\r\n\r\nuploads/test.bin\r\n--xyzzy\r\nContent-Disposition: form-data; \
             name=\"policy\"\r\n\r\neyJjb25kaXRpb25zIjpbXX0=\r\n--xyzzy\r\nContent-Disposition: \
             form-data; \
             name=\"x-amz-algorithm\"\r\n\r\nAWS4-HMAC-SHA256\r\n--xyzzy\r\nContent-Disposition: \
             form-data; name=\"file\"; filename=\"test.bin\"\r\nContent-Type: \
             application/octet-stream\r\n\r\n\x00\x01\x02\x03\r\n--xyzzy--\r\n";

        let result = parse_multipart(body.as_bytes(), boundary).expect("should parse");
        assert_eq!(
            result.fields.get("key").map(String::as_str),
            Some("uploads/test.bin")
        );
        assert_eq!(
            result.fields.get("policy").map(String::as_str),
            Some("eyJjb25kaXRpb25zIjpbXX0=")
        );
        assert_eq!(
            result.fields.get("x-amz-algorithm").map(String::as_str),
            Some("AWS4-HMAC-SHA256")
        );
        assert_eq!(result.file_data.as_ref(), b"\x00\x01\x02\x03");
    }

    #[test]
    fn test_should_reject_missing_file() {
        let boundary = "abc";
        let body =
            "--abc\r\nContent-Disposition: form-data; name=\"key\"\r\n\r\ntest\r\n--abc--\r\n";

        let result = parse_multipart(body.as_bytes(), boundary);
        assert!(result.is_err());
    }
}
