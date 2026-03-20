//! API Gateway v2 handler bridging HTTP to business logic.
//!
//! Parses JSON request bodies, dispatches to the provider, and serializes
//! JSON responses following the `restJson1` protocol.
//!
//! Uses manual `Pin<Box<dyn Future>>` return types because the `ApiGatewayV2Handler`
//! trait requires object safety for `Arc<dyn ApiGatewayV2Handler>`.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;

use ruststack_apigatewayv2_http::body::ApiGatewayV2ResponseBody;
use ruststack_apigatewayv2_http::dispatch::ApiGatewayV2Handler;
use ruststack_apigatewayv2_http::response::{empty_response, json_response};
use ruststack_apigatewayv2_http::router::PathParams;
use ruststack_apigatewayv2_model::error::ApiGatewayV2Error;
#[allow(clippy::wildcard_imports)]
use ruststack_apigatewayv2_model::input::*;
use ruststack_apigatewayv2_model::operations::ApiGatewayV2Operation;

use crate::provider::RustStackApiGatewayV2;

/// Handler that bridges the HTTP layer to the API Gateway v2 provider.
#[derive(Debug)]
pub struct RustStackApiGatewayV2Handler {
    provider: Arc<RustStackApiGatewayV2>,
}

impl RustStackApiGatewayV2Handler {
    /// Create a new handler wrapping a provider.
    #[must_use]
    pub fn new(provider: Arc<RustStackApiGatewayV2>) -> Self {
        Self { provider }
    }
}

impl ApiGatewayV2Handler for RustStackApiGatewayV2Handler {
    fn handle_operation(
        &self,
        op: ApiGatewayV2Operation,
        path_params: PathParams,
        query: String,
        _headers: http::HeaderMap,
        body: Bytes,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<http::Response<ApiGatewayV2ResponseBody>, ApiGatewayV2Error>>
                + Send,
        >,
    > {
        let provider = Arc::clone(&self.provider);
        Box::pin(async move { dispatch(&provider, op, &path_params, &query, &body).await })
    }
}

/// Parse query parameters from a URL query string.
fn parse_query_params(query: &str) -> Vec<(String, String)> {
    if query.is_empty() {
        return Vec::new();
    }
    query
        .split('&')
        .filter(|s| !s.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (percent_decode(key), percent_decode(value))
        })
        .collect()
}

/// Simple percent-decoding for query parameter keys and values.
fn percent_decode(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if hex.len() == 2 {
                if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                    result.push(byte as char);
                    continue;
                }
            }
            result.push('%');
            result.push_str(&hex);
        } else if ch == '+' {
            result.push(' ');
        } else {
            result.push(ch);
        }
    }
    result
}

/// Get a query parameter by name.
fn get_query_param<'a>(params: &'a [(String, String)], key: &str) -> Option<&'a str> {
    params
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// Extract a required path parameter or return an error.
fn require_path_param<'a>(
    params: &'a PathParams,
    name: &str,
) -> Result<&'a str, ApiGatewayV2Error> {
    params
        .get(name)
        .ok_or_else(|| ApiGatewayV2Error::internal_error(format!("Missing path parameter: {name}")))
}

/// Wrap a serializable value into a JSON response.
fn wrap_json_response(
    status: u16,
    body: &impl serde::Serialize,
) -> Result<http::Response<ApiGatewayV2ResponseBody>, ApiGatewayV2Error> {
    let bytes_response = json_response(status, body)?;
    let (parts, body) = bytes_response.into_parts();
    Ok(http::Response::from_parts(
        parts,
        ApiGatewayV2ResponseBody::from_bytes(body),
    ))
}

/// Wrap an empty response with the given status code.
fn wrap_empty_response(
    status: u16,
) -> Result<http::Response<ApiGatewayV2ResponseBody>, ApiGatewayV2Error> {
    let bytes_response = empty_response(status)?;
    let (parts, body) = bytes_response.into_parts();
    Ok(http::Response::from_parts(
        parts,
        ApiGatewayV2ResponseBody::from_bytes(body),
    ))
}

/// Merge path parameters and query parameters into the JSON body so that
/// required fields like `apiId` are present when deserializing the input struct.
fn merge_params(body: &[u8], path_params: &PathParams, query: &str) -> Vec<u8> {
    let mut obj: serde_json::Map<String, serde_json::Value> = if body.is_empty() {
        serde_json::Map::new()
    } else if let Ok(map) = serde_json::from_slice(body) {
        map
    } else {
        return body.to_vec();
    };

    // Insert each path param into the JSON body if not already present.
    // Handle special case: route pattern uses `resource-arn` but serde expects `resourceArn`.
    for (key, value) in path_params.iter() {
        let json_key = if key == "resource-arn" {
            "resourceArn".to_owned()
        } else {
            key.to_owned()
        };
        obj.entry(json_key)
            .or_insert(serde_json::Value::String(value.to_owned()));
    }

    // Merge query parameters. `tagKeys` can appear multiple times and must be an array.
    for (key, value) in parse_query_params(query) {
        let entry = obj.entry(key);
        match entry {
            serde_json::map::Entry::Vacant(v) => {
                if v.key() == "tagKeys" {
                    v.insert(serde_json::Value::Array(vec![serde_json::Value::String(
                        value,
                    )]));
                } else {
                    v.insert(serde_json::Value::String(value));
                }
            }
            serde_json::map::Entry::Occupied(mut o) => {
                if let serde_json::Value::Array(arr) = o.get_mut() {
                    arr.push(serde_json::Value::String(value));
                }
            }
        }
    }

    serde_json::to_vec(&obj).unwrap_or_else(|_| body.to_vec())
}

/// Dispatch an API Gateway v2 operation to the appropriate provider method.
#[allow(clippy::too_many_lines, clippy::assigning_clones, clippy::unused_async)]
async fn dispatch(
    provider: &RustStackApiGatewayV2,
    op: ApiGatewayV2Operation,
    path_params: &PathParams,
    query: &str,
    body: &[u8],
) -> Result<http::Response<ApiGatewayV2ResponseBody>, ApiGatewayV2Error> {
    // Merge path params and query params into body for deserialization.
    let merged = merge_params(body, path_params, query);
    let body = &merged;
    let query_params = parse_query_params(query);

    match op {
        // ---- API CRUD ----
        ApiGatewayV2Operation::CreateApi => {
            let input: CreateApiInput = parse_body(body)?;
            let output = provider
                .create_api(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(201, &output)
        }
        ApiGatewayV2Operation::GetApi => {
            let api_id = require_path_param(path_params, "apiId")?;
            let input = GetApiInput {
                api_id: api_id.to_owned(),
            };
            let output = provider.get_api(input).map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }
        ApiGatewayV2Operation::UpdateApi => {
            let api_id = require_path_param(path_params, "apiId")?;
            let mut input: UpdateApiInput = parse_body(body)?;
            input.api_id = api_id.to_owned();
            let output = provider
                .update_api(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }
        ApiGatewayV2Operation::DeleteApi => {
            let api_id = require_path_param(path_params, "apiId")?;
            let input = DeleteApiInput {
                api_id: api_id.to_owned(),
            };
            provider
                .delete_api(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_empty_response(204)
        }
        ApiGatewayV2Operation::GetApis => {
            let input = GetApisInput {
                max_results: get_query_param(&query_params, "maxResults").map(str::to_owned),
                next_token: get_query_param(&query_params, "nextToken").map(str::to_owned),
            };
            let output = provider.get_apis(input).map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }

        // ---- Route CRUD ----
        ApiGatewayV2Operation::CreateRoute => {
            let api_id = require_path_param(path_params, "apiId")?;
            let mut input: CreateRouteInput = parse_body(body)?;
            input.api_id = api_id.to_owned();
            let output = provider
                .create_route(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(201, &output)
        }
        ApiGatewayV2Operation::GetRoute => {
            let api_id = require_path_param(path_params, "apiId")?;
            let route_id = require_path_param(path_params, "routeId")?;
            let input = GetRouteInput {
                api_id: api_id.to_owned(),
                route_id: route_id.to_owned(),
            };
            let output = provider.get_route(input).map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }
        ApiGatewayV2Operation::UpdateRoute => {
            let api_id = require_path_param(path_params, "apiId")?;
            let route_id = require_path_param(path_params, "routeId")?;
            let mut input: UpdateRouteInput = parse_body(body)?;
            input.api_id = api_id.to_owned();
            input.route_id = route_id.to_owned();
            let output = provider
                .update_route(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }
        ApiGatewayV2Operation::DeleteRoute => {
            let api_id = require_path_param(path_params, "apiId")?;
            let route_id = require_path_param(path_params, "routeId")?;
            let input = DeleteRouteInput {
                api_id: api_id.to_owned(),
                route_id: route_id.to_owned(),
            };
            provider
                .delete_route(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_empty_response(204)
        }
        ApiGatewayV2Operation::GetRoutes => {
            let api_id = require_path_param(path_params, "apiId")?;
            let input = GetRoutesInput {
                api_id: api_id.to_owned(),
                max_results: get_query_param(&query_params, "maxResults").map(str::to_owned),
                next_token: get_query_param(&query_params, "nextToken").map(str::to_owned),
            };
            let output = provider
                .get_routes(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }

        // ---- Integration CRUD ----
        ApiGatewayV2Operation::CreateIntegration => {
            let api_id = require_path_param(path_params, "apiId")?;
            let mut input: CreateIntegrationInput = parse_body(body)?;
            input.api_id = api_id.to_owned();
            let output = provider
                .create_integration(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(201, &output)
        }
        ApiGatewayV2Operation::GetIntegration => {
            let api_id = require_path_param(path_params, "apiId")?;
            let integration_id = require_path_param(path_params, "integrationId")?;
            let input = GetIntegrationInput {
                api_id: api_id.to_owned(),
                integration_id: integration_id.to_owned(),
            };
            let output = provider
                .get_integration(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }
        ApiGatewayV2Operation::UpdateIntegration => {
            let api_id = require_path_param(path_params, "apiId")?;
            let integration_id = require_path_param(path_params, "integrationId")?;
            let mut input: UpdateIntegrationInput = parse_body(body)?;
            input.api_id = api_id.to_owned();
            input.integration_id = integration_id.to_owned();
            let output = provider
                .update_integration(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }
        ApiGatewayV2Operation::DeleteIntegration => {
            let api_id = require_path_param(path_params, "apiId")?;
            let integration_id = require_path_param(path_params, "integrationId")?;
            let input = DeleteIntegrationInput {
                api_id: api_id.to_owned(),
                integration_id: integration_id.to_owned(),
            };
            provider
                .delete_integration(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_empty_response(204)
        }
        ApiGatewayV2Operation::GetIntegrations => {
            let api_id = require_path_param(path_params, "apiId")?;
            let input = GetIntegrationsInput {
                api_id: api_id.to_owned(),
                max_results: get_query_param(&query_params, "maxResults").map(str::to_owned),
                next_token: get_query_param(&query_params, "nextToken").map(str::to_owned),
            };
            let output = provider
                .get_integrations(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }

        // ---- Stage CRUD ----
        ApiGatewayV2Operation::CreateStage => {
            let api_id = require_path_param(path_params, "apiId")?;
            let mut input: CreateStageInput = parse_body(body)?;
            input.api_id = api_id.to_owned();
            let output = provider
                .create_stage(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(201, &output)
        }
        ApiGatewayV2Operation::GetStage => {
            let api_id = require_path_param(path_params, "apiId")?;
            let stage_name = require_path_param(path_params, "stageName")?;
            let input = GetStageInput {
                api_id: api_id.to_owned(),
                stage_name: stage_name.to_owned(),
            };
            let output = provider.get_stage(input).map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }
        ApiGatewayV2Operation::UpdateStage => {
            let api_id = require_path_param(path_params, "apiId")?;
            let stage_name = require_path_param(path_params, "stageName")?;
            let mut input: UpdateStageInput = parse_body(body)?;
            input.api_id = api_id.to_owned();
            input.stage_name = stage_name.to_owned();
            let output = provider
                .update_stage(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }
        ApiGatewayV2Operation::DeleteStage => {
            let api_id = require_path_param(path_params, "apiId")?;
            let stage_name = require_path_param(path_params, "stageName")?;
            let input = DeleteStageInput {
                api_id: api_id.to_owned(),
                stage_name: stage_name.to_owned(),
            };
            provider
                .delete_stage(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_empty_response(204)
        }
        ApiGatewayV2Operation::GetStages => {
            let api_id = require_path_param(path_params, "apiId")?;
            let input = GetStagesInput {
                api_id: api_id.to_owned(),
                max_results: get_query_param(&query_params, "maxResults").map(str::to_owned),
                next_token: get_query_param(&query_params, "nextToken").map(str::to_owned),
            };
            let output = provider
                .get_stages(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }

        // ---- Deployment CRUD ----
        ApiGatewayV2Operation::CreateDeployment => {
            let api_id = require_path_param(path_params, "apiId")?;
            let mut input: CreateDeploymentInput = parse_body_or_default(body);
            input.api_id = api_id.to_owned();
            let output = provider
                .create_deployment(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(201, &output)
        }
        ApiGatewayV2Operation::GetDeployment => {
            let api_id = require_path_param(path_params, "apiId")?;
            let deployment_id = require_path_param(path_params, "deploymentId")?;
            let input = GetDeploymentInput {
                api_id: api_id.to_owned(),
                deployment_id: deployment_id.to_owned(),
            };
            let output = provider
                .get_deployment(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }
        ApiGatewayV2Operation::DeleteDeployment => {
            let api_id = require_path_param(path_params, "apiId")?;
            let deployment_id = require_path_param(path_params, "deploymentId")?;
            let input = DeleteDeploymentInput {
                api_id: api_id.to_owned(),
                deployment_id: deployment_id.to_owned(),
            };
            provider
                .delete_deployment(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_empty_response(204)
        }
        ApiGatewayV2Operation::GetDeployments => {
            let api_id = require_path_param(path_params, "apiId")?;
            let input = GetDeploymentsInput {
                api_id: api_id.to_owned(),
                max_results: get_query_param(&query_params, "maxResults").map(str::to_owned),
                next_token: get_query_param(&query_params, "nextToken").map(str::to_owned),
            };
            let output = provider
                .get_deployments(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }

        // ---- Route response CRUD ----
        ApiGatewayV2Operation::CreateRouteResponse => {
            let api_id = require_path_param(path_params, "apiId")?;
            let route_id = require_path_param(path_params, "routeId")?;
            let mut input: CreateRouteResponseInput = parse_body(body)?;
            input.api_id = api_id.to_owned();
            input.route_id = route_id.to_owned();
            let output = provider
                .create_route_response(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(201, &output)
        }
        ApiGatewayV2Operation::GetRouteResponse => {
            let api_id = require_path_param(path_params, "apiId")?;
            let route_id = require_path_param(path_params, "routeId")?;
            let rr_id = require_path_param(path_params, "routeResponseId")?;
            let input = GetRouteResponseInput {
                api_id: api_id.to_owned(),
                route_id: route_id.to_owned(),
                route_response_id: rr_id.to_owned(),
            };
            let output = provider
                .get_route_response(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }
        ApiGatewayV2Operation::DeleteRouteResponse => {
            let api_id = require_path_param(path_params, "apiId")?;
            let route_id = require_path_param(path_params, "routeId")?;
            let rr_id = require_path_param(path_params, "routeResponseId")?;
            let input = DeleteRouteResponseInput {
                api_id: api_id.to_owned(),
                route_id: route_id.to_owned(),
                route_response_id: rr_id.to_owned(),
            };
            provider
                .delete_route_response(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_empty_response(204)
        }
        ApiGatewayV2Operation::GetRouteResponses => {
            let api_id = require_path_param(path_params, "apiId")?;
            let route_id = require_path_param(path_params, "routeId")?;
            let input = GetRouteResponsesInput {
                api_id: api_id.to_owned(),
                route_id: route_id.to_owned(),
                max_results: get_query_param(&query_params, "maxResults").map(str::to_owned),
                next_token: get_query_param(&query_params, "nextToken").map(str::to_owned),
            };
            let output = provider
                .get_route_responses(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }

        // ---- Authorizer CRUD ----
        ApiGatewayV2Operation::CreateAuthorizer => {
            let api_id = require_path_param(path_params, "apiId")?;
            let mut input: CreateAuthorizerInput = parse_body(body)?;
            input.api_id = api_id.to_owned();
            let output = provider
                .create_authorizer(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(201, &output)
        }
        ApiGatewayV2Operation::GetAuthorizer => {
            let api_id = require_path_param(path_params, "apiId")?;
            let authorizer_id = require_path_param(path_params, "authorizerId")?;
            let input = GetAuthorizerInput {
                api_id: api_id.to_owned(),
                authorizer_id: authorizer_id.to_owned(),
            };
            let output = provider
                .get_authorizer(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }
        ApiGatewayV2Operation::UpdateAuthorizer => {
            let api_id = require_path_param(path_params, "apiId")?;
            let authorizer_id = require_path_param(path_params, "authorizerId")?;
            let mut input: UpdateAuthorizerInput = parse_body(body)?;
            input.api_id = api_id.to_owned();
            input.authorizer_id = authorizer_id.to_owned();
            let output = provider
                .update_authorizer(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }
        ApiGatewayV2Operation::DeleteAuthorizer => {
            let api_id = require_path_param(path_params, "apiId")?;
            let authorizer_id = require_path_param(path_params, "authorizerId")?;
            let input = DeleteAuthorizerInput {
                api_id: api_id.to_owned(),
                authorizer_id: authorizer_id.to_owned(),
            };
            provider
                .delete_authorizer(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_empty_response(204)
        }
        ApiGatewayV2Operation::GetAuthorizers => {
            let api_id = require_path_param(path_params, "apiId")?;
            let input = GetAuthorizersInput {
                api_id: api_id.to_owned(),
                max_results: get_query_param(&query_params, "maxResults").map(str::to_owned),
                next_token: get_query_param(&query_params, "nextToken").map(str::to_owned),
            };
            let output = provider
                .get_authorizers(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }

        // ---- Model CRUD ----
        ApiGatewayV2Operation::CreateModel => {
            let api_id = require_path_param(path_params, "apiId")?;
            let mut input: CreateModelInput = parse_body(body)?;
            input.api_id = api_id.to_owned();
            let output = provider
                .create_model(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(201, &output)
        }
        ApiGatewayV2Operation::GetModel => {
            let api_id = require_path_param(path_params, "apiId")?;
            let model_id = require_path_param(path_params, "modelId")?;
            let input = GetModelInput {
                api_id: api_id.to_owned(),
                model_id: model_id.to_owned(),
            };
            let output = provider.get_model(input).map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }
        ApiGatewayV2Operation::UpdateModel => {
            let api_id = require_path_param(path_params, "apiId")?;
            let model_id = require_path_param(path_params, "modelId")?;
            let mut input: UpdateModelInput = parse_body(body)?;
            input.api_id = api_id.to_owned();
            input.model_id = model_id.to_owned();
            let output = provider
                .update_model(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }
        ApiGatewayV2Operation::DeleteModel => {
            let api_id = require_path_param(path_params, "apiId")?;
            let model_id = require_path_param(path_params, "modelId")?;
            let input = DeleteModelInput {
                api_id: api_id.to_owned(),
                model_id: model_id.to_owned(),
            };
            provider
                .delete_model(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_empty_response(204)
        }
        ApiGatewayV2Operation::GetModels => {
            let api_id = require_path_param(path_params, "apiId")?;
            let input = GetModelsInput {
                api_id: api_id.to_owned(),
                max_results: get_query_param(&query_params, "maxResults").map(str::to_owned),
                next_token: get_query_param(&query_params, "nextToken").map(str::to_owned),
            };
            let output = provider
                .get_models(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }
        ApiGatewayV2Operation::GetModelTemplate => {
            let api_id = require_path_param(path_params, "apiId")?;
            let model_id = require_path_param(path_params, "modelId")?;
            let input = GetModelTemplateInput {
                api_id: api_id.to_owned(),
                model_id: model_id.to_owned(),
            };
            let output = provider
                .get_model_template(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }

        // ---- Domain name CRUD ----
        ApiGatewayV2Operation::CreateDomainName => {
            let input: CreateDomainNameInput = parse_body(body)?;
            let output = provider
                .create_domain_name(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(201, &output)
        }
        ApiGatewayV2Operation::GetDomainName => {
            let domain_name = require_path_param(path_params, "domainName")?;
            let input = GetDomainNameInput {
                domain_name: domain_name.to_owned(),
            };
            let output = provider
                .get_domain_name(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }
        ApiGatewayV2Operation::UpdateDomainName => {
            let domain_name = require_path_param(path_params, "domainName")?;
            let mut input: UpdateDomainNameInput = parse_body(body)?;
            input.domain_name = domain_name.to_owned();
            let output = provider
                .update_domain_name(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }
        ApiGatewayV2Operation::DeleteDomainName => {
            let domain_name = require_path_param(path_params, "domainName")?;
            let input = DeleteDomainNameInput {
                domain_name: domain_name.to_owned(),
            };
            provider
                .delete_domain_name(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_empty_response(204)
        }
        ApiGatewayV2Operation::GetDomainNames => {
            let input = GetDomainNamesInput {
                max_results: get_query_param(&query_params, "maxResults").map(str::to_owned),
                next_token: get_query_param(&query_params, "nextToken").map(str::to_owned),
            };
            let output = provider
                .get_domain_names(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }

        // ---- VPC link CRUD ----
        ApiGatewayV2Operation::CreateVpcLink => {
            let input: CreateVpcLinkInput = parse_body(body)?;
            let output = provider
                .create_vpc_link(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(201, &output)
        }
        ApiGatewayV2Operation::GetVpcLink => {
            let vpc_link_id = require_path_param(path_params, "vpcLinkId")?;
            let input = GetVpcLinkInput {
                vpc_link_id: vpc_link_id.to_owned(),
            };
            let output = provider
                .get_vpc_link(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }
        ApiGatewayV2Operation::UpdateVpcLink => {
            let vpc_link_id = require_path_param(path_params, "vpcLinkId")?;
            let mut input: UpdateVpcLinkInput = parse_body(body)?;
            input.vpc_link_id = vpc_link_id.to_owned();
            let output = provider
                .update_vpc_link(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }
        ApiGatewayV2Operation::DeleteVpcLink => {
            let vpc_link_id = require_path_param(path_params, "vpcLinkId")?;
            let input = DeleteVpcLinkInput {
                vpc_link_id: vpc_link_id.to_owned(),
            };
            let _output = provider
                .delete_vpc_link(input)
                .map_err(ApiGatewayV2Error::from)?;
            // DeleteVpcLink returns 202 Accepted
            wrap_empty_response(202)
        }
        ApiGatewayV2Operation::GetVpcLinks => {
            let input = GetVpcLinksInput {
                max_results: get_query_param(&query_params, "maxResults").map(str::to_owned),
                next_token: get_query_param(&query_params, "nextToken").map(str::to_owned),
            };
            let output = provider
                .get_vpc_links(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }

        // ---- Tags ----
        ApiGatewayV2Operation::TagResource => {
            // Path param is "resource-arn" (with hyphen)
            let resource_arn = require_path_param(path_params, "resource-arn")?;
            let mut input: TagResourceInput = parse_body(body)?;
            input.resource_arn = resource_arn.to_owned();
            let _output = provider
                .tag_resource(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_empty_response(201)
        }
        ApiGatewayV2Operation::UntagResource => {
            let resource_arn = require_path_param(path_params, "resource-arn")?;
            // tagKeys come as array query params: tagKeys=key1&tagKeys=key2
            let tag_keys: Vec<String> = query_params
                .iter()
                .filter(|(k, _)| k == "tagKeys")
                .map(|(_, v)| v.clone())
                .collect();
            let input = UntagResourceInput {
                resource_arn: resource_arn.to_owned(),
                tag_keys,
            };
            provider
                .untag_resource(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_empty_response(204)
        }
        ApiGatewayV2Operation::GetTags => {
            let resource_arn = require_path_param(path_params, "resource-arn")?;
            let input = GetTagsInput {
                resource_arn: resource_arn.to_owned(),
            };
            let output = provider.get_tags(input).map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }

        // ---- API Mapping CRUD ----
        ApiGatewayV2Operation::CreateApiMapping => {
            let domain_name = require_path_param(path_params, "domainName")?;
            let mut input: CreateApiMappingInput = parse_body(body)?;
            input.domain_name = domain_name.to_owned();
            let output = provider
                .create_api_mapping(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(201, &output)
        }
        ApiGatewayV2Operation::GetApiMapping => {
            let domain_name = require_path_param(path_params, "domainName")?;
            let api_mapping_id = require_path_param(path_params, "apiMappingId")?;
            let input = GetApiMappingInput {
                domain_name: domain_name.to_owned(),
                api_mapping_id: api_mapping_id.to_owned(),
            };
            let output = provider
                .get_api_mapping(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }
        ApiGatewayV2Operation::UpdateApiMapping => {
            let domain_name = require_path_param(path_params, "domainName")?;
            let api_mapping_id = require_path_param(path_params, "apiMappingId")?;
            let mut input: UpdateApiMappingInput = parse_body(body)?;
            input.domain_name = domain_name.to_owned();
            input.api_mapping_id = api_mapping_id.to_owned();
            let output = provider
                .update_api_mapping(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }
        ApiGatewayV2Operation::DeleteApiMapping => {
            let domain_name = require_path_param(path_params, "domainName")?;
            let api_mapping_id = require_path_param(path_params, "apiMappingId")?;
            let input = DeleteApiMappingInput {
                domain_name: domain_name.to_owned(),
                api_mapping_id: api_mapping_id.to_owned(),
            };
            provider
                .delete_api_mapping(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_empty_response(204)
        }
        ApiGatewayV2Operation::GetApiMappings => {
            let domain_name = require_path_param(path_params, "domainName")?;
            let input = GetApiMappingsInput {
                domain_name: domain_name.to_owned(),
                max_results: get_query_param(&query_params, "maxResults").map(str::to_owned),
                next_token: get_query_param(&query_params, "nextToken").map(str::to_owned),
            };
            let output = provider
                .get_api_mappings(input)
                .map_err(ApiGatewayV2Error::from)?;
            wrap_json_response(200, &output)
        }
    }
}

/// Parse a JSON body into the specified type.
fn parse_body<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, ApiGatewayV2Error> {
    serde_json::from_slice(body).map_err(|e| {
        ApiGatewayV2Error::with_message(
            ruststack_apigatewayv2_model::error::ApiGatewayV2ErrorCode::BadRequestException,
            format!("Invalid request body: {e}"),
        )
    })
}

/// Parse a JSON body or return default if body is empty.
fn parse_body_or_default<T: serde::de::DeserializeOwned + Default>(body: &[u8]) -> T {
    if body.is_empty() {
        T::default()
    } else {
        serde_json::from_slice(body).unwrap_or_default()
    }
}
