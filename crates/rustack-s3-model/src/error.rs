//! Auto-generated from AWS S3 Smithy model. DO NOT EDIT.

use std::fmt;

/// Well-known S3 error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum S3ErrorCode {
    /// Default error code.
    #[default]
    /// AccessDenied error.
    AccessDenied,
    /// AccountProblem error.
    AccountProblem,
    /// BucketAlreadyExists error.
    BucketAlreadyExists,
    /// BucketAlreadyOwnedByYou error.
    BucketAlreadyOwnedByYou,
    /// BucketNotEmpty error.
    BucketNotEmpty,
    /// EntityTooLarge error.
    EntityTooLarge,
    /// EntityTooSmall error.
    EntityTooSmall,
    /// InternalError error.
    InternalError,
    /// InvalidArgument error.
    InvalidArgument,
    /// InvalidBucketName error.
    InvalidBucketName,
    /// InvalidBucketState error.
    InvalidBucketState,
    /// InvalidDigest error.
    InvalidDigest,
    /// InvalidLocationConstraint error.
    InvalidLocationConstraint,
    /// InvalidObjectState error.
    InvalidObjectState,
    /// InvalidPart error.
    InvalidPart,
    /// InvalidPartOrder error.
    InvalidPartOrder,
    /// InvalidRange error.
    InvalidRange,
    /// InvalidRequest error.
    InvalidRequest,
    /// InvalidStorageClass error.
    InvalidStorageClass,
    /// KeyTooLongError error.
    KeyTooLongError,
    /// MalformedXML error.
    MalformedXML,
    /// MetadataTooLarge error.
    MetadataTooLarge,
    /// MethodNotAllowed error.
    MethodNotAllowed,
    /// MissingContentLength error.
    MissingContentLength,
    /// NoSuchBucket error.
    NoSuchBucket,
    /// NoSuchBucketPolicy error.
    NoSuchBucketPolicy,
    /// NoSuchCORSConfiguration error.
    NoSuchCORSConfiguration,
    /// NoSuchKey error.
    NoSuchKey,
    /// NoSuchLifecycleConfiguration error.
    NoSuchLifecycleConfiguration,
    /// NoSuchUpload error.
    NoSuchUpload,
    /// NoSuchVersion error.
    NoSuchVersion,
    /// NoSuchTagSet error.
    NoSuchTagSet,
    /// NoSuchWebsiteConfiguration error.
    NoSuchWebsiteConfiguration,
    /// NotImplemented error.
    NotImplemented,
    /// ObjectNotInActiveTierError error.
    ObjectNotInActiveTierError,
    /// ObjectLockConfigurationNotFoundError error.
    ObjectLockConfigurationNotFoundError,
    /// PreconditionFailed error.
    PreconditionFailed,
    /// SignatureDoesNotMatch error.
    SignatureDoesNotMatch,
    /// TooManyBuckets error.
    TooManyBuckets,
    /// XAmzContentSHA256Mismatch error.
    XAmzContentSHA256Mismatch,
    /// BadDigest error.
    BadDigest,
    /// ConditionalRequestConflict error.
    ConditionalRequestConflict,
    /// MaxMessageLengthExceeded error.
    MaxMessageLengthExceeded,
    /// NoSuchObjectLockConfiguration error.
    NoSuchObjectLockConfiguration,
    /// NoSuchPublicAccessBlockConfiguration error.
    NoSuchPublicAccessBlockConfiguration,
    /// NotModified error (HTTP 304).
    NotModified,
    /// OwnershipControlsNotFoundError error.
    OwnershipControlsNotFoundError,
    /// ReplicationConfigurationNotFoundError error.
    ReplicationConfigurationNotFoundError,
    /// ServerSideEncryptionConfigurationNotFoundError error.
    ServerSideEncryptionConfigurationNotFoundError,
    /// A custom error code not in the standard set.
    Custom(&'static str),
}

impl S3ErrorCode {
    /// Returns the error code as a string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AccessDenied => "AccessDenied",
            Self::AccountProblem => "AccountProblem",
            Self::BucketAlreadyExists => "BucketAlreadyExists",
            Self::BucketAlreadyOwnedByYou => "BucketAlreadyOwnedByYou",
            Self::BucketNotEmpty => "BucketNotEmpty",
            Self::EntityTooLarge => "EntityTooLarge",
            Self::EntityTooSmall => "EntityTooSmall",
            Self::InternalError => "InternalError",
            Self::InvalidArgument => "InvalidArgument",
            Self::InvalidBucketName => "InvalidBucketName",
            Self::InvalidBucketState => "InvalidBucketState",
            Self::InvalidDigest => "InvalidDigest",
            Self::InvalidLocationConstraint => "InvalidLocationConstraint",
            Self::InvalidObjectState => "InvalidObjectState",
            Self::InvalidPart => "InvalidPart",
            Self::InvalidPartOrder => "InvalidPartOrder",
            Self::InvalidRange => "InvalidRange",
            Self::InvalidRequest => "InvalidRequest",
            Self::InvalidStorageClass => "InvalidStorageClass",
            Self::KeyTooLongError => "KeyTooLongError",
            Self::MalformedXML => "MalformedXML",
            Self::MetadataTooLarge => "MetadataTooLarge",
            Self::MethodNotAllowed => "MethodNotAllowed",
            Self::MissingContentLength => "MissingContentLength",
            Self::NoSuchBucket => "NoSuchBucket",
            Self::NoSuchBucketPolicy => "NoSuchBucketPolicy",
            Self::NoSuchCORSConfiguration => "NoSuchCORSConfiguration",
            Self::NoSuchKey => "NoSuchKey",
            Self::NoSuchLifecycleConfiguration => "NoSuchLifecycleConfiguration",
            Self::NoSuchUpload => "NoSuchUpload",
            Self::NoSuchVersion => "NoSuchVersion",
            Self::NoSuchTagSet => "NoSuchTagSet",
            Self::NoSuchWebsiteConfiguration => "NoSuchWebsiteConfiguration",
            Self::NotImplemented => "NotImplemented",
            Self::ObjectNotInActiveTierError => "ObjectNotInActiveTierError",
            Self::ObjectLockConfigurationNotFoundError => "ObjectLockConfigurationNotFoundError",
            Self::PreconditionFailed => "PreconditionFailed",
            Self::SignatureDoesNotMatch => "SignatureDoesNotMatch",
            Self::TooManyBuckets => "TooManyBuckets",
            Self::XAmzContentSHA256Mismatch => "XAmzContentSHA256Mismatch",
            Self::BadDigest => "BadDigest",
            Self::ConditionalRequestConflict => "ConditionalRequestConflict",
            Self::MaxMessageLengthExceeded => "MaxMessageLengthExceeded",
            Self::NoSuchObjectLockConfiguration => "NoSuchObjectLockConfiguration",
            Self::NoSuchPublicAccessBlockConfiguration => "NoSuchPublicAccessBlockConfiguration",
            Self::NotModified => "NotModified",
            Self::OwnershipControlsNotFoundError => "OwnershipControlsNotFoundError",
            Self::ReplicationConfigurationNotFoundError => "ReplicationConfigurationNotFoundError",
            Self::ServerSideEncryptionConfigurationNotFoundError => {
                "ServerSideEncryptionConfigurationNotFoundError"
            }
            Self::Custom(s) => s,
        }
    }

    /// Returns the default HTTP status code for this error.
    #[must_use]
    #[allow(clippy::match_same_arms)]
    pub fn default_status_code(&self) -> http::StatusCode {
        match self {
            Self::NotModified => http::StatusCode::NOT_MODIFIED,
            Self::BadDigest
            | Self::EntityTooLarge
            | Self::EntityTooSmall
            | Self::InvalidArgument
            | Self::InvalidBucketName
            | Self::InvalidDigest
            | Self::InvalidLocationConstraint
            | Self::InvalidPart
            | Self::InvalidPartOrder
            | Self::InvalidRequest
            | Self::InvalidStorageClass
            | Self::KeyTooLongError
            | Self::MalformedXML
            | Self::MaxMessageLengthExceeded
            | Self::MetadataTooLarge
            | Self::ServerSideEncryptionConfigurationNotFoundError
            | Self::TooManyBuckets
            | Self::XAmzContentSHA256Mismatch => http::StatusCode::BAD_REQUEST,
            Self::AccessDenied
            | Self::AccountProblem
            | Self::InvalidObjectState
            | Self::ObjectNotInActiveTierError
            | Self::SignatureDoesNotMatch => http::StatusCode::FORBIDDEN,
            Self::NoSuchBucket
            | Self::NoSuchBucketPolicy
            | Self::NoSuchCORSConfiguration
            | Self::NoSuchKey
            | Self::NoSuchLifecycleConfiguration
            | Self::ObjectLockConfigurationNotFoundError
            | Self::NoSuchObjectLockConfiguration
            | Self::NoSuchPublicAccessBlockConfiguration
            | Self::NoSuchUpload
            | Self::NoSuchVersion
            | Self::NoSuchTagSet
            | Self::NoSuchWebsiteConfiguration
            | Self::OwnershipControlsNotFoundError
            | Self::ReplicationConfigurationNotFoundError => http::StatusCode::NOT_FOUND,
            Self::MethodNotAllowed => http::StatusCode::METHOD_NOT_ALLOWED,
            Self::BucketAlreadyExists
            | Self::BucketAlreadyOwnedByYou
            | Self::BucketNotEmpty
            | Self::ConditionalRequestConflict
            | Self::InvalidBucketState => http::StatusCode::CONFLICT,
            Self::MissingContentLength => http::StatusCode::LENGTH_REQUIRED,
            Self::PreconditionFailed => http::StatusCode::PRECONDITION_FAILED,
            Self::InvalidRange => http::StatusCode::RANGE_NOT_SATISFIABLE,
            Self::InternalError => http::StatusCode::INTERNAL_SERVER_ERROR,
            Self::NotImplemented => http::StatusCode::NOT_IMPLEMENTED,
            Self::Custom(_) => http::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Returns the default message for this error.
    #[must_use]
    pub fn default_message(&self) -> &'static str {
        match self {
            Self::AccessDenied => "Access Denied",
            Self::AccountProblem => "There is a problem with the account",
            Self::BucketAlreadyExists => "The requested bucket name is not available",
            Self::BucketAlreadyOwnedByYou => "The bucket is already owned by you",
            Self::BucketNotEmpty => "The bucket you tried to delete is not empty",
            Self::EntityTooLarge => "Your proposed upload exceeds the maximum allowed size",
            Self::EntityTooSmall => "Your proposed upload is smaller than the minimum allowed size",
            Self::InternalError => "Internal server error",
            Self::InvalidArgument => "Invalid Argument",
            Self::InvalidBucketName => "The specified bucket is not valid",
            Self::InvalidBucketState => {
                "The request is not valid with the current state of the bucket"
            }
            Self::InvalidDigest => "The Content-MD5 you specified is not valid",
            Self::InvalidLocationConstraint => "The specified location constraint is not valid",
            Self::InvalidObjectState => {
                "The operation is not valid for the current state of the object"
            }
            Self::InvalidPart => "One or more of the specified parts could not be found",
            Self::InvalidPartOrder => "The list of parts was not in ascending order",
            Self::InvalidRange => "The requested range cannot be satisfied",
            Self::InvalidRequest => "Invalid Request",
            Self::InvalidStorageClass => "The storage class you specified is not valid",
            Self::KeyTooLongError => "Your key is too long",
            Self::MalformedXML => "The XML you provided was not well-formed",
            Self::MetadataTooLarge => {
                "Your metadata headers exceed the maximum allowed metadata size"
            }
            Self::MethodNotAllowed => "The specified method is not allowed against this resource",
            Self::MissingContentLength => "You must provide the Content-Length HTTP header",
            Self::NoSuchBucket => "The specified bucket does not exist",
            Self::NoSuchBucketPolicy => "The specified bucket does not have a bucket policy",
            Self::NoSuchCORSConfiguration => "The CORS configuration does not exist",
            Self::NoSuchKey => "The specified key does not exist",
            Self::NoSuchLifecycleConfiguration => "The lifecycle configuration does not exist",
            Self::NoSuchUpload => "The specified multipart upload does not exist",
            Self::NoSuchVersion => "The specified version does not exist",
            Self::NoSuchTagSet => "The TagSet does not exist",
            Self::NoSuchWebsiteConfiguration => "The website configuration does not exist",
            Self::NotImplemented => "The functionality is not implemented",
            Self::ObjectNotInActiveTierError => {
                "The source object of the COPY operation is not in the active tier"
            }
            Self::ObjectLockConfigurationNotFoundError => {
                "Object Lock configuration does not exist for this bucket"
            }
            Self::PreconditionFailed => {
                "At least one of the preconditions you specified did not hold"
            }
            Self::SignatureDoesNotMatch => "The request signature does not match",
            Self::TooManyBuckets => "You have attempted to create more buckets than allowed",
            Self::XAmzContentSHA256Mismatch => {
                "The provided x-amz-content-sha256 header does not match"
            }
            Self::BadDigest => "The Content-MD5 you specified did not match what we received",
            Self::ConditionalRequestConflict => "The conditional request cannot be processed",
            Self::MaxMessageLengthExceeded => "Your request was too big",
            Self::NoSuchObjectLockConfiguration => {
                "Object Lock configuration does not exist for this bucket"
            }
            Self::NoSuchPublicAccessBlockConfiguration => {
                "The public access block configuration was not found"
            }
            Self::NotModified => "Not Modified",
            Self::OwnershipControlsNotFoundError => "The bucket ownership controls were not found",
            Self::ReplicationConfigurationNotFoundError => {
                "The replication configuration was not found"
            }
            Self::ServerSideEncryptionConfigurationNotFoundError => {
                "The server-side encryption configuration was not found"
            }
            Self::Custom(s) => s,
        }
    }
}

impl fmt::Display for S3ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An S3 error response.
#[derive(Debug)]
pub struct S3Error {
    /// The error code.
    pub code: S3ErrorCode,
    /// A human-readable error message.
    pub message: String,
    /// The resource that caused the error.
    pub resource: Option<String>,
    /// The request ID.
    pub request_id: Option<String>,
    /// The HTTP status code.
    pub status_code: http::StatusCode,
    /// The underlying source error, if any.
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
    /// Extra HTTP headers to include in the error response.
    ///
    /// Boxed to minimize the size of `S3Error` on the common (header-free) path.
    pub headers: Option<Box<Vec<(String, String)>>>,
}

impl fmt::Display for S3Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "S3Error({}): {}", self.code, self.message)
    }
}

impl std::error::Error for S3Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e.as_ref() as &(dyn std::error::Error + 'static))
    }
}

impl S3Error {
    /// Create a new S3Error from an error code.
    #[must_use]
    pub fn new(code: S3ErrorCode) -> Self {
        let status_code = code.default_status_code();
        let message = code.default_message().to_owned();
        Self {
            code,
            message,
            resource: None,
            request_id: None,
            status_code,
            source: None,
            headers: None,
        }
    }

    /// Create a new S3Error with a custom message.
    #[must_use]
    pub fn with_message(code: S3ErrorCode, message: impl Into<String>) -> Self {
        Self {
            status_code: code.default_status_code(),
            message: message.into(),
            code,
            resource: None,
            request_id: None,
            source: None,
            headers: None,
        }
    }

    /// Set the resource that caused this error.
    #[must_use]
    pub fn with_resource(mut self, resource: impl Into<String>) -> Self {
        self.resource = Some(resource.into());
        self
    }

    /// Set the request ID.
    #[must_use]
    pub fn with_request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }

    /// Set the source error.
    #[must_use]
    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    /// Add an extra HTTP header to the error response.
    #[must_use]
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers
            .get_or_insert_with(|| Box::new(Vec::new()))
            .push((name.into(), value.into()));
        self
    }

    /// Create a NoSuchBucket error.
    #[must_use]
    pub fn no_such_bucket(bucket_name: impl Into<String>) -> Self {
        Self::new(S3ErrorCode::NoSuchBucket).with_resource(bucket_name)
    }

    /// Create a NoSuchKey error.
    #[must_use]
    pub fn no_such_key(key: impl Into<String>) -> Self {
        Self::new(S3ErrorCode::NoSuchKey).with_resource(key)
    }

    /// Create a NoSuchUpload error.
    #[must_use]
    pub fn no_such_upload(upload_id: impl Into<String>) -> Self {
        Self::new(S3ErrorCode::NoSuchUpload).with_resource(upload_id)
    }

    /// Create a NoSuchVersion error.
    #[must_use]
    pub fn no_such_version(version_id: impl Into<String>) -> Self {
        Self::new(S3ErrorCode::NoSuchVersion).with_resource(version_id)
    }

    /// Create a BucketAlreadyExists error.
    #[must_use]
    pub fn bucket_already_exists(bucket_name: impl Into<String>) -> Self {
        Self::new(S3ErrorCode::BucketAlreadyExists).with_resource(bucket_name)
    }

    /// Create a BucketAlreadyOwnedByYou error.
    #[must_use]
    pub fn bucket_already_owned_by_you(bucket_name: impl Into<String>) -> Self {
        Self::new(S3ErrorCode::BucketAlreadyOwnedByYou).with_resource(bucket_name)
    }

    /// Create a BucketNotEmpty error.
    #[must_use]
    pub fn bucket_not_empty(bucket_name: impl Into<String>) -> Self {
        Self::new(S3ErrorCode::BucketNotEmpty).with_resource(bucket_name)
    }

    /// Create a AccessDenied error.
    #[must_use]
    pub fn access_denied(resource: impl Into<String>) -> Self {
        Self::new(S3ErrorCode::AccessDenied).with_resource(resource)
    }

    /// Create a InternalError error.
    #[must_use]
    pub fn internal_error(message: impl Into<String>) -> Self {
        Self::with_message(S3ErrorCode::InternalError, message)
    }

    /// Create a InvalidArgument error.
    #[must_use]
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::with_message(S3ErrorCode::InvalidArgument, message)
    }

    /// Create a InvalidBucketName error.
    #[must_use]
    pub fn invalid_bucket_name(bucket_name: impl Into<String>) -> Self {
        Self::new(S3ErrorCode::InvalidBucketName).with_resource(bucket_name)
    }

    /// Create a InvalidRange error.
    #[must_use]
    pub fn invalid_range(range: impl Into<String>) -> Self {
        Self::new(S3ErrorCode::InvalidRange).with_resource(range)
    }

    /// Create a InvalidPart error.
    #[must_use]
    pub fn invalid_part(part_info: impl Into<String>) -> Self {
        Self::new(S3ErrorCode::InvalidPart).with_resource(part_info)
    }

    /// Create a InvalidPartOrder error.
    #[must_use]
    pub fn invalid_part_order(detail: impl Into<String>) -> Self {
        Self::new(S3ErrorCode::InvalidPartOrder).with_resource(detail)
    }

    /// Create a MalformedXML error.
    #[must_use]
    pub fn malformed_xml(detail: impl Into<String>) -> Self {
        Self::new(S3ErrorCode::MalformedXML).with_resource(detail)
    }

    /// Create a MethodNotAllowed error.
    #[must_use]
    pub fn method_not_allowed(method: impl Into<String>) -> Self {
        Self::new(S3ErrorCode::MethodNotAllowed).with_resource(method)
    }

    /// Create a NotImplemented error.
    #[must_use]
    pub fn not_implemented(detail: impl Into<String>) -> Self {
        Self::new(S3ErrorCode::NotImplemented).with_resource(detail)
    }

    /// Create a PreconditionFailed error.
    #[must_use]
    pub fn precondition_failed(condition: impl Into<String>) -> Self {
        Self::new(S3ErrorCode::PreconditionFailed).with_resource(condition)
    }

    /// Create a SignatureDoesNotMatch error.
    #[must_use]
    pub fn signature_does_not_match(detail: impl Into<String>) -> Self {
        Self::new(S3ErrorCode::SignatureDoesNotMatch).with_resource(detail)
    }
}

/// Create an S3Error from an error code.
///
/// # Examples
///
/// ```
/// use rustack_s3_model::s3_error;
/// use rustack_s3_model::error::S3ErrorCode;
///
/// let err = s3_error!(NoSuchBucket);
/// assert_eq!(err.code, S3ErrorCode::NoSuchBucket);
///
/// let err = s3_error!(NoSuchKey, "The key does not exist");
/// assert_eq!(err.message, "The key does not exist");
/// ```
#[macro_export]
macro_rules! s3_error {
    ($code:ident) => {
        $crate::error::S3Error::new($crate::error::S3ErrorCode::$code)
    };
    ($code:ident, $msg:expr) => {
        $crate::error::S3Error::with_message($crate::error::S3ErrorCode::$code, $msg)
    };
}
