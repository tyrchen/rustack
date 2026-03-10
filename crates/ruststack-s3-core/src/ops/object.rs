//! Object CRUD operation handlers.
//!
//! Implements `put_object`, `get_object`, `head_object`, `delete_object`,
//! `delete_objects`, and `copy_object`.

use std::collections::HashMap;
use std::str::FromStr;

use bytes::Bytes;
use chrono::Utc;
use ruststack_s3_model::error::{S3Error, S3ErrorCode};
use ruststack_s3_model::input::{
    CopyObjectInput, DeleteObjectInput, DeleteObjectsInput, GetObjectInput, HeadObjectInput,
    PutObjectInput,
};
use ruststack_s3_model::output::{
    CopyObjectOutput, DeleteObjectOutput, DeleteObjectsOutput, GetObjectOutput, HeadObjectOutput,
    PutObjectOutput,
};
use ruststack_s3_model::request::StreamingBlob;
use ruststack_s3_model::types::{
    ChecksumType, CopyObjectResult, DeletedObject, MetadataDirective, ObjectCannedACL,
    ObjectLockLegalHoldStatus, ObjectLockMode, ServerSideEncryption, StorageClass,
};
use tracing::debug;

use crate::checksums::{ChecksumAlgorithm, compute_checksum};
use crate::error::S3ServiceError;
use crate::provider::RustStackS3;
use crate::state::keystore::ObjectStore;
use crate::state::object::{
    CannedAcl, ChecksumData, ObjectMetadata, Owner as InternalOwner, S3Object,
};
use crate::utils::{
    is_valid_if_match, is_valid_if_none_match, parse_copy_source, parse_range_header,
};
use crate::validation::{validate_content_md5, validate_metadata, validate_object_key};

/// Check whether Object Lock (legal hold or retention) prevents deletion of a
/// specific object version.
///
/// # Errors
///
/// Returns `AccessDenied` if the version has a legal hold enabled or an active
/// retention period that cannot be bypassed.
///
/// AWS S3 rules:
/// - DELETE *without* a version ID always succeeds (creates a delete marker).
/// - DELETE *with* a version ID must be rejected if the version has a legal
///   hold enabled or a retention period that has not yet expired.
/// - `BypassGovernanceRetention` allows skipping GOVERNANCE-mode retention
///   checks, but never COMPLIANCE-mode or legal holds.
///
/// Returns `Ok(())` when the deletion is allowed.
#[allow(clippy::result_large_err)]
fn check_object_lock_for_delete(
    store: &ObjectStore,
    key: &str,
    version_id: &str,
    bypass_governance: bool,
) -> Result<(), S3Error> {
    let Some(obj) = store.get_version(key, version_id) else {
        // Version not found — nothing to protect.
        return Ok(());
    };

    if obj.metadata.object_lock_legal_hold == Some(true) {
        return Err(S3Error::with_message(
            S3ErrorCode::AccessDenied,
            "Object Lock legal hold is enabled on this object",
        ));
    }

    if let Some(retain_until) = obj.metadata.object_lock_retain_until {
        if retain_until > Utc::now() {
            let is_governance = obj.metadata.object_lock_mode.as_deref() == Some("GOVERNANCE");
            if is_governance && bypass_governance {
                // GOVERNANCE mode retention can be bypassed.
            } else {
                return Err(S3Error::with_message(
                    S3ErrorCode::AccessDenied,
                    "Object Lock retention period has not expired",
                ));
            }
        }
    }

    Ok(())
}

// AWS S3 DTOs use signed integers (i32/i64) for inherently non-negative values
// (sizes, part counts). Casting from u64/u32/usize is safe in practice.
// These handler methods must remain async because some operations involve
// storage I/O.
#[allow(
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::unused_async
)]
impl RustStackS3 {
    /// Put (upload) a new object.
    pub async fn handle_put_object(
        &self,
        mut input: PutObjectInput,
    ) -> Result<PutObjectOutput, S3Error> {
        let bucket_name = input.bucket.clone();
        let key = input.key.clone();

        validate_object_key(&key).map_err(S3ServiceError::into_s3_error)?;

        // Verify bucket exists.
        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        // Take the body out before borrowing other fields from input.
        let body_data = input.body.take().map_or_else(Bytes::new, |b| b.data);

        // Validate Content-MD5 if provided.
        validate_content_md5(input.content_md5.as_deref(), &body_data)
            .map_err(S3ServiceError::into_s3_error)?;

        // Extract metadata from the request.
        let metadata = build_metadata(&input);
        validate_metadata(&metadata.user_metadata).map_err(S3ServiceError::into_s3_error)?;

        // Determine version ID based on versioning status.
        let version_id = if bucket.is_versioning_enabled() {
            crate::utils::generate_version_id()
        } else {
            "null".to_owned()
        };

        // Write to storage.
        let write_result = self
            .storage
            .write_object(&bucket_name, &key, &version_id, body_data.clone())
            .await
            .map_err(|e| S3ServiceError::Internal(anyhow::anyhow!("{e}")).into_s3_error())?;

        // Extract checksum from the request, or compute CRC32 by default.
        let client_checksum =
            extract_checksum_from_put(&input).map_err(S3ServiceError::into_s3_error)?;
        let is_client_provided = client_checksum.is_some();

        let checksum = client_checksum.unwrap_or_else(|| ChecksumData {
            algorithm: "CRC32".to_owned(),
            value: compute_checksum(ChecksumAlgorithm::Crc32, &body_data),
            checksum_type: "FULL_OBJECT".to_owned(),
        });

        // Validate client-provided checksum against server-computed value.
        if is_client_provided {
            if let Ok(algo) = ChecksumAlgorithm::from_str(&checksum.algorithm) {
                let computed = compute_checksum(algo, &body_data);
                if checksum.value != computed {
                    return Err(S3ServiceError::BadDigest.into_s3_error());
                }
            }
        }

        // Build the S3Object.
        let owner = InternalOwner::default();
        let obj = S3Object {
            key: key.clone(),
            version_id: version_id.clone(),
            etag: write_result.etag.clone(),
            size: write_result.size,
            last_modified: Utc::now(),
            storage_class: input
                .storage_class
                .as_ref()
                .map_or_else(|| "STANDARD".to_owned(), StorageClass::as_str_owned),
            metadata,
            owner,
            checksum: Some(checksum.clone()),
            parts_count: None,
            part_etags: Vec::new(),
        };

        // Store the object metadata.
        {
            let mut store = bucket.objects.write();
            store.put(obj);
        }

        debug!(bucket = %bucket_name, key = %key, version_id = %version_id, "put_object completed");

        let real_version_id = if version_id == "null" {
            None
        } else {
            Some(version_id)
        };

        let cksum = checksum_to_fields(&checksum);
        Ok(PutObjectOutput {
            e_tag: Some(write_result.etag),
            version_id: real_version_id,
            checksum_crc32: cksum.crc32,
            checksum_crc32c: cksum.crc32c,
            checksum_crc64nvme: cksum.crc64nvme,
            checksum_sha1: cksum.sha1,
            checksum_sha256: cksum.sha256,
            checksum_type: cksum.checksum_type,
            ..PutObjectOutput::default()
        })
    }

    /// Get (download) an object.
    #[allow(clippy::too_many_lines)]
    pub async fn handle_get_object(
        &self,
        input: GetObjectInput,
    ) -> Result<GetObjectOutput, S3Error> {
        let bucket_name = input.bucket;
        let key = input.key;
        let version_id_param = input.version_id;
        let if_match_param = input.if_match;
        let if_none_match_param = input.if_none_match;
        let range_param = input.range;
        let checksum_mode = input.checksum_mode;

        // S3 response header overrides (from query parameters in presigned URLs).
        let override_cache_control = input.response_cache_control;
        let override_content_disposition = input.response_content_disposition;
        let override_content_encoding = input.response_content_encoding;
        let override_content_language = input.response_content_language;
        let override_content_type = input.response_content_type;
        let override_expires = input.response_expires;

        // Look up the object and extract all needed data while holding the lock.
        // The lock must be dropped before any `.await` calls since parking_lot
        // guards are `!Send`.
        let (
            obj_size,
            obj_etag,
            obj_last_modified,
            obj_version_id,
            obj_storage_class,
            obj_meta,
            obj_parts_count,
            obj_checksum,
            version_for_storage,
        ) = {
            let bucket = self
                .state
                .get_bucket(&bucket_name)
                .map_err(S3ServiceError::into_s3_error)?;

            let store = bucket.objects.read();
            let obj = if let Some(ref version_id) = version_id_param {
                store.get_version(&key, version_id).ok_or_else(|| {
                    // Check if the version is a delete marker.
                    if store.is_delete_marker(&key, version_id) {
                        S3ServiceError::MethodNotAllowed
                            .into_s3_error()
                            .with_header("x-amz-delete-marker", "true")
                            .with_header("x-amz-version-id", version_id.clone())
                    } else {
                        S3ServiceError::NoSuchVersion {
                            key: key.clone(),
                            version_id: version_id.clone(),
                        }
                        .into_s3_error()
                    }
                })?
            } else {
                store
                    .get(&key)
                    .ok_or_else(|| S3ServiceError::NoSuchKey { key: key.clone() }.into_s3_error())?
            };

            // Conditional request checks.
            if let Some(ref if_match) = if_match_param {
                if !is_valid_if_match(&obj.etag, if_match) {
                    return Err(S3ServiceError::PreconditionFailed.into_s3_error());
                }
            }
            if let Some(ref if_none_match) = if_none_match_param {
                if !is_valid_if_none_match(&obj.etag, if_none_match) {
                    return Err(S3ServiceError::NotModified.into_s3_error());
                }
            }

            let version_id_opt = if obj.version_id == "null" {
                None
            } else {
                Some(obj.version_id.clone())
            };

            (
                obj.size,
                obj.etag.clone(),
                obj.last_modified,
                version_id_opt,
                obj.storage_class.clone(),
                obj.metadata.clone(),
                obj.parts_count,
                obj.checksum.clone(),
                obj.version_id.clone(),
            )
        };

        // Parse range header if provided.
        let range = if let Some(ref range_value) = range_param {
            let (start, end) =
                parse_range_header(range_value, obj_size).map_err(S3ServiceError::into_s3_error)?;
            Some((start, end))
        } else {
            None
        };

        // Read data from storage.
        let data = self
            .storage
            .read_object(&bucket_name, &key, &version_for_storage, range)
            .await
            .map_err(|e| S3ServiceError::Internal(anyhow::anyhow!("{e}")).into_s3_error())?;

        let content_length = data.len() as i64;

        // Build the streaming body from the data bytes.
        let body = StreamingBlob::new(data);

        let content_range = range.map(|(start, end)| format!("bytes {start}-{end}/{obj_size}"));

        let content_type = Some(
            obj_meta
                .content_type
                .clone()
                .unwrap_or_else(|| "binary/octet-stream".to_owned()),
        );

        let metadata = if obj_meta.user_metadata.is_empty() {
            HashMap::default()
        } else {
            obj_meta.user_metadata.clone()
        };

        // Only return checksums when ChecksumMode=ENABLED (matching AWS behavior)
        // and the request is for the full object (not a range request). A range
        // response covers a subset of the object data, so the full-object
        // checksum would not match and SDKs would reject the response.
        let checksum_enabled = checksum_mode
            .as_ref()
            .is_some_and(|m| m.as_str() == "ENABLED");
        let cksum = if checksum_enabled && range.is_none() {
            obj_checksum.as_ref().map(checksum_to_fields)
        } else {
            None
        };
        let output = GetObjectOutput {
            accept_ranges: Some("bytes".to_owned()),
            body: Some(body),
            cache_control: override_cache_control.or(obj_meta.cache_control),
            checksum_crc32: cksum.as_ref().and_then(|c| c.crc32.clone()),
            checksum_crc32c: cksum.as_ref().and_then(|c| c.crc32c.clone()),
            checksum_crc64nvme: cksum.as_ref().and_then(|c| c.crc64nvme.clone()),
            checksum_sha1: cksum.as_ref().and_then(|c| c.sha1.clone()),
            checksum_sha256: cksum.as_ref().and_then(|c| c.sha256.clone()),
            checksum_type: cksum.as_ref().and_then(|c| c.checksum_type.clone()),
            content_disposition: override_content_disposition.or(obj_meta.content_disposition),
            content_encoding: override_content_encoding.or(obj_meta.content_encoding),
            content_language: override_content_language.or(obj_meta.content_language),
            content_length: Some(content_length),
            content_range,
            content_type: override_content_type.or(content_type),
            expires: override_expires.map(|dt| dt.to_rfc2822()),
            e_tag: Some(obj_etag),
            last_modified: Some(obj_last_modified),
            metadata,
            object_lock_legal_hold_status: obj_meta
                .object_lock_legal_hold
                .filter(|&v| v)
                .map(|_| ObjectLockLegalHoldStatus::from("ON")),
            object_lock_mode: obj_meta
                .object_lock_mode
                .as_deref()
                .map(ObjectLockMode::from),
            object_lock_retain_until_date: obj_meta.object_lock_retain_until,
            parts_count: obj_parts_count.map(|n| n as i32),
            sse_customer_algorithm: obj_meta.sse_customer_algorithm,
            sse_customer_key_md5: obj_meta.sse_customer_key_md5,
            ssekms_key_id: obj_meta.sse_kms_key_id,
            server_side_encryption: obj_meta
                .sse_algorithm
                .as_deref()
                .map(ServerSideEncryption::from),
            storage_class: Some(StorageClass::from(obj_storage_class.as_str())),
            version_id: obj_version_id,
            ..GetObjectOutput::default()
        };
        Ok(output)
    }

    /// Head object (get metadata without body).
    #[allow(clippy::too_many_lines)]
    pub async fn handle_head_object(
        &self,
        input: HeadObjectInput,
    ) -> Result<HeadObjectOutput, S3Error> {
        let bucket_name = input.bucket;
        let key = input.key;
        let version_id_param = input.version_id;
        let checksum_mode = input.checksum_mode;

        // S3 response header overrides (from query parameters in presigned URLs).
        let override_cache_control = input.response_cache_control;
        let override_content_disposition = input.response_content_disposition;
        let override_content_encoding = input.response_content_encoding;
        let override_content_language = input.response_content_language;
        let override_content_type = input.response_content_type;
        let override_expires = input.response_expires;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let store = bucket.objects.read();
        let obj = if let Some(ref version_id) = version_id_param {
            store.get_version(&key, version_id).ok_or_else(|| {
                if store.is_delete_marker(&key, version_id) {
                    S3ServiceError::MethodNotAllowed
                        .into_s3_error()
                        .with_header("x-amz-delete-marker", "true")
                        .with_header("x-amz-version-id", version_id.clone())
                } else {
                    S3ServiceError::NoSuchVersion {
                        key: key.clone(),
                        version_id: version_id.clone(),
                    }
                    .into_s3_error()
                }
            })?
        } else {
            store
                .get(&key)
                .ok_or_else(|| S3ServiceError::NoSuchKey { key: key.clone() }.into_s3_error())?
        };

        let obj_version_id = if obj.version_id == "null" {
            None
        } else {
            Some(obj.version_id.clone())
        };

        let content_type = Some(
            obj.metadata
                .content_type
                .clone()
                .unwrap_or_else(|| "binary/octet-stream".to_owned()),
        );

        let metadata = if obj.metadata.user_metadata.is_empty() {
            HashMap::default()
        } else {
            obj.metadata.user_metadata.clone()
        };

        // Only return checksums when ChecksumMode=ENABLED (matching AWS behavior).
        let checksum_enabled = checksum_mode
            .as_ref()
            .is_some_and(|m| m.as_str() == "ENABLED");
        let cksum = if checksum_enabled {
            obj.checksum.as_ref().map(checksum_to_fields)
        } else {
            None
        };
        let output = HeadObjectOutput {
            accept_ranges: Some("bytes".to_owned()),
            cache_control: override_cache_control.or(obj.metadata.cache_control.clone()),
            checksum_crc32: cksum.as_ref().and_then(|c| c.crc32.clone()),
            checksum_crc32c: cksum.as_ref().and_then(|c| c.crc32c.clone()),
            checksum_crc64nvme: cksum.as_ref().and_then(|c| c.crc64nvme.clone()),
            checksum_sha1: cksum.as_ref().and_then(|c| c.sha1.clone()),
            checksum_sha256: cksum.as_ref().and_then(|c| c.sha256.clone()),
            checksum_type: cksum.as_ref().and_then(|c| c.checksum_type.clone()),
            content_disposition: override_content_disposition
                .or(obj.metadata.content_disposition.clone()),
            content_encoding: override_content_encoding.or(obj.metadata.content_encoding.clone()),
            content_language: override_content_language.or(obj.metadata.content_language.clone()),
            content_length: Some(obj.size as i64),
            content_type: override_content_type.or(content_type),
            expires: override_expires.map(|dt| dt.to_rfc2822()),
            e_tag: Some(obj.etag.clone()),
            last_modified: Some(obj.last_modified),
            metadata,
            object_lock_legal_hold_status: obj
                .metadata
                .object_lock_legal_hold
                .filter(|&v| v)
                .map(|_| ObjectLockLegalHoldStatus::from("ON")),
            object_lock_mode: obj
                .metadata
                .object_lock_mode
                .as_deref()
                .map(ObjectLockMode::from),
            object_lock_retain_until_date: obj.metadata.object_lock_retain_until,
            parts_count: obj.parts_count.map(|n| n as i32),
            sse_customer_algorithm: obj.metadata.sse_customer_algorithm.clone(),
            sse_customer_key_md5: obj.metadata.sse_customer_key_md5.clone(),
            ssekms_key_id: obj.metadata.sse_kms_key_id.clone(),
            server_side_encryption: obj
                .metadata
                .sse_algorithm
                .as_deref()
                .map(ServerSideEncryption::from),
            storage_class: Some(StorageClass::from(obj.storage_class.as_str())),
            version_id: obj_version_id,
            ..HeadObjectOutput::default()
        };
        Ok(output)
    }

    /// Delete a single object.
    pub async fn handle_delete_object(
        &self,
        input: DeleteObjectInput,
    ) -> Result<DeleteObjectOutput, S3Error> {
        let bucket_name = input.bucket;
        let key = input.key;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let (delete_marker_version_id, version_id_to_remove) =
            if let Some(version_id) = &input.version_id {
                // Delete a specific version.
                let bypass = input.bypass_governance_retention.unwrap_or(false);
                let mut store = bucket.objects.write();
                check_object_lock_for_delete(&store, &key, version_id, bypass)?;
                let removed = store.delete_version(&key, version_id);
                if let Some(ref version) = removed {
                    self.storage
                        .delete_object(&bucket_name, &key, version.version_id());
                }
                let is_dm = removed
                    .as_ref()
                    .is_some_and(crate::state::object::ObjectVersion::is_delete_marker);
                (is_dm, removed.map(|v| v.version_id().to_owned()))
            } else {
                // Delete without version: in versioned bucket, create delete marker.
                let mut store = bucket.objects.write();
                let (dm_id, _had) = store.delete_versioned(&key, &InternalOwner::default());
                if dm_id.is_none() {
                    // Un-versioned bucket: remove the storage data.
                    self.storage.delete_object(&bucket_name, &key, "null");
                }
                (dm_id.is_some(), dm_id)
            };

        debug!(bucket = %bucket_name, key = %key, "delete_object completed");

        Ok(DeleteObjectOutput {
            delete_marker: if delete_marker_version_id {
                Some(true)
            } else {
                None
            },
            request_charged: None,
            version_id: version_id_to_remove,
        })
    }

    /// Delete multiple objects (bulk delete).
    pub async fn handle_delete_objects(
        &self,
        input: DeleteObjectsInput,
    ) -> Result<DeleteObjectsOutput, S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let bypass = input.bypass_governance_retention.unwrap_or(false);
        let delete_request = input.delete;

        let objects = delete_request.objects;
        let quiet = delete_request.quiet.unwrap_or(false);

        let mut deleted: Vec<DeletedObject> = Vec::with_capacity(objects.len());
        let mut errors: Vec<ruststack_s3_model::types::Error> = Vec::new();

        for obj_id in objects {
            let key = obj_id.key;
            let version_id = obj_id.version_id;

            if let Some(ref vid) = version_id {
                // Delete a specific version.
                let mut store = bucket.objects.write();
                if let Err(lock_err) = check_object_lock_for_delete(&store, &key, vid, bypass) {
                    errors.push(ruststack_s3_model::types::Error {
                        code: Some(lock_err.code.as_str().to_owned()),
                        key: Some(key),
                        message: Some(lock_err.message),
                        version_id: Some(vid.clone()),
                    });
                    continue;
                }
                let removed = store.delete_version(&key, vid);
                if let Some(ref version) = removed {
                    self.storage
                        .delete_object(&bucket_name, &key, version.version_id());
                }
                let is_dm = removed
                    .as_ref()
                    .is_some_and(crate::state::object::ObjectVersion::is_delete_marker);
                deleted.push(DeletedObject {
                    delete_marker: if is_dm { Some(true) } else { None },
                    delete_marker_version_id: if is_dm { Some(vid.clone()) } else { None },
                    key: Some(key),
                    version_id: Some(vid.clone()),
                });
            } else {
                // Delete without version.
                let mut store = bucket.objects.write();
                let (dm_id, _had) = store.delete_versioned(&key, &InternalOwner::default());
                if dm_id.is_none() {
                    self.storage.delete_object(&bucket_name, &key, "null");
                }
                deleted.push(DeletedObject {
                    delete_marker: dm_id.as_ref().map(|_| true),
                    delete_marker_version_id: dm_id.clone(),
                    key: Some(key),
                    version_id: dm_id,
                });
            }
        }

        debug!(
            bucket = %bucket_name,
            deleted_count = deleted.len(),
            error_count = errors.len(),
            "delete_objects completed"
        );

        Ok(DeleteObjectsOutput {
            deleted: if quiet { Vec::new() } else { deleted },
            errors,
            request_charged: None,
        })
    }

    /// Copy an object from a source to a destination.
    #[allow(clippy::too_many_lines)]
    pub async fn handle_copy_object(
        &self,
        input: CopyObjectInput,
    ) -> Result<CopyObjectOutput, S3Error> {
        let dst_bucket = input.bucket.clone();
        let dst_key = input.key.clone();

        validate_object_key(&dst_key).map_err(S3ServiceError::into_s3_error)?;

        let (src_bucket, src_key, src_version_id) =
            parse_copy_source(&input.copy_source).map_err(S3ServiceError::into_s3_error)?;

        // Look up source object to get its metadata.
        // Keep this entire block synchronous -- no awaits while the lock is held.
        let (src_metadata, src_checksum, src_version_for_storage) = {
            let src_bucket_ref = self
                .state
                .get_bucket(&src_bucket)
                .map_err(S3ServiceError::into_s3_error)?;

            let src_store = src_bucket_ref.objects.read();
            let src_obj = if let Some(ref vid) = src_version_id {
                src_store.get_version(&src_key, vid).ok_or_else(|| {
                    S3ServiceError::NoSuchVersion {
                        key: src_key.clone(),
                        version_id: vid.clone(),
                    }
                    .into_s3_error()
                })?
            } else {
                src_store.get(&src_key).ok_or_else(|| {
                    S3ServiceError::NoSuchKey {
                        key: src_key.clone(),
                    }
                    .into_s3_error()
                })?
            };

            (
                src_obj.metadata.clone(),
                src_obj.checksum.clone(),
                src_obj.version_id.clone(),
            )
        };

        // Determine destination versioning.
        let dst_bucket_ref = self
            .state
            .get_bucket(&dst_bucket)
            .map_err(S3ServiceError::into_s3_error)?;

        let dst_version_id = if dst_bucket_ref.is_versioning_enabled() {
            crate::utils::generate_version_id()
        } else {
            "null".to_owned()
        };

        // Drop the bucket ref before await to avoid holding it across await points.
        drop(dst_bucket_ref);

        // Copy storage data.
        let write_result = self
            .storage
            .copy_object(
                &src_bucket,
                &src_key,
                &src_version_for_storage,
                &dst_bucket,
                &dst_key,
                &dst_version_id,
            )
            .await
            .map_err(|e| S3ServiceError::Internal(anyhow::anyhow!("{e}")).into_s3_error())?;

        // Determine metadata: use source metadata unless MetadataDirective is REPLACE.
        let metadata = if input
            .metadata_directive
            .as_ref()
            .is_some_and(|d| *d == MetadataDirective::Replace)
        {
            build_metadata_for_copy(&input)
        } else {
            src_metadata
        };

        let storage_class = input
            .storage_class
            .as_ref()
            .map_or_else(|| "STANDARD".to_owned(), StorageClass::as_str_owned);

        let now = Utc::now();
        let dst_obj = S3Object {
            key: dst_key.clone(),
            version_id: dst_version_id.clone(),
            etag: write_result.etag.clone(),
            size: write_result.size,
            last_modified: now,
            storage_class,
            metadata,
            owner: InternalOwner::default(),
            checksum: src_checksum,
            parts_count: None,
            part_etags: Vec::new(),
        };

        // Re-acquire the bucket ref to store the object.
        let dst_bucket_ref = self
            .state
            .get_bucket(&dst_bucket)
            .map_err(S3ServiceError::into_s3_error)?;
        {
            let mut store = dst_bucket_ref.objects.write();
            store.put(dst_obj);
        }

        debug!(
            src_bucket = %src_bucket,
            src_key = %src_key,
            dst_bucket = %dst_bucket,
            dst_key = %dst_key,
            "copy_object completed"
        );

        let real_version_id = if dst_version_id == "null" {
            None
        } else {
            Some(dst_version_id)
        };

        let copy_result = CopyObjectResult {
            e_tag: Some(write_result.etag),
            last_modified: Some(now),
            ..CopyObjectResult::default()
        };

        Ok(CopyObjectOutput {
            copy_object_result: Some(copy_result),
            copy_source_version_id: src_version_id,
            version_id: real_version_id,
            ..CopyObjectOutput::default()
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Helper trait to get an owned string from a [`StorageClass`] reference.
///
/// This avoids closure type inference issues when calling `as_str()` through
/// `Option::map_or_else`.
trait AsStrOwned {
    /// Return `as_str().to_owned()`.
    fn as_str_owned(&self) -> String;
}

impl AsStrOwned for StorageClass {
    fn as_str_owned(&self) -> String {
        self.as_str().to_owned()
    }
}

/// Build [`ObjectMetadata`] from a [`PutObjectInput`].
fn build_metadata(input: &PutObjectInput) -> ObjectMetadata {
    let user_metadata = input.metadata.clone();

    // Parse tagging from the x-amz-tagging header.
    let tagging = input
        .tagging
        .as_deref()
        .map(parse_tagging_header)
        .unwrap_or_default();

    let acl = parse_acl(input.acl.as_ref());

    ObjectMetadata {
        content_type: input.content_type.clone(),
        content_encoding: input.content_encoding.clone(),
        content_disposition: input.content_disposition.clone(),
        content_language: input.content_language.clone(),
        cache_control: input.cache_control.clone(),
        expires: input.expires.clone(),
        user_metadata,
        sse_algorithm: input
            .server_side_encryption
            .as_ref()
            .map(|sse: &ServerSideEncryption| sse.as_str().to_owned()),
        sse_kms_key_id: input.ssekms_key_id.clone(),
        sse_bucket_key_enabled: input.bucket_key_enabled,
        sse_customer_algorithm: input.sse_customer_algorithm.clone(),
        sse_customer_key_md5: input.sse_customer_key_md5.clone(),
        tagging,
        acl,
        object_lock_mode: input
            .object_lock_mode
            .as_ref()
            .map(|m: &ObjectLockMode| m.as_str().to_owned()),
        object_lock_retain_until: input.object_lock_retain_until_date,
        object_lock_legal_hold: input
            .object_lock_legal_hold_status
            .as_ref()
            .map(|s: &ObjectLockLegalHoldStatus| s.as_str() == "ON"),
    }
}

/// Build [`ObjectMetadata`] for a copy operation with REPLACE directive.
fn build_metadata_for_copy(input: &CopyObjectInput) -> ObjectMetadata {
    let user_metadata = input.metadata.clone();

    let tagging = input
        .tagging
        .as_deref()
        .map(parse_tagging_header)
        .unwrap_or_default();

    let acl = parse_acl(input.acl.as_ref());

    ObjectMetadata {
        content_type: input.content_type.clone(),
        content_encoding: input.content_encoding.clone(),
        content_disposition: input.content_disposition.clone(),
        content_language: input.content_language.clone(),
        cache_control: input.cache_control.clone(),
        expires: None,
        user_metadata,
        sse_algorithm: input
            .server_side_encryption
            .as_ref()
            .map(|sse: &ServerSideEncryption| sse.as_str().to_owned()),
        sse_kms_key_id: input.ssekms_key_id.clone(),
        sse_bucket_key_enabled: input.bucket_key_enabled,
        sse_customer_algorithm: input.sse_customer_algorithm.clone(),
        sse_customer_key_md5: input.sse_customer_key_md5.clone(),
        tagging,
        acl,
        object_lock_mode: input
            .object_lock_mode
            .as_ref()
            .map(|m: &ObjectLockMode| m.as_str().to_owned()),
        object_lock_retain_until: input.object_lock_retain_until_date,
        object_lock_legal_hold: input
            .object_lock_legal_hold_status
            .as_ref()
            .map(|s: &ObjectLockLegalHoldStatus| s.as_str() == "ON"),
    }
}

/// Parse an optional [`ObjectCannedACL`] into our internal [`CannedAcl`].
fn parse_acl(acl: Option<&ObjectCannedACL>) -> CannedAcl {
    acl.and_then(|a| a.as_str().parse::<CannedAcl>().ok())
        .unwrap_or_default()
}

/// Parse the `x-amz-tagging` URL-encoded query string into tag pairs.
pub(super) fn parse_tagging_header(tagging: &str) -> Vec<(String, String)> {
    tagging
        .split('&')
        .filter(|s| !s.is_empty())
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            let key = percent_encoding::percent_decode_str(k)
                .decode_utf8()
                .ok()?
                .to_string();
            let value = percent_encoding::percent_decode_str(v)
                .decode_utf8()
                .ok()?
                .to_string();
            Some((key, value))
        })
        .collect()
}

/// Holds the individual checksum fields for populating output structs.
struct ChecksumFields {
    crc32: Option<String>,
    crc32c: Option<String>,
    crc64nvme: Option<String>,
    sha1: Option<String>,
    sha256: Option<String>,
    checksum_type: Option<ChecksumType>,
}

/// Map a [`ChecksumData`] to individual output fields.
fn checksum_to_fields(checksum: &ChecksumData) -> ChecksumFields {
    let mut fields = ChecksumFields {
        crc32: None,
        crc32c: None,
        crc64nvme: None,
        sha1: None,
        sha256: None,
        checksum_type: None,
    };
    match checksum.algorithm.as_str() {
        "CRC32" => fields.crc32 = Some(checksum.value.clone()),
        "CRC32C" => fields.crc32c = Some(checksum.value.clone()),
        "CRC64NVME" => fields.crc64nvme = Some(checksum.value.clone()),
        "SHA1" => fields.sha1 = Some(checksum.value.clone()),
        "SHA256" => fields.sha256 = Some(checksum.value.clone()),
        _ => {}
    }
    fields.checksum_type = Some(match checksum.checksum_type.as_str() {
        "COMPOSITE" => ChecksumType::Composite,
        _ => ChecksumType::FullObject,
    });
    fields
}

/// Extract checksum data from a [`PutObjectInput`] if any checksum fields are
/// set.
///
/// Returns an error if multiple checksum fields are set (only one is allowed
/// per request, matching AWS behavior).
fn extract_checksum_from_put(
    input: &PutObjectInput,
) -> Result<Option<ChecksumData>, S3ServiceError> {
    let candidates: [(&str, &Option<String>); 5] = [
        ("CRC32", &input.checksum_crc32),
        ("CRC32C", &input.checksum_crc32c),
        ("CRC64NVME", &input.checksum_crc64nvme),
        ("SHA1", &input.checksum_sha1),
        ("SHA256", &input.checksum_sha256),
    ];
    let found: Vec<_> = candidates.iter().filter(|(_, v)| v.is_some()).collect();
    if found.len() > 1 {
        return Err(S3ServiceError::InvalidArgument {
            message: "Only one checksum value can be provided per request".to_owned(),
        });
    }
    Ok(found.into_iter().next().map(|(alg, val)| ChecksumData {
        algorithm: (*alg).to_owned(),
        value: val.as_ref().unwrap_or(&String::new()).clone(),
        checksum_type: "FULL_OBJECT".to_owned(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_parse_copy_source_simple() {
        let (bucket, key, vid) = parse_copy_source("my-bucket/my-key").unwrap();
        assert_eq!(bucket, "my-bucket");
        assert_eq!(key, "my-key");
        assert!(vid.is_none());
    }

    #[test]
    fn test_should_parse_copy_source_with_leading_slash() {
        let (bucket, key, vid) = parse_copy_source("/my-bucket/my-key").unwrap();
        assert_eq!(bucket, "my-bucket");
        assert_eq!(key, "my-key");
        assert!(vid.is_none());
    }

    #[test]
    fn test_should_parse_copy_source_with_version_id() {
        let (bucket, key, vid) = parse_copy_source("/my-bucket/my-key?versionId=abc123").unwrap();
        assert_eq!(bucket, "my-bucket");
        assert_eq!(key, "my-key");
        assert_eq!(vid.as_deref(), Some("abc123"));
    }

    #[test]
    fn test_should_parse_copy_source_with_nested_key() {
        let (bucket, key, vid) = parse_copy_source("bucket/path/to/key").unwrap();
        assert_eq!(bucket, "bucket");
        assert_eq!(key, "path/to/key");
        assert!(vid.is_none());
    }

    #[test]
    fn test_should_parse_copy_source_with_encoded_key() {
        let (bucket, key, vid) = parse_copy_source("bucket/path%20to/key%2B1").unwrap();
        assert_eq!(bucket, "bucket");
        assert_eq!(key, "path to/key+1");
        assert!(vid.is_none());
    }

    #[test]
    fn test_should_reject_copy_source_no_key() {
        assert!(parse_copy_source("bucket-only").is_err());
    }

    #[test]
    fn test_should_reject_copy_source_empty_bucket() {
        assert!(parse_copy_source("/").is_err());
    }

    #[test]
    fn test_should_reject_copy_source_empty_key() {
        assert!(parse_copy_source("bucket/").is_err());
    }

    #[test]
    fn test_should_parse_tagging_header_basic() {
        let tags = parse_tagging_header("key1=value1&key2=value2");
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0], ("key1".to_owned(), "value1".to_owned()));
        assert_eq!(tags[1], ("key2".to_owned(), "value2".to_owned()));
    }

    #[test]
    fn test_should_parse_tagging_header_encoded() {
        let tags = parse_tagging_header("key%201=value%201");
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0], ("key 1".to_owned(), "value 1".to_owned()));
    }

    #[test]
    fn test_should_parse_tagging_header_empty() {
        let tags = parse_tagging_header("");
        assert!(tags.is_empty());
    }

    #[test]
    fn test_should_parse_tagging_header_no_value() {
        let tags = parse_tagging_header("key1");
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0], ("key1".to_owned(), String::new()));
    }
}
