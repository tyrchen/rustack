//! Lambda response serialization and error formatting.
//!
//! Lambda uses the `restJson1` error convention:
//! - Error type in `X-Amzn-Errortype` header
//! - Body: `{"Type": "User"|"Service", "Message": "..."}`
//! - Content type: `application/json`

use bytes::Bytes;
use serde::Serialize;

use ruststack_lambda_model::error::LambdaError;

/// Content type for Lambda JSON responses.
pub const CONTENT_TYPE: &str = "application/json";

/// Convert a [`LambdaError`] into a complete HTTP error response.
///
/// The response includes:
/// - HTTP status code from the error code
/// - `X-Amzn-Errortype` header with the error type string
/// - JSON body with `Type` and `Message` fields
#[must_use]
pub fn error_to_response(error: &LambdaError, request_id: &str) -> http::Response<Bytes> {
    let body_obj = serde_json::json!({
        "Type": error.code.error_type(),
        "Message": error.message,
    });
    let body_bytes =
        serde_json::to_vec(&body_obj).expect("JSON serialization of error cannot fail");

    http::Response::builder()
        .status(error.code.status_code())
        .header("content-type", CONTENT_TYPE)
        .header("x-amzn-errortype", error.code.as_str())
        .header("x-amzn-requestid", request_id)
        .body(Bytes::from(body_bytes))
        .expect("valid error response")
}

/// Build a JSON success response with the given status code.
///
/// Serializes the provided value as JSON and returns a response with
/// `application/json` content type.
///
/// # Panics
///
/// Panics if the value cannot be serialized to JSON.
#[must_use]
pub fn json_response(status: u16, body: &impl Serialize) -> http::Response<Bytes> {
    let body_bytes = serde_json::to_vec(body).expect("JSON serialization cannot fail");
    http::Response::builder()
        .status(status)
        .header("content-type", CONTENT_TYPE)
        .body(Bytes::from(body_bytes))
        .expect("valid JSON response")
}

/// Build an empty response with the given status code.
///
/// Used for operations that return 204 No Content (e.g., `DeleteFunction`).
#[must_use]
pub fn empty_response(status: u16) -> http::Response<Bytes> {
    http::Response::builder()
        .status(status)
        .body(Bytes::new())
        .expect("valid empty response")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruststack_lambda_model::error::LambdaErrorCode;

    #[test]
    fn test_should_format_error_with_x_amzn_errortype_header() {
        let err = LambdaError::resource_not_found("Function not found: my-func");
        let resp = error_to_response(&err, "req-123");
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
        assert_eq!(
            resp.headers()
                .get("x-amzn-errortype")
                .expect("has errortype"),
            "ResourceNotFoundException",
        );
        assert_eq!(
            resp.headers()
                .get("content-type")
                .expect("has content-type"),
            CONTENT_TYPE,
        );
        assert_eq!(
            resp.headers()
                .get("x-amzn-requestid")
                .expect("has request id"),
            "req-123",
        );

        let parsed: serde_json::Value =
            serde_json::from_slice(resp.body()).expect("valid JSON body");
        assert_eq!(parsed["Type"], "User");
        assert_eq!(parsed["Message"], "Function not found: my-func");
    }

    #[test]
    fn test_should_format_service_error_with_type_service() {
        let err = LambdaError::service_error("internal failure");
        let resp = error_to_response(&err, "req-456");
        assert_eq!(resp.status(), http::StatusCode::INTERNAL_SERVER_ERROR);

        let parsed: serde_json::Value =
            serde_json::from_slice(resp.body()).expect("valid JSON body");
        assert_eq!(parsed["Type"], "Service");
    }

    #[test]
    fn test_should_build_json_success_response() {
        let body = serde_json::json!({"FunctionName": "my-func", "Runtime": "python3.12"});
        let resp = json_response(201, &body);
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
        let resp = empty_response(204);
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
            (LambdaErrorCode::InvalidParameterValueException, 400),
            (LambdaErrorCode::ResourceNotFoundException, 404),
            (LambdaErrorCode::ResourceConflictException, 409),
            (LambdaErrorCode::RequestTooLargeException, 413),
            (LambdaErrorCode::TooManyRequestsException, 429),
            (LambdaErrorCode::ServiceException, 500),
        ];
        for (code, expected_status) in cases {
            let err = LambdaError::new(code, "test");
            let resp = error_to_response(&err, "req");
            assert_eq!(
                resp.status().as_u16(),
                expected_status,
                "wrong status for {:?}",
                err.code
            );
        }
    }
}
