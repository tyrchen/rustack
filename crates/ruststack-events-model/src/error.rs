//! EventBridge error types.
//!
//! EventBridge errors use JSON format with a `__type` field containing the
//! short error type name (e.g., `ResourceNotFoundException`).

use std::fmt;

/// Well-known EventBridge error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum EventsErrorCode {
    /// Resource not found.
    ResourceNotFound,
    /// Resource already exists.
    ResourceAlreadyExists,
    /// Invalid event pattern.
    InvalidEventPattern,
    /// Validation error.
    #[default]
    ValidationException,
    /// Limit exceeded.
    LimitExceeded,
    /// Concurrent modification.
    ConcurrentModification,
    /// Internal error.
    InternalException,
    /// Invalid action (unrecognized operation).
    InvalidAction,
    /// Missing action header.
    MissingAction,
    /// Managed rule exception (cannot modify managed rules).
    ManagedRuleException,
    /// Operation not supported.
    OperationDisabled,
}

impl EventsErrorCode {
    /// Returns the short error type string for the JSON `__type` field.
    #[must_use]
    pub fn error_type(&self) -> &'static str {
        self.as_str()
    }

    /// Returns the short error code string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ResourceNotFound => "ResourceNotFoundException",
            Self::ResourceAlreadyExists => "ResourceAlreadyExistsException",
            Self::InvalidEventPattern => "InvalidEventPatternException",
            Self::ValidationException => "ValidationException",
            Self::LimitExceeded => "LimitExceededException",
            Self::ConcurrentModification => "ConcurrentModificationException",
            Self::InternalException => "InternalException",
            Self::InvalidAction => "InvalidAction",
            Self::MissingAction => "MissingAction",
            Self::ManagedRuleException => "ManagedRuleException",
            Self::OperationDisabled => "OperationDisabledException",
        }
    }

    /// Returns the default HTTP status code for this error.
    #[must_use]
    pub fn default_status_code(&self) -> http::StatusCode {
        match self {
            Self::InternalException => http::StatusCode::INTERNAL_SERVER_ERROR,
            _ => http::StatusCode::BAD_REQUEST,
        }
    }
}

impl fmt::Display for EventsErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An EventBridge error response.
#[derive(Debug)]
pub struct EventsError {
    /// The error code.
    pub code: EventsErrorCode,
    /// A human-readable error message.
    pub message: String,
    /// The HTTP status code.
    pub status_code: http::StatusCode,
    /// The underlying source error, if any.
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl fmt::Display for EventsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EventsError({}): {}", self.code, self.message)
    }
}

impl std::error::Error for EventsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e.as_ref() as &(dyn std::error::Error + 'static))
    }
}

impl EventsError {
    /// Create a new `EventsError` from an error code.
    #[must_use]
    pub fn new(code: EventsErrorCode) -> Self {
        Self {
            status_code: code.default_status_code(),
            message: code.as_str().to_owned(),
            code,
            source: None,
        }
    }

    /// Create a new `EventsError` with a custom message.
    #[must_use]
    pub fn with_message(code: EventsErrorCode, message: impl Into<String>) -> Self {
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

    /// Resource not found.
    #[must_use]
    pub fn resource_not_found(message: impl Into<String>) -> Self {
        Self::with_message(EventsErrorCode::ResourceNotFound, message)
    }

    /// Resource already exists.
    #[must_use]
    pub fn resource_already_exists(message: impl Into<String>) -> Self {
        Self::with_message(EventsErrorCode::ResourceAlreadyExists, message)
    }

    /// Invalid event pattern.
    #[must_use]
    pub fn invalid_event_pattern(message: impl Into<String>) -> Self {
        Self::with_message(EventsErrorCode::InvalidEventPattern, message)
    }

    /// Validation error.
    #[must_use]
    pub fn validation(message: impl Into<String>) -> Self {
        Self::with_message(EventsErrorCode::ValidationException, message)
    }

    /// Internal error.
    #[must_use]
    pub fn internal_error(message: impl Into<String>) -> Self {
        Self::with_message(EventsErrorCode::InternalException, message)
    }

    /// Missing action header.
    #[must_use]
    pub fn missing_action() -> Self {
        Self::with_message(
            EventsErrorCode::MissingAction,
            "Missing required header: X-Amz-Target",
        )
    }

    /// Unknown operation.
    #[must_use]
    pub fn unknown_operation(target: &str) -> Self {
        Self::with_message(
            EventsErrorCode::InvalidAction,
            format!("Operation {target} is not supported."),
        )
    }

    /// Not implemented.
    #[must_use]
    pub fn not_implemented(operation: &str) -> Self {
        Self::with_message(
            EventsErrorCode::InternalException,
            format!("Operation {operation} is not yet implemented"),
        )
    }

    /// Limit exceeded.
    #[must_use]
    pub fn limit_exceeded(message: impl Into<String>) -> Self {
        Self::with_message(EventsErrorCode::LimitExceeded, message)
    }
}

/// Create an `EventsError` from an error code.
///
/// # Examples
///
/// ```
/// use ruststack_events_model::events_error;
/// use ruststack_events_model::error::EventsErrorCode;
///
/// let err = events_error!(ValidationException);
/// assert_eq!(err.code, EventsErrorCode::ValidationException);
///
/// let err = events_error!(ResourceNotFound, "Event bus my-bus does not exist.");
/// assert_eq!(err.code, EventsErrorCode::ResourceNotFound);
/// ```
#[macro_export]
macro_rules! events_error {
    ($code:ident) => {
        $crate::error::EventsError::new($crate::error::EventsErrorCode::$code)
    };
    ($code:ident, $msg:expr) => {
        $crate::error::EventsError::with_message($crate::error::EventsErrorCode::$code, $msg)
    };
}
