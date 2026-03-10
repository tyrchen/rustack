//! Multipart upload state management.
//!
//! Tracks in-progress multipart uploads and their constituent parts.
//! Each [`MultipartUpload`] captures the metadata provided at initiation
//! time and accumulates [`UploadPart`] entries as they are uploaded.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::object::{ChecksumData, ObjectMetadata, Owner};

/// An in-progress multipart upload.
///
/// Created by `CreateMultipartUpload` and completed or aborted later.
/// Metadata is captured at creation time and applied to the final object
/// upon completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultipartUpload {
    /// Unique identifier for this upload.
    pub upload_id: String,
    /// The object key that this upload will create.
    pub key: String,
    /// When the upload was initiated.
    pub initiated: DateTime<Utc>,
    /// The owner who initiated the upload.
    pub owner: Owner,
    /// Object metadata captured at `CreateMultipartUpload` time.
    pub metadata: ObjectMetadata,
    /// The checksum algorithm requested for this upload (e.g. `CRC32`, `SHA256`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksum_algorithm: Option<String>,
    /// Whether the final checksum should be `FULL_OBJECT` or `COMPOSITE`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksum_type: Option<String>,
    /// Parts uploaded so far, keyed by part number (1-based).
    pub parts: BTreeMap<u32, UploadPart>,
    /// Server-side encryption algorithm for the final object.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sse_algorithm: Option<String>,
    /// KMS key ID for SSE-KMS encryption.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sse_kms_key_id: Option<String>,
    /// The storage class for the final object.
    pub storage_class: String,
}

impl MultipartUpload {
    /// Create a new multipart upload.
    #[must_use]
    pub fn new(upload_id: String, key: String, owner: Owner, metadata: ObjectMetadata) -> Self {
        Self {
            upload_id,
            key,
            initiated: Utc::now(),
            owner,
            metadata,
            checksum_algorithm: None,
            checksum_type: None,
            parts: BTreeMap::new(),
            sse_algorithm: None,
            sse_kms_key_id: None,
            storage_class: "STANDARD".to_owned(),
        }
    }

    /// Insert or replace a part in this upload.
    pub fn put_part(&mut self, part: UploadPart) {
        self.parts.insert(part.part_number, part);
    }

    /// Get a part by its number.
    #[must_use]
    pub fn get_part(&self, part_number: u32) -> Option<&UploadPart> {
        self.parts.get(&part_number)
    }

    /// Return the total number of parts uploaded so far.
    #[must_use]
    pub fn parts_count(&self) -> usize {
        self.parts.len()
    }

    /// Compute the total size of all uploaded parts.
    #[must_use]
    pub fn total_size(&self) -> u64 {
        self.parts.values().map(|p| p.size).sum()
    }
}

/// A single part within a multipart upload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadPart {
    /// The part number (1-based, up to 10 000).
    pub part_number: u32,
    /// The entity tag for this part (quoted hex MD5).
    pub etag: String,
    /// Size of this part in bytes.
    pub size: u64,
    /// When this part was last modified / uploaded.
    pub last_modified: DateTime<Utc>,
    /// Optional checksum data for this part.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksum: Option<ChecksumData>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_create_multipart_upload() {
        let upload = MultipartUpload::new(
            "upload-123".to_owned(),
            "my-key".to_owned(),
            Owner::default(),
            ObjectMetadata::default(),
        );

        assert_eq!(upload.upload_id, "upload-123");
        assert_eq!(upload.key, "my-key");
        assert_eq!(upload.storage_class, "STANDARD");
        assert_eq!(upload.parts_count(), 0);
        assert_eq!(upload.total_size(), 0);
    }

    #[test]
    fn test_should_put_and_get_parts() {
        let mut upload = MultipartUpload::new(
            "upload-456".to_owned(),
            "data.bin".to_owned(),
            Owner::default(),
            ObjectMetadata::default(),
        );

        let part1 = UploadPart {
            part_number: 1,
            etag: "\"abc123\"".to_owned(),
            size: 5 * 1024 * 1024,
            last_modified: Utc::now(),
            checksum: None,
        };
        let part2 = UploadPart {
            part_number: 2,
            etag: "\"def456\"".to_owned(),
            size: 3 * 1024 * 1024,
            last_modified: Utc::now(),
            checksum: None,
        };

        upload.put_part(part1);
        upload.put_part(part2);

        assert_eq!(upload.parts_count(), 2);
        assert_eq!(upload.total_size(), 8 * 1024 * 1024);

        let p1 = upload.get_part(1);
        assert!(p1.is_some());
        assert_eq!(p1.map(|p| &p.etag), Some(&"\"abc123\"".to_owned()));

        assert!(upload.get_part(3).is_none());
    }

    #[test]
    fn test_should_replace_existing_part() {
        let mut upload = MultipartUpload::new(
            "upload-789".to_owned(),
            "replace.bin".to_owned(),
            Owner::default(),
            ObjectMetadata::default(),
        );

        let part_v1 = UploadPart {
            part_number: 1,
            etag: "\"old\"".to_owned(),
            size: 100,
            last_modified: Utc::now(),
            checksum: None,
        };
        upload.put_part(part_v1);

        let part_v2 = UploadPart {
            part_number: 1,
            etag: "\"new\"".to_owned(),
            size: 200,
            last_modified: Utc::now(),
            checksum: None,
        };
        upload.put_part(part_v2);

        assert_eq!(upload.parts_count(), 1);
        assert_eq!(upload.total_size(), 200);
        assert_eq!(
            upload.get_part(1).map(|p| &p.etag),
            Some(&"\"new\"".to_owned()),
        );
    }

    #[test]
    fn test_should_store_checksum_on_part() {
        let part = UploadPart {
            part_number: 1,
            etag: "\"abc\"".to_owned(),
            size: 1024,
            last_modified: Utc::now(),
            checksum: Some(ChecksumData {
                algorithm: "CRC32".to_owned(),
                value: "AAAAAA==".to_owned(),
                checksum_type: "FULL_OBJECT".to_owned(),
            }),
        };
        let cs = part.checksum.as_ref();
        assert!(cs.is_some());
        assert_eq!(cs.map(|c| c.algorithm.as_str()), Some("CRC32"));
    }
}
