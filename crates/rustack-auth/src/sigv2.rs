//! AWS Signature Version 2 verification.
//!
//! SigV2 is an older signing mechanism that uses HMAC-SHA1. The `Authorization`
//! header has the format:
//!
//! ```text
//! AWS <AWSAccessKeyId>:<Signature>
//! ```
//!
//! Where `Signature = Base64(HMAC-SHA1(SecretKey, StringToSign))` and:
//!
//! ```text
//! StringToSign = HTTP-Verb + "\n" +
//!                Content-MD5 + "\n" +
//!                Content-Type + "\n" +
//!                Date + "\n" +
//!                CanonicalizedAmzHeaders +
//!                CanonicalizedResource
//! ```

use std::collections::BTreeMap;

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;
use subtle::ConstantTimeEq;
use tracing::debug;

use crate::{credentials::CredentialProvider, error::AuthError, sigv4::AuthResult};

type HmacSha1 = Hmac<Sha1>;

/// Check whether the `Authorization` header uses SigV2 format (`AWS AKID:sig`).
#[must_use]
pub fn is_sigv2(auth_header: &str) -> bool {
    auth_header.starts_with("AWS ") && !auth_header.starts_with("AWS4-")
}

/// Verify an AWS SigV2-signed HTTP request.
///
/// # Errors
///
/// Returns an [`AuthError`] if the header is malformed, the access key is not
/// found, or the signature does not match.
pub fn verify_sigv2(
    parts: &http::request::Parts,
    credential_provider: &dyn CredentialProvider,
) -> Result<AuthResult, AuthError> {
    let auth_header = parts
        .headers
        .get(http::header::AUTHORIZATION)
        .ok_or(AuthError::MissingAuthHeader)?
        .to_str()
        .map_err(|_| AuthError::InvalidAuthHeader)?;

    let (access_key_id, provided_signature) = parse_sigv2_header(auth_header)?;

    debug!(access_key_id = %access_key_id, "Verifying SigV2 signature");

    let secret_key = credential_provider.get_secret_key(&access_key_id)?;

    let string_to_sign = build_string_to_sign(parts);

    debug!(string_to_sign = ?string_to_sign, "Built SigV2 string to sign");

    let expected_signature = compute_sigv2_signature(&secret_key, &string_to_sign);

    if provided_signature
        .as_bytes()
        .ct_eq(expected_signature.as_bytes())
        .into()
    {
        debug!(access_key_id = %access_key_id, "SigV2 verification succeeded");
        Ok(AuthResult {
            access_key_id,
            region: String::new(),
            service: "s3".to_owned(),
            signed_headers: Vec::new(),
        })
    } else {
        debug!(
            expected = %expected_signature,
            provided = %provided_signature,
            "SigV2 signature mismatch"
        );
        Err(AuthError::SignatureDoesNotMatch)
    }
}

/// Parse a SigV2 `Authorization` header: `AWS AKID:Signature`.
fn parse_sigv2_header(header: &str) -> Result<(String, String), AuthError> {
    let rest = header
        .strip_prefix("AWS ")
        .ok_or(AuthError::InvalidAuthHeader)?;

    let (access_key_id, signature) = rest.split_once(':').ok_or(AuthError::InvalidAuthHeader)?;

    if access_key_id.is_empty() || signature.is_empty() {
        return Err(AuthError::InvalidAuthHeader);
    }

    Ok((access_key_id.to_owned(), signature.to_owned()))
}

/// Build the SigV2 string to sign from the request parts.
///
/// ```text
/// HTTP-Verb + "\n" +
/// Content-MD5 + "\n" +
/// Content-Type + "\n" +
/// Date + "\n" +
/// CanonicalizedAmzHeaders +
/// CanonicalizedResource
/// ```
fn build_string_to_sign(parts: &http::request::Parts) -> String {
    let method = parts.method.as_str();
    let content_md5 = header_value(parts, "content-md5");
    let content_type = header_value(parts, "content-type");

    // Use x-amz-date if present, otherwise use Date header.
    let date = if parts.headers.contains_key("x-amz-date") {
        "" // When x-amz-date is present, Date field in StringToSign is empty.
    } else {
        &header_value(parts, "date")
    };

    let amz_headers = build_canonicalized_amz_headers(parts);
    let resource = build_canonicalized_resource(parts);

    format!("{method}\n{content_md5}\n{content_type}\n{date}\n{amz_headers}{resource}")
}

/// Build the CanonicalizedAmzHeaders string.
///
/// All x-amz-* headers are lowercased, sorted, and joined with newlines.
/// Each header is formatted as `name:value\n`.
fn build_canonicalized_amz_headers(parts: &http::request::Parts) -> String {
    let mut amz_headers: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for (name, value) in &parts.headers {
        let name_str = name.as_str();
        if name_str.starts_with("x-amz-") {
            let val = value.to_str().unwrap_or("").trim().to_owned();
            amz_headers
                .entry(name_str.to_owned())
                .or_default()
                .push(val);
        }
    }

    let mut result = String::new();
    for (name, values) in &amz_headers {
        result.push_str(name);
        result.push(':');
        result.push_str(&values.join(","));
        result.push('\n');
    }

    result
}

/// Build the CanonicalizedResource string.
///
/// This is the URI path plus any sub-resource query parameters (sorted).
fn build_canonicalized_resource(parts: &http::request::Parts) -> String {
    // S3 sub-resources that must be included in the canonical resource.
    const SUB_RESOURCES: &[&str] = &[
        "acl",
        "cors",
        "delete",
        "lifecycle",
        "location",
        "logging",
        "notification",
        "partNumber",
        "policy",
        "requestPayment",
        "response-cache-control",
        "response-content-disposition",
        "response-content-encoding",
        "response-content-language",
        "response-content-type",
        "response-expires",
        "restore",
        "tagging",
        "torrent",
        "uploadId",
        "uploads",
        "versionId",
        "versioning",
        "versions",
        "website",
    ];

    let path = parts.uri.path();
    let query = parts.uri.query().unwrap_or("");
    let mut sub_params: Vec<(String, Option<String>)> = Vec::new();

    if !query.is_empty() {
        for param in query.split('&') {
            let (key, value) = param.split_once('=').map_or((param, None), |(k, v)| {
                let decoded = percent_encoding::percent_decode_str(v)
                    .decode_utf8_lossy()
                    .into_owned();
                // Treat empty values the same as absent values for sub-resources.
                let value = if decoded.is_empty() {
                    None
                } else {
                    Some(decoded)
                };
                (k, value)
            });
            if SUB_RESOURCES.contains(&key) {
                sub_params.push((key.to_owned(), value));
            }
        }
    }

    sub_params.sort_by(|a, b| a.0.cmp(&b.0));

    if sub_params.is_empty() {
        path.to_owned()
    } else {
        let params_str: Vec<String> = sub_params
            .iter()
            .map(|(k, v)| match v {
                Some(val) => format!("{k}={val}"),
                None => k.clone(),
            })
            .collect();
        format!("{path}?{}", params_str.join("&"))
    }
}

/// Compute the SigV2 signature: Base64(HMAC-SHA1(secret, string_to_sign)).
fn compute_sigv2_signature(secret_key: &str, string_to_sign: &str) -> String {
    let mut mac =
        HmacSha1::new_from_slice(secret_key.as_bytes()).expect("HMAC can accept any key length");
    mac.update(string_to_sign.as_bytes());
    let result = mac.finalize().into_bytes();
    BASE64.encode(result)
}

/// Extract a header value as a string, returning empty string if missing.
fn header_value(parts: &http::request::Parts, name: &str) -> String {
    parts
        .headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::StaticCredentialProvider;

    const TEST_ACCESS_KEY: &str = "minioadmin";
    const TEST_SECRET_KEY: &str = "minioadmin";

    fn test_credential_provider() -> StaticCredentialProvider {
        StaticCredentialProvider::new(vec![(
            TEST_ACCESS_KEY.to_owned(),
            TEST_SECRET_KEY.to_owned(),
        )])
    }

    #[test]
    fn test_should_detect_sigv2_header() {
        assert!(is_sigv2("AWS AKID:signature"));
        assert!(!is_sigv2("AWS4-HMAC-SHA256 Credential=..."));
        assert!(!is_sigv2("Bearer token"));
    }

    #[test]
    fn test_should_parse_sigv2_header() {
        let (akid, sig) = parse_sigv2_header("AWS mykey:mysignature").unwrap();
        assert_eq!(akid, "mykey");
        assert_eq!(sig, "mysignature");
    }

    #[test]
    fn test_should_reject_invalid_sigv2_header() {
        assert!(parse_sigv2_header("AWS :sig").is_err());
        assert!(parse_sigv2_header("AWS key:").is_err());
        assert!(parse_sigv2_header("AWS noseparator").is_err());
        assert!(parse_sigv2_header("NOTAWS key:sig").is_err());
    }

    #[test]
    fn test_should_compute_sigv2_signature() {
        let sig = compute_sigv2_signature("secret", "data");
        assert!(!sig.is_empty());
        // Base64(HMAC-SHA1) produces a deterministic output.
        let sig2 = compute_sigv2_signature("secret", "data");
        assert_eq!(sig, sig2);
    }

    #[test]
    fn test_should_verify_sigv2_roundtrip() {
        let provider = test_credential_provider();

        // Build a simple GET request.
        let date = "Sat, 28 Feb 2026 12:00:00 GMT";
        let string_to_sign = format!("GET\n\n\n{date}\n/test-bucket/");
        let signature = compute_sigv2_signature(TEST_SECRET_KEY, &string_to_sign);

        let auth_header = format!("AWS {TEST_ACCESS_KEY}:{signature}");

        let (parts, ()) = http::Request::builder()
            .method("GET")
            .uri("http://localhost:4566/test-bucket/")
            .header("host", "localhost:4566")
            .header("date", date)
            .header(http::header::AUTHORIZATION, &auth_header)
            .body(())
            .unwrap()
            .into_parts();

        let result = verify_sigv2(&parts, &provider);
        assert!(result.is_ok(), "verify_sigv2 failed: {result:?}");
        assert_eq!(result.unwrap().access_key_id, TEST_ACCESS_KEY);
    }
}
