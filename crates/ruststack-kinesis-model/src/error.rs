//! Auto-generated from AWS Kinesis Smithy model. DO NOT EDIT.
//!
//! Kinesis errors use JSON format with a `__type` field containing the
//! short error type name (e.g., `ResourceNotFoundException`).

use std::fmt;

/// Well-known Kinesis error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum KinesisErrorCode {
    /// AccessDeniedException error.
    AccessDeniedException,
    /// ExpiredIteratorException error.
    ExpiredIteratorException,
    /// ExpiredNextTokenException error.
    ExpiredNextTokenException,
    /// InternalFailureException error.
    InternalFailureException,
    /// InvalidAction error.
    InvalidAction,
    /// InvalidArgumentException error.
    InvalidArgumentException,
    /// KMSAccessDeniedException error.
    KMSAccessDeniedException,
    /// KMSDisabledException error.
    KMSDisabledException,
    /// KMSInvalidStateException error.
    KMSInvalidStateException,
    /// KMSNotFoundException error.
    KMSNotFoundException,
    /// KMSOptInRequired error.
    KMSOptInRequired,
    /// KMSThrottlingException error.
    KMSThrottlingException,
    /// LimitExceededException error.
    LimitExceededException,
    /// MissingAction error.
    MissingAction,
    /// ProvisionedThroughputExceededException error.
    ProvisionedThroughputExceededException,
    /// ResourceInUseException error.
    ResourceInUseException,
    /// ResourceNotFoundException error.
    ResourceNotFoundException,
    /// ValidationException error.
    #[default]
    ValidationException,
}

impl KinesisErrorCode {
    /// Returns the short error type string for the JSON `__type` field.
    #[must_use]
    pub fn error_type(&self) -> &'static str {
        self.as_str()
    }

    /// Returns the short error code string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AccessDeniedException => "AccessDeniedException",
            Self::ExpiredIteratorException => "ExpiredIteratorException",
            Self::ExpiredNextTokenException => "ExpiredNextTokenException",
            Self::InternalFailureException => "InternalFailureException",
            Self::InvalidAction => "InvalidAction",
            Self::InvalidArgumentException => "InvalidArgumentException",
            Self::KMSAccessDeniedException => "KMSAccessDeniedException",
            Self::KMSDisabledException => "KMSDisabledException",
            Self::KMSInvalidStateException => "KMSInvalidStateException",
            Self::KMSNotFoundException => "KMSNotFoundException",
            Self::KMSOptInRequired => "KMSOptInRequired",
            Self::KMSThrottlingException => "KMSThrottlingException",
            Self::LimitExceededException => "LimitExceededException",
            Self::MissingAction => "MissingAction",
            Self::ProvisionedThroughputExceededException => {
                "ProvisionedThroughputExceededException"
            }
            Self::ResourceInUseException => "ResourceInUseException",
            Self::ResourceNotFoundException => "ResourceNotFoundException",
            Self::ValidationException => "ValidationException",
        }
    }

    /// Returns the default HTTP status code for this error.
    #[must_use]
    pub fn default_status_code(&self) -> http::StatusCode {
        match self {
            Self::AccessDeniedException
            | Self::ExpiredIteratorException
            | Self::ExpiredNextTokenException
            | Self::InvalidAction
            | Self::InvalidArgumentException
            | Self::KMSAccessDeniedException
            | Self::KMSDisabledException
            | Self::KMSInvalidStateException
            | Self::KMSNotFoundException
            | Self::KMSOptInRequired
            | Self::KMSThrottlingException
            | Self::LimitExceededException
            | Self::MissingAction
            | Self::ProvisionedThroughputExceededException
            | Self::ResourceInUseException
            | Self::ResourceNotFoundException
            | Self::ValidationException => http::StatusCode::BAD_REQUEST,
            Self::InternalFailureException => http::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl fmt::Display for KinesisErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An Kinesis error response.
#[derive(Debug)]
pub struct KinesisError {
    /// The error code.
    pub code: KinesisErrorCode,
    /// A human-readable error message.
    pub message: String,
    /// The HTTP status code.
    pub status_code: http::StatusCode,
    /// The underlying source error, if any.
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl fmt::Display for KinesisError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "KinesisError({}): {}", self.code, self.message)
    }
}

impl std::error::Error for KinesisError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e.as_ref() as &(dyn std::error::Error + 'static))
    }
}

impl KinesisError {
    /// Create a new `KinesisError` from an error code.
    #[must_use]
    pub fn new(code: KinesisErrorCode) -> Self {
        Self {
            status_code: code.default_status_code(),
            message: code.as_str().to_owned(),
            code,
            source: None,
        }
    }

    /// Create a new `KinesisError` with a custom message.
    #[must_use]
    pub fn with_message(code: KinesisErrorCode, message: impl Into<String>) -> Self {
        Self {
            status_code: code.default_status_code(),
            message: message.into(),
            code,
            source: None,
        }
    }

    /// Returns the `__type` string for the JSON error response.
    #[must_use]
    pub fn error_type(&self) -> &'static str {
        self.code.error_type()
    }

    /// Validation error.
    #[must_use]
    pub fn validation(message: impl Into<String>) -> Self {
        Self::with_message(KinesisErrorCode::ValidationException, message)
    }

    /// Internal error.
    #[must_use]
    pub fn internal_error(message: impl Into<String>) -> Self {
        Self::with_message(KinesisErrorCode::InternalFailureException, message)
    }

    /// Missing action header.
    #[must_use]
    pub fn missing_action() -> Self {
        Self::with_message(
            KinesisErrorCode::MissingAction,
            "Missing required header: X-Amz-Target",
        )
    }

    /// Unknown operation.
    #[must_use]
    pub fn unknown_operation(target: &str) -> Self {
        Self::with_message(
            KinesisErrorCode::InvalidAction,
            format!("Operation {target} is not supported."),
        )
    }

    /// Not implemented.
    #[must_use]
    pub fn not_implemented(operation: &str) -> Self {
        Self::with_message(
            KinesisErrorCode::InternalFailureException,
            format!("Operation {operation} is not yet implemented"),
        )
    }
}

/// Create an `KinesisError` from an error code.
///
/// # Examples
///
/// ```ignore
/// let err = kinesis_error!(ValidationException);
/// assert_eq!(err.code, KinesisErrorCode::ValidationException);
/// ```
#[macro_export]
macro_rules! kinesis_error {
    ($code:ident) => {
        $crate::error::KinesisError::new($crate::error::KinesisErrorCode::$code)
    };
    ($code:ident, $msg:expr) => {
        $crate::error::KinesisError::with_message($crate::error::KinesisErrorCode::$code, $msg)
    };
}
