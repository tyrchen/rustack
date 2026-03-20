//! Mock integration for API Gateway v2.
//!
//! Returns a static response based on the integration configuration.

use bytes::Bytes;

use crate::error::ApiGatewayV2ServiceError;
use crate::storage::IntegrationRecord;

/// Handle a mock integration.
///
/// Returns a static 200 response with a JSON body containing the mock status.
pub fn handle_mock(
    integration: &IntegrationRecord,
) -> Result<http::Response<Bytes>, ApiGatewayV2ServiceError> {
    // For mock integrations, return a response based on request templates
    // or a default 200 OK.
    let status_code = integration
        .request_templates
        .get("application/json")
        .and_then(|template| {
            // Simple parsing: look for "statusCode" in the template
            serde_json::from_str::<serde_json::Value>(template)
                .ok()
                .and_then(|v| v["statusCode"].as_u64())
                .and_then(|s| u16::try_from(s).ok())
        })
        .unwrap_or(200);

    let body = serde_json::json!({
        "statusCode": status_code,
        "message": "Mock response",
    });

    let body_bytes = serde_json::to_vec(&body).map_err(|e| {
        ApiGatewayV2ServiceError::Internal(format!("Failed to serialize mock response: {e}"))
    })?;

    http::Response::builder()
        .status(status_code)
        .header("content-type", "application/json")
        .body(Bytes::from(body_bytes))
        .map_err(|e| {
            ApiGatewayV2ServiceError::Internal(format!("Failed to build mock response: {e}"))
        })
}
