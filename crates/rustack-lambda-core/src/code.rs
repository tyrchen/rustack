//! S3 code fetching seam for Lambda deployment packages.
//!
//! `RustackLambda` accepts `Code.S3Bucket`/`Code.S3Key` on `CreateFunction`
//! and `UpdateFunctionCode` and stores the fetched bytes through the same
//! path as inline `ZipFile`. The actual download is delegated to an
//! implementor of the [`S3CodeFetcher`] trait so the core crate stays
//! decoupled from any concrete S3 backend.
//!
//! Uses `async-trait` because the provider stores the fetcher as
//! `Arc<dyn S3CodeFetcher>` (object-safe dynamic dispatch), which native
//! `async fn` in traits cannot express (see AGENTS.md § Async & Concurrency).

use anyhow::anyhow;
use async_trait::async_trait;
use bytes::Bytes;

/// Errors that can occur while fetching function code from S3.
///
/// The variants mirror the S3 failure taxonomy the provider maps onto
/// AWS-compatible Lambda error messages.
#[derive(Debug, thiserror::Error)]
pub enum S3CodeFetchError {
    /// The referenced bucket does not exist.
    #[error("bucket not found: {bucket}")]
    BucketNotFound {
        /// Bucket name.
        bucket: String,
    },
    /// The referenced object does not exist (or is a delete marker).
    #[error("object not found: bucket={bucket}, key={key}")]
    ObjectNotFound {
        /// Bucket name.
        bucket: String,
        /// Object key.
        key: String,
    },
    /// The referenced object version does not exist.
    #[error("object version not found: bucket={bucket}, key={key}, version={version}")]
    VersionNotFound {
        /// Bucket name.
        bucket: String,
        /// Object key.
        key: String,
        /// Requested version id.
        version: String,
    },
    /// Any other failure (e.g. S3 service unavailable).
    #[error("internal error fetching code from S3: {0}")]
    Internal(#[source] anyhow::Error),
}

/// Fetches deployment package bytes from an S3 location.
///
/// # Object safety
///
/// This trait uses `async-trait` because it is stored behind
/// `Arc<dyn S3CodeFetcher>` for dynamic dispatch; native `async fn` in
/// traits is not object safe.
#[async_trait]
pub trait S3CodeFetcher: std::fmt::Debug + Send + Sync {
    /// Download the object at `bucket`/`key`, optionally pinning a specific
    /// version.
    ///
    /// # Errors
    ///
    /// Returns [`S3CodeFetchError`] when the bucket, object, or version is
    /// missing, or when the download fails for any other reason.
    async fn fetch_code(
        &self,
        bucket: &str,
        key: &str,
        version: Option<&str>,
    ) -> Result<Bytes, S3CodeFetchError>;
}

/// Default fetcher used when no S3 backend is wired (S3 service disabled or
/// feature not compiled).
///
/// Always fails with a message that tells the user how to proceed, so
/// `CreateFunction` rejects S3 code packages at creation time instead of
/// failing later at invoke with a confusing `missing code root` error.
#[derive(Debug)]
pub struct UnavailableS3CodeFetcher;

#[async_trait]
impl S3CodeFetcher for UnavailableS3CodeFetcher {
    async fn fetch_code(
        &self,
        _bucket: &str,
        _key: &str,
        _version: Option<&str>,
    ) -> Result<Bytes, S3CodeFetchError> {
        Err(S3CodeFetchError::Internal(anyhow!(
            "S3 service is not enabled. Enable S3 (SERVICES=s3,lambda) or provide code via ZipFile"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_should_describe_unavailable_fetcher_error() {
        let err = UnavailableS3CodeFetcher
            .fetch_code("bucket", "key", None)
            .await
            .expect_err("unavailable fetcher must fail");
        let msg = err.to_string();
        assert!(msg.contains("S3 service is not enabled"), "got: {msg}");
        assert!(msg.contains("ZipFile"), "got: {msg}");
    }
}
