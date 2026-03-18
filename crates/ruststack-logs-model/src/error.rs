//! Auto-generated from AWS CloudWatch Logs Smithy model. DO NOT EDIT.
//!
//! CloudWatch Logs errors use JSON format with a `__type` field containing the
//! short error type name (e.g., `ResourceNotFoundException`).

use std::fmt;

/// Well-known CloudWatch Logs error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum LogsErrorCode {
    /// DataAlreadyAcceptedException error.
    DataAlreadyAcceptedException,
    /// InvalidAction error.
    InvalidAction,
    /// InvalidOperationException error.
    InvalidOperationException,
    /// InvalidParameterException error.
    InvalidParameterException,
    /// InvalidSequenceTokenException error.
    InvalidSequenceTokenException,
    /// LimitExceededException error.
    LimitExceededException,
    /// MalformedQueryException error.
    MalformedQueryException,
    /// MissingAction error.
    MissingAction,
    /// OperationAbortedException error.
    OperationAbortedException,
    /// ResourceAlreadyExistsException error.
    ResourceAlreadyExistsException,
    /// ResourceNotFoundException error.
    ResourceNotFoundException,
    /// ServiceUnavailableException error.
    ServiceUnavailableException,
    /// TooManyTagsException error.
    TooManyTagsException,
    /// UnrecognizedClientException error.
    UnrecognizedClientException,
    /// ValidationException error.
    #[default]
    ValidationException,
}

impl LogsErrorCode {
    /// Returns the short error type string for the JSON `__type` field.
    #[must_use]
    pub fn error_type(&self) -> &'static str {
        self.as_str()
    }

    /// Returns the short error code string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DataAlreadyAcceptedException => "DataAlreadyAcceptedException",
            Self::InvalidAction => "InvalidAction",
            Self::InvalidOperationException => "InvalidOperationException",
            Self::InvalidParameterException => "InvalidParameterException",
            Self::InvalidSequenceTokenException => "InvalidSequenceTokenException",
            Self::LimitExceededException => "LimitExceededException",
            Self::MalformedQueryException => "MalformedQueryException",
            Self::MissingAction => "MissingAction",
            Self::OperationAbortedException => "OperationAbortedException",
            Self::ResourceAlreadyExistsException => "ResourceAlreadyExistsException",
            Self::ResourceNotFoundException => "ResourceNotFoundException",
            Self::ServiceUnavailableException => "ServiceUnavailableException",
            Self::TooManyTagsException => "TooManyTagsException",
            Self::UnrecognizedClientException => "UnrecognizedClientException",
            Self::ValidationException => "ValidationException",
        }
    }

    /// Returns the default HTTP status code for this error.
    #[must_use]
    pub fn default_status_code(&self) -> http::StatusCode {
        match self {
            Self::DataAlreadyAcceptedException
            | Self::InvalidAction
            | Self::InvalidOperationException
            | Self::InvalidParameterException
            | Self::InvalidSequenceTokenException
            | Self::LimitExceededException
            | Self::MalformedQueryException
            | Self::MissingAction
            | Self::OperationAbortedException
            | Self::ResourceAlreadyExistsException
            | Self::ResourceNotFoundException
            | Self::TooManyTagsException
            | Self::UnrecognizedClientException
            | Self::ValidationException => http::StatusCode::BAD_REQUEST,
            Self::ServiceUnavailableException => http::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl fmt::Display for LogsErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An CloudWatch Logs error response.
#[derive(Debug)]
pub struct LogsError {
    /// The error code.
    pub code: LogsErrorCode,
    /// A human-readable error message.
    pub message: String,
    /// The HTTP status code.
    pub status_code: http::StatusCode,
    /// The underlying source error, if any.
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl fmt::Display for LogsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LogsError({}): {}", self.code, self.message)
    }
}

impl std::error::Error for LogsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e.as_ref() as &(dyn std::error::Error + 'static))
    }
}

impl LogsError {
    /// Create a new `LogsError` from an error code.
    #[must_use]
    pub fn new(code: LogsErrorCode) -> Self {
        Self {
            status_code: code.default_status_code(),
            message: code.as_str().to_owned(),
            code,
            source: None,
        }
    }

    /// Create a new `LogsError` with a custom message.
    #[must_use]
    pub fn with_message(code: LogsErrorCode, message: impl Into<String>) -> Self {
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
        Self::with_message(LogsErrorCode::ValidationException, message)
    }

    /// Internal error.
    #[must_use]
    pub fn internal_error(message: impl Into<String>) -> Self {
        Self::with_message(LogsErrorCode::ServiceUnavailableException, message)
    }

    /// Missing action header.
    #[must_use]
    pub fn missing_action() -> Self {
        Self::with_message(
            LogsErrorCode::MissingAction,
            "Missing required header: X-Amz-Target",
        )
    }

    /// Unknown operation.
    #[must_use]
    pub fn unknown_operation(target: &str) -> Self {
        Self::with_message(
            LogsErrorCode::InvalidAction,
            format!("Operation {target} is not supported."),
        )
    }

    /// Not implemented.
    #[must_use]
    pub fn not_implemented(operation: &str) -> Self {
        Self::with_message(
            LogsErrorCode::ServiceUnavailableException,
            format!("Operation {operation} is not yet implemented"),
        )
    }
}

/// Create an `LogsError` from an error code.
///
/// # Examples
///
/// ```ignore
/// let err = logs_error!(ValidationException);
/// assert_eq!(err.code, LogsErrorCode::ValidationException);
/// ```
#[macro_export]
macro_rules! logs_error {
    ($code:ident) => {
        $crate::error::LogsError::new($crate::error::LogsErrorCode::$code)
    };
    ($code:ident, $msg:expr) => {
        $crate::error::LogsError::with_message($crate::error::LogsErrorCode::$code, $msg)
    };
}
