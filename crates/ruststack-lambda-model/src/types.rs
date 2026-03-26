//! Shared types for the Lambda API.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Lambda function runtime identifiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Runtime {
    /// Python 3.9 runtime.
    #[serde(rename = "python3.9")]
    Python39,
    /// Python 3.10 runtime.
    #[serde(rename = "python3.10")]
    Python310,
    /// Python 3.11 runtime.
    #[serde(rename = "python3.11")]
    Python311,
    /// Python 3.12 runtime.
    #[serde(rename = "python3.12")]
    Python312,
    /// Python 3.13 runtime.
    #[serde(rename = "python3.13")]
    Python313,
    /// Node.js 18.x runtime.
    #[serde(rename = "nodejs18.x")]
    Nodejs18x,
    /// Node.js 20.x runtime.
    #[serde(rename = "nodejs20.x")]
    Nodejs20x,
    /// Node.js 22.x runtime.
    #[serde(rename = "nodejs22.x")]
    Nodejs22x,
    /// Java 21 runtime.
    #[serde(rename = "java21")]
    Java21,
    /// Java 17 runtime.
    #[serde(rename = "java17")]
    Java17,
    /// .NET 8 runtime.
    #[serde(rename = "dotnet8")]
    Dotnet8,
    /// Ruby 3.3 runtime.
    #[serde(rename = "ruby3.3")]
    Ruby33,
    /// Ruby 3.4 runtime.
    #[serde(rename = "ruby3.4")]
    Ruby34,
    /// Custom runtime on Amazon Linux 2023.
    #[serde(rename = "provided.al2023")]
    ProvidedAl2023,
    /// Custom runtime on Amazon Linux 2.
    #[serde(rename = "provided.al2")]
    ProvidedAl2,
}

impl Runtime {
    /// Returns the runtime identifier string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Python39 => "python3.9",
            Self::Python310 => "python3.10",
            Self::Python311 => "python3.11",
            Self::Python312 => "python3.12",
            Self::Python313 => "python3.13",
            Self::Nodejs18x => "nodejs18.x",
            Self::Nodejs20x => "nodejs20.x",
            Self::Nodejs22x => "nodejs22.x",
            Self::Java21 => "java21",
            Self::Java17 => "java17",
            Self::Dotnet8 => "dotnet8",
            Self::Ruby33 => "ruby3.3",
            Self::Ruby34 => "ruby3.4",
            Self::ProvidedAl2023 => "provided.al2023",
            Self::ProvidedAl2 => "provided.al2",
        }
    }
}

impl std::fmt::Display for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Function package type.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PackageType {
    /// Zip deployment package.
    #[default]
    Zip,
    /// Container image.
    Image,
}

/// Function architecture.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Architecture {
    /// x86_64 architecture.
    #[serde(rename = "x86_64")]
    #[default]
    X86_64,
    /// ARM64 architecture.
    #[serde(rename = "arm64")]
    Arm64,
}

impl Architecture {
    /// Returns the architecture identifier string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Arm64 => "arm64",
        }
    }
}

/// Function state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum State {
    /// Function is being created or updated.
    Pending,
    /// Function is ready to invoke.
    Active,
    /// Function is inactive.
    Inactive,
    /// Function creation or update failed.
    Failed,
}

/// Reason for the last state transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StateReasonCode {
    /// No issues.
    Idle,
    /// Function is being created.
    Creating,
    /// Function is being restored.
    Restoring,
    /// Ephemeral storage is being initialized.
    EniLimitExceeded,
    /// Internal error.
    InternalError,
    /// Subnet or security group out of range.
    SubnetOutOfIPAddresses,
    /// Invalid subnet.
    InvalidSubnet,
    /// Invalid security group.
    InvalidSecurityGroup,
    /// Image deleted.
    ImageDeleted,
    /// Image access denied.
    ImageAccessDenied,
    /// Invalid image.
    InvalidImage,
}

/// Lambda invocation type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvocationType {
    /// Synchronous invocation.
    RequestResponse,
    /// Asynchronous invocation.
    Event,
    /// Validate parameters without invoking.
    DryRun,
}

impl InvocationType {
    /// Parse from a string value.
    #[must_use]
    pub fn from_str_value(s: &str) -> Option<Self> {
        match s {
            "RequestResponse" | "" => Some(Self::RequestResponse),
            "Event" => Some(Self::Event),
            "DryRun" => Some(Self::DryRun),
            _ => None,
        }
    }
}

/// Log type for invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogType {
    /// No logs.
    None,
    /// Include last 4KB of logs (base64-encoded).
    Tail,
}

impl LogType {
    /// Parse from a string value.
    #[must_use]
    pub fn from_str_value(s: &str) -> Option<Self> {
        match s {
            "None" | "" => Some(Self::None),
            "Tail" => Some(Self::Tail),
            _ => None,
        }
    }
}

/// Function URL auth type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FunctionUrlAuthType {
    /// No authentication.
    #[serde(rename = "NONE")]
    None,
    /// IAM authentication.
    #[serde(rename = "AWS_IAM")]
    AwsIam,
}

/// Function URL invoke mode.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum InvokeMode {
    /// Buffered response.
    #[serde(rename = "BUFFERED")]
    #[default]
    Buffered,
    /// Streaming response.
    #[serde(rename = "RESPONSE_STREAM")]
    ResponseStream,
}

/// Function code input for `CreateFunction` and `UpdateFunctionCode`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct FunctionCode {
    /// Base64-encoded zip file contents.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zip_file: Option<String>,
    /// S3 bucket (accepted but not functional).
    #[serde(rename = "S3Bucket")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub s3_bucket: Option<String>,
    /// S3 key (accepted but not functional).
    #[serde(rename = "S3Key")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub s3_key: Option<String>,
    /// S3 object version (accepted but not functional).
    #[serde(rename = "S3ObjectVersion")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub s3_object_version: Option<String>,
    /// Container image URI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_uri: Option<String>,
}

/// Function code location in `GetFunction` response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct FunctionCodeLocation {
    /// Repository type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_type: Option<String>,
    /// Pre-signed URL (simulated).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// Container image URI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_uri: Option<String>,
    /// Resolved image URI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_image_uri: Option<String>,
}

/// Environment variable configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Environment {
    /// Environment variable key-value pairs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variables: Option<HashMap<String, String>>,
}

/// Environment variable response (may include error info).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EnvironmentResponse {
    /// Environment variable key-value pairs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variables: Option<HashMap<String, String>>,
    /// Error information.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<EnvironmentError>,
}

/// Environment variable error.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EnvironmentError {
    /// Error code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    /// Error message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// VPC configuration (stored, not enforced).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct VpcConfig {
    /// Subnet IDs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subnet_ids: Option<Vec<String>>,
    /// Security group IDs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_group_ids: Option<Vec<String>>,
}

/// VPC configuration response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct VpcConfigResponse {
    /// Subnet IDs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subnet_ids: Option<Vec<String>>,
    /// Security group IDs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_group_ids: Option<Vec<String>>,
    /// VPC ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vpc_id: Option<String>,
}

/// Tracing configuration (stored, not enforced).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TracingConfig {
    /// Tracing mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

/// Tracing configuration response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TracingConfigResponse {
    /// Tracing mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

/// Dead letter queue configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeadLetterConfig {
    /// Target ARN (SQS queue or SNS topic).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_arn: Option<String>,
}

/// Ephemeral storage configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EphemeralStorage {
    /// Size in MB (512-10240).
    pub size: u32,
}

impl Default for EphemeralStorage {
    fn default() -> Self {
        Self { size: 512 }
    }
}

/// Image configuration override.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ImageConfig {
    /// Entry point override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_point: Option<Vec<String>>,
    /// Command override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<Vec<String>>,
    /// Working directory override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
}

/// Image configuration response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ImageConfigResponse {
    /// Image configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_config: Option<ImageConfig>,
    /// Error information.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ImageConfigError>,
}

/// Image configuration error.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ImageConfigError {
    /// Error code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    /// Error message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Logging configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LoggingConfig {
    /// Log format.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_format: Option<String>,
    /// Application log level.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub application_log_level: Option<String>,
    /// System log level.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_log_level: Option<String>,
    /// Log group name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_group: Option<String>,
}

/// SnapStart configuration (stored, not enforced).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SnapStart {
    /// Apply on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apply_on: Option<String>,
}

/// SnapStart response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SnapStartResponse {
    /// Apply on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apply_on: Option<String>,
    /// Optimization status.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optimization_status: Option<String>,
}

/// Layer reference.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Layer {
    /// Layer ARN.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
    /// Code size in bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_size: Option<i64>,
    /// Signing profile version ARN.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signing_profile_version_arn: Option<String>,
    /// Signing job ARN.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signing_job_arn: Option<String>,
}

/// Layer version code input for `PublishLayerVersion`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LayerVersionContentInput {
    /// S3 bucket containing the layer code.
    #[serde(rename = "S3Bucket")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub s3_bucket: Option<String>,
    /// S3 key for the layer code.
    #[serde(rename = "S3Key")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub s3_key: Option<String>,
    /// S3 object version.
    #[serde(rename = "S3ObjectVersion")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub s3_object_version: Option<String>,
    /// Base64-encoded zip file contents.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zip_file: Option<String>,
}

/// Layer version code output returned in layer version responses.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LayerVersionContentOutput {
    /// Pre-signed URL to download the layer code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// SHA-256 hash of the layer code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_sha256: Option<String>,
    /// Code size in bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_size: Option<i64>,
    /// Signing profile version ARN.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signing_profile_version_arn: Option<String>,
    /// Signing job ARN.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signing_job_arn: Option<String>,
}

/// Summary of a layer version in list responses.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LayerVersionsListItem {
    /// Layer version ARN.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer_version_arn: Option<String>,
    /// Version number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<i64>,
    /// Description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// ISO 8601 creation date.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_date: Option<String>,
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

/// Summary of a layer in list responses.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LayersListItem {
    /// Layer name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer_name: Option<String>,
    /// Layer ARN (without version).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer_arn: Option<String>,
    /// Latest matching version summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_matching_version: Option<LayerVersionsListItem>,
}

/// Function configuration returned by many operations.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct FunctionConfiguration {
    /// Function name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_name: Option<String>,
    /// Function ARN.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_arn: Option<String>,
    /// Runtime.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    /// IAM role ARN.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Handler function.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handler: Option<String>,
    /// Code size in bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_size: Option<i64>,
    /// Description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Timeout in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u32>,
    /// Memory size in MB.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_size: Option<u32>,
    /// Last modified timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<String>,
    /// Code SHA-256 hash.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_sha256: Option<String>,
    /// Version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Environment variables.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<EnvironmentResponse>,
    /// VPC configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vpc_config: Option<VpcConfigResponse>,
    /// Dead letter configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dead_letter_config: Option<DeadLetterConfig>,
    /// Tracing configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracing_config: Option<TracingConfigResponse>,
    /// Revision ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
    /// Layers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layers: Option<Vec<Layer>>,
    /// Function state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// State reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_reason: Option<String>,
    /// State reason code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_reason_code: Option<String>,
    /// Package type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_type: Option<String>,
    /// Architectures.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub architectures: Option<Vec<String>>,
    /// Ephemeral storage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ephemeral_storage: Option<EphemeralStorage>,
    /// Logging configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logging_config: Option<LoggingConfig>,
    /// SnapStart response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snap_start: Option<SnapStartResponse>,
    /// Image configuration response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_config_response: Option<ImageConfigResponse>,
    /// Last update status.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_update_status: Option<String>,
    /// Last update status reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_update_status_reason: Option<String>,
    /// Last update status reason code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_update_status_reason_code: Option<String>,
}

/// Alias configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AliasConfiguration {
    /// Alias ARN.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias_arn: Option<String>,
    /// Alias name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Target function version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_version: Option<String>,
    /// Description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Routing configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing_config: Option<AliasRoutingConfiguration>,
    /// Revision ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
}

/// Alias routing configuration for weighted aliases.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AliasRoutingConfiguration {
    /// Additional version weights.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_version_weights: Option<HashMap<String, f64>>,
}

/// CORS configuration for function URLs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Cors {
    /// Allowed headers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_headers: Option<Vec<String>>,
    /// Allowed methods.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_methods: Option<Vec<String>>,
    /// Allowed origins.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_origins: Option<Vec<String>>,
    /// Exposed headers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expose_headers: Option<Vec<String>>,
    /// Max age in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_age: Option<u32>,
    /// Allow credentials.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_credentials: Option<bool>,
}

/// Function URL configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct FunctionUrlConfig {
    /// Function URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_url: Option<String>,
    /// Function ARN.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_arn: Option<String>,
    /// Auth type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_type: Option<String>,
    /// CORS configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cors: Option<Cors>,
    /// Creation timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creation_time: Option<String>,
    /// Last modified timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_modified_time: Option<String>,
    /// Invoke mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invoke_mode: Option<String>,
}

/// Configuration of an event source mapping.
///
/// This is the response type shared across all event source mapping operations
/// (Create, Get, Update, Delete, List).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EventSourceMappingConfiguration {
    /// The event source mapping UUID.
    #[serde(rename = "UUID", skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    /// ARN of the event source.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_source_arn: Option<String>,
    /// Function ARN.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_arn: Option<String>,
    /// State of the mapping (Creating, Enabled, Disabled, Enabling, Disabling, Updating,
    /// Deleting).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Reason for the current state transition.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_transition_reason: Option<String>,
    /// Last modified timestamp (epoch seconds).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<f64>,
    /// Result of the last processing attempt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_processing_result: Option<String>,
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

// ---------------------------------------------------------------------------
// Concurrency
// ---------------------------------------------------------------------------

/// Concurrency configuration for a Lambda function.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Concurrency {
    /// The number of reserved concurrent executions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reserved_concurrent_executions: Option<i32>,
}

// ---------------------------------------------------------------------------
// Event Invoke Config
// ---------------------------------------------------------------------------

/// Destination configuration for asynchronous invocations.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DestinationConfig {
    /// Destination for successful invocations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_success: Option<OnSuccess>,
    /// Destination for failed invocations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_failure: Option<OnFailure>,
}

/// Destination for successful asynchronous invocations.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct OnSuccess {
    /// The ARN of the destination resource.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<String>,
}

/// Destination for failed asynchronous invocations.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct OnFailure {
    /// The ARN of the destination resource.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<String>,
}

/// Event invoke configuration for a function.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct FunctionEventInvokeConfig {
    /// The function ARN.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_arn: Option<String>,
    /// Maximum retry attempts (0-2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum_retry_attempts: Option<i32>,
    /// Maximum event age in seconds (60-21600).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum_event_age_in_seconds: Option<i32>,
    /// Last modified timestamp as epoch millis (float).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<f64>,
    /// Destination configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_config: Option<DestinationConfig>,
}
