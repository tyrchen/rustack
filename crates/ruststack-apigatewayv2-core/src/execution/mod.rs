//! API Gateway v2 execution engine.
//!
//! Handles routing incoming API requests to the appropriate backend integration
//! (Lambda proxy, HTTP proxy, or mock) and constructing the response.

pub mod cors;
pub mod event;
pub mod http_proxy;
pub mod lambda_proxy;
pub mod mock_integration;
pub mod router;

use bytes::Bytes;
use ruststack_apigatewayv2_model::types::IntegrationType;

use crate::error::ApiGatewayV2ServiceError;
use crate::provider::RustStackApiGatewayV2;

/// Target for an API execution request.
#[derive(Debug)]
pub struct ExecutionTarget {
    /// The API ID.
    pub api_id: String,
    /// The stage name.
    pub stage_name: String,
    /// The remaining path after stage.
    pub path: String,
}

/// Parse an execution path into API ID, stage name, and remaining path.
///
/// Expected format: `/{api_id}/{stage_name}/{remaining_path}`
///
/// # Errors
///
/// Returns `BadRequest` if the path format is invalid.
pub fn parse_execution_path(path: &str) -> Result<ExecutionTarget, ApiGatewayV2ServiceError> {
    let trimmed = path.strip_prefix('/').unwrap_or(path);
    let mut parts = trimmed.splitn(3, '/');

    let api_id = parts
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ApiGatewayV2ServiceError::BadRequest("Missing API ID in execution path".to_owned())
        })?
        .to_owned();

    let stage_name = parts
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ApiGatewayV2ServiceError::BadRequest("Missing stage name in execution path".to_owned())
        })?
        .to_owned();

    let remaining_path = parts
        .next()
        .map_or_else(|| "/".to_owned(), |p| format!("/{p}"));

    Ok(ExecutionTarget {
        api_id,
        stage_name,
        path: remaining_path,
    })
}

/// Handle an API execution request.
///
/// Routes the request to the matched integration and returns the response.
pub async fn handle_execution(
    provider: &RustStackApiGatewayV2,
    api_id: &str,
    stage_name: &str,
    method: &http::Method,
    path: &str,
    headers: &http::HeaderMap,
    body: &[u8],
) -> Result<http::Response<Bytes>, ApiGatewayV2ServiceError> {
    let api = provider.store().apis.get(api_id).ok_or_else(|| {
        ApiGatewayV2ServiceError::NotFound(format!("Unable to find Api with id '{api_id}'"))
    })?;

    // Verify the stage exists
    let _stage = api.stages.get(stage_name).ok_or_else(|| {
        ApiGatewayV2ServiceError::NotFound(format!("Unable to find Stage with name '{stage_name}'"))
    })?;

    // Route the request to the appropriate route
    let route_key = format!("{method} {path}");
    let matched = router::match_route(&api.routes, method, path);

    let (route, _path_params) = matched.ok_or_else(|| {
        ApiGatewayV2ServiceError::NotFound(format!("No route matches '{route_key}'"))
    })?;

    // Get the integration
    let integration_id = route
        .target
        .as_ref()
        .and_then(|t| t.strip_prefix("integrations/"))
        .ok_or_else(|| {
            ApiGatewayV2ServiceError::Internal("Route has no integration target".to_owned())
        })?;

    let integration = api.integrations.get(integration_id).ok_or_else(|| {
        ApiGatewayV2ServiceError::NotFound(format!(
            "Unable to find Integration with id '{integration_id}'"
        ))
    })?;

    // Check for CORS preflight
    if method == http::Method::OPTIONS {
        if let Some(cors_config) = &api.cors_configuration {
            return Ok(cors::build_cors_preflight_response(cors_config));
        }
    }

    // Dispatch based on integration type
    match integration.integration_type {
        IntegrationType::AwsProxy => {
            lambda_proxy::handle_lambda_proxy(
                provider,
                api_id,
                stage_name,
                &route_key,
                integration,
                headers,
                body,
            )
            .await
        }
        IntegrationType::HttpProxy | IntegrationType::Http => {
            http_proxy::handle_http_proxy(provider, integration, method, path, headers, body).await
        }
        IntegrationType::Mock => mock_integration::handle_mock(integration),
        IntegrationType::Aws => Err(ApiGatewayV2ServiceError::Internal(
            "AWS integration type is not supported in local emulation".to_owned(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_parse_execution_path() {
        let target = parse_execution_path("/abc123/prod/items/42").expect("should parse");
        assert_eq!(target.api_id, "abc123");
        assert_eq!(target.stage_name, "prod");
        assert_eq!(target.path, "/items/42");
    }

    #[test]
    fn test_should_parse_execution_path_without_trailing_path() {
        let target = parse_execution_path("/abc123/prod").expect("should parse");
        assert_eq!(target.api_id, "abc123");
        assert_eq!(target.stage_name, "prod");
        assert_eq!(target.path, "/");
    }

    #[test]
    fn test_should_fail_on_missing_api_id() {
        let result = parse_execution_path("/");
        assert!(result.is_err());
    }
}
