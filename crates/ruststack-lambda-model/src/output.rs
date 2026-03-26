//! Lambda operation output types.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::types::{
    AliasConfiguration, EventSourceMappingConfiguration, FunctionCodeLocation,
    FunctionConfiguration, FunctionEventInvokeConfig, FunctionUrlConfig, LayerVersionContentOutput,
    LayerVersionsListItem, LayersListItem,
};

/// Output for `GetFunction`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetFunctionOutput {
    /// Function configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configuration: Option<FunctionConfiguration>,
    /// Code location.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<FunctionCodeLocation>,
    /// Tags.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<HashMap<String, String>>,
}

/// Output for `ListFunctions`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListFunctionsOutput {
    /// List of function configurations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub functions: Option<Vec<FunctionConfiguration>>,
    /// Next pagination marker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_marker: Option<String>,
}

/// Output for `ListVersionsByFunction`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListVersionsOutput {
    /// List of version configurations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub versions: Option<Vec<FunctionConfiguration>>,
    /// Next pagination marker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_marker: Option<String>,
}

/// Output for `ListAliases`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListAliasesOutput {
    /// List of alias configurations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aliases: Option<Vec<AliasConfiguration>>,
    /// Next pagination marker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_marker: Option<String>,
}

/// Output for `GetPolicy`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetPolicyOutput {
    /// JSON policy document.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    /// Revision ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
}

/// Output for `AddPermission`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AddPermissionOutput {
    /// JSON statement.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub statement: Option<String>,
}

/// Output for `ListTags`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListTagsOutput {
    /// Tags.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<HashMap<String, String>>,
}

/// Output for `GetAccountSettings`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetAccountSettingsOutput {
    /// Account limit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_limit: Option<AccountLimit>,
    /// Account usage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_usage: Option<AccountUsage>,
}

/// Account limits.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AccountLimit {
    /// Total code size limit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_code_size: Option<i64>,
    /// Code size unzipped limit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_size_unzipped: Option<i64>,
    /// Code size zipped limit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_size_zipped: Option<i64>,
    /// Concurrent executions limit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concurrent_executions: Option<i32>,
    /// Unreserved concurrent executions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unreserved_concurrent_executions: Option<i32>,
}

/// Account usage.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AccountUsage {
    /// Total code size in bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_code_size: Option<i64>,
    /// Number of functions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_count: Option<i64>,
}

/// Output for `ListFunctionUrlConfigs`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListFunctionUrlConfigsOutput {
    /// List of function URL configs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_url_configs: Option<Vec<FunctionUrlConfig>>,
    /// Next pagination marker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_marker: Option<String>,
}

/// Output for `PublishLayerVersion` and `GetLayerVersion`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PublishLayerVersionOutput {
    /// Layer version content metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<LayerVersionContentOutput>,
    /// Layer ARN (without version).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer_arn: Option<String>,
    /// Layer version ARN.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer_version_arn: Option<String>,
    /// Description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// ISO 8601 creation date.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_date: Option<String>,
    /// Version number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<i64>,
    /// Compatible runtimes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatible_runtimes: Option<Vec<String>>,
    /// License info.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license_info: Option<String>,
    /// Compatible architectures.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatible_architectures: Option<Vec<String>>,
}

/// Output for `ListLayerVersions`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListLayerVersionsOutput {
    /// Layer versions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer_versions: Option<Vec<LayerVersionsListItem>>,
    /// Next pagination marker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_marker: Option<String>,
}

/// Output for `ListLayers`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListLayersOutput {
    /// Layers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layers: Option<Vec<LayersListItem>>,
    /// Next pagination marker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_marker: Option<String>,
}

/// Output for `GetLayerVersionPolicy`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetLayerVersionPolicyOutput {
    /// JSON policy document.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    /// Revision ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
}

/// Output for `ListEventSourceMappings`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListEventSourceMappingsOutput {
    /// List of event source mapping configurations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_source_mappings: Option<Vec<EventSourceMappingConfiguration>>,
    /// Next pagination marker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_marker: Option<String>,
}

/// Output for `AddLayerVersionPermission`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AddLayerVersionPermissionOutput {
    /// JSON policy statement.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub statement: Option<String>,
    /// Revision ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
}

/// Output for `ListFunctionEventInvokeConfigs`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListFunctionEventInvokeConfigsOutput {
    /// List of event invoke configs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_event_invoke_configs: Option<Vec<FunctionEventInvokeConfig>>,
    /// Next pagination marker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_marker: Option<String>,
}
