//! Lambda error types.

use std::error::Error;
use std::fmt;

/// Lambda error codes matching AWS Lambda API error types.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LambdaErrorCode {
    /// Invalid parameter value.
    InvalidParameterValueException,
    /// Invalid request content.
    InvalidRequestContentException,
    /// Resource not found (function, version, alias).
    ResourceNotFoundException,
    /// Resource already exists.
    ResourceConflictException,
    /// Request payload too large.
    RequestTooLargeException,
    /// Invalid runtime.
    InvalidRuntimeException,
    /// Resource not ready.
    ResourceNotReadyException,
    /// Too many requests.
    TooManyRequestsException,
    /// Internal server error.
    ServiceException,
    /// Unknown operation.
    UnknownOperation,
}

impl LambdaErrorCode {
    /// Returns the error type string for the `X-Amzn-Errortype` header.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::InvalidParameterValueException => "InvalidParameterValueException",
            Self::InvalidRequestContentException => "InvalidRequestContentException",
            Self::ResourceNotFoundException => "ResourceNotFoundException",
            Self::ResourceConflictException => "ResourceConflictException",
            Self::RequestTooLargeException => "RequestTooLargeException",
            Self::InvalidRuntimeException => "InvalidRuntimeException",
            Self::ResourceNotReadyException => "ResourceNotReadyException",
            Self::TooManyRequestsException => "TooManyRequestsException",
            Self::ServiceException | Self::UnknownOperation => "ServiceException",
        }
    }

    /// Returns the HTTP status code for this error.
    #[must_use]
    pub fn status_code(&self) -> http::StatusCode {
        match self {
            Self::InvalidParameterValueException | Self::InvalidRequestContentException => {
                http::StatusCode::BAD_REQUEST
            }
            Self::ResourceNotFoundException => http::StatusCode::NOT_FOUND,
            Self::ResourceConflictException => http::StatusCode::CONFLICT,
            Self::RequestTooLargeException => http::StatusCode::PAYLOAD_TOO_LARGE,
            Self::InvalidRuntimeException | Self::ResourceNotReadyException => {
                http::StatusCode::BAD_GATEWAY
            }
            Self::TooManyRequestsException => http::StatusCode::TOO_MANY_REQUESTS,
            Self::ServiceException | Self::UnknownOperation => {
                http::StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }

    /// Returns the error type category for the response body.
    #[must_use]
    pub fn error_type(&self) -> &'static str {
        if self.status_code().as_u16() >= 500 {
            "Service"
        } else {
            "User"
        }
    }
}

/// Lambda error with error code, message, and HTTP status.
#[derive(Debug)]
pub struct LambdaError {
    /// Error classification.
    pub code: LambdaErrorCode,
    /// Human-readable error message.
    pub message: String,
    /// Optional source error.
    source: Option<Box<dyn Error + Send + Sync>>,
}

impl LambdaError {
    /// Create a new Lambda error.
    #[must_use]
    pub fn new(code: LambdaErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            source: None,
        }
    }

    /// Attach a source error.
    #[must_use]
    pub fn with_source(mut self, source: impl Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    /// Resource not found error.
    #[must_use]
    pub fn resource_not_found(message: impl Into<String>) -> Self {
        Self::new(LambdaErrorCode::ResourceNotFoundException, message)
    }

    /// Resource conflict error.
    #[must_use]
    pub fn resource_conflict(message: impl Into<String>) -> Self {
        Self::new(LambdaErrorCode::ResourceConflictException, message)
    }

    /// Invalid parameter error.
    #[must_use]
    pub fn invalid_parameter(message: impl Into<String>) -> Self {
        Self::new(LambdaErrorCode::InvalidParameterValueException, message)
    }

    /// Internal service error.
    #[must_use]
    pub fn service_error(message: impl Into<String>) -> Self {
        Self::new(LambdaErrorCode::ServiceException, message)
    }

    /// Unknown operation error.
    #[must_use]
    pub fn unknown_operation(method: &http::Method, path: &str) -> Self {
        Self::new(
            LambdaErrorCode::UnknownOperation,
            format!("no Lambda operation matches {method} {path}"),
        )
    }
}

impl fmt::Display for LambdaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl Error for LambdaError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e.as_ref() as &(dyn Error + 'static))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_map_error_codes_to_status() {
        assert_eq!(
            LambdaErrorCode::ResourceNotFoundException.status_code(),
            http::StatusCode::NOT_FOUND
        );
        assert_eq!(
            LambdaErrorCode::ResourceConflictException.status_code(),
            http::StatusCode::CONFLICT
        );
        assert_eq!(
            LambdaErrorCode::ServiceException.status_code(),
            http::StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn test_should_classify_error_types() {
        assert_eq!(
            LambdaErrorCode::ResourceNotFoundException.error_type(),
            "User"
        );
        assert_eq!(LambdaErrorCode::ServiceException.error_type(), "Service");
    }
}
