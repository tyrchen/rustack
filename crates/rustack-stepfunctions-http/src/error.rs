//! Typed sources and static transport diagnostics, independent of user payloads.

use std::fmt;

use http::StatusCode;
use rustack_auth::AuthError;
use rustack_core::{http::BodyReadError, json::JsonError};
use rustack_stepfunctions_model::{StepFunctionsError, StepFunctionsErrorCode};

/// HTTP boundary failures; Debug/Display never expose underlying input values.
#[derive(thiserror::Error)]
#[non_exhaustive]
pub enum HttpError {
    /// Invalid trusted scope, authentication configuration or budget.
    #[error("invalid Step Functions HTTP configuration")]
    Configuration,
    /// Request method/path/headers are ambiguous or outside this protocol.
    #[error("invalid Step Functions HTTP envelope")]
    Envelope,
    /// No known action exactly matches the target.
    #[error("unknown Step Functions operation")]
    UnknownOperation,
    /// A modeled action is outside this transport profile.
    #[error("unsupported Step Functions operation")]
    UnsupportedOperation,
    /// Authentication failed; the typed source is deliberately not displayed.
    #[error("Step Functions authentication failed")]
    Authentication(#[source] AuthError),
    /// Source body bounds/IO failed without accepting partial bytes.
    #[error("Step Functions body collection failed")]
    Body(#[source] BodyReadError),
    /// Bounded JSON preflight/schema failed.
    #[error("invalid Step Functions request JSON")]
    Json(#[source] JsonError),
    /// An operation-specific input bound failed.
    #[error("invalid Step Functions request field")]
    Field(StepFunctionsErrorCode),
    /// The caller's absolute deadline has elapsed.
    #[error("Step Functions HTTP deadline exceeded")]
    Deadline,
    /// A typed handler rejected a request; diagnostics remain redacted.
    #[error("Step Functions handler rejected request")]
    Handler(#[source] StepFunctionsError),
    /// An invalid/mismatched or oversized handler reply cannot become success.
    #[error("invalid Step Functions handler response")]
    Response,
    /// Encoding failed; source may contain values and is never displayed.
    #[error("Step Functions response encoding failed")]
    Encoding(#[source] serde_json::Error),
}
impl fmt::Debug for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl HttpError {
    pub(crate) fn wire(&self) -> (&'static str, &str, StatusCode) {
        match self {
            Self::Configuration | Self::Response | Self::Encoding(_) => (
                "InternalFailure",
                "Request processing failed",
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            Self::Envelope => (
                "ValidationException",
                "Invalid HTTP request envelope",
                StatusCode::BAD_REQUEST,
            ),
            Self::UnknownOperation => (
                "UnknownOperationException",
                "Unknown Step Functions operation",
                StatusCode::NOT_FOUND,
            ),
            Self::UnsupportedOperation => (
                "UnsupportedOperation",
                "Operation is outside the supported Step Functions profile",
                StatusCode::BAD_REQUEST,
            ),
            Self::Authentication(_) => (
                "AccessDeniedException",
                "Request authentication failed",
                StatusCode::FORBIDDEN,
            ),
            Self::Body(error) => {
                let code = match error {
                    BodyReadError::TooLarge => "RequestEntityTooLargeException",
                    BodyReadError::Deadline | BodyReadError::Idle => "RequestTimeoutException",
                    _ => "MalformedHttpRequestException",
                };
                (
                    code,
                    "Request body could not be accepted",
                    error.status_code(),
                )
            }
            Self::Json(_) => (
                "ValidationException",
                "Invalid request JSON",
                StatusCode::BAD_REQUEST,
            ),
            Self::Field(code) => (
                code.as_str(),
                "Invalid request field",
                code.default_status_code(),
            ),
            Self::Deadline => (
                "RequestTimeoutException",
                "Request deadline exceeded",
                StatusCode::REQUEST_TIMEOUT,
            ),
            Self::Handler(error)
                if error.message.len() <= 1024
                    && (error.status_code.is_client_error()
                        || error.status_code.is_server_error()) =>
            {
                (error.error_type(), &error.message, error.status_code)
            }
            Self::Handler(_) => (
                "InternalFailure",
                "Invalid handler error",
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        }
    }
}
