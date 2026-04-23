//! Multipart upload operation handlers.
//!
//! Implements `create_multipart_upload`, `upload_part`, `upload_part_copy`,
//! `complete_multipart_upload`, `abort_multipart_upload`, `list_parts`,
//! and `list_multipart_uploads`.

use std::str::FromStr;

use chrono::Utc;
use rustack_s3_model::{
    error::{S3Error, S3ErrorCode},
    input::{
        AbortMultipartUploadInput, CompleteMultipartUploadInput, CreateMultipartUploadInput,
        ListMultipartUploadsInput, ListPartsInput, UploadPartCopyInput, UploadPartInput,
    },
    output::{
        AbortMultipartUploadOutput, CompleteMultipartUploadOutput, CreateMultipartUploadOutput,
        ListMultipartUploadsOutput, ListPartsOutput, UploadPartCopyOutput, UploadPartOutput,
    },
    types::{
        ChecksumAlgorithm, ChecksumType, CopyPartResult, Initiator,
        MultipartUpload as ModelMultipartUpload, Part, StorageClass,
    },
};
use tracing::debug;

use crate::{
    checksums::{
        ChecksumAlgorithm as CoreChecksumAlgorithm, compute_checksum, compute_composite_checksum,
    },
    error::S3ServiceError,
    provider::RustackS3,
    state::{
        multipart::{MultipartUpload, UploadPart},
        object::{ChecksumData, ObjectMetadata, Owner as InternalOwner, S3Object},
    },
    utils::{generate_upload_id, parse_copy_source},
    validation::{validate_content_md5, validate_object_key},
};

/// Minimum part size for multipart uploads (5 MB). All parts except the last
/// must be at least this size per the S3 specification.
const MIN_PART_SIZE: u64 = 5 * 1024 * 1024;

use super::bucket::to_model_owner;

// AWS S3 DTOs use signed integers (i32/i64) for inherently non-negative values.
// These handler methods must remain async for consistency.
#[allow(
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::unused_async
)]
impl RustackS3 {
    /// Create a new multipart upload.
    #[allow(clippy::too_many_lines)]
    pub async fn handle_create_multipart_upload(
        &self,
        input: CreateMultipartUploadInput,
    ) -> Result<CreateMultipartUploadOutput, S3Error> {
        let bucket_name = input.bucket;
        let key = input.key;

        validate_object_key(&key).map_err(S3ServiceError::into_s3_error)?;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let upload_id = generate_upload_id();

        // Build metadata from the request.
        let metadata = ObjectMetadata {
            content_type: input.content_type.clone(),
            content_encoding: input.content_encoding.clone(),
            content_disposition: input.content_disposition.clone(),
            content_language: input.content_language.clone(),
            cache_control: input.cache_control.clone(),
            expires: None,
            user_metadata: input.metadata.clone(),
            sse_algorithm: input
                .server_side_encryption
                .as_ref()
                .map(|s| s.as_str().to_owned()),
            sse_kms_key_id: input.ssekms_key_id.clone(),
            sse_bucket_key_enabled: input.bucket_key_enabled,
            sse_customer_algorithm: input.sse_customer_algorithm.clone(),
            sse_customer_key_md5: input.sse_customer_key_md5.clone(),
            tagging: input
                .tagging
                .as_ref()
                .map(|t| super::object::parse_tagging_header(t))
                .unwrap_or_default(),
            acl: input
                .acl
                .as_ref()
                .and_then(|a| a.as_str().parse().ok())
                .unwrap_or_default(),
            object_lock_mode: input
                .object_lock_mode
                .as_ref()
                .map(|m| m.as_str().to_owned()),
            object_lock_retain_until: input.object_lock_retain_until_date,
            object_lock_legal_hold: input
                .object_lock_legal_hold_status
                .as_ref()
                .map(|s| s.as_str() == "ON"),
        };

        let mut upload = MultipartUpload::new(
            upload_id.clone(),
            key.clone(),
            InternalOwner::default(),
            metadata,
        );

        upload.storage_class = input
            .storage_class
            .as_ref()
            .map_or_else(|| "STANDARD".to_owned(), |s| s.as_str().to_owned());

        upload.checksum_algorithm = input
            .checksum_algorithm
            .as_ref()
            .map(|a| a.as_str().to_owned());

        // Determine checksum type: CRC64NVME forces FULL_OBJECT, SHA-1/SHA-256
        // force COMPOSITE, CRC32/CRC32C default to COMPOSITE but allow override.
        if let Some(ref algo_str) = upload.checksum_algorithm {
            let requested_type = input
                .checksum_type
                .as_ref()
                .map(|ct| ct.as_str().to_owned());
            upload.checksum_type = Some(match algo_str.as_str() {
                "CRC64NVME" => "FULL_OBJECT".to_owned(),
                "SHA1" | "SHA256" => "COMPOSITE".to_owned(),
                _ => requested_type.unwrap_or_else(|| "COMPOSITE".to_owned()),
            });
        }

        upload.sse_algorithm = input
            .server_side_encryption
            .as_ref()
            .map(|s| s.as_str().to_owned());

        upload.sse_kms_key_id.clone_from(&input.ssekms_key_id);

        let output_checksum_type = upload.checksum_type.as_ref().map(|ct| match ct.as_str() {
            "FULL_OBJECT" => ChecksumType::FullObject,
            _ => ChecksumType::Composite,
        });

        bucket.multipart_uploads.insert(upload_id.clone(), upload);

        debug!(
            bucket = %bucket_name,
            key = %key,
            upload_id = %upload_id,
            "create_multipart_upload completed"
        );

        Ok(CreateMultipartUploadOutput {
            abort_date: None,
            abort_rule_id: None,
            bucket: Some(bucket_name),
            bucket_key_enabled: None,
            checksum_algorithm: input.checksum_algorithm,
            checksum_type: output_checksum_type,
            key: Some(key),
            request_charged: None,
            sse_customer_algorithm: None,
            sse_customer_key_md5: None,
            ssekms_encryption_context: None,
            ssekms_key_id: None,
            server_side_encryption: None,
            upload_id: Some(upload_id),
        })
    }

    /// Upload a single part of a multipart upload.
    #[allow(clippy::too_many_lines)]
    pub async fn handle_upload_part(
        &self,
        mut input: UploadPartInput,
    ) -> Result<UploadPartOutput, S3Error> {
        // Extract checksum before moving fields out of input.
        let part_checksum = extract_checksum_from_part(&input)?;

        let bucket_name = input.bucket;
        let key = input.key;
        let upload_id = input.upload_id;
        let part_number = input.part_number;

        if !(1..=10_000).contains(&part_number) {
            return Err(S3Error::invalid_argument(
                "Part number must be between 1 and 10000",
            ));
        }

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        // Verify the upload exists and get its checksum algorithm.
        let upload_checksum_algorithm = {
            let upload_ref = bucket.multipart_uploads.get(&upload_id).ok_or_else(|| {
                S3ServiceError::NoSuchUpload {
                    upload_id: upload_id.clone(),
                }
                .into_s3_error()
            })?;
            upload_ref.checksum_algorithm.clone()
        };

        // Collect body data.
        let body_data = input.body.take().map(|b| b.data).unwrap_or_default();

        // Validate Content-MD5 if provided.
        validate_content_md5(input.content_md5.as_deref(), &body_data)
            .map_err(S3ServiceError::into_s3_error)?;

        // If the multipart upload has a checksum algorithm, validate the part
        // checksum and compute server-side if not provided.
        let checksum = if let Some(ref algo_str) = upload_checksum_algorithm {
            if let Ok(algo) = CoreChecksumAlgorithm::from_str(algo_str) {
                let computed = compute_checksum(algo, &body_data);
                if let Some(ref client_cksum) = part_checksum {
                    // Validate algorithm matches.
                    if !client_cksum.algorithm.eq_ignore_ascii_case(algo_str) {
                        return Err(S3ServiceError::InvalidArgument {
                            message: format!(
                                "Checksum algorithm mismatch: expected {algo_str}, got {}",
                                client_cksum.algorithm
                            ),
                        }
                        .into_s3_error());
                    }
                    // Validate value.
                    if client_cksum.value != computed {
                        return Err(S3ServiceError::BadDigest.into_s3_error());
                    }
                }
                Some(ChecksumData {
                    algorithm: algo_str.clone(),
                    value: computed,
                    checksum_type: "FULL_OBJECT".to_owned(),
                })
            } else {
                part_checksum
            }
        } else {
            part_checksum
        };

        // Write part to storage.
        let write_result = self
            .storage
            .write_part(&bucket_name, &upload_id, part_number as u32, body_data)
            .await
            .map_err(|e| S3ServiceError::Internal(anyhow::anyhow!("{e}")).into_s3_error())?;

        // Build checksum output fields.
        let (out_crc32, out_crc32c, out_crc64nvme, out_sha1, out_sha256) =
            checksum_to_output_fields(checksum.as_ref());

        // Record the part metadata.
        let part = UploadPart {
            part_number: part_number as u32,
            etag: write_result.etag.clone(),
            size: write_result.size,
            last_modified: Utc::now(),
            checksum,
        };

        if let Some(mut upload) = bucket.multipart_uploads.get_mut(&upload_id) {
            upload.put_part(part);
        }

        debug!(
            bucket = %bucket_name,
            key = %key,
            upload_id = %upload_id,
            part_number,
            "upload_part completed"
        );

        Ok(UploadPartOutput {
            bucket_key_enabled: None,
            checksum_crc32: out_crc32,
            checksum_crc32c: out_crc32c,
            checksum_crc64nvme: out_crc64nvme,
            checksum_sha1: out_sha1,
            checksum_sha256: out_sha256,
            e_tag: Some(write_result.etag),
            request_charged: None,
            sse_customer_algorithm: None,
            sse_customer_key_md5: None,
            ssekms_key_id: None,
            server_side_encryption: None,
        })
    }

    /// Upload a part by copying from an existing object.
    pub async fn handle_upload_part_copy(
        &self,
        input: UploadPartCopyInput,
    ) -> Result<UploadPartCopyOutput, S3Error> {
        let bucket_name = input.bucket;
        let upload_id = input.upload_id;
        let part_number = input.part_number;

        let (src_bucket, src_key, src_version_id) =
            parse_copy_source(&input.copy_source).map_err(S3ServiceError::into_s3_error)?;

        // Read source object data.
        let src_vid = src_version_id.as_deref().unwrap_or("null");
        let data = self
            .storage
            .read_object(&src_bucket, &src_key, src_vid, None)
            .await
            .map_err(|e| S3ServiceError::Internal(anyhow::anyhow!("{e}")).into_s3_error())?;

        // Write as part.
        let write_result = self
            .storage
            .write_part(&bucket_name, &upload_id, part_number as u32, data)
            .await
            .map_err(|e| S3ServiceError::Internal(anyhow::anyhow!("{e}")).into_s3_error())?;

        // Record the part metadata.
        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let part = UploadPart {
            part_number: part_number as u32,
            etag: write_result.etag.clone(),
            size: write_result.size,
            last_modified: Utc::now(),
            checksum: None,
        };

        if let Some(mut upload) = bucket.multipart_uploads.get_mut(&upload_id) {
            upload.put_part(part);
        }

        let copy_result = CopyPartResult {
            checksum_crc32: None,
            checksum_crc32c: None,
            checksum_crc64nvme: None,
            checksum_sha1: None,
            checksum_sha256: None,
            e_tag: Some(write_result.etag),
            last_modified: Some(Utc::now()),
        };

        Ok(UploadPartCopyOutput {
            bucket_key_enabled: None,
            copy_part_result: Some(copy_result),
            copy_source_version_id: src_version_id,
            request_charged: None,
            sse_customer_algorithm: None,
            sse_customer_key_md5: None,
            ssekms_key_id: None,
            server_side_encryption: None,
        })
    }

    /// Complete a multipart upload by assembling parts into the final object.
    #[allow(clippy::too_many_lines)]
    pub async fn handle_complete_multipart_upload(
        &self,
        input: CompleteMultipartUploadInput,
    ) -> Result<CompleteMultipartUploadOutput, S3Error> {
        let bucket_name = input.bucket;
        let key = input.key;
        let upload_id = input.upload_id;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        // Get the upload.
        let upload = bucket
            .multipart_uploads
            .get(&upload_id)
            .ok_or_else(|| {
                S3ServiceError::NoSuchUpload {
                    upload_id: upload_id.clone(),
                }
                .into_s3_error()
            })?
            .clone();

        // Extract the requested part list.
        let requested_parts = input
            .multipart_upload
            .map(|mu| mu.parts)
            .unwrap_or_default();

        // Validate parts are in order and exist.
        let mut part_numbers: Vec<u32> = Vec::with_capacity(requested_parts.len());
        let mut last_num = 0i32;

        for cp in &requested_parts {
            let part_num = cp.part_number.ok_or_else(|| {
                S3Error::with_message(S3ErrorCode::InvalidArgument, "Part number is required")
            })?;

            if part_num <= last_num {
                return Err(S3ServiceError::InvalidPartOrder.into_s3_error());
            }
            last_num = part_num;

            let part_num_u32 = u32::try_from(part_num).map_err(|_| {
                S3Error::with_message(S3ErrorCode::InvalidArgument, "Invalid part number")
            })?;

            // Verify the part exists in our upload record.
            upload
                .get_part(part_num_u32)
                .ok_or_else(|| S3ServiceError::InvalidPart.into_s3_error())?;

            part_numbers.push(part_num_u32);
        }

        // Validate minimum part size for all parts except the last one.
        // S3 requires each part (except the final one) to be at least 5 MB.
        if part_numbers.len() > 1 {
            for &num in &part_numbers[..part_numbers.len() - 1] {
                if let Some(part) = upload.get_part(num) {
                    if part.size < MIN_PART_SIZE {
                        return Err(S3Error::with_message(
                            S3ErrorCode::EntityTooSmall,
                            format!(
                                "Your proposed upload is smaller than the minimum allowed size. \
                                 Part {num} has size {} bytes, minimum is {MIN_PART_SIZE}",
                                part.size
                            ),
                        ));
                    }
                }
            }
        }

        // Determine version ID.
        let version_id = if bucket.is_versioning_enabled() {
            crate::utils::generate_version_id()
        } else {
            "null".to_owned()
        };

        // Assemble parts in storage.
        let (write_result, _part_md5s) = self
            .storage
            .complete_multipart(&bucket_name, &upload_id, &key, &version_id, &part_numbers)
            .await
            .map_err(|e| S3ServiceError::Internal(anyhow::anyhow!("{e}")).into_s3_error())?;

        // Compute the combined checksum for the final object if the multipart
        // upload was created with a checksum algorithm.
        //
        // FULL_OBJECT: the checksum of the concatenated object bytes (no `-N`
        // suffix). AWS CLI verifies downloaded multipart objects by recomputing
        // this over the body and comparing to the header value.
        //
        // COMPOSITE: the hash of concatenated per-part checksums, with `-N`
        // suffix. Used for SHA-1/SHA-256 multipart uploads.
        let final_checksum = if let Some(ref algo_str) = upload.checksum_algorithm {
            if let Ok(algo) = CoreChecksumAlgorithm::from_str(algo_str) {
                let checksum_type_str = upload.checksum_type.as_deref().unwrap_or("COMPOSITE");

                let value = if checksum_type_str == "FULL_OBJECT" {
                    let assembled = self
                        .storage
                        .read_object(&bucket_name, &key, &version_id, None)
                        .await
                        .map_err(|e| {
                            S3ServiceError::Internal(anyhow::anyhow!("{e}")).into_s3_error()
                        })?;
                    compute_checksum(algo, &assembled)
                } else {
                    // Collect part checksums in order for composite calculation.
                    let part_checksums: Vec<String> = part_numbers
                        .iter()
                        .filter_map(|&num| {
                            upload
                                .get_part(num)
                                .and_then(|p| p.checksum.as_ref())
                                .map(|c| c.value.clone())
                        })
                        .collect();
                    compute_composite_checksum(algo, &part_checksums)
                };

                Some(ChecksumData {
                    algorithm: algo_str.clone(),
                    value,
                    checksum_type: checksum_type_str.to_owned(),
                })
            } else {
                None
            }
        } else {
            None
        };

        let (out_crc32, out_crc32c, out_crc64nvme, out_sha1, out_sha256) =
            checksum_to_output_fields(final_checksum.as_ref());

        let out_checksum_type = final_checksum
            .as_ref()
            .map(|c| match c.checksum_type.as_str() {
                "FULL_OBJECT" => ChecksumType::FullObject,
                _ => ChecksumType::Composite,
            });

        // Build the final object.
        let obj = S3Object {
            key: key.clone(),
            version_id: version_id.clone(),
            etag: write_result.etag.clone(),
            size: write_result.size,
            last_modified: Utc::now(),
            storage_class: upload.storage_class.clone(),
            metadata: upload.metadata.clone(),
            owner: upload.owner.clone(),
            checksum: final_checksum,
            parts_count: Some(part_numbers.len() as u32),
            part_etags: requested_parts
                .iter()
                .filter_map(|p| p.e_tag.clone())
                .collect(),
        };

        {
            let mut store = bucket.objects.write();
            store.put(obj);
        }

        // Remove the completed upload.
        bucket.multipart_uploads.remove(&upload_id);

        debug!(
            bucket = %bucket_name,
            key = %key,
            upload_id = %upload_id,
            parts = part_numbers.len(),
            "complete_multipart_upload completed"
        );

        let real_version_id = if version_id == "null" {
            None
        } else {
            Some(version_id)
        };

        Ok(CompleteMultipartUploadOutput {
            bucket: Some(bucket_name.clone()),
            bucket_key_enabled: None,
            checksum_crc32: out_crc32,
            checksum_crc32c: out_crc32c,
            checksum_crc64nvme: out_crc64nvme,
            checksum_sha1: out_sha1,
            checksum_sha256: out_sha256,
            checksum_type: out_checksum_type,
            e_tag: Some(write_result.etag),
            expiration: None,
            key: Some(key),
            location: Some(format!("http://s3.amazonaws.com/{bucket_name}")),
            request_charged: None,
            ssekms_key_id: None,
            server_side_encryption: None,
            version_id: real_version_id,
        })
    }

    /// Abort a multipart upload.
    pub async fn handle_abort_multipart_upload(
        &self,
        input: AbortMultipartUploadInput,
    ) -> Result<AbortMultipartUploadOutput, S3Error> {
        let bucket_name = input.bucket;
        let upload_id = input.upload_id;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        // Remove the upload metadata (idempotent: no error if already gone).
        bucket.multipart_uploads.remove(&upload_id);

        // Clean up storage parts.
        self.storage.abort_multipart(&bucket_name, &upload_id);

        debug!(
            bucket = %bucket_name,
            upload_id = %upload_id,
            "abort_multipart_upload completed"
        );

        Ok(AbortMultipartUploadOutput {
            request_charged: None,
        })
    }

    /// List parts that have been uploaded for a multipart upload.
    pub async fn handle_list_parts(
        &self,
        input: ListPartsInput,
    ) -> Result<ListPartsOutput, S3Error> {
        let bucket_name = input.bucket;
        let key = input.key;
        let upload_id = input.upload_id;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let upload = bucket.multipart_uploads.get(&upload_id).ok_or_else(|| {
            S3ServiceError::NoSuchUpload {
                upload_id: upload_id.clone(),
            }
            .into_s3_error()
        })?;

        let max_parts = input.max_parts.unwrap_or(1000) as usize;
        let part_number_marker: u32 = input
            .part_number_marker
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        let all_parts: Vec<&UploadPart> = upload
            .parts
            .values()
            .filter(|p| p.part_number > part_number_marker)
            .collect();

        let is_truncated = all_parts.len() > max_parts;
        let parts_to_return = &all_parts[..all_parts.len().min(max_parts)];

        let s3_parts: Vec<Part> = parts_to_return
            .iter()
            .map(|p| {
                let (crc32, crc32c, crc64nvme, sha1, sha256) =
                    checksum_to_output_fields(p.checksum.as_ref());
                Part {
                    checksum_crc32: crc32,
                    checksum_crc32c: crc32c,
                    checksum_crc64nvme: crc64nvme,
                    checksum_sha1: sha1,
                    checksum_sha256: sha256,
                    e_tag: Some(p.etag.clone()),
                    last_modified: Some(p.last_modified),
                    part_number: Some(p.part_number as i32),
                    size: Some(p.size as i64),
                }
            })
            .collect();

        let next_marker = if is_truncated {
            s3_parts.last().and_then(|p| p.part_number)
        } else {
            None
        };

        let owner = to_model_owner(&upload.owner);

        Ok(ListPartsOutput {
            abort_date: None,
            abort_rule_id: None,
            bucket: Some(bucket_name),
            checksum_algorithm: upload
                .checksum_algorithm
                .as_ref()
                .map(|a| ChecksumAlgorithm::from(a.as_str())),
            checksum_type: upload.checksum_type.as_ref().map(|ct| match ct.as_str() {
                "FULL_OBJECT" => ChecksumType::FullObject,
                _ => ChecksumType::Composite,
            }),
            initiator: Some(Initiator {
                display_name: Some(upload.owner.display_name.clone()),
                id: Some(upload.owner.id.clone()),
            }),
            is_truncated: Some(is_truncated),
            key: Some(key),
            max_parts: Some(max_parts as i32),
            next_part_number_marker: next_marker.map(|n| n.to_string()),
            owner: Some(owner),
            part_number_marker: Some(part_number_marker.to_string()),
            parts: s3_parts,
            request_charged: None,
            storage_class: Some(StorageClass::from(upload.storage_class.as_str())),
            upload_id: Some(upload_id),
        })
    }

    /// List in-progress multipart uploads for a bucket.
    pub async fn handle_list_multipart_uploads(
        &self,
        input: ListMultipartUploadsInput,
    ) -> Result<ListMultipartUploadsOutput, S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let prefix = input.prefix.unwrap_or_default();
        let max_uploads = input.max_uploads.unwrap_or(1000) as usize;

        let mut uploads: Vec<MultipartUpload> = bucket
            .multipart_uploads
            .iter()
            .filter(|entry| entry.key.starts_with(&prefix))
            .map(|entry| entry.value().clone())
            .collect();

        // Sort by key then by initiated time.
        uploads.sort_by(|a, b| a.key.cmp(&b.key).then(a.initiated.cmp(&b.initiated)));

        let is_truncated = uploads.len() > max_uploads;
        let uploads_to_return = &uploads[..uploads.len().min(max_uploads)];

        let s3_uploads: Vec<ModelMultipartUpload> = uploads_to_return
            .iter()
            .map(|u| ModelMultipartUpload {
                checksum_algorithm: u
                    .checksum_algorithm
                    .as_ref()
                    .map(|a| ChecksumAlgorithm::from(a.as_str())),
                checksum_type: None,
                initiated: Some(u.initiated),
                initiator: Some(Initiator {
                    display_name: Some(u.owner.display_name.clone()),
                    id: Some(u.owner.id.clone()),
                }),
                key: Some(u.key.clone()),
                owner: Some(to_model_owner(&u.owner)),
                storage_class: Some(StorageClass::from(u.storage_class.as_str())),
                upload_id: Some(u.upload_id.clone()),
            })
            .collect();

        let next_key_marker = if is_truncated {
            s3_uploads.last().and_then(|u| u.key.clone())
        } else {
            None
        };
        let next_upload_id_marker = if is_truncated {
            s3_uploads.last().and_then(|u| u.upload_id.clone())
        } else {
            None
        };

        Ok(ListMultipartUploadsOutput {
            bucket: Some(bucket_name),
            common_prefixes: Vec::new(),
            delimiter: input.delimiter,
            encoding_type: None,
            is_truncated: Some(is_truncated),
            key_marker: input.key_marker,
            max_uploads: Some(max_uploads as i32),
            next_key_marker,
            next_upload_id_marker,
            prefix: Some(prefix),
            request_charged: None,
            upload_id_marker: input.upload_id_marker,
            uploads: s3_uploads,
        })
    }
}

/// Per-algorithm checksum output fields.
type ChecksumOutputFields = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// Extract a checksum from an [`UploadPartInput`] if any checksum fields are set.
///
/// Returns at most one checksum. If multiple checksum fields are set, returns
/// an error.
#[allow(clippy::result_large_err)]
fn extract_checksum_from_part(input: &UploadPartInput) -> Result<Option<ChecksumData>, S3Error> {
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
        }
        .into_s3_error());
    }
    Ok(found.into_iter().next().map(|(alg, val)| ChecksumData {
        algorithm: (*alg).to_owned(),
        value: val.as_ref().unwrap_or(&String::new()).clone(),
        checksum_type: "FULL_OBJECT".to_owned(),
    }))
}

/// Map an optional [`ChecksumData`] to individual output fields for the five
/// supported algorithms.
fn checksum_to_output_fields(checksum: Option<&ChecksumData>) -> ChecksumOutputFields {
    let Some(c) = checksum else {
        return (None, None, None, None, None);
    };
    match c.algorithm.as_str() {
        "CRC32" => (Some(c.value.clone()), None, None, None, None),
        "CRC32C" => (None, Some(c.value.clone()), None, None, None),
        "CRC64NVME" => (None, None, Some(c.value.clone()), None, None),
        "SHA1" => (None, None, None, Some(c.value.clone()), None),
        "SHA256" => (None, None, None, None, Some(c.value.clone())),
        _ => (None, None, None, None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_parse_copy_source_basic() {
        let (bucket, key, version) = parse_copy_source("/my-bucket/my-key").unwrap();
        assert_eq!(bucket, "my-bucket");
        assert_eq!(key, "my-key");
        assert!(version.is_none());
    }

    #[test]
    fn test_should_parse_copy_source_without_leading_slash() {
        let (bucket, key, version) = parse_copy_source("my-bucket/my-key").unwrap();
        assert_eq!(bucket, "my-bucket");
        assert_eq!(key, "my-key");
        assert!(version.is_none());
    }

    #[test]
    fn test_should_parse_copy_source_with_version_id() {
        let (bucket, key, version) =
            parse_copy_source("/my-bucket/my-key?versionId=abc123").unwrap();
        assert_eq!(bucket, "my-bucket");
        assert_eq!(key, "my-key");
        assert_eq!(version.as_deref(), Some("abc123"));
    }

    #[test]
    fn test_should_parse_copy_source_with_nested_key() {
        let (bucket, key, version) =
            parse_copy_source("/my-bucket/path/to/my-key?versionId=v1").unwrap();
        assert_eq!(bucket, "my-bucket");
        assert_eq!(key, "path/to/my-key");
        assert_eq!(version.as_deref(), Some("v1"));
    }

    #[test]
    fn test_should_fail_on_invalid_copy_source() {
        let result = parse_copy_source("no-slash");
        assert!(result.is_err());
    }

    #[test]
    fn test_should_fail_on_empty_bucket() {
        let result = parse_copy_source("/");
        assert!(result.is_err());
    }
}
