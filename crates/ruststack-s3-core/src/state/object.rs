//! S3 object types and metadata.
//!
//! This module defines the core data structures for S3 objects, delete markers,
//! object metadata, ownership, ACL configuration, and versioning.

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Owner
// ---------------------------------------------------------------------------

/// The owner of an S3 object or bucket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Owner {
    /// The canonical user ID of the owner.
    pub id: String,
    /// The display name of the owner.
    pub display_name: String,
}

impl Default for Owner {
    fn default() -> Self {
        Self {
            id: "75aa57f09aa0c8caeab4f8c24e99d10f8e7faeebf76c078efc7c6caea54ba06a".to_owned(),
            display_name: "webfile".to_owned(),
        }
    }
}

impl fmt::Display for Owner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({})", self.display_name, self.id)
    }
}

// ---------------------------------------------------------------------------
// CannedAcl
// ---------------------------------------------------------------------------

/// Predefined (canned) ACL grants for S3 buckets and objects.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CannedAcl {
    /// Owner gets `FULL_CONTROL`. No one else has access rights (default).
    #[default]
    Private,
    /// Owner gets `FULL_CONTROL`. The `AllUsers` group gets `READ` access.
    PublicRead,
    /// Owner gets `FULL_CONTROL`. The `AllUsers` group gets `READ` and `WRITE` access.
    PublicReadWrite,
    /// Owner gets `FULL_CONTROL`. The `AuthenticatedUsers` group gets `READ` access.
    AuthenticatedRead,
    /// Owner gets `FULL_CONTROL`. Amazon EC2 gets `READ` access to GET an
    /// Amazon Machine Image (AMI) bundle from Amazon S3.
    AwsExecRead,
    /// Object owner gets `FULL_CONTROL`. Bucket owner gets `READ` access.
    BucketOwnerRead,
    /// Both the object owner and the bucket owner get `FULL_CONTROL` over the object.
    BucketOwnerFullControl,
    /// The `LogDelivery` group gets `WRITE` and `READ_ACP` permissions on the bucket.
    LogDeliveryWrite,
}

impl CannedAcl {
    /// Return the string representation of the canned ACL.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Private => "private",
            Self::PublicRead => "public-read",
            Self::PublicReadWrite => "public-read-write",
            Self::AuthenticatedRead => "authenticated-read",
            Self::AwsExecRead => "aws-exec-read",
            Self::BucketOwnerRead => "bucket-owner-read",
            Self::BucketOwnerFullControl => "bucket-owner-full-control",
            Self::LogDeliveryWrite => "log-delivery-write",
        }
    }
}

impl fmt::Display for CannedAcl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when parsing a [`CannedAcl`] from a string fails.
#[derive(Debug, Clone, thiserror::Error)]
#[error("unknown canned ACL: {0}")]
pub struct ParseCannedAclError(String);

impl FromStr for CannedAcl {
    type Err = ParseCannedAclError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "private" => Ok(Self::Private),
            "public-read" => Ok(Self::PublicRead),
            "public-read-write" => Ok(Self::PublicReadWrite),
            "authenticated-read" => Ok(Self::AuthenticatedRead),
            "aws-exec-read" => Ok(Self::AwsExecRead),
            "bucket-owner-read" => Ok(Self::BucketOwnerRead),
            "bucket-owner-full-control" => Ok(Self::BucketOwnerFullControl),
            "log-delivery-write" => Ok(Self::LogDeliveryWrite),
            _ => Err(ParseCannedAclError(s.to_owned())),
        }
    }
}

// ---------------------------------------------------------------------------
// Grant / Grantee / Permission
// ---------------------------------------------------------------------------

/// An ACL grant that pairs a grantee with a permission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Grant {
    /// The entity receiving the permission.
    pub grantee: Grantee,
    /// The permission granted.
    pub permission: Permission,
}

/// A grantee in an ACL grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum Grantee {
    /// A canonical user identified by an AWS account ID.
    CanonicalUser {
        /// The canonical user ID.
        id: String,
        /// The display name for the user.
        display_name: String,
    },
    /// A predefined Amazon S3 group.
    Group {
        /// The URI of the group (e.g.,
        /// `http://acs.amazonaws.com/groups/global/AllUsers`).
        uri: String,
    },
    /// A grantee identified by email (legacy, seldom used).
    Email {
        /// The email address of the grantee.
        email: String,
    },
}

/// A permission that can be granted to a grantee.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Permission {
    /// Grants full control (READ, WRITE, READ_ACP, WRITE_ACP).
    FullControl,
    /// Allows grantee to list objects in the bucket or read the object data.
    Read,
    /// Allows grantee to create objects in the bucket.
    Write,
    /// Allows grantee to read the bucket/object ACL.
    ReadAcp,
    /// Allows grantee to write the bucket/object ACL.
    WriteAcp,
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::FullControl => "FULL_CONTROL",
            Self::Read => "READ",
            Self::Write => "WRITE",
            Self::ReadAcp => "READ_ACP",
            Self::WriteAcp => "WRITE_ACP",
        };
        f.write_str(s)
    }
}

// ---------------------------------------------------------------------------
// ChecksumData
// ---------------------------------------------------------------------------

/// Checksum data attached to an S3 object or part.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChecksumData {
    /// The checksum algorithm (e.g. `CRC32`, `CRC32C`, `CRC64NVME`, `SHA1`, `SHA256`).
    pub algorithm: String,
    /// The base64-encoded checksum value.
    pub value: String,
    /// Whether this is a `FULL_OBJECT` or `COMPOSITE` checksum.
    #[serde(default = "default_checksum_type")]
    pub checksum_type: String,
}

/// Default checksum type for single-object uploads.
fn default_checksum_type() -> String {
    "FULL_OBJECT".to_owned()
}

// ---------------------------------------------------------------------------
// ObjectMetadata
// ---------------------------------------------------------------------------

/// Metadata associated with an S3 object.
///
/// Includes standard HTTP headers, user-defined metadata (`x-amz-meta-*`),
/// server-side encryption settings, tagging, ACL, and object-lock fields.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectMetadata {
    /// The MIME type of the object (e.g. `application/octet-stream`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    /// Content encoding (e.g. `gzip`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_encoding: Option<String>,
    /// Content disposition (e.g. `attachment; filename="file.txt"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_disposition: Option<String>,
    /// Content language (e.g. `en-US`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_language: Option<String>,
    /// Cache control directives (e.g. `max-age=3600`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<String>,
    /// Expiration date/time string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires: Option<String>,
    /// User-defined metadata headers (`x-amz-meta-*`).
    #[serde(default)]
    pub user_metadata: HashMap<String, String>,
    /// Server-side encryption algorithm (e.g. `AES256`, `aws:kms`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sse_algorithm: Option<String>,
    /// KMS key ID used for server-side encryption.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sse_kms_key_id: Option<String>,
    /// Whether an S3 Bucket Key is enabled for SSE-KMS.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sse_bucket_key_enabled: Option<bool>,
    /// Customer-provided encryption algorithm for SSE-C.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sse_customer_algorithm: Option<String>,
    /// Base64-encoded MD5 of the customer-provided encryption key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sse_customer_key_md5: Option<String>,
    /// Object tags as key-value pairs.
    #[serde(default)]
    pub tagging: Vec<(String, String)>,
    /// Canned ACL applied to this object.
    #[serde(default)]
    pub acl: CannedAcl,
    /// Object lock retention mode (`GOVERNANCE` or `COMPLIANCE`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_lock_mode: Option<String>,
    /// Object lock retain-until date.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_lock_retain_until: Option<DateTime<Utc>>,
    /// Whether a legal hold is in effect for this object.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_lock_legal_hold: Option<bool>,
}

// ---------------------------------------------------------------------------
// S3Object
// ---------------------------------------------------------------------------

/// A stored S3 object (non-delete-marker).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct S3Object {
    /// The object key.
    pub key: String,
    /// The version ID (`"null"` for un-versioned objects).
    pub version_id: String,
    /// The entity tag (quoted hex MD5 digest, e.g. `"d41d8cd98f00b204e9800998ecf8427e"`).
    pub etag: String,
    /// The object size in bytes.
    pub size: u64,
    /// The time this version was last modified.
    pub last_modified: DateTime<Utc>,
    /// The storage class (default `STANDARD`).
    pub storage_class: String,
    /// Object metadata (headers, tags, encryption, etc.).
    pub metadata: ObjectMetadata,
    /// The owner of this object.
    pub owner: Owner,
    /// Optional checksum data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksum: Option<ChecksumData>,
    /// The number of parts if this object was created via multipart upload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parts_count: Option<u32>,
    /// Individual part ETags (used for composite ETag generation in multipart uploads).
    #[serde(default)]
    pub part_etags: Vec<String>,
}

impl S3Object {
    /// Returns `false` because an `S3Object` is never a delete marker.
    #[must_use]
    pub fn is_delete_marker(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// S3DeleteMarker
// ---------------------------------------------------------------------------

/// A delete marker in a versioned bucket.
///
/// Delete markers are created when an object is deleted in a versioned bucket.
/// They act as a placeholder that indicates the object has been logically deleted.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct S3DeleteMarker {
    /// The object key.
    pub key: String,
    /// The version ID of this delete marker.
    pub version_id: String,
    /// The time this delete marker was created.
    pub last_modified: DateTime<Utc>,
    /// The owner of this delete marker.
    pub owner: Owner,
}

impl S3DeleteMarker {
    /// Returns `true` because an `S3DeleteMarker` is always a delete marker.
    #[must_use]
    pub fn is_delete_marker(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// ObjectVersion
// ---------------------------------------------------------------------------

/// A version entry in a versioned bucket, which is either an object or a
/// delete marker.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum ObjectVersion {
    /// A real object version (boxed to reduce enum size).
    Object(Box<S3Object>),
    /// A delete-marker version.
    DeleteMarker(S3DeleteMarker),
}

impl ObjectVersion {
    /// Returns the object key.
    #[must_use]
    pub fn key(&self) -> &str {
        match self {
            Self::Object(obj) => &obj.key,
            Self::DeleteMarker(dm) => &dm.key,
        }
    }

    /// Returns the version ID.
    #[must_use]
    pub fn version_id(&self) -> &str {
        match self {
            Self::Object(obj) => &obj.version_id,
            Self::DeleteMarker(dm) => &dm.version_id,
        }
    }

    /// Returns the last-modified timestamp.
    #[must_use]
    pub fn last_modified(&self) -> DateTime<Utc> {
        match self {
            Self::Object(obj) => obj.last_modified,
            Self::DeleteMarker(dm) => dm.last_modified,
        }
    }

    /// Returns `true` if this version is a delete marker.
    #[must_use]
    pub fn is_delete_marker(&self) -> bool {
        matches!(self, Self::DeleteMarker(_))
    }

    /// Returns the owner of this version.
    #[must_use]
    pub fn owner(&self) -> &Owner {
        match self {
            Self::Object(obj) => &obj.owner,
            Self::DeleteMarker(dm) => &dm.owner,
        }
    }

    /// Returns a reference to the inner `S3Object`, if this is an object version.
    #[must_use]
    pub fn as_object(&self) -> Option<&S3Object> {
        match self {
            Self::Object(obj) => Some(obj),
            Self::DeleteMarker(_) => None,
        }
    }

    /// Returns a mutable reference to the inner `S3Object`, if this is an object version.
    pub fn as_object_mut(&mut self) -> Option<&mut S3Object> {
        match self {
            Self::Object(obj) => Some(obj),
            Self::DeleteMarker(_) => None,
        }
    }

    /// Returns a reference to the inner `S3DeleteMarker`, if this is a delete marker.
    #[must_use]
    pub fn as_delete_marker(&self) -> Option<&S3DeleteMarker> {
        match self {
            Self::Object(_) => None,
            Self::DeleteMarker(dm) => Some(dm),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_use_default_owner() {
        let owner = Owner::default();
        assert_eq!(owner.display_name, "webfile");
        assert!(!owner.id.is_empty());
    }

    #[test]
    fn test_should_display_owner() {
        let owner = Owner {
            id: "abc123".to_owned(),
            display_name: "alice".to_owned(),
        };
        assert_eq!(format!("{owner}"), "alice(abc123)");
    }

    #[test]
    fn test_should_default_canned_acl_to_private() {
        assert_eq!(CannedAcl::default(), CannedAcl::Private);
        assert_eq!(CannedAcl::default().as_str(), "private");
    }

    #[test]
    fn test_should_roundtrip_canned_acl_from_str() {
        let cases = [
            ("private", CannedAcl::Private),
            ("public-read", CannedAcl::PublicRead),
            ("public-read-write", CannedAcl::PublicReadWrite),
            ("authenticated-read", CannedAcl::AuthenticatedRead),
            ("aws-exec-read", CannedAcl::AwsExecRead),
            ("bucket-owner-read", CannedAcl::BucketOwnerRead),
            (
                "bucket-owner-full-control",
                CannedAcl::BucketOwnerFullControl,
            ),
            ("log-delivery-write", CannedAcl::LogDeliveryWrite),
        ];
        for (s, expected) in cases {
            let parsed: CannedAcl = s.parse().unwrap_or_else(|_| panic!("failed to parse {s}"));
            assert_eq!(parsed, expected);
            assert_eq!(parsed.as_str(), s);
        }
    }

    #[test]
    fn test_should_reject_unknown_canned_acl() {
        let result = "unknown-acl".parse::<CannedAcl>();
        assert!(result.is_err());
    }

    #[test]
    fn test_should_identify_object_as_not_delete_marker() {
        let obj = make_test_object("test-key");
        assert!(!obj.is_delete_marker());
    }

    #[test]
    fn test_should_identify_delete_marker() {
        let dm = S3DeleteMarker {
            key: "test-key".to_owned(),
            version_id: "v1".to_owned(),
            last_modified: Utc::now(),
            owner: Owner::default(),
        };
        assert!(dm.is_delete_marker());
    }

    #[test]
    fn test_should_access_object_version_fields() {
        let obj = make_test_object("my-key");
        let version = ObjectVersion::Object(Box::new(obj));

        assert_eq!(version.key(), "my-key");
        assert_eq!(version.version_id(), "null");
        assert!(!version.is_delete_marker());
        assert!(version.as_object().is_some());
        assert!(version.as_delete_marker().is_none());
    }

    #[test]
    fn test_should_access_delete_marker_version_fields() {
        let dm = S3DeleteMarker {
            key: "deleted-key".to_owned(),
            version_id: "dm-v1".to_owned(),
            last_modified: Utc::now(),
            owner: Owner::default(),
        };
        let version = ObjectVersion::DeleteMarker(dm);

        assert_eq!(version.key(), "deleted-key");
        assert_eq!(version.version_id(), "dm-v1");
        assert!(version.is_delete_marker());
        assert!(version.as_object().is_none());
        assert!(version.as_delete_marker().is_some());
    }

    #[test]
    fn test_should_default_object_metadata() {
        let meta = ObjectMetadata::default();
        assert!(meta.content_type.is_none());
        assert!(meta.user_metadata.is_empty());
        assert!(meta.tagging.is_empty());
        assert_eq!(meta.acl, CannedAcl::Private);
        assert!(meta.object_lock_mode.is_none());
    }

    #[test]
    fn test_should_display_permission() {
        assert_eq!(format!("{}", Permission::FullControl), "FULL_CONTROL");
        assert_eq!(format!("{}", Permission::Read), "READ");
        assert_eq!(format!("{}", Permission::Write), "WRITE");
        assert_eq!(format!("{}", Permission::ReadAcp), "READ_ACP");
        assert_eq!(format!("{}", Permission::WriteAcp), "WRITE_ACP");
    }

    // ---- helpers ----

    fn make_test_object(key: &str) -> S3Object {
        S3Object {
            key: key.to_owned(),
            version_id: "null".to_owned(),
            etag: "\"d41d8cd98f00b204e9800998ecf8427e\"".to_owned(),
            size: 0,
            last_modified: Utc::now(),
            storage_class: "STANDARD".to_owned(),
            metadata: ObjectMetadata::default(),
            owner: Owner::default(),
            checksum: None,
            parts_count: None,
            part_etags: Vec::new(),
        }
    }
}
