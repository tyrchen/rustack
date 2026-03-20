//! API Gateway v2 response serialization and error formatting.
//!
//! API Gateway v2 uses a JSON body with a lowercase `message` field:
//! `{"message": "..."}`

use bytes::Bytes;
use serde::Serialize;

use ruststack_apigatewayv2_model::error::ApiGatewayV2Error;

/// Content type for API Gateway v2 JSON responses.
pub const CONTENT_TYPE: &str = "application/json";

/// Convert an [`ApiGatewayV2Error`] into a complete HTTP error response.
///
/// The response includes:
/// - HTTP status code from the error
/// - JSON body with `message` field
///
/// # Errors
///
/// Returns `ApiGatewayV2Error` if JSON serialization or response construction fails.
pub fn error_to_response(
    error: &ApiGatewayV2Error,
) -> Result<http::Response<Bytes>, ApiGatewayV2Error> {
    let body_obj = serde_json::json!({
        "message": error.message,
    });
    let body_bytes = serde_json::to_vec(&body_obj).map_err(|e| {
        ApiGatewayV2Error::internal_error(format!("Failed to serialize error response: {e}"))
    })?;

    http::Response::builder()
        .status(error.status_code)
        .header("content-type", CONTENT_TYPE)
        .body(Bytes::from(body_bytes))
        .map_err(|e| {
            ApiGatewayV2Error::internal_error(format!("Failed to build error response: {e}"))
        })
}

/// Build a JSON success response with the given status code.
///
/// # Errors
///
/// Returns `ApiGatewayV2Error` if serialization fails.
pub fn json_response(
    status: u16,
    body: &impl Serialize,
) -> Result<http::Response<Bytes>, ApiGatewayV2Error> {
    let body_bytes = serde_json::to_vec(body).map_err(|e| {
        ApiGatewayV2Error::internal_error(format!("Failed to serialize response: {e}"))
    })?;
    http::Response::builder()
        .status(status)
        .header("content-type", CONTENT_TYPE)
        .body(Bytes::from(body_bytes))
        .map_err(|e| ApiGatewayV2Error::internal_error(format!("Failed to build response: {e}")))
}

/// Build an empty response with the given status code.
///
/// # Errors
///
/// Returns `ApiGatewayV2Error` if response construction fails.
pub fn empty_response(status: u16) -> Result<http::Response<Bytes>, ApiGatewayV2Error> {
    http::Response::builder()
        .status(status)
        .body(Bytes::new())
        .map_err(|e| {
            ApiGatewayV2Error::internal_error(format!("Failed to build empty response: {e}"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruststack_apigatewayv2_model::error::ApiGatewayV2ErrorCode;

    #[test]
    fn test_should_format_error_with_lowercase_message() {
        let err = ApiGatewayV2Error::with_message(
            ApiGatewayV2ErrorCode::NotFoundException,
            "API not found: abc123",
        );
        let resp = error_to_response(&err).expect("should build");
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
        assert_eq!(
            resp.headers()
                .get("content-type")
                .expect("has content-type"),
            CONTENT_TYPE,
        );

        let parsed: serde_json::Value =
            serde_json::from_slice(resp.body()).expect("valid JSON body");
        assert_eq!(parsed["message"], "API not found: abc123");
        // Must NOT have Type field.
        assert!(parsed.get("Type").is_none());
    }

    #[test]
    fn test_should_format_bad_request_error() {
        let err = ApiGatewayV2Error::with_message(
            ApiGatewayV2ErrorCode::BadRequestException,
            "Invalid input",
        );
        let resp = error_to_response(&err).expect("should build");
        assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);

        let parsed: serde_json::Value =
            serde_json::from_slice(resp.body()).expect("valid JSON body");
        assert_eq!(parsed["message"], "Invalid input");
    }

    #[test]
    fn test_should_build_json_success_response() {
        let body = serde_json::json!({"apiId": "abc123", "name": "my-api"});
        let resp = json_response(201, &body).expect("should build");
        assert_eq!(resp.status().as_u16(), 201);
        assert_eq!(
            resp.headers()
                .get("content-type")
                .expect("has content-type"),
            CONTENT_TYPE,
        );
        assert!(!resp.body().is_empty());
    }

    #[test]
    fn test_should_build_empty_response() {
        let resp = empty_response(204).expect("should build");
        assert_eq!(resp.status().as_u16(), 204);
        assert!(resp.body().is_empty());
    }

    #[test]
    fn test_should_use_application_json_content_type() {
        assert_eq!(CONTENT_TYPE, "application/json");
    }

    #[test]
    fn test_should_map_all_error_codes_to_correct_status() {
        let cases = [
            (ApiGatewayV2ErrorCode::BadRequestException, 400),
            (ApiGatewayV2ErrorCode::AccessDeniedException, 403),
            (ApiGatewayV2ErrorCode::NotFoundException, 404),
            (ApiGatewayV2ErrorCode::ConflictException, 409),
            (ApiGatewayV2ErrorCode::TooManyRequestsException, 500),
            (ApiGatewayV2ErrorCode::UnknownOperation, 500),
        ];
        for (code, expected_status) in cases {
            let err = ApiGatewayV2Error::with_message(code, "test");
            let resp = error_to_response(&err).expect("should build");
            assert_eq!(
                resp.status().as_u16(),
                expected_status,
                "wrong status for {code:?}",
            );
        }
    }

    #[test]
    fn test_should_not_include_type_field_in_error() {
        let err =
            ApiGatewayV2Error::with_message(ApiGatewayV2ErrorCode::AccessDeniedException, "denied");
        let resp = error_to_response(&err).expect("should build");
        let parsed: serde_json::Value = serde_json::from_slice(resp.body()).expect("valid JSON");
        assert!(parsed.get("Type").is_none());
        assert!(parsed.get("__type").is_none());
        assert_eq!(parsed["message"], "denied");
    }
}
