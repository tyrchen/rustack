//! In-memory function storage using `DashMap` for concurrent access.
//!
//! Stores function records, version snapshots, aliases, policies, tags,
//! and function URL configurations. Code is stored as raw bytes with
//! SHA-256 hashes computed on ingestion.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use bytes::Bytes;
use dashmap::DashMap;
use sha2::{Digest, Sha256};

use ruststack_lambda_model::types::{
    Cors, DeadLetterConfig, ImageConfig, LoggingConfig, SnapStart, TracingConfig, VpcConfig,
};

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
    /// ISO 8601 creation timestamp.
    pub created_at: String,
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
}
