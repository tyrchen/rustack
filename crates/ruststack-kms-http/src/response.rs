//! KMS response serialization and error formatting.

use ruststack_kms_model::error::KmsError;

use crate::body::KmsResponseBody;

/// Content type for KMS JSON responses (awsJson1_1).
pub const CONTENT_TYPE: &str = "application/x-amz-json-1.1";

/// Serialize a KMS error into a JSON response body.
///
/// The error format follows the AWS KMS JSON protocol:
///
/// ```json
/// {
///   "__type": "NotFoundException",
///   "message": "..."
/// }
/// ```
#[must_use]
pub fn error_to_json(error: &KmsError) -> Vec<u8> {
    let obj = serde_json::json!({
        "__type": error.error_type(),
        "message": error.message,
    });
    serde_json::to_vec(&obj).expect("JSON serialization of error cannot fail")
}

/// Convert a `KmsError` into a complete HTTP error response.
#[must_use]
pub fn error_to_response(error: &KmsError, request_id: &str) -> http::Response<KmsResponseBody> {
    let json = error_to_json(error);
    let crc = crc32fast::hash(&json);
    let body = KmsResponseBody::from_json(json);

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
pub fn json_response(json: Vec<u8>, request_id: &str) -> http::Response<KmsResponseBody> {
    let crc = crc32fast::hash(&json);
    let body = KmsResponseBody::from_json(json);

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
