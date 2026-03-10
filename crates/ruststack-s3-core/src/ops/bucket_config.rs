//! Bucket configuration operation handlers.
//!
//! Implements versioning, encryption, CORS, lifecycle, policy, tagging,
//! notification, logging, public access block, ownership controls,
//! object lock, accelerate, request payment, website, ACL, and
//! policy status operations.

use ruststack_s3_model::error::S3Error;
use ruststack_s3_model::input::{
    DeleteBucketCorsInput, DeleteBucketEncryptionInput, DeleteBucketLifecycleInput,
    DeleteBucketOwnershipControlsInput, DeleteBucketPolicyInput, DeleteBucketTaggingInput,
    DeleteBucketWebsiteInput, DeletePublicAccessBlockInput, GetBucketAccelerateConfigurationInput,
    GetBucketAclInput, GetBucketCorsInput, GetBucketEncryptionInput,
    GetBucketLifecycleConfigurationInput, GetBucketLoggingInput,
    GetBucketNotificationConfigurationInput, GetBucketOwnershipControlsInput, GetBucketPolicyInput,
    GetBucketPolicyStatusInput, GetBucketRequestPaymentInput, GetBucketTaggingInput,
    GetBucketVersioningInput, GetBucketWebsiteInput, GetObjectLockConfigurationInput,
    GetPublicAccessBlockInput, PutBucketAccelerateConfigurationInput, PutBucketAclInput,
    PutBucketCorsInput, PutBucketEncryptionInput, PutBucketLifecycleConfigurationInput,
    PutBucketLoggingInput, PutBucketNotificationConfigurationInput,
    PutBucketOwnershipControlsInput, PutBucketPolicyInput, PutBucketRequestPaymentInput,
    PutBucketTaggingInput, PutBucketVersioningInput, PutBucketWebsiteInput,
    PutObjectLockConfigurationInput, PutPublicAccessBlockInput,
};
use ruststack_s3_model::output::{
    GetBucketAccelerateConfigurationOutput, GetBucketAclOutput, GetBucketCorsOutput,
    GetBucketEncryptionOutput, GetBucketLifecycleConfigurationOutput, GetBucketLoggingOutput,
    GetBucketNotificationConfigurationOutput, GetBucketOwnershipControlsOutput,
    GetBucketPolicyOutput, GetBucketPolicyStatusOutput, GetBucketRequestPaymentOutput,
    GetBucketTaggingOutput, GetBucketVersioningOutput, GetBucketWebsiteOutput,
    GetObjectLockConfigurationOutput, GetPublicAccessBlockOutput,
    PutBucketLifecycleConfigurationOutput, PutObjectLockConfigurationOutput,
};
use ruststack_s3_model::types::{
    BucketAccelerateStatus, BucketVersioningStatus, CORSRule,
    DefaultRetention as ModelDefaultRetention, ErrorDocument, Grant, Grantee, IndexDocument,
    ObjectLockConfiguration as ModelObjectLockConfiguration, ObjectLockEnabled,
    ObjectLockRetentionMode, ObjectLockRule as ModelObjectLockRule, ObjectOwnership,
    OwnershipControls, OwnershipControlsRule, Payer, Permission, PolicyStatus, Protocol,
    PublicAccessBlockConfiguration, RedirectAllRequestsTo, ServerSideEncryption,
    ServerSideEncryptionByDefault, ServerSideEncryptionConfiguration, ServerSideEncryptionRule,
    Tag,
};
use tracing::debug;

use crate::cors::CorsRule;
use crate::error::S3ServiceError;
use crate::provider::RustStackS3;
use crate::state::bucket::{
    BucketEncryption, CorsRuleConfig, ObjectLockConfiguration, ObjectLockRule,
    OwnershipControlsConfig, PublicAccessBlockConfig, VersioningStatus, WebsiteConfig,
};
use crate::state::object::{CannedAcl, Owner as InternalOwner};

use super::bucket::to_model_owner;

// These handler methods must remain async for consistency.
#[allow(clippy::unused_async)]
impl RustStackS3 {
    // -----------------------------------------------------------------------
    // Versioning
    // -----------------------------------------------------------------------

    /// Get the versioning configuration for a bucket.
    pub async fn handle_get_bucket_versioning(
        &self,
        input: GetBucketVersioningInput,
    ) -> Result<GetBucketVersioningOutput, S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let status = match *bucket.versioning.read() {
            VersioningStatus::Enabled => Some(BucketVersioningStatus::from("Enabled")),
            VersioningStatus::Suspended => Some(BucketVersioningStatus::from("Suspended")),
            VersioningStatus::Disabled => None,
        };

        Ok(GetBucketVersioningOutput {
            mfa_delete: None,
            status,
        })
    }

    /// Set the versioning configuration for a bucket.
    pub async fn handle_put_bucket_versioning(
        &self,
        input: PutBucketVersioningInput,
    ) -> Result<(), S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let config = input.versioning_configuration;
        if let Some(status) = config.status {
            match status.as_str() {
                "Enabled" => bucket.enable_versioning(),
                "Suspended" => bucket.suspend_versioning(),
                _ => {
                    return Err(S3Error::invalid_argument("Invalid versioning status"));
                }
            }
        }

        debug!(bucket = %bucket_name, "put_bucket_versioning completed");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Encryption
    // -----------------------------------------------------------------------

    /// Get the server-side encryption configuration for a bucket.
    pub async fn handle_get_bucket_encryption(
        &self,
        input: GetBucketEncryptionInput,
    ) -> Result<GetBucketEncryptionOutput, S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let enc = bucket.encryption.read();
        let enc_config = enc
            .as_ref()
            .ok_or(S3ServiceError::ServerSideEncryptionConfigurationNotFoundError)
            .map_err(S3ServiceError::into_s3_error)?;

        let rule = ServerSideEncryptionRule {
            apply_server_side_encryption_by_default: Some(ServerSideEncryptionByDefault {
                kms_master_key_id: enc_config.kms_master_key_id.clone(),
                sse_algorithm: ServerSideEncryption::from(enc_config.sse_algorithm.as_str()),
            }),
            blocked_encryption_types: None,
            bucket_key_enabled: Some(enc_config.bucket_key_enabled),
        };

        Ok(GetBucketEncryptionOutput {
            server_side_encryption_configuration: Some(ServerSideEncryptionConfiguration {
                rules: vec![rule],
            }),
        })
    }

    /// Set the server-side encryption configuration for a bucket.
    pub async fn handle_put_bucket_encryption(
        &self,
        input: PutBucketEncryptionInput,
    ) -> Result<(), S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let config = input.server_side_encryption_configuration;

        let rule = config
            .rules
            .first()
            .ok_or_else(|| S3Error::invalid_argument("At least one rule is required"))?;

        let default = rule
            .apply_server_side_encryption_by_default
            .as_ref()
            .ok_or_else(|| {
                S3Error::invalid_argument("ApplyServerSideEncryptionByDefault is required")
            })?;

        let enc = BucketEncryption {
            sse_algorithm: default.sse_algorithm.as_str().to_owned(),
            kms_master_key_id: default.kms_master_key_id.clone(),
            bucket_key_enabled: rule.bucket_key_enabled.unwrap_or(false),
        };

        *bucket.encryption.write() = Some(enc);

        debug!(bucket = %bucket_name, "put_bucket_encryption completed");
        Ok(())
    }

    /// Delete the server-side encryption configuration for a bucket.
    pub async fn handle_delete_bucket_encryption(
        &self,
        input: DeleteBucketEncryptionInput,
    ) -> Result<(), S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        *bucket.encryption.write() = None;

        debug!(bucket = %bucket_name, "delete_bucket_encryption completed");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // CORS
    // -----------------------------------------------------------------------

    /// Get CORS configuration for a bucket.
    pub async fn handle_get_bucket_cors(
        &self,
        input: GetBucketCorsInput,
    ) -> Result<GetBucketCorsOutput, S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let cors_rules = bucket.cors_rules.read();
        let rules = cors_rules
            .as_ref()
            .ok_or(S3ServiceError::NoSuchCorsConfiguration)
            .map_err(S3ServiceError::into_s3_error)?;

        let s3_rules: Vec<CORSRule> = rules.iter().map(cors_config_to_dto).collect();

        Ok(GetBucketCorsOutput {
            cors_rules: s3_rules,
        })
    }

    /// Set CORS configuration for a bucket.
    pub async fn handle_put_bucket_cors(&self, input: PutBucketCorsInput) -> Result<(), S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let cors_config = input.cors_configuration;

        let configs: Vec<CorsRuleConfig> = cors_config
            .cors_rules
            .iter()
            .map(dto_to_cors_config)
            .collect();
        let index_rules: Vec<CorsRule> = configs
            .iter()
            .map(|c| CorsRule {
                allowed_origins: c.allowed_origins.clone(),
                allowed_methods: c.allowed_methods.clone(),
                allowed_headers: c.allowed_headers.clone(),
                expose_headers: c.expose_headers.clone(),
                max_age_seconds: c.max_age_seconds,
            })
            .collect();

        *bucket.cors_rules.write() = Some(configs);
        self.cors_index.set_rules(&bucket_name, index_rules);

        debug!(bucket = %bucket_name, "put_bucket_cors completed");
        Ok(())
    }

    /// Delete CORS configuration for a bucket.
    pub async fn handle_delete_bucket_cors(
        &self,
        input: DeleteBucketCorsInput,
    ) -> Result<(), S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        *bucket.cors_rules.write() = None;
        self.cors_index.delete_rules(&bucket_name);

        debug!(bucket = %bucket_name, "delete_bucket_cors completed");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Lifecycle
    // -----------------------------------------------------------------------

    /// Get lifecycle configuration for a bucket.
    pub async fn handle_get_bucket_lifecycle_configuration(
        &self,
        input: GetBucketLifecycleConfigurationInput,
    ) -> Result<GetBucketLifecycleConfigurationOutput, S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let lifecycle = bucket.lifecycle.read();
        let config = lifecycle
            .as_ref()
            .ok_or(S3ServiceError::NoSuchLifecycleConfiguration)
            .map_err(S3ServiceError::into_s3_error)?;

        Ok(GetBucketLifecycleConfigurationOutput {
            rules: config.rules.clone(),
            transition_default_minimum_object_size: None,
        })
    }

    /// Set lifecycle configuration for a bucket.
    pub async fn handle_put_bucket_lifecycle_configuration(
        &self,
        input: PutBucketLifecycleConfigurationInput,
    ) -> Result<PutBucketLifecycleConfigurationOutput, S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        *bucket.lifecycle.write() = input.lifecycle_configuration;

        debug!(bucket = %bucket_name, "put_bucket_lifecycle_configuration completed");
        Ok(PutBucketLifecycleConfigurationOutput {
            transition_default_minimum_object_size: None,
        })
    }

    /// Delete lifecycle configuration for a bucket.
    pub async fn handle_delete_bucket_lifecycle(
        &self,
        input: DeleteBucketLifecycleInput,
    ) -> Result<(), S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        *bucket.lifecycle.write() = None;

        debug!(bucket = %bucket_name, "delete_bucket_lifecycle completed");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Policy
    // -----------------------------------------------------------------------

    /// Get the bucket policy.
    pub async fn handle_get_bucket_policy(
        &self,
        input: GetBucketPolicyInput,
    ) -> Result<GetBucketPolicyOutput, S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let policy = bucket.policy.read();
        let policy_str = policy
            .as_ref()
            .ok_or(S3ServiceError::NoSuchBucketPolicy)
            .map_err(S3ServiceError::into_s3_error)?
            .clone();

        Ok(GetBucketPolicyOutput {
            policy: Some(policy_str),
        })
    }

    /// Set the bucket policy.
    pub async fn handle_put_bucket_policy(
        &self,
        input: PutBucketPolicyInput,
    ) -> Result<(), S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        *bucket.policy.write() = Some(input.policy);

        debug!(bucket = %bucket_name, "put_bucket_policy completed");
        Ok(())
    }

    /// Delete the bucket policy.
    pub async fn handle_delete_bucket_policy(
        &self,
        input: DeleteBucketPolicyInput,
    ) -> Result<(), S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        *bucket.policy.write() = None;

        debug!(bucket = %bucket_name, "delete_bucket_policy completed");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Tagging
    // -----------------------------------------------------------------------

    /// Get the tag set for a bucket.
    pub async fn handle_get_bucket_tagging(
        &self,
        input: GetBucketTaggingInput,
    ) -> Result<GetBucketTaggingOutput, S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let tags = bucket.tags.read();
        if tags.is_empty() {
            return Err(S3ServiceError::NoSuchTagSet.into_s3_error());
        }

        let tag_set: Vec<Tag> = tags
            .iter()
            .map(|(k, v)| Tag {
                key: k.clone(),
                value: v.clone(),
            })
            .collect();

        Ok(GetBucketTaggingOutput { tag_set })
    }

    /// Set the tag set for a bucket.
    pub async fn handle_put_bucket_tagging(
        &self,
        input: PutBucketTaggingInput,
    ) -> Result<(), S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let tagging = input.tagging;

        let tags: Vec<(String, String)> = tagging
            .tag_set
            .into_iter()
            .map(|t| (t.key, t.value))
            .collect();

        crate::validation::validate_tags(&tags).map_err(S3ServiceError::into_s3_error)?;

        *bucket.tags.write() = tags;

        debug!(bucket = %bucket_name, "put_bucket_tagging completed");
        Ok(())
    }

    /// Delete the tag set for a bucket.
    pub async fn handle_delete_bucket_tagging(
        &self,
        input: DeleteBucketTaggingInput,
    ) -> Result<(), S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        *bucket.tags.write() = Vec::new();

        debug!(bucket = %bucket_name, "delete_bucket_tagging completed");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Notification
    // -----------------------------------------------------------------------

    /// Get notification configuration for a bucket.
    pub async fn handle_get_bucket_notification_configuration(
        &self,
        input: GetBucketNotificationConfigurationInput,
    ) -> Result<GetBucketNotificationConfigurationOutput, S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let notification_configuration = bucket.notification_configuration.read().clone();

        Ok(GetBucketNotificationConfigurationOutput {
            notification_configuration,
        })
    }

    /// Set notification configuration for a bucket.
    pub async fn handle_put_bucket_notification_configuration(
        &self,
        input: PutBucketNotificationConfigurationInput,
    ) -> Result<(), S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        *bucket.notification_configuration.write() = Some(input.notification_configuration);

        debug!(bucket = %bucket_name, "put_bucket_notification_configuration completed");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Logging
    // -----------------------------------------------------------------------

    /// Get logging configuration for a bucket.
    pub async fn handle_get_bucket_logging(
        &self,
        input: GetBucketLoggingInput,
    ) -> Result<GetBucketLoggingOutput, S3Error> {
        let bucket_name = input.bucket;

        let _bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        Ok(GetBucketLoggingOutput {
            logging_enabled: None,
        })
    }

    /// Set logging configuration for a bucket.
    pub async fn handle_put_bucket_logging(
        &self,
        input: PutBucketLoggingInput,
    ) -> Result<(), S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        *bucket.logging.write() = Some(serde_json::json!({"status": "configured"}));

        debug!(bucket = %bucket_name, "put_bucket_logging completed");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Public Access Block
    // -----------------------------------------------------------------------

    /// Get public access block configuration for a bucket.
    pub async fn handle_get_public_access_block(
        &self,
        input: GetPublicAccessBlockInput,
    ) -> Result<GetPublicAccessBlockOutput, S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let pab = bucket.public_access_block.read();
        let config = pab
            .as_ref()
            .ok_or(S3ServiceError::NoSuchPublicAccessBlockConfiguration)
            .map_err(S3ServiceError::into_s3_error)?;

        Ok(GetPublicAccessBlockOutput {
            public_access_block_configuration: Some(PublicAccessBlockConfiguration {
                block_public_acls: Some(config.block_public_acls),
                block_public_policy: Some(config.block_public_policy),
                ignore_public_acls: Some(config.ignore_public_acls),
                restrict_public_buckets: Some(config.restrict_public_buckets),
            }),
        })
    }

    /// Set public access block configuration for a bucket.
    pub async fn handle_put_public_access_block(
        &self,
        input: PutPublicAccessBlockInput,
    ) -> Result<(), S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let config = input.public_access_block_configuration;

        let internal_config = PublicAccessBlockConfig {
            block_public_acls: config.block_public_acls.unwrap_or(false),
            ignore_public_acls: config.ignore_public_acls.unwrap_or(false),
            block_public_policy: config.block_public_policy.unwrap_or(false),
            restrict_public_buckets: config.restrict_public_buckets.unwrap_or(false),
        };

        *bucket.public_access_block.write() = Some(internal_config);

        debug!(bucket = %bucket_name, "put_public_access_block completed");
        Ok(())
    }

    /// Delete public access block configuration for a bucket.
    pub async fn handle_delete_public_access_block(
        &self,
        input: DeletePublicAccessBlockInput,
    ) -> Result<(), S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        *bucket.public_access_block.write() = None;

        debug!(bucket = %bucket_name, "delete_public_access_block completed");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Ownership Controls
    // -----------------------------------------------------------------------

    /// Get ownership controls configuration for a bucket.
    pub async fn handle_get_bucket_ownership_controls(
        &self,
        input: GetBucketOwnershipControlsInput,
    ) -> Result<GetBucketOwnershipControlsOutput, S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let controls = bucket.ownership_controls.read();
        let config = controls
            .as_ref()
            .ok_or(S3ServiceError::OwnershipControlsNotFoundError)
            .map_err(S3ServiceError::into_s3_error)?;

        let rule = OwnershipControlsRule {
            object_ownership: ObjectOwnership::from(config.object_ownership.as_str()),
        };

        Ok(GetBucketOwnershipControlsOutput {
            ownership_controls: Some(OwnershipControls { rules: vec![rule] }),
        })
    }

    /// Set ownership controls configuration for a bucket.
    pub async fn handle_put_bucket_ownership_controls(
        &self,
        input: PutBucketOwnershipControlsInput,
    ) -> Result<(), S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let controls = input.ownership_controls;

        let rule = controls
            .rules
            .first()
            .ok_or_else(|| S3Error::invalid_argument("At least one rule is required"))?;

        let ownership = rule.object_ownership.as_str().to_owned();

        *bucket.ownership_controls.write() = Some(OwnershipControlsConfig {
            object_ownership: ownership,
        });

        debug!(bucket = %bucket_name, "put_bucket_ownership_controls completed");
        Ok(())
    }

    /// Delete ownership controls configuration for a bucket.
    pub async fn handle_delete_bucket_ownership_controls(
        &self,
        input: DeleteBucketOwnershipControlsInput,
    ) -> Result<(), S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        *bucket.ownership_controls.write() = None;

        debug!(bucket = %bucket_name, "delete_bucket_ownership_controls completed");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Object Lock Configuration
    // -----------------------------------------------------------------------

    /// Get object lock configuration for a bucket.
    pub async fn handle_get_object_lock_configuration(
        &self,
        input: GetObjectLockConfigurationInput,
    ) -> Result<GetObjectLockConfigurationOutput, S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        if !*bucket.object_lock_enabled.read() {
            return Err(S3ServiceError::ObjectLockConfigurationNotFoundError.into_s3_error());
        }

        let lock_config = bucket.object_lock_configuration.read();
        let rule = lock_config.as_ref().and_then(|c| {
            c.rule.as_ref().map(|r| ModelObjectLockRule {
                default_retention: r
                    .default_retention
                    .as_ref()
                    .map(|dr| ModelDefaultRetention {
                        days: dr.days,
                        mode: Some(ObjectLockRetentionMode::from(dr.mode.as_str())),
                        years: dr.years,
                    }),
            })
        });

        Ok(GetObjectLockConfigurationOutput {
            object_lock_configuration: Some(ModelObjectLockConfiguration {
                object_lock_enabled: Some(ObjectLockEnabled::from("Enabled")),
                rule,
            }),
        })
    }

    /// Set object lock configuration for a bucket.
    pub async fn handle_put_object_lock_configuration(
        &self,
        input: PutObjectLockConfigurationInput,
    ) -> Result<PutObjectLockConfigurationOutput, S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        if let Some(config) = input.object_lock_configuration {
            *bucket.object_lock_enabled.write() = true;
            bucket.enable_versioning();

            let internal_config = ObjectLockConfiguration {
                object_lock_enabled: config
                    .object_lock_enabled
                    .as_ref()
                    .map_or_else(|| "Enabled".to_owned(), |e| e.as_str().to_owned()),
                rule: config.rule.map(|r| ObjectLockRule {
                    default_retention: r.default_retention.map(|dr| {
                        crate::state::bucket::DefaultRetention {
                            mode: dr
                                .mode
                                .as_ref()
                                .map(|m| m.as_str().to_owned())
                                .unwrap_or_default(),
                            days: dr.days,
                            years: dr.years,
                        }
                    }),
                }),
            };
            *bucket.object_lock_configuration.write() = Some(internal_config);
        }

        debug!(bucket = %bucket_name, "put_object_lock_configuration completed");
        Ok(PutObjectLockConfigurationOutput {
            request_charged: None,
        })
    }

    // -----------------------------------------------------------------------
    // Accelerate
    // -----------------------------------------------------------------------

    /// Get the transfer acceleration configuration for a bucket.
    pub async fn handle_get_bucket_accelerate_configuration(
        &self,
        input: GetBucketAccelerateConfigurationInput,
    ) -> Result<GetBucketAccelerateConfigurationOutput, S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let status = bucket
            .accelerate
            .read()
            .as_ref()
            .map(|s| BucketAccelerateStatus::from(s.as_str()));

        Ok(GetBucketAccelerateConfigurationOutput {
            request_charged: None,
            status,
        })
    }

    /// Set the transfer acceleration configuration for a bucket.
    pub async fn handle_put_bucket_accelerate_configuration(
        &self,
        input: PutBucketAccelerateConfigurationInput,
    ) -> Result<(), S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let config = input.accelerate_configuration;
        let status = config.status.map(|s| s.as_str().to_owned());
        *bucket.accelerate.write() = status;

        debug!(bucket = %bucket_name, "put_bucket_accelerate_configuration completed");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Request Payment
    // -----------------------------------------------------------------------

    /// Get the request payment configuration for a bucket.
    pub async fn handle_get_bucket_request_payment(
        &self,
        input: GetBucketRequestPaymentInput,
    ) -> Result<GetBucketRequestPaymentOutput, S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let payer = bucket.request_payment.read().clone();

        Ok(GetBucketRequestPaymentOutput {
            payer: Some(Payer::from(payer.as_str())),
        })
    }

    /// Set the request payment configuration for a bucket.
    pub async fn handle_put_bucket_request_payment(
        &self,
        input: PutBucketRequestPaymentInput,
    ) -> Result<(), S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let config = input.request_payment_configuration;
        config
            .payer
            .as_str()
            .clone_into(&mut bucket.request_payment.write());

        debug!(bucket = %bucket_name, "put_bucket_request_payment completed");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Website
    // -----------------------------------------------------------------------

    /// Get the website configuration for a bucket.
    pub async fn handle_get_bucket_website(
        &self,
        input: GetBucketWebsiteInput,
    ) -> Result<GetBucketWebsiteOutput, S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let guard = bucket.website.read();
        let config = guard
            .as_ref()
            .ok_or_else(|| S3ServiceError::NoSuchWebsiteConfiguration.into_s3_error())?;

        Ok(GetBucketWebsiteOutput {
            index_document: config
                .index_document_suffix
                .as_ref()
                .map(|s| IndexDocument { suffix: s.clone() }),
            error_document: config
                .error_document_key
                .as_ref()
                .map(|k| ErrorDocument { key: k.clone() }),
            redirect_all_requests_to: config.redirect_all_requests_to_host.as_ref().map(|host| {
                RedirectAllRequestsTo {
                    host_name: host.clone(),
                    protocol: config.redirect_all_requests_to_protocol.as_ref().map(|p| {
                        match p.as_str() {
                            "https" => Protocol::Https,
                            _ => Protocol::Http,
                        }
                    }),
                }
            }),
            routing_rules: Vec::new(),
        })
    }

    /// Set the website configuration for a bucket.
    pub async fn handle_put_bucket_website(
        &self,
        input: PutBucketWebsiteInput,
    ) -> Result<(), S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let wc = &input.website_configuration;
        *bucket.website.write() = Some(WebsiteConfig {
            index_document_suffix: wc.index_document.as_ref().map(|d| d.suffix.clone()),
            error_document_key: wc.error_document.as_ref().map(|d| d.key.clone()),
            redirect_all_requests_to_host: wc
                .redirect_all_requests_to
                .as_ref()
                .map(|r| r.host_name.clone()),
            redirect_all_requests_to_protocol: wc
                .redirect_all_requests_to
                .as_ref()
                .and_then(|r| r.protocol.as_ref().map(|p| p.as_str().to_owned())),
        });

        debug!(bucket = %bucket_name, "put_bucket_website completed");
        Ok(())
    }

    /// Delete the website configuration for a bucket.
    pub async fn handle_delete_bucket_website(
        &self,
        input: DeleteBucketWebsiteInput,
    ) -> Result<(), S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        *bucket.website.write() = None;

        debug!(bucket = %bucket_name, "delete_bucket_website completed");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Bucket ACL
    // -----------------------------------------------------------------------

    /// Get the ACL for a bucket.
    pub async fn handle_get_bucket_acl(
        &self,
        input: GetBucketAclInput,
    ) -> Result<GetBucketAclOutput, S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let owner = to_model_owner(&bucket.owner);
        let acl = *bucket.acl.read();
        let grants = canned_acl_to_grants(&bucket.owner, acl);

        Ok(GetBucketAclOutput {
            grants,
            owner: Some(owner),
        })
    }

    /// Set the ACL for a bucket.
    pub async fn handle_put_bucket_acl(&self, input: PutBucketAclInput) -> Result<(), S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        if let Some(acl_val) = input.acl {
            let acl: CannedAcl = acl_val
                .as_str()
                .parse()
                .map_err(|_| S3Error::invalid_argument("Invalid canned ACL"))?;
            *bucket.acl.write() = acl;
        }

        debug!(bucket = %bucket_name, "put_bucket_acl completed");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Policy Status
    // -----------------------------------------------------------------------

    /// Get the policy status for a bucket.
    pub async fn handle_get_bucket_policy_status(
        &self,
        input: GetBucketPolicyStatusInput,
    ) -> Result<GetBucketPolicyStatusOutput, S3Error> {
        let bucket_name = input.bucket;

        let _bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        Ok(GetBucketPolicyStatusOutput {
            policy_status: Some(PolicyStatus {
                is_public: Some(false),
            }),
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert a [`CorsRuleConfig`] to a model [`CORSRule`].
fn cors_config_to_dto(config: &CorsRuleConfig) -> CORSRule {
    CORSRule {
        allowed_headers: config.allowed_headers.clone(),
        allowed_methods: config.allowed_methods.clone(),
        allowed_origins: config.allowed_origins.clone(),
        expose_headers: config.expose_headers.clone(),
        id: config.id.clone(),
        max_age_seconds: config.max_age_seconds,
    }
}

/// Convert a model [`CORSRule`] to a [`CorsRuleConfig`].
fn dto_to_cors_config(rule: &CORSRule) -> CorsRuleConfig {
    CorsRuleConfig {
        id: rule.id.clone(),
        allowed_origins: rule.allowed_origins.clone(),
        allowed_methods: rule.allowed_methods.clone(),
        allowed_headers: rule.allowed_headers.clone(),
        expose_headers: rule.expose_headers.clone(),
        max_age_seconds: rule.max_age_seconds,
    }
}

/// Convert a canned ACL to a list of model [`Grant`] DTOs.
fn canned_acl_to_grants(owner: &InternalOwner, acl: CannedAcl) -> Vec<Grant> {
    let owner_grant = Grant {
        grantee: Some(Grantee {
            display_name: Some(owner.display_name.clone()),
            email_address: None,
            id: Some(owner.id.clone()),
            r#type: ruststack_s3_model::types::Type::from("CanonicalUser"),
            uri: None,
        }),
        permission: Some(Permission::from("FULL_CONTROL")),
    };

    match acl {
        CannedAcl::PublicRead => {
            let public_read = Grant {
                grantee: Some(Grantee {
                    display_name: None,
                    email_address: None,
                    id: None,
                    r#type: ruststack_s3_model::types::Type::from("Group"),
                    uri: Some("http://acs.amazonaws.com/groups/global/AllUsers".to_owned()),
                }),
                permission: Some(Permission::from("READ")),
            };
            vec![owner_grant, public_read]
        }
        _ => vec![owner_grant],
    }
}
