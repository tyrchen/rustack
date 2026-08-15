//! Bridge between Lambda and rustack's in-process S3 provider.
//!
//! Implements the [`S3CodeFetcher`] trait from `rustack-lambda-core` by
//! wrapping the actual S3 provider. This bridge lives in the server binary
//! to avoid a direct dependency from `rustack-lambda-core` to
//! `rustack-s3-core` (same pattern as `sns_bridge.rs` and
//! `events_bridge.rs`).

use std::sync::Arc;

use anyhow::anyhow;
use async_trait::async_trait;
use bytes::Bytes;
use rustack_lambda_core::code::{S3CodeFetchError, S3CodeFetcher};
use rustack_s3_core::{RustackS3, error::S3ServiceError};

/// Fetches Lambda deployment packages from rustack's own S3 service.
#[derive(Debug)]
pub struct LambdaS3CodeFetcher {
    s3: Arc<RustackS3>,
}

impl LambdaS3CodeFetcher {
    /// Create a new fetcher wrapping the given S3 provider.
    #[must_use]
    pub fn new(s3: Arc<RustackS3>) -> Self {
        Self { s3 }
    }
}

#[async_trait]
impl S3CodeFetcher for LambdaS3CodeFetcher {
    async fn fetch_code(
        &self,
        bucket: &str,
        key: &str,
        version: Option<&str>,
    ) -> Result<Bytes, S3CodeFetchError> {
        // Resolve the bucket; missing buckets fail fast at creation time.
        let state = self.s3.state();
        let bucket_ref =
            state
                .get_bucket(bucket)
                .map_err(|_| S3CodeFetchError::BucketNotFound {
                    bucket: bucket.to_owned(),
                })?;

        // Resolve the object version while holding the objects lock. The
        // parking_lot guard is `!Send`, so it must be dropped before any
        // `.await` (mirroring `handle_get_object`).
        let storage_version_id = {
            let store = bucket_ref.objects.read();
            let obj = match version {
                Some(v) => {
                    if store.is_delete_marker(key, v) {
                        return Err(S3CodeFetchError::ObjectNotFound {
                            bucket: bucket.to_owned(),
                            key: key.to_owned(),
                        });
                    }
                    store
                        .get_version(key, v)
                        .ok_or_else(|| S3CodeFetchError::VersionNotFound {
                            bucket: bucket.to_owned(),
                            key: key.to_owned(),
                            version: v.to_owned(),
                        })?
                }
                None => store
                    .get(key)
                    .ok_or_else(|| S3CodeFetchError::ObjectNotFound {
                        bucket: bucket.to_owned(),
                        key: key.to_owned(),
                    })?,
            };
            obj.version_id.clone()
        };

        // Read the object data (outside the lock).
        self.s3
            .storage()
            .read_object(bucket, key, &storage_version_id, None)
            .await
            .map_err(|e| match e {
                S3ServiceError::NoSuchKey { .. } => S3CodeFetchError::ObjectNotFound {
                    bucket: bucket.to_owned(),
                    key: key.to_owned(),
                },
                other => S3CodeFetchError::Internal(anyhow!(other)),
            })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rustack_s3_core::{RustackS3, S3Config};
    use rustack_s3_model::{
        input::{CreateBucketInput, PutObjectInput},
        request::StreamingBlob,
    };

    use super::*;

    fn test_s3() -> Arc<RustackS3> {
        Arc::new(RustackS3::new(S3Config::default()))
    }

    async fn create_bucket(s3: &RustackS3, name: &str) {
        s3.handle_create_bucket(CreateBucketInput {
            bucket: name.to_owned(),
            ..Default::default()
        })
        .await
        .expect("create bucket");
    }

    async fn put_object(s3: &RustackS3, bucket: &str, key: &str, data: &[u8]) {
        s3.handle_put_object(PutObjectInput {
            bucket: bucket.to_owned(),
            key: key.to_owned(),
            body: Some(StreamingBlob {
                data: Bytes::copy_from_slice(data),
            }),
            ..Default::default()
        })
        .await
        .expect("put object");
    }

    #[tokio::test]
    async fn test_should_fetch_object_round_trip() {
        let s3 = test_s3();
        create_bucket(&s3, "code-bucket").await;
        let data = b"PK\x03\x04 fake lambda zip".to_vec();
        put_object(&s3, "code-bucket", "demo.zip", &data).await;

        let fetcher = LambdaS3CodeFetcher::new(s3);
        let result = fetcher
            .fetch_code("code-bucket", "demo.zip", None)
            .await
            .expect("fetch should succeed");
        assert_eq!(result.as_ref(), data.as_slice());
    }

    #[tokio::test]
    async fn test_should_error_on_missing_bucket() {
        let s3 = test_s3();
        let fetcher = LambdaS3CodeFetcher::new(s3);
        let err = fetcher
            .fetch_code("no-such-bucket", "demo.zip", None)
            .await
            .expect_err("missing bucket must fail");
        assert!(
            matches!(err, S3CodeFetchError::BucketNotFound { .. }),
            "got: {err:?}"
        );
    }

    #[tokio::test]
    async fn test_should_error_on_missing_object() {
        let s3 = test_s3();
        create_bucket(&s3, "code-bucket").await;
        let fetcher = LambdaS3CodeFetcher::new(s3);
        let err = fetcher
            .fetch_code("code-bucket", "missing.zip", None)
            .await
            .expect_err("missing object must fail");
        assert!(
            matches!(err, S3CodeFetchError::ObjectNotFound { .. }),
            "got: {err:?}"
        );
    }

    #[tokio::test]
    async fn test_should_error_on_missing_version() {
        let s3 = test_s3();
        create_bucket(&s3, "code-bucket").await;
        put_object(&s3, "code-bucket", "demo.zip", b"v1").await;
        let fetcher = LambdaS3CodeFetcher::new(s3);
        let err = fetcher
            .fetch_code("code-bucket", "demo.zip", Some("nope"))
            .await
            .expect_err("missing version must fail");
        assert!(
            matches!(err, S3CodeFetchError::VersionNotFound { .. }),
            "got: {err:?}"
        );
    }

    #[tokio::test]
    async fn test_should_fetch_explicit_version_from_versioned_bucket() {
        let s3 = test_s3();
        create_bucket(&s3, "code-bucket").await;
        // Enable versioning so puts generate distinct version ids. This also
        // transitions the object store to the versioned backing store.
        s3.state()
            .get_bucket("code-bucket")
            .expect("bucket exists")
            .enable_versioning();
        put_object(&s3, "code-bucket", "demo.zip", b"version-one").await;
        let first = s3
            .state()
            .get_bucket("code-bucket")
            .expect("bucket exists")
            .objects
            .read()
            .get("demo.zip")
            .expect("object exists")
            .version_id
            .clone();
        put_object(&s3, "code-bucket", "demo.zip", b"version-two").await;

        let fetcher = LambdaS3CodeFetcher::new(s3);
        // Latest should be the second put.
        let latest = fetcher
            .fetch_code("code-bucket", "demo.zip", None)
            .await
            .expect("latest fetch");
        assert_eq!(latest.as_ref(), b"version-two");
        // Explicit version should return the first put.
        let pinned = fetcher
            .fetch_code("code-bucket", "demo.zip", Some(&first))
            .await
            .expect("pinned fetch");
        assert_eq!(pinned.as_ref(), b"version-one");
    }

    #[tokio::test]
    async fn test_should_error_on_delete_marker_version() {
        let s3 = test_s3();
        create_bucket(&s3, "code-bucket").await;
        s3.state()
            .get_bucket("code-bucket")
            .expect("bucket exists")
            .enable_versioning();
        put_object(&s3, "code-bucket", "demo.zip", b"version-one").await;
        let object_version_id = s3
            .state()
            .get_bucket("code-bucket")
            .expect("bucket exists")
            .objects
            .read()
            .get("demo.zip")
            .expect("object exists")
            .version_id
            .clone();
        // Delete the object (creates a delete marker in a versioned bucket).
        let del = s3
            .handle_delete_object(rustack_s3_model::input::DeleteObjectInput {
                bucket: "code-bucket".to_owned(),
                key: "demo.zip".to_owned(),
                ..Default::default()
            })
            .await
            .expect("delete object");
        let marker_version_id = del.version_id.expect("delete marker version id");

        let fetcher = LambdaS3CodeFetcher::new(s3);
        // Pinning the delete marker's own version id must fail fast.
        let err = fetcher
            .fetch_code("code-bucket", "demo.zip", Some(&marker_version_id))
            .await
            .expect_err("delete-marked version must fail");
        assert!(
            matches!(err, S3CodeFetchError::ObjectNotFound { .. }),
            "got: {err:?}"
        );
        // The original object version is still retrievable (matches S3).
        let result = fetcher
            .fetch_code("code-bucket", "demo.zip", Some(&object_version_id))
            .await
            .expect("original version remains retrievable");
        assert_eq!(result.as_ref(), b"version-one");
        // Latest is gone once the newest version is a delete marker.
        let err = fetcher
            .fetch_code("code-bucket", "demo.zip", None)
            .await
            .expect_err("delete-marked object must fail");
        assert!(
            matches!(err, S3CodeFetchError::ObjectNotFound { .. }),
            "got: {err:?}"
        );
    }

    #[tokio::test]
    async fn test_should_fetch_explicit_null_version_from_unversioned_bucket() {
        let s3 = test_s3();
        create_bucket(&s3, "code-bucket").await;
        put_object(&s3, "code-bucket", "demo.zip", b"plain").await;

        let fetcher = LambdaS3CodeFetcher::new(s3);
        // Unversioned objects live under the "null" version id.
        let result = fetcher
            .fetch_code("code-bucket", "demo.zip", Some("null"))
            .await
            .expect("null version fetch");
        assert_eq!(result.as_ref(), b"plain");
    }
}
