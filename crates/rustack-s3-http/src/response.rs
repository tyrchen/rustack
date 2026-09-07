//! S3 Output struct to HTTP response serialization.
//!
//! This module provides the [`IntoS3Response`] trait and implementations for converting
//! typed S3 Output structs from `rustack-s3-model` into HTTP responses with the
//! appropriate status code, headers, and body.
//!
//! Response categories:
//! - **Header-only**: Most write operations that return metadata in response headers.
//! - **XML body**: List operations and configuration getters that return XML payloads.
//! - **Streaming body**: `GetObject` passes through the body bytes.
//! - **Mixed**: Operations like `CopyObject` return both XML body and response headers.
//!
//! XML serialization is delegated to `rustack-s3-xml`.

use bytes::Bytes;
use http::header::HeaderValue;
use rustack_s3_model::error::S3Error;

use crate::body::S3ResponseBody;

/// Trait for converting an S3 output struct into an HTTP response.
///
/// Each S3 operation's Output type implements this trait to produce the correct
/// HTTP response with headers, status code, and body.
pub trait IntoS3Response {
    /// Convert this output into an HTTP response.
    ///
    /// # Errors
    ///
    /// Returns an `S3Error` if the response cannot be constructed (e.g., invalid
    /// header value).
    fn into_s3_response(self) -> Result<http::Response<S3ResponseBody>, S3Error>;
}

// ---------------------------------------------------------------------------
// Helper functions for building responses
// ---------------------------------------------------------------------------

/// Set an optional header on a response builder if the value is `Some`.
fn set_optional_header(
    builder: http::response::Builder,
    name: &str,
    value: Option<&str>,
) -> http::response::Builder {
    if let Some(v) = value {
        if let Ok(hv) = HeaderValue::from_str(v) {
            return builder.header(name, hv);
        }
    }
    builder
}

/// Set an optional boolean header.
fn set_optional_bool_header(
    builder: http::response::Builder,
    name: &str,
    value: Option<bool>,
) -> http::response::Builder {
    if let Some(v) = value {
        return builder.header(name, if v { "true" } else { "false" });
    }
    builder
}

/// Set an optional integer header.
fn set_optional_int_header(
    builder: http::response::Builder,
    name: &str,
    value: Option<i64>,
) -> http::response::Builder {
    if let Some(v) = value {
        return builder.header(name, v);
    }
    builder
}

/// Set an optional i32 header.
fn set_optional_i32_header(
    builder: http::response::Builder,
    name: &str,
    value: Option<i32>,
) -> http::response::Builder {
    if let Some(v) = value {
        return builder.header(name, i64::from(v));
    }
    builder
}

/// Set an optional header from a type implementing `Display`.
fn set_optional_display_header<T: std::fmt::Display>(
    builder: http::response::Builder,
    name: &str,
    value: Option<&T>,
) -> http::response::Builder {
    if let Some(v) = value {
        let s = v.to_string();
        if let Ok(hv) = HeaderValue::from_str(&s) {
            return builder.header(name, hv);
        }
    }
    builder
}

/// Set an optional HTTP date header from a `DateTime<Utc>`.
fn set_optional_timestamp_header(
    builder: http::response::Builder,
    name: &str,
    value: Option<&chrono::DateTime<chrono::Utc>>,
) -> http::response::Builder {
    if let Some(v) = value {
        let formatted = v.format("%a, %d %b %Y %H:%M:%S GMT").to_string();
        if let Ok(hv) = HeaderValue::from_str(&formatted) {
            return builder.header(name, hv);
        }
    }
    builder
}

/// Set metadata prefix headers from a `HashMap`.
fn set_metadata_headers(
    mut builder: http::response::Builder,
    metadata: &std::collections::HashMap<String, String>,
) -> http::response::Builder {
    for (key, value) in metadata {
        let header_name = format!("x-amz-meta-{key}");
        if let Ok(hv) = HeaderValue::from_str(value) {
            builder = builder.header(header_name, hv);
        }
    }
    builder
}

/// Build a response from a builder, converting build errors to `S3Error`.
fn build_response(
    builder: http::response::Builder,
    body: S3ResponseBody,
) -> Result<http::Response<S3ResponseBody>, S3Error> {
    builder
        .body(body)
        .map_err(|e| S3Error::internal_error(format!("failed to build HTTP response: {e}")))
}

/// Serialize a value to XML and wrap it in an `S3ResponseBody`.
fn serialize_xml_body<T: rustack_s3_xml::S3Serialize>(
    root_element: &str,
    value: &T,
) -> Result<S3ResponseBody, S3Error> {
    let xml_bytes = rustack_s3_xml::to_xml(root_element, value)
        .map_err(|e| S3Error::internal_error(format!("XML serialization failed: {e}")))?;
    Ok(S3ResponseBody::from_bytes(Bytes::from(xml_bytes)))
}

// ---------------------------------------------------------------------------
// Implementations
// ---------------------------------------------------------------------------

#[allow(clippy::wildcard_imports)] // All output types are used in IntoS3Response impls below.
use rustack_s3_model::output::*;

// --- Bucket operations ---

impl IntoS3Response for CreateBucketOutput {
    fn into_s3_response(self) -> Result<http::Response<S3ResponseBody>, S3Error> {
        let mut builder = http::Response::builder().status(http::StatusCode::OK);
        builder = set_optional_header(builder, "Location", self.location.as_deref());
        builder = set_optional_header(builder, "x-amz-bucket-arn", self.bucket_arn.as_deref());
        build_response(builder, S3ResponseBody::empty())
    }
}

impl IntoS3Response for HeadBucketOutput {
    fn into_s3_response(self) -> Result<http::Response<S3ResponseBody>, S3Error> {
        let mut builder = http::Response::builder().status(http::StatusCode::OK);
        builder =
            set_optional_bool_header(builder, "x-amz-access-point-alias", self.access_point_alias);
        builder = set_optional_header(builder, "x-amz-bucket-arn", self.bucket_arn.as_deref());
        builder = set_optional_header(
            builder,
            "x-amz-bucket-location-name",
            self.bucket_location_name.as_deref(),
        );
        builder = set_optional_display_header(
            builder,
            "x-amz-bucket-location-type",
            self.bucket_location_type.as_ref(),
        );
        builder = set_optional_header(
            builder,
            "x-amz-bucket-region",
            self.bucket_region.as_deref(),
        );
        build_response(builder, S3ResponseBody::empty())
    }
}

impl IntoS3Response for GetBucketLocationOutput {
    fn into_s3_response(self) -> Result<http::Response<S3ResponseBody>, S3Error> {
        // Returns XML body with LocationConstraint.
        // Simplified until rustack-s3-xml has serialization.
        let body = if let Some(constraint) = &self.location_constraint {
            let xml = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
                 <LocationConstraint xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">{constraint}</LocationConstraint>",
            );
            S3ResponseBody::from_string(xml)
        } else {
            let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
                       <LocationConstraint xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"/>";
            S3ResponseBody::from_string(xml)
        };

        let builder = http::Response::builder()
            .status(http::StatusCode::OK)
            .header("Content-Type", "application/xml");
        build_response(builder, body)
    }
}

impl IntoS3Response for ListBucketsOutput {
    fn into_s3_response(self) -> Result<http::Response<S3ResponseBody>, S3Error> {
        let body = serialize_xml_body("ListAllMyBucketsResult", &self)?;
        let builder = http::Response::builder()
            .status(http::StatusCode::OK)
            .header("Content-Type", "application/xml");
        build_response(builder, body)
    }
}

// --- Object operations ---

/// Shared object metadata fields used by both `GetObject` and `HeadObject` responses.
///
/// These two operations return nearly identical response headers. This struct captures
/// the common fields to eliminate duplication between their `IntoS3Response` impls.
struct ObjectMetadataHeaders<'a> {
    accept_ranges: Option<&'a str>,
    bucket_key_enabled: Option<bool>,
    cache_control: Option<&'a str>,
    checksum_crc32: Option<&'a str>,
    checksum_crc32c: Option<&'a str>,
    checksum_crc64nvme: Option<&'a str>,
    checksum_sha1: Option<&'a str>,
    checksum_sha256: Option<&'a str>,
    checksum_type: Option<&'a rustack_s3_model::types::ChecksumType>,
    content_disposition: Option<&'a str>,
    content_encoding: Option<&'a str>,
    content_language: Option<&'a str>,
    content_length: Option<i64>,
    content_range: Option<&'a str>,
    content_type: Option<&'a str>,
    delete_marker: Option<bool>,
    e_tag: Option<&'a str>,
    expiration: Option<&'a str>,
    expires: Option<&'a str>,
    last_modified: Option<&'a chrono::DateTime<chrono::Utc>>,
    metadata: &'a std::collections::HashMap<String, String>,
    missing_meta: Option<i32>,
    object_lock_legal_hold_status: Option<&'a rustack_s3_model::types::ObjectLockLegalHoldStatus>,
    object_lock_mode: Option<&'a rustack_s3_model::types::ObjectLockMode>,
    object_lock_retain_until_date: Option<&'a chrono::DateTime<chrono::Utc>>,
    parts_count: Option<i32>,
    replication_status: Option<&'a rustack_s3_model::types::ReplicationStatus>,
    request_charged: Option<&'a rustack_s3_model::types::RequestCharged>,
    restore: Option<&'a str>,
    sse_customer_algorithm: Option<&'a str>,
    sse_customer_key_md5: Option<&'a str>,
    ssekms_key_id: Option<&'a str>,
    server_side_encryption: Option<&'a rustack_s3_model::types::ServerSideEncryption>,
    storage_class: Option<&'a rustack_s3_model::types::StorageClass>,
    tag_count: Option<i32>,
    version_id: Option<&'a str>,
    website_redirect_location: Option<&'a str>,
}

/// Apply all shared object metadata headers to a response builder.
fn set_object_metadata_headers(
    mut builder: http::response::Builder,
    h: &ObjectMetadataHeaders<'_>,
) -> http::response::Builder {
    builder = set_optional_header(builder, "accept-ranges", h.accept_ranges);
    builder = set_optional_bool_header(
        builder,
        "x-amz-server-side-encryption-bucket-key-enabled",
        h.bucket_key_enabled,
    );
    builder = set_optional_header(builder, "Cache-Control", h.cache_control);
    builder = set_optional_header(builder, "x-amz-checksum-crc32", h.checksum_crc32);
    builder = set_optional_header(builder, "x-amz-checksum-crc32c", h.checksum_crc32c);
    builder = set_optional_header(builder, "x-amz-checksum-crc64nvme", h.checksum_crc64nvme);
    builder = set_optional_header(builder, "x-amz-checksum-sha1", h.checksum_sha1);
    builder = set_optional_header(builder, "x-amz-checksum-sha256", h.checksum_sha256);
    builder = set_optional_display_header(builder, "x-amz-checksum-type", h.checksum_type);
    builder = set_optional_header(builder, "Content-Disposition", h.content_disposition);
    builder = set_optional_header(builder, "Content-Encoding", h.content_encoding);
    builder = set_optional_header(builder, "Content-Language", h.content_language);
    builder = set_optional_int_header(builder, "Content-Length", h.content_length);
    builder = set_optional_header(builder, "Content-Range", h.content_range);
    builder = set_optional_header(builder, "Content-Type", h.content_type);
    builder = set_optional_bool_header(builder, "x-amz-delete-marker", h.delete_marker);
    builder = set_optional_header(builder, "ETag", h.e_tag);
    builder = set_optional_header(builder, "x-amz-expiration", h.expiration);
    builder = set_optional_header(builder, "Expires", h.expires);
    builder = set_optional_timestamp_header(builder, "Last-Modified", h.last_modified);
    builder = set_metadata_headers(builder, h.metadata);
    builder = set_optional_i32_header(builder, "x-amz-missing-meta", h.missing_meta);
    builder = set_optional_display_header(
        builder,
        "x-amz-object-lock-legal-hold",
        h.object_lock_legal_hold_status,
    );
    builder = set_optional_display_header(builder, "x-amz-object-lock-mode", h.object_lock_mode);
    builder = set_optional_timestamp_header(
        builder,
        "x-amz-object-lock-retain-until-date",
        h.object_lock_retain_until_date,
    );
    builder = set_optional_i32_header(builder, "x-amz-mp-parts-count", h.parts_count);
    builder =
        set_optional_display_header(builder, "x-amz-replication-status", h.replication_status);
    builder = set_optional_display_header(builder, "x-amz-request-charged", h.request_charged);
    builder = set_optional_header(builder, "x-amz-restore", h.restore);
    builder = set_optional_header(
        builder,
        "x-amz-server-side-encryption-customer-algorithm",
        h.sse_customer_algorithm,
    );
    builder = set_optional_header(
        builder,
        "x-amz-server-side-encryption-customer-key-MD5",
        h.sse_customer_key_md5,
    );
    builder = set_optional_header(
        builder,
        "x-amz-server-side-encryption-aws-kms-key-id",
        h.ssekms_key_id,
    );
    builder = set_optional_display_header(
        builder,
        "x-amz-server-side-encryption",
        h.server_side_encryption,
    );
    builder = set_optional_display_header(builder, "x-amz-storage-class", h.storage_class);
    builder = set_optional_i32_header(builder, "x-amz-tagging-count", h.tag_count);
    builder = set_optional_header(builder, "x-amz-version-id", h.version_id);
    builder = set_optional_header(
        builder,
        "x-amz-website-redirect-location",
        h.website_redirect_location,
    );
    builder
}

impl IntoS3Response for GetObjectOutput {
    fn into_s3_response(self) -> Result<http::Response<S3ResponseBody>, S3Error> {
        let headers = ObjectMetadataHeaders {
            accept_ranges: self.accept_ranges.as_deref(),
            bucket_key_enabled: self.bucket_key_enabled,
            cache_control: self.cache_control.as_deref(),
            checksum_crc32: self.checksum_crc32.as_deref(),
            checksum_crc32c: self.checksum_crc32c.as_deref(),
            checksum_crc64nvme: self.checksum_crc64nvme.as_deref(),
            checksum_sha1: self.checksum_sha1.as_deref(),
            checksum_sha256: self.checksum_sha256.as_deref(),
            checksum_type: self.checksum_type.as_ref(),
            content_disposition: self.content_disposition.as_deref(),
            content_encoding: self.content_encoding.as_deref(),
            content_language: self.content_language.as_deref(),
            content_length: self.content_length,
            content_range: self.content_range.as_deref(),
            content_type: self.content_type.as_deref(),
            delete_marker: self.delete_marker,
            e_tag: self.e_tag.as_deref(),
            expiration: self.expiration.as_deref(),
            expires: self.expires.as_deref(),
            last_modified: self.last_modified.as_ref(),
            metadata: &self.metadata,
            missing_meta: self.missing_meta,
            object_lock_legal_hold_status: self.object_lock_legal_hold_status.as_ref(),
            object_lock_mode: self.object_lock_mode.as_ref(),
            object_lock_retain_until_date: self.object_lock_retain_until_date.as_ref(),
            parts_count: self.parts_count,
            replication_status: self.replication_status.as_ref(),
            request_charged: self.request_charged.as_ref(),
            restore: self.restore.as_deref(),
            sse_customer_algorithm: self.sse_customer_algorithm.as_deref(),
            sse_customer_key_md5: self.sse_customer_key_md5.as_deref(),
            ssekms_key_id: self.ssekms_key_id.as_deref(),
            server_side_encryption: self.server_side_encryption.as_ref(),
            storage_class: self.storage_class.as_ref(),
            tag_count: self.tag_count,
            version_id: self.version_id.as_deref(),
            website_redirect_location: self.website_redirect_location.as_deref(),
        };

        let builder = http::Response::builder().status(http::StatusCode::OK);
        let builder = set_object_metadata_headers(builder, &headers);

        let body = if let Some(blob) = self.body {
            S3ResponseBody::from_bytes(blob.data)
        } else {
            S3ResponseBody::empty()
        };

        build_response(builder, body)
    }
}

impl IntoS3Response for HeadObjectOutput {
    fn into_s3_response(self) -> Result<http::Response<S3ResponseBody>, S3Error> {
        let headers = ObjectMetadataHeaders {
            accept_ranges: self.accept_ranges.as_deref(),
            bucket_key_enabled: self.bucket_key_enabled,
            cache_control: self.cache_control.as_deref(),
            checksum_crc32: self.checksum_crc32.as_deref(),
            checksum_crc32c: self.checksum_crc32c.as_deref(),
            checksum_crc64nvme: self.checksum_crc64nvme.as_deref(),
            checksum_sha1: self.checksum_sha1.as_deref(),
            checksum_sha256: self.checksum_sha256.as_deref(),
            checksum_type: self.checksum_type.as_ref(),
            content_disposition: self.content_disposition.as_deref(),
            content_encoding: self.content_encoding.as_deref(),
            content_language: self.content_language.as_deref(),
            content_length: self.content_length,
            content_range: self.content_range.as_deref(),
            content_type: self.content_type.as_deref(),
            delete_marker: self.delete_marker,
            e_tag: self.e_tag.as_deref(),
            expiration: self.expiration.as_deref(),
            expires: self.expires.as_deref(),
            last_modified: self.last_modified.as_ref(),
            metadata: &self.metadata,
            missing_meta: self.missing_meta,
            object_lock_legal_hold_status: self.object_lock_legal_hold_status.as_ref(),
            object_lock_mode: self.object_lock_mode.as_ref(),
            object_lock_retain_until_date: self.object_lock_retain_until_date.as_ref(),
            parts_count: self.parts_count,
            replication_status: self.replication_status.as_ref(),
            request_charged: self.request_charged.as_ref(),
            restore: self.restore.as_deref(),
            sse_customer_algorithm: self.sse_customer_algorithm.as_deref(),
            sse_customer_key_md5: self.sse_customer_key_md5.as_deref(),
            ssekms_key_id: self.ssekms_key_id.as_deref(),
            server_side_encryption: self.server_side_encryption.as_ref(),
            storage_class: self.storage_class.as_ref(),
            tag_count: self.tag_count,
            version_id: self.version_id.as_deref(),
            website_redirect_location: self.website_redirect_location.as_deref(),
        };

        let mut builder = http::Response::builder().status(http::StatusCode::OK);
        // HeadObject has an additional archive_status header not present in GetObject.
        builder = set_optional_display_header(
            builder,
            "x-amz-archive-status",
            self.archive_status.as_ref(),
        );
        let builder = set_object_metadata_headers(builder, &headers);
        build_response(builder, S3ResponseBody::empty())
    }
}

impl IntoS3Response for PutObjectOutput {
    fn into_s3_response(self) -> Result<http::Response<S3ResponseBody>, S3Error> {
        let mut builder = http::Response::builder().status(http::StatusCode::OK);
        builder = set_optional_bool_header(
            builder,
            "x-amz-server-side-encryption-bucket-key-enabled",
            self.bucket_key_enabled,
        );
        builder = set_optional_header(
            builder,
            "x-amz-checksum-crc32",
            self.checksum_crc32.as_deref(),
        );
        builder = set_optional_header(
            builder,
            "x-amz-checksum-crc32c",
            self.checksum_crc32c.as_deref(),
        );
        builder = set_optional_header(
            builder,
            "x-amz-checksum-crc64nvme",
            self.checksum_crc64nvme.as_deref(),
        );
        builder = set_optional_header(
            builder,
            "x-amz-checksum-sha1",
            self.checksum_sha1.as_deref(),
        );
        builder = set_optional_header(
            builder,
            "x-amz-checksum-sha256",
            self.checksum_sha256.as_deref(),
        );
        builder = set_optional_display_header(
            builder,
            "x-amz-checksum-type",
            self.checksum_type.as_ref(),
        );
        builder = set_optional_header(builder, "ETag", self.e_tag.as_deref());
        builder = set_optional_header(builder, "x-amz-expiration", self.expiration.as_deref());
        builder = set_optional_display_header(
            builder,
            "x-amz-request-charged",
            self.request_charged.as_ref(),
        );
        builder = set_optional_header(
            builder,
            "x-amz-server-side-encryption-customer-algorithm",
            self.sse_customer_algorithm.as_deref(),
        );
        builder = set_optional_header(
            builder,
            "x-amz-server-side-encryption-customer-key-MD5",
            self.sse_customer_key_md5.as_deref(),
        );
        builder = set_optional_header(
            builder,
            "x-amz-server-side-encryption-context",
            self.ssekms_encryption_context.as_deref(),
        );
        builder = set_optional_header(
            builder,
            "x-amz-server-side-encryption-aws-kms-key-id",
            self.ssekms_key_id.as_deref(),
        );
        builder = set_optional_display_header(
            builder,
            "x-amz-server-side-encryption",
            self.server_side_encryption.as_ref(),
        );
        builder = set_optional_int_header(builder, "x-amz-object-size", self.size);
        builder = set_optional_header(builder, "x-amz-version-id", self.version_id.as_deref());
        build_response(builder, S3ResponseBody::empty())
    }
}

impl IntoS3Response for CopyObjectOutput {
    fn into_s3_response(self) -> Result<http::Response<S3ResponseBody>, S3Error> {
        let mut builder = http::Response::builder().status(http::StatusCode::OK);
        builder = set_optional_bool_header(
            builder,
            "x-amz-server-side-encryption-bucket-key-enabled",
            self.bucket_key_enabled,
        );
        builder = set_optional_header(
            builder,
            "x-amz-copy-source-version-id",
            self.copy_source_version_id.as_deref(),
        );
        builder = set_optional_header(builder, "x-amz-expiration", self.expiration.as_deref());
        builder = set_optional_display_header(
            builder,
            "x-amz-request-charged",
            self.request_charged.as_ref(),
        );
        builder = set_optional_header(
            builder,
            "x-amz-server-side-encryption-customer-algorithm",
            self.sse_customer_algorithm.as_deref(),
        );
        builder = set_optional_header(
            builder,
            "x-amz-server-side-encryption-customer-key-MD5",
            self.sse_customer_key_md5.as_deref(),
        );
        builder = set_optional_header(
            builder,
            "x-amz-server-side-encryption-context",
            self.ssekms_encryption_context.as_deref(),
        );
        builder = set_optional_header(
            builder,
            "x-amz-server-side-encryption-aws-kms-key-id",
            self.ssekms_key_id.as_deref(),
        );
        builder = set_optional_display_header(
            builder,
            "x-amz-server-side-encryption",
            self.server_side_encryption.as_ref(),
        );
        builder = set_optional_header(builder, "x-amz-version-id", self.version_id.as_deref());
        builder = builder.header("Content-Type", "application/xml");

        let body = if let Some(ref result) = self.copy_object_result {
            serialize_xml_body("CopyObjectResult", result)?
        } else {
            S3ResponseBody::empty()
        };
        build_response(builder, body)
    }
}

impl IntoS3Response for DeleteObjectOutput {
    fn into_s3_response(self) -> Result<http::Response<S3ResponseBody>, S3Error> {
        let mut builder = http::Response::builder().status(http::StatusCode::NO_CONTENT);
        builder = set_optional_bool_header(builder, "x-amz-delete-marker", self.delete_marker);
        builder = set_optional_display_header(
            builder,
            "x-amz-request-charged",
            self.request_charged.as_ref(),
        );
        builder = set_optional_header(builder, "x-amz-version-id", self.version_id.as_deref());
        build_response(builder, S3ResponseBody::empty())
    }
}

impl IntoS3Response for DeleteObjectsOutput {
    fn into_s3_response(self) -> Result<http::Response<S3ResponseBody>, S3Error> {
        let body = serialize_xml_body("DeleteResult", &self)?;
        let mut builder = http::Response::builder()
            .status(http::StatusCode::OK)
            .header("Content-Type", "application/xml");
        builder = set_optional_display_header(
            builder,
            "x-amz-request-charged",
            self.request_charged.as_ref(),
        );
        build_response(builder, body)
    }
}

// --- Config GET operations (return XML) ---

/// Macro for config GET outputs that return an XML body with Content-Type header.
macro_rules! impl_xml_body_response {
    ($ty:ty, $root:expr) => {
        impl IntoS3Response for $ty {
            fn into_s3_response(self) -> Result<http::Response<S3ResponseBody>, S3Error> {
                let body = serialize_xml_body($root, &self)?;
                let builder = http::Response::builder()
                    .status(http::StatusCode::OK)
                    .header("Content-Type", "application/xml");
                build_response(builder, body)
            }
        }
    };
}

impl_xml_body_response!(
    GetBucketAccelerateConfigurationOutput,
    "AccelerateConfiguration"
);
impl_xml_body_response!(GetBucketAclOutput, "AccessControlPolicy");
impl_xml_body_response!(GetBucketCorsOutput, "CORSConfiguration");
impl_xml_body_response!(
    GetBucketEncryptionOutput,
    "ServerSideEncryptionConfiguration"
);
impl_xml_body_response!(GetBucketLoggingOutput, "BucketLoggingStatus");
impl_xml_body_response!(
    GetBucketNotificationConfigurationOutput,
    "NotificationConfiguration"
);
impl_xml_body_response!(GetBucketOwnershipControlsOutput, "OwnershipControls");
impl_xml_body_response!(GetBucketPolicyStatusOutput, "PolicyStatus");
impl_xml_body_response!(GetBucketRequestPaymentOutput, "RequestPaymentConfiguration");
impl_xml_body_response!(GetBucketTaggingOutput, "Tagging");
impl_xml_body_response!(GetBucketVersioningOutput, "VersioningConfiguration");
impl_xml_body_response!(GetBucketWebsiteOutput, "WebsiteConfiguration");
impl_xml_body_response!(GetObjectAclOutput, "AccessControlPolicy");
impl_xml_body_response!(GetObjectLegalHoldOutput, "LegalHold");
impl_xml_body_response!(GetObjectLockConfigurationOutput, "ObjectLockConfiguration");
impl_xml_body_response!(GetObjectRetentionOutput, "Retention");
impl_xml_body_response!(GetPublicAccessBlockOutput, "PublicAccessBlockConfiguration");
impl_xml_body_response!(ListObjectVersionsOutput, "ListVersionsResult");
impl_xml_body_response!(ListMultipartUploadsOutput, "ListMultipartUploadsResult");
impl_xml_body_response!(ListPartsOutput, "ListPartsResult");

impl IntoS3Response for ListObjectsOutput {
    fn into_s3_response(self) -> Result<http::Response<S3ResponseBody>, S3Error> {
        let body = serialize_xml_body("ListBucketResult", &self)?;
        let builder = http::Response::builder()
            .status(http::StatusCode::OK)
            .header("Content-Type", "application/xml");
        build_response(builder, body)
    }
}

impl IntoS3Response for ListObjectsV2Output {
    fn into_s3_response(self) -> Result<http::Response<S3ResponseBody>, S3Error> {
        let body = serialize_xml_body("ListBucketResult", &self)?;
        let builder = http::Response::builder()
            .status(http::StatusCode::OK)
            .header("Content-Type", "application/xml");
        build_response(builder, body)
    }
}

impl IntoS3Response for GetBucketPolicyOutput {
    fn into_s3_response(self) -> Result<http::Response<S3ResponseBody>, S3Error> {
        let body = if let Some(policy) = self.policy {
            S3ResponseBody::from_string(policy)
        } else {
            S3ResponseBody::empty()
        };
        let builder = http::Response::builder()
            .status(http::StatusCode::OK)
            .header("Content-Type", "application/json");
        build_response(builder, body)
    }
}

impl IntoS3Response for GetBucketLifecycleConfigurationOutput {
    fn into_s3_response(self) -> Result<http::Response<S3ResponseBody>, S3Error> {
        let body = serialize_xml_body("LifecycleConfiguration", &self)?;
        let mut builder = http::Response::builder()
            .status(http::StatusCode::OK)
            .header("Content-Type", "application/xml");
        builder = set_optional_display_header(
            builder,
            "x-amz-transition-default-minimum-object-size",
            self.transition_default_minimum_object_size.as_ref(),
        );
        build_response(builder, body)
    }
}

impl IntoS3Response for GetObjectTaggingOutput {
    fn into_s3_response(self) -> Result<http::Response<S3ResponseBody>, S3Error> {
        let body = serialize_xml_body("Tagging", &self)?;
        let mut builder = http::Response::builder()
            .status(http::StatusCode::OK)
            .header("Content-Type", "application/xml");
        builder = set_optional_header(builder, "x-amz-version-id", self.version_id.as_deref());
        build_response(builder, body)
    }
}

impl IntoS3Response for GetObjectAttributesOutput {
    fn into_s3_response(self) -> Result<http::Response<S3ResponseBody>, S3Error> {
        let body = serialize_xml_body("GetObjectAttributesOutput", &self)?;
        let mut builder = http::Response::builder()
            .status(http::StatusCode::OK)
            .header("Content-Type", "application/xml");
        builder = set_optional_bool_header(builder, "x-amz-delete-marker", self.delete_marker);
        builder =
            set_optional_timestamp_header(builder, "Last-Modified", self.last_modified.as_ref());
        builder = set_optional_display_header(
            builder,
            "x-amz-request-charged",
            self.request_charged.as_ref(),
        );
        builder = set_optional_header(builder, "x-amz-version-id", self.version_id.as_deref());
        build_response(builder, body)
    }
}

// --- Delete/config operations (no body, 200) ---

/// Empty 200 response for PUT bucket-config operations.
fn empty_200_response() -> Result<http::Response<S3ResponseBody>, S3Error> {
    build_response(
        http::Response::builder().status(http::StatusCode::OK),
        S3ResponseBody::empty(),
    )
}

/// Implement `IntoS3Response` that returns 200 OK with no body.
macro_rules! impl_empty_200_response {
    ($ty:ty) => {
        impl IntoS3Response for $ty {
            fn into_s3_response(self) -> Result<http::Response<S3ResponseBody>, S3Error> {
                empty_200_response()
            }
        }
    };
}

// Bucket config put operations return 200 OK.
impl_empty_200_response!(PutBucketLifecycleConfigurationOutput);
impl_empty_200_response!(PutObjectAclOutput);
impl_empty_200_response!(PutObjectLegalHoldOutput);
impl_empty_200_response!(PutObjectLockConfigurationOutput);
impl_empty_200_response!(PutObjectRetentionOutput);

impl IntoS3Response for DeleteObjectTaggingOutput {
    fn into_s3_response(self) -> Result<http::Response<S3ResponseBody>, S3Error> {
        let mut builder = http::Response::builder().status(http::StatusCode::NO_CONTENT);
        builder = set_optional_header(builder, "x-amz-version-id", self.version_id.as_deref());
        build_response(builder, S3ResponseBody::empty())
    }
}

impl IntoS3Response for PutObjectTaggingOutput {
    fn into_s3_response(self) -> Result<http::Response<S3ResponseBody>, S3Error> {
        let mut builder = http::Response::builder().status(http::StatusCode::OK);
        builder = set_optional_header(builder, "x-amz-version-id", self.version_id.as_deref());
        build_response(builder, S3ResponseBody::empty())
    }
}

// --- Multipart operations ---

impl IntoS3Response for CreateMultipartUploadOutput {
    fn into_s3_response(self) -> Result<http::Response<S3ResponseBody>, S3Error> {
        let mut builder = http::Response::builder()
            .status(http::StatusCode::OK)
            .header("Content-Type", "application/xml");
        builder =
            set_optional_timestamp_header(builder, "x-amz-abort-date", self.abort_date.as_ref());
        builder = set_optional_header(
            builder,
            "x-amz-abort-rule-id",
            self.abort_rule_id.as_deref(),
        );
        builder = set_optional_bool_header(
            builder,
            "x-amz-server-side-encryption-bucket-key-enabled",
            self.bucket_key_enabled,
        );
        builder = set_optional_display_header(
            builder,
            "x-amz-checksum-algorithm",
            self.checksum_algorithm.as_ref(),
        );
        builder = set_optional_display_header(
            builder,
            "x-amz-checksum-type",
            self.checksum_type.as_ref(),
        );
        builder = set_optional_display_header(
            builder,
            "x-amz-request-charged",
            self.request_charged.as_ref(),
        );
        builder = set_optional_header(
            builder,
            "x-amz-server-side-encryption-customer-algorithm",
            self.sse_customer_algorithm.as_deref(),
        );
        builder = set_optional_header(
            builder,
            "x-amz-server-side-encryption-customer-key-MD5",
            self.sse_customer_key_md5.as_deref(),
        );
        builder = set_optional_header(
            builder,
            "x-amz-server-side-encryption-context",
            self.ssekms_encryption_context.as_deref(),
        );
        builder = set_optional_header(
            builder,
            "x-amz-server-side-encryption-aws-kms-key-id",
            self.ssekms_key_id.as_deref(),
        );
        builder = set_optional_display_header(
            builder,
            "x-amz-server-side-encryption",
            self.server_side_encryption.as_ref(),
        );
        let body = serialize_xml_body("InitiateMultipartUploadResult", &self)?;
        build_response(builder, body)
    }
}

impl IntoS3Response for CompleteMultipartUploadOutput {
    fn into_s3_response(self) -> Result<http::Response<S3ResponseBody>, S3Error> {
        let mut builder = http::Response::builder()
            .status(http::StatusCode::OK)
            .header("Content-Type", "application/xml");
        builder = set_optional_bool_header(
            builder,
            "x-amz-server-side-encryption-bucket-key-enabled",
            self.bucket_key_enabled,
        );
        builder = set_optional_header(builder, "x-amz-expiration", self.expiration.as_deref());
        builder = set_optional_display_header(
            builder,
            "x-amz-request-charged",
            self.request_charged.as_ref(),
        );
        builder = set_optional_header(
            builder,
            "x-amz-server-side-encryption-aws-kms-key-id",
            self.ssekms_key_id.as_deref(),
        );
        builder = set_optional_display_header(
            builder,
            "x-amz-server-side-encryption",
            self.server_side_encryption.as_ref(),
        );
        builder = set_optional_header(builder, "x-amz-version-id", self.version_id.as_deref());
        let body = serialize_xml_body("CompleteMultipartUploadResult", &self)?;
        build_response(builder, body)
    }
}

impl IntoS3Response for AbortMultipartUploadOutput {
    fn into_s3_response(self) -> Result<http::Response<S3ResponseBody>, S3Error> {
        let mut builder = http::Response::builder().status(http::StatusCode::NO_CONTENT);
        builder = set_optional_display_header(
            builder,
            "x-amz-request-charged",
            self.request_charged.as_ref(),
        );
        build_response(builder, S3ResponseBody::empty())
    }
}

impl IntoS3Response for UploadPartOutput {
    fn into_s3_response(self) -> Result<http::Response<S3ResponseBody>, S3Error> {
        let mut builder = http::Response::builder().status(http::StatusCode::OK);
        builder = set_optional_bool_header(
            builder,
            "x-amz-server-side-encryption-bucket-key-enabled",
            self.bucket_key_enabled,
        );
        builder = set_optional_header(
            builder,
            "x-amz-checksum-crc32",
            self.checksum_crc32.as_deref(),
        );
        builder = set_optional_header(
            builder,
            "x-amz-checksum-crc32c",
            self.checksum_crc32c.as_deref(),
        );
        builder = set_optional_header(
            builder,
            "x-amz-checksum-crc64nvme",
            self.checksum_crc64nvme.as_deref(),
        );
        builder = set_optional_header(
            builder,
            "x-amz-checksum-sha1",
            self.checksum_sha1.as_deref(),
        );
        builder = set_optional_header(
            builder,
            "x-amz-checksum-sha256",
            self.checksum_sha256.as_deref(),
        );
        builder = set_optional_header(builder, "ETag", self.e_tag.as_deref());
        builder = set_optional_display_header(
            builder,
            "x-amz-request-charged",
            self.request_charged.as_ref(),
        );
        builder = set_optional_header(
            builder,
            "x-amz-server-side-encryption-customer-algorithm",
            self.sse_customer_algorithm.as_deref(),
        );
        builder = set_optional_header(
            builder,
            "x-amz-server-side-encryption-customer-key-MD5",
            self.sse_customer_key_md5.as_deref(),
        );
        builder = set_optional_header(
            builder,
            "x-amz-server-side-encryption-aws-kms-key-id",
            self.ssekms_key_id.as_deref(),
        );
        builder = set_optional_display_header(
            builder,
            "x-amz-server-side-encryption",
            self.server_side_encryption.as_ref(),
        );
        build_response(builder, S3ResponseBody::empty())
    }
}

impl IntoS3Response for UploadPartCopyOutput {
    fn into_s3_response(self) -> Result<http::Response<S3ResponseBody>, S3Error> {
        let mut builder = http::Response::builder()
            .status(http::StatusCode::OK)
            .header("Content-Type", "application/xml");
        builder = set_optional_bool_header(
            builder,
            "x-amz-server-side-encryption-bucket-key-enabled",
            self.bucket_key_enabled,
        );
        builder = set_optional_header(
            builder,
            "x-amz-copy-source-version-id",
            self.copy_source_version_id.as_deref(),
        );
        builder = set_optional_display_header(
            builder,
            "x-amz-request-charged",
            self.request_charged.as_ref(),
        );
        builder = set_optional_header(
            builder,
            "x-amz-server-side-encryption-customer-algorithm",
            self.sse_customer_algorithm.as_deref(),
        );
        builder = set_optional_header(
            builder,
            "x-amz-server-side-encryption-customer-key-MD5",
            self.sse_customer_key_md5.as_deref(),
        );
        builder = set_optional_header(
            builder,
            "x-amz-server-side-encryption-aws-kms-key-id",
            self.ssekms_key_id.as_deref(),
        );
        builder = set_optional_display_header(
            builder,
            "x-amz-server-side-encryption",
            self.server_side_encryption.as_ref(),
        );
        let body = if let Some(ref result) = self.copy_part_result {
            serialize_xml_body("CopyPartResult", result)?
        } else {
            S3ResponseBody::empty()
        };
        build_response(builder, body)
    }
}

// --- S3Error to HTTP response ---

/// Convert an S3Error into an HTTP error response with an XML body.
#[must_use]
pub fn error_to_response(err: &S3Error, request_id: &str) -> http::Response<S3ResponseBody> {
    let xml_bytes = rustack_s3_xml::error::error_to_xml(
        err.code.as_str(),
        &err.message,
        err.resource.as_deref(),
        request_id,
    );

    let status = err.status_code;
    let body = S3ResponseBody::from_bytes(Bytes::from(xml_bytes));

    // Build the error response - this should not fail for valid status codes.
    let mut builder = http::Response::builder()
        .status(status)
        .header("Content-Type", "application/xml");

    // Add any extra headers attached to the error (e.g. x-amz-delete-marker).
    if let Some(ref headers) = err.headers {
        for (name, value) in headers.iter() {
            builder = builder.header(name.as_str(), value.as_str());
        }
    }

    builder.body(body).unwrap_or_else(|_| {
        http::Response::builder()
            .status(http::StatusCode::INTERNAL_SERVER_ERROR)
            .body(S3ResponseBody::empty())
            .expect("static response should be valid")
    })
}

#[cfg(test)]
mod tests {
    use std::{error::Error, io, mem::size_of};

    use http_body_util::BodyExt;
    use rustack_s3_model::request::StreamingBlob;

    use super::*;

    #[test]
    fn test_should_create_put_object_response() {
        let output = PutObjectOutput {
            e_tag: Some("\"abc123\"".to_owned()),
            version_id: Some("v1".to_owned()),
            ..Default::default()
        };
        let resp = output.into_s3_response().expect("should build response");
        assert_eq!(resp.status(), http::StatusCode::OK);
        assert_eq!(
            resp.headers().get("ETag").and_then(|v| v.to_str().ok()),
            Some("\"abc123\""),
        );
        assert_eq!(
            resp.headers()
                .get("x-amz-version-id")
                .and_then(|v| v.to_str().ok()),
            Some("v1"),
        );
    }

    #[test]
    fn test_should_create_delete_object_response() {
        let output = DeleteObjectOutput {
            delete_marker: Some(true),
            ..Default::default()
        };
        let resp = output.into_s3_response().expect("should build response");
        assert_eq!(resp.status(), http::StatusCode::NO_CONTENT);
        assert_eq!(
            resp.headers()
                .get("x-amz-delete-marker")
                .and_then(|v| v.to_str().ok()),
            Some("true"),
        );
    }

    #[test]
    fn test_should_create_error_response() {
        let err = S3Error::no_such_bucket("my-bucket");
        let resp = error_to_response(&err, "req-123");
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
        assert_eq!(
            resp.headers()
                .get("Content-Type")
                .and_then(|v| v.to_str().ok()),
            Some("application/xml"),
        );
    }

    #[tokio::test]
    async fn test_should_preserve_compact_error_metadata_in_response() {
        // Keep errors below Clippy's large-Err threshold without boxing every Result.
        assert!(size_of::<S3Error>() < 128);
        let err = S3Error::no_such_key("folder/a&b")
            .with_request_id("request-123")
            .with_source(io::Error::other("storage failure"))
            .with_header("x-amz-delete-marker", "true");
        assert_eq!(err.resource.as_deref(), Some("folder/a&b"));
        assert_eq!(err.request_id.as_deref(), Some("request-123"));
        assert_eq!(
            err.source().map(ToString::to_string).as_deref(),
            Some("storage failure")
        );

        let response = error_to_response(&err, "request-123");
        assert_eq!(response.status(), http::StatusCode::NOT_FOUND);
        assert_eq!(response.headers()["x-amz-delete-marker"], "true");
        let body = response
            .into_body()
            .collect()
            .await
            .expect("error body")
            .to_bytes();
        let xml = String::from_utf8(body.to_vec()).expect("UTF-8 error XML");
        assert!(xml.contains("<Code>NoSuchKey</Code>"));
        assert!(xml.contains("<Resource>folder/a&amp;b</Resource>"));
        assert!(xml.contains("<RequestId>request-123</RequestId>"));
    }

    #[test]
    fn test_should_create_head_bucket_response() {
        let output = HeadBucketOutput {
            bucket_region: Some("us-east-1".to_owned()),
            ..Default::default()
        };
        let resp = output.into_s3_response().expect("should build response");
        assert_eq!(resp.status(), http::StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get("x-amz-bucket-region")
                .and_then(|v| v.to_str().ok()),
            Some("us-east-1"),
        );
    }

    #[test]
    fn test_should_create_get_object_response_with_body() {
        let output = GetObjectOutput {
            body: Some(StreamingBlob::new(Bytes::from("file content"))),
            content_type: Some("text/plain".to_owned()),
            content_length: Some(12),
            e_tag: Some("\"etag\"".to_owned()),
            ..Default::default()
        };
        let resp = output.into_s3_response().expect("should build response");
        assert_eq!(resp.status(), http::StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get("Content-Type")
                .and_then(|v| v.to_str().ok()),
            Some("text/plain"),
        );
    }

    #[test]
    fn test_should_create_get_bucket_location_response() {
        let output = GetBucketLocationOutput {
            location_constraint: None,
        };
        let resp = output.into_s3_response().expect("should build response");
        assert_eq!(resp.status(), http::StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get("Content-Type")
                .and_then(|v| v.to_str().ok()),
            Some("application/xml"),
        );
    }
}
