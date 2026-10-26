//! In-memory function storage using `DashMap` for concurrent access.
//!
//! Stores function records, version snapshots, aliases, policies, tags,
//! and function URL configurations. Code is stored as raw bytes with
//! SHA-256 hashes computed on ingestion.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt,
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
    str::from_utf8,
};

use base64::{Engine, engine::general_purpose::STANDARD};
use bytes::Bytes;
use dashmap::DashMap;
use rustack_lambda_model::types::{
    Cors, DeadLetterConfig, DestinationConfig, ImageConfig, LoggingConfig, SnapStart,
    TracingConfig, VpcConfig,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::watch;
use zip::{System, read::ZipFile};

use crate::{
    artifact::{ImageArtifact, ResolvedImage, ValidatedArtifact},
    deployment::DeploymentState,
    error::LambdaServiceError,
    identity::FunctionResourceId,
};

/// In-memory store for Lambda functions.
#[derive(Debug)]
pub struct FunctionStore {
    /// All functions keyed by function name.
    functions: DashMap<String, FunctionRecord>,
    /// Root directory for storing extracted code.
    code_dir: PathBuf,
    /// Own completed artifacts until the store is dropped; in-flight revisions remain readable.
    artifacts: DashMap<PathBuf, ValidatedArtifact>,
    staging: std::sync::Arc<tokio::sync::Semaphore>,
    changes: watch::Sender<()>,
}

/// Rolls back a newly registered artifact unless its record mutation commits.
pub(crate) struct StagedArtifact<'a> {
    store: &'a FunctionStore,
    artifact: ValidatedArtifact,
    committed: bool,
}

impl fmt::Debug for StagedArtifact<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StagedArtifact")
            .field("artifact", &self.artifact)
            .field("committed", &self.committed)
            .finish()
    }
}

impl<'a> StagedArtifact<'a> {
    fn new(store: &'a FunctionStore, artifact: ValidatedArtifact) -> Self {
        Self {
            store,
            artifact,
            committed: false,
        }
    }
    pub(crate) fn artifact(&self) -> &ValidatedArtifact {
        &self.artifact
    }
    pub(crate) fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for StagedArtifact<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.store
                .discard_staged_artifact(self.artifact.code_root());
        }
    }
}

/// Complete record for a Lambda function.
#[derive(Debug, Clone)]
pub struct FunctionRecord {
    /// Stable identity for this resource's lifetime, independent of its name.
    pub resource_id: FunctionResourceId,
    /// Function name.
    pub name: String,
    /// Function ARN.
    pub arn: String,
    /// The `$LATEST` version record.
    pub latest: VersionRecord,
    /// Published versions (1, 2, 3, ...).
    pub versions: BTreeMap<u64, VersionRecord>,
    /// Next version number to assign.
    pub next_version: u64,
    /// Named aliases (e.g., `prod`, `staging`).
    pub aliases: HashMap<String, AliasRecord>,
    /// Resource-based policy document.
    pub policy: PolicyDocument,
    /// Resource tags.
    pub tags: HashMap<String, String>,
    /// Associated code signing config ARN.
    pub code_signing_config_arn: Option<String>,
    /// Function URL configuration.
    pub url_config: Option<FunctionUrlConfigRecord>,
    /// Reserved concurrent executions.
    pub reserved_concurrent_executions: Option<i32>,
    /// Event invoke configurations keyed by qualifier.
    pub event_invoke_configs: HashMap<String, EventInvokeConfigRecord>,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
}

/// Stored event invoke configuration for a function qualifier.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventInvokeConfigRecord {
    /// The qualified function ARN.
    pub function_arn: String,
    /// The qualifier (version or alias, defaults to `$LATEST`).
    pub qualifier: String,
    /// Maximum retry attempts (0-2).
    pub maximum_retry_attempts: Option<i32>,
    /// Maximum event age in seconds (60-21600).
    pub maximum_event_age_in_seconds: Option<i32>,
    /// ISO 8601 last-modified timestamp.
    pub last_modified: String,
    /// Destination configuration.
    pub destination_config: Option<DestinationConfig>,
}

/// A snapshot of function configuration at a specific version.
#[derive(Debug, Clone)]
pub struct VersionRecord {
    /// Version string (`$LATEST` or a numeric string like `"1"`).
    pub version: String,
    /// Runtime identifier.
    pub runtime: Option<String>,
    /// Handler function identifier.
    pub handler: Option<String>,
    /// IAM execution role ARN.
    pub role: String,
    /// Description.
    pub description: String,
    /// Timeout in seconds.
    pub timeout: u32,
    /// Memory size in MB.
    pub memory_size: u32,
    /// Environment variables.
    pub environment: HashMap<String, String>,
    /// Package type (`Zip` or `Image`).
    pub package_type: String,
    /// Path to extracted code on disk.
    pub code_path: Option<PathBuf>,
    /// Container image URI.
    pub image_uri: Option<String>,
    /// Unresolved, restored metadata, or shared live image availability.
    pub image: ImageArtifact,
    /// Raw zip bytes.
    pub zip_bytes: Option<Bytes>,
    /// Authoritative readiness and update progress.
    pub deployment: DeploymentState,
    /// ISO 8601 last-modified timestamp.
    pub last_modified: String,
    /// Supported architectures.
    pub architectures: Vec<String>,
    /// Ephemeral storage size in MB.
    pub ephemeral_storage_size: u32,
    /// Base64-encoded SHA-256 of the deployment package.
    pub code_sha256: String,
    /// Code size in bytes.
    pub code_size: u64,
    /// Revision ID for optimistic concurrency.
    pub revision_id: String,
    /// Layer ARNs.
    pub layers: Vec<String>,
    /// VPC configuration.
    pub vpc_config: Option<VpcConfig>,
    /// Dead letter queue configuration.
    pub dead_letter_config: Option<DeadLetterConfig>,
    /// Tracing configuration.
    pub tracing_config: Option<TracingConfig>,
    /// Image configuration override.
    pub image_config: Option<ImageConfig>,
    /// Logging configuration.
    pub logging_config: Option<LoggingConfig>,
    /// `SnapStart` configuration.
    pub snap_start: Option<SnapStart>,
}

/// Alias mapping to a function version.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AliasRecord {
    /// Alias name.
    pub name: String,
    /// Target function version string.
    pub function_version: String,
    /// Description.
    pub description: String,
    /// Routing configuration for weighted aliases.
    pub routing_config: Option<HashMap<String, f64>>,
    /// Revision ID.
    pub revision_id: String,
}

/// Resource-based policy document.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyDocument {
    /// Policy statements.
    pub statements: Vec<PolicyStatement>,
}

/// A single policy statement.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyStatement {
    /// Statement ID.
    pub sid: String,
    /// Effect (`Allow` or `Deny`).
    pub effect: String,
    /// Principal.
    pub principal: String,
    /// Action.
    pub action: String,
    /// Resource ARN.
    pub resource: String,
    /// Condition map (serialized as JSON).
    pub condition: Option<serde_json::Value>,
}

/// Function URL configuration record.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionUrlConfigRecord {
    /// The generated function URL.
    pub function_url: String,
    /// Auth type (`NONE` or `AWS_IAM`).
    pub auth_type: String,
    /// CORS configuration.
    pub cors: Option<Cors>,
    /// Invoke mode (`BUFFERED` or `RESPONSE_STREAM`).
    pub invoke_mode: String,
    /// ISO 8601 creation timestamp.
    pub creation_time: String,
    /// ISO 8601 last-modified timestamp.
    pub last_modified_time: String,
}

/// Complete record for a Lambda layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerRecord {
    /// Layer name.
    pub name: String,
    /// Layer ARN (without version).
    pub layer_arn: String,
    /// Published layer versions keyed by version number.
    pub versions: BTreeMap<u64, LayerVersionRecord>,
    /// Next version number to assign.
    pub next_version: u64,
}

/// A snapshot of a layer at a specific version.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerVersionRecord {
    /// Version number.
    pub version: u64,
    /// Description.
    pub description: String,
    /// Compatible runtimes.
    pub compatible_runtimes: Vec<String>,
    /// Compatible architectures.
    pub compatible_architectures: Vec<String>,
    /// License info.
    pub license_info: Option<String>,
    /// Base64-encoded SHA-256 of the layer code.
    pub code_sha256: String,
    /// Code size in bytes.
    pub code_size: u64,
    /// ISO 8601 creation date.
    pub created_date: String,
    /// Layer ARN (without version).
    pub layer_arn: String,
    /// Layer version ARN.
    pub layer_version_arn: String,
    /// Resource-based policy document for this layer version.
    pub policy: PolicyDocument,
}

/// In-memory store for Lambda layers.
#[derive(Debug)]
pub struct LayerStore {
    /// All layers keyed by layer name.
    layers: DashMap<String, LayerRecord>,
}

impl LayerStore {
    /// Create a new empty layer store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            layers: DashMap::new(),
        }
    }

    /// Get a clone of a layer record by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<LayerRecord> {
        self.layers.get(name).map(|r| r.value().clone())
    }

    /// Publish a new version of a layer.
    ///
    /// Creates the layer record if it does not exist, then inserts a new version.
    /// Returns the assigned version number.
    #[must_use]
    pub fn publish_version(
        &self,
        name: &str,
        layer_arn: &str,
        version_record: LayerVersionRecord,
    ) -> u64 {
        use dashmap::mapref::entry::Entry;
        match self.layers.entry(name.to_owned()) {
            Entry::Occupied(mut entry) => {
                let record = entry.get_mut();
                let version_num = record.next_version;
                record.next_version += 1;
                record.versions.insert(version_num, version_record);
                version_num
            }
            Entry::Vacant(entry) => {
                let version_num = 1;
                let mut versions = BTreeMap::new();
                versions.insert(version_num, version_record);
                entry.insert(LayerRecord {
                    name: name.to_owned(),
                    layer_arn: layer_arn.to_owned(),
                    versions,
                    next_version: 2,
                });
                version_num
            }
        }
    }

    /// Get a specific layer version.
    #[must_use]
    pub fn get_version(&self, name: &str, version: u64) -> Option<LayerVersionRecord> {
        self.layers
            .get(name)
            .and_then(|r| r.versions.get(&version).cloned())
    }

    /// List all versions of a layer, sorted by version number.
    #[must_use]
    pub fn list_versions(&self, name: &str) -> Vec<LayerVersionRecord> {
        self.layers
            .get(name)
            .map(|r| r.versions.values().cloned().collect())
            .unwrap_or_default()
    }

    /// List all layers with their latest version.
    ///
    /// Returns cloned records sorted by layer name.
    #[must_use]
    pub fn list_layers(&self) -> Vec<LayerRecord> {
        let mut records: Vec<LayerRecord> = self.layers.iter().map(|r| r.value().clone()).collect();
        records.sort_by(|a, b| a.name.cmp(&b.name));
        records
    }

    /// Export all layer metadata in deterministic order.
    #[must_use]
    pub fn export_snapshot(&self) -> LayerStoreSnapshot {
        LayerStoreSnapshot {
            layers: self.list_layers(),
        }
    }

    /// Replace layer metadata from a snapshot.
    pub fn import_snapshot(&self, snapshot: LayerStoreSnapshot) {
        self.layers.clear();
        for layer in snapshot.layers {
            self.layers.insert(layer.name.clone(), layer);
        }
    }

    /// Delete a specific layer version.
    ///
    /// Returns `true` if the version existed and was removed.
    #[must_use]
    pub fn delete_version(&self, name: &str, version: u64) -> bool {
        if let Some(mut entry) = self.layers.get_mut(name) {
            let removed = entry.versions.remove(&version).is_some();
            // If no versions remain, remove the entire layer record.
            if entry.versions.is_empty() {
                drop(entry);
                self.layers.remove(name);
            }
            removed
        } else {
            false
        }
    }

    /// Mutate a layer version record in place.
    ///
    /// # Errors
    ///
    /// Returns an error if the layer or version does not exist.
    pub fn update_version<F, R>(
        &self,
        name: &str,
        version: u64,
        f: F,
    ) -> Result<R, LambdaServiceError>
    where
        F: FnOnce(&mut LayerVersionRecord) -> R,
    {
        match self.layers.get_mut(name) {
            Some(mut entry) => match entry.versions.get_mut(&version) {
                Some(ver) => Ok(f(ver)),
                None => Err(LambdaServiceError::InvalidParameter {
                    message: format!("Layer version not found: {name}:{version}"),
                }),
            },
            None => Err(LambdaServiceError::InvalidParameter {
                message: format!("Layer not found: {name}"),
            }),
        }
    }
}

impl Default for LayerStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Serializable layer store snapshot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerStoreSnapshot {
    /// Layer records.
    pub layers: Vec<LayerRecord>,
}

/// Complete record for a Lambda event source mapping.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventSourceMappingRecord {
    /// Unique identifier for the mapping.
    pub uuid: String,
    /// ARN of the event source (e.g., SQS queue, `DynamoDB` stream).
    pub event_source_arn: String,
    /// ARN of the target Lambda function.
    pub function_arn: String,
    /// Whether the mapping is enabled.
    pub enabled: bool,
    /// Maximum number of records per batch.
    pub batch_size: i32,
    /// Maximum batching window in seconds.
    pub maximum_batching_window_in_seconds: i32,
    /// Starting position for stream-based sources.
    pub starting_position: Option<String>,
    /// Timestamp for `AT_TIMESTAMP` starting position.
    pub starting_position_timestamp: Option<String>,
    /// Maximum age of a record in seconds before discarding.
    pub maximum_record_age_in_seconds: Option<i32>,
    /// Whether to split a batch on function error.
    pub bisect_batch_on_function_error: Option<bool>,
    /// Maximum number of retry attempts.
    pub maximum_retry_attempts: Option<i32>,
    /// Parallelization factor (1-10).
    pub parallelization_factor: Option<i32>,
    /// Function response types (e.g., `ReportBatchItemFailures`).
    pub function_response_types: Vec<String>,
    /// State of the mapping (`Enabled` or `Disabled`).
    pub state: String,
    /// Reason for the current state transition.
    pub state_transition_reason: String,
    /// Last modified time as epoch seconds.
    pub last_modified: i64,
    /// Result of the last processing attempt.
    pub last_processing_result: String,
}

/// In-memory store for Lambda event source mappings.
#[derive(Debug)]
pub struct EventSourceMappingStore {
    /// All mappings keyed by UUID.
    mappings: DashMap<String, EventSourceMappingRecord>,
}

impl EventSourceMappingStore {
    /// Create a new empty event source mapping store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            mappings: DashMap::new(),
        }
    }

    /// Insert a new event source mapping record.
    pub fn create(&self, record: EventSourceMappingRecord) {
        self.mappings.insert(record.uuid.clone(), record);
    }

    /// Get a clone of an event source mapping by UUID.
    #[must_use]
    pub fn get(&self, uuid: &str) -> Option<EventSourceMappingRecord> {
        self.mappings.get(uuid).map(|r| r.value().clone())
    }

    /// Update an event source mapping in place.
    ///
    /// # Errors
    ///
    /// Returns an error if the mapping does not exist.
    pub fn update<F, R>(&self, uuid: &str, f: F) -> Result<R, LambdaServiceError>
    where
        F: FnOnce(&mut EventSourceMappingRecord) -> R,
    {
        match self.mappings.get_mut(uuid) {
            Some(mut entry) => Ok(f(entry.value_mut())),
            None => Err(LambdaServiceError::EventSourceMappingNotFound {
                uuid: uuid.to_owned(),
            }),
        }
    }

    /// Delete an event source mapping by UUID.
    ///
    /// Returns the removed record, or `None` if it did not exist.
    #[must_use]
    pub fn delete(&self, uuid: &str) -> Option<EventSourceMappingRecord> {
        self.mappings.remove(uuid).map(|(_, v)| v)
    }

    /// List all event source mappings, optionally filtering by function name and/or event source
    /// ARN.
    ///
    /// Results are sorted by UUID for deterministic ordering.
    #[must_use]
    pub fn list(
        &self,
        function_name_filter: Option<&str>,
        event_source_arn_filter: Option<&str>,
    ) -> Vec<EventSourceMappingRecord> {
        let mut records: Vec<EventSourceMappingRecord> = self
            .mappings
            .iter()
            .filter(|entry| {
                let record = entry.value();
                if let Some(fn_filter) = function_name_filter {
                    // Match against function ARN (contains function name or full ARN match).
                    if !record.function_arn.contains(fn_filter) {
                        return false;
                    }
                }
                if let Some(arn_filter) = event_source_arn_filter {
                    if record.event_source_arn != arn_filter {
                        return false;
                    }
                }
                true
            })
            .map(|entry| entry.value().clone())
            .collect();
        records.sort_by(|a, b| a.uuid.cmp(&b.uuid));
        records
    }

    /// Export all event source mappings in deterministic order.
    #[must_use]
    pub fn export_snapshot(&self) -> EventSourceMappingStoreSnapshot {
        EventSourceMappingStoreSnapshot {
            mappings: self.list(None, None),
        }
    }

    /// Replace all event source mappings from a snapshot.
    pub fn import_snapshot(&self, snapshot: EventSourceMappingStoreSnapshot) {
        self.mappings.clear();
        for mapping in snapshot.mappings {
            self.mappings.insert(mapping.uuid.clone(), mapping);
        }
    }
}

impl Default for EventSourceMappingStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Serializable event source mapping store snapshot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventSourceMappingStoreSnapshot {
    /// Event source mapping records.
    pub mappings: Vec<EventSourceMappingRecord>,
}

/// Serializable function store snapshot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionStoreSnapshot {
    /// Lambda function records.
    pub functions: Vec<FunctionRecordSnapshot>,
}

/// Serializable Lambda function record.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionRecordSnapshot {
    /// Stable identity; older snapshots allocate one when first decoded.
    #[serde(default)]
    pub resource_id: FunctionResourceId,
    /// Function name.
    pub name: String,
    /// Function ARN.
    pub arn: String,
    /// The `$LATEST` version record.
    pub latest: VersionRecordSnapshot,
    /// Published versions.
    pub versions: BTreeMap<u64, VersionRecordSnapshot>,
    /// Next version number to assign.
    pub next_version: u64,
    /// Named aliases.
    pub aliases: HashMap<String, AliasRecord>,
    /// Resource-based policy document.
    pub policy: PolicyDocument,
    /// Resource tags.
    pub tags: HashMap<String, String>,
    /// Associated code signing config ARN.
    pub code_signing_config_arn: Option<String>,
    /// Function URL configuration.
    pub url_config: Option<FunctionUrlConfigRecord>,
    /// Reserved concurrent executions.
    pub reserved_concurrent_executions: Option<i32>,
    /// Event invoke configurations keyed by qualifier.
    pub event_invoke_configs: HashMap<String, EventInvokeConfigRecord>,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
}

/// Serializable Lambda function version snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionRecordSnapshot {
    /// Version string.
    pub version: String,
    /// Runtime identifier.
    pub runtime: Option<String>,
    /// Handler function identifier.
    pub handler: Option<String>,
    /// IAM execution role ARN.
    pub role: String,
    /// Description.
    pub description: String,
    /// Timeout in seconds.
    pub timeout: u32,
    /// Memory size in MB.
    pub memory_size: u32,
    /// Environment variables.
    pub environment: HashMap<String, String>,
    /// Package type.
    pub package_type: String,
    /// Container image URI.
    pub image_uri: Option<String>,
    /// Immutable image identity; older metadata-only snapshots have none.
    #[serde(default)]
    pub resolved_image: Option<ResolvedImage>,
    /// Raw zip bytes encoded as standard base64.
    pub zip_bytes_base64: Option<String>,
    /// Legacy function state, derived from deployment on new exports.
    pub state: String,
    /// Authoritative deployment state in current snapshots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment: Option<DeploymentState>,
    /// ISO 8601 last-modified timestamp.
    pub last_modified: String,
    /// Supported architectures.
    pub architectures: Vec<String>,
    /// Ephemeral storage size in MB.
    pub ephemeral_storage_size: u32,
    /// Base64-encoded SHA-256 of the deployment package.
    pub code_sha256: String,
    /// Code size in bytes.
    pub code_size: u64,
    /// Revision ID for optimistic concurrency.
    pub revision_id: String,
    /// Layer ARNs.
    pub layers: Vec<String>,
    /// VPC configuration.
    pub vpc_config: Option<VpcConfig>,
    /// Dead letter queue configuration.
    pub dead_letter_config: Option<DeadLetterConfig>,
    /// Tracing configuration.
    pub tracing_config: Option<TracingConfig>,
    /// Image configuration override.
    pub image_config: Option<ImageConfig>,
    /// Logging configuration.
    pub logging_config: Option<LoggingConfig>,
    /// `SnapStart` configuration.
    pub snap_start: Option<SnapStart>,
}

impl FunctionRecordSnapshot {
    fn from_record(record: FunctionRecord) -> Self {
        Self {
            resource_id: record.resource_id,
            name: record.name,
            arn: record.arn,
            latest: VersionRecordSnapshot::from_record(record.latest),
            versions: record
                .versions
                .into_iter()
                .map(|(version, record)| (version, VersionRecordSnapshot::from_record(record)))
                .collect(),
            next_version: record.next_version,
            aliases: record.aliases,
            policy: record.policy,
            tags: record.tags,
            code_signing_config_arn: record.code_signing_config_arn,
            url_config: record.url_config,
            reserved_concurrent_executions: record.reserved_concurrent_executions,
            event_invoke_configs: record.event_invoke_configs,
            created_at: record.created_at,
        }
    }

    async fn into_record(
        self,
        store: &FunctionStore,
    ) -> Result<(FunctionRecord, Vec<StagedArtifact<'_>>), LambdaServiceError> {
        let name = self.name;
        let (latest, pending) = self.latest.into_record(store, &name).await?;
        let mut artifacts: Vec<_> = pending.into_iter().collect();
        let mut versions = BTreeMap::new();
        for (version, record) in self.versions {
            let (record, artifact) = record.into_record(store, &name).await?;
            artifacts.extend(artifact);
            versions.insert(version, record);
        }
        Ok((
            FunctionRecord {
                resource_id: self.resource_id,
                name,
                arn: self.arn,
                latest,
                versions,
                next_version: self.next_version,
                aliases: self.aliases,
                policy: self.policy,
                tags: self.tags,
                code_signing_config_arn: self.code_signing_config_arn,
                url_config: self.url_config,
                reserved_concurrent_executions: self.reserved_concurrent_executions,
                event_invoke_configs: self.event_invoke_configs,
                created_at: self.created_at,
            },
            artifacts,
        ))
    }
}

impl VersionRecordSnapshot {
    fn from_record(record: VersionRecord) -> Self {
        Self {
            version: record.version,
            runtime: record.runtime,
            handler: record.handler,
            role: record.role,
            description: record.description,
            timeout: record.timeout,
            memory_size: record.memory_size,
            environment: record.environment,
            package_type: record.package_type,
            image_uri: record.image_uri,
            resolved_image: record.image.resolved().cloned(),
            zip_bytes_base64: record
                .zip_bytes
                .as_ref()
                .map(|bytes| STANDARD.encode(bytes.as_ref())),
            state: record.deployment.state().into(),
            deployment: Some(record.deployment),
            last_modified: record.last_modified,
            architectures: record.architectures,
            ephemeral_storage_size: record.ephemeral_storage_size,
            code_sha256: record.code_sha256,
            code_size: record.code_size,
            revision_id: record.revision_id,
            layers: record.layers,
            vpc_config: record.vpc_config,
            dead_letter_config: record.dead_letter_config,
            tracing_config: record.tracing_config,
            image_config: record.image_config,
            logging_config: record.logging_config,
            snap_start: record.snap_start,
        }
    }

    async fn into_record<'a>(
        self,
        store: &'a FunctionStore,
        function_name: &str,
    ) -> Result<(VersionRecord, Option<StagedArtifact<'a>>), LambdaServiceError> {
        crate::function_config::validate_snapshot_settings(&self)?;
        let zip_bytes = match self.zip_bytes_base64 {
            Some(encoded) => {
                let decoded = STANDARD.decode(encoded.as_bytes()).map_err(|source| {
                    LambdaServiceError::Internal {
                        message: format!("Failed to decode Lambda zip bytes: {source}"),
                    }
                })?;
                Some(Bytes::from(decoded))
            }
            None => None,
        };
        let artifact = if let Some(bytes) = zip_bytes.as_ref() {
            Some(
                store
                    .stage_zip_code(function_name, &self.version, bytes)
                    .await?,
            )
        } else {
            None
        };
        let code_path = artifact
            .as_ref()
            .map(|artifact| artifact.artifact().code_root().to_path_buf());

        Ok((
            VersionRecord {
                version: self.version,
                runtime: self.runtime,
                handler: self.handler,
                role: self.role,
                description: self.description,
                timeout: self.timeout,
                memory_size: self.memory_size,
                environment: self.environment,
                package_type: self.package_type,
                code_path,
                image_uri: self.image_uri,
                image: self
                    .resolved_image
                    .map_or(ImageArtifact::Unresolved, ImageArtifact::Restored),
                zip_bytes,
                deployment: DeploymentState::restore(&self.state, self.deployment)?,
                last_modified: self.last_modified,
                architectures: self.architectures,
                ephemeral_storage_size: self.ephemeral_storage_size,
                code_sha256: self.code_sha256,
                code_size: self.code_size,
                revision_id: self.revision_id,
                layers: self.layers,
                vpc_config: self.vpc_config,
                dead_letter_config: self.dead_letter_config,
                tracing_config: self.tracing_config,
                image_config: self.image_config,
                logging_config: self.logging_config,
                snap_start: self.snap_start,
            },
            artifact,
        ))
    }
}

impl FunctionStore {
    fn validate_record(&self, record: &FunctionRecord) -> Result<(), LambdaServiceError> {
        let invalid = |message: &str| LambdaServiceError::InvalidParameter {
            message: message.into(),
        };
        crate::resolver::FunctionName::parse(&record.name)?;
        crate::resolver::FunctionArn::parse(&record.arn)?;
        let (name, qualifier) = crate::resolver::resolve_function_ref(&record.arn)?;
        if name != record.name || qualifier.is_some() {
            return Err(invalid("Function ARN mismatch"));
        }
        if record.latest.version != "$LATEST"
            || record.next_version == 0
            || record.next_version == u64::MAX
            || record
                .versions
                .last_key_value()
                .is_some_and(|(version, _)| *version >= record.next_version)
            || record
                .reserved_concurrent_executions
                .is_some_and(|n| !(0..=1000).contains(&n))
        {
            return Err(invalid("Invalid function version or concurrency"));
        }
        for (number, version) in &record.versions {
            if *number == 0
                || version.version != number.to_string()
                || version.deployment != DeploymentState::Ready
                || (version.package_type == "Image" && version.image.resolved().is_none())
            {
                return Err(invalid("Published version number mismatch"));
            }
        }
        for (name, alias) in &record.aliases {
            crate::resolver::Qualifier::parse(name)?;
            if alias.name != *name {
                return Err(invalid("Alias name mismatch"));
            }
            crate::resolver::Qualifier::parse(&alias.function_version)?;
        }
        for (qualifier, config) in &record.event_invoke_configs {
            crate::resolver::Qualifier::parse(qualifier)?;
            if config.qualifier != *qualifier {
                return Err(invalid("Event qualifier mismatch"));
            }
        }
        for version in std::iter::once(&record.latest).chain(record.versions.values()) {
            crate::resolver::Qualifier::parse(&version.version)?;
            crate::function_config::validate_execution_settings(version)?;
            if let Some(path) = &version.code_path {
                let artifact = self
                    .artifacts
                    .get(path)
                    .ok_or_else(|| invalid("Code path is not an owned immutable artifact"))?;
                let zip_matches = version.zip_bytes.as_ref().is_some_and(|bytes| {
                    bytes.len() == artifact.zip_bytes().len()
                        && (bytes.as_ptr() == artifact.zip_bytes().as_ptr()
                            || bytes == artifact.zip_bytes())
                });
                if !zip_matches
                    || version.code_sha256 != artifact.sha256()
                    || version.code_size != artifact.zip_bytes().len() as u64
                {
                    return Err(invalid("Deployment package metadata mismatch"));
                }
            } else if version.zip_bytes.is_some() {
                return Err(invalid("ZIP bytes have no owned extracted artifact"));
            }
        }
        Ok(())
    }

    /// Create a new function store with the given code storage directory.
    pub fn new(code_dir: impl Into<PathBuf>) -> Self {
        Self {
            functions: DashMap::new(),
            code_dir: code_dir.into(),
            artifacts: DashMap::new(),
            staging: std::sync::Arc::new(tokio::sync::Semaphore::new(4)),
            changes: watch::channel(()).0,
        }
    }

    #[cfg(unix)]
    pub(crate) fn changes(&self) -> watch::Receiver<()> {
        self.changes.subscribe()
    }

    /// Returns a reference to the code directory path.
    #[must_use]
    pub fn code_dir(&self) -> &Path {
        &self.code_dir
    }

    /// Insert a function record into the store.
    ///
    /// # Errors
    ///
    /// Returns `ResourceConflict` if a function with the same name already exists.
    pub fn insert(&self, record: FunctionRecord) -> Result<(), LambdaServiceError> {
        use dashmap::mapref::entry::Entry;
        self.validate_record(&record)?;
        match self.functions.entry(record.name.clone()) {
            Entry::Occupied(_) => Err(LambdaServiceError::ResourceConflict {
                message: format!("Function already exist: {}", record.name),
            }),
            Entry::Vacant(entry) => {
                entry.insert(record);
                self.changes.send_replace(());
                Ok(())
            }
        }
    }

    /// Get a clone of a function record by name.
    ///
    /// # Errors
    ///
    /// Rejects an invalid logical function name; an absent valid name returns `None`.
    pub fn get(&self, name: &str) -> Result<Option<FunctionRecord>, LambdaServiceError> {
        crate::resolver::FunctionName::parse(name)?;
        Ok(self.functions.get(name).map(|r| r.value().clone()))
    }

    /// Check whether a function exists.
    ///
    /// # Errors
    ///
    /// Rejects an invalid logical function name.
    pub fn contains(&self, name: &str) -> Result<bool, LambdaServiceError> {
        crate::resolver::FunctionName::parse(name)?;
        Ok(self.functions.contains_key(name))
    }

    /// Mutate a function record in place.
    ///
    /// The closure receives a mutable reference to the `FunctionRecord`.
    ///
    /// # Errors
    ///
    /// Returns `FunctionNotFound` if the function does not exist.
    pub fn update<F, R>(&self, name: &str, f: F) -> Result<R, LambdaServiceError>
    where
        F: FnOnce(&mut FunctionRecord) -> R,
    {
        self.update_if_revision(name, None, f)
    }

    /// Atomically replace a validated record only if its latest revision still matches.
    ///
    /// # Errors
    /// Rejects invalid names/state, missing functions, and concurrent updates/recreation.
    pub fn update_if_revision<F, R>(
        &self,
        name: &str,
        expected: Option<&str>,
        f: F,
    ) -> Result<R, LambdaServiceError>
    where
        F: FnOnce(&mut FunctionRecord) -> R,
    {
        self.try_update_if_revision(name, expected, |record| Ok(f(record)))
    }

    /// Apply a fallible mutation to a private candidate and commit only on success.
    ///
    /// # Errors
    /// Rejects stale revisions, invalid records, missing functions or closure failures;
    /// a closure error leaves all live metadata unchanged.
    ///
    /// ```no_run
    /// # fn update(store: &rustack_lambda_core::storage::FunctionStore) -> Result<(), rustack_lambda_core::error::LambdaServiceError> {
    /// store.try_update_if_revision("worker", None, |record| {
    ///     record.latest.description = "updated".into();
    ///     Ok(())
    /// })?;
    /// # Ok(()) }
    /// ```
    pub fn try_update_if_revision<F, R>(
        &self,
        name: &str,
        expected: Option<&str>,
        f: F,
    ) -> Result<R, LambdaServiceError>
    where
        F: FnOnce(&mut FunctionRecord) -> Result<R, LambdaServiceError>,
    {
        crate::resolver::FunctionName::parse(name)?;
        match self.functions.get_mut(name) {
            Some(mut entry) => {
                if expected.is_some_and(|revision| revision != entry.latest.revision_id) {
                    return Err(LambdaServiceError::ResourceConflict {
                        message: "Function changed while deployment was staged".into(),
                    });
                }
                let mut candidate = entry.value().clone();
                let result = f(&mut candidate)?;
                if candidate.name != name || candidate.resource_id != entry.resource_id {
                    return Err(LambdaServiceError::InvalidParameter {
                        message: "Function identity is immutable".into(),
                    });
                }
                self.validate_record(&candidate)?;
                *entry = candidate;
                self.changes.send_replace(());
                Ok(result)
            }
            None => Err(LambdaServiceError::FunctionNotFound {
                name: name.to_owned(),
            }),
        }
    }

    /// Remove a function from the store.
    ///
    /// Returns the removed record, or `None` if it did not exist.
    ///
    /// # Errors
    ///
    /// Rejects an invalid logical function name before changing the store.
    pub fn remove(&self, name: &str) -> Result<Option<FunctionRecord>, LambdaServiceError> {
        crate::resolver::FunctionName::parse(name)?;
        let removed = self.functions.remove(name).map(|(_, v)| v);
        if removed.is_some() {
            self.changes.send_replace(());
        }
        Ok(removed)
    }

    /// List all function records.
    ///
    /// Returns cloned records sorted by function name.
    #[must_use]
    pub fn list(&self) -> Vec<FunctionRecord> {
        let mut records: Vec<FunctionRecord> =
            self.functions.iter().map(|r| r.value().clone()).collect();
        records.sort_by(|a, b| a.name.cmp(&b.name));
        records
    }

    /// Returns the number of stored functions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.functions.len()
    }

    /// Returns `true` if the store is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.functions.is_empty()
    }

    /// Export all function metadata and deployment package bytes.
    #[must_use]
    pub fn export_snapshot(&self) -> FunctionStoreSnapshot {
        FunctionStoreSnapshot {
            functions: self
                .list()
                .into_iter()
                .map(FunctionRecordSnapshot::from_record)
                .collect(),
        }
    }

    /// Replace all functions from a snapshot and rebuild extracted code paths.
    ///
    /// # Errors
    ///
    /// Returns an error when code directory cleanup or zip restoration fails.
    pub async fn import_snapshot(
        &self,
        snapshot: FunctionStoreSnapshot,
    ) -> Result<(), LambdaServiceError> {
        self.validate_root().await?;
        let mut staged = BTreeMap::new();
        let mut identities = HashSet::new();
        let mut artifacts = Vec::new();
        for function in snapshot.functions {
            if !identities.insert(function.resource_id) {
                return Err(LambdaServiceError::InvalidParameter {
                    message: "Duplicate snapshot function resource identity".into(),
                });
            }
            crate::resolver::FunctionName::parse(&function.name)?;
            let (name, qualifier) = crate::resolver::resolve_function_ref(&function.arn)?;
            if name != function.name || qualifier.is_some() {
                return Err(LambdaServiceError::InvalidParameter {
                    message: "Snapshot function ARN mismatch".into(),
                });
            }
            for (alias, record) in &function.aliases {
                crate::resolver::Qualifier::parse(alias)?;
                crate::resolver::Qualifier::parse(&record.function_version)?;
            }
            let (record, allocations) = function.into_record(self).await?;
            artifacts.extend(allocations);
            self.validate_record(&record)?;
            if staged.insert(record.name.clone(), record).is_some() {
                return Err(LambdaServiceError::InvalidParameter {
                    message: "Duplicate snapshot function".into(),
                });
            }
        }
        self.functions.clear();
        for (name, record) in staged {
            self.functions.insert(name, record);
        }
        for artifact in artifacts {
            artifact.commit();
        }
        self.changes.send_replace(());
        Ok(())
    }

    /// Store zip code bytes for a function version and extract them.
    ///
    /// Writes ZIP bytes and extracted contents to a new private, internal-ID artifact.
    /// Logical names and versions never participate in filesystem paths.
    /// Returns the **extracted directory** (which is what the executor needs as
    /// the code root, e.g. for `provided.*` it must contain a `bootstrap`
    /// binary), along with the base64-encoded SHA-256 and the code size.
    ///
    /// Unix file modes from the zip are preserved so executable bits stick.
    /// Every ZIP/IO error propagates. Failed staging never modifies old artifacts.
    /// Completed artifacts remain owned by the store until shutdown/drop.
    ///
    /// # Errors
    ///
    /// Returns `Internal` if directory creation or file writing fails.
    /// Returns `InvalidZipFile` if the archive contains entries that escape
    /// the extraction root (path traversal).
    pub async fn store_zip_code(
        &self,
        function_name: &str,
        version: &str,
        zip_bytes: &Bytes,
    ) -> Result<(PathBuf, String, u64), LambdaServiceError> {
        crate::resolver::FunctionName::parse(function_name)?;
        crate::resolver::Qualifier::parse(version)?;
        if zip_bytes.len() > 50 * 1024 * 1024 {
            return Err(LambdaServiceError::InvalidZipFile {
                message: "ZIP exceeds 50 MiB".into(),
            });
        }
        self.validate_root().await?;
        let permit = std::sync::Arc::clone(&self.staging)
            .try_acquire_owned()
            .map_err(|_| LambdaServiceError::ResourceNotReady {
                message: "ZIP staging capacity exhausted".into(),
            })?;
        let root = self.code_dir.clone();
        let bytes_owned = zip_bytes.clone();
        let task = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            stage_zip(&root, bytes_owned)
        });
        let artifact = tokio::time::timeout(std::time::Duration::from_secs(30), task)
            .await
            .map_err(|_| LambdaServiceError::Internal {
                message: "ZIP staging deadline exceeded".into(),
            })?
            .map_err(|e| LambdaServiceError::Internal {
                message: format!("ZIP extraction task failed: {e}"),
            })??;
        let extracted = artifact.code_root().to_path_buf();
        let sha256 = artifact.sha256().to_owned();
        let code_size = artifact.zip_bytes().len() as u64;
        self.artifacts.insert(extracted.clone(), artifact);

        Ok((extracted, sha256, code_size))
    }

    pub(crate) async fn stage_zip_code(
        &self,
        function_name: &str,
        version: &str,
        bytes: &Bytes,
    ) -> Result<StagedArtifact<'_>, LambdaServiceError> {
        let (path, _, _) = self.store_zip_code(function_name, version, bytes).await?;
        let artifact = self
            .artifacts
            .get(&path)
            .map(|artifact| artifact.value().clone())
            .ok_or_else(|| LambdaServiceError::Internal {
                message: "Newly staged artifact is missing".into(),
            })?;
        Ok(StagedArtifact::new(self, artifact))
    }

    fn discard_staged_artifact(&self, path: &Path) {
        self.artifacts.remove(path);
    }

    /// Validate logical retirement of a function's code.
    ///
    /// Immutable artifacts are retained until store drop so published/in-flight
    /// references remain valid. No name-derived directory is ever deleted.
    ///
    /// # Errors
    ///
    /// Rejects an invalid function name, a symlink/non-directory artifact root, or
    /// filesystem inspection failures.
    pub async fn cleanup_code(&self, function_name: &str) -> Result<(), LambdaServiceError> {
        crate::resolver::FunctionName::parse(function_name)?;
        self.validate_root().await
    }

    /// Check an immutable artifact before handing it to an executor.
    ///
    /// # Errors
    ///
    /// Rejects unowned artifacts, replaced roots, symlinked artifact components/bootstrap,
    /// paths outside the root, or filesystem inspection/canonicalization failures.
    /// An absent bootstrap is accepted here; execution performs its own readiness check.
    pub async fn validate_artifact(&self, path: &Path) -> Result<(), LambdaServiceError> {
        self.validate_root().await?;
        let owned_root = self
            .artifacts
            .get(path)
            .map(|artifact| artifact.storage_root().to_path_buf())
            .ok_or_else(|| LambdaServiceError::InvalidParameter {
                message: "Unowned Lambda artifact".into(),
            })?;
        let root = tokio::fs::canonicalize(&self.code_dir)
            .await
            .map_err(|error| LambdaServiceError::Internal {
                message: format!("Resolve artifact root: {error}"),
            })?;
        if root != owned_root {
            return Err(LambdaServiceError::InvalidParameter {
                message: "Artifact root identity changed".into(),
            });
        }
        let mut current = path;
        while current != root {
            let meta = tokio::fs::symlink_metadata(current)
                .await
                .map_err(|error| LambdaServiceError::Internal {
                    message: format!("Inspect artifact: {error}"),
                })?;
            if meta.file_type().is_symlink() {
                return Err(LambdaServiceError::InvalidParameter {
                    message: "Symlink in Lambda artifact".into(),
                });
            }
            current = current
                .parent()
                .ok_or_else(|| LambdaServiceError::InvalidParameter {
                    message: "Artifact outside storage root".into(),
                })?;
        }
        let resolved =
            tokio::fs::canonicalize(path)
                .await
                .map_err(|error| LambdaServiceError::Internal {
                    message: format!("Resolve artifact: {error}"),
                })?;
        if !resolved.starts_with(root) {
            return Err(LambdaServiceError::InvalidParameter {
                message: "Artifact escapes storage root".into(),
            });
        }
        let bootstrap = path.join("bootstrap");
        match tokio::fs::symlink_metadata(bootstrap).await {
            Ok(meta) if meta.file_type().is_symlink() => {
                Err(LambdaServiceError::InvalidParameter {
                    message: "Bootstrap cannot be a symlink".into(),
                })
            }
            Ok(_) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(LambdaServiceError::Internal {
                message: format!("Inspect bootstrap: {error}"),
            }),
        }
    }

    /// Issue a shared artifact lease after bounded ownership/filesystem validation.
    ///
    /// # Errors
    /// Rejects unowned or replaced paths, filesystem failures and validation timeout.
    ///
    /// ```no_run
    /// # async fn capture(store: &rustack_lambda_core::storage::FunctionStore, path: &std::path::Path)
    /// # -> Result<(), rustack_lambda_core::error::LambdaServiceError> {
    /// let artifact = store.capture_artifact(path).await?;
    /// assert_eq!(artifact.code_root(), path);
    /// # Ok(()) }
    /// ```
    pub async fn capture_artifact(
        &self,
        path: &Path,
    ) -> Result<ValidatedArtifact, LambdaServiceError> {
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            self.validate_artifact(path),
        )
        .await
        .map_err(|_| LambdaServiceError::ResourceNotReady {
            message: "Artifact validation deadline exceeded".into(),
        })??;
        self.artifacts
            .get(path)
            .map(|artifact| artifact.value().clone())
            .ok_or_else(|| LambdaServiceError::InvalidParameter {
                message: "Unowned Lambda artifact".into(),
            })
    }

    /// Reject a symlink at the operator-selected artifact root.
    ///
    /// # Errors
    ///
    /// Rejects a symlink/non-directory root or filesystem inspection errors; an absent
    /// root is valid before first staging.
    pub async fn validate_root(&self) -> Result<(), LambdaServiceError> {
        match tokio::fs::symlink_metadata(&self.code_dir).await {
            Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
                Err(LambdaServiceError::InvalidParameter {
                    message: "Lambda artifact root must be a real directory".into(),
                })
            }
            Ok(_) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(LambdaServiceError::Internal {
                message: format!("Inspect artifact root: {error}"),
            }),
        }
    }
}

/// Stage a package in a freshly created private directory on a blocking worker.
#[allow(clippy::disallowed_methods, clippy::disallowed_types)] // Synchronous filesystem work is isolated to spawn_blocking.
pub(crate) fn stage_zip(
    root: &Path,
    bytes: Bytes,
) -> Result<ValidatedArtifact, LambdaServiceError> {
    let io_error = |source| LambdaServiceError::ArtifactIo { source };
    std::fs::create_dir_all(root).map_err(io_error)?;
    if std::fs::symlink_metadata(root)
        .map_err(io_error)?
        .file_type()
        .is_symlink()
    {
        return Err(LambdaServiceError::InvalidParameter {
            message: "Artifact root is a symlink".into(),
        });
    }
    let root = std::fs::canonicalize(root).map_err(io_error)?;
    let artifact = tempfile::Builder::new()
        .prefix("artifact-")
        .tempdir_in(&root)
        .map_err(io_error)?;
    let extracted = artifact.path().join("extracted");
    let result = (|| {
        std::fs::create_dir(&extracted).map_err(io_error)?;
        std::fs::write(artifact.path().join("code.zip"), &bytes).map_err(io_error)?;
        extract_zip(&bytes, &extracted, MAX_EXTRACTED_SIZE)
    })();
    if let Err(error) = result {
        artifact.close().map_err(io_error)?;
        return Err(error);
    }
    Ok(ValidatedArtifact::from_staged(
        artifact, root, extracted, bytes,
    ))
}

/// Maximum extracted deployment package size (250 MB, mirroring the AWS
/// unzipped-package limit). Bounds zip-bomb expansion during extraction.
const MAX_EXTRACTED_SIZE: u64 = 250 * 1024 * 1024;

/// Extract a zip archive into `target`, preserving unix file modes.
///
/// Rejects entries whose normalized path escapes `target` (path traversal)
/// and archives whose entries would expand beyond [`MAX_EXTRACTED_SIZE`]
/// (zip bombs). ZIP and filesystem failures are always returned to the caller.
///
/// Synchronous `std::fs` is intentional: this runs inside `spawn_blocking` and
/// the `zip` crate's reader API is itself blocking, so wrapping each I/O in
/// tokio would only add overhead.
#[allow(clippy::disallowed_methods, clippy::disallowed_types)]
fn extract_zip(
    zip_bytes: &[u8],
    target: &Path,
    max_extracted: u64,
) -> Result<(), LambdaServiceError> {
    use std::{
        fs::{self, File},
        io::{self, Cursor, Read as _, Write as _},
    };

    let cursor = Cursor::new(zip_bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| LambdaServiceError::InvalidZipFile {
            message: format!("not a valid zip archive: {e}"),
        })?;

    if archive.has_duplicate_names() {
        return Err(LambdaServiceError::InvalidZipFile {
            message: "Duplicate ZIP entry".into(),
        });
    }
    if archive.len() > 10_000 {
        return Err(LambdaServiceError::InvalidZipFile {
            message: "Too many ZIP entries".into(),
        });
    }
    let mut paths = std::collections::HashSet::new();
    let mut extracted_total: u64 = 0;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| LambdaServiceError::InvalidZipFile {
                message: format!("zip entry {i}: {e}"),
            })?;
        let rel = validated_zip_path(&entry)?;
        if !paths.insert(rel.clone()) {
            return Err(LambdaServiceError::InvalidZipFile {
                message: "Duplicate ZIP entry".into(),
            });
        }
        let out_path = target.join(&rel);
        // Defense in depth: ensure the resolved path stays within target.
        if !out_path.starts_with(target) {
            return Err(LambdaServiceError::InvalidZipFile {
                message: "ZIP entry escapes extraction root".into(),
            });
        }
        if entry.is_dir() {
            fs::create_dir_all(&out_path).map_err(|e| LambdaServiceError::Internal {
                message: format!("create dir {}: {e}", out_path.display()),
            })?;
            continue;
        }
        // Zip-bomb guard: reject entries whose declared uncompressed size
        // exceeds the remaining budget, and cap actual bytes copied.
        let remaining = max_extracted.saturating_sub(extracted_total);
        if entry.size() > remaining {
            return Err(LambdaServiceError::InvalidZipFile {
                message: format!("ZIP archive expands beyond {max_extracted} bytes"),
            });
        }
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|e| LambdaServiceError::Internal {
                message: format!("create parent {}: {e}", parent.display()),
            })?;
        }
        let mut out = File::create_new(&out_path)
            .map_err(|source| LambdaServiceError::ArtifactIo { source })?;
        let copied = io::copy(
            &mut (&mut entry).take(remaining.saturating_add(1)),
            &mut out,
        )
        .map_err(|source| {
            if source.kind() == ErrorKind::InvalidData {
                LambdaServiceError::InvalidZipFile {
                    message: format!("ZIP CRC/decompression failed: {source}"),
                }
            } else {
                LambdaServiceError::ArtifactIo { source }
            }
        })?;
        extracted_total = extracted_total.saturating_add(copied);
        if extracted_total > max_extracted {
            return Err(LambdaServiceError::InvalidZipFile {
                message: format!("zip archive expands beyond {max_extracted} bytes"),
            });
        }
        out.flush().map_err(|e| LambdaServiceError::Internal {
            message: format!("flush file {}: {e}", out_path.display()),
        })?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&out_path, fs::Permissions::from_mode(mode)).map_err(|e| {
                LambdaServiceError::Internal {
                    message: format!("chmod {}: {e}", out_path.display()),
                }
            })?;
        }
    }
    Ok(())
}

/// Check original names and attributes before ZIP9 normalizes decoded metadata.
fn validated_zip_path<R: Read + ?Sized>(
    entry: &ZipFile<'_, R>,
) -> Result<PathBuf, LambdaServiceError> {
    let invalid = || LambdaServiceError::InvalidZipFile {
        message: "Unsafe ZIP entry name or attributes".into(),
    };
    let raw_name = entry.name_raw();
    if raw_name.len() > 1024
        || (entry.flags().as_u16() & (1 << 11) != 0 && from_utf8(raw_name).is_err())
    {
        return Err(invalid());
    }
    let name = entry.name().map_err(|_| invalid())?;
    let mode = if entry.system() == System::Dos {
        0
    } else {
        entry.external_attributes() >> 16
    };
    if name.len() > 1024
        || name.contains('\\')
        || name.split('/').any(|component| component == "..")
        || mode & 0o170_000 == 0o120_000
        || mode & 0o7000 != 0
    {
        return Err(invalid());
    }
    entry.enclosed_name().ok_or_else(invalid)
}

/// Compute base64-encoded SHA-256 hash of the given data.
#[must_use]
pub fn compute_sha256(data: &[u8]) -> String {
    use base64::Engine;
    let hash = Sha256::digest(data);
    base64::engine::general_purpose::STANDARD.encode(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_should_release_unpublished_artifacts_and_preserve_committed_code()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let store = FunctionStore::new(root.path());
        let bytes = Bytes::from(crate::test_zip::minimal_zip(b"code"));
        let (committed, _, _) = store.store_zip_code("committed", "$LATEST", &bytes).await?;
        let pending = store.stage_zip_code("pending", "$LATEST", &bytes).await?;
        let pending_path = pending.artifact().code_root().to_path_buf();
        assert_eq!(store.artifacts.len(), 2);
        drop(pending);
        assert_eq!(store.artifacts.len(), 1);
        assert!(tokio::fs::try_exists(committed).await?);
        assert!(!tokio::fs::try_exists(pending_path).await?);
        Ok(())
    }

    #[tokio::test]
    async fn test_should_rollback_unpublished_code_when_its_owner_future_is_aborted()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let store = std::sync::Arc::new(FunctionStore::new(root.path()));
        let owner = std::sync::Arc::clone(&store);
        let (started, receiver) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let bytes = Bytes::from(crate::test_zip::minimal_zip(b"pending"));
            let staged = owner.stage_zip_code("cancelled", "$LATEST", &bytes).await?;
            let _ = started.send(staged.artifact().code_root().to_path_buf());
            std::future::pending::<()>().await;
            staged.commit();
            Ok::<(), LambdaServiceError>(())
        });
        let _cancel = crate::work::CancelOnDrop(task.abort_handle());
        let path = tokio::time::timeout(std::time::Duration::from_secs(5), receiver).await??;
        task.abort();
        assert!(task.await.is_err_and(|error| error.is_cancelled()));
        assert!(store.artifacts.is_empty());
        assert!(!tokio::fs::try_exists(path).await?);
        Ok(())
    }

    #[tokio::test]
    async fn test_should_rollback_snapshot_artifacts_when_later_validation_rejects_restore()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let store = FunctionStore::new(root.path());
        store.insert(sample_record("live"))?;
        let bytes = Bytes::from(crate::test_zip::minimal_zip(b"unpublished"));
        let mut candidate = sample_record("candidate");
        candidate.latest.code_sha256 = "mismatched-snapshot-hash".into();
        candidate.latest.code_size = bytes.len() as u64;
        candidate.latest.zip_bytes = Some(bytes);
        let snapshot = FunctionStoreSnapshot {
            functions: vec![FunctionRecordSnapshot::from_record(candidate)],
        };
        assert!(matches!(
            store.import_snapshot(snapshot).await,
            Err(LambdaServiceError::InvalidParameter { .. })
        ));
        assert!(store.contains("live")?);
        assert_eq!(store.artifacts.len(), 0);
        assert!(
            tokio::fs::read_dir(root.path())
                .await?
                .next_entry()
                .await?
                .is_none()
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_should_validate_snapshot_settings_before_decoding_or_staging_its_archive()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let store = FunctionStore::new(root.path());
        let mut snapshot = FunctionRecordSnapshot::from_record(sample_record("invalid-settings"));
        snapshot.latest.timeout = 0;
        snapshot.latest.zip_bytes_base64 = Some("invalid-base64".into());
        let result = store
            .import_snapshot(FunctionStoreSnapshot {
                functions: vec![snapshot],
            })
            .await;
        assert!(matches!(
            result,
            Err(LambdaServiceError::InvalidParameter { .. })
        ));
        assert!(store.is_empty());
        assert!(store.artifacts.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn test_should_keep_code_alive_until_the_last_invocation_lease_releases_it()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let store = FunctionStore::new(root.path());
        let bytes = Bytes::from(crate::test_zip::minimal_zip(b"owned-code"));
        let (path, sha256, size) = store.store_zip_code("leased", "$LATEST", &bytes).await?;
        let lease = store.capture_artifact(&path).await?;
        assert_eq!(lease.zip_bytes().as_ptr(), bytes.as_ptr());
        assert_eq!(lease.sha256(), sha256);
        assert_eq!(lease.zip_bytes().len() as u64, size);
        drop(store);
        assert!(tokio::fs::metadata(&path).await?.is_dir());
        assert_eq!(
            tokio::fs::read(path.join("fixture.txt")).await?,
            b"owned-code"
        );
        let retained = lease.clone();
        drop(lease);
        assert!(tokio::fs::metadata(&path).await?.is_dir());
        drop(retained);
        assert!(
            matches!(tokio::fs::metadata(path).await, Err(error) if error.kind() == ErrorKind::NotFound)
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_should_reject_unowned_code_when_issuing_an_artifact_lease()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let store = FunctionStore::new(root.path());
        assert!(matches!(
            store.capture_artifact(root.path()).await,
            Err(LambdaServiceError::InvalidParameter { .. })
        ));
        Ok(())
    }

    #[tokio::test]
    async fn test_should_reject_hostile_names_at_every_store_boundary() {
        let root = tempfile::tempdir().unwrap();
        let store = FunctionStore::new(root.path());
        let zip = crate::test_zip::minimal_zip(b"safe");
        for name in [
            "../escape",
            "/absolute",
            "a/b",
            "a\\b",
            "a%2fb",
            "..",
            "",
            "é",
        ] {
            assert!(store.insert(sample_record(name)).is_err());
            assert!(store.get(name).is_err());
            assert!(store.contains(name).is_err());
            assert!(store.remove(name).is_err());
            assert!(store.update(name, |_| ()).is_err());
            assert!(
                store
                    .store_zip_code(name, "$LATEST", &Bytes::copy_from_slice(&zip))
                    .await
                    .is_err()
            );
            assert!(store.cleanup_code(name).await.is_err());
        }
        assert!(store.is_empty());
        assert!(
            tokio::fs::read_dir(root.path())
                .await
                .unwrap()
                .next_entry()
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_should_stage_immutable_artifacts_and_preserve_old_on_bad_zip() {
        let root = tempfile::tempdir().unwrap();
        let store = FunctionStore::new(root.path());
        let (a, _, _) = store
            .store_zip_code(
                "safe",
                "$LATEST",
                &Bytes::from(crate::test_zip::minimal_zip(b"A")),
            )
            .await
            .unwrap();
        assert!(matches!(
            store
                .store_zip_code("safe", "$LATEST", &Bytes::from_static(b"bad ZIP"))
                .await,
            Err(LambdaServiceError::InvalidZipFile { .. })
        ));
        let (b, _, _) = store
            .store_zip_code(
                "safe",
                "$LATEST",
                &Bytes::from(crate::test_zip::minimal_zip(b"B")),
            )
            .await
            .unwrap();
        assert_ne!(a, b);
        assert_eq!(tokio::fs::read(a.join("fixture.txt")).await.unwrap(), b"A");
        assert_eq!(tokio::fs::read(b.join("fixture.txt")).await.unwrap(), b"B");
        assert!(
            !a.components()
                .any(|component| component.as_os_str() == "safe")
        );
    }

    #[tokio::test]
    async fn test_should_propagate_crc_and_staging_io_errors_without_replacing_old_artifact() {
        use std::io::{Cursor, Write};
        let root = tempfile::tempdir().unwrap();
        let store = FunctionStore::new(root.path());
        let (old, _, _) = store
            .store_zip_code(
                "safe",
                "$LATEST",
                &Bytes::from(crate::test_zip::minimal_zip(b"old")),
            )
            .await
            .unwrap();
        let mut corrupt = crate::test_zip::minimal_zip(b"crc-marker");
        let position = corrupt
            .windows(10)
            .position(|bytes| bytes == b"crc-marker")
            .unwrap();
        corrupt[position] ^= 1;
        assert!(
            store
                .store_zip_code("safe", "$LATEST", &Bytes::copy_from_slice(&corrupt))
                .await
                .is_err()
        );
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file("file", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"file").unwrap();
        writer
            .start_file("file/child", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"cannot create child below file").unwrap();
        let conflict = writer.finish().unwrap().into_inner();
        assert!(
            store
                .store_zip_code("safe", "$LATEST", &Bytes::copy_from_slice(&conflict))
                .await
                .is_err()
        );
        assert_eq!(
            tokio::fs::read(old.join("fixture.txt")).await.unwrap(),
            b"old"
        );
        assert_eq!(
            store.artifacts.len(),
            1,
            "failed staging cannot publish an artifact"
        );
        let mut dirs = tokio::fs::read_dir(root.path()).await.unwrap();
        let mut count = 0;
        while dirs.next_entry().await.unwrap().is_some() {
            count += 1;
        }
        assert_eq!(count, 1, "failed staging directories are removed");
    }

    #[tokio::test]
    async fn test_should_preserve_store_on_invalid_snapshot_and_update_identity() {
        let root = tempfile::tempdir().unwrap();
        let store = FunctionStore::new(root.path());
        store.insert(sample_record("safe")).unwrap();
        let mut snapshot = store.export_snapshot();
        snapshot.functions.first_mut().unwrap().name = "../escape".into();
        assert!(store.import_snapshot(snapshot).await.is_err());
        assert!(store.get("safe").unwrap().is_some());
        assert!(
            store
                .update("safe", |record| record.name = "../escape".into())
                .is_err()
        );
        assert_eq!(store.get("safe").unwrap().unwrap().name, "safe");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_should_reject_symlink_root_and_replaced_artifact_without_touching_sentinel() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let sentinel = outside.path().join("sentinel");
        tokio::fs::write(&sentinel, b"untouched").await.unwrap();
        let link = root.path().join("linked");
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        let store = FunctionStore::new(&link);
        assert!(
            store
                .store_zip_code(
                    "safe",
                    "$LATEST",
                    &Bytes::from(crate::test_zip::minimal_zip(b"A"))
                )
                .await
                .is_err()
        );
        assert!(store.cleanup_code("safe").await.is_err());
        assert!(
            store
                .import_snapshot(FunctionStoreSnapshot::default())
                .await
                .is_err()
        );
        let store = FunctionStore::new(root.path().join("owned"));
        let (artifact, _, _) = store
            .store_zip_code(
                "safe",
                "$LATEST",
                &Bytes::from(crate::test_zip::minimal_zip(b"A")),
            )
            .await
            .unwrap();
        tokio::fs::remove_dir_all(&artifact).await.unwrap();
        std::os::unix::fs::symlink(outside.path(), &artifact).unwrap();
        assert!(store.validate_artifact(&artifact).await.is_err());
        drop(store);
        assert_eq!(tokio::fs::read(&sentinel).await.unwrap(), b"untouched");
        let managed = root.path().join("replaceable");
        let store = FunctionStore::new(&managed);
        let (artifact, _, _) = store
            .store_zip_code(
                "safe",
                "$LATEST",
                &Bytes::from(crate::test_zip::minimal_zip(b"A")),
            )
            .await
            .unwrap();
        let artifact_id = artifact.parent().unwrap().file_name().unwrap();
        let outside_artifact = outside.path().join(artifact_id);
        tokio::fs::create_dir(&outside_artifact).await.unwrap();
        tokio::fs::write(outside_artifact.join("sentinel"), b"outside")
            .await
            .unwrap();
        tokio::fs::rename(&managed, root.path().join("original-root"))
            .await
            .unwrap();
        std::os::unix::fs::symlink(outside.path(), &managed).unwrap();
        assert!(store.cleanup_code("safe").await.is_err());
        drop(store);
        assert_eq!(
            tokio::fs::read(outside_artifact.join("sentinel"))
                .await
                .unwrap(),
            b"outside"
        );
    }

    fn test_store() -> FunctionStore {
        FunctionStore::new("/tmp/rustack-lambda-test")
    }

    fn sample_version() -> VersionRecord {
        VersionRecord {
            version: "$LATEST".to_owned(),
            runtime: Some("python3.12".to_owned()),
            handler: Some("index.handler".to_owned()),
            role: "arn:aws:iam::000000000000:role/test-role".to_owned(),
            description: String::new(),
            timeout: 3,
            memory_size: 128,
            environment: HashMap::new(),
            package_type: "Zip".to_owned(),
            code_path: None,
            image_uri: None,
            image: ImageArtifact::Unresolved,
            zip_bytes: None,
            deployment: DeploymentState::Ready,
            last_modified: "2024-01-01T00:00:00.000+0000".to_owned(),
            architectures: vec!["x86_64".to_owned()],
            ephemeral_storage_size: 512,
            code_sha256: "abc123".to_owned(),
            code_size: 0,
            revision_id: uuid::Uuid::new_v4().to_string(),
            layers: Vec::new(),
            vpc_config: None,
            dead_letter_config: None,
            tracing_config: None,
            image_config: None,
            logging_config: None,
            snap_start: None,
        }
    }

    fn sample_record(name: &str) -> FunctionRecord {
        FunctionRecord {
            resource_id: FunctionResourceId::default(),
            name: name.to_owned(),
            arn: format!("arn:aws:lambda:us-east-1:000000000000:function:{name}"),
            latest: sample_version(),
            versions: BTreeMap::new(),
            next_version: 2,
            aliases: HashMap::new(),
            policy: PolicyDocument::default(),
            tags: HashMap::new(),
            code_signing_config_arn: None,
            url_config: None,
            reserved_concurrent_executions: None,
            event_invoke_configs: HashMap::new(),
            created_at: "2024-01-01T00:00:00.000+0000".to_owned(),
        }
    }

    #[tokio::test]
    async fn test_should_preserve_function_resource_identity_through_snapshot_and_updates()
    -> Result<(), Box<dyn std::error::Error>> {
        let store = test_store();
        let record = sample_record("identity");
        let identity = record.resource_id;
        store.insert(record)?;
        store.update("identity", |record| {
            record.latest.description = "changed".into();
        })?;
        let saved = serde_json::to_vec(&store.export_snapshot())?;
        let snapshot: FunctionStoreSnapshot = serde_json::from_slice(&saved)?;
        assert_eq!(
            snapshot.functions.first().map(|record| record.resource_id),
            Some(identity)
        );
        let restored = test_store();
        restored.import_snapshot(snapshot).await?;
        assert_eq!(
            restored.get("identity")?.map(|record| record.resource_id),
            Some(identity)
        );
        let result = restored.update("identity", |record| {
            record.resource_id = FunctionResourceId::default();
        });
        assert!(matches!(
            result,
            Err(LambdaServiceError::InvalidParameter { .. })
        ));
        assert_eq!(
            restored.get("identity")?.map(|record| record.resource_id),
            Some(identity)
        );
        restored.remove("identity")?;
        restored.insert(sample_record("identity"))?;
        assert_ne!(
            restored.get("identity")?.map(|record| record.resource_id),
            Some(identity)
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_should_reject_duplicate_snapshot_identities_without_replacing_live_store()
    -> Result<(), LambdaServiceError> {
        let store = test_store();
        store.insert(sample_record("live"))?;
        let mut first = sample_record("first");
        let second = sample_record("second");
        first.resource_id = second.resource_id;
        let snapshot = FunctionStoreSnapshot {
            functions: vec![
                FunctionRecordSnapshot::from_record(first),
                FunctionRecordSnapshot::from_record(second),
            ],
        };
        assert!(matches!(
            store.import_snapshot(snapshot).await,
            Err(LambdaServiceError::InvalidParameter { .. })
        ));
        assert!(store.contains("live")?);
        assert_eq!(store.len(), 1);
        Ok(())
    }

    #[test]
    fn test_should_assign_identity_only_when_legacy_snapshot_omits_it()
    -> Result<(), Box<dyn std::error::Error>> {
        let snapshot = FunctionRecordSnapshot::from_record(sample_record("legacy"));
        let mut value = serde_json::to_value(snapshot)?;
        let fields = value.as_object_mut().ok_or("snapshot must be an object")?;
        fields.remove("resourceId");
        let restored: FunctionRecordSnapshot = serde_json::from_value(value.clone())?;
        let saved = serde_json::to_value(&restored)?;
        assert_eq!(
            serde_json::from_value::<FunctionRecordSnapshot>(saved)?.resource_id,
            restored.resource_id
        );
        value
            .as_object_mut()
            .ok_or("snapshot must be an object")?
            .insert(
                "resourceId".into(),
                "00000000-0000-0000-0000-000000000000".into(),
            );
        assert!(serde_json::from_value::<FunctionRecordSnapshot>(value).is_err());
        Ok(())
    }

    #[test]
    fn test_should_insert_and_get_function() {
        let store = test_store();
        let record = sample_record("my-func");
        store.insert(record).unwrap();

        let retrieved = store.get("my-func").unwrap().unwrap();
        assert_eq!(retrieved.name, "my-func");
    }

    #[test]
    fn test_should_reject_duplicate_insert() {
        let store = test_store();
        store.insert(sample_record("my-func")).unwrap();
        let err = store.insert(sample_record("my-func")).unwrap_err();
        assert!(matches!(err, LambdaServiceError::ResourceConflict { .. }));
    }

    #[test]
    fn test_should_update_function() {
        let store = test_store();
        store.insert(sample_record("my-func")).unwrap();

        store
            .update("my-func", |rec| {
                rec.latest.timeout = 30;
            })
            .unwrap();

        let retrieved = store.get("my-func").unwrap().unwrap();
        assert_eq!(retrieved.latest.timeout, 30);
    }

    #[test]
    fn test_should_error_on_update_nonexistent() {
        let store = test_store();
        let err = store.update("no-such-func", |_| {}).unwrap_err();
        assert!(matches!(err, LambdaServiceError::FunctionNotFound { .. }));
    }

    #[test]
    fn test_should_remove_function() {
        let store = test_store();
        store.insert(sample_record("my-func")).unwrap();
        assert!(store.contains("my-func").unwrap());

        let removed = store.remove("my-func").unwrap();
        assert!(removed.is_some());
        assert!(!store.contains("my-func").unwrap());
    }

    #[test]
    fn test_should_list_functions_sorted() {
        let store = test_store();
        store.insert(sample_record("charlie")).unwrap();
        store.insert(sample_record("alpha")).unwrap();
        store.insert(sample_record("bravo")).unwrap();

        let list = store.list();
        let names: Vec<&str> = list.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["alpha", "bravo", "charlie"]);
    }

    #[test]
    fn test_should_report_length() {
        let store = test_store();
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);

        store.insert(sample_record("a")).unwrap();
        store.insert(sample_record("b")).unwrap();
        assert!(!store.is_empty());
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn test_should_compute_sha256() {
        use base64::Engine;

        let hash = compute_sha256(b"hello world");
        // Known SHA-256 of "hello world" base64-encoded.
        assert_ne!(hash, "");
        // Verify it is base64 by checking it decodes.
        let decoded = base64::engine::general_purpose::STANDARD.decode(&hash);
        assert!(decoded.is_ok());
        assert_eq!(decoded.unwrap().len(), 32);
    }

    // ---- Layer store tests ----

    #[test]
    fn test_should_publish_and_get_layer_version() {
        let store = LayerStore::new();
        let ver = LayerVersionRecord {
            version: 0,
            description: "test".to_owned(),
            compatible_runtimes: vec!["python3.12".to_owned()],
            compatible_architectures: Vec::new(),
            license_info: None,
            code_sha256: "abc".to_owned(),
            code_size: 100,
            created_date: "2024-01-01".to_owned(),
            layer_arn: "arn:layer".to_owned(),
            layer_version_arn: "arn:layer:1".to_owned(),
            policy: PolicyDocument::default(),
        };

        let num = store.publish_version("my-layer", "arn:layer", ver);
        assert_eq!(num, 1);

        let retrieved = store.get_version("my-layer", 1);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().description, "test");
    }

    #[test]
    fn test_should_list_layer_versions() {
        let store = LayerStore::new();
        for i in 0..3 {
            let ver = LayerVersionRecord {
                version: 0,
                description: format!("v{i}"),
                compatible_runtimes: Vec::new(),
                compatible_architectures: Vec::new(),
                license_info: None,
                code_sha256: "abc".to_owned(),
                code_size: 100,
                created_date: "2024-01-01".to_owned(),
                layer_arn: "arn:layer".to_owned(),
                layer_version_arn: format!("arn:layer:{}", i + 1),
                policy: PolicyDocument::default(),
            };
            let _ = store.publish_version("my-layer", "arn:layer", ver);
        }

        let versions = store.list_versions("my-layer");
        assert_eq!(versions.len(), 3);
    }

    #[test]
    fn test_should_delete_layer_version_and_cleanup() {
        let store = LayerStore::new();
        let ver = LayerVersionRecord {
            version: 0,
            description: String::new(),
            compatible_runtimes: Vec::new(),
            compatible_architectures: Vec::new(),
            license_info: None,
            code_sha256: "abc".to_owned(),
            code_size: 0,
            created_date: "2024-01-01".to_owned(),
            layer_arn: "arn:layer".to_owned(),
            layer_version_arn: "arn:layer:1".to_owned(),
            policy: PolicyDocument::default(),
        };
        let _ = store.publish_version("my-layer", "arn:layer", ver);

        assert!(store.delete_version("my-layer", 1));
        // Layer record should be removed since no versions remain.
        assert!(store.get("my-layer").is_none());
    }

    #[test]
    fn test_should_list_layers_sorted() {
        let store = LayerStore::new();
        let make_ver = || LayerVersionRecord {
            version: 0,
            description: String::new(),
            compatible_runtimes: Vec::new(),
            compatible_architectures: Vec::new(),
            license_info: None,
            code_sha256: "abc".to_owned(),
            code_size: 0,
            created_date: "2024-01-01".to_owned(),
            layer_arn: String::new(),
            layer_version_arn: String::new(),
            policy: PolicyDocument::default(),
        };

        let _ = store.publish_version("charlie", "arn:charlie", make_ver());
        let _ = store.publish_version("alpha", "arn:alpha", make_ver());
        let _ = store.publish_version("bravo", "arn:bravo", make_ver());

        let layers = store.list_layers();
        let names: Vec<&str> = layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["alpha", "bravo", "charlie"]);
    }

    #[tokio::test]
    async fn test_should_store_and_cleanup_zip_code() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FunctionStore::new(tmp.path());

        let zip_data = crate::test_zip::minimal_zip(b"fixture");
        let (dir, sha256, size) = store
            .store_zip_code("test-func", "$LATEST", &Bytes::copy_from_slice(&zip_data))
            .await
            .unwrap();

        assert!(
            dir.exists(),
            "extracted dir should exist: {}",
            dir.display()
        );
        assert_eq!(dir.file_name().and_then(|s| s.to_str()), Some("extracted"));
        // Raw zip is preserved alongside the extracted dir.
        let parent = dir.parent().expect("extracted dir has a parent");
        assert!(parent.join("code.zip").exists());
        assert_ne!(sha256, "");
        assert_eq!(size, zip_data.len() as u64);

        store.cleanup_code("test-func").await.unwrap();
        assert!(
            dir.exists(),
            "in-flight immutable artifacts survive logical deletion"
        );
        drop(store);
        assert!(!dir.exists());
    }

    #[tokio::test]
    async fn test_should_extract_real_zip_and_preserve_exec_mode() {
        use std::io::Write as _;

        let tmp = tempfile::tempdir().unwrap();
        let store = FunctionStore::new(tmp.path());

        // Build a real zip in memory with two entries; the first is marked
        // executable (mode 0o755) — typical for a `bootstrap` binary.
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let exe_opts: zip::write::SimpleFileOptions =
                zip::write::SimpleFileOptions::default().unix_permissions(0o755);
            w.start_file("bootstrap", exe_opts).unwrap();
            w.write_all(b"#!/bin/sh\necho hi\n").unwrap();
            let plain_opts: zip::write::SimpleFileOptions =
                zip::write::SimpleFileOptions::default().unix_permissions(0o644);
            w.start_file("README.txt", plain_opts).unwrap();
            w.write_all(b"hello").unwrap();
            w.finish().unwrap();
        }

        let (dir, _sha, _size) = store
            .store_zip_code("real-func", "$LATEST", &Bytes::copy_from_slice(&buf))
            .await
            .unwrap();

        let bootstrap = dir.join("bootstrap");
        assert!(bootstrap.exists(), "bootstrap should be extracted");
        assert!(dir.join("README.txt").exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = tokio::fs::metadata(&bootstrap)
                .await
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o755, "bootstrap should keep its exec bit");
        }
    }

    #[tokio::test]
    async fn test_should_reject_zip_with_path_traversal() {
        use std::io::Write as _;

        let tmp = tempfile::tempdir().unwrap();
        let store = FunctionStore::new(tmp.path());

        // Construct a zip that targets `../escape`, which `enclosed_name`
        // rejects. We must bypass the high-level helper to actually emit such
        // a name; using the raw API.
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
            w.start_file("../escape.txt", opts).unwrap();
            w.write_all(b"oops").unwrap();
            w.finish().unwrap();
        }

        let err = store
            .store_zip_code("evil", "$LATEST", &Bytes::copy_from_slice(&buf))
            .await
            .unwrap_err();
        assert!(matches!(err, LambdaServiceError::InvalidZipFile { .. }));
    }

    #[rstest::rstest]
    #[case::invalid_utf8(vec![0xff; 11], true)]
    #[case::raw_limit(vec![b'a'; 1025], true)]
    #[case::decoded_limit(vec![0x82; 513], false)]
    #[tokio::test]
    async fn test_should_reject_invalid_zip_names_without_publishing_artifacts(
        #[case] raw: Vec<u8>,
        #[case] utf8: bool,
    ) {
        use std::io::{Cursor, Write};

        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file(
                "a".repeat(raw.len()),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.write_all(b"payload").unwrap();
        let mut bytes = writer.finish().unwrap().into_inner();
        let central = bytes
            .windows(4)
            .position(|part| part == b"PK\x01\x02")
            .unwrap();
        let flag = if utf8 { 1u16 << 11 } else { 0 };
        bytes
            .get_mut(6..8)
            .unwrap()
            .copy_from_slice(&flag.to_le_bytes());
        bytes
            .get_mut(central + 8..central + 10)
            .unwrap()
            .copy_from_slice(&flag.to_le_bytes());
        bytes
            .get_mut(30..30 + raw.len())
            .unwrap()
            .copy_from_slice(&raw);
        bytes
            .get_mut(central + 46..central + 46 + raw.len())
            .unwrap()
            .copy_from_slice(&raw);

        let root = tempfile::tempdir().unwrap();
        let store = FunctionStore::new(root.path());
        let error = store
            .store_zip_code("safe", "$LATEST", &Bytes::from(bytes))
            .await
            .unwrap_err();
        assert!(matches!(error, LambdaServiceError::InvalidZipFile { .. }));
        assert!(store.artifacts.is_empty());
        let mut entries = tokio::fs::read_dir(root.path()).await.unwrap();
        assert!(entries.next_entry().await.unwrap().is_none());
    }

    #[rstest::rstest]
    #[case::symlink(0o120_777)]
    #[case::setuid(0o104_755)]
    #[case::setgid(0o102_755)]
    #[case::sticky(0o101_755)]
    #[tokio::test]
    async fn test_should_reject_hostile_zip_attributes_without_replacing_code(#[case] mode: u32) {
        use std::io::Cursor;

        let root = tempfile::tempdir().unwrap();
        let code_dir = root.path().join("code");
        let sentinel = root.path().join("sentinel");
        tokio::fs::write(&sentinel, b"outside").await.unwrap();
        let store = FunctionStore::new(&code_dir);
        let (old, _, _) = store
            .store_zip_code(
                "safe",
                "$LATEST",
                &Bytes::from(crate::test_zip::minimal_zip(b"old")),
            )
            .await
            .unwrap();

        // Encode the central-directory attributes directly: safe writer APIs
        // deliberately strip special permission bits. The symlink payload would
        // target the outside sentinel if materialized as a link.
        let mut hostile = crate::test_zip::minimal_zip(sentinel.as_os_str().as_encoded_bytes());
        let central = hostile
            .windows(4)
            .position(|bytes| bytes == b"PK\x01\x02")
            .unwrap();
        hostile
            .get_mut(central + 4..central + 6)
            .unwrap()
            .copy_from_slice(&((3u16 << 8) | 0x14).to_le_bytes());
        hostile
            .get_mut(central + 38..central + 42)
            .unwrap()
            .copy_from_slice(&(mode << 16).to_le_bytes());
        let mut archive = zip::ZipArchive::new(Cursor::new(&hostile)).unwrap();
        assert_eq!(
            archive.by_index(0).unwrap().external_attributes() >> 16,
            mode
        );

        let error = store
            .store_zip_code("safe", "$LATEST", &Bytes::from(hostile))
            .await
            .unwrap_err();
        assert!(matches!(error, LambdaServiceError::InvalidZipFile { .. }));
        assert_eq!(tokio::fs::read(&sentinel).await.unwrap(), b"outside");
        assert_eq!(
            tokio::fs::read(old.join("fixture.txt")).await.unwrap(),
            b"old"
        );
        assert_eq!(store.artifacts.len(), 1);
        let mut entries = tokio::fs::read_dir(&code_dir).await.unwrap();
        assert!(entries.next_entry().await.unwrap().is_some());
        assert!(entries.next_entry().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_should_reject_duplicate_raw_zip_names_before_indexed_extraction() {
        use std::io::{Cursor, Write};

        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file("first", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"one").unwrap();
        writer
            .start_file("other", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"two").unwrap();
        let mut bytes = writer.finish().unwrap().into_inner();
        let local = bytes
            .windows(4)
            .rposition(|part| part == b"PK\x03\x04")
            .unwrap();
        let central = bytes
            .windows(4)
            .rposition(|part| part == b"PK\x01\x02")
            .unwrap();
        bytes
            .get_mut(local + 30..local + 35)
            .unwrap()
            .copy_from_slice(b"first");
        bytes
            .get_mut(central + 46..central + 51)
            .unwrap()
            .copy_from_slice(b"first");
        let archive = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
        assert_eq!(
            archive.len(),
            1,
            "upstream metadata deduplicates identical raw names"
        );
        assert!(archive.has_duplicate_names());

        let root = tempfile::tempdir().unwrap();
        let store = FunctionStore::new(root.path());
        let error = store
            .store_zip_code("duplicate", "$LATEST", &Bytes::from(bytes))
            .await
            .unwrap_err();
        assert!(matches!(error, LambdaServiceError::InvalidZipFile { .. }));
        assert!(store.artifacts.is_empty());
        let mut entries = tokio::fs::read_dir(root.path()).await.unwrap();
        assert!(entries.next_entry().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_should_preserve_dos_attribute_semantics_without_special_permissions() {
        let mut bytes = crate::test_zip::minimal_zip(b"safe");
        let central = bytes
            .windows(4)
            .position(|part| part == b"PK\x01\x02")
            .unwrap();
        bytes
            .get_mut(central + 4..central + 6)
            .unwrap()
            .copy_from_slice(&20u16.to_le_bytes());
        bytes
            .get_mut(central + 38..central + 42)
            .unwrap()
            .copy_from_slice(&(0o104_755u32 << 16).to_le_bytes());
        let root = tempfile::tempdir().unwrap();
        let store = FunctionStore::new(root.path());
        let (path, _, _) = store
            .store_zip_code("dos", "$LATEST", &Bytes::from(bytes))
            .await
            .unwrap();
        assert_eq!(
            tokio::fs::read(path.join("fixture.txt")).await.unwrap(),
            b"safe"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = tokio::fs::metadata(path.join("fixture.txt")).await.unwrap();
            assert_eq!(metadata.permissions().mode() & 0o7000, 0);
        }
    }

    #[test]
    fn test_should_reject_zip_that_expands_beyond_extraction_budget() {
        use std::io::Write as _;

        // A small compressed entry that declares a large uncompressed size
        // must be rejected when it exceeds the extraction budget.
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
            w.start_file("big.bin", opts).unwrap();
            w.write_all(&vec![0u8; 1024]).unwrap();
            w.finish().unwrap();
        }

        let tmp = tempfile::tempdir().unwrap();
        let err =
            extract_zip(&buf, tmp.path(), 100).expect_err("expansion beyond budget must fail");
        assert!(
            matches!(err, LambdaServiceError::InvalidZipFile { .. }),
            "got: {err:?}"
        );
    }

    #[test]
    fn test_should_extract_zip_within_budget() {
        use std::io::Write as _;

        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
            w.start_file("small.txt", opts).unwrap();
            w.write_all(b"hello").unwrap();
            w.finish().unwrap();
        }

        let tmp = tempfile::tempdir().unwrap();
        extract_zip(&buf, tmp.path(), 1024).expect("within-budget zip extracts");
        assert!(tmp.path().join("small.txt").exists());
    }

    // ---- Event source mapping store tests ----

    fn sample_esm_record(uuid: &str, function_arn: &str) -> EventSourceMappingRecord {
        EventSourceMappingRecord {
            uuid: uuid.to_owned(),
            event_source_arn: "arn:aws:sqs:us-east-1:000000000000:my-queue".to_owned(),
            function_arn: function_arn.to_owned(),
            enabled: true,
            batch_size: 10,
            maximum_batching_window_in_seconds: 0,
            starting_position: None,
            starting_position_timestamp: None,
            maximum_record_age_in_seconds: None,
            bisect_batch_on_function_error: None,
            maximum_retry_attempts: None,
            parallelization_factor: None,
            function_response_types: Vec::new(),
            state: "Enabled".to_owned(),
            state_transition_reason: "User action".to_owned(),
            last_modified: 1_700_000_000,
            last_processing_result: "No records processed".to_owned(),
        }
    }

    #[test]
    fn test_should_create_and_get_esm() {
        let store = EventSourceMappingStore::new();
        let record = sample_esm_record(
            "uuid-1",
            "arn:aws:lambda:us-east-1:000000000000:function:my-func",
        );
        store.create(record);

        let retrieved = store.get("uuid-1");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.as_ref().map(|r| r.uuid.as_str()), Some("uuid-1"));
        assert_eq!(retrieved.as_ref().map(|r| r.batch_size), Some(10));
    }

    #[test]
    fn test_should_return_none_for_missing_esm() {
        let store = EventSourceMappingStore::new();
        assert!(store.get("no-such-uuid").is_none());
    }

    #[test]
    fn test_should_update_esm() {
        let store = EventSourceMappingStore::new();
        store.create(sample_esm_record("uuid-1", "arn:func"));

        store
            .update("uuid-1", |record| {
                record.batch_size = 50;
                record.enabled = false;
                "Disabled".clone_into(&mut record.state);
            })
            .unwrap();

        let retrieved = store.get("uuid-1").unwrap();
        assert_eq!(retrieved.batch_size, 50);
        assert!(!retrieved.enabled);
        assert_eq!(retrieved.state, "Disabled");
    }

    #[test]
    fn test_should_error_on_update_nonexistent_esm() {
        let store = EventSourceMappingStore::new();
        let err = store.update("no-such", |_| {}).unwrap_err();
        assert!(matches!(
            err,
            LambdaServiceError::EventSourceMappingNotFound { .. }
        ));
    }

    #[test]
    fn test_should_delete_esm() {
        let store = EventSourceMappingStore::new();
        store.create(sample_esm_record("uuid-1", "arn:func"));

        let removed = store.delete("uuid-1");
        assert!(removed.is_some());
        assert!(store.get("uuid-1").is_none());
    }

    #[test]
    fn test_should_return_none_on_delete_nonexistent_esm() {
        let store = EventSourceMappingStore::new();
        assert!(store.delete("no-such").is_none());
    }

    #[test]
    fn test_should_list_esm_sorted_by_uuid() {
        let store = EventSourceMappingStore::new();
        store.create(sample_esm_record("charlie", "arn:func-a"));
        store.create(sample_esm_record("alpha", "arn:func-b"));
        store.create(sample_esm_record("bravo", "arn:func-a"));

        let all = store.list(None, None);
        let uuids: Vec<&str> = all.iter().map(|r| r.uuid.as_str()).collect();
        assert_eq!(uuids, ["alpha", "bravo", "charlie"]);
    }

    #[test]
    fn test_should_filter_esm_by_function_name() {
        let store = EventSourceMappingStore::new();
        store.create(sample_esm_record(
            "uuid-1",
            "arn:aws:lambda:us-east-1:000000000000:function:func-a",
        ));
        store.create(sample_esm_record(
            "uuid-2",
            "arn:aws:lambda:us-east-1:000000000000:function:func-b",
        ));

        let filtered = store.list(Some("func-a"), None);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].uuid, "uuid-1");
    }

    #[test]
    fn test_should_filter_esm_by_event_source_arn() {
        let store = EventSourceMappingStore::new();
        let mut record1 = sample_esm_record("uuid-1", "arn:func");
        record1.event_source_arn = "arn:aws:sqs:us-east-1:000000000000:queue-a".to_owned();
        let mut record2 = sample_esm_record("uuid-2", "arn:func");
        record2.event_source_arn = "arn:aws:sqs:us-east-1:000000000000:queue-b".to_owned();
        store.create(record1);
        store.create(record2);

        let filtered = store.list(None, Some("arn:aws:sqs:us-east-1:000000000000:queue-a"));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].uuid, "uuid-1");
    }
}
