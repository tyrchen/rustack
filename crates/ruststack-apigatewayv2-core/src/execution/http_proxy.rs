//! HTTP proxy integration for API Gateway v2.
//!
//! Forwards requests to the configured HTTP endpoint and returns the response.

use bytes::Bytes;

use crate::error::ApiGatewayV2ServiceError;
use crate::provider::RustStackApiGatewayV2;
use crate::storage::IntegrationRecord;

/// Handle an HTTP proxy integration.
///
/// Forwards the request to the integration URI and returns the response.
pub async fn handle_http_proxy(
    provider: &RustStackApiGatewayV2,
    integration: &IntegrationRecord,
    method: &http::Method,
    path: &str,
    headers: &http::HeaderMap,
    body: &[u8],
) -> Result<http::Response<Bytes>, ApiGatewayV2ServiceError> {
    let base_uri = integration.integration_uri.as_deref().ok_or_else(|| {
        ApiGatewayV2ServiceError::Internal("HTTP integration has no URI".to_owned())
    })?;

    let target_url = format!("{base_uri}{path}");
    let reqwest_method = reqwest::Method::from_bytes(method.as_str().as_bytes())
        .map_err(|e| ApiGatewayV2ServiceError::BadRequest(format!("Invalid HTTP method: {e}")))?;

    let mut request = provider.http_client().request(reqwest_method, &target_url);

    // Forward headers (skip host header)
    for (name, value) in headers {
        if name != "host" {
            if let Ok(v) = value.to_str() {
                request = request.header(name.as_str(), v);
            }
        }
    }

    if !body.is_empty() {
        request = request.body(body.to_vec());
    }

    let response = request.send().await.map_err(|e| {
        ApiGatewayV2ServiceError::IntegrationError(format!("HTTP proxy request failed: {e}"))
    })?;

    let status = response.status().as_u16();
    let resp_headers = response.headers().clone();
    let resp_body = response.bytes().await.map_err(|e| {
        ApiGatewayV2ServiceError::IntegrationError(format!(
            "Failed to read HTTP proxy response: {e}"
        ))
    })?;

    let mut builder = http::Response::builder().status(status);
    for (name, value) in &resp_headers {
        builder = builder.header(name.as_str(), value.as_bytes());
    }

    builder
        .body(Bytes::from(resp_body.to_vec()))
        .map_err(|e| ApiGatewayV2ServiceError::Internal(format!("Failed to build response: {e}")))
}
