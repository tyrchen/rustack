//! Lambda operation input types.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::types::{
    AliasRoutingConfiguration, Cors, DeadLetterConfig, DestinationConfig, Environment,
    EphemeralStorage, FunctionCode, ImageConfig, LayerVersionContentInput, LoggingConfig,
    SnapStart, TracingConfig, VpcConfig,
};

/// Input for `CreateFunction`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CreateFunctionInput {
    /// Function name (1-140 characters).
    pub function_name: String,
    /// Runtime identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    /// IAM execution role ARN.
    pub role: String,
    /// Handler function identifier (e.g., `index.handler`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handler: Option<String>,
    /// Deployment package.
    pub code: FunctionCode,
    /// Description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Timeout in seconds (1-900).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u32>,
    /// Memory in MB (128-10240).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_size: Option<u32>,
    /// Whether to publish version 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publish: Option<bool>,
    /// Package type (Zip or Image).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_type: Option<String>,
    /// Environment variables.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<Environment>,
    /// VPC configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vpc_config: Option<VpcConfig>,
    /// Dead letter configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dead_letter_config: Option<DeadLetterConfig>,
    /// Tracing configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracing_config: Option<TracingConfig>,
    /// Tags.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<HashMap<String, String>>,
    /// Layer ARNs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layers: Option<Vec<String>>,
    /// Architectures.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub architectures: Option<Vec<String>>,
    /// Ephemeral storage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ephemeral_storage: Option<EphemeralStorage>,
    /// Image configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_config: Option<ImageConfig>,
    /// Logging configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logging_config: Option<LoggingConfig>,
    /// SnapStart configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snap_start: Option<SnapStart>,
    /// Code signing config ARN (stored, not enforced).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_signing_config_arn: Option<String>,
}

/// Input for `UpdateFunctionCode`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UpdateFunctionCodeInput {
    /// Base64-encoded zip file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zip_file: Option<String>,
    /// S3 bucket (accepted, not functional).
    #[serde(rename = "S3Bucket")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub s3_bucket: Option<String>,
    /// S3 key (accepted, not functional).
    #[serde(rename = "S3Key")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub s3_key: Option<String>,
    /// S3 object version (accepted, not functional).
    #[serde(rename = "S3ObjectVersion")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub s3_object_version: Option<String>,
    /// Container image URI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_uri: Option<String>,
    /// Whether to publish a new version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publish: Option<bool>,
    /// Architectures.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub architectures: Option<Vec<String>>,
}

/// Input for `UpdateFunctionConfiguration`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UpdateFunctionConfigurationInput {
    /// Runtime.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    /// IAM role ARN.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Handler.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handler: Option<String>,
    /// Description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Timeout in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u32>,
    /// Memory in MB.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_size: Option<u32>,
    /// Environment variables.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<Environment>,
    /// VPC configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vpc_config: Option<VpcConfig>,
    /// Dead letter configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dead_letter_config: Option<DeadLetterConfig>,
    /// Tracing configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracing_config: Option<TracingConfig>,
    /// Layer ARNs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layers: Option<Vec<String>>,
    /// Ephemeral storage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ephemeral_storage: Option<EphemeralStorage>,
    /// Image configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_config: Option<ImageConfig>,
    /// Logging configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logging_config: Option<LoggingConfig>,
    /// SnapStart configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snap_start: Option<SnapStart>,
}

/// Input for `PublishVersion`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PublishVersionInput {
    /// Code SHA-256 for validation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_sha256: Option<String>,
    /// Description for the published version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Revision ID for optimistic concurrency.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
}

/// Input for `CreateAlias`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CreateAliasInput {
    /// Alias name.
    pub name: String,
    /// Target function version.
    pub function_version: String,
    /// Description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Routing configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing_config: Option<AliasRoutingConfiguration>,
}

/// Input for `UpdateAlias`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UpdateAliasInput {
    /// Target function version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_version: Option<String>,
    /// Description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Routing configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing_config: Option<AliasRoutingConfiguration>,
    /// Revision ID for optimistic concurrency.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
}

/// Input for `AddPermission`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AddPermissionInput {
    /// Statement ID.
    pub statement_id: Option<String>,
    /// Action.
    pub action: Option<String>,
    /// Principal.
    pub principal: Option<String>,
    /// Source ARN.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_arn: Option<String>,
    /// Source account.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_account: Option<String>,
    /// Event source token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_source_token: Option<String>,
    /// Revision ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
    /// Principal org ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub principal_org_id: Option<String>,
    /// Function URL auth type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_url_auth_type: Option<String>,
}

/// Input for `TagResource`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TagResourceInput {
    /// Tags to add.
    pub tags: HashMap<String, String>,
}

/// Input for `CreateFunctionUrlConfig`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CreateFunctionUrlConfigInput {
    /// Auth type.
    pub auth_type: String,
    /// CORS configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cors: Option<Cors>,
    /// Invoke mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invoke_mode: Option<String>,
}

/// Input for `UpdateFunctionUrlConfig`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UpdateFunctionUrlConfigInput {
    /// Auth type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_type: Option<String>,
    /// CORS configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cors: Option<Cors>,
    /// Invoke mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invoke_mode: Option<String>,
}

/// Input for `PublishLayerVersion`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PublishLayerVersionInput {
    /// Layer name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer_name: Option<String>,
    /// Description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Layer code content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<LayerVersionContentInput>,
    /// Compatible runtimes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatible_runtimes: Option<Vec<String>>,
    /// License info (max 512 characters).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license_info: Option<String>,
    /// Compatible architectures.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatible_architectures: Option<Vec<String>>,
}

/// Input for `CreateEventSourceMapping`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateEventSourceMappingInput {
    /// ARN of the event source (e.g., SQS queue, DynamoDB stream, Kinesis stream).
    pub event_source_arn: String,
    /// Function name or ARN.
    pub function_name: String,
    /// Whether the mapping is enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Maximum number of records per batch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_size: Option<i32>,
    /// Maximum batching window in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum_batching_window_in_seconds: Option<i32>,
    /// Starting position for stream-based sources.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starting_position: Option<String>,
    /// Timestamp for `AT_TIMESTAMP` starting position.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starting_position_timestamp: Option<String>,
    /// Maximum age of a record in seconds before discarding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum_record_age_in_seconds: Option<i32>,
    /// Whether to split a batch on function error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bisect_batch_on_function_error: Option<bool>,
    /// Maximum number of retry attempts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum_retry_attempts: Option<i32>,
    /// Parallelization factor (1-10).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallelization_factor: Option<i32>,
    /// Function response types (e.g., `ReportBatchItemFailures`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_response_types: Option<Vec<String>>,
}

/// Input for `UpdateEventSourceMapping`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateEventSourceMappingInput {
    /// Function name or ARN.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_name: Option<String>,
    /// Whether the mapping is enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Maximum number of records per batch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_size: Option<i32>,
    /// Maximum batching window in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum_batching_window_in_seconds: Option<i32>,
    /// Maximum age of a record in seconds before discarding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum_record_age_in_seconds: Option<i32>,
    /// Whether to split a batch on function error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bisect_batch_on_function_error: Option<bool>,
    /// Maximum number of retry attempts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum_retry_attempts: Option<i32>,
    /// Parallelization factor (1-10).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallelization_factor: Option<i32>,
    /// Function response types (e.g., `ReportBatchItemFailures`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_response_types: Option<Vec<String>>,
}

/// Input for `AddLayerVersionPermission`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AddLayerVersionPermissionInput {
    /// Statement ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub statement_id: Option<String>,
    /// Action (e.g., `lambda:GetLayerVersion`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// Principal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub principal: Option<String>,
    /// Organization ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization_id: Option<String>,
    /// Revision ID for optimistic concurrency.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Concurrency
// ---------------------------------------------------------------------------

/// Input for `PutFunctionConcurrency`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutFunctionConcurrencyInput {
    /// The number of reserved concurrent executions.
    pub reserved_concurrent_executions: i32,
}

// ---------------------------------------------------------------------------
// Event Invoke Config
// ---------------------------------------------------------------------------

/// Input for `PutFunctionEventInvokeConfig` and `UpdateFunctionEventInvokeConfig`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EventInvokeConfigInput {
    /// Maximum retry attempts (0-2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum_retry_attempts: Option<i32>,
    /// Maximum event age in seconds (60-21600).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum_event_age_in_seconds: Option<i32>,
    /// Destination configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_config: Option<DestinationConfig>,
}
