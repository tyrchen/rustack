//! Auto-generated from AWS ApiGatewayV2 Smithy model. DO NOT EDIT.
//!
//! ApiGatewayV2 errors use JSON format with a `__type` field containing the
//! short error type name (e.g., `ResourceNotFoundException`).

use std::fmt;

/// Well-known ApiGatewayV2 error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ApiGatewayV2ErrorCode {
    /// AccessDeniedException error.
    #[default]
    AccessDeniedException,
    /// BadRequestException error.
    BadRequestException,
    /// ConflictException error.
    ConflictException,
    /// NotFoundException error.
    NotFoundException,
    /// TooManyRequestsException error.
    TooManyRequestsException,
    /// UnknownOperation error.
    UnknownOperation,
}

impl ApiGatewayV2ErrorCode {
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
            Self::BadRequestException => "BadRequestException",
            Self::ConflictException => "ConflictException",
            Self::NotFoundException => "NotFoundException",
            Self::TooManyRequestsException => "TooManyRequestsException",
            Self::UnknownOperation => "UnknownOperation",
        }
    }

    /// Returns the default HTTP status code for this error.
    #[must_use]
    pub fn default_status_code(&self) -> http::StatusCode {
        match self {
            Self::BadRequestException => http::StatusCode::BAD_REQUEST,
            Self::AccessDeniedException => http::StatusCode::FORBIDDEN,
            Self::NotFoundException => http::StatusCode::NOT_FOUND,
            Self::ConflictException => http::StatusCode::CONFLICT,
            Self::TooManyRequestsException | Self::UnknownOperation => {
                http::StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }
}

impl fmt::Display for ApiGatewayV2ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An ApiGatewayV2 error response.
#[derive(Debug)]
pub struct ApiGatewayV2Error {
    /// The error code.
    pub code: ApiGatewayV2ErrorCode,
    /// A human-readable error message.
    pub message: String,
    /// The HTTP status code.
    pub status_code: http::StatusCode,
    /// The underlying source error, if any.
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl fmt::Display for ApiGatewayV2Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ApiGatewayV2Error({}): {}", self.code, self.message)
    }
}

impl std::error::Error for ApiGatewayV2Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e.as_ref() as &(dyn std::error::Error + 'static))
    }
}

impl ApiGatewayV2Error {
    /// Create a new `ApiGatewayV2Error` from an error code.
    #[must_use]
    pub fn new(code: ApiGatewayV2ErrorCode) -> Self {
        Self {
            status_code: code.default_status_code(),
            message: code.as_str().to_owned(),
            code,
            source: None,
        }
    }

    /// Create a new `ApiGatewayV2Error` with a custom message.
    #[must_use]
    pub fn with_message(code: ApiGatewayV2ErrorCode, message: impl Into<String>) -> Self {
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
        Self::with_message(ApiGatewayV2ErrorCode::UnknownOperation, message)
    }

    /// Unknown operation error.
    #[must_use]
    pub fn unknown_operation(method: &http::Method, path: &str) -> Self {
        Self::with_message(
            ApiGatewayV2ErrorCode::UnknownOperation,
            format!("no ApiGatewayV2 operation matches {method} {path}"),
        )
    }

    /// Not implemented.
    #[must_use]
    pub fn not_implemented(operation: &str) -> Self {
        Self::with_message(
            ApiGatewayV2ErrorCode::UnknownOperation,
            format!("Operation {operation} is not yet implemented"),
        )
    }
}

/// Create an `ApiGatewayV2Error` from an error code.
///
/// # Examples
///
/// ```ignore
/// let err = apigatewayv2_error!(AccessDeniedException);
/// assert_eq!(err.code, ApiGatewayV2ErrorCode::AccessDeniedException);
/// ```
#[macro_export]
macro_rules! apigatewayv2_error {
    ($code:ident) => {
        $crate::error::ApiGatewayV2Error::new($crate::error::ApiGatewayV2ErrorCode::$code)
    };
    ($code:ident, $msg:expr) => {
        $crate::error::ApiGatewayV2Error::with_message(
            $crate::error::ApiGatewayV2ErrorCode::$code,
            $msg,
        )
    };
}
