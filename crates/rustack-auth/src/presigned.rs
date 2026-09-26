//! Presigned URL verification for AWS Signature Version 4.
//!
//! Presigned URLs carry authentication information in query parameters rather
//! than HTTP headers. The query parameters include:
//!
//! - `X-Amz-Algorithm` - Must be `AWS4-HMAC-SHA256`
//! - `X-Amz-Credential` - `AKID/date/region/service/aws4_request`
//! - `X-Amz-Date` - ISO 8601 basic format timestamp (`YYYYMMDDTHHMMSSZ`)
//! - `X-Amz-Expires` - Validity duration in seconds
//! - `X-Amz-SignedHeaders` - Semicolon-separated signed header names
//! - `X-Amz-Signature` - The hex-encoded signature
//!
//! For presigned URLs, the payload hash is always `UNSIGNED-PAYLOAD`.

use std::collections::HashMap;

use chrono::{NaiveDateTime, Utc};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tracing::debug;

use crate::{
    canonical::{
        build_canonical_headers, build_canonical_query_string,
        build_canonical_query_string_normalized, build_canonical_uri, build_signed_headers_string,
        service_supports_normalized_fallback,
    },
    credentials::CredentialProvider,
    error::AuthError,
    sigv4::{AuthResult, build_string_to_sign, compute_signature, derive_signing_key},
};

/// The payload hash value used for all presigned URL requests.
const UNSIGNED_PAYLOAD: &str = "UNSIGNED-PAYLOAD";

/// Parsed components from presigned URL query parameters.
#[derive(Debug, Clone)]
pub struct ParsedPresignedParams {
    /// The signing algorithm (must be `AWS4-HMAC-SHA256`).
    pub algorithm: String,
    /// The access key ID.
    pub access_key_id: String,
    /// The date component of the credential scope (YYYYMMDD).
    pub date: String,
    /// The AWS region from the credential scope.
    pub region: String,
    /// The AWS service from the credential scope.
    pub service: String,
    /// The ISO 8601 basic format timestamp.
    pub timestamp: String,
    /// The URL validity duration in seconds.
    pub expires: u64,
    /// The list of signed header names.
    pub signed_headers: Vec<String>,
    /// The hex-encoded signature.
    pub signature: String,
}

/// Parse presigned URL query parameters into their components.
///
/// # Errors
///
/// Returns [`AuthError::MissingQueryParam`] if any required parameter is absent,
/// [`AuthError::UnsupportedAlgorithm`] if the algorithm is not `AWS4-HMAC-SHA256`,
/// or [`AuthError::InvalidCredential`] if the credential format is invalid.
pub fn parse_presigned_params(query: &str) -> Result<ParsedPresignedParams, AuthError> {
    let params: HashMap<String, String> = query
        .split('&')
        .filter(|s| !s.is_empty())
        .filter_map(|param| {
            let (key, value) = param.split_once('=')?;
            Some((key.to_owned(), url_decode(value)))
        })
        .collect();

    let algorithm = get_required_param(&params, "X-Amz-Algorithm")?;
    if algorithm != "AWS4-HMAC-SHA256" {
        return Err(AuthError::UnsupportedAlgorithm(algorithm));
    }

    let credential = get_required_param(&params, "X-Amz-Credential")?;
    let timestamp = get_required_param(&params, "X-Amz-Date")?;
    let expires_str = get_required_param(&params, "X-Amz-Expires")?;
    let signed_headers_str = get_required_param(&params, "X-Amz-SignedHeaders")?;
    let signature = get_required_param(&params, "X-Amz-Signature")?;

    // Parse credential: AKID/date/region/service/aws4_request
    let cred_parts: Vec<&str> = credential.splitn(5, '/').collect();
    if cred_parts.len() != 5 || cred_parts[4] != "aws4_request" {
        return Err(AuthError::InvalidCredential);
    }

    let expires: u64 = expires_str
        .parse()
        .map_err(|_| AuthError::MissingQueryParam("X-Amz-Expires (invalid integer)".to_owned()))?;

    let signed_headers: Vec<String> = signed_headers_str
        .split(';')
        .map(ToOwned::to_owned)
        .collect();

    Ok(ParsedPresignedParams {
        algorithm,
        access_key_id: cred_parts[0].to_owned(),
        date: cred_parts[1].to_owned(),
        region: cred_parts[2].to_owned(),
        service: cred_parts[3].to_owned(),
        timestamp,
        expires,
        signed_headers,
        signature,
    })
}

/// Verify a presigned URL request.
///
/// This function:
/// 1. Parses the presigned URL query parameters
/// 2. Checks whether the URL has expired
/// 3. Resolves the secret key via the credential provider
/// 4. Reconstructs the canonical request (excluding `X-Amz-Signature` from query)
/// 5. Computes the expected signature
/// 6. Compares signatures using constant-time comparison
///
/// # Errors
///
/// Returns an [`AuthError`] if:
/// - Required query parameters are missing or malformed
/// - The URL has expired
/// - The access key is not found
/// - Required signed headers are missing
/// - The signature does not match
pub fn verify_presigned(
    parts: &http::request::Parts,
    credential_provider: &dyn CredentialProvider,
) -> Result<AuthResult, AuthError> {
    let query = parts.uri.query().unwrap_or("");
    let parsed = parse_presigned_params(query)?;

    debug!(
        access_key_id = %parsed.access_key_id,
        date = %parsed.date,
        region = %parsed.region,
        service = %parsed.service,
        expires = parsed.expires,
        "Verifying presigned URL"
    );

    // Check expiration.
    check_expiration(&parsed.timestamp, parsed.expires)?;

    // Resolve the secret key.
    let secret_key = credential_provider.get_secret_key(&parsed.access_key_id)?;

    // Build the canonical request.
    let method = parts.method.as_str();
    let uri = parts.uri.path();
    let canonical_uri = build_canonical_uri(uri);

    // Collect signed headers.
    let signed_header_refs: Vec<&str> = parsed.signed_headers.iter().map(String::as_str).collect();
    let header_pairs: Vec<(&str, &str)> =
        collect_signed_headers_for_presigned(parts, &signed_header_refs)?;

    let canonical_headers = build_canonical_headers(&header_pairs, &signed_header_refs);
    let signed_headers_str = build_signed_headers_string(&signed_header_refs);

    // Build the credential scope and derive the signing key once; both
    // canonicalization attempts below share them.
    let credential_scope = format!(
        "{}/{}/{}/aws4_request",
        parsed.date, parsed.region, parsed.service
    );
    let signing_key =
        derive_signing_key(&secret_key, &parsed.date, &parsed.region, &parsed.service);
    let provided_bytes = parsed.signature.as_bytes();

    // Check the provided signature against the canonical request built from
    // `canonical_query`. Returns true on a constant-time match.
    // For presigned URLs, the payload hash is always UNSIGNED-PAYLOAD.
    #[rustfmt::skip]
    let signature_matches = |canonical_query: &str| -> bool {
        let canonical_request = format!(
            "{method}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n\n{signed_headers_str}\n{UNSIGNED_PAYLOAD}"
        );
        let canonical_hash = hex::encode(Sha256::digest(canonical_request.as_bytes()));
        let string_to_sign =
            build_string_to_sign(&parsed.timestamp, &credential_scope, &canonical_hash);
        let expected_signature = compute_signature(&signing_key, &string_to_sign);
        bool::from(provided_bytes.ct_eq(expected_signature.as_bytes()))
    };

    debug!("Built presigned canonical request");

    // First attempt: raw query string values preserved as-is (excluding
    // `X-Amz-Signature`). This matches clients that sign whatever encoding
    // appears in the URL.
    let canonical_query = build_canonical_query_string_without_signature(query);
    if signature_matches(&canonical_query) {
        debug!(access_key_id = %parsed.access_key_id, "Presigned URL verification succeeded");
        return Ok(AuthResult {
            access_key_id: parsed.access_key_id,
            region: parsed.region,
            service: parsed.service,
            signed_headers: parsed.signed_headers,
        });
    }

    // Second attempt: SigV4-normalized query string (percent-decode, then
    // re-encode per spec, excluding `X-Amz-Signature`). This matches
    // spec-compliant clients that sign the encoded form.
    // The fallback is skipped when the wire query contains a raw `+`: it is
    // the one byte whose meaning downstream decoders disagree on (S3 decodes
    // a literal plus, API Gateway / Lambda decode a space), so no single
    // normalized form is safe for every service. Reject the ambiguous
    // representation instead of guessing.
    // The fallback is additionally restricted to services whose downstream
    // query decoding has been audited (see `service_supports_normalized_fallback`
    // in `canonical.rs`): the normalized form asserts that alternate wire
    // encodings are equivalent, which only holds when the service
    // percent-decodes query parameters before acting on them. Unlisted
    // services fail closed (raw-only verification).
    // If the query is not valid UTF-8 after percent-decoding, normalization
    // is impossible; the raw attempt above already failed, so fall through to
    // the signature mismatch below.
    let normalized_matches = !query.contains('+')
        && service_supports_normalized_fallback(&parsed.service)
        && build_canonical_query_string_without_signature_normalized(query).is_ok_and(
            |normalized| normalized != canonical_query && signature_matches(&normalized),
        );
    if normalized_matches {
        debug!(access_key_id = %parsed.access_key_id, "Presigned URL verification succeeded (normalized query string)");
        return Ok(AuthResult {
            access_key_id: parsed.access_key_id,
            region: parsed.region,
            service: parsed.service,
            signed_headers: parsed.signed_headers,
        });
    }

    debug!("Presigned URL signature mismatch");
    Err(AuthError::SignatureDoesNotMatch)
}

/// Build the canonical query string excluding the `X-Amz-Signature` parameter.
///
/// The remaining parameters are sorted; raw values are preserved as-is.
/// See [`build_canonical_query_string`].
fn build_canonical_query_string_without_signature(query: &str) -> String {
    build_canonical_query_string(&filter_signature_param(query))
}

/// Build the SigV4-normalized canonical query string excluding `X-Amz-Signature`.
///
/// The remaining parameters are percent-decoded, re-encoded per the SigV4 spec,
/// and sorted. See [`build_canonical_query_string_normalized`].
///
/// # Errors
///
/// Returns [`AuthError::InvalidQueryString`] if a parameter is not valid UTF-8
/// after percent-decoding.
fn build_canonical_query_string_without_signature_normalized(
    query: &str,
) -> Result<String, AuthError> {
    build_canonical_query_string_normalized(&filter_signature_param(query))
}

/// Remove the `X-Amz-Signature` parameter from a query string.
fn filter_signature_param(query: &str) -> String {
    query
        .split('&')
        .filter(|param| !param.starts_with("X-Amz-Signature="))
        .collect::<Vec<_>>()
        .join("&")
}

/// Check whether the presigned URL has expired.
fn check_expiration(timestamp: &str, expires: u64) -> Result<(), AuthError> {
    let request_time = NaiveDateTime::parse_from_str(timestamp, "%Y%m%dT%H%M%SZ")
        .map_err(|_| AuthError::MissingQueryParam("X-Amz-Date (invalid format)".to_owned()))?;

    let expiry_time = request_time
        + chrono::Duration::seconds(i64::try_from(expires).map_err(|_| AuthError::RequestExpired)?);

    let now = Utc::now().naive_utc();
    if now > expiry_time {
        return Err(AuthError::RequestExpired);
    }

    Ok(())
}

/// Collect header values for the signed headers from the request.
fn collect_signed_headers_for_presigned<'a>(
    parts: &'a http::request::Parts,
    signed_headers: &[&'a str],
) -> Result<Vec<(&'a str, &'a str)>, AuthError> {
    let mut result = Vec::with_capacity(signed_headers.len());

    for &name in signed_headers {
        let value = parts
            .headers
            .get(name)
            .ok_or_else(|| AuthError::MissingHeader(name.to_owned()))?
            .to_str()
            .map_err(|_| AuthError::MissingHeader(name.to_owned()))?;
        result.push((name, value));
    }

    Ok(result)
}

/// Perform basic percent-decoding of a URL-encoded string.
fn url_decode(input: &str) -> String {
    percent_encoding::percent_decode_str(input)
        .decode_utf8_lossy()
        .into_owned()
}

/// Extract a required query parameter, returning an error if missing.
fn get_required_param(params: &HashMap<String, String>, name: &str) -> Result<String, AuthError> {
    params
        .get(name)
        .cloned()
        .ok_or_else(|| AuthError::MissingQueryParam(name.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::StaticCredentialProvider;

    const TEST_ACCESS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";
    const TEST_SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";

    fn test_credential_provider() -> StaticCredentialProvider {
        StaticCredentialProvider::new(vec![(
            TEST_ACCESS_KEY.to_owned(),
            TEST_SECRET_KEY.to_owned(),
        )])
    }

    #[test]
    fn test_should_parse_presigned_params() {
        #[rustfmt::skip]
        let query = "X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20130524%2Fus-east-1%2Fs3%2Faws4_request&X-Amz-Date=20130524T000000Z&X-Amz-Expires=86400&X-Amz-SignedHeaders=host&X-Amz-Signature=aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404";

        let parsed = parse_presigned_params(query).unwrap();
        assert_eq!(parsed.algorithm, "AWS4-HMAC-SHA256");
        assert_eq!(parsed.access_key_id, "AKIAIOSFODNN7EXAMPLE");
        assert_eq!(parsed.date, "20130524");
        assert_eq!(parsed.region, "us-east-1");
        assert_eq!(parsed.service, "s3");
        assert_eq!(parsed.timestamp, "20130524T000000Z");
        assert_eq!(parsed.expires, 86400);
        assert_eq!(parsed.signed_headers, vec!["host"]);
        assert_eq!(
            parsed.signature,
            "aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404"
        );
    }

    #[test]
    fn test_should_reject_missing_algorithm_param() {
        let query = "X-Amz-Credential=AKID%2F20130524%2Fus-east-1%2Fs3%2Faws4_request&\
                     X-Amz-Date=20130524T000000Z&X-Amz-Expires=86400&X-Amz-SignedHeaders=host&\
                     X-Amz-Signature=abc";

        let result = parse_presigned_params(query);
        assert!(matches!(result, Err(AuthError::MissingQueryParam(_))));
    }

    #[test]
    fn test_should_reject_expired_presigned_url() {
        // A timestamp far in the past with a short expiry.
        let result = check_expiration("20130524T000000Z", 86400);
        assert!(matches!(result, Err(AuthError::RequestExpired)));
    }

    #[test]
    fn test_should_accept_non_expired_presigned_url() {
        // A timestamp in the future (or now) with a large expiry.
        let now = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        let result = check_expiration(&now, 86400);
        assert!(result.is_ok());
    }

    #[test]
    fn test_should_build_query_string_without_signature() {
        let query = "X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=AKID%2F20130524%\
                     2Fus-east-1%2Fs3%2Faws4_request&X-Amz-Date=20130524T000000Z&\
                     X-Amz-Expires=86400&X-Amz-SignedHeaders=host&X-Amz-Signature=abc123";

        let result = build_canonical_query_string_without_signature(query);
        assert!(!result.contains("X-Amz-Signature"));
        assert!(result.contains("X-Amz-Algorithm"));
        assert!(result.contains("X-Amz-Expires"));
    }

    #[test]
    fn test_should_verify_presigned_url_matching_aws_example() {
        // Use the AWS test vector for presigned URL verification.
        // The AWS example uses date 20130524T000000Z which is in the past,
        // so we must test the signature computation separately from expiration.

        // Verify the signature computation is correct by manually
        // replicating the presigned URL flow.
        let signing_key = derive_signing_key(TEST_SECRET_KEY, "20130524", "us-east-1", "s3");

        // Build canonical request for the presigned URL test vector.
        let canonical_request = "GET\n/test.txt\nX-Amz-Algorithm=AWS4-HMAC-SHA256&\
                                 X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20130524%2Fus-east-1%\
                                 2Fs3%2Faws4_request&X-Amz-Date=20130524T000000Z&\
                                 X-Amz-Expires=86400&X-Amz-SignedHeaders=host\nhost:examplebucket.\
                                 s3.amazonaws.com\n\nhost\nUNSIGNED-PAYLOAD";

        let canonical_hash = hex::encode(Sha256::digest(canonical_request.as_bytes()));
        assert_eq!(
            canonical_hash,
            "3bfa292879f6447bbcda7001decf97f4a54dc650c8942174ae0a9121cf58ad04"
        );

        let string_to_sign = build_string_to_sign(
            "20130524T000000Z",
            "20130524/us-east-1/s3/aws4_request",
            &canonical_hash,
        );

        let signature = compute_signature(&signing_key, &string_to_sign);
        assert_eq!(
            signature,
            "aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404"
        );
    }

    #[test]
    fn test_should_verify_presigned_url_with_live_timestamp() {
        // Test full presigned URL verification with a non-expired timestamp.
        let provider = test_credential_provider();
        let now = Utc::now();
        let timestamp = now.format("%Y%m%dT%H%M%SZ").to_string();
        let date = now.format("%Y%m%d").to_string();

        let credential = format!("{TEST_ACCESS_KEY}/{date}/us-east-1/s3/aws4_request");

        // Build the canonical request components.
        let canonical_uri = "/test.txt";
        let query_without_sig = format!(
            "X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential={}&X-Amz-Date={timestamp}&\
             X-Amz-Expires=86400&X-Amz-SignedHeaders=host",
            percent_encoding::utf8_percent_encode(&credential, percent_encoding::NON_ALPHANUMERIC)
        );

        let canonical_query = build_canonical_query_string(&query_without_sig);

        #[rustfmt::skip]
        let canonical_request = format!(
            "GET\n{canonical_uri}\n{canonical_query}\nhost:examplebucket.s3.amazonaws.com\n\nhost\nUNSIGNED-PAYLOAD"
        );

        let canonical_hash = hex::encode(Sha256::digest(canonical_request.as_bytes()));
        let credential_scope = format!("{date}/us-east-1/s3/aws4_request");
        let string_to_sign = build_string_to_sign(&timestamp, &credential_scope, &canonical_hash);

        let signing_key = derive_signing_key(TEST_SECRET_KEY, &date, "us-east-1", "s3");
        let signature = compute_signature(&signing_key, &string_to_sign);

        // Build the full query with signature.
        let full_query = format!("{query_without_sig}&X-Amz-Signature={signature}");
        let uri = format!("http://examplebucket.s3.amazonaws.com/test.txt?{full_query}");

        let (parts, _body) = http::Request::builder()
            .method("GET")
            .uri(&uri)
            .header("host", "examplebucket.s3.amazonaws.com")
            .body(())
            .unwrap()
            .into_parts();

        let result = verify_presigned(&parts, &provider);
        assert!(result.is_ok());

        let auth_result = result.unwrap();
        assert_eq!(auth_result.access_key_id, TEST_ACCESS_KEY);
        assert_eq!(auth_result.region, "us-east-1");
        assert_eq!(auth_result.service, "s3");
    }

    #[test]
    fn test_should_verify_presigned_url_signed_with_normalized_query() {
        // A spec-compliant client signs the normalized canonical query string
        // even when the URL carries raw values (e.g. `prefix=periods/`).
        let provider = test_credential_provider();
        let now = Utc::now();
        let timestamp = now.format("%Y%m%dT%H%M%SZ").to_string();
        let date = now.format("%Y%m%d").to_string();

        let credential = format!("{TEST_ACCESS_KEY}/{date}/us-east-1/s3/aws4_request");
        let encoded_credential =
            percent_encoding::utf8_percent_encode(&credential, percent_encoding::NON_ALPHANUMERIC);

        // Raw value on the wire, as a client like Transmit 5 would send it.
        let query_without_sig = format!(
            "X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential={encoded_credential}&\
             X-Amz-Date={timestamp}&X-Amz-Expires=86400&X-Amz-SignedHeaders=host&prefix=periods/"
        );

        // Sign the normalized canonical query string.
        let canonical_query =
            build_canonical_query_string_without_signature_normalized(&query_without_sig).unwrap();
        assert!(canonical_query.contains("prefix=periods%2F"));

        #[rustfmt::skip]
        let canonical_request = format!(
            "GET\n/test.txt\n{canonical_query}\nhost:examplebucket.s3.amazonaws.com\n\nhost\nUNSIGNED-PAYLOAD"
        );

        let canonical_hash = hex::encode(Sha256::digest(canonical_request.as_bytes()));
        let credential_scope = format!("{date}/us-east-1/s3/aws4_request");
        let string_to_sign = build_string_to_sign(&timestamp, &credential_scope, &canonical_hash);

        let signing_key = derive_signing_key(TEST_SECRET_KEY, &date, "us-east-1", "s3");
        let signature = compute_signature(&signing_key, &string_to_sign);

        let full_query = format!("{query_without_sig}&X-Amz-Signature={signature}");
        let uri = format!("http://examplebucket.s3.amazonaws.com/test.txt?{full_query}");

        let (parts, _body) = http::Request::builder()
            .method("GET")
            .uri(&uri)
            .header("host", "examplebucket.s3.amazonaws.com")
            .body(())
            .unwrap()
            .into_parts();

        let result = verify_presigned(&parts, &provider);
        assert!(result.is_ok());
    }

    #[test]
    fn test_should_reject_plus_sign_tampering_in_presigned_url() {
        // Same scenario as the header-auth tampering test, through the
        // presigned-URL path: signed `x=%2B`, tampered to `x=+` on the wire.
        let provider = test_credential_provider();
        let now = Utc::now();
        let timestamp = now.format("%Y%m%dT%H%M%SZ").to_string();
        let date = now.format("%Y%m%d").to_string();

        let credential = format!("{TEST_ACCESS_KEY}/{date}/us-east-1/s3/aws4_request");
        let encoded_credential =
            percent_encoding::utf8_percent_encode(&credential, percent_encoding::NON_ALPHANUMERIC);

        let query_without_sig = format!(
            "X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential={encoded_credential}&\
             X-Amz-Date={timestamp}&X-Amz-Expires=86400&X-Amz-SignedHeaders=host&x=%2B"
        );

        // Sign the normalized canonical query string.
        let canonical_query =
            build_canonical_query_string_without_signature_normalized(&query_without_sig).unwrap();
        assert!(canonical_query.contains("x=%2B"));

        #[rustfmt::skip]
        let canonical_request = format!(
            "GET\n/test.txt\n{canonical_query}\nhost:examplebucket.s3.amazonaws.com\n\nhost\nUNSIGNED-PAYLOAD"
        );

        let canonical_hash = hex::encode(Sha256::digest(canonical_request.as_bytes()));
        let credential_scope = format!("{date}/us-east-1/s3/aws4_request");
        let string_to_sign = build_string_to_sign(&timestamp, &credential_scope, &canonical_hash);

        let signing_key = derive_signing_key(TEST_SECRET_KEY, &date, "us-east-1", "s3");
        let signature = compute_signature(&signing_key, &string_to_sign);

        // Tamper: `%2B` becomes a raw `+` on the wire.
        let tampered_query = query_without_sig.replace("x=%2B", "x=+");
        let full_query = format!("{tampered_query}&X-Amz-Signature={signature}");
        let uri = format!("http://examplebucket.s3.amazonaws.com/test.txt?{full_query}");

        let (parts, _body) = http::Request::builder()
            .method("GET")
            .uri(&uri)
            .header("host", "examplebucket.s3.amazonaws.com")
            .body(())
            .unwrap()
            .into_parts();

        let result = verify_presigned(&parts, &provider);
        assert!(matches!(result, Err(AuthError::SignatureDoesNotMatch)));
    }

    #[test]
    fn test_should_reject_inverse_plus_sign_tampering_in_presigned_url() {
        // The inverse collision through the presigned-URL path: signed
        // `x=%20` (a space), tampered to `x=+` on the wire. S3's downstream
        // decoder keeps the literal plus, so the original signature must not
        // verify the tampered URL.
        let provider = test_credential_provider();
        let now = Utc::now();
        let timestamp = now.format("%Y%m%dT%H%M%SZ").to_string();
        let date = now.format("%Y%m%d").to_string();

        let credential = format!("{TEST_ACCESS_KEY}/{date}/us-east-1/s3/aws4_request");
        let encoded_credential =
            percent_encoding::utf8_percent_encode(&credential, percent_encoding::NON_ALPHANUMERIC);

        let query_without_sig = format!(
            "X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential={encoded_credential}&\
             X-Amz-Date={timestamp}&X-Amz-Expires=86400&X-Amz-SignedHeaders=host&x=%20"
        );

        // Sign the normalized canonical query string.
        let canonical_query =
            build_canonical_query_string_without_signature_normalized(&query_without_sig).unwrap();
        assert!(canonical_query.contains("x=%20"));

        #[rustfmt::skip]
        let canonical_request = format!(
            "GET\n/test.txt\n{canonical_query}\nhost:examplebucket.s3.amazonaws.com\n\nhost\nUNSIGNED-PAYLOAD"
        );

        let canonical_hash = hex::encode(Sha256::digest(canonical_request.as_bytes()));
        let credential_scope = format!("{date}/us-east-1/s3/aws4_request");
        let string_to_sign = build_string_to_sign(&timestamp, &credential_scope, &canonical_hash);

        let signing_key = derive_signing_key(TEST_SECRET_KEY, &date, "us-east-1", "s3");
        let signature = compute_signature(&signing_key, &string_to_sign);

        // Tamper: `%20` becomes a raw `+` on the wire.
        let tampered_query = query_without_sig.replace("x=%20", "x=+");
        let full_query = format!("{tampered_query}&X-Amz-Signature={signature}");
        let uri = format!("http://examplebucket.s3.amazonaws.com/test.txt?{full_query}");

        let (parts, _body) = http::Request::builder()
            .method("GET")
            .uri(&uri)
            .header("host", "examplebucket.s3.amazonaws.com")
            .body(())
            .unwrap()
            .into_parts();

        let result = verify_presigned(&parts, &provider);
        assert!(matches!(result, Err(AuthError::SignatureDoesNotMatch)));
    }
}
