//! In-memory store for CloudFront resources.
//!
//! Each resource kind is a `DashMap` keyed by ID. Tags are keyed by ARN so a
//! single `TagResource` / `UntagResource` implementation works across every
//! taggable resource kind.

use std::sync::Arc;

use dashmap::DashMap;
use rustack_cloudfront_model::{
    CachePolicy, CloudFrontFunction, CloudFrontOriginAccessIdentity, Distribution,
    FieldLevelEncryption, FieldLevelEncryptionProfile, Invalidation, KeyGroup, KeyValueStore,
    MonitoringSubscription, OriginAccessControl, OriginRequestPolicy, PublicKey, RealtimeLogConfig,
    ResponseHeadersPolicy, TagSet,
};

/// In-memory store for all CloudFront resource kinds.
#[derive(Debug, Default)]
pub struct CloudFrontStore {
    /// Distributions, keyed by distribution ID.
    pub distributions: DashMap<String, Distribution>,
    /// Invalidations, keyed by `(distribution_id, invalidation_id)`.
    pub invalidations: DashMap<(String, String), Invalidation>,
    /// Origin access controls.
    pub origin_access_controls: DashMap<String, OriginAccessControl>,
    /// Origin access identities (legacy).
    pub origin_access_identities: DashMap<String, CloudFrontOriginAccessIdentity>,
    /// Cache policies (managed + customer).
    pub cache_policies: DashMap<String, CachePolicy>,
    /// Origin request policies (managed + customer).
    pub origin_request_policies: DashMap<String, OriginRequestPolicy>,
    /// Response headers policies (managed + customer).
    pub response_headers_policies: DashMap<String, ResponseHeadersPolicy>,
    /// Key groups.
    pub key_groups: DashMap<String, KeyGroup>,
    /// Public keys.
    pub public_keys: DashMap<String, PublicKey>,
    /// CloudFront Functions, keyed by function name.
    pub functions: DashMap<String, CloudFrontFunction>,
    /// Field-level encryption configs.
    pub fle_configs: DashMap<String, FieldLevelEncryption>,
    /// Field-level encryption profiles.
    pub fle_profiles: DashMap<String, FieldLevelEncryptionProfile>,
    /// Monitoring subscriptions, keyed by distribution ID.
    pub monitoring_subscriptions: DashMap<String, MonitoringSubscription>,
    /// Key-value stores.
    pub key_value_stores: DashMap<String, KeyValueStore>,
    /// Realtime log configs, keyed by name.
    pub realtime_log_configs: DashMap<String, RealtimeLogConfig>,
    /// Tag sets, keyed by resource ARN.
    pub tags: DashMap<String, TagSet>,
}

impl CloudFrontStore {
    /// Create a new empty store wrapped in `Arc`.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}
