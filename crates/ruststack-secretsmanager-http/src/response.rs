//! Secrets Manager response serialization and error formatting.

use ruststack_secretsmanager_model::error::SecretsManagerError;

use crate::body::SecretsManagerResponseBody;

/// Content type for Secrets Manager JSON responses (awsJson1_1).
pub const CONTENT_TYPE: &str = "application/x-amz-json-1.1";

/// Serialize a Secrets Manager error into a JSON response body.
///
/// The error format follows the AWS Secrets Manager JSON protocol:
///
/// ```json
/// {
///   "__type": "ResourceNotFoundException",
///   "Message": "..."
/// }
/// ```
///
/// Note: Secrets Manager uses short error type names and capital `"Message"` field.
#[must_use]
pub fn error_to_json(error: &SecretsManagerError) -> Vec<u8> {
    let obj = serde_json::json!({
        "__type": error.error_type(),
        "Message": error.message,
    });
    serde_json::to_vec(&obj).expect("JSON serialization of error cannot fail")
}

/// Convert a `SecretsManagerError` into a complete HTTP error response.
#[must_use]
pub fn error_to_response(
    error: &SecretsManagerError,
    request_id: &str,
) -> http::Response<SecretsManagerResponseBody> {
    let json = error_to_json(error);
    let crc = crc32fast::hash(&json);
    let body = SecretsManagerResponseBody::from_json(json);

    let mut response = http::Response::builder()
        .status(error.status_code)
        .header("content-type", CONTENT_TYPE)
        .header("x-amzn-requestid", request_id)
        .body(body)
        .expect("valid error response");

    if let Ok(hv) = http::HeaderValue::from_str(&crc.to_string()) {
        response.headers_mut().insert("x-amz-crc32", hv);
    }

    response
}

/// Build a success response from JSON bytes.
#[must_use]
pub fn json_response(
    json: Vec<u8>,
    request_id: &str,
) -> http::Response<SecretsManagerResponseBody> {
    let crc = crc32fast::hash(&json);
    let body = SecretsManagerResponseBody::from_json(json);

    let mut response = http::Response::builder()
        .status(http::StatusCode::OK)
        .header("content-type", CONTENT_TYPE)
        .header("x-amzn-requestid", request_id)
        .body(body)
        .expect("valid JSON response");

    if let Ok(hv) = http::HeaderValue::from_str(&crc.to_string()) {
        response.headers_mut().insert("x-amz-crc32", hv);
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruststack_secretsmanager_model::error::SecretsManagerErrorCode;

    #[test]
    fn test_should_format_error_json_with_capital_message() {
        let err = SecretsManagerError::with_message(
            SecretsManagerErrorCode::ResourceNotFoundException,
            "Secret my/secret not found",
        );
        let json = error_to_json(&err);
        let parsed: serde_json::Value = serde_json::from_slice(&json).expect("valid JSON");
        assert_eq!(parsed["__type"], "ResourceNotFoundException");
        assert_eq!(parsed["Message"], "Secret my/secret not found");
        // Ensure lowercase "message" is NOT present
        assert!(parsed.get("message").is_none());
    }

    #[test]
    fn test_should_build_error_response_with_correct_status() {
        let err = SecretsManagerError::with_message(
            SecretsManagerErrorCode::ResourceNotFoundException,
            "not found",
        );
        let resp = error_to_response(&err, "test-req-123");
        assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);
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
            "test-req-123",
        );
        assert!(resp.headers().get("x-amz-crc32").is_some());
    }

    #[test]
    fn test_should_build_json_success_response() {
        let json =
            serde_json::to_vec(&serde_json::json!({"Name": "my-secret", "VersionId": "abc123"}))
                .expect("valid JSON");
        let resp = json_response(json, "req-456");
        assert_eq!(resp.status(), http::StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get("content-type")
                .expect("has content-type"),
            CONTENT_TYPE,
        );
        assert!(resp.headers().get("x-amz-crc32").is_some());
    }

    #[test]
    fn test_should_use_1_1_content_type() {
        assert_eq!(CONTENT_TYPE, "application/x-amz-json-1.1");
    }
}
