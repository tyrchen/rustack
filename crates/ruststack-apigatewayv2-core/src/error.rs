//! API Gateway v2 service error types.

use ruststack_apigatewayv2_model::error::{ApiGatewayV2Error, ApiGatewayV2ErrorCode};

/// API Gateway v2 service error.
#[derive(Debug, thiserror::Error)]
pub enum ApiGatewayV2ServiceError {
    /// Resource not found (404).
    #[error("Not found: {0}")]
    NotFound(String),

    /// Conflict (409).
    #[error("Conflict: {0}")]
    Conflict(String),

    /// Bad request (400).
    #[error("Bad request: {0}")]
    BadRequest(String),

    /// Too many requests (429).
    #[error("Too many requests: {0}")]
    TooManyRequests(String),

    /// Access denied (403).
    #[error("Access denied: {0}")]
    AccessDenied(String),

    /// Integration error (502).
    #[error("Integration error: {0}")]
    IntegrationError(String),

    /// Internal server error (500).
    #[error("Internal error: {0}")]
    Internal(String),
}

impl ApiGatewayV2ServiceError {
    /// Convert to an error response tuple of (status_code, error_type, message).
    #[must_use]
    pub fn to_error_response(&self) -> (u16, &'static str, String) {
        match self {
            Self::NotFound(msg) => (404, "NotFoundException", msg.clone()),
            Self::Conflict(msg) => (409, "ConflictException", msg.clone()),
            Self::BadRequest(msg) => (400, "BadRequestException", msg.clone()),
            Self::TooManyRequests(msg) => (429, "TooManyRequestsException", msg.clone()),
            Self::AccessDenied(msg) => (403, "AccessDeniedException", msg.clone()),
            Self::IntegrationError(msg) => (502, "BadGatewayException", msg.clone()),
            Self::Internal(msg) => (500, "InternalServerError", msg.clone()),
        }
    }
}

impl From<ApiGatewayV2ServiceError> for ApiGatewayV2Error {
    fn from(err: ApiGatewayV2ServiceError) -> Self {
        match err {
            ApiGatewayV2ServiceError::NotFound(msg) => {
                Self::with_message(ApiGatewayV2ErrorCode::NotFoundException, msg)
            }
            ApiGatewayV2ServiceError::Conflict(msg) => {
                Self::with_message(ApiGatewayV2ErrorCode::ConflictException, msg)
            }
            ApiGatewayV2ServiceError::BadRequest(msg)
            | ApiGatewayV2ServiceError::IntegrationError(msg) => {
                Self::with_message(ApiGatewayV2ErrorCode::BadRequestException, msg)
            }
            ApiGatewayV2ServiceError::TooManyRequests(msg) => {
                Self::with_message(ApiGatewayV2ErrorCode::TooManyRequestsException, msg)
            }
            ApiGatewayV2ServiceError::AccessDenied(msg) => {
                Self::with_message(ApiGatewayV2ErrorCode::AccessDeniedException, msg)
            }
            ApiGatewayV2ServiceError::Internal(msg) => Self::internal_error(msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_convert_not_found_to_error_response() {
        let err = ApiGatewayV2ServiceError::NotFound("API abc not found".to_owned());
        let (status, error_type, msg) = err.to_error_response();
        assert_eq!(status, 404);
        assert_eq!(error_type, "NotFoundException");
        assert_eq!(msg, "API abc not found");
    }

    #[test]
    fn test_should_convert_to_apigatewayv2_error() {
        let err = ApiGatewayV2ServiceError::Conflict("already exists".to_owned());
        let apigw_err: ApiGatewayV2Error = err.into();
        assert_eq!(apigw_err.code, ApiGatewayV2ErrorCode::ConflictException);
    }
}
