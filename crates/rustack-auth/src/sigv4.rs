//! AWS Signature Version 4 verification.
//!
//! This module implements the core SigV4 signature verification flow:
//!
//! 1. Parse the `Authorization` header to extract the algorithm, credential scope, signed headers,
//!    and provided signature.
//! 2. Reconstruct the canonical request from the HTTP request parts.
//! 3. Build the string to sign from the timestamp, credential scope, and canonical request hash.
//! 4. Derive the signing key using HMAC-SHA256 from the secret key and credential scope components.
//! 5. Compute the expected signature and compare it to the provided signature using constant-time
//!    comparison.
//!
//! The main entry point is [`verify_sigv4`].

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tracing::debug;

use crate::{
    canonical::{
        build_canonical_query_string, build_canonical_query_string_normalized,
        build_canonical_request,
    },
    credentials::CredentialProvider,
    error::AuthError,
};

/// The only algorithm supported by this implementation.
const SUPPORTED_ALGORITHM: &str = "AWS4-HMAC-SHA256";

type HmacSha256 = Hmac<Sha256>;

/// The result of a successful SigV4 verification.
#[derive(Debug, Clone)]
pub struct AuthResult {
    /// The access key ID that signed the request.
    pub access_key_id: String,
    /// The AWS region from the credential scope.
    pub region: String,
    /// The AWS service from the credential scope.
    pub service: String,
    /// The list of headers that were included in the signature.
    pub signed_headers: Vec<String>,
}

/// Parsed components of an AWS SigV4 `Authorization` header.
///
/// Format:
/// ```text
/// AWS4-HMAC-SHA256 Credential=AKID/20130524/us-east-1/s3/aws4_request,
///   SignedHeaders=host;x-amz-content-sha256;x-amz-date,
///   Signature=<hex-signature>
/// ```
#[derive(Debug, Clone)]
pub struct ParsedAuth {
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
    /// The list of signed header names (lowercase).
    pub signed_headers: Vec<String>,
    /// The hex-encoded signature.
    pub signature: String,
}

/// Parse an AWS SigV4 `Authorization` header value into its components.
///
/// # Errors
///
/// Returns [`AuthError::InvalidAuthHeader`] if the header format is invalid,
/// or [`AuthError::UnsupportedAlgorithm`] if the algorithm is not `AWS4-HMAC-SHA256`.
pub fn parse_authorization_header(header: &str) -> Result<ParsedAuth, AuthError> {
    // Split algorithm from the rest: "AWS4-HMAC-SHA256
    // Credential=...,SignedHeaders=...,Signature=..."
    let (algorithm, rest) = header.split_once(' ').ok_or(AuthError::InvalidAuthHeader)?;

    if algorithm != SUPPORTED_ALGORITHM {
        return Err(AuthError::UnsupportedAlgorithm(algorithm.to_owned()));
    }

    // Parse the key=value pairs separated by ", " or ","
    let mut credential = None;
    let mut signed_headers = None;
    let mut signature = None;

    for part in rest.split(',') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix("Credential=") {
            credential = Some(value);
        } else if let Some(value) = part.strip_prefix("SignedHeaders=") {
            signed_headers = Some(value);
        } else if let Some(value) = part.strip_prefix("Signature=") {
            signature = Some(value);
        }
    }

    let credential = credential.ok_or(AuthError::InvalidAuthHeader)?;
    let signed_headers = signed_headers.ok_or(AuthError::InvalidAuthHeader)?;
    let signature = signature.ok_or(AuthError::InvalidAuthHeader)?;

    // Parse credential: AKID/date/region/service/aws4_request
    let cred_parts: Vec<&str> = credential.splitn(5, '/').collect();
    if cred_parts.len() != 5 || cred_parts[4] != "aws4_request" {
        return Err(AuthError::InvalidCredential);
    }

    let parsed_signed_headers: Vec<String> =
        signed_headers.split(';').map(ToOwned::to_owned).collect();

    Ok(ParsedAuth {
        algorithm: algorithm.to_owned(),
        access_key_id: cred_parts[0].to_owned(),
        date: cred_parts[1].to_owned(),
        region: cred_parts[2].to_owned(),
        service: cred_parts[3].to_owned(),
        signed_headers: parsed_signed_headers,
        signature: signature.to_owned(),
    })
}

/// Build the SigV4 string to sign.
///
/// Format:
/// ```text
/// AWS4-HMAC-SHA256\n
/// <ISO8601 timestamp>\n
/// <credential_scope>\n
/// <hex(SHA256(canonical_request))>
/// ```
///
/// # Examples
///
/// ```
/// use rustack_auth::sigv4::build_string_to_sign;
///
/// let sts = build_string_to_sign(
///     "20130524T000000Z",
///     "20130524/us-east-1/s3/aws4_request",
///     "7344ae5b7ee6c3e7e6b0fe0640412a37625d1fbfff95c48bbb2dc43964946972",
/// );
/// assert!(sts.starts_with("AWS4-HMAC-SHA256\n20130524T000000Z\n"));
/// ```
#[must_use]
pub fn build_string_to_sign(
    timestamp: &str,
    credential_scope: &str,
    canonical_request_hash: &str,
) -> String {
    format!("{SUPPORTED_ALGORITHM}\n{timestamp}\n{credential_scope}\n{canonical_request_hash}")
}

/// Derive the SigV4 signing key using HMAC-SHA256 chain.
///
/// ```text
/// DateKey              = HMAC-SHA256("AWS4" + secret_key, date)
/// DateRegionKey        = HMAC-SHA256(DateKey, region)
/// DateRegionServiceKey = HMAC-SHA256(DateRegionKey, service)
/// SigningKey           = HMAC-SHA256(DateRegionServiceKey, "aws4_request")
/// ```
///
/// # Examples
///
/// ```
/// use rustack_auth::sigv4::derive_signing_key;
///
/// let key = derive_signing_key(
///     "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
///     "20130524",
///     "us-east-1",
///     "s3",
/// );
/// assert!(!key.is_empty());
/// ```
#[must_use]
pub fn derive_signing_key(secret_key: &str, date: &str, region: &str, service: &str) -> Vec<u8> {
    let date_key = hmac_sha256(format!("AWS4{secret_key}").as_bytes(), date.as_bytes());
    let date_region_key = hmac_sha256(&date_key, region.as_bytes());
    let date_region_service_key = hmac_sha256(&date_region_key, service.as_bytes());
    hmac_sha256(&date_region_service_key, b"aws4_request")
}

/// Compute the HMAC-SHA256 signature of `data` using the given `signing_key`.
///
/// Returns the hex-encoded signature.
#[must_use]
pub fn compute_signature(signing_key: &[u8], data: &str) -> String {
    let sig = hmac_sha256(signing_key, data.as_bytes());
    hex::encode(sig)
}

/// Verify an AWS SigV4-signed HTTP request.
///
/// This function:
/// 1. Parses the `Authorization` header
/// 2. Resolves the secret key via the credential provider
/// 3. Reconstructs the canonical request
/// 4. Computes the expected signature
/// 5. Compares signatures using constant-time comparison
///
/// The query string is verified exactly as it appears on the wire
/// (raw-preserving). Unlike [`verify_s3_sigv4`], this does NOT attempt the
/// SigV4-normalized fallback: the normalized form asserts that alternate
/// wire encodings are equivalent, which is only true when the serving
/// endpoint percent-decodes query parameters before acting on them. Whether
/// the fallback is safe is a property of the serving endpoint, never of the
/// request — the credential scope's service string is request-supplied and
/// must not select it. Endpoints whose query decoding has been audited opt
/// in by calling [`verify_s3_sigv4`] instead.
///
/// # Errors
///
/// Returns an [`AuthError`] if:
/// - The `Authorization` header is missing or malformed
/// - The access key is not found
/// - Required signed headers are missing
/// - The signature does not match
pub fn verify_sigv4(
    parts: &http::request::Parts,
    body_hash: &str,
    credential_provider: &dyn CredentialProvider,
) -> Result<AuthResult, AuthError> {
    verify_sigv4_with_policy(parts, body_hash, credential_provider, false, false)
}

/// Verify S3 SigV4, allowing the explicit unsigned-payload protocol exception.
///
/// This verifies the seed signature only for streaming markers. Callers MUST additionally
/// verify every chunk and signed trailer with [`StreamingVerifier`] before publishing data.
///
/// In addition to the raw-preserving attempt, this accepts the
/// SigV4-normalized canonical query string as a fallback (see
/// [`build_canonical_query_string_normalized`]): spec-compliant S3 clients
/// such as Transmit 5 sign the encoded form even when the wire carries the
/// raw value. This endpoint entry point is the opt-in — S3 percent-decodes
/// every query parameter before acting on it
/// (`rustack-s3-http/src/router.rs::parse_query_params`), so the normalized
/// equivalence holds. Do NOT use this entry point for an endpoint that reads
/// query parameters from the raw query string without decoding (e.g.
/// CloudFront's `DeleteRealtimeLogConfig` reading `Name`): a verified
/// signature could then authorize a different effective value than the one
/// acted upon. Such endpoints must use [`verify_sigv4`].
///
/// # Errors
/// Returns an authentication error for invalid signatures or payload declarations.
pub fn verify_s3_sigv4(
    parts: &http::request::Parts,
    body_hash: &str,
    credential_provider: &dyn CredentialProvider,
) -> Result<AuthResult, AuthError> {
    verify_sigv4_with_policy(parts, body_hash, credential_provider, true, true)
}

fn verify_sigv4_with_policy(
    parts: &http::request::Parts,
    body_hash: &str,
    credential_provider: &dyn CredentialProvider,
    allow_unsigned: bool,
    allow_normalized_fallback: bool,
) -> Result<AuthResult, AuthError> {
    let payload_hash = validated_payload_hash(parts, body_hash, allow_unsigned)?;
    if parts
        .headers
        .get_all(http::header::AUTHORIZATION)
        .iter()
        .count()
        > 1
    {
        return Err(AuthError::InvalidAuthHeader);
    }
    // Extract and parse the Authorization header.
    let auth_header = parts
        .headers
        .get(http::header::AUTHORIZATION)
        .ok_or(AuthError::MissingAuthHeader)?
        .to_str()
        .map_err(|_| AuthError::InvalidAuthHeader)?;

    debug!("Parsing SigV4 authorization header");

    let parsed = parse_authorization_header(auth_header)?;

    // Resolve the secret key.
    let secret_key = credential_provider.get_secret_key(&parsed.access_key_id)?;

    // Extract the timestamp from x-amz-date header.
    let timestamp = extract_header_value(parts, "x-amz-date")?;

    debug!(
        access_key_id = %parsed.access_key_id,
        date = %parsed.date,
        region = %parsed.region,
        service = %parsed.service,
        "Verifying SigV4 signature"
    );

    // Build the canonical request.
    let method = parts.method.as_str();
    let uri = parts.uri.path();
    let query = parts.uri.query().unwrap_or("");

    // Collect headers that are in the signed headers list.
    let signed_header_refs: Vec<&str> = parsed.signed_headers.iter().map(String::as_str).collect();
    let header_pairs: Vec<(&str, &str)> = collect_signed_headers(parts, &signed_header_refs)?;

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
    // `query_string`. Returns true on a constant-time match.
    let signature_matches = |query_string: &str| -> bool {
        let canonical_request = build_canonical_request(
            method,
            uri,
            query_string,
            &header_pairs,
            &signed_header_refs,
            payload_hash,
        );
        let canonical_hash = hex::encode(Sha256::digest(canonical_request.as_bytes()));
        let string_to_sign = build_string_to_sign(&timestamp, &credential_scope, &canonical_hash);
        let expected_signature = compute_signature(&signing_key, &string_to_sign);
        bool::from(provided_bytes.ct_eq(expected_signature.as_bytes()))
    };

    // First attempt: raw query string values preserved as-is. This matches
    // clients that sign whatever encoding appears on the wire (e.g. AWS SDKs
    // with pre-encoded values, minio-java via OkHttp with raw values).
    if signature_matches(query) {
        debug!(access_key_id = %parsed.access_key_id, "Signature verification succeeded");
        return Ok(AuthResult {
            access_key_id: parsed.access_key_id,
            region: parsed.region,
            service: parsed.service,
            signed_headers: parsed.signed_headers,
        });
    }

    // Second attempt: SigV4-normalized query string (percent-decode, then
    // re-encode per spec). This matches spec-compliant clients such as
    // Transmit 5, which sign the encoded form even when the wire carries the
    // raw value (e.g. `prefix=periods/` on the wire, `prefix=periods%2F` signed).
    // Passing the normalized string through `build_canonical_request` is safe:
    // it is already sorted and contains no raw `&`/`=`, so the raw sorting pass
    // is a no-op.
    // This attempt only runs when the caller opted in via
    // `allow_normalized_fallback` — i.e. the serving endpoint, not the
    // request. The normalized form asserts that alternate wire encodings are
    // equivalent, which is only true when the endpoint percent-decodes query
    // parameters before acting on them. The credential scope's service string
    // is request-supplied and must never select this: otherwise a request
    // scoped `s3` served by a non-decoding endpoint (e.g. CloudFront reading
    // `Name` raw) would verify a replayed encoding that acts on a different
    // value.
    // The fallback is skipped when the wire query contains a raw `+`: it is
    // the one byte whose meaning downstream decoders disagree on (S3 decodes
    // a literal plus, API Gateway / Lambda decode a space), so no single
    // normalized form is safe for every service. Reject the ambiguous
    // representation instead of guessing (the raw attempt above already
    // failed, so fall through to the signature mismatch below).
    // If the query is not valid UTF-8 after percent-decoding, normalization
    // is impossible; the raw attempt above already failed, so fall through to
    // the signature mismatch below.
    let normalized_matches = allow_normalized_fallback
        && !query.contains('+')
        && build_canonical_query_string_normalized(query).is_ok_and(|normalized| {
            normalized != build_canonical_query_string(query) && signature_matches(&normalized)
        });
    if normalized_matches {
        debug!(access_key_id = %parsed.access_key_id, "Signature verification succeeded (normalized query string)");
        return Ok(AuthResult {
            access_key_id: parsed.access_key_id,
            region: parsed.region,
            service: parsed.service,
            signed_headers: parsed.signed_headers,
        });
    }

    debug!("Signature mismatch");
    Err(AuthError::SignatureDoesNotMatch)
}

/// SigV4 streaming HMAC chain, including terminal chunks and signed trailers.
pub struct StreamingVerifier {
    key: Vec<u8>,
    timestamp: String,
    scope: String,
    previous: String,
}

impl std::fmt::Debug for StreamingVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StreamingVerifier([REDACTED])")
    }
}

impl StreamingVerifier {
    /// Authenticate the seed and initialize the chunk chain.
    /// # Errors
    /// Returns seed signature or header validation failures.
    pub fn new(
        parts: &http::request::Parts,
        provider: &dyn CredentialProvider,
    ) -> Result<Self, AuthError> {
        verify_s3_sigv4(parts, &hash_payload(b""), provider)?;
        let header = extract_header_value(parts, "authorization")?;
        let parsed = parse_authorization_header(&header)?;
        let secret = provider.get_secret_key(&parsed.access_key_id)?;
        Ok(Self {
            key: derive_signing_key(&secret, &parsed.date, &parsed.region, &parsed.service),
            timestamp: extract_header_value(parts, "x-amz-date")?,
            scope: format!(
                "{}/{}/{}/aws4_request",
                parsed.date, parsed.region, parsed.service
            ),
            previous: parsed.signature,
        })
    }

    /// Verify a data chunk, including the final zero-size chunk.
    /// # Errors
    /// Rejects a missing, malformed or mismatching chunk signature.
    pub fn verify_chunk(&mut self, actual_sha256: &str, signature: &str) -> Result<(), AuthError> {
        let text = format!(
            "AWS4-HMAC-SHA256-PAYLOAD\n{}\n{}\n{}\n{}\n{}",
            self.timestamp,
            self.scope,
            self.previous,
            hash_payload(b""),
            actual_sha256
        );
        self.verify_next(&text, signature)
    }

    /// Verify the canonical declared trailer block after the terminal chunk.
    /// # Errors
    /// Returns an error if the trailer signature does not match.
    pub fn verify_trailer(&mut self, canonical: &str, signature: &str) -> Result<(), AuthError> {
        let text = format!(
            "AWS4-HMAC-SHA256-TRAILER\n{}\n{}\n{}\n{}",
            self.timestamp,
            self.scope,
            self.previous,
            hash_payload(canonical.as_bytes())
        );
        self.verify_next(&text, signature)
    }

    fn verify_next(&mut self, text: &str, signature: &str) -> Result<(), AuthError> {
        let expected = compute_signature(&self.key, text);
        if !bool::from(expected.as_bytes().ct_eq(signature.as_bytes())) {
            return Err(AuthError::SignatureDoesNotMatch);
        }
        self.previous = expected;
        Ok(())
    }
}

fn validated_payload_hash<'a>(
    parts: &'a http::request::Parts,
    actual: &'a str,
    allow_unsigned: bool,
) -> Result<&'a str, AuthError> {
    let mut values = parts.headers.get_all("x-amz-content-sha256").iter();
    let Some(value) = values.next() else {
        return Ok(actual);
    };
    if values.next().is_some() {
        return Err(AuthError::InvalidPayloadHash);
    }
    let declared = value.to_str().map_err(|_| AuthError::InvalidPayloadHash)?;
    if allow_unsigned
        && matches!(
            declared,
            "UNSIGNED-PAYLOAD"
                | "STREAMING-AWS4-HMAC-SHA256-PAYLOAD"
                | "STREAMING-AWS4-HMAC-SHA256-PAYLOAD-TRAILER"
                | "STREAMING-UNSIGNED-PAYLOAD-TRAILER"
        )
    {
        return Ok(declared);
    }
    if declared.len() != 64
        || !declared
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || !bool::from(declared.as_bytes().ct_eq(actual.as_bytes()))
    {
        return Err(AuthError::InvalidPayloadHash);
    }
    Ok(actual)
}

/// Extract a header value as a string from the request parts.
fn extract_header_value(parts: &http::request::Parts, name: &str) -> Result<String, AuthError> {
    parts
        .headers
        .get(name)
        .ok_or_else(|| AuthError::MissingHeader(name.to_owned()))?
        .to_str()
        .map(ToOwned::to_owned)
        .map_err(|_| AuthError::MissingHeader(name.to_owned()))
}

/// Collect header name-value pairs for the specified signed headers.
fn collect_signed_headers<'a>(
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

/// Compute HMAC-SHA256 and return the raw bytes.
fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC can accept keys of any length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// Compute the SHA-256 hash of the given payload and return it as a hex string.
///
/// This is a convenience function for computing the `x-amz-content-sha256` header value.
///
/// # Examples
///
/// ```
/// use rustack_auth::sigv4::hash_payload;
///
/// // SHA-256 of empty payload
/// assert_eq!(
///     hash_payload(b""),
///     "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
/// );
/// ```
#[must_use]
pub fn hash_payload(payload: &[u8]) -> String {
    hex::encode(Sha256::digest(payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{canonical::build_signed_headers_string, credentials::StaticCredentialProvider};

    const TEST_ACCESS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";
    const TEST_SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
    const TEST_DATE: &str = "20130524";
    const TEST_REGION: &str = "us-east-1";
    const TEST_SERVICE: &str = "s3";

    fn test_credential_provider() -> StaticCredentialProvider {
        StaticCredentialProvider::new(vec![(
            TEST_ACCESS_KEY.to_owned(),
            TEST_SECRET_KEY.to_owned(),
        )])
    }

    #[test]
    fn test_should_derive_signing_key_matching_aws_test_vector() {
        let key = derive_signing_key(TEST_SECRET_KEY, TEST_DATE, TEST_REGION, TEST_SERVICE);
        // The signing key itself is not published as hex in the AWS docs,
        // but we can verify it produces the correct signature when used.
        assert_eq!(key.len(), 32); // SHA-256 produces 32 bytes
    }

    #[test]
    fn test_should_parse_authorization_header() {
        let header = "AWS4-HMAC-SHA256 \
                      Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request,\
                      SignedHeaders=host;range;x-amz-content-sha256;x-amz-date,\
                      Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41";

        let parsed = parse_authorization_header(header).unwrap();
        assert_eq!(parsed.algorithm, "AWS4-HMAC-SHA256");
        assert_eq!(parsed.access_key_id, "AKIAIOSFODNN7EXAMPLE");
        assert_eq!(parsed.date, "20130524");
        assert_eq!(parsed.region, "us-east-1");
        assert_eq!(parsed.service, "s3");
        assert_eq!(
            parsed.signed_headers,
            vec!["host", "range", "x-amz-content-sha256", "x-amz-date"]
        );
        assert_eq!(
            parsed.signature,
            "f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    #[test]
    fn test_should_reject_unsupported_algorithm() {
        let header = "AWS4-HMAC-SHA512 \
                      Credential=AKID/20130524/us-east-1/s3/aws4_request,SignedHeaders=host,\
                      Signature=abc";
        let result = parse_authorization_header(header);
        assert!(matches!(result, Err(AuthError::UnsupportedAlgorithm(_))));
    }

    #[test]
    fn test_should_reject_invalid_credential_format() {
        let header =
            "AWS4-HMAC-SHA256 Credential=AKID/20130524/us-east-1,SignedHeaders=host,Signature=abc";
        let result = parse_authorization_header(header);
        assert!(matches!(result, Err(AuthError::InvalidCredential)));
    }

    #[test]
    fn test_should_build_string_to_sign_matching_aws_example() {
        let canonical_hash = "7344ae5b7ee6c3e7e6b0fe0640412a37625d1fbfff95c48bbb2dc43964946972";
        let sts = build_string_to_sign(
            "20130524T000000Z",
            "20130524/us-east-1/s3/aws4_request",
            canonical_hash,
        );
        #[rustfmt::skip]
        let expected = "AWS4-HMAC-SHA256\n\
                        20130524T000000Z\n\
                        20130524/us-east-1/s3/aws4_request\n\
                        7344ae5b7ee6c3e7e6b0fe0640412a37625d1fbfff95c48bbb2dc43964946972";
        assert_eq!(sts, expected);
    }

    #[test]
    fn test_should_compute_correct_signature_for_aws_get_object_example() {
        // Full end-to-end test using the AWS GET Object example.
        let signing_key = derive_signing_key(TEST_SECRET_KEY, TEST_DATE, TEST_REGION, TEST_SERVICE);

        #[rustfmt::skip]
        let string_to_sign = "AWS4-HMAC-SHA256\n\
                              20130524T000000Z\n\
                              20130524/us-east-1/s3/aws4_request\n\
                              7344ae5b7ee6c3e7e6b0fe0640412a37625d1fbfff95c48bbb2dc43964946972";

        let signature = compute_signature(&signing_key, string_to_sign);
        assert_eq!(
            signature,
            "f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    fn signed_request(payload_hash: &str, declared: bool) -> http::request::Parts {
        let (mut parts, ()) = http::Request::builder()
            .method("POST")
            .uri("/")
            .header("host", "localhost")
            .header("x-amz-date", "20130524T000000Z")
            .body(())
            .unwrap()
            .into_parts();
        let headers = [("host", "localhost"), ("x-amz-date", "20130524T000000Z")];
        let signed = ["host", "x-amz-date"];
        let canonical = build_canonical_request("POST", "/", "", &headers, &signed, payload_hash);
        let text = build_string_to_sign(
            "20130524T000000Z",
            "20130524/us-east-1/s3/aws4_request",
            &hash_payload(canonical.as_bytes()),
        );
        let signature = compute_signature(
            &derive_signing_key(TEST_SECRET_KEY, "20130524", "us-east-1", "s3"),
            &text,
        );
        parts.headers.insert(
            "authorization",
            format!(
                "AWS4-HMAC-SHA256 \
                 Credential={TEST_ACCESS_KEY}/20130524/us-east-1/s3/aws4_request,\
                 SignedHeaders=host;x-amz-date,Signature={signature}"
            )
            .parse()
            .unwrap(),
        );
        if declared {
            parts
                .headers
                .insert("x-amz-content-sha256", payload_hash.parse().unwrap());
        }
        parts
    }

    #[test]
    fn test_should_bind_actual_payload_even_with_unsigned_hash_header() {
        let provider = test_credential_provider();
        let original = hash_payload(b"original");
        for declared in [false, true] {
            let mut parts = signed_request(&original, declared);
            assert!(verify_sigv4(&parts, &original, &provider).is_ok());
            assert!(verify_sigv4(&parts, &hash_payload(b"modified"), &provider).is_err());
            parts
                .headers
                .insert("x-amz-content-sha256", original.parse().unwrap());
            assert!(matches!(
                verify_sigv4(&parts, &hash_payload(b"modified"), &provider),
                Err(AuthError::InvalidPayloadHash)
            ));
        }
    }

    #[test]
    fn test_should_reject_duplicate_malformed_and_protocol_hashes() {
        let provider = test_credential_provider();
        let actual = hash_payload(b"original");
        let mut parts = signed_request(&actual, true);
        parts
            .headers
            .append("x-amz-content-sha256", actual.parse().unwrap());
        assert!(matches!(
            verify_sigv4(&parts, &actual, &provider),
            Err(AuthError::InvalidPayloadHash)
        ));
        for value in [
            "invalid",
            "UNSIGNED-PAYLOAD",
            "STREAMING-UNSIGNED-PAYLOAD-TRAILER",
            "STREAMING-AWS4-HMAC-SHA256-PAYLOAD",
            "UNSIGNED-PAYLOAD-UNKNOWN",
        ] {
            let parts = signed_request(value, true);
            assert!(matches!(
                verify_sigv4(&parts, &actual, &provider),
                Err(AuthError::InvalidPayloadHash)
            ));
        }
        let parts = signed_request("UNSIGNED-PAYLOAD", true);
        assert!(verify_s3_sigv4(&parts, &actual, &provider).is_ok());
    }

    #[test]
    fn test_should_verify_streaming_chunk_chain_and_reject_tampering() {
        let provider = test_credential_provider();
        let parts = signed_request("STREAMING-AWS4-HMAC-SHA256-PAYLOAD", true);
        let mut verifier = StreamingVerifier::new(&parts, &provider).unwrap();
        let digest = hash_payload(b"chunk");
        let text = format!(
            "AWS4-HMAC-SHA256-PAYLOAD\n{}\n{}\n{}\n{}\n{}",
            verifier.timestamp,
            verifier.scope,
            verifier.previous,
            hash_payload(b""),
            digest
        );
        let signature = compute_signature(&verifier.key, &text);
        assert!(
            verifier
                .verify_chunk(&hash_payload(b"wrong"), &signature)
                .is_err()
        );
        assert!(verifier.verify_chunk(&digest, &signature).is_ok());
        assert!(
            verifier.verify_chunk(&digest, &signature).is_err(),
            "a chunk cannot be replayed at the next chain position"
        );
        assert!(!format!("{verifier:?}").contains(&signature));
    }

    #[test]
    fn test_should_verify_sigv4_success() {
        let provider = test_credential_provider();
        let empty_hash = hash_payload(b"");

        // Build a request matching the AWS test vector.
        let mut builder = http::Request::builder()
            .method("GET")
            .uri("http://examplebucket.s3.amazonaws.com/test.txt")
            .header("host", "examplebucket.s3.amazonaws.com")
            .header("range", "bytes=0-9")
            .header("x-amz-content-sha256", &empty_hash)
            .header("x-amz-date", "20130524T000000Z");

        // Compute the expected signature to build the auth header.
        let auth_value = format!(
            "AWS4-HMAC-SHA256 \
             Credential={TEST_ACCESS_KEY}/20130524/us-east-1/s3/aws4_request,SignedHeaders=host;\
             range;x-amz-content-sha256;x-amz-date,\
             Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
        builder = builder.header(http::header::AUTHORIZATION, &auth_value);

        let (parts, _body) = builder.body(()).unwrap().into_parts();
        let result = verify_sigv4(&parts, &empty_hash, &provider);
        assert!(result.is_ok());

        let auth_result = result.unwrap();
        assert_eq!(auth_result.access_key_id, TEST_ACCESS_KEY);
        assert_eq!(auth_result.region, "us-east-1");
        assert_eq!(auth_result.service, "s3");
    }

    #[test]
    fn test_should_fail_sigv4_with_wrong_key() {
        let provider = StaticCredentialProvider::new(vec![(
            TEST_ACCESS_KEY.to_owned(),
            "WRONG_SECRET_KEY".to_owned(),
        )]);
        let empty_hash = hash_payload(b"");

        let auth_value = format!(
            "AWS4-HMAC-SHA256 \
             Credential={TEST_ACCESS_KEY}/20130524/us-east-1/s3/aws4_request,SignedHeaders=host;\
             range;x-amz-content-sha256;x-amz-date,\
             Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );

        let (parts, _body) = http::Request::builder()
            .method("GET")
            .uri("http://examplebucket.s3.amazonaws.com/test.txt")
            .header("host", "examplebucket.s3.amazonaws.com")
            .header("range", "bytes=0-9")
            .header("x-amz-content-sha256", &empty_hash)
            .header("x-amz-date", "20130524T000000Z")
            .header(http::header::AUTHORIZATION, &auth_value)
            .body(())
            .unwrap()
            .into_parts();

        let result = verify_sigv4(&parts, &empty_hash, &provider);
        assert!(matches!(result, Err(AuthError::SignatureDoesNotMatch)));
    }

    #[test]
    fn test_should_verify_request_signed_with_normalized_query_string() {
        // Reproduces the Transmit 5 scenario from the issue: the client signs
        // the SigV4-normalized canonical query string (`prefix=periods%2F`)
        // while the request line carries the raw value (`prefix=periods/`).
        let provider = test_credential_provider();
        let empty_hash = hash_payload(b"");

        let wire_query = "prefix=periods/&max-keys=1";
        let normalized_query = build_canonical_query_string_normalized(wire_query).unwrap();
        assert_eq!(normalized_query, "max-keys=1&prefix=periods%2F");

        // Sign the request the way a spec-compliant client does: canonical
        // request built from the normalized query string.
        let headers = [
            ("host", "examplebucket.s3.amazonaws.com"),
            ("x-amz-date", "20130524T000000Z"),
        ];
        let signed = ["host", "x-amz-date"];
        let canonical = build_canonical_request(
            "GET",
            "/bucket-1",
            &normalized_query,
            &headers.iter().map(|(k, v)| (*k, *v)).collect::<Vec<_>>(),
            &signed,
            &empty_hash,
        );
        let string_to_sign = build_string_to_sign(
            "20130524T000000Z",
            "20130524/us-east-1/s3/aws4_request",
            &hash_payload(canonical.as_bytes()),
        );
        let signature = compute_signature(
            &derive_signing_key(TEST_SECRET_KEY, "20130524", "us-east-1", "s3"),
            &string_to_sign,
        );

        let auth_value = format!(
            "AWS4-HMAC-SHA256 \
             Credential={TEST_ACCESS_KEY}/20130524/us-east-1/s3/aws4_request,SignedHeaders=host;\
             x-amz-date,Signature={signature}"
        );
        let uri = format!("http://examplebucket.s3.amazonaws.com/bucket-1?{wire_query}");
        let (parts, _body) = http::Request::builder()
            .method("GET")
            .uri(&uri)
            .header("host", "examplebucket.s3.amazonaws.com")
            .header("x-amz-date", "20130524T000000Z")
            .header(http::header::AUTHORIZATION, &auth_value)
            .body(())
            .unwrap()
            .into_parts();

        let result = verify_s3_sigv4(&parts, &empty_hash, &provider);
        assert!(result.is_ok());
    }

    /// Build request parts for a GET whose Authorization header signs
    /// `signed_query`, while the request line carries `wire_query`.
    /// Exercises the normalized-fallback path of [`verify_sigv4`].
    fn build_normalized_fallback_request_parts(
        service: &str,
        signed_query: &str,
        wire_query: &str,
    ) -> http::request::Parts {
        let empty_hash = hash_payload(b"");
        let headers = [
            ("host", "examplebucket.s3.amazonaws.com"),
            ("x-amz-date", "20130524T000000Z"),
        ];
        let signed = ["host", "x-amz-date"];
        let canonical = build_canonical_request(
            "GET",
            "/bucket-1",
            signed_query,
            &headers.iter().map(|(k, v)| (*k, *v)).collect::<Vec<_>>(),
            &signed,
            &empty_hash,
        );
        let string_to_sign = build_string_to_sign(
            "20130524T000000Z",
            &format!("20130524/us-east-1/{service}/aws4_request"),
            &hash_payload(canonical.as_bytes()),
        );
        let signature = compute_signature(
            &derive_signing_key(TEST_SECRET_KEY, "20130524", "us-east-1", service),
            &string_to_sign,
        );
        let auth_value = format!(
            "AWS4-HMAC-SHA256 \
             Credential={TEST_ACCESS_KEY}/20130524/us-east-1/{service}/aws4_request,\
             SignedHeaders=host;x-amz-date,Signature={signature}"
        );
        let uri = format!("http://examplebucket.s3.amazonaws.com/bucket-1?{wire_query}");
        http::Request::builder()
            .method("GET")
            .uri(&uri)
            .header("host", "examplebucket.s3.amazonaws.com")
            .header("x-amz-date", "20130524T000000Z")
            .header(http::header::AUTHORIZATION, &auth_value)
            .body(())
            .unwrap()
            .into_parts()
            .0
    }

    #[test]
    fn test_should_reject_plus_sign_tampering_in_normalized_query() {
        // A request signed for `x=%2B` (a literal plus) must not verify when
        // the wire query is tampered to `x=+`. The raw attempt fails (bytes
        // differ) and the normalized fallback is skipped for queries with a
        // raw `+`, because downstream decoders disagree on its meaning (S3:
        // literal plus; API Gateway / Lambda: space).
        let provider = test_credential_provider();
        let empty_hash = hash_payload(b"");

        let parts = build_normalized_fallback_request_parts("s3", "x=%2B", "x=+");
        let result = verify_s3_sigv4(&parts, &empty_hash, &provider);
        assert!(matches!(result, Err(AuthError::SignatureDoesNotMatch)));
    }

    #[test]
    fn test_should_reject_inverse_plus_sign_tampering_in_normalized_query() {
        // The inverse collision: a request signed for `x=%20` (a space) must
        // not verify when the wire query is tampered to `x=+`. S3's downstream
        // decoder keeps the literal plus, so accepting the original signature
        // would authenticate a different effective parameter value.
        let provider = test_credential_provider();
        let empty_hash = hash_payload(b"");

        let parts = build_normalized_fallback_request_parts("s3", "x=%20", "x=+");
        let result = verify_s3_sigv4(&parts, &empty_hash, &provider);
        assert!(matches!(result, Err(AuthError::SignatureDoesNotMatch)));
    }

    #[test]
    fn test_should_reject_ambiguous_raw_plus_in_normalized_fallback() {
        // A form-style client sending `x=a+b` (meaning `a b` downstream of
        // API Gateway / Lambda) while signing the normalized `x=a%20b` is
        // rejected: the same wire bytes mean a literal plus downstream of S3,
        // so the fallback cannot pick a canonical form that is safe for every
        // service. Per the SigV4 spec `+` must be percent-encoded in the
        // canonical query string, so spec-compliant signers are unaffected.
        let provider = test_credential_provider();
        let empty_hash = hash_payload(b"");

        let parts = build_normalized_fallback_request_parts("s3", "x=a%20b", "x=a+b");
        let result = verify_s3_sigv4(&parts, &empty_hash, &provider);
        assert!(matches!(result, Err(AuthError::SignatureDoesNotMatch)));
    }

    #[test]
    fn test_should_select_normalized_fallback_by_endpoint_not_scope() {
        // Fallback eligibility is a property of the serving endpoint, never
        // of the request: the credential scope's service string is
        // request-supplied. A request scoped `s3` served by a non-decoding
        // endpoint (e.g. CloudFront reading `Name` from the raw query) must
        // not get the fallback even though the scope claims `s3` — otherwise
        // a captured `Name=a%2Fb` signature could be replayed as `Name=a/b`
        // and act on a different value than the one signed.
        // The generic verifier (used by non-S3 endpoints) rejects the
        // Transmit-5-style encoding difference; the S3 entry point accepts it.
        let provider = test_credential_provider();
        let empty_hash = hash_payload(b"");

        let parts =
            build_normalized_fallback_request_parts("s3", "prefix=periods%2F", "prefix=periods/");
        assert!(matches!(
            verify_sigv4(&parts, &empty_hash, &provider),
            Err(AuthError::SignatureDoesNotMatch)
        ));

        let parts =
            build_normalized_fallback_request_parts("s3", "prefix=periods%2F", "prefix=periods/");
        assert!(verify_s3_sigv4(&parts, &empty_hash, &provider).is_ok());
    }

    #[test]
    fn test_should_still_verify_request_signed_with_raw_query_values() {
        // Clients such as minio-java (via OkHttp) sign the raw, unencoded
        // values. The raw-preserving first attempt must keep accepting them.
        let provider = test_credential_provider();
        let empty_hash = hash_payload(b"");

        let wire_query = "events=s3:ObjectCreated:*&prefix=test";

        let headers = [
            ("host", "examplebucket.s3.amazonaws.com"),
            ("x-amz-date", "20130524T000000Z"),
        ];
        let signed = ["host", "x-amz-date"];
        let canonical = build_canonical_request(
            "GET",
            "/bucket-1",
            wire_query,
            &headers.iter().map(|(k, v)| (*k, *v)).collect::<Vec<_>>(),
            &signed,
            &empty_hash,
        );
        let string_to_sign = build_string_to_sign(
            "20130524T000000Z",
            "20130524/us-east-1/s3/aws4_request",
            &hash_payload(canonical.as_bytes()),
        );
        let signature = compute_signature(
            &derive_signing_key(TEST_SECRET_KEY, "20130524", "us-east-1", "s3"),
            &string_to_sign,
        );

        let auth_value = format!(
            "AWS4-HMAC-SHA256 \
             Credential={TEST_ACCESS_KEY}/20130524/us-east-1/s3/aws4_request,SignedHeaders=host;\
             x-amz-date,Signature={signature}"
        );
        let uri = format!("http://examplebucket.s3.amazonaws.com/bucket-1?{wire_query}");
        let (parts, _body) = http::Request::builder()
            .method("GET")
            .uri(&uri)
            .header("host", "examplebucket.s3.amazonaws.com")
            .header("x-amz-date", "20130524T000000Z")
            .header(http::header::AUTHORIZATION, &auth_value)
            .body(())
            .unwrap()
            .into_parts();

        let result = verify_sigv4(&parts, &empty_hash, &provider);
        assert!(result.is_ok());
    }

    #[test]
    fn test_should_fail_sigv4_with_missing_auth_header() {
        let provider = test_credential_provider();
        let empty_hash = hash_payload(b"");

        let (parts, _body) = http::Request::builder()
            .method("GET")
            .uri("http://example.com/")
            .header("host", "example.com")
            .body(())
            .unwrap()
            .into_parts();

        let result = verify_sigv4(&parts, &empty_hash, &provider);
        assert!(matches!(result, Err(AuthError::MissingAuthHeader)));
    }

    #[test]
    fn test_should_fail_sigv4_with_unknown_access_key() {
        let provider = StaticCredentialProvider::new(vec![]);
        let empty_hash = hash_payload(b"");

        let auth_value = "AWS4-HMAC-SHA256 \
                          Credential=UNKNOWN_KEY/20130524/us-east-1/s3/aws4_request,\
                          SignedHeaders=host;x-amz-date,Signature=abc123"
            .to_owned();

        let (parts, _body) = http::Request::builder()
            .method("GET")
            .uri("http://example.com/")
            .header("host", "example.com")
            .header("x-amz-date", "20130524T000000Z")
            .header(http::header::AUTHORIZATION, &auth_value)
            .body(())
            .unwrap()
            .into_parts();

        let result = verify_sigv4(&parts, &empty_hash, &provider);
        assert!(matches!(result, Err(AuthError::AccessKeyNotFound(_))));
    }

    #[test]
    fn test_should_hash_empty_payload() {
        assert_eq!(
            hash_payload(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_should_hash_nonempty_payload() {
        let hash = hash_payload(b"Hello, World!");
        assert_eq!(hash.len(), 64); // 32 bytes hex-encoded
        assert_ne!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_should_build_signed_headers_string_from_parsed() {
        let headers = [
            "host".to_owned(),
            "range".to_owned(),
            "x-amz-content-sha256".to_owned(),
            "x-amz-date".to_owned(),
        ];
        let refs: Vec<&str> = headers.iter().map(String::as_str).collect();
        let result = build_signed_headers_string(&refs);
        assert_eq!(result, "host;range;x-amz-content-sha256;x-amz-date");
    }
}
