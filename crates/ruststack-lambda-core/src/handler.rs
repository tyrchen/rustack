//! Lambda handler implementation bridging HTTP to business logic.
//!
//! Parses JSON request bodies, dispatches to the provider, and serializes
//! JSON responses following the `restJson1` protocol.
//!
//! Uses manual `Pin<Box<dyn Future>>` return types because the `LambdaHandler`
//! trait requires object safety for `Arc<dyn LambdaHandler>`.

use std::{future::Future, pin::Pin, sync::Arc};

use bytes::Bytes;
use ruststack_lambda_http::{
    body::LambdaResponseBody,
    dispatch::LambdaHandler,
    response::{empty_response, json_response},
    router::PathParams,
};
use ruststack_lambda_model::{
    error::LambdaError,
    input::{
        AddLayerVersionPermissionInput, AddPermissionInput, CreateAliasInput,
        CreateEventSourceMappingInput, CreateFunctionInput, CreateFunctionUrlConfigInput,
        EventInvokeConfigInput, PublishLayerVersionInput, PublishVersionInput,
        PutFunctionConcurrencyInput, TagResourceInput, UpdateAliasInput,
        UpdateEventSourceMappingInput, UpdateFunctionCodeInput, UpdateFunctionConfigurationInput,
        UpdateFunctionUrlConfigInput,
    },
    operations::LambdaOperation,
    output::ListFunctionEventInvokeConfigsOutput,
    types::InvocationType,
};

use crate::provider::RustStackLambda;

/// Handler that bridges the HTTP layer to the Lambda provider.
#[derive(Debug)]
pub struct RustStackLambdaHandler {
    provider: Arc<RustStackLambda>,
}

impl RustStackLambdaHandler {
    /// Create a new handler wrapping a provider.
    #[must_use]
    pub fn new(provider: Arc<RustStackLambda>) -> Self {
        Self { provider }
    }
}

impl LambdaHandler for RustStackLambdaHandler {
    fn handle_operation(
        &self,
        op: LambdaOperation,
        path_params: PathParams,
        query: String,
        headers: http::HeaderMap,
        body: Bytes,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<LambdaResponseBody>, LambdaError>> + Send>>
    {
        let provider = Arc::clone(&self.provider);
        Box::pin(
            async move { dispatch(&provider, op, &path_params, &query, &headers, &body).await },
        )
    }
}

/// Parse query parameters from a URL query string.
///
/// Splits on `&` and `=` to extract key-value pairs. Handles percent-encoded
/// values by decoding `%XX` sequences.
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

/// Dispatch a Lambda operation to the appropriate provider method.
#[allow(clippy::too_many_lines)]
async fn dispatch(
    provider: &RustStackLambda,
    op: LambdaOperation,
    path_params: &PathParams,
    query: &str,
    headers: &http::HeaderMap,
    body: &[u8],
) -> Result<http::Response<LambdaResponseBody>, LambdaError> {
    let query_params = parse_query_params(query);
    let qualifier = get_query_param(&query_params, "Qualifier");

    match op {
        // ---- Phase 0: Function CRUD ----
        LambdaOperation::CreateFunction => {
            let input: CreateFunctionInput = serde_json::from_slice(body).map_err(|e| {
                LambdaError::invalid_parameter(format!("Invalid request body: {e}"))
            })?;
            let config = provider
                .create_function(input)
                .await
                .map_err(LambdaError::from)?;
            wrap_json_response(201, &config)
        }

        LambdaOperation::GetFunction => {
            let function_name = require_path_param(path_params, "FunctionName")?;
            let output = provider
                .get_function(function_name, qualifier)
                .map_err(LambdaError::from)?;
            wrap_json_response(200, &output)
        }

        LambdaOperation::GetFunctionConfiguration => {
            let function_name = require_path_param(path_params, "FunctionName")?;
            let config = provider
                .get_function_configuration(function_name, qualifier)
                .map_err(LambdaError::from)?;
            wrap_json_response(200, &config)
        }

        LambdaOperation::UpdateFunctionCode => {
            let function_name = require_path_param(path_params, "FunctionName")?;
            let input: UpdateFunctionCodeInput = serde_json::from_slice(body).map_err(|e| {
                LambdaError::invalid_parameter(format!("Invalid request body: {e}"))
            })?;
            let config = provider
                .update_function_code(function_name, input)
                .await
                .map_err(LambdaError::from)?;
            wrap_json_response(200, &config)
        }

        LambdaOperation::UpdateFunctionConfiguration => {
            let function_name = require_path_param(path_params, "FunctionName")?;
            let input: UpdateFunctionConfigurationInput =
                serde_json::from_slice(body).map_err(|e| {
                    LambdaError::invalid_parameter(format!("Invalid request body: {e}"))
                })?;
            let config = provider
                .update_function_configuration(function_name, &input)
                .map_err(LambdaError::from)?;
            wrap_json_response(200, &config)
        }

        LambdaOperation::DeleteFunction => {
            let function_name = require_path_param(path_params, "FunctionName")?;
            provider
                .delete_function(function_name, qualifier)
                .await
                .map_err(LambdaError::from)?;
            wrap_empty_response(204)
        }

        LambdaOperation::ListFunctions => {
            let marker = get_query_param(&query_params, "Marker");
            let max_items =
                get_query_param(&query_params, "MaxItems").and_then(|v| v.parse::<usize>().ok());
            let output = provider.list_functions(marker, max_items);
            wrap_json_response(200, &output)
        }

        // ---- Phase 0: Invoke ----
        LambdaOperation::Invoke => {
            let function_name = require_path_param(path_params, "FunctionName")?;

            // Parse InvocationType from header.
            let invocation_type = headers
                .get("x-amz-invocation-type")
                .and_then(|v| v.to_str().ok())
                .and_then(InvocationType::from_str_value)
                .unwrap_or(InvocationType::RequestResponse);

            if invocation_type == InvocationType::DryRun {
                // DryRun: validate function exists, return 204.
                let (status, _) = provider
                    .invoke(function_name, qualifier, body, true)
                    .map_err(LambdaError::from)?;
                return wrap_empty_response(status);
            }

            // For Event invocation, validate and return 202 immediately.
            // Actual async execution would be queued (not yet implemented).
            if invocation_type == InvocationType::Event {
                // Validate the function exists and qualifier resolves.
                provider
                    .invoke(function_name, qualifier, body, true)
                    .map_err(LambdaError::from)?;
                return wrap_empty_response(202);
            }

            // RequestResponse: synchronous invocation.
            let (status, response_body) = provider
                .invoke(function_name, qualifier, body, false)
                .map_err(LambdaError::from)?;

            // Build response with proper invoke headers.
            let mut response = http::Response::builder()
                .status(status)
                .body(LambdaResponseBody::from_bytes(response_body))
                .map_err(|e| {
                    LambdaError::service_error(format!("Failed to build invoke response: {e}"))
                })?;

            // Set X-Amz-Executed-Version header.
            if let Ok(hv) = http::HeaderValue::from_str(qualifier.unwrap_or("$LATEST")) {
                response.headers_mut().insert("x-amz-executed-version", hv);
            }

            Ok(response)
        }

        // ---- Phase 1: Versions + Aliases ----
        LambdaOperation::PublishVersion => {
            let function_name = require_path_param(path_params, "FunctionName")?;
            let input: PublishVersionInput = if body.is_empty() {
                PublishVersionInput::default()
            } else {
                serde_json::from_slice(body).map_err(|e| {
                    LambdaError::invalid_parameter(format!("Invalid request body: {e}"))
                })?
            };
            let config = provider
                .publish_version(function_name, &input)
                .map_err(LambdaError::from)?;
            wrap_json_response(201, &config)
        }

        LambdaOperation::ListVersionsByFunction => {
            let function_name = require_path_param(path_params, "FunctionName")?;
            let marker = get_query_param(&query_params, "Marker");
            let max_items =
                get_query_param(&query_params, "MaxItems").and_then(|v| v.parse::<usize>().ok());
            let output = provider
                .list_versions_by_function(function_name, marker, max_items)
                .map_err(LambdaError::from)?;
            wrap_json_response(200, &output)
        }

        LambdaOperation::CreateAlias => {
            let function_name = require_path_param(path_params, "FunctionName")?;
            let input: CreateAliasInput = serde_json::from_slice(body).map_err(|e| {
                LambdaError::invalid_parameter(format!("Invalid request body: {e}"))
            })?;
            let config = provider
                .create_alias(function_name, input)
                .map_err(LambdaError::from)?;
            wrap_json_response(201, &config)
        }

        LambdaOperation::GetAlias => {
            let function_name = require_path_param(path_params, "FunctionName")?;
            let alias_name = require_path_param(path_params, "Name")?;
            let config = provider
                .get_alias(function_name, alias_name)
                .map_err(LambdaError::from)?;
            wrap_json_response(200, &config)
        }

        LambdaOperation::UpdateAlias => {
            let function_name = require_path_param(path_params, "FunctionName")?;
            let alias_name = require_path_param(path_params, "Name")?;
            let input: UpdateAliasInput = serde_json::from_slice(body).map_err(|e| {
                LambdaError::invalid_parameter(format!("Invalid request body: {e}"))
            })?;
            let config = provider
                .update_alias(function_name, alias_name, &input)
                .map_err(LambdaError::from)?;
            wrap_json_response(200, &config)
        }

        LambdaOperation::DeleteAlias => {
            let function_name = require_path_param(path_params, "FunctionName")?;
            let alias_name = require_path_param(path_params, "Name")?;
            provider
                .delete_alias(function_name, alias_name)
                .map_err(LambdaError::from)?;
            wrap_empty_response(204)
        }

        LambdaOperation::ListAliases => {
            let function_name = require_path_param(path_params, "FunctionName")?;
            let marker = get_query_param(&query_params, "Marker");
            let max_items =
                get_query_param(&query_params, "MaxItems").and_then(|v| v.parse::<usize>().ok());
            let output = provider
                .list_aliases(function_name, marker, max_items)
                .map_err(LambdaError::from)?;
            wrap_json_response(200, &output)
        }

        // ---- Phase 2: Permissions + Tags + Account ----
        LambdaOperation::AddPermission => {
            let function_name = require_path_param(path_params, "FunctionName")?;
            let input: AddPermissionInput = serde_json::from_slice(body).map_err(|e| {
                LambdaError::invalid_parameter(format!("Invalid request body: {e}"))
            })?;
            let output = provider
                .add_permission(function_name, qualifier, &input)
                .map_err(LambdaError::from)?;
            wrap_json_response(201, &output)
        }

        LambdaOperation::RemovePermission => {
            let function_name = require_path_param(path_params, "FunctionName")?;
            let statement_id = require_path_param(path_params, "StatementId")?;
            provider
                .remove_permission(function_name, statement_id, qualifier)
                .map_err(LambdaError::from)?;
            wrap_empty_response(204)
        }

        LambdaOperation::GetPolicy => {
            let function_name = require_path_param(path_params, "FunctionName")?;
            let output = provider
                .get_policy(function_name, qualifier)
                .map_err(LambdaError::from)?;
            wrap_json_response(200, &output)
        }

        LambdaOperation::TagResource => {
            let resource = require_path_param(path_params, "Resource")?;
            let input: TagResourceInput = serde_json::from_slice(body).map_err(|e| {
                LambdaError::invalid_parameter(format!("Invalid request body: {e}"))
            })?;
            provider
                .tag_resource(resource, &input)
                .map_err(LambdaError::from)?;
            wrap_empty_response(204)
        }

        LambdaOperation::UntagResource => {
            let resource = require_path_param(path_params, "Resource")?;
            let tag_keys: Vec<String> = query_params
                .iter()
                .filter(|(k, _)| k == "tagKeys")
                .map(|(_, v)| v.clone())
                .collect();
            provider
                .untag_resource(resource, &tag_keys)
                .map_err(LambdaError::from)?;
            wrap_empty_response(204)
        }

        LambdaOperation::ListTags => {
            let resource = require_path_param(path_params, "Resource")?;
            let output = provider.list_tags(resource).map_err(LambdaError::from)?;
            wrap_json_response(200, &output)
        }

        LambdaOperation::GetAccountSettings => {
            let output = provider.get_account_settings();
            wrap_json_response(200, &output)
        }

        // ---- Phase 3: Function URLs ----
        LambdaOperation::CreateFunctionUrlConfig => {
            let function_name = require_path_param(path_params, "FunctionName")?;
            let input: CreateFunctionUrlConfigInput =
                serde_json::from_slice(body).map_err(|e| {
                    LambdaError::invalid_parameter(format!("Invalid request body: {e}"))
                })?;
            let output = provider
                .create_function_url_config(function_name, qualifier, input)
                .map_err(LambdaError::from)?;
            wrap_json_response(201, &output)
        }

        LambdaOperation::GetFunctionUrlConfig => {
            let function_name = require_path_param(path_params, "FunctionName")?;
            let output = provider
                .get_function_url_config(function_name, qualifier)
                .map_err(LambdaError::from)?;
            wrap_json_response(200, &output)
        }

        LambdaOperation::UpdateFunctionUrlConfig => {
            let function_name = require_path_param(path_params, "FunctionName")?;
            let input: UpdateFunctionUrlConfigInput =
                serde_json::from_slice(body).map_err(|e| {
                    LambdaError::invalid_parameter(format!("Invalid request body: {e}"))
                })?;
            let output = provider
                .update_function_url_config(function_name, qualifier, &input)
                .map_err(LambdaError::from)?;
            wrap_json_response(200, &output)
        }

        LambdaOperation::DeleteFunctionUrlConfig => {
            let function_name = require_path_param(path_params, "FunctionName")?;
            provider
                .delete_function_url_config(function_name, qualifier)
                .map_err(LambdaError::from)?;
            wrap_empty_response(204)
        }

        LambdaOperation::ListFunctionUrlConfigs => {
            let function_name = require_path_param(path_params, "FunctionName")?;
            let output = provider
                .list_function_url_configs(function_name)
                .map_err(LambdaError::from)?;
            wrap_json_response(200, &output)
        }

        // ---- Phase 2b: Lambda Layers ----
        LambdaOperation::PublishLayerVersion => {
            let layer_name = require_path_param(path_params, "LayerName")?;
            let input: PublishLayerVersionInput = serde_json::from_slice(body).map_err(|e| {
                LambdaError::invalid_parameter(format!("Invalid request body: {e}"))
            })?;
            let output = provider
                .publish_layer_version(layer_name, &input)
                .map_err(LambdaError::from)?;
            wrap_json_response(201, &output)
        }

        LambdaOperation::GetLayerVersion => {
            let layer_name = require_path_param(path_params, "LayerName")?;
            let version_number = require_path_param(path_params, "VersionNumber")?;
            let version: u64 = version_number.parse().map_err(|_| {
                LambdaError::invalid_parameter(format!("Invalid version number: {version_number}"))
            })?;
            let output = provider
                .get_layer_version(layer_name, version)
                .map_err(LambdaError::from)?;
            wrap_json_response(200, &output)
        }

        LambdaOperation::GetLayerVersionByArn => {
            // The ARN is passed as a query parameter.
            let arn = get_query_param(&query_params, "Arn")
                .ok_or_else(|| LambdaError::invalid_parameter("Arn query parameter is required"))?;
            let output = provider
                .get_layer_version_by_arn(arn)
                .map_err(LambdaError::from)?;
            wrap_json_response(200, &output)
        }

        LambdaOperation::ListLayerVersions => {
            let layer_name = require_path_param(path_params, "LayerName")?;
            let marker = get_query_param(&query_params, "Marker");
            let max_items =
                get_query_param(&query_params, "MaxItems").and_then(|v| v.parse::<usize>().ok());
            let output = provider
                .list_layer_versions(layer_name, marker, max_items)
                .map_err(LambdaError::from)?;
            wrap_json_response(200, &output)
        }

        LambdaOperation::ListLayers => {
            let marker = get_query_param(&query_params, "Marker");
            let max_items =
                get_query_param(&query_params, "MaxItems").and_then(|v| v.parse::<usize>().ok());
            let output = provider.list_layers(marker, max_items);
            wrap_json_response(200, &output)
        }

        LambdaOperation::DeleteLayerVersion => {
            let layer_name = require_path_param(path_params, "LayerName")?;
            let version_number = require_path_param(path_params, "VersionNumber")?;
            let version: u64 = version_number.parse().map_err(|_| {
                LambdaError::invalid_parameter(format!("Invalid version number: {version_number}"))
            })?;
            provider
                .delete_layer_version(layer_name, version)
                .map_err(LambdaError::from)?;
            wrap_empty_response(204)
        }

        LambdaOperation::AddLayerVersionPermission => {
            let layer_name = require_path_param(path_params, "LayerName")?;
            let version_number = require_path_param(path_params, "VersionNumber")?;
            let version: u64 = version_number.parse().map_err(|_| {
                LambdaError::invalid_parameter(format!("Invalid version number: {version_number}"))
            })?;
            let input: AddLayerVersionPermissionInput =
                serde_json::from_slice(body).map_err(|e| {
                    LambdaError::invalid_parameter(format!("Invalid request body: {e}"))
                })?;
            let output = provider
                .add_layer_version_permission(layer_name, version, &input)
                .map_err(LambdaError::from)?;
            wrap_json_response(201, &output)
        }

        LambdaOperation::GetLayerVersionPolicy => {
            let layer_name = require_path_param(path_params, "LayerName")?;
            let version_number = require_path_param(path_params, "VersionNumber")?;
            let version: u64 = version_number.parse().map_err(|_| {
                LambdaError::invalid_parameter(format!("Invalid version number: {version_number}"))
            })?;
            let output = provider
                .get_layer_version_policy(layer_name, version)
                .map_err(LambdaError::from)?;
            wrap_json_response(200, &output)
        }

        LambdaOperation::RemoveLayerVersionPermission => {
            let layer_name = require_path_param(path_params, "LayerName")?;
            let version_number = require_path_param(path_params, "VersionNumber")?;
            let version: u64 = version_number.parse().map_err(|_| {
                LambdaError::invalid_parameter(format!("Invalid version number: {version_number}"))
            })?;
            let statement_id = require_path_param(path_params, "StatementId")?;
            provider
                .remove_layer_version_permission(layer_name, version, statement_id)
                .map_err(LambdaError::from)?;
            wrap_empty_response(204)
        }

        // ---- Phase 3: Event Source Mappings ----
        LambdaOperation::CreateEventSourceMapping => {
            let input: CreateEventSourceMappingInput =
                serde_json::from_slice(body).map_err(|e| {
                    LambdaError::invalid_parameter(format!("Invalid request body: {e}"))
                })?;
            let output = provider
                .create_event_source_mapping(&input)
                .map_err(LambdaError::from)?;
            wrap_json_response(202, &output)
        }

        LambdaOperation::GetEventSourceMapping => {
            let uuid = require_path_param(path_params, "UUID")?;
            let output = provider
                .get_event_source_mapping(uuid)
                .map_err(LambdaError::from)?;
            wrap_json_response(200, &output)
        }

        LambdaOperation::UpdateEventSourceMapping => {
            let uuid = require_path_param(path_params, "UUID")?;
            let input: UpdateEventSourceMappingInput =
                serde_json::from_slice(body).map_err(|e| {
                    LambdaError::invalid_parameter(format!("Invalid request body: {e}"))
                })?;
            let output = provider
                .update_event_source_mapping(uuid, &input)
                .map_err(LambdaError::from)?;
            wrap_json_response(202, &output)
        }

        LambdaOperation::DeleteEventSourceMapping => {
            let uuid = require_path_param(path_params, "UUID")?;
            let output = provider
                .delete_event_source_mapping(uuid)
                .map_err(LambdaError::from)?;
            wrap_json_response(202, &output)
        }

        LambdaOperation::ListEventSourceMappings => {
            let function_name = get_query_param(&query_params, "FunctionName");
            let event_source_arn = get_query_param(&query_params, "EventSourceArn");
            let marker = get_query_param(&query_params, "Marker");
            let max_items =
                get_query_param(&query_params, "MaxItems").and_then(|v| v.parse::<usize>().ok());
            let output = provider.list_event_source_mappings(
                function_name,
                event_source_arn,
                marker,
                max_items,
            );
            wrap_json_response(200, &output)
        }

        // ---- Phase 6: Concurrency ----
        LambdaOperation::PutFunctionConcurrency => {
            let function_name = require_path_param(path_params, "FunctionName")?;
            let input: PutFunctionConcurrencyInput = serde_json::from_slice(body).map_err(|e| {
                LambdaError::invalid_parameter(format!("Invalid request body: {e}"))
            })?;
            let output = provider
                .put_function_concurrency(function_name, input.reserved_concurrent_executions)
                .map_err(LambdaError::from)?;
            wrap_json_response(200, &output)
        }

        LambdaOperation::GetFunctionConcurrency => {
            let function_name = require_path_param(path_params, "FunctionName")?;
            let output = provider
                .get_function_concurrency(function_name)
                .map_err(LambdaError::from)?;
            wrap_json_response(200, &output)
        }

        LambdaOperation::DeleteFunctionConcurrency => {
            let function_name = require_path_param(path_params, "FunctionName")?;
            provider
                .delete_function_concurrency(function_name)
                .map_err(LambdaError::from)?;
            wrap_empty_response(204)
        }

        // ---- Phase 6: Event Invoke Config ----
        LambdaOperation::PutFunctionEventInvokeConfig => {
            let function_name = require_path_param(path_params, "FunctionName")?;
            let input: EventInvokeConfigInput = if body.is_empty() {
                EventInvokeConfigInput::default()
            } else {
                serde_json::from_slice(body).map_err(|e| {
                    LambdaError::invalid_parameter(format!("Invalid request body: {e}"))
                })?
            };
            let output = provider
                .put_function_event_invoke_config(function_name, qualifier, &input)
                .map_err(LambdaError::from)?;
            wrap_json_response(200, &output)
        }

        LambdaOperation::GetFunctionEventInvokeConfig => {
            let function_name = require_path_param(path_params, "FunctionName")?;
            let output = provider
                .get_function_event_invoke_config(function_name, qualifier)
                .map_err(LambdaError::from)?;
            wrap_json_response(200, &output)
        }

        LambdaOperation::UpdateFunctionEventInvokeConfig => {
            let function_name = require_path_param(path_params, "FunctionName")?;
            let input: EventInvokeConfigInput = if body.is_empty() {
                EventInvokeConfigInput::default()
            } else {
                serde_json::from_slice(body).map_err(|e| {
                    LambdaError::invalid_parameter(format!("Invalid request body: {e}"))
                })?
            };
            let output = provider
                .update_function_event_invoke_config(function_name, qualifier, &input)
                .map_err(LambdaError::from)?;
            wrap_json_response(200, &output)
        }

        LambdaOperation::DeleteFunctionEventInvokeConfig => {
            let function_name = require_path_param(path_params, "FunctionName")?;
            provider
                .delete_function_event_invoke_config(function_name, qualifier)
                .map_err(LambdaError::from)?;
            wrap_empty_response(204)
        }

        LambdaOperation::ListFunctionEventInvokeConfigs => {
            let function_name = require_path_param(path_params, "FunctionName")?;
            let configs = provider
                .list_function_event_invoke_configs(function_name)
                .map_err(LambdaError::from)?;
            let output = ListFunctionEventInvokeConfigsOutput {
                function_event_invoke_configs: if configs.is_empty() {
                    None
                } else {
                    Some(configs)
                },
                next_marker: None,
            };
            wrap_json_response(200, &output)
        }

        _ => Err(LambdaError::service_error(format!(
            "Operation {op} is not implemented"
        ))),
    }
}

/// Extract a required path parameter or return an error.
fn require_path_param<'a>(params: &'a PathParams, name: &str) -> Result<&'a str, LambdaError> {
    params
        .get(name)
        .ok_or_else(|| LambdaError::invalid_parameter(format!("Missing path parameter: {name}")))
}

/// Wrap a serializable value into a JSON `LambdaResponseBody` response.
fn wrap_json_response(
    status: u16,
    body: &impl serde::Serialize,
) -> Result<http::Response<LambdaResponseBody>, LambdaError> {
    let bytes_response = json_response(status, body)?;
    let (parts, body) = bytes_response.into_parts();
    Ok(http::Response::from_parts(
        parts,
        LambdaResponseBody::from_bytes(body),
    ))
}

/// Wrap an empty response with the given status code.
fn wrap_empty_response(status: u16) -> Result<http::Response<LambdaResponseBody>, LambdaError> {
    let bytes_response = empty_response(status)?;
    let (parts, body) = bytes_response.into_parts();
    Ok(http::Response::from_parts(
        parts,
        LambdaResponseBody::from_bytes(body),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_parse_empty_query() {
        let params = parse_query_params("");
        assert!(params.is_empty());
    }

    #[test]
    fn test_should_parse_query_with_marker_and_max() {
        let params = parse_query_params("Marker=abc&MaxItems=10");
        assert_eq!(get_query_param(&params, "Marker"), Some("abc"));
        assert_eq!(get_query_param(&params, "MaxItems"), Some("10"));
    }

    #[test]
    fn test_should_parse_qualifier_from_query() {
        let params = parse_query_params("Qualifier=prod");
        assert_eq!(get_query_param(&params, "Qualifier"), Some("prod"));
    }

    #[test]
    fn test_should_return_none_for_missing_param() {
        let params = parse_query_params("Foo=bar");
        assert!(get_query_param(&params, "Missing").is_none());
    }
}
