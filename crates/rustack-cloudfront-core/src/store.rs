//! In-memory store for CloudFront resources.
//!
//! Each resource kind is a `DashMap` keyed by ID. Tags are keyed by ARN so a
//! single `TagResource` / `UntagResource` implementation works across every
//! taggable resource kind.

use std::{hash::Hash, sync::Arc};

use dashmap::DashMap;
use rustack_cloudfront_model::{
    CachePolicy, CloudFrontFunction, CloudFrontOriginAccessIdentity, Distribution,
    FieldLevelEncryption, FieldLevelEncryptionProfile, Invalidation, KeyGroup, KeyValueStore,
    MonitoringSubscription, OriginAccessControl, OriginRequestPolicy, PublicKey, RealtimeLogConfig,
    ResponseHeadersPolicy, TagSet,
};
use serde::{Deserialize, Serialize};

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

    /// Export all CloudFront resources in deterministic order.
    #[must_use]
    pub fn export_snapshot(&self) -> CloudFrontStoreSnapshot {
        CloudFrontStoreSnapshot {
            distributions: sorted_values(&self.distributions, |value| value.id.clone()),
            invalidations: sorted_values(&self.invalidations, |value| {
                (value.distribution_id.clone(), value.id.clone())
            }),
            origin_access_controls: sorted_values(&self.origin_access_controls, |value| {
                value.id.clone()
            }),
            origin_access_identities: sorted_values(&self.origin_access_identities, |value| {
                value.id.clone()
            }),
            cache_policies: sorted_values(&self.cache_policies, |value| value.id.clone()),
            origin_request_policies: sorted_values(&self.origin_request_policies, |value| {
                value.id.clone()
            }),
            response_headers_policies: sorted_values(&self.response_headers_policies, |value| {
                value.id.clone()
            }),
            key_groups: sorted_values(&self.key_groups, |value| value.id.clone()),
            public_keys: sorted_values(&self.public_keys, |value| value.id.clone()),
            functions: sorted_values(&self.functions, |value| value.name.clone()),
            fle_configs: sorted_values(&self.fle_configs, |value| value.id.clone()),
            fle_profiles: sorted_values(&self.fle_profiles, |value| value.id.clone()),
            monitoring_subscriptions: sorted_values(&self.monitoring_subscriptions, |value| {
                value.distribution_id.clone()
            }),
            key_value_stores: sorted_values(&self.key_value_stores, |value| value.name.clone()),
            realtime_log_configs: sorted_values(&self.realtime_log_configs, |value| {
                value.name.clone()
            }),
            tags: sorted_key_values(&self.tags),
        }
    }

    /// Replace all CloudFront resources with snapshot contents.
    pub fn import_snapshot(&self, snapshot: CloudFrontStoreSnapshot) {
        self.distributions.clear();
        self.invalidations.clear();
        self.origin_access_controls.clear();
        self.origin_access_identities.clear();
        self.cache_policies.clear();
        self.origin_request_policies.clear();
        self.response_headers_policies.clear();
        self.key_groups.clear();
        self.public_keys.clear();
        self.functions.clear();
        self.fle_configs.clear();
        self.fle_profiles.clear();
        self.monitoring_subscriptions.clear();
        self.key_value_stores.clear();
        self.realtime_log_configs.clear();
        self.tags.clear();

        for value in snapshot.distributions {
            self.distributions.insert(value.id.clone(), value);
        }
        for value in snapshot.invalidations {
            self.invalidations
                .insert((value.distribution_id.clone(), value.id.clone()), value);
        }
        for value in snapshot.origin_access_controls {
            self.origin_access_controls.insert(value.id.clone(), value);
        }
        for value in snapshot.origin_access_identities {
            self.origin_access_identities
                .insert(value.id.clone(), value);
        }
        for value in snapshot.cache_policies {
            self.cache_policies.insert(value.id.clone(), value);
        }
        for value in snapshot.origin_request_policies {
            self.origin_request_policies.insert(value.id.clone(), value);
        }
        for value in snapshot.response_headers_policies {
            self.response_headers_policies
                .insert(value.id.clone(), value);
        }
        for value in snapshot.key_groups {
            self.key_groups.insert(value.id.clone(), value);
        }
        for value in snapshot.public_keys {
            self.public_keys.insert(value.id.clone(), value);
        }
        for value in snapshot.functions {
            self.functions.insert(value.name.clone(), value);
        }
        for value in snapshot.fle_configs {
            self.fle_configs.insert(value.id.clone(), value);
        }
        for value in snapshot.fle_profiles {
            self.fle_profiles.insert(value.id.clone(), value);
        }
        for value in snapshot.monitoring_subscriptions {
            self.monitoring_subscriptions
                .insert(value.distribution_id.clone(), value);
        }
        for value in snapshot.key_value_stores {
            self.key_value_stores.insert(value.name.clone(), value);
        }
        for value in snapshot.realtime_log_configs {
            self.realtime_log_configs.insert(value.name.clone(), value);
        }
        for (arn, tags) in snapshot.tags {
            self.tags.insert(arn, tags);
        }
    }
}

/// Serializable CloudFront store snapshot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudFrontStoreSnapshot {
    /// Distributions.
    pub distributions: Vec<Distribution>,
    /// Invalidations.
    pub invalidations: Vec<Invalidation>,
    /// Origin access controls.
    pub origin_access_controls: Vec<OriginAccessControl>,
    /// Legacy origin access identities.
    pub origin_access_identities: Vec<CloudFrontOriginAccessIdentity>,
    /// Cache policies.
    pub cache_policies: Vec<CachePolicy>,
    /// Origin request policies.
    pub origin_request_policies: Vec<OriginRequestPolicy>,
    /// Response headers policies.
    pub response_headers_policies: Vec<ResponseHeadersPolicy>,
    /// Key groups.
    pub key_groups: Vec<KeyGroup>,
    /// Public keys.
    pub public_keys: Vec<PublicKey>,
    /// CloudFront Functions.
    pub functions: Vec<CloudFrontFunction>,
    /// Field-level encryption configs.
    pub fle_configs: Vec<FieldLevelEncryption>,
    /// Field-level encryption profiles.
    pub fle_profiles: Vec<FieldLevelEncryptionProfile>,
    /// Monitoring subscriptions.
    pub monitoring_subscriptions: Vec<MonitoringSubscription>,
    /// Key-value stores.
    pub key_value_stores: Vec<KeyValueStore>,
    /// Realtime log configs.
    pub realtime_log_configs: Vec<RealtimeLogConfig>,
    /// Resource tags keyed by ARN.
    pub tags: Vec<(String, TagSet)>,
}

fn sorted_values<K, T, F, O>(map: &DashMap<K, T>, key_fn: F) -> Vec<T>
where
    K: Eq + Hash,
    T: Clone,
    F: Fn(&T) -> O,
    O: Ord,
{
    let mut values: Vec<T> = map.iter().map(|entry| entry.value().clone()).collect();
    values.sort_by_key(key_fn);
    values
}

fn sorted_key_values<T>(map: &DashMap<String, T>) -> Vec<(String, T)>
where
    T: Clone,
{
    let mut values: Vec<(String, T)> = map
        .iter()
        .map(|entry| (entry.key().clone(), entry.value().clone()))
        .collect();
    values.sort_by(|left, right| left.0.cmp(&right.0));
    values
}
