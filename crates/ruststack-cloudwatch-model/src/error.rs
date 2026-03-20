//! Auto-generated from AWS CloudWatch Smithy model. DO NOT EDIT.
//!
//! CloudWatch errors use JSON format with a `__type` field containing the
//! short error type name (e.g., `ResourceNotFoundException`).

use std::fmt;

/// Well-known CloudWatch error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum CloudWatchErrorCode {
    /// ConcurrentModificationException error.
    #[default]
    ConcurrentModificationException,
    /// ConflictException error.
    ConflictException,
    /// DashboardInvalidInputError error.
    DashboardInvalidInputError,
    /// DashboardNotFoundError error.
    DashboardNotFoundError,
    /// InternalServiceFault error.
    InternalServiceFault,
    /// InvalidAction error.
    InvalidAction,
    /// InvalidFormatFault error.
    InvalidFormatFault,
    /// InvalidNextToken error.
    InvalidNextToken,
    /// InvalidParameterCombinationException error.
    InvalidParameterCombinationException,
    /// InvalidParameterValueException error.
    InvalidParameterValueException,
    /// LimitExceededException error.
    LimitExceededException,
    /// LimitExceededFault error.
    LimitExceededFault,
    /// MissingAction error.
    MissingAction,
    /// MissingRequiredParameterException error.
    MissingRequiredParameterException,
    /// ResourceNotFound error.
    ResourceNotFound,
    /// ResourceNotFoundException error.
    ResourceNotFoundException,
}

impl CloudWatchErrorCode {
    /// Returns the short error type string for the JSON `__type` field.
    #[must_use]
    pub fn error_type(&self) -> &'static str {
        self.as_str()
    }

    /// Returns the short error code string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ConcurrentModificationException => "ConcurrentModificationException",
            Self::ConflictException => "ConflictException",
            Self::DashboardInvalidInputError => "DashboardInvalidInputError",
            Self::DashboardNotFoundError => "DashboardNotFoundError",
            Self::InternalServiceFault => "InternalServiceFault",
            Self::InvalidAction => "InvalidAction",
            Self::InvalidFormatFault => "InvalidFormatFault",
            Self::InvalidNextToken => "InvalidNextToken",
            Self::InvalidParameterCombinationException => "InvalidParameterCombinationException",
            Self::InvalidParameterValueException => "InvalidParameterValueException",
            Self::LimitExceededException => "LimitExceededException",
            Self::LimitExceededFault => "LimitExceededFault",
            Self::MissingAction => "MissingAction",
            Self::MissingRequiredParameterException => "MissingRequiredParameterException",
            Self::ResourceNotFound => "ResourceNotFound",
            Self::ResourceNotFoundException => "ResourceNotFoundException",
        }
    }

    /// Returns the default HTTP status code for this error.
    #[must_use]
    pub fn default_status_code(&self) -> http::StatusCode {
        match self {
            Self::DashboardInvalidInputError
            | Self::InvalidAction
            | Self::InvalidFormatFault
            | Self::InvalidNextToken
            | Self::InvalidParameterCombinationException
            | Self::InvalidParameterValueException
            | Self::LimitExceededException
            | Self::LimitExceededFault
            | Self::MissingAction
            | Self::MissingRequiredParameterException => http::StatusCode::BAD_REQUEST,
            Self::DashboardNotFoundError
            | Self::ResourceNotFound
            | Self::ResourceNotFoundException => http::StatusCode::NOT_FOUND,
            Self::ConflictException => http::StatusCode::CONFLICT,
            Self::ConcurrentModificationException | Self::InternalServiceFault => {
                http::StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }
}

impl fmt::Display for CloudWatchErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An CloudWatch error response.
#[derive(Debug)]
pub struct CloudWatchError {
    /// The error code.
    pub code: CloudWatchErrorCode,
    /// A human-readable error message.
    pub message: String,
    /// The HTTP status code.
    pub status_code: http::StatusCode,
    /// The underlying source error, if any.
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl fmt::Display for CloudWatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CloudWatchError({}): {}", self.code, self.message)
    }
}

impl std::error::Error for CloudWatchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e.as_ref() as &(dyn std::error::Error + 'static))
    }
}

impl CloudWatchError {
    /// Create a new `CloudWatchError` from an error code.
    #[must_use]
    pub fn new(code: CloudWatchErrorCode) -> Self {
        Self {
            status_code: code.default_status_code(),
            message: code.as_str().to_owned(),
            code,
            source: None,
        }
    }

    /// Create a new `CloudWatchError` with a custom message.
    #[must_use]
    pub fn with_message(code: CloudWatchErrorCode, message: impl Into<String>) -> Self {
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
        Self::with_message(CloudWatchErrorCode::InternalServiceFault, message)
    }

    /// Missing action header.
    #[must_use]
    pub fn missing_action() -> Self {
        Self::with_message(
            CloudWatchErrorCode::MissingAction,
            "Missing required header: X-Amz-Target",
        )
    }

    /// Unknown operation.
    #[must_use]
    pub fn unknown_operation(target: &str) -> Self {
        Self::with_message(
            CloudWatchErrorCode::InvalidAction,
            format!("Operation {target} is not supported."),
        )
    }

    /// Not implemented.
    #[must_use]
    pub fn not_implemented(operation: &str) -> Self {
        Self::with_message(
            CloudWatchErrorCode::InternalServiceFault,
            format!("Operation {operation} is not yet implemented"),
        )
    }
}

/// Create an `CloudWatchError` from an error code.
///
/// # Examples
///
/// ```ignore
/// let err = cloudwatch_error!(ConcurrentModificationException);
/// assert_eq!(err.code, CloudWatchErrorCode::ConcurrentModificationException);
/// ```
#[macro_export]
macro_rules! cloudwatch_error {
    ($code:ident) => {
        $crate::error::CloudWatchError::new($crate::error::CloudWatchErrorCode::$code)
    };
    ($code:ident, $msg:expr) => {
        $crate::error::CloudWatchError::with_message(
            $crate::error::CloudWatchErrorCode::$code,
            $msg,
        )
    };
}
