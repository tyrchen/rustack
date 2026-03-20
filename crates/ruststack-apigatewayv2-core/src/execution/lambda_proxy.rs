//! Lambda proxy integration for API Gateway v2.
//!
//! Constructs a Lambda event v2, invokes the Lambda function, and transforms
//! the response back into an HTTP response.

use std::collections::HashMap;

use bytes::Bytes;
use serde::Deserialize;

use crate::error::ApiGatewayV2ServiceError;
use crate::provider::RustStackApiGatewayV2;
use crate::storage::IntegrationRecord;

use super::event::build_lambda_event;

/// Lambda function response (payload format version 2.0).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LambdaResponse {
    #[serde(default = "default_status")]
    status_code: u16,
    #[serde(default)]
    headers: HashMap<String, String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    is_base64_encoded: bool,
}

fn default_status() -> u16 {
    200
}

/// Handle a Lambda proxy integration.
///
/// Constructs the event payload, sends it to the Lambda invoke endpoint,
/// and transforms the response.
pub async fn handle_lambda_proxy(
    provider: &RustStackApiGatewayV2,
    api_id: &str,
    stage_name: &str,
    route_key: &str,
    integration: &IntegrationRecord,
    headers: &http::HeaderMap,
    body: &[u8],
) -> Result<http::Response<Bytes>, ApiGatewayV2ServiceError> {
    let integration_uri = integration.integration_uri.as_deref().ok_or_else(|| {
        ApiGatewayV2ServiceError::Internal("Lambda integration has no URI".to_owned())
    })?;

    // Extract function name from ARN or use as-is
    let function_name = extract_function_name(integration_uri);

    let event = build_lambda_event(&super::event::LambdaEventParams {
        api_id,
        stage_name,
        route_key,
        method: "GET",
        path: "/",
        headers,
        body,
        path_params: &HashMap::new(),
        stage_variables: &HashMap::new(),
        account_id: &provider.config().account_id,
        region: &provider.config().default_region,
    });

    let event_json = serde_json::to_vec(&event).map_err(|e| {
        ApiGatewayV2ServiceError::Internal(format!("Failed to serialize Lambda event: {e}"))
    })?;

    // Invoke Lambda via the local endpoint
    let invoke_url = format!(
        "http://{}:{}/2015-03-31/functions/{function_name}/invocations",
        provider.config().host,
        provider.config().port,
    );

    let response = provider
        .http_client()
        .post(&invoke_url)
        .header("content-type", "application/json")
        .body(event_json)
        .send()
        .await
        .map_err(|e| {
            ApiGatewayV2ServiceError::IntegrationError(format!(
                "Failed to invoke Lambda function: {e}"
            ))
        })?;

    let response_bytes = response.bytes().await.map_err(|e| {
        ApiGatewayV2ServiceError::IntegrationError(format!("Failed to read Lambda response: {e}"))
    })?;

    // Parse Lambda response
    let lambda_resp: LambdaResponse = serde_json::from_slice(&response_bytes).map_err(|e| {
        ApiGatewayV2ServiceError::IntegrationError(format!("Failed to parse Lambda response: {e}"))
    })?;

    // Build HTTP response
    let response_body = if let Some(b) = lambda_resp.body {
        if lambda_resp.is_base64_encoded {
            use base64::Engine;
            Bytes::from(
                base64::engine::general_purpose::STANDARD
                    .decode(&b)
                    .unwrap_or_else(|_| b.into_bytes()),
            )
        } else {
            Bytes::from(b)
        }
    } else {
        Bytes::new()
    };

    let mut builder = http::Response::builder().status(lambda_resp.status_code);
    for (k, v) in &lambda_resp.headers {
        builder = builder.header(k.as_str(), v.as_str());
    }

    builder
        .body(response_body)
        .map_err(|e| ApiGatewayV2ServiceError::Internal(format!("Failed to build response: {e}")))
}

/// Extract the function name from a Lambda ARN or return the input as-is.
fn extract_function_name(uri: &str) -> &str {
    // ARN format: arn:aws:lambda:{region}:{account}:function:{name}
    if uri.starts_with("arn:") {
        uri.rsplit(':').next().unwrap_or(uri)
    } else {
        uri
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_extract_function_name_from_arn() {
        let arn = "arn:aws:lambda:us-east-1:123456789012:function:my-func";
        assert_eq!(extract_function_name(arn), "my-func");
    }

    #[test]
    fn test_should_return_plain_name_as_is() {
        assert_eq!(extract_function_name("my-func"), "my-func");
    }
}
