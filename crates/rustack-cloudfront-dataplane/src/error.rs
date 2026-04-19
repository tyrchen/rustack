//! Data-plane error type.

use thiserror::Error;

/// Errors that can occur while serving a data-plane request.
#[derive(Debug, Error)]
pub enum DataPlaneError {
    /// The distribution ID in the request path does not exist.
    #[error("NoSuchDistribution: {0}")]
    NoSuchDistribution(String),

    /// The distribution exists but is disabled.
    #[error("DistributionDisabled: {0}")]
    DistributionDisabled(String),

    /// The request method is not in the behavior's `AllowedMethods`.
    #[error("MethodNotAllowed: {0}")]
    MethodNotAllowed(String),

    /// The behavior could not be matched or has no valid target origin.
    #[error("BehaviorResolution: {0}")]
    BehaviorResolution(String),

    /// Origin call returned 4xx (but wasn't NoSuchKey).
    #[error("OriginClientError({status}): {message}")]
    OriginClientError {
        /// HTTP status.
        status: u16,
        /// Message.
        message: String,
    },

    /// Origin call returned 5xx.
    #[error("OriginServerError({status}): {message}")]
    OriginServerError {
        /// HTTP status.
        status: u16,
        /// Message.
        message: String,
    },

    /// Object not found at origin.
    #[error("ObjectNotFound: {0}")]
    ObjectNotFound(String),

    /// Feature triggered that Rustack does not execute (FAIL_ON_FUNCTION).
    #[error("FunctionExecutionSkipped: {0}")]
    FunctionExecutionSkipped(String),

    /// Signed URL required but not supplied (FAIL_ON_FUNCTION).
    #[error("SignedUrlRequired: {0}")]
    SignedUrlRequired(String),

    /// Upstream body exceeded the configured buffer cap.
    #[error("PayloadTooLarge: {0}")]
    PayloadTooLarge(String),

    /// Catch-all internal error.
    #[error("Internal: {0}")]
    Internal(String),
}

impl DataPlaneError {
    /// HTTP status for this error.
    #[must_use]
    pub fn http_status(&self) -> u16 {
        match self {
            Self::NoSuchDistribution(_) | Self::ObjectNotFound(_) => 404,
            Self::DistributionDisabled(_) | Self::SignedUrlRequired(_) => 403,
            Self::MethodNotAllowed(_) => 405,
            Self::BehaviorResolution(_) | Self::Internal(_) | Self::FunctionExecutionSkipped(_) => {
                500
            }
            Self::OriginClientError { status, .. } | Self::OriginServerError { status, .. } => {
                *status
            }
            Self::PayloadTooLarge(_) => 413,
        }
    }

    /// AWS-style error code for the `X-Amzn-Errortype` header.
    #[must_use]
    pub fn error_type(&self) -> &'static str {
        match self {
            Self::NoSuchDistribution(_) => "NoSuchDistribution",
            Self::DistributionDisabled(_) => "DistributionDisabled",
            Self::MethodNotAllowed(_) => "MethodNotAllowed",
            Self::BehaviorResolution(_) => "BehaviorResolutionFailed",
            Self::OriginClientError { .. } => "OriginClientError",
            Self::OriginServerError { .. } => "OriginServerError",
            Self::ObjectNotFound(_) => "NoSuchKey",
            Self::FunctionExecutionSkipped(_) => "FunctionExecutionSkipped",
            Self::SignedUrlRequired(_) => "SignedUrlRequired",
            Self::PayloadTooLarge(_) => "PayloadTooLarge",
            Self::Internal(_) => "InternalError",
        }
    }
}
