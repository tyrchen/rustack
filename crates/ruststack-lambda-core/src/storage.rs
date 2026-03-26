//! In-memory function storage using `DashMap` for concurrent access.
//!
//! Stores function records, version snapshots, aliases, policies, tags,
//! and function URL configurations. Code is stored as raw bytes with
//! SHA-256 hashes computed on ingestion.

use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
};

use bytes::Bytes;
use dashmap::DashMap;
use ruststack_lambda_model::types::{
    Cors, DeadLetterConfig, DestinationConfig, ImageConfig, LoggingConfig, SnapStart,
    TracingConfig, VpcConfig,
};
use sha2::{Digest, Sha256};

use crate::error::LambdaServiceError;

/// In-memory store for Lambda functions.
#[derive(Debug)]
pub struct FunctionStore {
    /// All functions keyed by function name.
    functions: DashMap<String, FunctionRecord>,
    /// Root directory for storing extracted code.
    code_dir: PathBuf,
}

/// Complete record for a Lambda function.
#[derive(Debug, Clone)]
pub struct FunctionRecord {
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
#[derive(Debug, Clone)]
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
    /// Raw zip bytes.
    pub zip_bytes: Option<Bytes>,
    /// Function state (`Active`, `Pending`, etc.).
    pub state: String,
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
    /// SnapStart configuration.
    pub snap_start: Option<SnapStart>,
}

/// Alias mapping to a function version.
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone, Default)]
pub struct PolicyDocument {
    /// Policy statements.
    pub statements: Vec<PolicyStatement>,
}

/// A single policy statement.
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
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

/// Complete record for a Lambda event source mapping.
#[derive(Debug, Clone)]
pub struct EventSourceMappingRecord {
    /// Unique identifier for the mapping.
    pub uuid: String,
    /// ARN of the event source (e.g., SQS queue, DynamoDB stream).
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
}

impl Default for EventSourceMappingStore {
    fn default() -> Self {
        Self::new()
    }
}

impl FunctionStore {
    /// Create a new function store with the given code storage directory.
    pub fn new(code_dir: impl Into<PathBuf>) -> Self {
        Self {
            functions: DashMap::new(),
            code_dir: code_dir.into(),
        }
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
        match self.functions.entry(record.name.clone()) {
            Entry::Occupied(_) => Err(LambdaServiceError::ResourceConflict {
                message: format!("Function already exist: {}", record.name),
            }),
            Entry::Vacant(entry) => {
                entry.insert(record);
                Ok(())
            }
        }
    }

    /// Get a clone of a function record by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<FunctionRecord> {
        self.functions.get(name).map(|r| r.value().clone())
    }

    /// Check whether a function exists.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.functions.contains_key(name)
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
        match self.functions.get_mut(name) {
            Some(mut entry) => Ok(f(entry.value_mut())),
            None => Err(LambdaServiceError::FunctionNotFound {
                name: name.to_owned(),
            }),
        }
    }

    /// Remove a function from the store.
    ///
    /// Returns the removed record, or `None` if it did not exist.
    #[must_use]
    pub fn remove(&self, name: &str) -> Option<FunctionRecord> {
        self.functions.remove(name).map(|(_, v)| v)
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

    /// Store zip code bytes for a function version.
    ///
    /// Writes the raw zip bytes to disk at
    /// `{code_dir}/{function_name}/{version}/code.zip` and returns
    /// the storage path, base64-encoded SHA-256, and code size in bytes.
    ///
    /// # Errors
    ///
    /// Returns `Internal` if directory creation or file writing fails.
    pub async fn store_zip_code(
        &self,
        function_name: &str,
        version: &str,
        zip_bytes: &[u8],
    ) -> Result<(PathBuf, String, u64), LambdaServiceError> {
        let dir = self.code_dir.join(function_name).join(version);
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| LambdaServiceError::Internal {
                message: format!("Failed to create code directory: {e}"),
            })?;

        let zip_path = dir.join("code.zip");
        tokio::fs::write(&zip_path, zip_bytes)
            .await
            .map_err(|e| LambdaServiceError::Internal {
                message: format!("Failed to write code zip: {e}"),
            })?;

        let sha256 = compute_sha256(zip_bytes);
        let code_size = zip_bytes.len() as u64;

        Ok((dir, sha256, code_size))
    }

    /// Clean up code directory for a function.
    ///
    /// Removes the `{code_dir}/{function_name}` directory tree.
    pub async fn cleanup_code(&self, function_name: &str) {
        let dir = self.code_dir.join(function_name);
        if dir.exists() {
            let _ = tokio::fs::remove_dir_all(&dir).await;
        }
    }
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

    fn test_store() -> FunctionStore {
        FunctionStore::new("/tmp/ruststack-lambda-test")
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
            zip_bytes: None,
            state: "Active".to_owned(),
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
            name: name.to_owned(),
            arn: format!("arn:aws:lambda:us-east-1:000000000000:function:{name}"),
            latest: sample_version(),
            versions: BTreeMap::new(),
            next_version: 2,
            aliases: HashMap::new(),
            policy: PolicyDocument::default(),
            tags: HashMap::new(),
            url_config: None,
            reserved_concurrent_executions: None,
            event_invoke_configs: HashMap::new(),
            created_at: "2024-01-01T00:00:00.000+0000".to_owned(),
        }
    }

    #[test]
    fn test_should_insert_and_get_function() {
        let store = test_store();
        let record = sample_record("my-func");
        store.insert(record).unwrap();

        let retrieved = store.get("my-func").unwrap();
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

        let retrieved = store.get("my-func").unwrap();
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
        assert!(store.contains("my-func"));

        let removed = store.remove("my-func");
        assert!(removed.is_some());
        assert!(!store.contains("my-func"));
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
        assert!(!hash.is_empty());
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

        let zip_data = b"PK\x03\x04fake-zip-data";
        let (dir, sha256, size) = store
            .store_zip_code("test-func", "$LATEST", zip_data)
            .await
            .unwrap();

        assert!(dir.exists());
        assert!(dir.join("code.zip").exists());
        assert!(!sha256.is_empty());
        assert_eq!(size, zip_data.len() as u64);

        store.cleanup_code("test-func").await;
        assert!(!dir.exists());
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
