//! CloudFront provider — owns the store and implements every operation.
//!
//! Operations are grouped by resource kind. Each mutating op follows the same
//! skeleton:
//!
//! 1. Look up the resource (or fail with the appropriate `NoSuch*`).
//! 2. Check `If-Match` for Update/Delete (or fail `PreconditionFailed`).
//! 3. Mutate, bump ETag, update `last_modified_time`.
//! 4. Spawn propagation simulator if this is a distribution/invalidation.
//!
//! The ETag model is monotonic: every successful mutation generates a brand
//! new opaque token. ETags are never reused.

#![allow(clippy::too_many_lines)]
#![allow(clippy::too_many_arguments)]

use std::sync::Arc;

use chrono::Utc;
use rustack_cloudfront_model::{
    CachePolicy, CachePolicyConfig, CloudFrontError, CloudFrontFunction,
    CloudFrontOriginAccessIdentity, CloudFrontOriginAccessIdentityConfig, Distribution,
    DistributionConfig, FieldLevelEncryption, FieldLevelEncryptionConfig,
    FieldLevelEncryptionProfile, FieldLevelEncryptionProfileConfig, FunctionConfig,
    FunctionMetadata, Invalidation, InvalidationBatch, KeyGroup, KeyGroupConfig, KeyValueStore,
    MonitoringSubscription, OriginAccessControl, OriginAccessControlConfig, OriginRequestPolicy,
    OriginRequestPolicyConfig, PublicKey, PublicKeyConfig, RealtimeLogConfig, ResourceStatus,
    ResponseHeadersPolicy, ResponseHeadersPolicyConfig, Tag, TagSet,
};
use tracing::info;

use crate::{
    arn::{
        cache_policy_arn, distribution_arn, function_arn, key_group_arn, kvs_arn, oai_arn,
        origin_access_control_arn, origin_request_policy_arn, public_key_arn, realtime_log_arn,
        response_headers_policy_arn,
    },
    config::CloudFrontConfig,
    id_gen::{
        deterministic_id_with_prefix, distribution_domain_name, new_distribution_id, new_etag,
        new_id_with_prefix, new_invalidation_id, new_s3_canonical_user_id,
    },
    managed::{
        managed_cache_policies, managed_origin_request_policies, managed_response_headers_policies,
    },
    store::CloudFrontStore,
};

/// Main provider.
#[derive(Debug)]
pub struct RustackCloudFront {
    store: Arc<CloudFrontStore>,
    config: Arc<CloudFrontConfig>,
}

impl RustackCloudFront {
    /// Build a new provider with managed policies pre-seeded.
    #[must_use]
    pub fn new(config: CloudFrontConfig) -> Self {
        let store = CloudFrontStore::new();
        for p in managed_cache_policies() {
            store.cache_policies.insert(p.id.clone(), p);
        }
        for p in managed_origin_request_policies() {
            store.origin_request_policies.insert(p.id.clone(), p);
        }
        for p in managed_response_headers_policies() {
            store.response_headers_policies.insert(p.id.clone(), p);
        }
        Self {
            store,
            config: Arc::new(config),
        }
    }

    /// Shared store handle.
    #[must_use]
    pub fn store(&self) -> &Arc<CloudFrontStore> {
        &self.store
    }

    /// Runtime configuration.
    #[must_use]
    pub fn config(&self) -> &CloudFrontConfig {
        &self.config
    }

    // ---------------------------------------------------------------------
    // Distribution operations
    // ---------------------------------------------------------------------

    /// CreateDistribution / CreateDistributionWithTags.
    pub fn create_distribution(
        self: &Arc<Self>,
        config: DistributionConfig,
        tags: TagSet,
    ) -> Result<Distribution, CloudFrontError> {
        validate_distribution_config(&config)?;

        let id = if self.config.deterministic_ids {
            deterministic_id_with_prefix('E', &config.caller_reference)
        } else {
            new_distribution_id()
        };
        let arn = distribution_arn(&self.config.account_id, &id);
        let domain_name = distribution_domain_name(&id, &self.config.domain_suffix);
        let etag = new_etag();
        let dist = Distribution {
            id: id.clone(),
            arn: arn.clone(),
            status: ResourceStatus::InProgress,
            last_modified_time: Utc::now(),
            domain_name,
            in_progress_invalidation_batches: 0,
            active_trusted_signers_enabled: false,
            active_trusted_key_groups_enabled: false,
            config,
            tags: tags.clone(),
            etag,
            alias_icp_recordal: Vec::new(),
        };

        if !tags.is_empty() {
            self.store.tags.insert(arn.clone(), tags);
        }
        self.store.distributions.insert(id.clone(), dist.clone());
        self.spawn_distribution_deployment(&id);
        info!(distribution_id = %id, "created distribution");
        Ok(dist)
    }

    /// GetDistribution returns the full `Distribution` record.
    pub fn get_distribution(&self, id: &str) -> Result<Distribution, CloudFrontError> {
        self.store
            .distributions
            .get(id)
            .map(|r| r.value().clone())
            .ok_or_else(|| CloudFrontError::no_such_distribution(id))
    }

    /// UpdateDistribution.
    pub fn update_distribution(
        self: &Arc<Self>,
        id: &str,
        if_match: Option<&str>,
        new_config: DistributionConfig,
    ) -> Result<Distribution, CloudFrontError> {
        validate_distribution_config(&new_config)?;
        let mut entry = self
            .store
            .distributions
            .get_mut(id)
            .ok_or_else(|| CloudFrontError::no_such_distribution(id))?;
        check_if_match(if_match, &entry.etag)?;
        entry.config = new_config;
        entry.etag = new_etag();
        entry.status = ResourceStatus::InProgress;
        entry.last_modified_time = Utc::now();
        let clone = entry.value().clone();
        drop(entry);
        self.spawn_distribution_deployment(id);
        Ok(clone)
    }

    /// DeleteDistribution.
    pub fn delete_distribution(
        &self,
        id: &str,
        if_match: Option<&str>,
    ) -> Result<(), CloudFrontError> {
        let dist = self
            .store
            .distributions
            .get(id)
            .ok_or_else(|| CloudFrontError::no_such_distribution(id))?;
        check_if_match(if_match, &dist.etag)?;
        if dist.config.enabled {
            return Err(CloudFrontError::DistributionNotDisabled(format!(
                "Distribution {id} is enabled; disable it before deleting."
            )));
        }
        drop(dist);
        self.store.distributions.remove(id);
        self.store
            .tags
            .remove(&distribution_arn(&self.config.account_id, id));
        Ok(())
    }

    /// ListDistributions (unpaginated — Rustack scale makes this fine).
    pub fn list_distributions(&self) -> Vec<Distribution> {
        let mut v: Vec<_> = self
            .store
            .distributions
            .iter()
            .map(|e| e.value().clone())
            .collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    /// CopyDistribution: clone an existing distribution under a new ID.
    pub fn copy_distribution(
        self: &Arc<Self>,
        primary_id: &str,
        caller_reference: &str,
        staging: bool,
    ) -> Result<Distribution, CloudFrontError> {
        let primary = self
            .store
            .distributions
            .get(primary_id)
            .ok_or_else(|| CloudFrontError::no_such_distribution(primary_id))?;
        let mut new_cfg = primary.config.clone();
        new_cfg.caller_reference = caller_reference.to_owned();
        new_cfg.staging = staging;
        drop(primary);
        self.create_distribution(new_cfg, Vec::new())
    }

    // ---------------------------------------------------------------------
    // Invalidation operations
    // ---------------------------------------------------------------------

    /// CreateInvalidation.
    pub fn create_invalidation(
        self: &Arc<Self>,
        distribution_id: &str,
        batch: InvalidationBatch,
    ) -> Result<Invalidation, CloudFrontError> {
        if !self.store.distributions.contains_key(distribution_id) {
            return Err(CloudFrontError::no_such_distribution(distribution_id));
        }
        if batch.paths.is_empty() {
            return Err(CloudFrontError::InvalidArgument(
                "Invalidation batch must contain at least one path".to_owned(),
            ));
        }
        let id = if self.config.deterministic_ids {
            deterministic_id_with_prefix('I', &batch.caller_reference)
        } else {
            new_invalidation_id()
        };
        let inv = Invalidation {
            id: id.clone(),
            status: ResourceStatus::InProgress,
            create_time: Utc::now(),
            distribution_id: distribution_id.to_owned(),
            batch,
        };
        self.store
            .invalidations
            .insert((distribution_id.to_owned(), id.clone()), inv.clone());
        if let Some(mut d) = self.store.distributions.get_mut(distribution_id) {
            d.in_progress_invalidation_batches += 1;
        }
        self.spawn_invalidation_completion(distribution_id, &id);
        Ok(inv)
    }

    /// GetInvalidation.
    pub fn get_invalidation(
        &self,
        distribution_id: &str,
        invalidation_id: &str,
    ) -> Result<Invalidation, CloudFrontError> {
        self.store
            .invalidations
            .get(&(distribution_id.to_owned(), invalidation_id.to_owned()))
            .map(|r| r.value().clone())
            .ok_or_else(|| CloudFrontError::no_such_invalidation(invalidation_id))
    }

    /// ListInvalidations for a distribution.
    pub fn list_invalidations(&self, distribution_id: &str) -> Vec<Invalidation> {
        let mut v: Vec<_> = self
            .store
            .invalidations
            .iter()
            .filter(|e| e.key().0 == distribution_id)
            .map(|e| e.value().clone())
            .collect();
        v.sort_by(|a, b| b.create_time.cmp(&a.create_time));
        v
    }

    // ---------------------------------------------------------------------
    // Origin Access Control (OAC)
    // ---------------------------------------------------------------------

    /// CreateOriginAccessControl.
    pub fn create_oac(
        &self,
        cfg: OriginAccessControlConfig,
    ) -> Result<OriginAccessControl, CloudFrontError> {
        if cfg.name.is_empty() {
            return Err(CloudFrontError::InvalidArgument(
                "OriginAccessControl Name is required".to_owned(),
            ));
        }
        let id = new_id_with_prefix('E');
        let oac = OriginAccessControl {
            id: id.clone(),
            config: cfg,
            etag: new_etag(),
        };
        self.store.origin_access_controls.insert(id, oac.clone());
        Ok(oac)
    }

    /// GetOriginAccessControl.
    pub fn get_oac(&self, id: &str) -> Result<OriginAccessControl, CloudFrontError> {
        self.store
            .origin_access_controls
            .get(id)
            .map(|r| r.value().clone())
            .ok_or_else(|| CloudFrontError::no_such_origin_access_control(id))
    }

    /// UpdateOriginAccessControl.
    pub fn update_oac(
        &self,
        id: &str,
        if_match: Option<&str>,
        cfg: OriginAccessControlConfig,
    ) -> Result<OriginAccessControl, CloudFrontError> {
        let mut entry = self
            .store
            .origin_access_controls
            .get_mut(id)
            .ok_or_else(|| CloudFrontError::no_such_origin_access_control(id))?;
        check_if_match(if_match, &entry.etag)?;
        entry.config = cfg;
        entry.etag = new_etag();
        Ok(entry.value().clone())
    }

    /// DeleteOriginAccessControl.
    pub fn delete_oac(&self, id: &str, if_match: Option<&str>) -> Result<(), CloudFrontError> {
        let entry = self
            .store
            .origin_access_controls
            .get(id)
            .ok_or_else(|| CloudFrontError::no_such_origin_access_control(id))?;
        check_if_match(if_match, &entry.etag)?;
        drop(entry);
        self.store.origin_access_controls.remove(id);
        self.store
            .tags
            .remove(&origin_access_control_arn(&self.config.account_id, id));
        Ok(())
    }

    /// ListOriginAccessControls.
    pub fn list_oacs(&self) -> Vec<OriginAccessControl> {
        let mut v: Vec<_> = self
            .store
            .origin_access_controls
            .iter()
            .map(|e| e.value().clone())
            .collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    // ---------------------------------------------------------------------
    // Cloudfront Origin Access Identity (OAI, legacy)
    // ---------------------------------------------------------------------

    /// CreateCloudFrontOriginAccessIdentity.
    pub fn create_oai(
        &self,
        cfg: CloudFrontOriginAccessIdentityConfig,
    ) -> Result<CloudFrontOriginAccessIdentity, CloudFrontError> {
        let id = new_id_with_prefix('E');
        let oai = CloudFrontOriginAccessIdentity {
            id: id.clone(),
            s3_canonical_user_id: new_s3_canonical_user_id(),
            config: cfg,
            etag: new_etag(),
        };
        self.store.origin_access_identities.insert(id, oai.clone());
        Ok(oai)
    }

    /// GetCloudFrontOriginAccessIdentity.
    pub fn get_oai(&self, id: &str) -> Result<CloudFrontOriginAccessIdentity, CloudFrontError> {
        self.store
            .origin_access_identities
            .get(id)
            .map(|r| r.value().clone())
            .ok_or_else(|| CloudFrontError::no_such_oai(id))
    }

    /// UpdateCloudFrontOriginAccessIdentity.
    pub fn update_oai(
        &self,
        id: &str,
        if_match: Option<&str>,
        cfg: CloudFrontOriginAccessIdentityConfig,
    ) -> Result<CloudFrontOriginAccessIdentity, CloudFrontError> {
        let mut entry = self
            .store
            .origin_access_identities
            .get_mut(id)
            .ok_or_else(|| CloudFrontError::no_such_oai(id))?;
        check_if_match(if_match, &entry.etag)?;
        entry.config = cfg;
        entry.etag = new_etag();
        Ok(entry.value().clone())
    }

    /// DeleteCloudFrontOriginAccessIdentity.
    pub fn delete_oai(&self, id: &str, if_match: Option<&str>) -> Result<(), CloudFrontError> {
        let entry = self
            .store
            .origin_access_identities
            .get(id)
            .ok_or_else(|| CloudFrontError::no_such_oai(id))?;
        check_if_match(if_match, &entry.etag)?;
        drop(entry);
        self.store.origin_access_identities.remove(id);
        self.store
            .tags
            .remove(&oai_arn(&self.config.account_id, id));
        Ok(())
    }

    /// ListCloudFrontOriginAccessIdentities.
    pub fn list_oais(&self) -> Vec<CloudFrontOriginAccessIdentity> {
        let mut v: Vec<_> = self
            .store
            .origin_access_identities
            .iter()
            .map(|e| e.value().clone())
            .collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    // ---------------------------------------------------------------------
    // Cache policy
    // ---------------------------------------------------------------------

    /// CreateCachePolicy.
    pub fn create_cache_policy(
        &self,
        cfg: CachePolicyConfig,
    ) -> Result<CachePolicy, CloudFrontError> {
        if cfg.name.is_empty() {
            return Err(CloudFrontError::InvalidArgument(
                "CachePolicyConfig.Name is required".to_owned(),
            ));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let p = CachePolicy {
            id: id.clone(),
            last_modified_time: Utc::now(),
            config: cfg,
            etag: new_etag(),
            managed: false,
        };
        self.store.cache_policies.insert(id, p.clone());
        Ok(p)
    }

    /// GetCachePolicy.
    pub fn get_cache_policy(&self, id: &str) -> Result<CachePolicy, CloudFrontError> {
        self.store
            .cache_policies
            .get(id)
            .map(|r| r.value().clone())
            .ok_or_else(|| CloudFrontError::no_such_cache_policy(id))
    }

    /// UpdateCachePolicy — managed policies are immutable.
    pub fn update_cache_policy(
        &self,
        id: &str,
        if_match: Option<&str>,
        cfg: CachePolicyConfig,
    ) -> Result<CachePolicy, CloudFrontError> {
        let mut entry = self
            .store
            .cache_policies
            .get_mut(id)
            .ok_or_else(|| CloudFrontError::no_such_cache_policy(id))?;
        if entry.managed {
            return Err(CloudFrontError::AccessDenied(
                "AWS-managed cache policies cannot be modified".to_owned(),
            ));
        }
        check_if_match(if_match, &entry.etag)?;
        entry.config = cfg;
        entry.etag = new_etag();
        entry.last_modified_time = Utc::now();
        Ok(entry.value().clone())
    }

    /// DeleteCachePolicy.
    pub fn delete_cache_policy(
        &self,
        id: &str,
        if_match: Option<&str>,
    ) -> Result<(), CloudFrontError> {
        let entry = self
            .store
            .cache_policies
            .get(id)
            .ok_or_else(|| CloudFrontError::no_such_cache_policy(id))?;
        if entry.managed {
            return Err(CloudFrontError::AccessDenied(
                "AWS-managed cache policies cannot be deleted".to_owned(),
            ));
        }
        check_if_match(if_match, &entry.etag)?;
        drop(entry);
        self.store.cache_policies.remove(id);
        self.store
            .tags
            .remove(&cache_policy_arn(&self.config.account_id, id));
        Ok(())
    }

    /// ListCachePolicies.
    pub fn list_cache_policies(&self) -> Vec<CachePolicy> {
        let mut v: Vec<_> = self
            .store
            .cache_policies
            .iter()
            .map(|e| e.value().clone())
            .collect();
        v.sort_by(|a, b| a.config.name.cmp(&b.config.name));
        v
    }

    // ---------------------------------------------------------------------
    // Origin request policy
    // ---------------------------------------------------------------------

    /// CreateOriginRequestPolicy.
    pub fn create_origin_request_policy(
        &self,
        cfg: OriginRequestPolicyConfig,
    ) -> Result<OriginRequestPolicy, CloudFrontError> {
        if cfg.name.is_empty() {
            return Err(CloudFrontError::InvalidArgument(
                "OriginRequestPolicyConfig.Name is required".to_owned(),
            ));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let p = OriginRequestPolicy {
            id: id.clone(),
            last_modified_time: Utc::now(),
            config: cfg,
            etag: new_etag(),
            managed: false,
        };
        self.store.origin_request_policies.insert(id, p.clone());
        Ok(p)
    }

    /// GetOriginRequestPolicy.
    pub fn get_origin_request_policy(
        &self,
        id: &str,
    ) -> Result<OriginRequestPolicy, CloudFrontError> {
        self.store
            .origin_request_policies
            .get(id)
            .map(|r| r.value().clone())
            .ok_or_else(|| CloudFrontError::no_such_origin_request_policy(id))
    }

    /// UpdateOriginRequestPolicy.
    pub fn update_origin_request_policy(
        &self,
        id: &str,
        if_match: Option<&str>,
        cfg: OriginRequestPolicyConfig,
    ) -> Result<OriginRequestPolicy, CloudFrontError> {
        let mut entry = self
            .store
            .origin_request_policies
            .get_mut(id)
            .ok_or_else(|| CloudFrontError::no_such_origin_request_policy(id))?;
        if entry.managed {
            return Err(CloudFrontError::AccessDenied(
                "AWS-managed origin request policies cannot be modified".to_owned(),
            ));
        }
        check_if_match(if_match, &entry.etag)?;
        entry.config = cfg;
        entry.etag = new_etag();
        entry.last_modified_time = Utc::now();
        Ok(entry.value().clone())
    }

    /// DeleteOriginRequestPolicy.
    pub fn delete_origin_request_policy(
        &self,
        id: &str,
        if_match: Option<&str>,
    ) -> Result<(), CloudFrontError> {
        let entry = self
            .store
            .origin_request_policies
            .get(id)
            .ok_or_else(|| CloudFrontError::no_such_origin_request_policy(id))?;
        if entry.managed {
            return Err(CloudFrontError::AccessDenied(
                "AWS-managed origin request policies cannot be deleted".to_owned(),
            ));
        }
        check_if_match(if_match, &entry.etag)?;
        drop(entry);
        self.store.origin_request_policies.remove(id);
        self.store
            .tags
            .remove(&origin_request_policy_arn(&self.config.account_id, id));
        Ok(())
    }

    /// ListOriginRequestPolicies.
    pub fn list_origin_request_policies(&self) -> Vec<OriginRequestPolicy> {
        let mut v: Vec<_> = self
            .store
            .origin_request_policies
            .iter()
            .map(|e| e.value().clone())
            .collect();
        v.sort_by(|a, b| a.config.name.cmp(&b.config.name));
        v
    }

    // ---------------------------------------------------------------------
    // Response headers policy
    // ---------------------------------------------------------------------

    /// CreateResponseHeadersPolicy.
    pub fn create_response_headers_policy(
        &self,
        cfg: ResponseHeadersPolicyConfig,
    ) -> Result<ResponseHeadersPolicy, CloudFrontError> {
        if cfg.name.is_empty() {
            return Err(CloudFrontError::InvalidArgument(
                "ResponseHeadersPolicyConfig.Name is required".to_owned(),
            ));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let p = ResponseHeadersPolicy {
            id: id.clone(),
            last_modified_time: Utc::now(),
            config: cfg,
            etag: new_etag(),
            managed: false,
        };
        self.store.response_headers_policies.insert(id, p.clone());
        Ok(p)
    }

    /// GetResponseHeadersPolicy.
    pub fn get_response_headers_policy(
        &self,
        id: &str,
    ) -> Result<ResponseHeadersPolicy, CloudFrontError> {
        self.store
            .response_headers_policies
            .get(id)
            .map(|r| r.value().clone())
            .ok_or_else(|| CloudFrontError::no_such_response_headers_policy(id))
    }

    /// UpdateResponseHeadersPolicy.
    pub fn update_response_headers_policy(
        &self,
        id: &str,
        if_match: Option<&str>,
        cfg: ResponseHeadersPolicyConfig,
    ) -> Result<ResponseHeadersPolicy, CloudFrontError> {
        let mut entry = self
            .store
            .response_headers_policies
            .get_mut(id)
            .ok_or_else(|| CloudFrontError::no_such_response_headers_policy(id))?;
        if entry.managed {
            return Err(CloudFrontError::AccessDenied(
                "AWS-managed response headers policies cannot be modified".to_owned(),
            ));
        }
        check_if_match(if_match, &entry.etag)?;
        entry.config = cfg;
        entry.etag = new_etag();
        entry.last_modified_time = Utc::now();
        Ok(entry.value().clone())
    }

    /// DeleteResponseHeadersPolicy.
    pub fn delete_response_headers_policy(
        &self,
        id: &str,
        if_match: Option<&str>,
    ) -> Result<(), CloudFrontError> {
        let entry = self
            .store
            .response_headers_policies
            .get(id)
            .ok_or_else(|| CloudFrontError::no_such_response_headers_policy(id))?;
        if entry.managed {
            return Err(CloudFrontError::AccessDenied(
                "AWS-managed response headers policies cannot be deleted".to_owned(),
            ));
        }
        check_if_match(if_match, &entry.etag)?;
        drop(entry);
        self.store.response_headers_policies.remove(id);
        self.store
            .tags
            .remove(&response_headers_policy_arn(&self.config.account_id, id));
        Ok(())
    }

    /// ListResponseHeadersPolicies.
    pub fn list_response_headers_policies(&self) -> Vec<ResponseHeadersPolicy> {
        let mut v: Vec<_> = self
            .store
            .response_headers_policies
            .iter()
            .map(|e| e.value().clone())
            .collect();
        v.sort_by(|a, b| a.config.name.cmp(&b.config.name));
        v
    }

    // ---------------------------------------------------------------------
    // Key group / public key
    // ---------------------------------------------------------------------

    /// CreateKeyGroup.
    pub fn create_key_group(&self, cfg: KeyGroupConfig) -> Result<KeyGroup, CloudFrontError> {
        if cfg.name.is_empty() {
            return Err(CloudFrontError::InvalidArgument(
                "KeyGroupConfig.Name is required".to_owned(),
            ));
        }
        let id = new_id_with_prefix('K');
        let kg = KeyGroup {
            id: id.clone(),
            last_modified_time: Utc::now(),
            config: cfg,
            etag: new_etag(),
        };
        self.store.key_groups.insert(id, kg.clone());
        Ok(kg)
    }

    /// GetKeyGroup.
    pub fn get_key_group(&self, id: &str) -> Result<KeyGroup, CloudFrontError> {
        self.store
            .key_groups
            .get(id)
            .map(|r| r.value().clone())
            .ok_or_else(|| CloudFrontError::no_such_resource("KeyGroup", id))
    }

    /// UpdateKeyGroup.
    pub fn update_key_group(
        &self,
        id: &str,
        if_match: Option<&str>,
        cfg: KeyGroupConfig,
    ) -> Result<KeyGroup, CloudFrontError> {
        let mut entry = self
            .store
            .key_groups
            .get_mut(id)
            .ok_or_else(|| CloudFrontError::no_such_resource("KeyGroup", id))?;
        check_if_match(if_match, &entry.etag)?;
        entry.config = cfg;
        entry.etag = new_etag();
        entry.last_modified_time = Utc::now();
        Ok(entry.value().clone())
    }

    /// DeleteKeyGroup.
    pub fn delete_key_group(
        &self,
        id: &str,
        if_match: Option<&str>,
    ) -> Result<(), CloudFrontError> {
        let entry = self
            .store
            .key_groups
            .get(id)
            .ok_or_else(|| CloudFrontError::no_such_resource("KeyGroup", id))?;
        check_if_match(if_match, &entry.etag)?;
        drop(entry);
        self.store.key_groups.remove(id);
        self.store
            .tags
            .remove(&key_group_arn(&self.config.account_id, id));
        Ok(())
    }

    /// ListKeyGroups.
    pub fn list_key_groups(&self) -> Vec<KeyGroup> {
        let mut v: Vec<_> = self
            .store
            .key_groups
            .iter()
            .map(|e| e.value().clone())
            .collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    /// CreatePublicKey.
    pub fn create_public_key(&self, cfg: PublicKeyConfig) -> Result<PublicKey, CloudFrontError> {
        if cfg.name.is_empty() || cfg.encoded_key.is_empty() {
            return Err(CloudFrontError::InvalidArgument(
                "PublicKeyConfig Name and EncodedKey are required".to_owned(),
            ));
        }
        let id = new_id_with_prefix('K');
        let pk = PublicKey {
            id: id.clone(),
            created_time: Utc::now(),
            config: cfg,
            etag: new_etag(),
        };
        self.store.public_keys.insert(id, pk.clone());
        Ok(pk)
    }

    /// GetPublicKey.
    pub fn get_public_key(&self, id: &str) -> Result<PublicKey, CloudFrontError> {
        self.store
            .public_keys
            .get(id)
            .map(|r| r.value().clone())
            .ok_or_else(|| CloudFrontError::no_such_public_key(id))
    }

    /// UpdatePublicKey.
    pub fn update_public_key(
        &self,
        id: &str,
        if_match: Option<&str>,
        cfg: PublicKeyConfig,
    ) -> Result<PublicKey, CloudFrontError> {
        let mut entry = self
            .store
            .public_keys
            .get_mut(id)
            .ok_or_else(|| CloudFrontError::no_such_public_key(id))?;
        check_if_match(if_match, &entry.etag)?;
        entry.config = cfg;
        entry.etag = new_etag();
        Ok(entry.value().clone())
    }

    /// DeletePublicKey.
    pub fn delete_public_key(
        &self,
        id: &str,
        if_match: Option<&str>,
    ) -> Result<(), CloudFrontError> {
        let entry = self
            .store
            .public_keys
            .get(id)
            .ok_or_else(|| CloudFrontError::no_such_public_key(id))?;
        check_if_match(if_match, &entry.etag)?;
        drop(entry);
        self.store.public_keys.remove(id);
        self.store
            .tags
            .remove(&public_key_arn(&self.config.account_id, id));
        Ok(())
    }

    /// ListPublicKeys.
    pub fn list_public_keys(&self) -> Vec<PublicKey> {
        let mut v: Vec<_> = self
            .store
            .public_keys
            .iter()
            .map(|e| e.value().clone())
            .collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    // ---------------------------------------------------------------------
    // Functions (store-only; no JS runtime)
    // ---------------------------------------------------------------------

    /// CreateFunction.
    pub fn create_function(
        &self,
        name: String,
        cfg: FunctionConfig,
        code: Vec<u8>,
    ) -> Result<CloudFrontFunction, CloudFrontError> {
        if name.is_empty() {
            return Err(CloudFrontError::InvalidArgument(
                "Function Name is required".to_owned(),
            ));
        }
        if self.store.functions.contains_key(&name) {
            return Err(CloudFrontError::AlreadyExists {
                code: "FunctionAlreadyExists",
                message: format!("Function {name} already exists"),
            });
        }
        let arn = function_arn(&self.config.account_id, &name);
        let now = Utc::now();
        let f = CloudFrontFunction {
            name: name.clone(),
            arn: arn.clone(),
            last_modified_time: now,
            stage: "DEVELOPMENT".to_owned(),
            metadata: FunctionMetadata {
                function_arn: arn,
                stage: "DEVELOPMENT".to_owned(),
                created_time: now,
                last_modified_time: now,
            },
            config: cfg,
            code,
            etag: new_etag(),
            status: "UNPUBLISHED".to_owned(),
        };
        self.store.functions.insert(name, f.clone());
        Ok(f)
    }

    /// DescribeFunction / GetFunction (code is the distinction — same storage).
    pub fn get_function(&self, name: &str) -> Result<CloudFrontFunction, CloudFrontError> {
        self.store
            .functions
            .get(name)
            .map(|r| r.value().clone())
            .ok_or_else(|| CloudFrontError::no_such_resource("Function", name))
    }

    /// UpdateFunction.
    pub fn update_function(
        &self,
        name: &str,
        if_match: Option<&str>,
        cfg: FunctionConfig,
        code: Vec<u8>,
    ) -> Result<CloudFrontFunction, CloudFrontError> {
        let mut entry = self
            .store
            .functions
            .get_mut(name)
            .ok_or_else(|| CloudFrontError::no_such_resource("Function", name))?;
        check_if_match(if_match, &entry.etag)?;
        entry.config = cfg;
        entry.code = code;
        entry.etag = new_etag();
        entry.last_modified_time = Utc::now();
        Ok(entry.value().clone())
    }

    /// DeleteFunction.
    pub fn delete_function(
        &self,
        name: &str,
        if_match: Option<&str>,
    ) -> Result<(), CloudFrontError> {
        let entry = self
            .store
            .functions
            .get(name)
            .ok_or_else(|| CloudFrontError::no_such_resource("Function", name))?;
        check_if_match(if_match, &entry.etag)?;
        drop(entry);
        self.store.functions.remove(name);
        Ok(())
    }

    /// PublishFunction: flips stage from DEVELOPMENT to LIVE.
    pub fn publish_function(
        &self,
        name: &str,
        if_match: Option<&str>,
    ) -> Result<CloudFrontFunction, CloudFrontError> {
        let mut entry = self
            .store
            .functions
            .get_mut(name)
            .ok_or_else(|| CloudFrontError::no_such_resource("Function", name))?;
        check_if_match(if_match, &entry.etag)?;
        entry.stage = "LIVE".to_owned();
        entry.metadata.stage = "LIVE".to_owned();
        entry.status = "PUBLISHED".to_owned();
        entry.etag = new_etag();
        Ok(entry.value().clone())
    }

    /// TestFunction: returns canned success.
    pub fn test_function(
        &self,
        name: &str,
        event_object: &[u8],
    ) -> Result<(Vec<u8>, String), CloudFrontError> {
        let _ = self.get_function(name)?;
        let result = br#"{"status":"success","testStatus":"OK"}"#.to_vec();
        let compute_util = format!("compute_utilization_percent={}", event_object.len());
        Ok((result, compute_util))
    }

    /// ListFunctions.
    pub fn list_functions(&self) -> Vec<CloudFrontFunction> {
        let mut v: Vec<_> = self
            .store
            .functions
            .iter()
            .map(|e| e.value().clone())
            .collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }

    // ---------------------------------------------------------------------
    // Field-Level Encryption (store-only)
    // ---------------------------------------------------------------------

    /// CreateFieldLevelEncryptionConfig.
    pub fn create_fle_config(
        &self,
        cfg: FieldLevelEncryptionConfig,
    ) -> Result<FieldLevelEncryption, CloudFrontError> {
        let id = new_id_with_prefix('F');
        let f = FieldLevelEncryption {
            id: id.clone(),
            last_modified_time: Utc::now(),
            config: cfg,
            etag: new_etag(),
        };
        self.store.fle_configs.insert(id, f.clone());
        Ok(f)
    }

    /// GetFieldLevelEncryption / GetFieldLevelEncryptionConfig.
    pub fn get_fle_config(&self, id: &str) -> Result<FieldLevelEncryption, CloudFrontError> {
        self.store
            .fle_configs
            .get(id)
            .map(|r| r.value().clone())
            .ok_or_else(|| CloudFrontError::no_such_resource("FieldLevelEncryption", id))
    }

    /// UpdateFieldLevelEncryptionConfig.
    pub fn update_fle_config(
        &self,
        id: &str,
        if_match: Option<&str>,
        cfg: FieldLevelEncryptionConfig,
    ) -> Result<FieldLevelEncryption, CloudFrontError> {
        let mut entry = self
            .store
            .fle_configs
            .get_mut(id)
            .ok_or_else(|| CloudFrontError::no_such_resource("FieldLevelEncryption", id))?;
        check_if_match(if_match, &entry.etag)?;
        entry.config = cfg;
        entry.etag = new_etag();
        entry.last_modified_time = Utc::now();
        Ok(entry.value().clone())
    }

    /// DeleteFieldLevelEncryptionConfig.
    pub fn delete_fle_config(
        &self,
        id: &str,
        if_match: Option<&str>,
    ) -> Result<(), CloudFrontError> {
        let entry = self
            .store
            .fle_configs
            .get(id)
            .ok_or_else(|| CloudFrontError::no_such_resource("FieldLevelEncryption", id))?;
        check_if_match(if_match, &entry.etag)?;
        drop(entry);
        self.store.fle_configs.remove(id);
        Ok(())
    }

    /// ListFieldLevelEncryptionConfigs.
    pub fn list_fle_configs(&self) -> Vec<FieldLevelEncryption> {
        self.store
            .fle_configs
            .iter()
            .map(|e| e.value().clone())
            .collect()
    }

    /// CreateFieldLevelEncryptionProfile.
    pub fn create_fle_profile(
        &self,
        cfg: FieldLevelEncryptionProfileConfig,
    ) -> Result<FieldLevelEncryptionProfile, CloudFrontError> {
        let id = new_id_with_prefix('P');
        let p = FieldLevelEncryptionProfile {
            id: id.clone(),
            last_modified_time: Utc::now(),
            config: cfg,
            etag: new_etag(),
        };
        self.store.fle_profiles.insert(id, p.clone());
        Ok(p)
    }

    /// GetFieldLevelEncryptionProfile.
    pub fn get_fle_profile(
        &self,
        id: &str,
    ) -> Result<FieldLevelEncryptionProfile, CloudFrontError> {
        self.store
            .fle_profiles
            .get(id)
            .map(|r| r.value().clone())
            .ok_or_else(|| CloudFrontError::no_such_resource("FieldLevelEncryptionProfile", id))
    }

    /// UpdateFieldLevelEncryptionProfile.
    pub fn update_fle_profile(
        &self,
        id: &str,
        if_match: Option<&str>,
        cfg: FieldLevelEncryptionProfileConfig,
    ) -> Result<FieldLevelEncryptionProfile, CloudFrontError> {
        let mut entry =
            self.store.fle_profiles.get_mut(id).ok_or_else(|| {
                CloudFrontError::no_such_resource("FieldLevelEncryptionProfile", id)
            })?;
        check_if_match(if_match, &entry.etag)?;
        entry.config = cfg;
        entry.etag = new_etag();
        entry.last_modified_time = Utc::now();
        Ok(entry.value().clone())
    }

    /// DeleteFieldLevelEncryptionProfile.
    pub fn delete_fle_profile(
        &self,
        id: &str,
        if_match: Option<&str>,
    ) -> Result<(), CloudFrontError> {
        let entry =
            self.store.fle_profiles.get(id).ok_or_else(|| {
                CloudFrontError::no_such_resource("FieldLevelEncryptionProfile", id)
            })?;
        check_if_match(if_match, &entry.etag)?;
        drop(entry);
        self.store.fle_profiles.remove(id);
        Ok(())
    }

    /// ListFieldLevelEncryptionProfiles.
    pub fn list_fle_profiles(&self) -> Vec<FieldLevelEncryptionProfile> {
        self.store
            .fle_profiles
            .iter()
            .map(|e| e.value().clone())
            .collect()
    }

    // ---------------------------------------------------------------------
    // Monitoring subscription
    // ---------------------------------------------------------------------

    /// CreateMonitoringSubscription.
    pub fn create_monitoring_subscription(
        &self,
        distribution_id: &str,
        enabled: bool,
    ) -> Result<MonitoringSubscription, CloudFrontError> {
        if !self.store.distributions.contains_key(distribution_id) {
            return Err(CloudFrontError::no_such_distribution(distribution_id));
        }
        let sub = MonitoringSubscription {
            distribution_id: distribution_id.to_owned(),
            realtime_metrics_subscription_status: if enabled {
                "Enabled".to_owned()
            } else {
                "Disabled".to_owned()
            },
        };
        self.store
            .monitoring_subscriptions
            .insert(distribution_id.to_owned(), sub.clone());
        Ok(sub)
    }

    /// GetMonitoringSubscription.
    pub fn get_monitoring_subscription(
        &self,
        distribution_id: &str,
    ) -> Result<MonitoringSubscription, CloudFrontError> {
        self.store
            .monitoring_subscriptions
            .get(distribution_id)
            .map(|r| r.value().clone())
            .ok_or_else(|| {
                CloudFrontError::no_such_resource("MonitoringSubscription", distribution_id)
            })
    }

    /// DeleteMonitoringSubscription.
    pub fn delete_monitoring_subscription(
        &self,
        distribution_id: &str,
    ) -> Result<(), CloudFrontError> {
        if self
            .store
            .monitoring_subscriptions
            .remove(distribution_id)
            .is_none()
        {
            return Err(CloudFrontError::no_such_resource(
                "MonitoringSubscription",
                distribution_id,
            ));
        }
        Ok(())
    }

    // ---------------------------------------------------------------------
    // KeyValueStore
    // ---------------------------------------------------------------------

    /// CreateKeyValueStore.
    pub fn create_kvs(
        &self,
        name: String,
        comment: String,
    ) -> Result<KeyValueStore, CloudFrontError> {
        let id = uuid::Uuid::new_v4().to_string();
        let arn = kvs_arn(&self.config.account_id, &id);
        let kvs = KeyValueStore {
            id: id.clone(),
            name,
            arn,
            comment,
            status: "PROVISIONING".to_owned(),
            last_modified_time: Utc::now(),
            etag: new_etag(),
        };
        self.store.key_value_stores.insert(id, kvs.clone());
        Ok(kvs)
    }

    /// DescribeKeyValueStore.
    pub fn get_kvs(&self, id: &str) -> Result<KeyValueStore, CloudFrontError> {
        self.store
            .key_value_stores
            .get(id)
            .map(|r| r.value().clone())
            .ok_or_else(|| CloudFrontError::no_such_resource("KeyValueStore", id))
    }

    /// UpdateKeyValueStore.
    pub fn update_kvs(
        &self,
        id: &str,
        if_match: Option<&str>,
        comment: String,
    ) -> Result<KeyValueStore, CloudFrontError> {
        let mut entry = self
            .store
            .key_value_stores
            .get_mut(id)
            .ok_or_else(|| CloudFrontError::no_such_resource("KeyValueStore", id))?;
        check_if_match(if_match, &entry.etag)?;
        entry.comment = comment;
        entry.etag = new_etag();
        entry.last_modified_time = Utc::now();
        Ok(entry.value().clone())
    }

    /// DeleteKeyValueStore.
    pub fn delete_kvs(&self, id: &str, if_match: Option<&str>) -> Result<(), CloudFrontError> {
        let entry = self
            .store
            .key_value_stores
            .get(id)
            .ok_or_else(|| CloudFrontError::no_such_resource("KeyValueStore", id))?;
        check_if_match(if_match, &entry.etag)?;
        drop(entry);
        self.store.key_value_stores.remove(id);
        Ok(())
    }

    /// ListKeyValueStores.
    pub fn list_kvs(&self) -> Vec<KeyValueStore> {
        self.store
            .key_value_stores
            .iter()
            .map(|e| e.value().clone())
            .collect()
    }

    // ---------------------------------------------------------------------
    // Realtime log configs
    // ---------------------------------------------------------------------

    /// CreateRealtimeLogConfig.
    pub fn create_realtime_log_config(
        &self,
        cfg: RealtimeLogConfig,
    ) -> Result<RealtimeLogConfig, CloudFrontError> {
        if cfg.name.is_empty() {
            return Err(CloudFrontError::InvalidArgument(
                "RealtimeLogConfig Name is required".to_owned(),
            ));
        }
        let mut final_cfg = cfg;
        if final_cfg.arn.is_empty() {
            final_cfg.arn = realtime_log_arn(&self.config.account_id, &final_cfg.name);
        }
        self.store
            .realtime_log_configs
            .insert(final_cfg.name.clone(), final_cfg.clone());
        Ok(final_cfg)
    }

    /// GetRealtimeLogConfig.
    pub fn get_realtime_log_config(
        &self,
        name: &str,
    ) -> Result<RealtimeLogConfig, CloudFrontError> {
        self.store
            .realtime_log_configs
            .get(name)
            .map(|r| r.value().clone())
            .ok_or_else(|| CloudFrontError::no_such_resource("RealtimeLogConfig", name))
    }

    /// UpdateRealtimeLogConfig.
    pub fn update_realtime_log_config(
        &self,
        cfg: RealtimeLogConfig,
    ) -> Result<RealtimeLogConfig, CloudFrontError> {
        if !self.store.realtime_log_configs.contains_key(&cfg.name) {
            return Err(CloudFrontError::no_such_resource(
                "RealtimeLogConfig",
                &cfg.name,
            ));
        }
        self.store
            .realtime_log_configs
            .insert(cfg.name.clone(), cfg.clone());
        Ok(cfg)
    }

    /// DeleteRealtimeLogConfig.
    pub fn delete_realtime_log_config(&self, name: &str) -> Result<(), CloudFrontError> {
        if self.store.realtime_log_configs.remove(name).is_none() {
            return Err(CloudFrontError::no_such_resource("RealtimeLogConfig", name));
        }
        Ok(())
    }

    /// ListRealtimeLogConfigs.
    pub fn list_realtime_log_configs(&self) -> Vec<RealtimeLogConfig> {
        self.store
            .realtime_log_configs
            .iter()
            .map(|e| e.value().clone())
            .collect()
    }

    // ---------------------------------------------------------------------
    // Tagging (uniform across all taggable resources)
    // ---------------------------------------------------------------------

    /// TagResource.
    pub fn tag_resource(&self, arn: &str, new_tags: &[Tag]) -> Result<(), CloudFrontError> {
        let mut entry = self.store.tags.entry(arn.to_owned()).or_default();
        merge_tags(entry.value_mut(), new_tags);
        Ok(())
    }

    /// UntagResource.
    pub fn untag_resource(&self, arn: &str, keys: &[String]) -> Result<(), CloudFrontError> {
        if let Some(mut entry) = self.store.tags.get_mut(arn) {
            entry.retain(|t| !keys.iter().any(|k| k == &t.key));
        }
        Ok(())
    }

    /// ListTagsForResource.
    pub fn list_tags_for_resource(&self, arn: &str) -> Result<TagSet, CloudFrontError> {
        Ok(self
            .store
            .tags
            .get(arn)
            .map(|r| r.value().clone())
            .unwrap_or_default())
    }

    // ---------------------------------------------------------------------
    // Lifecycle simulators
    // ---------------------------------------------------------------------

    fn spawn_distribution_deployment(self: &Arc<Self>, id: &str) {
        let delay = self.config.distribution_propagation;
        let id = id.to_owned();
        let provider = Arc::clone(self);
        tokio::spawn(async move {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            if let Some(mut d) = provider.store.distributions.get_mut(&id) {
                d.status = ResourceStatus::Deployed;
            }
        });
    }

    fn spawn_invalidation_completion(
        self: &Arc<Self>,
        distribution_id: &str,
        invalidation_id: &str,
    ) {
        let delay = self.config.invalidation_propagation;
        let dist_id = distribution_id.to_owned();
        let inv_id = invalidation_id.to_owned();
        let provider = Arc::clone(self);
        tokio::spawn(async move {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            if let Some(mut inv) = provider
                .store
                .invalidations
                .get_mut(&(dist_id.clone(), inv_id))
            {
                inv.status = ResourceStatus::Completed;
            }
            if let Some(mut d) = provider.store.distributions.get_mut(&dist_id) {
                d.in_progress_invalidation_batches =
                    (d.in_progress_invalidation_batches - 1).max(0);
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

fn validate_distribution_config(cfg: &DistributionConfig) -> Result<(), CloudFrontError> {
    if cfg.caller_reference.is_empty() {
        return Err(CloudFrontError::MissingArgument(
            "CallerReference is required".to_owned(),
        ));
    }
    if cfg.origins.is_empty() {
        return Err(CloudFrontError::MissingArgument(
            "At least one Origin is required".to_owned(),
        ));
    }
    if cfg.default_cache_behavior.target_origin_id.is_empty() {
        return Err(CloudFrontError::MissingArgument(
            "DefaultCacheBehavior.TargetOriginId is required".to_owned(),
        ));
    }
    let origin_ids: std::collections::HashSet<&str> =
        cfg.origins.iter().map(|o| o.id.as_str()).collect();
    if !origin_ids.contains(cfg.default_cache_behavior.target_origin_id.as_str())
        && !cfg
            .origin_groups
            .iter()
            .any(|g| g.id == cfg.default_cache_behavior.target_origin_id)
    {
        return Err(CloudFrontError::MalformedInput(format!(
            "DefaultCacheBehavior.TargetOriginId {} does not match any Origin.Id",
            cfg.default_cache_behavior.target_origin_id
        )));
    }
    for cb in &cfg.cache_behaviors {
        if !origin_ids.contains(cb.target_origin_id.as_str())
            && !cfg
                .origin_groups
                .iter()
                .any(|g| g.id == cb.target_origin_id)
        {
            return Err(CloudFrontError::MalformedInput(format!(
                "CacheBehavior.TargetOriginId {} does not match any Origin.Id",
                cb.target_origin_id
            )));
        }
    }
    Ok(())
}

fn check_if_match(supplied: Option<&str>, current: &str) -> Result<(), CloudFrontError> {
    match supplied {
        None | Some("") => Err(CloudFrontError::InvalidIfMatchVersion(
            "The If-Match version is missing".to_owned(),
        )),
        Some(v) if v == current => Ok(()),
        Some(_) => Err(CloudFrontError::PreconditionFailed(
            "The If-Match version is not valid for the resource".to_owned(),
        )),
    }
}

fn merge_tags(existing: &mut TagSet, new_tags: &[Tag]) {
    for t in new_tags {
        if let Some(existing_tag) = existing.iter_mut().find(|e| e.key == t.key) {
            existing_tag.value = t.value.clone();
        } else {
            existing.push(t.clone());
        }
    }
}
