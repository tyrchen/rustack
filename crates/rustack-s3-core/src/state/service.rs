//! Top-level S3 service state.
//!
//! [`S3ServiceState`] manages the collection of buckets and enforces global
//! bucket-name uniqueness. All operations are thread-safe via `DashMap`.

use chrono::{DateTime, Utc};
use dashmap::{
    DashMap,
    mapref::one::{Ref, RefMut},
};
use tracing::{debug, info};

use super::{bucket::S3Bucket, object::Owner};
use crate::error::S3ServiceError;

/// Top-level S3 service state holding all buckets.
///
/// Bucket names are globally unique across accounts, enforced by
/// `global_bucket_owner`. Per-account bucket data is stored in `buckets`.
///
/// All fields are accessed concurrently via `DashMap`; no external locking is
/// required.
pub struct S3ServiceState {
    /// Bucket name to `S3Bucket` mapping.
    buckets: DashMap<String, S3Bucket>,
    /// Bucket name to account-ID mapping (enforces global uniqueness).
    global_bucket_owner: DashMap<String, String>,
}

impl std::fmt::Debug for S3ServiceState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3ServiceState")
            .field("bucket_count", &self.buckets.len())
            .finish_non_exhaustive()
    }
}

impl Default for S3ServiceState {
    fn default() -> Self {
        Self::new()
    }
}

impl S3ServiceState {
    /// Create a new, empty service state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            buckets: DashMap::new(),
            global_bucket_owner: DashMap::new(),
        }
    }

    /// Create a new bucket.
    ///
    /// # Errors
    ///
    /// - [`S3ServiceError::BucketAlreadyOwnedByYou`] if the caller already owns a bucket with the
    ///   same name.
    /// - [`S3ServiceError::BucketAlreadyExists`] if the bucket name is taken by a different
    ///   account.
    pub fn create_bucket(
        &self,
        name: String,
        region: String,
        owner: Owner,
    ) -> Result<(), S3ServiceError> {
        let account_id = owner.id.clone();

        // Check global uniqueness.
        if let Some(existing_owner) = self.global_bucket_owner.get(&name) {
            if *existing_owner == account_id {
                return Err(S3ServiceError::BucketAlreadyOwnedByYou { bucket: name });
            }
            return Err(S3ServiceError::BucketAlreadyExists { bucket: name });
        }

        // Insert into both maps.
        let bucket = S3Bucket::new(name.clone(), region, owner);
        self.buckets.insert(name.clone(), bucket);
        self.global_bucket_owner.insert(name.clone(), account_id);

        info!(bucket = %name, "bucket created");
        Ok(())
    }

    /// Delete a bucket.
    ///
    /// # Errors
    ///
    /// - [`S3ServiceError::NoSuchBucket`] if the bucket does not exist.
    /// - [`S3ServiceError::BucketNotEmpty`] if the bucket still contains objects or in-progress
    ///   multipart uploads.
    pub fn delete_bucket(&self, name: &str) -> Result<(), S3ServiceError> {
        let bucket_ref = self
            .buckets
            .get(name)
            .ok_or_else(|| S3ServiceError::NoSuchBucket {
                bucket: name.to_owned(),
            })?;

        if !bucket_ref.is_empty() {
            return Err(S3ServiceError::BucketNotEmpty {
                bucket: name.to_owned(),
            });
        }

        // Drop the read reference before removing.
        drop(bucket_ref);

        self.buckets.remove(name);
        self.global_bucket_owner.remove(name);

        info!(bucket = %name, "bucket deleted");
        Ok(())
    }

    /// Get an immutable reference to a bucket.
    ///
    /// # Errors
    ///
    /// Returns [`S3ServiceError::NoSuchBucket`] if the bucket does not exist.
    pub fn get_bucket(&self, name: &str) -> Result<Ref<'_, String, S3Bucket>, S3ServiceError> {
        self.buckets
            .get(name)
            .ok_or_else(|| S3ServiceError::NoSuchBucket {
                bucket: name.to_owned(),
            })
    }

    /// Get a mutable reference to a bucket.
    ///
    /// # Errors
    ///
    /// Returns [`S3ServiceError::NoSuchBucket`] if the bucket does not exist.
    pub fn get_bucket_mut(
        &self,
        name: &str,
    ) -> Result<RefMut<'_, String, S3Bucket>, S3ServiceError> {
        self.buckets
            .get_mut(name)
            .ok_or_else(|| S3ServiceError::NoSuchBucket {
                bucket: name.to_owned(),
            })
    }

    /// List all buckets, returning `(name, creation_date)` pairs sorted by name.
    #[must_use]
    pub fn list_buckets(&self) -> Vec<(String, DateTime<Utc>)> {
        let mut buckets: Vec<(String, DateTime<Utc>)> = self
            .buckets
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().creation_date))
            .collect();
        buckets.sort_by(|a, b| a.0.cmp(&b.0));
        buckets
    }

    /// Check whether a bucket exists.
    #[must_use]
    pub fn bucket_exists(&self, name: &str) -> bool {
        self.buckets.contains_key(name)
    }

    /// Reset all state, removing all buckets.
    pub fn reset(&self) {
        debug!("resetting all S3 service state");
        self.buckets.clear();
        self.global_bucket_owner.clear();
    }

    /// Return all bucket names sorted for deterministic snapshot export.
    #[must_use]
    pub(crate) fn snapshot_bucket_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .buckets
            .iter()
            .map(|entry| entry.key().clone())
            .collect();
        names.sort();
        names
    }

    /// Insert a bucket while rebuilding the global owner index.
    pub(crate) fn insert_snapshot_bucket(&self, bucket: S3Bucket) {
        let name = bucket.name.clone();
        let owner_id = bucket.owner.id.clone();
        self.buckets.insert(name.clone(), bucket);
        self.global_bucket_owner.insert(name, owner_id);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn default_owner() -> Owner {
        Owner::default()
    }

    fn other_owner() -> Owner {
        Owner {
            id: "other-account-id".to_owned(),
            display_name: "other-user".to_owned(),
        }
    }

    #[test]
    fn test_should_create_empty_service_state() {
        let state = S3ServiceState::new();
        assert!(!state.bucket_exists("anything"));
        assert!(state.list_buckets().is_empty());
    }

    #[test]
    fn test_should_debug_format_service_state() {
        let state = S3ServiceState::new();
        let debug_str = format!("{state:?}");
        assert!(debug_str.contains("S3ServiceState"));
    }

    #[test]
    fn test_should_create_and_list_bucket() {
        let state = S3ServiceState::new();
        state
            .create_bucket(
                "my-bucket".to_owned(),
                "us-east-1".to_owned(),
                default_owner(),
            )
            .unwrap_or_else(|e| panic!("create_bucket failed: {e}"));

        assert!(state.bucket_exists("my-bucket"));

        let buckets = state.list_buckets();
        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets[0].0, "my-bucket");
    }

    #[test]
    fn test_should_reject_duplicate_bucket_same_owner() {
        let state = S3ServiceState::new();
        state
            .create_bucket("dup".to_owned(), "us-east-1".to_owned(), default_owner())
            .unwrap_or_else(|e| panic!("first create failed: {e}"));

        let result = state.create_bucket("dup".to_owned(), "us-east-1".to_owned(), default_owner());
        assert!(
            matches!(result, Err(S3ServiceError::BucketAlreadyOwnedByYou { .. })),
            "expected BucketAlreadyOwnedByYou, got {result:?}"
        );
    }

    #[test]
    fn test_should_reject_duplicate_bucket_different_owner() {
        let state = S3ServiceState::new();
        state
            .create_bucket("shared".to_owned(), "us-east-1".to_owned(), default_owner())
            .unwrap_or_else(|e| panic!("first create failed: {e}"));

        let result =
            state.create_bucket("shared".to_owned(), "eu-west-1".to_owned(), other_owner());
        assert!(
            matches!(result, Err(S3ServiceError::BucketAlreadyExists { .. })),
            "expected BucketAlreadyExists, got {result:?}"
        );
    }

    #[test]
    fn test_should_delete_empty_bucket() {
        let state = S3ServiceState::new();
        state
            .create_bucket(
                "deleteme".to_owned(),
                "us-east-1".to_owned(),
                default_owner(),
            )
            .unwrap_or_else(|e| panic!("create failed: {e}"));

        state
            .delete_bucket("deleteme")
            .unwrap_or_else(|e| panic!("delete failed: {e}"));

        assert!(!state.bucket_exists("deleteme"));
        assert!(state.list_buckets().is_empty());
    }

    #[test]
    fn test_should_reject_delete_nonexistent_bucket() {
        let state = S3ServiceState::new();
        let result = state.delete_bucket("ghost");
        assert!(matches!(result, Err(S3ServiceError::NoSuchBucket { .. })));
    }

    #[test]
    fn test_should_reject_delete_non_empty_bucket() {
        use crate::state::object::{ObjectMetadata, S3Object};

        let state = S3ServiceState::new();
        state
            .create_bucket("full".to_owned(), "us-east-1".to_owned(), default_owner())
            .unwrap_or_else(|e| panic!("create failed: {e}"));

        // Insert an object via the bucket's object store.
        {
            let bucket = state
                .get_bucket("full")
                .unwrap_or_else(|e| panic!("get failed: {e}"));
            let obj = S3Object {
                key: "file.txt".to_owned(),
                version_id: "null".to_owned(),
                etag: "\"abc\"".to_owned(),
                size: 42,
                last_modified: chrono::Utc::now(),
                storage_class: "STANDARD".to_owned(),
                metadata: ObjectMetadata::default(),
                owner: default_owner(),
                checksum: None,
                parts_count: None,
                part_etags: Vec::new(),
            };
            bucket.objects.write().put(obj);
        }

        let result = state.delete_bucket("full");
        assert!(
            matches!(result, Err(S3ServiceError::BucketNotEmpty { .. })),
            "expected BucketNotEmpty, got {result:?}"
        );
    }

    #[test]
    fn test_should_get_bucket_immutable_ref() {
        let state = S3ServiceState::new();
        state
            .create_bucket(
                "ref-test".to_owned(),
                "us-east-1".to_owned(),
                default_owner(),
            )
            .unwrap_or_else(|e| panic!("create failed: {e}"));

        let bucket = state
            .get_bucket("ref-test")
            .unwrap_or_else(|e| panic!("get failed: {e}"));
        assert_eq!(bucket.name, "ref-test");
        assert_eq!(bucket.region, "us-east-1");
    }

    #[test]
    fn test_should_get_bucket_mutable_ref() {
        let state = S3ServiceState::new();
        state
            .create_bucket(
                "mut-test".to_owned(),
                "us-east-1".to_owned(),
                default_owner(),
            )
            .unwrap_or_else(|e| panic!("create failed: {e}"));

        let bucket = state
            .get_bucket_mut("mut-test")
            .unwrap_or_else(|e| panic!("get_mut failed: {e}"));
        assert_eq!(bucket.name, "mut-test");
    }

    #[test]
    fn test_should_return_error_for_nonexistent_bucket() {
        let state = S3ServiceState::new();
        assert!(matches!(
            state.get_bucket("nope"),
            Err(S3ServiceError::NoSuchBucket { .. })
        ));
        assert!(matches!(
            state.get_bucket_mut("nope"),
            Err(S3ServiceError::NoSuchBucket { .. })
        ));
    }

    #[test]
    fn test_should_list_buckets_sorted() {
        let state = S3ServiceState::new();
        for name in ["charlie", "alpha", "bravo"] {
            state
                .create_bucket(name.to_owned(), "us-east-1".to_owned(), default_owner())
                .unwrap_or_else(|e| panic!("create {name} failed: {e}"));
        }

        let names: Vec<String> = state.list_buckets().into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, vec!["alpha", "bravo", "charlie"]);
    }

    #[test]
    fn test_should_reset_all_state() {
        let state = S3ServiceState::new();
        state
            .create_bucket("a".to_owned(), "us-east-1".to_owned(), default_owner())
            .unwrap_or_else(|e| panic!("create failed: {e}"));
        state
            .create_bucket("b".to_owned(), "us-east-1".to_owned(), default_owner())
            .unwrap_or_else(|e| panic!("create failed: {e}"));

        assert_eq!(state.list_buckets().len(), 2);
        state.reset();
        assert!(state.list_buckets().is_empty());
        assert!(!state.bucket_exists("a"));
        assert!(!state.bucket_exists("b"));
    }

    #[test]
    fn test_should_recreate_bucket_after_delete() {
        let state = S3ServiceState::new();
        state
            .create_bucket("reuse".to_owned(), "us-east-1".to_owned(), default_owner())
            .unwrap_or_else(|e| panic!("create failed: {e}"));
        state
            .delete_bucket("reuse")
            .unwrap_or_else(|e| panic!("delete failed: {e}"));

        // Should be able to recreate.
        state
            .create_bucket("reuse".to_owned(), "eu-west-1".to_owned(), default_owner())
            .unwrap_or_else(|e| panic!("recreate failed: {e}"));

        let bucket = state
            .get_bucket("reuse")
            .unwrap_or_else(|e| panic!("get failed: {e}"));
        assert_eq!(bucket.region, "eu-west-1");
    }

    #[test]
    fn test_should_use_default_trait() {
        let state = S3ServiceState::default();
        assert!(state.list_buckets().is_empty());
    }
}
