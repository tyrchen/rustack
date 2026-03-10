//! Object configuration operation handlers.
//!
//! Implements `get_object_tagging`, `put_object_tagging`, `delete_object_tagging`,
//! `get_object_acl`, `put_object_acl`, `get_object_retention`,
//! `put_object_retention`, `get_object_legal_hold`, `put_object_legal_hold`,
//! and `get_object_attributes`.

use ruststack_s3_model::error::S3Error;
use ruststack_s3_model::input::{
    DeleteObjectTaggingInput, GetObjectAclInput, GetObjectAttributesInput, GetObjectLegalHoldInput,
    GetObjectRetentionInput, GetObjectTaggingInput, PutObjectAclInput, PutObjectLegalHoldInput,
    PutObjectRetentionInput, PutObjectTaggingInput,
};
use ruststack_s3_model::output::{
    DeleteObjectTaggingOutput, GetObjectAclOutput, GetObjectAttributesOutput,
    GetObjectLegalHoldOutput, GetObjectRetentionOutput, GetObjectTaggingOutput, PutObjectAclOutput,
    PutObjectLegalHoldOutput, PutObjectRetentionOutput, PutObjectTaggingOutput,
};
use ruststack_s3_model::types::{
    Checksum, ChecksumType, GetObjectAttributesParts, Grant, Grantee, ObjectLockLegalHold,
    ObjectLockLegalHoldStatus, ObjectLockRetention, ObjectLockRetentionMode, Permission,
    StorageClass, Tag, Type,
};
use tracing::debug;

use crate::error::S3ServiceError;
use crate::provider::RustStackS3;
use crate::state::object::CannedAcl;

use super::bucket::to_model_owner;

// AWS S3 DTOs use signed integers (i32/i64) for inherently non-negative values.
// These handler methods must remain async for consistency.
#[allow(
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::unused_async
)]
impl RustStackS3 {
    // -----------------------------------------------------------------------
    // Object Tagging
    // -----------------------------------------------------------------------

    /// Get tags for an object.
    pub async fn handle_get_object_tagging(
        &self,
        input: GetObjectTaggingInput,
    ) -> Result<GetObjectTaggingOutput, S3Error> {
        let bucket_name = input.bucket;
        let key = input.key;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let store = bucket.objects.read();
        let obj = if let Some(version_id) = &input.version_id {
            store.get_version(&key, version_id).ok_or_else(|| {
                S3ServiceError::NoSuchVersion {
                    key: key.clone(),
                    version_id: version_id.clone(),
                }
                .into_s3_error()
            })?
        } else {
            store
                .get(&key)
                .ok_or_else(|| S3ServiceError::NoSuchKey { key: key.clone() }.into_s3_error())?
        };

        let tag_set: Vec<Tag> = obj
            .metadata
            .tagging
            .iter()
            .map(|(k, v)| Tag {
                key: k.clone(),
                value: v.clone(),
            })
            .collect();

        let version_id = if obj.version_id == "null" {
            None
        } else {
            Some(obj.version_id.clone())
        };

        Ok(GetObjectTaggingOutput {
            tag_set,
            version_id,
        })
    }

    /// Set tags for an object.
    pub async fn handle_put_object_tagging(
        &self,
        input: PutObjectTaggingInput,
    ) -> Result<PutObjectTaggingOutput, S3Error> {
        let bucket_name = input.bucket;
        let key = input.key;

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

        // We need mutable access to the object. Since ObjectStore wraps objects
        // immutably, we re-insert a modified copy.
        let mut store = bucket.objects.write();
        let obj = if let Some(version_id) = &input.version_id {
            store.get_version(&key, version_id).ok_or_else(|| {
                S3ServiceError::NoSuchVersion {
                    key: key.clone(),
                    version_id: version_id.clone(),
                }
                .into_s3_error()
            })?
        } else {
            store
                .get(&key)
                .ok_or_else(|| S3ServiceError::NoSuchKey { key: key.clone() }.into_s3_error())?
        };

        let mut updated = obj.clone();
        updated.metadata.tagging = tags;
        store.put(updated);

        debug!(bucket = %bucket_name, key = %key, "put_object_tagging completed");

        let version_id_out = input.version_id;
        Ok(PutObjectTaggingOutput {
            version_id: version_id_out,
        })
    }

    /// Delete tags for an object.
    pub async fn handle_delete_object_tagging(
        &self,
        input: DeleteObjectTaggingInput,
    ) -> Result<DeleteObjectTaggingOutput, S3Error> {
        let bucket_name = input.bucket;
        let key = input.key;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let mut store = bucket.objects.write();
        let obj = if let Some(version_id) = &input.version_id {
            store.get_version(&key, version_id).ok_or_else(|| {
                S3ServiceError::NoSuchVersion {
                    key: key.clone(),
                    version_id: version_id.clone(),
                }
                .into_s3_error()
            })?
        } else {
            store
                .get(&key)
                .ok_or_else(|| S3ServiceError::NoSuchKey { key: key.clone() }.into_s3_error())?
        };

        let mut updated = obj.clone();
        updated.metadata.tagging = Vec::new();
        store.put(updated);

        debug!(bucket = %bucket_name, key = %key, "delete_object_tagging completed");

        let version_id_out = input.version_id;
        Ok(DeleteObjectTaggingOutput {
            version_id: version_id_out,
        })
    }

    // -----------------------------------------------------------------------
    // Object ACL
    // -----------------------------------------------------------------------

    /// Get the ACL for an object.
    pub async fn handle_get_object_acl(
        &self,
        input: GetObjectAclInput,
    ) -> Result<GetObjectAclOutput, S3Error> {
        let bucket_name = input.bucket;
        let key = input.key;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let store = bucket.objects.read();
        let obj = if let Some(version_id) = &input.version_id {
            store.get_version(&key, version_id).ok_or_else(|| {
                S3ServiceError::NoSuchVersion {
                    key: key.clone(),
                    version_id: version_id.clone(),
                }
                .into_s3_error()
            })?
        } else {
            store
                .get(&key)
                .ok_or_else(|| S3ServiceError::NoSuchKey { key: key.clone() }.into_s3_error())?
        };

        let owner = to_model_owner(&obj.owner);

        let grant = Grant {
            grantee: Some(Grantee {
                display_name: Some(obj.owner.display_name.clone()),
                email_address: None,
                id: Some(obj.owner.id.clone()),
                r#type: Type::CanonicalUser,
                uri: None,
            }),
            permission: Some(Permission::FullControl),
        };

        Ok(GetObjectAclOutput {
            grants: vec![grant],
            owner: Some(owner),
            request_charged: None,
        })
    }

    /// Set the ACL for an object.
    pub async fn handle_put_object_acl(
        &self,
        input: PutObjectAclInput,
    ) -> Result<PutObjectAclOutput, S3Error> {
        let bucket_name = input.bucket;
        let key = input.key;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        if let Some(acl_enum) = input.acl {
            let acl: CannedAcl = acl_enum
                .as_str()
                .parse()
                .map_err(|_| S3Error::invalid_argument("Invalid canned ACL"))?;

            let mut store = bucket.objects.write();
            let obj = if let Some(version_id) = &input.version_id {
                store.get_version(&key, version_id).ok_or_else(|| {
                    S3ServiceError::NoSuchVersion {
                        key: key.clone(),
                        version_id: version_id.clone(),
                    }
                    .into_s3_error()
                })?
            } else {
                store
                    .get(&key)
                    .ok_or_else(|| S3ServiceError::NoSuchKey { key: key.clone() }.into_s3_error())?
            };

            let mut updated = obj.clone();
            updated.metadata.acl = acl;
            store.put(updated);
        }

        debug!(bucket = %bucket_name, key = %key, "put_object_acl completed");

        Ok(PutObjectAclOutput {
            request_charged: None,
        })
    }

    // -----------------------------------------------------------------------
    // Object Retention
    // -----------------------------------------------------------------------

    /// Get the retention configuration for an object.
    pub async fn handle_get_object_retention(
        &self,
        input: GetObjectRetentionInput,
    ) -> Result<GetObjectRetentionOutput, S3Error> {
        let bucket_name = input.bucket;
        let key = input.key;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let store = bucket.objects.read();
        let obj = if let Some(version_id) = &input.version_id {
            store.get_version(&key, version_id).ok_or_else(|| {
                S3ServiceError::NoSuchVersion {
                    key: key.clone(),
                    version_id: version_id.clone(),
                }
                .into_s3_error()
            })?
        } else {
            store
                .get(&key)
                .ok_or_else(|| S3ServiceError::NoSuchKey { key: key.clone() }.into_s3_error())?
        };

        let retention = match (
            &obj.metadata.object_lock_mode,
            obj.metadata.object_lock_retain_until,
        ) {
            (Some(mode), Some(until)) => Some(ObjectLockRetention {
                mode: Some(ObjectLockRetentionMode::from(mode.as_str())),
                retain_until_date: Some(until),
            }),
            _ => None,
        };

        if retention.is_none() {
            return Err(S3Error::invalid_argument(
                "No retention configuration found",
            ));
        }

        Ok(GetObjectRetentionOutput { retention })
    }

    /// Set the retention configuration for an object.
    pub async fn handle_put_object_retention(
        &self,
        input: PutObjectRetentionInput,
    ) -> Result<PutObjectRetentionOutput, S3Error> {
        let bucket_name = input.bucket;
        let key = input.key;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let retention = input.retention;

        let mut store = bucket.objects.write();

        // First read existing retention metadata (immutable borrow).
        {
            let obj = if let Some(version_id) = &input.version_id {
                store.get_version(&key, version_id).ok_or_else(|| {
                    S3ServiceError::NoSuchVersion {
                        key: key.clone(),
                        version_id: version_id.clone(),
                    }
                    .into_s3_error()
                })?
            } else {
                store
                    .get(&key)
                    .ok_or_else(|| S3ServiceError::NoSuchKey { key: key.clone() }.into_s3_error())?
            };

            // Enforce Object Lock retention rules.
            let existing_until = obj.metadata.object_lock_retain_until;
            let existing_mode = obj.metadata.object_lock_mode.as_deref();
            let bypass = input.bypass_governance_retention.unwrap_or(false);

            if let Some(current_until) = existing_until {
                let now = chrono::Utc::now();
                if current_until > now {
                    let new_mode = retention.as_ref().and_then(|r| r.mode.as_ref());
                    let new_until = retention.as_ref().and_then(|r| r.retain_until_date);

                    // COMPLIANCE mode: cannot change mode, cannot shorten, cannot remove.
                    if existing_mode == Some("COMPLIANCE") {
                        let mode_changed = new_mode.is_none_or(|m| m.as_str() != "COMPLIANCE");
                        let is_shortening = match new_until {
                            Some(new) => new < current_until,
                            None => true,
                        };
                        if mode_changed || is_shortening {
                            return Err(S3ServiceError::AccessDenied.into_s3_error());
                        }
                    } else {
                        // GOVERNANCE mode: can be bypassed with the bypass flag.
                        let is_shortening = match new_until {
                            Some(new) => new < current_until,
                            None => true,
                        };
                        if is_shortening && !bypass {
                            return Err(S3ServiceError::AccessDenied.into_s3_error());
                        }
                    }
                }
            }
        }

        // Now mutate the version metadata in-place (mutable borrow).
        let obj = if let Some(version_id) = &input.version_id {
            store.get_version_mut(&key, version_id).ok_or_else(|| {
                S3ServiceError::NoSuchVersion {
                    key: key.clone(),
                    version_id: version_id.clone(),
                }
                .into_s3_error()
            })?
        } else {
            store
                .get_mut(&key)
                .ok_or_else(|| S3ServiceError::NoSuchKey { key: key.clone() }.into_s3_error())?
        };

        if let Some(ret) = retention {
            obj.metadata.object_lock_mode = ret.mode.as_ref().map(|m| m.as_str().to_owned());
            obj.metadata.object_lock_retain_until = ret.retain_until_date;
        } else {
            obj.metadata.object_lock_mode = None;
            obj.metadata.object_lock_retain_until = None;
        }

        debug!(bucket = %bucket_name, key = %key, "put_object_retention completed");

        Ok(PutObjectRetentionOutput {
            request_charged: None,
        })
    }

    // -----------------------------------------------------------------------
    // Object Legal Hold
    // -----------------------------------------------------------------------

    /// Get the legal hold status for an object.
    pub async fn handle_get_object_legal_hold(
        &self,
        input: GetObjectLegalHoldInput,
    ) -> Result<GetObjectLegalHoldOutput, S3Error> {
        let bucket_name = input.bucket;
        let key = input.key;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        if !*bucket.object_lock_enabled.read() {
            return Err(S3ServiceError::ObjectLockConfigurationNotFoundError.into_s3_error());
        }

        let store = bucket.objects.read();
        let obj = if let Some(version_id) = &input.version_id {
            store.get_version(&key, version_id).ok_or_else(|| {
                S3ServiceError::NoSuchVersion {
                    key: key.clone(),
                    version_id: version_id.clone(),
                }
                .into_s3_error()
            })?
        } else {
            store
                .get(&key)
                .ok_or_else(|| S3ServiceError::NoSuchKey { key: key.clone() }.into_s3_error())?
        };

        let is_on = obj.metadata.object_lock_legal_hold.unwrap_or(false);
        let status = if is_on {
            ObjectLockLegalHoldStatus::On
        } else {
            ObjectLockLegalHoldStatus::Off
        };

        Ok(GetObjectLegalHoldOutput {
            legal_hold: Some(ObjectLockLegalHold {
                status: Some(status),
            }),
        })
    }

    /// Set the legal hold status for an object.
    pub async fn handle_put_object_legal_hold(
        &self,
        input: PutObjectLegalHoldInput,
    ) -> Result<PutObjectLegalHoldOutput, S3Error> {
        let bucket_name = input.bucket;
        let key = input.key;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        if !*bucket.object_lock_enabled.read() {
            return Err(S3ServiceError::ObjectLockConfigurationNotFoundError.into_s3_error());
        }

        let legal_hold = input.legal_hold;

        // Reject requests with missing or empty LegalHold body.
        let status = legal_hold.as_ref().and_then(|lh| lh.status.as_ref());
        if status.is_none() {
            return Err(S3Error::malformed_xml("Missing LegalHold status"));
        }

        let mut store = bucket.objects.write();

        // Verify the object exists first.
        if let Some(version_id) = &input.version_id {
            store.get_version(&key, version_id).ok_or_else(|| {
                S3ServiceError::NoSuchVersion {
                    key: key.clone(),
                    version_id: version_id.clone(),
                }
                .into_s3_error()
            })?;
        } else {
            store
                .get(&key)
                .ok_or_else(|| S3ServiceError::NoSuchKey { key: key.clone() }.into_s3_error())?;
        }

        // Update the legal hold status in-place (no new version created).
        let new_hold = legal_hold
            .and_then(|lh| lh.status)
            .map(|s| s.as_str() == "ON");

        let obj = if let Some(version_id) = &input.version_id {
            store.get_version_mut(&key, version_id)
        } else {
            store.get_mut(&key)
        }
        .ok_or_else(|| S3ServiceError::NoSuchKey { key: key.clone() }.into_s3_error())?;

        obj.metadata.object_lock_legal_hold = new_hold;

        debug!(bucket = %bucket_name, key = %key, "put_object_legal_hold completed");

        Ok(PutObjectLegalHoldOutput {
            request_charged: None,
        })
    }

    // -----------------------------------------------------------------------
    // Get Object Attributes
    // -----------------------------------------------------------------------

    /// Get attributes for an object.
    pub async fn handle_get_object_attributes(
        &self,
        input: GetObjectAttributesInput,
    ) -> Result<GetObjectAttributesOutput, S3Error> {
        let bucket_name = input.bucket;
        let key = input.key;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let store = bucket.objects.read();
        let obj = if let Some(version_id) = &input.version_id {
            store.get_version(&key, version_id).ok_or_else(|| {
                S3ServiceError::NoSuchVersion {
                    key: key.clone(),
                    version_id: version_id.clone(),
                }
                .into_s3_error()
            })?
        } else {
            store
                .get(&key)
                .ok_or_else(|| S3ServiceError::NoSuchKey { key: key.clone() }.into_s3_error())?
        };

        let version_id = if obj.version_id == "null" {
            None
        } else {
            Some(obj.version_id.clone())
        };

        let checksum = obj.checksum.as_ref().map(|c| {
            let mut cksum = Checksum::default();
            match c.algorithm.as_str() {
                "CRC32" => cksum.checksum_crc32 = Some(c.value.clone()),
                "CRC32C" => cksum.checksum_crc32c = Some(c.value.clone()),
                "CRC64NVME" => cksum.checksum_crc64nvme = Some(c.value.clone()),
                "SHA1" => cksum.checksum_sha1 = Some(c.value.clone()),
                "SHA256" => cksum.checksum_sha256 = Some(c.value.clone()),
                _ => {}
            }
            cksum.checksum_type = Some(ChecksumType::FullObject);
            cksum
        });

        Ok(GetObjectAttributesOutput {
            checksum,
            delete_marker: None,
            e_tag: Some(obj.etag.clone()),
            last_modified: Some(obj.last_modified),
            object_parts: obj.parts_count.map(|n| GetObjectAttributesParts {
                is_truncated: None,
                max_parts: None,
                next_part_number_marker: None,
                part_number_marker: None,
                parts: Vec::new(),
                total_parts_count: Some(n as i32),
            }),
            object_size: Some(obj.size as i64),
            request_charged: None,
            storage_class: Some(StorageClass::from(obj.storage_class.as_str())),
            version_id,
        })
    }
}
