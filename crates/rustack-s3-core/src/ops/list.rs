//! List operation handlers.
//!
//! Implements `list_objects` (v1), `list_objects_v2`, and `list_object_versions`.

use rustack_s3_model::{
    error::S3Error,
    input::{ListObjectVersionsInput, ListObjectsInput, ListObjectsV2Input},
    output::{ListObjectVersionsOutput, ListObjectsOutput, ListObjectsV2Output},
    types::{
        CommonPrefix, DeleteMarkerEntry, Object, ObjectStorageClass, ObjectVersion,
        ObjectVersionStorageClass, Owner,
    },
};
use tracing::debug;

use crate::{
    error::S3ServiceError,
    provider::RustackS3,
    state::{keystore::VersionListEntry, object::Owner as InternalOwner},
    utils::{decode_continuation_token, encode_continuation_token},
};

/// Default maximum number of keys returned in a single listing response.
const DEFAULT_MAX_KEYS: i32 = 1000;

/// Validate the `max_keys` parameter, rejecting negative values.
///
/// # Errors
///
/// Returns [`S3Error`] with [`S3ErrorCode::InvalidArgument`] if `max_keys` is negative.
fn validate_max_keys(max_keys: Option<i32>) -> Result<i32, S3Error> {
    let value = max_keys.unwrap_or(DEFAULT_MAX_KEYS);
    if value < 0 {
        return Err(S3ServiceError::InvalidArgument {
            message: format!(
                "Argument max-keys must be an integer between 0 and {DEFAULT_MAX_KEYS}"
            ),
        }
        .into_s3_error());
    }
    Ok(value)
}

/// Convert an internal [`crate::state::object::S3Object`] to a model [`Object`].
#[allow(clippy::cast_possible_wrap)]
fn to_model_object(obj: &crate::state::object::S3Object) -> Object {
    let owner = Owner {
        display_name: Some(obj.owner.display_name.clone()),
        id: Some(obj.owner.id.clone()),
    };
    Object {
        checksum_algorithm: Vec::new(),
        checksum_type: None,
        e_tag: Some(obj.etag.clone()),
        key: Some(obj.key.clone()),
        last_modified: Some(obj.last_modified),
        owner: Some(owner),
        restore_status: None,
        size: Some(obj.size as i64),
        storage_class: Some(ObjectStorageClass::from(obj.storage_class.as_str())),
    }
}

/// Convert an internal [`InternalOwner`] to a model [`Owner`].
fn to_model_owner(owner: &InternalOwner) -> Owner {
    Owner {
        display_name: Some(owner.display_name.clone()),
        id: Some(owner.id.clone()),
    }
}

/// Convert common prefix strings to model [`CommonPrefix`] values.
fn to_common_prefixes(prefixes: &[String]) -> Vec<CommonPrefix> {
    prefixes
        .iter()
        .map(|p| CommonPrefix {
            prefix: Some(p.clone()),
        })
        .collect()
}

// AWS S3 DTOs use signed integers (i32/i64) for inherently non-negative values.
// These handler methods must remain async for consistency with other handlers.
// Keep handlers lazy and uniformly awaitable at the dispatch boundary, including
// in-memory operations that currently complete without yielding.
#[allow(
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::unused_async,
    clippy::unused_async_trait_impl
)]
impl RustackS3 {
    /// List objects (v1 API).
    pub async fn handle_list_objects(
        &self,
        input: ListObjectsInput,
    ) -> Result<ListObjectsOutput, S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let prefix = input.prefix.as_deref().unwrap_or("");
        let delimiter = input.delimiter.as_deref().unwrap_or("");
        let marker = input.marker.as_deref().unwrap_or("");
        let max_keys = validate_max_keys(input.max_keys)?;
        let max_keys_usize = usize::try_from(max_keys).unwrap_or(1000);

        let store = bucket.objects.read();
        let result = store.list_objects(prefix, delimiter, marker, max_keys_usize);
        drop(store);
        drop(bucket);

        let contents: Vec<Object> = result.objects.iter().map(to_model_object).collect();
        let common_prefixes = to_common_prefixes(&result.common_prefixes);

        let next_marker = if result.is_truncated {
            result.next_marker.clone()
        } else {
            None
        };

        debug!(
            bucket = %bucket_name,
            prefix = %prefix,
            count = contents.len(),
            is_truncated = result.is_truncated,
            "list_objects completed"
        );

        Ok(ListObjectsOutput {
            common_prefixes,
            contents,
            delimiter: input.delimiter,
            encoding_type: input.encoding_type,
            is_truncated: Some(result.is_truncated),
            marker: input.marker,
            max_keys: Some(max_keys),
            name: Some(bucket_name),
            next_marker,
            prefix: input.prefix,
            request_charged: None,
        })
    }

    /// List objects (v2 API with continuation tokens).
    pub async fn handle_list_objects_v2(
        &self,
        input: ListObjectsV2Input,
    ) -> Result<ListObjectsV2Output, S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let prefix = input.prefix.as_deref().unwrap_or("");
        let delimiter = input.delimiter.as_deref().unwrap_or("");
        let max_keys = validate_max_keys(input.max_keys)?;
        let max_keys_usize = usize::try_from(max_keys).unwrap_or(1000);
        let fetch_owner = input.fetch_owner.unwrap_or(false);

        // Determine start_after: either from continuation token or start_after param.
        let decoded_token = if let Some(token) = &input.continuation_token {
            Some(decode_continuation_token(token).map_err(S3ServiceError::into_s3_error)?)
        } else {
            None
        };
        let start_after = decoded_token
            .as_deref()
            .or(input.start_after.as_deref())
            .unwrap_or("");

        let store = bucket.objects.read();
        let result = store.list_objects(prefix, delimiter, start_after, max_keys_usize);
        drop(store);
        drop(bucket);

        let contents: Vec<Object> = result
            .objects
            .iter()
            .map(|obj| {
                let mut s3_obj = to_model_object(obj);
                if !fetch_owner {
                    s3_obj.owner = None;
                }
                s3_obj
            })
            .collect();
        let common_prefixes = to_common_prefixes(&result.common_prefixes);

        let next_continuation_token = if result.is_truncated {
            result
                .next_marker
                .as_ref()
                .map(|m| encode_continuation_token(m))
        } else {
            None
        };

        let key_count = contents.len() as i32;

        debug!(
            bucket = %bucket_name,
            prefix = %prefix,
            count = key_count,
            is_truncated = result.is_truncated,
            "list_objects_v2 completed"
        );

        Ok(ListObjectsV2Output {
            common_prefixes,
            contents,
            continuation_token: input.continuation_token,
            delimiter: input.delimiter,
            encoding_type: input.encoding_type,
            is_truncated: Some(result.is_truncated),
            key_count: Some(key_count),
            max_keys: Some(max_keys),
            name: Some(bucket_name),
            next_continuation_token,
            prefix: input.prefix,
            request_charged: None,
            start_after: input.start_after,
        })
    }

    /// List object versions.
    pub async fn handle_list_object_versions(
        &self,
        input: ListObjectVersionsInput,
    ) -> Result<ListObjectVersionsOutput, S3Error> {
        // S3 requires KeyMarker when VersionIdMarker is specified.
        if input.version_id_marker.is_some() && input.key_marker.is_none() {
            return Err(S3Error::invalid_argument(
                "A version-id marker cannot be specified without a key marker",
            ));
        }

        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let prefix = input.prefix.as_deref().unwrap_or("");
        let delimiter = input.delimiter.as_deref().unwrap_or("");
        let key_marker = input.key_marker.as_deref().unwrap_or("");
        let version_id_marker = input.version_id_marker.as_deref().unwrap_or("");
        let max_keys = validate_max_keys(input.max_keys)?;
        let max_keys_usize = usize::try_from(max_keys).unwrap_or(1000);

        let store = bucket.objects.read();
        let result = store.list_object_versions(
            prefix,
            delimiter,
            key_marker,
            version_id_marker,
            max_keys_usize,
        );
        drop(store);
        drop(bucket);

        // Separate versions and delete markers.
        let (versions, delete_markers) = partition_version_list_entries(&result.versions);

        let common_prefixes = to_common_prefixes(&result.common_prefixes);

        debug!(
            bucket = %bucket_name,
            prefix = %prefix,
            versions = versions.len(),
            delete_markers = delete_markers.len(),
            is_truncated = result.is_truncated,
            "list_object_versions completed"
        );

        Ok(ListObjectVersionsOutput {
            common_prefixes,
            delete_markers,
            delimiter: input.delimiter,
            encoding_type: input.encoding_type,
            is_truncated: Some(result.is_truncated),
            key_marker: input.key_marker,
            max_keys: Some(max_keys),
            name: Some(bucket_name),
            next_key_marker: result.next_key_marker,
            next_version_id_marker: result.next_version_id_marker,
            prefix: input.prefix,
            request_charged: None,
            version_id_marker: input.version_id_marker,
            versions,
        })
    }
}

/// Partition a list of [`VersionListEntry`] into model [`ObjectVersion`] and
/// [`DeleteMarkerEntry`] values.
#[allow(clippy::cast_possible_wrap)]
fn partition_version_list_entries(
    entries: &[VersionListEntry],
) -> (Vec<ObjectVersion>, Vec<DeleteMarkerEntry>) {
    let mut versions = Vec::new();
    let mut delete_markers = Vec::new();

    for entry in entries {
        match &entry.version {
            crate::state::object::ObjectVersion::Object(obj) => {
                let owner = to_model_owner(&obj.owner);
                versions.push(ObjectVersion {
                    checksum_algorithm: Vec::new(),
                    checksum_type: None,
                    e_tag: Some(obj.etag.clone()),
                    is_latest: Some(entry.is_latest),
                    key: Some(obj.key.clone()),
                    last_modified: Some(obj.last_modified),
                    owner: Some(owner),
                    restore_status: None,
                    size: Some(obj.size as i64),
                    storage_class: Some(ObjectVersionStorageClass::from(
                        obj.storage_class.as_str(),
                    )),
                    version_id: Some(obj.version_id.clone()),
                });
            }
            crate::state::object::ObjectVersion::DeleteMarker(dm) => {
                let owner = to_model_owner(&dm.owner);
                delete_markers.push(DeleteMarkerEntry {
                    is_latest: Some(entry.is_latest),
                    key: Some(dm.key.clone()),
                    last_modified: Some(dm.last_modified),
                    owner: Some(owner),
                    version_id: Some(dm.version_id.clone()),
                });
            }
        }
    }

    (versions, delete_markers)
}
