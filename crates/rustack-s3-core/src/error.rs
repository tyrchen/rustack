//! S3-specific error types.
//!
//! Defines [`S3ServiceError`], a domain-specific error enum covering all S3
//! error codes that Rustack may produce. Each variant maps to a concrete
//! [`rustack_s3_model::error::S3ErrorCode`] and HTTP status code through
//! the [`From`] implementation, making it easy to convert domain errors into
//! wire errors.
//!
//! # Usage
//!
//! ```
//! use rustack_s3_core::error::S3ServiceError;
//! use rustack_s3_model::error::{S3Error, S3ErrorCode};
//!
//! let err = S3ServiceError::NoSuchBucket {
//!     bucket: "my-bucket".to_owned(),
//! };
//! let s3_err: S3Error = err.into();
//! assert_eq!(s3_err.code, S3ErrorCode::NoSuchBucket);
//! ```

use rustack_s3_model::error::{S3Error, S3ErrorCode};

/// S3 service error type.
///
/// Each variant corresponds to a well-known S3 error code. Converting to
/// [`S3Error`] via [`From`] attaches the correct error code and a
/// human-readable message; the HTTP status code is derived automatically from
/// the error code.
#[derive(Debug, thiserror::Error)]
pub enum S3ServiceError {
    // -----------------------------------------------------------------------
    // Bucket errors
    // -----------------------------------------------------------------------
    /// The specified bucket does not exist.
    #[error("The specified bucket does not exist: {bucket}")]
    NoSuchBucket {
        /// The bucket name that was not found.
        bucket: String,
    },

    /// The requested bucket name is not available (owned by another account).
    #[error("The requested bucket name is not available: {bucket}")]
    BucketAlreadyExists {
        /// The bucket name that already exists.
        bucket: String,
    },

    /// The bucket already exists and is owned by you.
    #[error(
        "Your previous request to create the named bucket succeeded and you already own it: \
         {bucket}"
    )]
    BucketAlreadyOwnedByYou {
        /// The bucket name that already exists.
        bucket: String,
    },

    /// The bucket is not empty and cannot be deleted.
    #[error("The bucket you tried to delete is not empty: {bucket}")]
    BucketNotEmpty {
        /// The bucket name that is not empty.
        bucket: String,
    },

    // -----------------------------------------------------------------------
    // Object / key errors
    // -----------------------------------------------------------------------
    /// The specified key does not exist.
    #[error("The specified key does not exist: {key}")]
    NoSuchKey {
        /// The key that was not found.
        key: String,
    },

    /// The specified version does not exist.
    #[error("The specified version does not exist: key={key}, version_id={version_id}")]
    NoSuchVersion {
        /// The key for the version.
        key: String,
        /// The version ID that was not found.
        version_id: String,
    },

    // -----------------------------------------------------------------------
    // Multipart upload errors
    // -----------------------------------------------------------------------
    /// The specified multipart upload does not exist.
    #[error("The specified upload does not exist: {upload_id}")]
    NoSuchUpload {
        /// The upload ID that was not found.
        upload_id: String,
    },

    /// The list of parts was not in ascending order.
    #[error("The list of parts was not in ascending order")]
    InvalidPartOrder,

    /// One or more of the specified parts could not be found.
    #[error("One or more of the specified parts could not be found")]
    InvalidPart,

    /// A proposed upload part is smaller than the minimum allowed size.
    #[error("Your proposed upload is smaller than the minimum allowed object size")]
    EntityTooSmall,

    /// The entity body is too large.
    #[error("Your proposed upload exceeds the maximum allowed object size")]
    EntityTooLarge,

    // -----------------------------------------------------------------------
    // Validation errors
    // -----------------------------------------------------------------------
    /// The specified bucket name is not valid.
    #[error("Invalid bucket name: {name}: {reason}")]
    InvalidBucketName {
        /// The invalid bucket name.
        name: String,
        /// The reason for the error.
        reason: String,
    },

    /// An argument provided is invalid.
    #[error("Invalid argument: {message}")]
    InvalidArgument {
        /// Description of the invalid argument.
        message: String,
    },

    /// The requested range is not satisfiable.
    #[error("The requested range is not satisfiable")]
    InvalidRange,

    /// A tag key or value is invalid.
    #[error("Invalid tag: {message}")]
    InvalidTag {
        /// Description of the tag error.
        message: String,
    },

    /// The XML body is malformed.
    #[error("The XML you provided was not well-formed")]
    MalformedXml,

    // -----------------------------------------------------------------------
    // Authorization / access errors
    // -----------------------------------------------------------------------
    /// Access denied.
    #[error("Access Denied")]
    AccessDenied,

    /// The HTTP method is not allowed against this resource.
    #[error("The specified method is not allowed against this resource")]
    MethodNotAllowed,

    // -----------------------------------------------------------------------
    // Feature / implementation errors
    // -----------------------------------------------------------------------
    /// The requested functionality is not implemented.
    #[error("A header you provided implies functionality that is not implemented")]
    NotImplemented,

    // -----------------------------------------------------------------------
    // Conditional request errors
    // -----------------------------------------------------------------------
    /// A precondition specified in the request was not met.
    #[error("At least one of the preconditions you specified did not hold")]
    PreconditionFailed,

    /// The conditional request cannot be processed.
    #[error("The conditional request cannot be processed")]
    ConditionalRequestConflict,

    /// The resource has not been modified (304).
    #[error("Not Modified")]
    NotModified,

    // -----------------------------------------------------------------------
    // Object state errors
    // -----------------------------------------------------------------------
    /// The operation is not valid for the object's storage class.
    #[error("The operation is not valid for the object's storage class")]
    InvalidObjectState,

    /// The object is not in an active tier.
    #[error("The source object of the COPY action is not in the active tier")]
    ObjectNotInActiveTierError,

    // -----------------------------------------------------------------------
    // Digest / content errors
    // -----------------------------------------------------------------------
    /// The Content-MD5 you specified is invalid.
    #[error("The Content-MD5 you specified is not valid")]
    InvalidDigest,

    /// The Content-MD5 you specified did not match what we received.
    #[error("The Content-MD5 you specified did not match what we received")]
    BadDigest,

    /// Missing Content-Length header.
    #[error("You must provide the Content-Length HTTP header")]
    MissingContentLength,

    /// The key is too long.
    #[error("Your key is too long")]
    KeyTooLong,

    /// The message body exceeds the maximum length.
    #[error("Your request was too big")]
    MaxMessageLengthExceeded,

    // -----------------------------------------------------------------------
    // Configuration-not-found errors
    // -----------------------------------------------------------------------
    /// The CORS configuration does not exist.
    #[error("The CORS configuration does not exist")]
    NoSuchCorsConfiguration,

    /// The tag set does not exist.
    #[error("The TagSet does not exist")]
    NoSuchTagSet,

    /// The lifecycle configuration does not exist.
    #[error("The lifecycle configuration does not exist")]
    NoSuchLifecycleConfiguration,

    /// The bucket policy does not exist.
    #[error("The bucket policy does not exist")]
    NoSuchBucketPolicy,

    /// The website configuration does not exist.
    #[error("The specified bucket does not have a website configuration")]
    NoSuchWebsiteConfiguration,

    /// The public access block configuration does not exist.
    #[error("The public access block configuration was not found")]
    NoSuchPublicAccessBlockConfiguration,

    /// The server-side encryption configuration does not exist.
    #[error("The server-side encryption configuration was not found")]
    ServerSideEncryptionConfigurationNotFoundError,

    /// The object lock configuration does not exist.
    #[error("Object Lock configuration does not exist for this bucket")]
    ObjectLockConfigurationNotFoundError,

    /// The ownership controls configuration does not exist.
    #[error("The bucket ownership controls were not found")]
    OwnershipControlsNotFoundError,

    /// The replication configuration does not exist.
    #[error("The replication configuration was not found")]
    ReplicationConfigurationNotFoundError,

    // -----------------------------------------------------------------------
    // Internal / catch-all
    // -----------------------------------------------------------------------
    /// Internal error with context.
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl S3ServiceError {
    /// Convert this error into an [`S3Error`].
    ///
    /// This is equivalent to `S3Error::from(self)` but available as a
    /// method for convenience in chained calls.
    #[must_use]
    pub fn into_s3_error(self) -> S3Error {
        S3Error::from(self)
    }
}

impl From<S3ServiceError> for S3Error {
    fn from(err: S3ServiceError) -> Self {
        let code = error_code(&err);
        // Use the error code's standard message for the XML <Message> element.
        // SDKs compare message strings exactly, so context-specific details
        // (bucket/key names) must not appear in <Message>.
        // For validation errors, preserve the specific message.
        let message = match &err {
            S3ServiceError::InvalidArgument { message }
            | S3ServiceError::InvalidTag { message } => message.clone(),
            S3ServiceError::InvalidBucketName { name, reason } => {
                format!("Invalid bucket name: {name}: {reason}")
            }
            S3ServiceError::Internal(e) => e.to_string(),
            _ => code.default_message().to_owned(),
        };
        S3Error::with_message(code, message)
    }
}

/// Map an [`S3ServiceError`] variant to the corresponding [`S3ErrorCode`].
fn error_code(err: &S3ServiceError) -> S3ErrorCode {
    match err {
        S3ServiceError::NoSuchBucket { .. } => S3ErrorCode::NoSuchBucket,
        S3ServiceError::BucketAlreadyExists { .. } => S3ErrorCode::BucketAlreadyExists,
        S3ServiceError::BucketAlreadyOwnedByYou { .. } => S3ErrorCode::BucketAlreadyOwnedByYou,
        S3ServiceError::BucketNotEmpty { .. } => S3ErrorCode::BucketNotEmpty,
        S3ServiceError::NoSuchKey { .. } => S3ErrorCode::NoSuchKey,
        S3ServiceError::NoSuchVersion { .. } => S3ErrorCode::NoSuchVersion,
        S3ServiceError::NoSuchUpload { .. } => S3ErrorCode::NoSuchUpload,
        S3ServiceError::InvalidPartOrder => S3ErrorCode::InvalidPartOrder,
        S3ServiceError::InvalidPart => S3ErrorCode::InvalidPart,
        S3ServiceError::EntityTooSmall => S3ErrorCode::EntityTooSmall,
        S3ServiceError::EntityTooLarge => S3ErrorCode::EntityTooLarge,
        S3ServiceError::InvalidBucketName { .. } => S3ErrorCode::InvalidBucketName,
        S3ServiceError::InvalidArgument { .. } | S3ServiceError::InvalidTag { .. } => {
            S3ErrorCode::InvalidArgument
        }
        S3ServiceError::InvalidRange => S3ErrorCode::InvalidRange,
        S3ServiceError::MalformedXml => S3ErrorCode::MalformedXML,
        S3ServiceError::AccessDenied => S3ErrorCode::AccessDenied,
        S3ServiceError::MethodNotAllowed => S3ErrorCode::MethodNotAllowed,
        S3ServiceError::NotImplemented => S3ErrorCode::NotImplemented,
        S3ServiceError::PreconditionFailed => S3ErrorCode::PreconditionFailed,
        S3ServiceError::ConditionalRequestConflict => S3ErrorCode::ConditionalRequestConflict,
        S3ServiceError::NotModified => S3ErrorCode::NotModified,
        S3ServiceError::InvalidObjectState => S3ErrorCode::InvalidObjectState,
        S3ServiceError::ObjectNotInActiveTierError => S3ErrorCode::ObjectNotInActiveTierError,
        S3ServiceError::InvalidDigest => S3ErrorCode::InvalidDigest,
        S3ServiceError::BadDigest => S3ErrorCode::BadDigest,
        S3ServiceError::MissingContentLength => S3ErrorCode::MissingContentLength,
        S3ServiceError::KeyTooLong => S3ErrorCode::KeyTooLongError,
        S3ServiceError::MaxMessageLengthExceeded => S3ErrorCode::MaxMessageLengthExceeded,
        S3ServiceError::NoSuchCorsConfiguration => S3ErrorCode::NoSuchCORSConfiguration,
        S3ServiceError::NoSuchTagSet => S3ErrorCode::NoSuchTagSet,
        S3ServiceError::NoSuchLifecycleConfiguration => S3ErrorCode::NoSuchLifecycleConfiguration,
        S3ServiceError::NoSuchBucketPolicy => S3ErrorCode::NoSuchBucketPolicy,
        S3ServiceError::NoSuchWebsiteConfiguration => S3ErrorCode::NoSuchWebsiteConfiguration,
        S3ServiceError::NoSuchPublicAccessBlockConfiguration => {
            S3ErrorCode::NoSuchPublicAccessBlockConfiguration
        }
        S3ServiceError::ServerSideEncryptionConfigurationNotFoundError => {
            S3ErrorCode::ServerSideEncryptionConfigurationNotFoundError
        }
        S3ServiceError::ObjectLockConfigurationNotFoundError => {
            S3ErrorCode::ObjectLockConfigurationNotFoundError
        }
        S3ServiceError::OwnershipControlsNotFoundError => {
            S3ErrorCode::OwnershipControlsNotFoundError
        }
        S3ServiceError::ReplicationConfigurationNotFoundError => {
            S3ErrorCode::ReplicationConfigurationNotFoundError
        }
        S3ServiceError::Internal(_) => S3ErrorCode::InternalError,
    }
}

/// Convenience result type for S3 service operations.
pub type S3ServiceResult<T> = Result<T, S3ServiceError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_convert_no_such_bucket_to_s3_error() {
        let err = S3ServiceError::NoSuchBucket {
            bucket: "my-bucket".to_owned(),
        };
        let s3_err: S3Error = err.into();
        assert_eq!(s3_err.code, S3ErrorCode::NoSuchBucket);
        // Message should use the standard S3 error message without bucket name.
        assert_eq!(s3_err.message, "The specified bucket does not exist");
    }

    #[test]
    fn test_should_convert_no_such_key_to_s3_error() {
        let err = S3ServiceError::NoSuchKey {
            key: "path/to/obj".to_owned(),
        };
        let s3_err: S3Error = err.into();
        assert_eq!(s3_err.code, S3ErrorCode::NoSuchKey);
    }

    #[test]
    fn test_should_convert_bucket_already_exists_to_s3_error() {
        let err = S3ServiceError::BucketAlreadyExists {
            bucket: "taken".to_owned(),
        };
        let s3_err: S3Error = err.into();
        assert_eq!(s3_err.code, S3ErrorCode::BucketAlreadyExists);
    }

    #[test]
    fn test_should_convert_bucket_already_owned_to_s3_error() {
        let err = S3ServiceError::BucketAlreadyOwnedByYou {
            bucket: "mine".to_owned(),
        };
        let s3_err: S3Error = err.into();
        assert_eq!(s3_err.code, S3ErrorCode::BucketAlreadyOwnedByYou);
    }

    #[test]
    fn test_should_convert_bucket_not_empty_to_s3_error() {
        let err = S3ServiceError::BucketNotEmpty {
            bucket: "full".to_owned(),
        };
        let s3_err: S3Error = err.into();
        assert_eq!(s3_err.code, S3ErrorCode::BucketNotEmpty);
    }

    #[test]
    fn test_should_convert_invalid_bucket_name_to_s3_error() {
        let err = S3ServiceError::InvalidBucketName {
            name: "BAD".to_owned(),
            reason: "uppercase".to_owned(),
        };
        let s3_err: S3Error = err.into();
        assert_eq!(s3_err.code, S3ErrorCode::InvalidBucketName);
    }

    #[test]
    fn test_should_convert_entity_too_small_to_s3_error() {
        let err = S3ServiceError::EntityTooSmall;
        let s3_err: S3Error = err.into();
        assert_eq!(s3_err.code, S3ErrorCode::EntityTooSmall);
    }

    #[test]
    fn test_should_convert_access_denied_to_s3_error() {
        let err = S3ServiceError::AccessDenied;
        let s3_err: S3Error = err.into();
        assert_eq!(s3_err.code, S3ErrorCode::AccessDenied);
    }

    #[test]
    fn test_should_convert_internal_error_to_s3_error() {
        let err = S3ServiceError::Internal(anyhow::anyhow!("disk I/O failure"));
        let s3_err: S3Error = err.into();
        assert_eq!(s3_err.code, S3ErrorCode::InternalError);
    }

    #[test]
    fn test_should_use_into_s3_error_method() {
        let err = S3ServiceError::InvalidRange;
        let s3_err = err.into_s3_error();
        assert_eq!(s3_err.code, S3ErrorCode::InvalidRange);
    }

    #[test]
    fn test_should_convert_no_such_upload_to_s3_error() {
        let err = S3ServiceError::NoSuchUpload {
            upload_id: "abc123".to_owned(),
        };
        let s3_err: S3Error = err.into();
        assert_eq!(s3_err.code, S3ErrorCode::NoSuchUpload);
    }

    #[test]
    fn test_should_convert_precondition_failed_to_s3_error() {
        let err = S3ServiceError::PreconditionFailed;
        let s3_err: S3Error = err.into();
        assert_eq!(s3_err.code, S3ErrorCode::PreconditionFailed);
    }

    #[test]
    fn test_should_convert_config_not_found_errors() {
        let cases: Vec<(S3ServiceError, S3ErrorCode)> = vec![
            (
                S3ServiceError::NoSuchCorsConfiguration,
                S3ErrorCode::NoSuchCORSConfiguration,
            ),
            (S3ServiceError::NoSuchTagSet, S3ErrorCode::NoSuchTagSet),
            (
                S3ServiceError::NoSuchLifecycleConfiguration,
                S3ErrorCode::NoSuchLifecycleConfiguration,
            ),
            (
                S3ServiceError::NoSuchBucketPolicy,
                S3ErrorCode::NoSuchBucketPolicy,
            ),
            (
                S3ServiceError::NoSuchWebsiteConfiguration,
                S3ErrorCode::NoSuchWebsiteConfiguration,
            ),
            (
                S3ServiceError::NoSuchPublicAccessBlockConfiguration,
                S3ErrorCode::NoSuchPublicAccessBlockConfiguration,
            ),
            (
                S3ServiceError::ObjectLockConfigurationNotFoundError,
                S3ErrorCode::ObjectLockConfigurationNotFoundError,
            ),
            (
                S3ServiceError::OwnershipControlsNotFoundError,
                S3ErrorCode::OwnershipControlsNotFoundError,
            ),
            (
                S3ServiceError::ReplicationConfigurationNotFoundError,
                S3ErrorCode::ReplicationConfigurationNotFoundError,
            ),
            (
                S3ServiceError::ServerSideEncryptionConfigurationNotFoundError,
                S3ErrorCode::ServerSideEncryptionConfigurationNotFoundError,
            ),
        ];

        for (err, expected_code) in cases {
            let s3_err: S3Error = err.into();
            assert_eq!(s3_err.code, expected_code);
        }
    }

    #[test]
    fn test_should_convert_not_modified_to_304() {
        let err = S3ServiceError::NotModified;
        let s3_err: S3Error = err.into();
        assert_eq!(s3_err.code, S3ErrorCode::NotModified);
        assert_eq!(s3_err.status_code, http::StatusCode::NOT_MODIFIED);
    }

    #[test]
    fn test_should_convert_conditional_request_conflict_to_409() {
        let err = S3ServiceError::ConditionalRequestConflict;
        let s3_err: S3Error = err.into();
        assert_eq!(s3_err.code, S3ErrorCode::ConditionalRequestConflict);
        assert_eq!(s3_err.status_code, http::StatusCode::CONFLICT);
    }

    #[test]
    fn test_should_convert_bad_digest_to_400() {
        let err = S3ServiceError::BadDigest;
        let s3_err: S3Error = err.into();
        assert_eq!(s3_err.code, S3ErrorCode::BadDigest);
        assert_eq!(s3_err.status_code, http::StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_should_convert_max_message_length_exceeded_to_400() {
        let err = S3ServiceError::MaxMessageLengthExceeded;
        let s3_err: S3Error = err.into();
        assert_eq!(s3_err.code, S3ErrorCode::MaxMessageLengthExceeded);
        assert_eq!(s3_err.status_code, http::StatusCode::BAD_REQUEST);
    }
}
