//! Auto-generated from AWS DynamoDB Streams Smithy model. DO NOT EDIT.
//!
//! DynamoDB Streams errors use JSON format with a `__type` field containing the
//! short error type name (e.g., `ResourceNotFoundException`).

use std::fmt;

/// Well-known DynamoDB Streams error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum DynamoDBStreamsErrorCode {
    /// ExpiredIteratorException error.
    #[default]
    ExpiredIteratorException,
    /// InternalServerError error.
    InternalServerError,
    /// InvalidAction error.
    InvalidAction,
    /// LimitExceededException error.
    LimitExceededException,
    /// MissingAction error.
    MissingAction,
    /// ResourceNotFoundException error.
    ResourceNotFoundException,
    /// TrimmedDataAccessException error.
    TrimmedDataAccessException,
    /// ValidationException error.
    ValidationException,
}

impl DynamoDBStreamsErrorCode {
    /// Returns the short error type string for the JSON `__type` field.
    #[must_use]
    pub fn error_type(&self) -> &'static str {
        self.as_str()
    }

    /// Returns the short error code string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ExpiredIteratorException => "ExpiredIteratorException",
            Self::InternalServerError => "InternalServerError",
            Self::InvalidAction => "InvalidAction",
            Self::LimitExceededException => "LimitExceededException",
            Self::MissingAction => "MissingAction",
            Self::ResourceNotFoundException => "ResourceNotFoundException",
            Self::TrimmedDataAccessException => "TrimmedDataAccessException",
            Self::ValidationException => "ValidationException",
        }
    }

    /// Returns the default HTTP status code for this error.
    #[must_use]
    pub fn default_status_code(&self) -> http::StatusCode {
        match self {
            Self::ExpiredIteratorException
            | Self::InvalidAction
            | Self::LimitExceededException
            | Self::MissingAction
            | Self::ResourceNotFoundException
            | Self::TrimmedDataAccessException
            | Self::ValidationException => http::StatusCode::BAD_REQUEST,
            Self::InternalServerError => http::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl fmt::Display for DynamoDBStreamsErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An DynamoDB Streams error response.
#[derive(Debug)]
pub struct DynamoDBStreamsError {
    /// The error code.
    pub code: DynamoDBStreamsErrorCode,
    /// A human-readable error message.
    pub message: String,
    /// The HTTP status code.
    pub status_code: http::StatusCode,
    /// The underlying source error, if any.
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl fmt::Display for DynamoDBStreamsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DynamoDBStreamsError({}): {}", self.code, self.message)
    }
}

impl std::error::Error for DynamoDBStreamsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e.as_ref() as &(dyn std::error::Error + 'static))
    }
}

impl DynamoDBStreamsError {
    /// Create a new `DynamoDBStreamsError` from an error code.
    #[must_use]
    pub fn new(code: DynamoDBStreamsErrorCode) -> Self {
        Self {
            status_code: code.default_status_code(),
            message: code.as_str().to_owned(),
            code,
            source: None,
        }
    }

    /// Create a new `DynamoDBStreamsError` with a custom message.
    #[must_use]
    pub fn with_message(code: DynamoDBStreamsErrorCode, message: impl Into<String>) -> Self {
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

    /// Internal error.
    #[must_use]
    pub fn internal_error(message: impl Into<String>) -> Self {
        Self::with_message(DynamoDBStreamsErrorCode::InternalServerError, message)
    }

    /// Missing action header.
    #[must_use]
    pub fn missing_action() -> Self {
        Self::with_message(
            DynamoDBStreamsErrorCode::MissingAction,
            "Missing required header: X-Amz-Target",
        )
    }

    /// Unknown operation.
    #[must_use]
    pub fn unknown_operation(target: &str) -> Self {
        Self::with_message(
            DynamoDBStreamsErrorCode::InvalidAction,
            format!("Operation {target} is not supported."),
        )
    }

    /// Not implemented.
    #[must_use]
    pub fn not_implemented(operation: &str) -> Self {
        Self::with_message(
            DynamoDBStreamsErrorCode::InternalServerError,
            format!("Operation {operation} is not yet implemented"),
        )
    }

    /// Resource not found.
    #[must_use]
    pub fn resource_not_found(message: impl Into<String>) -> Self {
        Self::with_message(DynamoDBStreamsErrorCode::ResourceNotFoundException, message)
    }

    /// Validation error.
    #[must_use]
    pub fn validation(message: impl Into<String>) -> Self {
        Self::with_message(DynamoDBStreamsErrorCode::ValidationException, message)
    }

    /// Expired iterator.
    #[must_use]
    pub fn expired_iterator(message: impl Into<String>) -> Self {
        Self::with_message(DynamoDBStreamsErrorCode::ExpiredIteratorException, message)
    }

    /// Trimmed data access.
    #[must_use]
    pub fn trimmed_data_access(message: impl Into<String>) -> Self {
        Self::with_message(
            DynamoDBStreamsErrorCode::TrimmedDataAccessException,
            message,
        )
    }

    /// Serialization error.
    #[must_use]
    pub fn serialization_exception(message: impl Into<String>) -> Self {
        Self::with_message(DynamoDBStreamsErrorCode::ValidationException, message)
    }
}

/// Create an `DynamoDBStreamsError` from an error code.
///
/// # Examples
///
/// ```ignore
/// let err = dynamodbstreams_error!(ExpiredIteratorException);
/// assert_eq!(err.code, DynamoDBStreamsErrorCode::ExpiredIteratorException);
/// ```
#[macro_export]
macro_rules! dynamodbstreams_error {
    ($code:ident) => {
        $crate::error::DynamoDBStreamsError::new($crate::error::DynamoDBStreamsErrorCode::$code)
    };
    ($code:ident, $msg:expr) => {
        $crate::error::DynamoDBStreamsError::with_message(
            $crate::error::DynamoDBStreamsErrorCode::$code,
            $msg,
        )
    };
}
