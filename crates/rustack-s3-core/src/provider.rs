//! S3 provider that owns the business logic state.
//!
//! [`RustackS3`] is the core S3 provider that owns all service state
//! (buckets, objects, multipart uploads) and the storage backend.
//! Individual S3 operations are implemented in the [`crate::ops`] submodules
//! as `handle_*` methods on `RustackS3`.
//!
//! The server binary implements the `S3Handler` trait (from `rustack-s3-http`)
//! for `RustackS3`, bridging the HTTP layer to these handler methods.

use std::sync::Arc;

use crate::{
    config::S3Config, cors::CorsIndex, state::service::S3ServiceState, storage::InMemoryStorage,
};

/// The main S3 provider.
///
/// All fields are `Arc`-wrapped for cheap cloning and shared ownership
/// across handler tasks.
///
/// # Examples
///
/// ```
/// use rustack_s3_core::RustackS3;
/// use rustack_s3_core::config::S3Config;
///
/// let provider = RustackS3::new(S3Config::default());
/// assert!(!provider.config().gateway_listen.is_empty());
/// ```
#[derive(Debug, Clone)]
pub struct RustackS3 {
    /// Bucket and object metadata state.
    pub(crate) state: Arc<S3ServiceState>,
    /// Object body storage (in-memory with disk spillover).
    pub(crate) storage: Arc<InMemoryStorage>,
    /// Per-bucket CORS rule index for request-time matching.
    pub(crate) cors_index: Arc<CorsIndex>,
    /// Provider configuration.
    pub(crate) config: Arc<S3Config>,
}

impl RustackS3 {
    /// Create a new S3 provider with the given configuration.
    ///
    /// Initializes an empty service state, a storage backend configured with
    /// the memory threshold from `config`, and an empty CORS index.
    #[must_use]
    pub fn new(config: S3Config) -> Self {
        let storage = InMemoryStorage::new(config.s3_max_memory_object_size);
        Self {
            state: Arc::new(S3ServiceState::new()),
            storage: Arc::new(storage),
            cors_index: Arc::new(CorsIndex::new()),
            config: Arc::new(config),
        }
    }

    /// Returns a reference to the service state.
    #[must_use]
    pub fn state(&self) -> &S3ServiceState {
        &self.state
    }

    /// Returns a reference to the storage backend.
    #[must_use]
    pub fn storage(&self) -> &InMemoryStorage {
        &self.storage
    }

    /// Returns a reference to the CORS index.
    #[must_use]
    pub fn cors_index(&self) -> &CorsIndex {
        &self.cors_index
    }

    /// Returns a reference to the provider configuration.
    #[must_use]
    pub fn config(&self) -> &S3Config {
        &self.config
    }

    /// Reset all state (buckets, objects, multipart uploads, CORS rules).
    ///
    /// Primarily useful for testing and the `/_localstack/health` reset endpoint.
    pub fn reset(&self) {
        self.state.reset();
        self.storage.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_create_provider_with_defaults() {
        let provider = RustackS3::new(S3Config::default());
        assert_eq!(provider.config().gateway_listen, "127.0.0.1:4566");
        assert!(provider.state().list_buckets().is_empty());
    }

    #[test]
    fn test_should_debug_format_provider() {
        let provider = RustackS3::new(S3Config::default());
        let debug_str = format!("{provider:?}");
        assert!(debug_str.contains("RustackS3"));
    }

    #[test]
    fn test_should_share_via_arc() {
        let provider = Arc::new(RustackS3::new(S3Config::default()));
        let clone = Arc::clone(&provider);
        assert_eq!(
            provider.config().default_region,
            clone.config().default_region
        );
    }

    #[test]
    fn test_should_clone_provider() {
        let provider = RustackS3::new(S3Config::default());
        let cloned = provider.clone();
        assert_eq!(
            provider.config().default_region,
            cloned.config().default_region
        );
    }

    #[test]
    fn test_should_reset_state() {
        let provider = RustackS3::new(S3Config::default());
        provider
            .state()
            .create_bucket(
                "test".to_owned(),
                "us-east-1".to_owned(),
                crate::state::object::Owner::default(),
            )
            .unwrap_or_else(|e| panic!("create failed: {e}"));
        assert!(provider.state().bucket_exists("test"));

        provider.reset();
        assert!(!provider.state().bucket_exists("test"));
    }
}
