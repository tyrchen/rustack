//! S3 bucket data structure and configuration types.
//!
//! An [`S3Bucket`] holds all per-bucket state: objects, multipart uploads,
//! versioning status, and the many optional configurations (encryption, CORS,
//! lifecycle, policy, tags, ACL, notification, logging, public-access-block,
//! ownership controls, object lock, accelerate, request-payment, website,
//! replication, analytics, metrics, inventory, intelligent-tiering).
//!
//! Interior mutability is achieved through `parking_lot::RwLock` for
//! single-valued configuration fields and for the object store, and
//! `DashMap` for the multipart upload table.

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::debug;

use super::keystore::ObjectStore;
use super::multipart::MultipartUpload;
use super::object::{CannedAcl, Owner};

// ---------------------------------------------------------------------------
// Supporting configuration types
// ---------------------------------------------------------------------------

/// Bucket versioning status.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VersioningStatus {
    /// Versioning has never been enabled on this bucket.
    #[default]
    Disabled,
    /// Versioning is currently enabled.
    Enabled,
    /// Versioning was previously enabled but is now suspended.
    Suspended,
}

/// Server-side encryption configuration for a bucket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BucketEncryption {
    /// The encryption algorithm (e.g. `AES256`, `aws:kms`, `aws:kms:dsse`).
    pub sse_algorithm: String,
    /// KMS master key ID (only for `aws:kms` or `aws:kms:dsse`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kms_master_key_id: Option<String>,
    /// Whether an S3 Bucket Key is enabled for SSE-KMS.
    #[serde(default)]
    pub bucket_key_enabled: bool,
}

/// CORS rule configuration stored on a bucket.
///
/// This is the raw configuration value, not the evaluated CORS rule used at
/// request time (see `cors.rs` for the runtime representation).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorsRuleConfig {
    /// Optional identifier for the rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Origins that are allowed to make cross-domain requests.
    pub allowed_origins: Vec<String>,
    /// HTTP methods that the origin is allowed to execute.
    pub allowed_methods: Vec<String>,
    /// Headers that are allowed in a pre-flight `OPTIONS` request.
    #[serde(default)]
    pub allowed_headers: Vec<String>,
    /// Headers in the response that customers are able to access.
    #[serde(default)]
    pub expose_headers: Vec<String>,
    /// Time in seconds that the browser should cache the preflight response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_age_seconds: Option<i32>,
}

/// Public access block configuration for a bucket.
///
/// AWS defines exactly four boolean fields for this configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)]
pub struct PublicAccessBlockConfig {
    /// Whether Amazon S3 should block public ACLs for this bucket.
    #[serde(default)]
    pub block_public_acls: bool,
    /// Whether Amazon S3 should ignore public ACLs for this bucket.
    #[serde(default)]
    pub ignore_public_acls: bool,
    /// Whether Amazon S3 should block public bucket policies.
    #[serde(default)]
    pub block_public_policy: bool,
    /// Whether Amazon S3 should restrict public bucket policies.
    #[serde(default)]
    pub restrict_public_buckets: bool,
}

/// Bucket ownership controls configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipControlsConfig {
    /// The object ownership setting (e.g. `BucketOwnerPreferred`,
    /// `ObjectWriter`, `BucketOwnerEnforced`).
    pub object_ownership: String,
}

/// Object Lock configuration for a bucket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectLockConfiguration {
    /// Whether object lock is enabled (`Enabled`).
    pub object_lock_enabled: String,
    /// Optional default retention rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<ObjectLockRule>,
}

/// A default retention rule within an Object Lock configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectLockRule {
    /// The default retention settings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_retention: Option<DefaultRetention>,
}

/// Default retention settings for Object Lock.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultRetention {
    /// The retention mode (`GOVERNANCE` or `COMPLIANCE`).
    pub mode: String,
    /// Number of days to retain the object.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub days: Option<i32>,
    /// Number of years to retain the object.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub years: Option<i32>,
}

/// Static website hosting configuration for a bucket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebsiteConfig {
    /// The name of the index document (e.g. `index.html`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_document_suffix: Option<String>,
    /// The key of the error document (e.g. `error.html`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_document_key: Option<String>,
    /// Redirect all requests to another host/protocol.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redirect_all_requests_to_host: Option<String>,
    /// Protocol for the redirect (`http` or `https`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redirect_all_requests_to_protocol: Option<String>,
}

// ---------------------------------------------------------------------------
// S3Bucket
// ---------------------------------------------------------------------------

/// An S3 bucket with all its state and configuration.
///
/// Thread-safe: interior fields use `parking_lot::RwLock` for configuration
/// and objects, `DashMap` for multipart uploads.
pub struct S3Bucket {
    /// Bucket name.
    pub name: String,
    /// AWS region where this bucket was created.
    pub region: String,
    /// When the bucket was created.
    pub creation_date: DateTime<Utc>,
    /// The bucket owner.
    pub owner: Owner,

    // -- object storage --
    /// Object key storage (un-versioned or versioned).
    pub objects: RwLock<ObjectStore>,
    /// In-progress multipart uploads, keyed by upload ID.
    pub multipart_uploads: DashMap<String, MultipartUpload>,

    // -- versioning --
    /// Bucket versioning status.
    pub versioning: RwLock<VersioningStatus>,

    // -- configurations (all wrapped in RwLock for interior mutability) --
    /// Server-side encryption configuration.
    pub encryption: RwLock<Option<BucketEncryption>>,
    /// CORS rules.
    pub cors_rules: RwLock<Option<Vec<CorsRuleConfig>>>,
    /// Lifecycle configuration.
    pub lifecycle: RwLock<Option<ruststack_s3_model::types::BucketLifecycleConfiguration>>,
    /// Bucket policy (JSON string).
    pub policy: RwLock<Option<String>>,
    /// Bucket tags.
    pub tags: RwLock<Vec<(String, String)>>,
    /// Canned ACL for the bucket.
    pub acl: RwLock<CannedAcl>,
    /// Notification configuration for the bucket.
    pub notification_configuration:
        RwLock<Option<ruststack_s3_model::types::NotificationConfiguration>>,
    /// Logging configuration (stored as opaque JSON).
    pub logging: RwLock<Option<serde_json::Value>>,
    /// Public access block settings.
    pub public_access_block: RwLock<Option<PublicAccessBlockConfig>>,
    /// Ownership controls.
    pub ownership_controls: RwLock<Option<OwnershipControlsConfig>>,
    /// Whether Object Lock is enabled on this bucket.
    pub object_lock_enabled: RwLock<bool>,
    /// Object Lock configuration (retention rules).
    pub object_lock_configuration: RwLock<Option<ObjectLockConfiguration>>,
    /// Transfer acceleration status (e.g. `"Enabled"`, `"Suspended"`).
    pub accelerate: RwLock<Option<String>>,
    /// Request payment configuration (default `"BucketOwner"`).
    pub request_payment: RwLock<String>,
    /// Static website hosting configuration.
    pub website: RwLock<Option<WebsiteConfig>>,
    /// Replication configuration (stored as opaque JSON).
    pub replication: RwLock<Option<serde_json::Value>>,
    /// Analytics configuration (stored as opaque JSON).
    pub analytics: RwLock<Option<serde_json::Value>>,
    /// Metrics configuration (stored as opaque JSON).
    pub metrics: RwLock<Option<serde_json::Value>>,
    /// Inventory configuration (stored as opaque JSON).
    pub inventory: RwLock<Option<serde_json::Value>>,
    /// Intelligent-Tiering configuration (stored as opaque JSON).
    pub intelligent_tiering: RwLock<Option<serde_json::Value>>,
}

impl std::fmt::Debug for S3Bucket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Bucket")
            .field("name", &self.name)
            .field("region", &self.region)
            .field("creation_date", &self.creation_date)
            .field("owner", &self.owner)
            .field("versioning", &*self.versioning.read())
            .finish_non_exhaustive()
    }
}

impl S3Bucket {
    /// Create a new bucket with the given name, region, and owner.
    ///
    /// All configuration fields are initialized to their defaults.
    #[must_use]
    pub fn new(name: String, region: String, owner: Owner) -> Self {
        Self {
            name,
            region,
            creation_date: Utc::now(),
            owner,
            objects: RwLock::new(ObjectStore::default()),
            multipart_uploads: DashMap::new(),
            versioning: RwLock::new(VersioningStatus::default()),
            encryption: RwLock::new(None),
            cors_rules: RwLock::new(None),
            lifecycle: RwLock::new(None),
            policy: RwLock::new(None),
            tags: RwLock::new(Vec::new()),
            acl: RwLock::new(CannedAcl::default()),
            notification_configuration: RwLock::new(None),
            logging: RwLock::new(None),
            public_access_block: RwLock::new(None),
            ownership_controls: RwLock::new(None),
            object_lock_enabled: RwLock::new(false),
            object_lock_configuration: RwLock::new(None),
            accelerate: RwLock::new(None),
            request_payment: RwLock::new("BucketOwner".to_owned()),
            website: RwLock::new(None),
            replication: RwLock::new(None),
            analytics: RwLock::new(None),
            metrics: RwLock::new(None),
            inventory: RwLock::new(None),
            intelligent_tiering: RwLock::new(None),
        }
    }

    /// Whether the bucket contains zero objects (and no in-progress multipart uploads).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.objects.read().is_empty() && self.multipart_uploads.is_empty()
    }

    /// Whether versioning is currently enabled on this bucket.
    #[must_use]
    pub fn is_versioning_enabled(&self) -> bool {
        *self.versioning.read() == VersioningStatus::Enabled
    }

    /// Enable versioning on this bucket.
    ///
    /// If the bucket is currently un-versioned, the object store is
    /// transitioned to a [`super::keystore::VersionedKeyStore`]. If
    /// versioning was suspended, it is simply re-enabled.
    pub fn enable_versioning(&self) {
        let mut status = self.versioning.write();
        if *status != VersioningStatus::Enabled {
            debug!(bucket = %self.name, "enabling versioning");
            // Transition the object store to versioned if it is not already.
            let mut store = self.objects.write();
            store.transition_to_versioned();
            *status = VersioningStatus::Enabled;
        }
    }

    /// Suspend versioning on this bucket.
    ///
    /// Objects already stored retain their version history. New puts will
    /// receive a version ID of `"null"` (overwriting any existing `"null"`
    /// version).
    pub fn suspend_versioning(&self) {
        let mut status = self.versioning.write();
        if *status == VersioningStatus::Enabled {
            debug!(bucket = %self.name, "suspending versioning");
            *status = VersioningStatus::Suspended;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bucket(name: &str) -> S3Bucket {
        S3Bucket::new(name.to_owned(), "us-east-1".to_owned(), Owner::default())
    }

    #[test]
    fn test_should_create_bucket_with_defaults() {
        let bucket = make_bucket("test-bucket");
        assert_eq!(bucket.name, "test-bucket");
        assert_eq!(bucket.region, "us-east-1");
        assert!(bucket.is_empty());
        assert!(!bucket.is_versioning_enabled());
        assert_eq!(*bucket.versioning.read(), VersioningStatus::Disabled);
        assert_eq!(*bucket.acl.read(), CannedAcl::Private);
        assert_eq!(*bucket.request_payment.read(), "BucketOwner");
    }

    #[test]
    fn test_should_debug_format_bucket() {
        let bucket = make_bucket("debug-bucket");
        let debug_str = format!("{bucket:?}");
        assert!(debug_str.contains("debug-bucket"));
        assert!(debug_str.contains("S3Bucket"));
    }

    #[test]
    fn test_should_enable_versioning() {
        let bucket = make_bucket("versioned-bucket");
        assert!(!bucket.is_versioning_enabled());
        assert!(!bucket.objects.read().is_versioned());

        bucket.enable_versioning();
        assert!(bucket.is_versioning_enabled());
        assert!(bucket.objects.read().is_versioned());
    }

    #[test]
    fn test_should_suspend_versioning() {
        let bucket = make_bucket("suspend-bucket");
        bucket.enable_versioning();
        assert!(bucket.is_versioning_enabled());

        bucket.suspend_versioning();
        assert!(!bucket.is_versioning_enabled());
        assert_eq!(*bucket.versioning.read(), VersioningStatus::Suspended);
        // Object store remains versioned even when suspended.
        assert!(bucket.objects.read().is_versioned());
    }

    #[test]
    fn test_should_not_suspend_if_never_enabled() {
        let bucket = make_bucket("never-versioned");
        bucket.suspend_versioning();
        // Should remain Disabled, not Suspended.
        assert_eq!(*bucket.versioning.read(), VersioningStatus::Disabled);
    }

    #[test]
    fn test_should_enable_versioning_idempotent() {
        let bucket = make_bucket("idem-bucket");
        bucket.enable_versioning();
        bucket.enable_versioning();
        assert!(bucket.is_versioning_enabled());
    }

    #[test]
    fn test_should_report_empty_with_no_objects_or_uploads() {
        let bucket = make_bucket("empty-bucket");
        assert!(bucket.is_empty());
    }

    #[test]
    fn test_should_report_not_empty_with_multipart() {
        let bucket = make_bucket("mp-bucket");
        let upload = super::super::multipart::MultipartUpload::new(
            "upload-1".to_owned(),
            "key".to_owned(),
            Owner::default(),
            super::super::object::ObjectMetadata::default(),
        );
        bucket
            .multipart_uploads
            .insert("upload-1".to_owned(), upload);
        assert!(!bucket.is_empty());
    }

    #[test]
    fn test_should_default_versioning_status_to_disabled() {
        assert_eq!(VersioningStatus::default(), VersioningStatus::Disabled);
    }

    #[test]
    fn test_should_create_cors_rule_config() {
        let rule = CorsRuleConfig {
            id: Some("rule-1".to_owned()),
            allowed_origins: vec!["*".to_owned()],
            allowed_methods: vec!["GET".to_owned(), "PUT".to_owned()],
            allowed_headers: vec!["*".to_owned()],
            expose_headers: Vec::new(),
            max_age_seconds: Some(3600),
        };
        assert_eq!(rule.id, Some("rule-1".to_owned()));
        assert_eq!(rule.allowed_methods.len(), 2);
    }

    #[test]
    fn test_should_create_public_access_block_config() {
        let config = PublicAccessBlockConfig {
            block_public_acls: true,
            ignore_public_acls: true,
            block_public_policy: true,
            restrict_public_buckets: true,
        };
        assert!(config.block_public_acls);
        assert!(config.restrict_public_buckets);
    }

    #[test]
    fn test_should_create_object_lock_configuration() {
        let config = ObjectLockConfiguration {
            object_lock_enabled: "Enabled".to_owned(),
            rule: Some(ObjectLockRule {
                default_retention: Some(DefaultRetention {
                    mode: "GOVERNANCE".to_owned(),
                    days: Some(30),
                    years: None,
                }),
            }),
        };
        let retention = config
            .rule
            .as_ref()
            .and_then(|r| r.default_retention.as_ref());
        assert!(retention.is_some());
        assert_eq!(retention.map(|r| r.days), Some(Some(30)));
    }

    #[test]
    fn test_should_create_bucket_encryption() {
        let enc = BucketEncryption {
            sse_algorithm: "aws:kms".to_owned(),
            kms_master_key_id: Some("arn:aws:kms:us-east-1:123456789012:key/abc".to_owned()),
            bucket_key_enabled: true,
        };
        assert_eq!(enc.sse_algorithm, "aws:kms");
        assert!(enc.bucket_key_enabled);
    }
}
