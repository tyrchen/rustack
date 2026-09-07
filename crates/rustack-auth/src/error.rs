//! Error types for SigV4 authentication.
//!
//! All authentication failures are represented by [`AuthError`], which provides
//! specific variants for each failure mode encountered during signature verification.

/// Errors that can occur during AWS Signature Version 4 authentication.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// Strict authentication requires a credential provider.
    #[error("Strict authentication requires a credential provider")]
    MissingCredentialProvider,

    /// The payload declaration is malformed or does not match the actual body.
    #[error("Invalid or mismatching payload SHA256")]
    InvalidPayloadHash,
    /// The `Authorization` header is missing from the request.
    #[error("Missing Authorization header")]
    MissingAuthHeader,

    /// The `Authorization` header could not be parsed.
    #[error("Invalid Authorization header format")]
    InvalidAuthHeader,

    /// The signing algorithm is not supported (only AWS4-HMAC-SHA256 is supported).
    #[error("Unsupported algorithm: {0}")]
    UnsupportedAlgorithm(String),

    /// A required HTTP header referenced in `SignedHeaders` is missing.
    #[error("Missing required header: {0}")]
    MissingHeader(String),

    /// The `Credential` component does not match the expected format
    /// (`AKID/date/region/service/aws4_request`).
    #[error("Invalid credential format")]
    InvalidCredential,

    /// The access key ID was not found in the credential store.
    #[error("Access key not found: {0}")]
    AccessKeyNotFound(String),

    /// The computed signature does not match the provided signature.
    #[error("Signature does not match")]
    SignatureDoesNotMatch,

    /// The presigned URL has expired (current time exceeds `X-Amz-Date` + `X-Amz-Expires`).
    #[error("Request has expired")]
    RequestExpired,

    /// A required query parameter for presigned URL authentication is missing.
    #[error("Missing required query parameter: {0}")]
    MissingQueryParam(String),
}
