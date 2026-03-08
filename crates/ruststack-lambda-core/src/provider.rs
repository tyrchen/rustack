//! Lambda business logic provider.
//!
//! Implements all Lambda CRUD operations, building responses from internal
//! storage types. Phase 0 operations (function CRUD + invoke) are fully
//! implemented; later phases (versions, aliases, permissions, tags, URLs)
//! return appropriate errors until implemented.

use std::collections::HashMap;

use bytes::Bytes;
use tracing::info;

/// Maximum deployment package size (50 MB zipped, per Appendix C).
const MAX_ZIP_SIZE: u64 = 50 * 1024 * 1024;

/// Maximum synchronous invoke payload size (6 MB, per Appendix C).
const MAX_SYNC_PAYLOAD: usize = 6 * 1024 * 1024;

use ruststack_lambda_model::input::{
    AddPermissionInput, CreateAliasInput, CreateFunctionInput, CreateFunctionUrlConfigInput,
    PublishVersionInput, TagResourceInput, UpdateAliasInput, UpdateFunctionCodeInput,
    UpdateFunctionConfigurationInput, UpdateFunctionUrlConfigInput,
};
use ruststack_lambda_model::output::{
    AccountLimit, AccountUsage, AddPermissionOutput, GetAccountSettingsOutput, GetFunctionOutput,
    GetPolicyOutput, ListAliasesOutput, ListFunctionUrlConfigsOutput, ListFunctionsOutput,
    ListTagsOutput, ListVersionsOutput,
};
use ruststack_lambda_model::types::{
    AliasConfiguration, AliasRoutingConfiguration, EnvironmentResponse, EphemeralStorage,
    FunctionCodeLocation, FunctionConfiguration, FunctionUrlConfig, ImageConfigResponse, Layer,
    SnapStartResponse, TracingConfigResponse, VpcConfigResponse,
};

use crate::config::LambdaConfig;
use crate::error::LambdaServiceError;
use crate::resolver::{
    alias_arn, function_arn, function_version_arn, resolve_function_ref, resolve_version,
};
use crate::storage::{
    AliasRecord, FunctionRecord, FunctionStore, FunctionUrlConfigRecord, PolicyDocument,
    PolicyStatement, VersionRecord, compute_sha256,
};

/// Lambda business logic provider.
///
/// Holds the function store and service configuration. All operations
/// are implemented as async methods that return domain types or errors.
#[derive(Debug)]
pub struct RustStackLambda {
    store: FunctionStore,
    config: LambdaConfig,
}

impl RustStackLambda {
    /// Create a new Lambda provider with the given store and config.
    #[must_use]
    pub fn with_store(store: FunctionStore, config: LambdaConfig) -> Self {
        Self { store, config }
    }

    /// Create a new Lambda provider from config, using a temp directory for code.
    #[must_use]
    pub fn new(config: LambdaConfig) -> Self {
        let code_dir = std::env::temp_dir().join("ruststack-lambda-code");
        let store = FunctionStore::new(code_dir);
        Self { store, config }
    }

    /// Returns a reference to the underlying function store.
    #[must_use]
    pub fn store(&self) -> &FunctionStore {
        &self.store
    }

    /// Returns a reference to the service configuration.
    #[must_use]
    pub fn config(&self) -> &LambdaConfig {
        &self.config
    }

    // ---------------------------------------------------------------
    // Phase 0: Function CRUD
    // ---------------------------------------------------------------

    /// Create a new Lambda function.
    ///
    /// Validates the input, stores the deployment package, and inserts
    /// the function record into the store.
    #[allow(clippy::too_many_lines)]
    pub async fn create_function(
        &self,
        input: CreateFunctionInput,
    ) -> Result<FunctionConfiguration, LambdaServiceError> {
        let name = &input.function_name;
        if name.is_empty() || name.len() > 140 {
            return Err(LambdaServiceError::InvalidParameter {
                message: "Function name must be between 1 and 140 characters".to_owned(),
            });
        }

        // Validate handler length (Appendix C: max 128 chars).
        if let Some(ref handler) = input.handler {
            if handler.len() > 128 {
                return Err(LambdaServiceError::InvalidParameter {
                    message: "Handler must be at most 128 characters".to_owned(),
                });
            }
        }

        // Validate description length (Appendix C: max 256 chars).
        if let Some(ref desc) = input.description {
            if desc.len() > 256 {
                return Err(LambdaServiceError::InvalidParameter {
                    message: "Description must be at most 256 characters".to_owned(),
                });
            }
        }

        // Validate timeout (Appendix C: 1-900 seconds).
        if let Some(timeout) = input.timeout {
            if !(1..=900).contains(&timeout) {
                return Err(LambdaServiceError::InvalidParameter {
                    message: "Timeout must be between 1 and 900 seconds".to_owned(),
                });
            }
        }

        // Validate memory size (Appendix C: 128-10240 MB).
        if let Some(memory) = input.memory_size {
            if !(128..=10_240).contains(&memory) {
                return Err(LambdaServiceError::InvalidParameter {
                    message: "Memory size must be between 128 and 10240 MB".to_owned(),
                });
            }
        }

        // Validate ephemeral storage (Appendix C: 512-10240 MB).
        if let Some(ref ephemeral) = input.ephemeral_storage {
            if !(512..=10_240).contains(&ephemeral.size) {
                return Err(LambdaServiceError::InvalidParameter {
                    message: "Ephemeral storage must be between 512 and 10240 MB".to_owned(),
                });
            }
        }

        // Validate environment variables total size (Appendix C: 4 KB).
        if let Some(ref env) = input.environment {
            if let Some(ref vars) = env.variables {
                let total_size: usize = vars.iter().map(|(k, v)| k.len() + v.len()).sum();
                if total_size > 4096 {
                    return Err(LambdaServiceError::InvalidParameter {
                        message: "Environment variables total size exceeds 4 KB limit".to_owned(),
                    });
                }
            }
        }

        // Validate tags count (Appendix C: max 50).
        if let Some(ref tags) = input.tags {
            if tags.len() > 50 {
                return Err(LambdaServiceError::InvalidParameter {
                    message: "Tags count exceeds the limit of 50".to_owned(),
                });
            }
        }

        if self.store.contains(name) {
            return Err(LambdaServiceError::ResourceConflict {
                message: format!("Function already exist: {name}"),
            });
        }

        let now = now_iso8601();
        let revision_id = uuid::Uuid::new_v4().to_string();
        let arn = function_arn(&self.config.default_region, &self.config.account_id, name);

        // Validate code is provided.
        let package_type = input
            .package_type
            .clone()
            .unwrap_or_else(|| "Zip".to_owned());

        if package_type == "Zip" && input.code.zip_file.is_none() && input.code.s3_bucket.is_none()
        {
            return Err(LambdaServiceError::InvalidParameter {
                message: "Code is required for Zip package type. Provide ZipFile or S3Bucket."
                    .to_owned(),
            });
        }
        if package_type == "Image" && input.code.image_uri.is_none() {
            return Err(LambdaServiceError::InvalidParameter {
                message: "ImageUri is required for Image package type.".to_owned(),
            });
        }

        // Validate runtime/handler for Zip packages.
        if package_type == "Zip" {
            if input.runtime.is_none() {
                return Err(LambdaServiceError::InvalidParameter {
                    message: "Runtime is required for Zip package type.".to_owned(),
                });
            }
            if input.handler.is_none() {
                return Err(LambdaServiceError::InvalidParameter {
                    message: "Handler is required for Zip package type.".to_owned(),
                });
            }
        }

        // Process code.
        let (code_sha256, code_size, zip_bytes, code_path, image_uri) = self
            .process_code(
                name,
                "$LATEST",
                input.code.zip_file.as_deref(),
                input.code.image_uri.as_deref(),
            )
            .await?;

        // Validate deployment package size (Appendix C: 50 MB zipped).
        if code_size > MAX_ZIP_SIZE {
            return Err(LambdaServiceError::InvalidParameter {
                message: format!("Unzipped size must be smaller than {MAX_ZIP_SIZE} bytes"),
            });
        }

        let timeout = input.timeout.unwrap_or(3);
        let memory_size = input.memory_size.unwrap_or(128);
        let architectures = input
            .architectures
            .clone()
            .unwrap_or_else(|| vec!["x86_64".to_owned()]);
        let ephemeral_storage_size = input.ephemeral_storage.as_ref().map_or(512, |e| e.size);
        let env_vars = input
            .environment
            .as_ref()
            .and_then(|e| e.variables.clone())
            .unwrap_or_default();

        let version_record = VersionRecord {
            version: "$LATEST".to_owned(),
            runtime: input.runtime.clone(),
            handler: input.handler.clone(),
            role: input.role.clone(),
            description: input.description.clone().unwrap_or_default(),
            timeout,
            memory_size,
            environment: env_vars,
            package_type: package_type.clone(),
            code_path,
            image_uri,
            zip_bytes,
            state: "Active".to_owned(),
            last_modified: now.clone(),
            architectures,
            ephemeral_storage_size,
            code_sha256,
            code_size,
            revision_id,
            layers: input.layers.clone().unwrap_or_default(),
            vpc_config: input.vpc_config.clone(),
            dead_letter_config: input.dead_letter_config.clone(),
            tracing_config: input.tracing_config.clone(),
            image_config: input.image_config.clone(),
            logging_config: input.logging_config.clone(),
            snap_start: input.snap_start.clone(),
        };

        let record = FunctionRecord {
            name: name.clone(),
            arn: arn.clone(),
            latest: version_record,
            versions: std::collections::BTreeMap::new(),
            next_version: 1,
            aliases: HashMap::new(),
            policy: PolicyDocument::default(),
            tags: input.tags.clone().unwrap_or_default(),
            url_config: None,
            created_at: now,
        };

        let should_publish = input.publish.unwrap_or(false);
        self.store.insert(record)?;

        // If publish=true, publish version 1 immediately.
        let config = if should_publish {
            let publish_input = crate::provider::PublishVersionInput::default();
            self.publish_version(name, &publish_input)?
        } else {
            let record = self.get_record(name)?;
            self.build_function_configuration(&record, &record.latest)
        };

        info!(function_name = %name, "created Lambda function");
        Ok(config)
    }

    /// Get function information including configuration, code location, and tags.
    pub fn get_function(
        &self,
        function_ref: &str,
        qualifier: Option<&str>,
    ) -> Result<GetFunctionOutput, LambdaServiceError> {
        let (name, ref_qualifier) = resolve_function_ref(function_ref)?;
        let qualifier = qualifier.or(ref_qualifier.as_deref());

        let record = self.get_record(&name)?;
        let version = resolve_version(&record, qualifier)?;
        let config = self.build_function_configuration(&record, version);

        let code_location = FunctionCodeLocation {
            repository_type: Some("S3".to_owned()),
            location: Some(format!(
                "https://awslambda-{region}-tasks.s3.{region}.amazonaws.com/snapshots/{account}/{name}",
                region = self.config.default_region,
                account = self.config.account_id,
            )),
            image_uri: version.image_uri.clone(),
            resolved_image_uri: None,
        };

        let tags = if record.tags.is_empty() {
            None
        } else {
            Some(record.tags.clone())
        };

        Ok(GetFunctionOutput {
            configuration: Some(config),
            code: Some(code_location),
            tags,
        })
    }

    /// Get function configuration only.
    pub fn get_function_configuration(
        &self,
        function_ref: &str,
        qualifier: Option<&str>,
    ) -> Result<FunctionConfiguration, LambdaServiceError> {
        let (name, ref_qualifier) = resolve_function_ref(function_ref)?;
        let qualifier = qualifier.or(ref_qualifier.as_deref());

        let record = self.get_record(&name)?;
        let version = resolve_version(&record, qualifier)?;
        Ok(self.build_function_configuration(&record, version))
    }

    /// Update function code.
    pub async fn update_function_code(
        &self,
        function_ref: &str,
        input: UpdateFunctionCodeInput,
    ) -> Result<FunctionConfiguration, LambdaServiceError> {
        // Validate that some code source is provided.
        if input.zip_file.is_none() && input.image_uri.is_none() && input.s3_bucket.is_none() {
            return Err(LambdaServiceError::InvalidParameter {
                message: "Provide at least one of ZipFile, S3Bucket, or ImageUri.".to_owned(),
            });
        }

        let (name, _) = resolve_function_ref(function_ref)?;
        let should_publish = input.publish.unwrap_or(false);

        let (code_sha256, code_size, zip_bytes, code_path, image_uri) = self
            .process_code(
                &name,
                "$LATEST",
                input.zip_file.as_deref(),
                input.image_uri.as_deref(),
            )
            .await?;

        self.store.update(&name, |record| {
            let now = now_iso8601();
            record.latest.code_sha256 = code_sha256;
            record.latest.code_size = code_size;
            record.latest.zip_bytes = zip_bytes;
            record.latest.code_path = code_path;
            record.latest.image_uri = image_uri;
            record.latest.last_modified = now;
            record.latest.revision_id = uuid::Uuid::new_v4().to_string();

            if let Some(archs) = input.architectures.clone() {
                record.latest.architectures = archs;
            }
        })?;

        // If publish=true, publish a new version.
        let config = if should_publish {
            let publish_input = PublishVersionInput::default();
            self.publish_version(&name, &publish_input)?
        } else {
            let record = self.get_record(&name)?;
            self.build_function_configuration(&record, &record.latest)
        };

        info!(function_name = %name, "updated Lambda function code");
        Ok(config)
    }

    /// Update function configuration (handler, runtime, env vars, etc.).
    pub fn update_function_configuration(
        &self,
        function_ref: &str,
        input: &UpdateFunctionConfigurationInput,
    ) -> Result<FunctionConfiguration, LambdaServiceError> {
        let (name, _) = resolve_function_ref(function_ref)?;

        // Validate handler length (Appendix C: max 128 chars).
        if let Some(ref handler) = input.handler {
            if handler.len() > 128 {
                return Err(LambdaServiceError::InvalidParameter {
                    message: "Handler must be at most 128 characters".to_owned(),
                });
            }
        }
        // Validate description length (Appendix C: max 256 chars).
        if let Some(ref desc) = input.description {
            if desc.len() > 256 {
                return Err(LambdaServiceError::InvalidParameter {
                    message: "Description must be at most 256 characters".to_owned(),
                });
            }
        }
        // Validate timeout (Appendix C: 1-900 seconds).
        if let Some(timeout) = input.timeout {
            if !(1..=900).contains(&timeout) {
                return Err(LambdaServiceError::InvalidParameter {
                    message: "Timeout must be between 1 and 900 seconds".to_owned(),
                });
            }
        }
        // Validate memory size (Appendix C: 128-10240 MB).
        if let Some(memory) = input.memory_size {
            if !(128..=10_240).contains(&memory) {
                return Err(LambdaServiceError::InvalidParameter {
                    message: "Memory size must be between 128 and 10240 MB".to_owned(),
                });
            }
        }
        // Validate ephemeral storage (Appendix C: 512-10240 MB).
        if let Some(ref ephemeral) = input.ephemeral_storage {
            if !(512..=10_240).contains(&ephemeral.size) {
                return Err(LambdaServiceError::InvalidParameter {
                    message: "Ephemeral storage must be between 512 and 10240 MB".to_owned(),
                });
            }
        }
        // Validate environment variables total size (Appendix C: 4 KB).
        if let Some(ref env) = input.environment {
            if let Some(ref vars) = env.variables {
                let total_size: usize = vars.iter().map(|(k, v)| k.len() + v.len()).sum();
                if total_size > 4096 {
                    return Err(LambdaServiceError::InvalidParameter {
                        message: "Environment variables total size exceeds 4 KB limit".to_owned(),
                    });
                }
            }
        }

        let config = self.store.update(&name, |record| {
            let now = now_iso8601();

            if let Some(runtime) = &input.runtime {
                record.latest.runtime = Some(runtime.clone());
            }
            if let Some(role) = &input.role {
                record.latest.role.clone_from(role);
            }
            if let Some(handler) = &input.handler {
                record.latest.handler = Some(handler.clone());
            }
            if let Some(description) = &input.description {
                record.latest.description.clone_from(description);
            }
            if let Some(timeout) = input.timeout {
                record.latest.timeout = timeout;
            }
            if let Some(memory_size) = input.memory_size {
                record.latest.memory_size = memory_size;
            }
            if let Some(env) = &input.environment {
                record.latest.environment = env.variables.clone().unwrap_or_default();
            }
            if let Some(layers) = &input.layers {
                record.latest.layers.clone_from(layers);
            }
            if let Some(ephemeral) = &input.ephemeral_storage {
                record.latest.ephemeral_storage_size = ephemeral.size;
            }
            if let Some(vpc) = &input.vpc_config {
                record.latest.vpc_config = Some(vpc.clone());
            }
            if let Some(dlc) = &input.dead_letter_config {
                record.latest.dead_letter_config = Some(dlc.clone());
            }
            if let Some(tc) = &input.tracing_config {
                record.latest.tracing_config = Some(tc.clone());
            }
            if let Some(ic) = &input.image_config {
                record.latest.image_config = Some(ic.clone());
            }
            if let Some(lc) = &input.logging_config {
                record.latest.logging_config = Some(lc.clone());
            }
            if let Some(ss) = &input.snap_start {
                record.latest.snap_start = Some(ss.clone());
            }

            record.latest.last_modified = now;
            record.latest.revision_id = uuid::Uuid::new_v4().to_string();

            self.build_function_configuration(record, &record.latest)
        })?;

        info!(function_name = %name, "updated Lambda function configuration");
        Ok(config)
    }

    /// Delete a function.
    pub async fn delete_function(
        &self,
        function_ref: &str,
        qualifier: Option<&str>,
    ) -> Result<(), LambdaServiceError> {
        let (name, ref_qualifier) = resolve_function_ref(function_ref)?;
        let qualifier = qualifier.or(ref_qualifier.as_deref());

        // If qualifier is specified, delete that specific version/alias.
        if let Some(q) = qualifier {
            if q != "$LATEST" {
                // Try to delete a published version.
                if let Ok(version_num) = q.parse::<u64>() {
                    self.store.update(&name, |record| {
                        record.versions.remove(&version_num);
                    })?;
                    return Ok(());
                }
                // Otherwise it might be an alias -- but DeleteFunction with
                // alias qualifier is not a standard API operation; ignore.
            }
        }

        // Delete the entire function.
        if self.store.remove(&name).is_none() {
            return Err(LambdaServiceError::FunctionNotFound { name: name.clone() });
        }

        // Clean up code directory.
        self.store.cleanup_code(&name).await;

        info!(function_name = %name, "deleted Lambda function");
        Ok(())
    }

    /// List all functions with optional pagination.
    #[must_use]
    pub fn list_functions(
        &self,
        marker: Option<&str>,
        max_items: Option<usize>,
    ) -> ListFunctionsOutput {
        let all = self.store.list();
        let max = max_items.unwrap_or(50).min(10_000);

        // Find start position based on marker (function name).
        let start = marker
            .and_then(|m| all.iter().position(|r| r.name.as_str() > m))
            .unwrap_or(0);

        let page: Vec<FunctionConfiguration> = all
            .iter()
            .skip(start)
            .take(max)
            .map(|r| self.build_function_configuration(r, &r.latest))
            .collect();

        let next_marker = if start + max < all.len() {
            page.last().and_then(|c| c.function_name.clone())
        } else {
            None
        };

        ListFunctionsOutput {
            functions: Some(page),
            next_marker,
        }
    }

    // ---------------------------------------------------------------
    // Phase 0: Invoke
    // ---------------------------------------------------------------

    /// Invoke a function.
    ///
    /// Currently returns a Docker-not-available error when Docker is
    /// disabled, or a DryRun 204 for `DryRun` invocation type.
    pub fn invoke(
        &self,
        function_ref: &str,
        qualifier: Option<&str>,
        payload: &[u8],
        is_dry_run: bool,
    ) -> Result<(u16, Bytes), LambdaServiceError> {
        // Validate synchronous payload size (Appendix C: 6 MB).
        if payload.len() > MAX_SYNC_PAYLOAD {
            let payload_len = payload.len();
            return Err(LambdaServiceError::RequestTooLarge {
                message: format!(
                    "Request payload size {payload_len} exceeds the synchronous invoke limit of {MAX_SYNC_PAYLOAD} bytes",
                ),
            });
        }

        let (name, ref_qualifier) = resolve_function_ref(function_ref)?;
        let qualifier = qualifier.or(ref_qualifier.as_deref());

        // Validate function exists and qualifier resolves.
        let record = self.get_record(&name)?;
        let _version = resolve_version(&record, qualifier)?;

        if is_dry_run {
            return Ok((204, Bytes::new()));
        }

        if !self.config.docker_enabled {
            return Err(LambdaServiceError::DockerNotAvailable);
        }

        // Docker execution will be implemented in a future phase.
        // For now, return a stub response with the payload echoed back.
        let response = serde_json::json!({
            "statusCode": 200,
            "body": String::from_utf8_lossy(payload),
        });
        let body = serde_json::to_vec(&response).map_err(|e| LambdaServiceError::Internal {
            message: format!("Failed to serialize invoke response: {e}"),
        })?;

        Ok((200, Bytes::from(body)))
    }

    // ---------------------------------------------------------------
    // Phase 1: Versions + Aliases
    // ---------------------------------------------------------------

    /// Publish a version from `$LATEST`.
    pub fn publish_version(
        &self,
        function_ref: &str,
        input: &PublishVersionInput,
    ) -> Result<FunctionConfiguration, LambdaServiceError> {
        let (name, _) = resolve_function_ref(function_ref)?;

        let config = self.store.update(&name, |record| {
            let version_num = record.next_version;
            record.next_version += 1;

            let mut published = record.latest.clone();
            published.version = version_num.to_string();
            if let Some(desc) = &input.description {
                published.description.clone_from(desc);
            }
            published.revision_id = uuid::Uuid::new_v4().to_string();
            published.last_modified = now_iso8601();

            let config = self.build_function_configuration(record, &published);
            record.versions.insert(version_num, published);
            config
        })?;

        info!(function_name = %name, "published Lambda function version");
        Ok(config)
    }

    /// List versions of a function.
    pub fn list_versions_by_function(
        &self,
        function_ref: &str,
        marker: Option<&str>,
        max_items: Option<usize>,
    ) -> Result<ListVersionsOutput, LambdaServiceError> {
        let (name, _) = resolve_function_ref(function_ref)?;
        let record = self.get_record(&name)?;
        let max = max_items.unwrap_or(50).min(10_000);

        let mut versions = Vec::with_capacity(record.versions.len() + 1);
        versions.push(self.build_function_configuration(&record, &record.latest));
        for ver in record.versions.values() {
            versions.push(self.build_function_configuration(&record, ver));
        }

        // Simple marker-based pagination.
        let start = marker
            .and_then(|m| {
                versions
                    .iter()
                    .position(|v| v.version.as_deref() == Some(m))
            })
            .map_or(0, |pos| pos + 1);

        let total = versions.len();
        let page: Vec<FunctionConfiguration> = versions.into_iter().skip(start).take(max).collect();

        let next_marker = if start + page.len() < total {
            page.last().and_then(|v| v.version.clone())
        } else {
            None
        };

        Ok(ListVersionsOutput {
            versions: Some(page),
            next_marker,
        })
    }

    /// Create an alias.
    pub fn create_alias(
        &self,
        function_ref: &str,
        input: CreateAliasInput,
    ) -> Result<AliasConfiguration, LambdaServiceError> {
        let (name, _) = resolve_function_ref(function_ref)?;

        let config = self.store.update(
            &name,
            |record| -> Result<AliasConfiguration, LambdaServiceError> {
                if record.aliases.contains_key(&input.name) {
                    return Err(LambdaServiceError::ResourceConflict {
                        message: format!("Alias already exists: {}", input.name),
                    });
                }

                // Validate target version exists.
                let target = &input.function_version;
                if target != "$LATEST" {
                    let version_num: u64 =
                        target
                            .parse()
                            .map_err(|_| LambdaServiceError::InvalidParameter {
                                message: format!("Invalid version: {target}"),
                            })?;
                    if !record.versions.contains_key(&version_num) {
                        return Err(LambdaServiceError::VersionNotFound {
                            function_name: name.clone(),
                            version: target.clone(),
                        });
                    }
                }

                let revision_id = uuid::Uuid::new_v4().to_string();
                let alias_record = AliasRecord {
                    name: input.name.clone(),
                    function_version: input.function_version.clone(),
                    description: input.description.clone().unwrap_or_default(),
                    routing_config: input
                        .routing_config
                        .as_ref()
                        .and_then(|r| r.additional_version_weights.clone()),
                    revision_id: revision_id.clone(),
                };

                let arn = alias_arn(
                    &self.config.default_region,
                    &self.config.account_id,
                    &name,
                    &input.name,
                );

                record.aliases.insert(input.name.clone(), alias_record);

                Ok(AliasConfiguration {
                    alias_arn: Some(arn),
                    name: Some(input.name),
                    function_version: Some(input.function_version),
                    description: input.description,
                    routing_config: input.routing_config,
                    revision_id: Some(revision_id),
                })
            },
        )??;

        Ok(config)
    }

    /// Get an alias.
    pub fn get_alias(
        &self,
        function_ref: &str,
        alias_name: &str,
    ) -> Result<AliasConfiguration, LambdaServiceError> {
        let (name, _) = resolve_function_ref(function_ref)?;
        let record = self.get_record(&name)?;

        let alias = record
            .aliases
            .get(alias_name)
            .ok_or(LambdaServiceError::AliasNotFound {
                function_name: name.clone(),
                alias: alias_name.to_owned(),
            })?;

        let arn = alias_arn(
            &self.config.default_region,
            &self.config.account_id,
            &name,
            alias_name,
        );

        Ok(AliasConfiguration {
            alias_arn: Some(arn),
            name: Some(alias.name.clone()),
            function_version: Some(alias.function_version.clone()),
            description: if alias.description.is_empty() {
                None
            } else {
                Some(alias.description.clone())
            },
            routing_config: alias
                .routing_config
                .as_ref()
                .map(|w| AliasRoutingConfiguration {
                    additional_version_weights: Some(w.clone()),
                }),
            revision_id: Some(alias.revision_id.clone()),
        })
    }

    /// Update an alias.
    pub fn update_alias(
        &self,
        function_ref: &str,
        alias_name: &str,
        input: &UpdateAliasInput,
    ) -> Result<AliasConfiguration, LambdaServiceError> {
        let (name, _) = resolve_function_ref(function_ref)?;

        let config = self.store.update(
            &name,
            |record| -> Result<AliasConfiguration, LambdaServiceError> {
                let alias = record.aliases.get_mut(alias_name).ok_or(
                    LambdaServiceError::AliasNotFound {
                        function_name: name.clone(),
                        alias: alias_name.to_owned(),
                    },
                )?;

                if let Some(fv) = &input.function_version {
                    // Validate target version exists.
                    if fv != "$LATEST" {
                        let version_num: u64 =
                            fv.parse()
                                .map_err(|_| LambdaServiceError::InvalidParameter {
                                    message: format!("Invalid version: {fv}"),
                                })?;
                        if !record.versions.contains_key(&version_num) {
                            return Err(LambdaServiceError::VersionNotFound {
                                function_name: name.clone(),
                                version: fv.clone(),
                            });
                        }
                    }
                    alias.function_version.clone_from(fv);
                }
                if let Some(desc) = &input.description {
                    alias.description.clone_from(desc);
                }
                if let Some(rc) = &input.routing_config {
                    alias
                        .routing_config
                        .clone_from(&rc.additional_version_weights);
                }
                alias.revision_id = uuid::Uuid::new_v4().to_string();

                let arn = alias_arn(
                    &self.config.default_region,
                    &self.config.account_id,
                    &name,
                    alias_name,
                );

                Ok(AliasConfiguration {
                    alias_arn: Some(arn),
                    name: Some(alias.name.clone()),
                    function_version: Some(alias.function_version.clone()),
                    description: if alias.description.is_empty() {
                        None
                    } else {
                        Some(alias.description.clone())
                    },
                    routing_config: alias.routing_config.as_ref().map(|w| {
                        AliasRoutingConfiguration {
                            additional_version_weights: Some(w.clone()),
                        }
                    }),
                    revision_id: Some(alias.revision_id.clone()),
                })
            },
        )??;

        Ok(config)
    }

    /// Delete an alias.
    pub fn delete_alias(
        &self,
        function_ref: &str,
        alias_name: &str,
    ) -> Result<(), LambdaServiceError> {
        let (name, _) = resolve_function_ref(function_ref)?;

        self.store.update(&name, |record| {
            if record.aliases.remove(alias_name).is_none() {
                return Err(LambdaServiceError::AliasNotFound {
                    function_name: name.clone(),
                    alias: alias_name.to_owned(),
                });
            }
            Ok(())
        })??;

        Ok(())
    }

    /// List aliases for a function.
    pub fn list_aliases(
        &self,
        function_ref: &str,
        marker: Option<&str>,
        max_items: Option<usize>,
    ) -> Result<ListAliasesOutput, LambdaServiceError> {
        let (name, _) = resolve_function_ref(function_ref)?;
        let record = self.get_record(&name)?;
        let max = max_items.unwrap_or(50).min(10_000);

        let mut aliases: Vec<(&String, &AliasRecord)> = record.aliases.iter().collect();
        aliases.sort_by_key(|(k, _)| k.as_str());

        let start = marker
            .and_then(|m| aliases.iter().position(|(k, _)| k.as_str() > m))
            .unwrap_or(0);

        let page: Vec<AliasConfiguration> = aliases
            .iter()
            .skip(start)
            .take(max)
            .map(|(_, alias)| {
                let arn = alias_arn(
                    &self.config.default_region,
                    &self.config.account_id,
                    &name,
                    &alias.name,
                );
                AliasConfiguration {
                    alias_arn: Some(arn),
                    name: Some(alias.name.clone()),
                    function_version: Some(alias.function_version.clone()),
                    description: if alias.description.is_empty() {
                        None
                    } else {
                        Some(alias.description.clone())
                    },
                    routing_config: alias.routing_config.as_ref().map(|w| {
                        AliasRoutingConfiguration {
                            additional_version_weights: Some(w.clone()),
                        }
                    }),
                    revision_id: Some(alias.revision_id.clone()),
                }
            })
            .collect();

        let next_marker = if start + max < aliases.len() {
            page.last().and_then(|a| a.name.clone())
        } else {
            None
        };

        Ok(ListAliasesOutput {
            aliases: Some(page),
            next_marker,
        })
    }

    // ---------------------------------------------------------------
    // Phase 2: Permissions + Tags + Account
    // ---------------------------------------------------------------

    /// Add a permission to a function's resource policy.
    pub fn add_permission(
        &self,
        function_ref: &str,
        qualifier: Option<&str>,
        input: &AddPermissionInput,
    ) -> Result<AddPermissionOutput, LambdaServiceError> {
        let (name, ref_qualifier) = resolve_function_ref(function_ref)?;
        let _qualifier = qualifier.or(ref_qualifier.as_deref());

        // Validate required fields per AWS API.
        let sid = match &input.statement_id {
            Some(s) if !s.is_empty() => s.clone(),
            _ => {
                return Err(LambdaServiceError::InvalidParameter {
                    message: "StatementId is required".to_owned(),
                });
            }
        };
        let action = match &input.action {
            Some(a) if !a.is_empty() => a.clone(),
            _ => {
                return Err(LambdaServiceError::InvalidParameter {
                    message: "Action is required".to_owned(),
                });
            }
        };
        let principal = match &input.principal {
            Some(p) if !p.is_empty() => p.clone(),
            _ => {
                return Err(LambdaServiceError::InvalidParameter {
                    message: "Principal is required".to_owned(),
                });
            }
        };

        let resource_arn =
            function_arn(&self.config.default_region, &self.config.account_id, &name);

        let statement = PolicyStatement {
            sid: sid.clone(),
            effect: "Allow".to_owned(),
            principal: principal.clone(),
            action: action.clone(),
            resource: resource_arn.clone(),
            condition: None,
        };

        let statement_json = serde_json::json!({
            "Sid": sid,
            "Effect": "Allow",
            "Principal": { "Service": principal },
            "Action": action,
            "Resource": resource_arn,
        });

        self.store
            .update(&name, |record| -> Result<(), LambdaServiceError> {
                // Check for duplicate statement ID.
                if record.policy.statements.iter().any(|s| s.sid == sid) {
                    return Err(LambdaServiceError::ResourceConflict {
                        message: format!("The statement id ({sid}) provided already exists."),
                    });
                }
                record.policy.statements.push(statement);
                Ok(())
            })??;

        Ok(AddPermissionOutput {
            statement: Some(statement_json.to_string()),
        })
    }

    /// Remove a permission from a function's resource policy.
    pub fn remove_permission(
        &self,
        function_ref: &str,
        statement_id: &str,
        qualifier: Option<&str>,
    ) -> Result<(), LambdaServiceError> {
        let (name, ref_qualifier) = resolve_function_ref(function_ref)?;
        let _qualifier = qualifier.or(ref_qualifier.as_deref());

        self.store.update(&name, |record| {
            let initial_len = record.policy.statements.len();
            record.policy.statements.retain(|s| s.sid != statement_id);
            if record.policy.statements.len() == initial_len {
                return Err(LambdaServiceError::PolicyNotFound {
                    sid: statement_id.to_owned(),
                });
            }
            Ok(())
        })??;

        Ok(())
    }

    /// Get the resource policy for a function.
    pub fn get_policy(
        &self,
        function_ref: &str,
        qualifier: Option<&str>,
    ) -> Result<GetPolicyOutput, LambdaServiceError> {
        let (name, ref_qualifier) = resolve_function_ref(function_ref)?;
        let _qualifier = qualifier.or(ref_qualifier.as_deref());

        let record = self.get_record(&name)?;

        if record.policy.statements.is_empty() {
            return Err(LambdaServiceError::PolicyNotFound { sid: name.clone() });
        }

        let statements: Vec<serde_json::Value> = record
            .policy
            .statements
            .iter()
            .map(|s| {
                serde_json::json!({
                    "Sid": s.sid,
                    "Effect": s.effect,
                    "Principal": { "Service": s.principal },
                    "Action": s.action,
                    "Resource": s.resource,
                })
            })
            .collect();

        let policy = serde_json::json!({
            "Version": "2012-10-17",
            "Id": "default",
            "Statement": statements,
        });

        Ok(GetPolicyOutput {
            policy: Some(policy.to_string()),
            revision_id: Some(record.latest.revision_id.clone()),
        })
    }

    /// Tag a resource.
    pub fn tag_resource(
        &self,
        resource_arn: &str,
        input: &TagResourceInput,
    ) -> Result<(), LambdaServiceError> {
        let name = Self::extract_function_name_from_arn(resource_arn)?;

        // Validate tag count after merge (Appendix C: max 50).
        let record = self.get_record(&name)?;
        let new_count = {
            let mut merged = record.tags.clone();
            merged.extend(input.tags.clone());
            merged.len()
        };
        if new_count > 50 {
            return Err(LambdaServiceError::InvalidParameter {
                message: "Tags count exceeds the limit of 50".to_owned(),
            });
        }

        self.store.update(&name, |record| {
            record.tags.extend(input.tags.clone());
        })?;

        Ok(())
    }

    /// Remove tags from a resource.
    pub fn untag_resource(
        &self,
        resource_arn: &str,
        tag_keys: &[String],
    ) -> Result<(), LambdaServiceError> {
        let name = Self::extract_function_name_from_arn(resource_arn)?;

        self.store.update(&name, |record| {
            for key in tag_keys {
                record.tags.remove(key);
            }
        })?;

        Ok(())
    }

    /// List tags for a resource.
    pub fn list_tags(&self, resource_arn: &str) -> Result<ListTagsOutput, LambdaServiceError> {
        let name = Self::extract_function_name_from_arn(resource_arn)?;
        let record = self.get_record(&name)?;

        let tags = if record.tags.is_empty() {
            None
        } else {
            Some(record.tags.clone())
        };

        Ok(ListTagsOutput { tags })
    }

    /// Get account settings.
    #[must_use]
    pub fn get_account_settings(&self) -> GetAccountSettingsOutput {
        #[allow(clippy::cast_possible_wrap)]
        let function_count = self.store.len() as i64;

        GetAccountSettingsOutput {
            account_limit: Some(AccountLimit {
                total_code_size: Some(80_530_636_800),
                code_size_unzipped: Some(262_144_000),
                code_size_zipped: Some(52_428_800),
                concurrent_executions: Some(1000),
                unreserved_concurrent_executions: Some(1000),
            }),
            account_usage: Some(AccountUsage {
                total_code_size: Some(0),
                function_count: Some(function_count),
            }),
        }
    }

    // ---------------------------------------------------------------
    // Phase 3: Function URLs
    // ---------------------------------------------------------------

    /// Create a function URL configuration.
    pub fn create_function_url_config(
        &self,
        function_ref: &str,
        qualifier: Option<&str>,
        input: CreateFunctionUrlConfigInput,
    ) -> Result<FunctionUrlConfig, LambdaServiceError> {
        let (name, ref_qualifier) = resolve_function_ref(function_ref)?;
        let _qualifier = qualifier.or(ref_qualifier.as_deref());

        let now = now_iso8601();
        // Use local URL format for development: http://{host}:{port}/lambda-url/{name}/
        let function_url = format!(
            "http://{}:{}/lambda-url/{name}/",
            self.config.host, self.config.port,
        );

        let function_arn_str =
            function_arn(&self.config.default_region, &self.config.account_id, &name);

        let invoke_mode = input
            .invoke_mode
            .clone()
            .unwrap_or_else(|| "BUFFERED".to_owned());

        let url_record = FunctionUrlConfigRecord {
            function_url: function_url.clone(),
            auth_type: input.auth_type.clone(),
            cors: input.cors.clone(),
            invoke_mode: invoke_mode.clone(),
            creation_time: now.clone(),
            last_modified_time: now.clone(),
        };

        self.store
            .update(&name, |record| -> Result<(), LambdaServiceError> {
                if record.url_config.is_some() {
                    return Err(LambdaServiceError::ResourceConflict {
                        message: format!("Function URL config already exists for: {name}"),
                    });
                }
                record.url_config = Some(url_record);
                Ok(())
            })??;

        Ok(FunctionUrlConfig {
            function_url: Some(function_url),
            function_arn: Some(function_arn_str),
            auth_type: Some(input.auth_type),
            cors: input.cors,
            creation_time: Some(now.clone()),
            last_modified_time: Some(now),
            invoke_mode: Some(invoke_mode),
        })
    }

    /// Get function URL configuration.
    pub fn get_function_url_config(
        &self,
        function_ref: &str,
        qualifier: Option<&str>,
    ) -> Result<FunctionUrlConfig, LambdaServiceError> {
        let (name, ref_qualifier) = resolve_function_ref(function_ref)?;
        let _qualifier = qualifier.or(ref_qualifier.as_deref());

        let record = self.get_record(&name)?;
        let url_config =
            record
                .url_config
                .as_ref()
                .ok_or(LambdaServiceError::FunctionNotFound {
                    name: format!("{name} (no URL config)"),
                })?;

        let function_arn_str =
            function_arn(&self.config.default_region, &self.config.account_id, &name);

        Ok(FunctionUrlConfig {
            function_url: Some(url_config.function_url.clone()),
            function_arn: Some(function_arn_str),
            auth_type: Some(url_config.auth_type.clone()),
            cors: url_config.cors.clone(),
            creation_time: Some(url_config.creation_time.clone()),
            last_modified_time: Some(url_config.last_modified_time.clone()),
            invoke_mode: Some(url_config.invoke_mode.clone()),
        })
    }

    /// Update function URL configuration.
    pub fn update_function_url_config(
        &self,
        function_ref: &str,
        qualifier: Option<&str>,
        input: &UpdateFunctionUrlConfigInput,
    ) -> Result<FunctionUrlConfig, LambdaServiceError> {
        let (name, ref_qualifier) = resolve_function_ref(function_ref)?;
        let _qualifier = qualifier.or(ref_qualifier.as_deref());

        let function_arn_str =
            function_arn(&self.config.default_region, &self.config.account_id, &name);

        let config = self.store.update(
            &name,
            |record| -> Result<FunctionUrlConfig, LambdaServiceError> {
                let url_config =
                    record
                        .url_config
                        .as_mut()
                        .ok_or(LambdaServiceError::FunctionNotFound {
                            name: format!("{name} (no URL config)"),
                        })?;

                if let Some(auth_type) = &input.auth_type {
                    url_config.auth_type.clone_from(auth_type);
                }
                if let Some(cors) = &input.cors {
                    url_config.cors = Some(cors.clone());
                }
                if let Some(invoke_mode) = &input.invoke_mode {
                    url_config.invoke_mode.clone_from(invoke_mode);
                }
                url_config.last_modified_time = now_iso8601();

                Ok(FunctionUrlConfig {
                    function_url: Some(url_config.function_url.clone()),
                    function_arn: Some(function_arn_str.clone()),
                    auth_type: Some(url_config.auth_type.clone()),
                    cors: url_config.cors.clone(),
                    creation_time: Some(url_config.creation_time.clone()),
                    last_modified_time: Some(url_config.last_modified_time.clone()),
                    invoke_mode: Some(url_config.invoke_mode.clone()),
                })
            },
        )??;

        Ok(config)
    }

    /// Delete function URL configuration.
    pub fn delete_function_url_config(
        &self,
        function_ref: &str,
        qualifier: Option<&str>,
    ) -> Result<(), LambdaServiceError> {
        let (name, ref_qualifier) = resolve_function_ref(function_ref)?;
        let _qualifier = qualifier.or(ref_qualifier.as_deref());

        self.store
            .update(&name, |record| -> Result<(), LambdaServiceError> {
                if record.url_config.is_none() {
                    return Err(LambdaServiceError::FunctionNotFound {
                        name: format!("{name} (no URL config)"),
                    });
                }
                record.url_config = None;
                Ok(())
            })??;

        Ok(())
    }

    /// List function URL configurations.
    pub fn list_function_url_configs(
        &self,
        function_ref: &str,
    ) -> Result<ListFunctionUrlConfigsOutput, LambdaServiceError> {
        let (name, _) = resolve_function_ref(function_ref)?;
        let record = self.get_record(&name)?;

        let function_arn_str =
            function_arn(&self.config.default_region, &self.config.account_id, &name);

        let configs = match &record.url_config {
            Some(url_config) => vec![FunctionUrlConfig {
                function_url: Some(url_config.function_url.clone()),
                function_arn: Some(function_arn_str),
                auth_type: Some(url_config.auth_type.clone()),
                cors: url_config.cors.clone(),
                creation_time: Some(url_config.creation_time.clone()),
                last_modified_time: Some(url_config.last_modified_time.clone()),
                invoke_mode: Some(url_config.invoke_mode.clone()),
            }],
            None => Vec::new(),
        };

        Ok(ListFunctionUrlConfigsOutput {
            function_url_configs: Some(configs),
            next_marker: None,
        })
    }

    // ---------------------------------------------------------------
    // Internal helpers
    // ---------------------------------------------------------------

    /// Get a function record by name, returning `FunctionNotFound` if absent.
    fn get_record(&self, name: &str) -> Result<FunctionRecord, LambdaServiceError> {
        self.store
            .get(name)
            .ok_or(LambdaServiceError::FunctionNotFound {
                name: name.to_owned(),
            })
    }

    /// Process code input (zip or image URI), returning code metadata.
    async fn process_code(
        &self,
        function_name: &str,
        version: &str,
        zip_file_b64: Option<&str>,
        image_uri: Option<&str>,
    ) -> Result<
        (
            String,
            u64,
            Option<Bytes>,
            Option<std::path::PathBuf>,
            Option<String>,
        ),
        LambdaServiceError,
    > {
        if let Some(b64) = zip_file_b64 {
            use base64::Engine;
            let zip_bytes = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .map_err(|e| LambdaServiceError::InvalidZipFile {
                    message: format!("Invalid base64 encoding: {e}"),
                })?;

            let (code_path, sha256, size) = self
                .store
                .store_zip_code(function_name, version, &zip_bytes)
                .await?;

            Ok((
                sha256,
                size,
                Some(Bytes::from(zip_bytes)),
                Some(code_path),
                None,
            ))
        } else if let Some(uri) = image_uri {
            let sha256 = compute_sha256(uri.as_bytes());
            Ok((sha256, 0, None, None, Some(uri.to_owned())))
        } else {
            // No code provided - use empty hash.
            let sha256 = compute_sha256(b"");
            Ok((sha256, 0, None, None, None))
        }
    }

    /// Build a `FunctionConfiguration` from internal records.
    fn build_function_configuration(
        &self,
        record: &FunctionRecord,
        version: &VersionRecord,
    ) -> FunctionConfiguration {
        let arn = if version.version == "$LATEST" {
            record.arn.clone()
        } else {
            function_version_arn(
                &self.config.default_region,
                &self.config.account_id,
                &record.name,
                &version.version,
            )
        };

        let env_response = if version.environment.is_empty() {
            None
        } else {
            Some(EnvironmentResponse {
                variables: Some(version.environment.clone()),
                error: None,
            })
        };

        let vpc_response = version.vpc_config.as_ref().map(|vpc| VpcConfigResponse {
            subnet_ids: vpc.subnet_ids.clone(),
            security_group_ids: vpc.security_group_ids.clone(),
            vpc_id: None,
        });

        let tracing_response = version
            .tracing_config
            .as_ref()
            .map(|tc| TracingConfigResponse {
                mode: tc.mode.clone(),
            });

        let layers = if version.layers.is_empty() {
            None
        } else {
            Some(
                version
                    .layers
                    .iter()
                    .map(|l| Layer {
                        arn: Some(l.clone()),
                        code_size: None,
                        signing_profile_version_arn: None,
                        signing_job_arn: None,
                    })
                    .collect(),
            )
        };

        let image_config_response = version.image_config.as_ref().map(|ic| ImageConfigResponse {
            image_config: Some(ic.clone()),
            error: None,
        });

        let snap_start_response = version.snap_start.as_ref().map(|ss| SnapStartResponse {
            apply_on: ss.apply_on.clone(),
            optimization_status: Some("Off".to_owned()),
        });

        FunctionConfiguration {
            function_name: Some(record.name.clone()),
            function_arn: Some(arn),
            runtime: version.runtime.clone(),
            role: Some(version.role.clone()),
            handler: version.handler.clone(),
            #[allow(clippy::cast_possible_wrap)]
            code_size: Some(version.code_size as i64),
            description: if version.description.is_empty() {
                None
            } else {
                Some(version.description.clone())
            },
            timeout: Some(version.timeout),
            memory_size: Some(version.memory_size),
            last_modified: Some(version.last_modified.clone()),
            code_sha256: Some(version.code_sha256.clone()),
            version: Some(version.version.clone()),
            environment: env_response,
            vpc_config: vpc_response,
            dead_letter_config: version.dead_letter_config.clone(),
            tracing_config: tracing_response,
            revision_id: Some(version.revision_id.clone()),
            layers,
            state: Some(version.state.clone()),
            state_reason: None,
            state_reason_code: None,
            package_type: Some(version.package_type.clone()),
            architectures: Some(version.architectures.clone()),
            ephemeral_storage: Some(EphemeralStorage {
                size: version.ephemeral_storage_size,
            }),
            logging_config: version.logging_config.clone(),
            snap_start: snap_start_response,
            image_config_response,
            last_update_status: Some("Successful".to_owned()),
            last_update_status_reason: None,
            last_update_status_reason_code: None,
        }
    }

    /// Extract function name from a Lambda function ARN.
    fn extract_function_name_from_arn(arn: &str) -> Result<String, LambdaServiceError> {
        // Try ARN parsing first.
        if arn.starts_with("arn:") {
            let (name, _) = resolve_function_ref(arn)?;
            return Ok(name);
        }
        // If it's not an ARN, treat it as a function name.
        Ok(arn.to_owned())
    }
}

/// Get current time in ISO 8601 format matching AWS Lambda conventions.
fn now_iso8601() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3f+0000")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_provider() -> RustStackLambda {
        let tmp = tempfile::tempdir().unwrap();
        let store = FunctionStore::new(tmp.path());
        let config = LambdaConfig::default();
        RustStackLambda::with_store(store, config)
    }

    fn sample_create_input(name: &str) -> CreateFunctionInput {
        use base64::Engine;
        let zip_data = base64::engine::general_purpose::STANDARD.encode(b"PK\x03\x04fake");
        CreateFunctionInput {
            function_name: name.to_owned(),
            runtime: Some("python3.12".to_owned()),
            role: "arn:aws:iam::000000000000:role/test-role".to_owned(),
            handler: Some("index.handler".to_owned()),
            code: ruststack_lambda_model::types::FunctionCode {
                zip_file: Some(zip_data),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_should_create_and_get_function() {
        let provider = test_provider();

        let config = provider
            .create_function(sample_create_input("my-func"))
            .await
            .unwrap();
        assert_eq!(config.function_name, Some("my-func".to_owned()));
        assert_eq!(config.runtime, Some("python3.12".to_owned()));
        assert_eq!(config.state, Some("Active".to_owned()));

        let output = provider.get_function("my-func", None).unwrap();
        assert_eq!(
            output.configuration.as_ref().unwrap().function_name,
            Some("my-func".to_owned()),
        );
    }

    #[tokio::test]
    async fn test_should_reject_duplicate_create() {
        let provider = test_provider();
        provider
            .create_function(sample_create_input("my-func"))
            .await
            .unwrap();

        let err = provider
            .create_function(sample_create_input("my-func"))
            .await
            .unwrap_err();
        assert!(matches!(err, LambdaServiceError::ResourceConflict { .. }));
    }

    #[tokio::test]
    async fn test_should_get_function_configuration() {
        let provider = test_provider();
        provider
            .create_function(sample_create_input("my-func"))
            .await
            .unwrap();

        let config = provider
            .get_function_configuration("my-func", None)
            .unwrap();
        assert_eq!(config.function_name, Some("my-func".to_owned()));
        assert_eq!(config.handler, Some("index.handler".to_owned()));
    }

    #[tokio::test]
    async fn test_should_update_function_configuration() {
        let provider = test_provider();
        provider
            .create_function(sample_create_input("my-func"))
            .await
            .unwrap();

        let input = UpdateFunctionConfigurationInput {
            timeout: Some(30),
            memory_size: Some(256),
            description: Some("Updated function".to_owned()),
            ..Default::default()
        };

        let config = provider
            .update_function_configuration("my-func", &input)
            .unwrap();
        assert_eq!(config.timeout, Some(30));
        assert_eq!(config.memory_size, Some(256));
        assert_eq!(config.description, Some("Updated function".to_owned()));
    }

    #[tokio::test]
    async fn test_should_update_function_code() {
        use base64::Engine;

        let provider = test_provider();
        provider
            .create_function(sample_create_input("my-func"))
            .await
            .unwrap();

        let new_zip = base64::engine::general_purpose::STANDARD.encode(b"PK\x03\x04new-code");
        let input = UpdateFunctionCodeInput {
            zip_file: Some(new_zip),
            ..Default::default()
        };

        let config = provider
            .update_function_code("my-func", input)
            .await
            .unwrap();
        assert_eq!(config.function_name, Some("my-func".to_owned()));
        // Code size should change.
        assert!(config.code_size.unwrap() > 0);
    }

    #[tokio::test]
    async fn test_should_delete_function() {
        let provider = test_provider();
        provider
            .create_function(sample_create_input("my-func"))
            .await
            .unwrap();

        provider.delete_function("my-func", None).await.unwrap();

        let err = provider.get_function("my-func", None).unwrap_err();
        assert!(matches!(err, LambdaServiceError::FunctionNotFound { .. }));
    }

    #[tokio::test]
    async fn test_should_list_functions() {
        let provider = test_provider();
        provider
            .create_function(sample_create_input("alpha"))
            .await
            .unwrap();
        provider
            .create_function(sample_create_input("bravo"))
            .await
            .unwrap();

        let output = provider.list_functions(None, None);
        assert_eq!(output.functions.as_ref().unwrap().len(), 2);
        assert_eq!(
            output.functions.as_ref().unwrap()[0].function_name,
            Some("alpha".to_owned()),
        );
    }

    #[tokio::test]
    async fn test_should_invoke_dry_run() {
        let provider = test_provider();
        provider
            .create_function(sample_create_input("my-func"))
            .await
            .unwrap();

        let (status, body) = provider.invoke("my-func", None, b"{}", true).unwrap();
        assert_eq!(status, 204);
        assert!(body.is_empty());
    }

    #[tokio::test]
    async fn test_should_error_invoke_without_docker() {
        let provider = test_provider();
        provider
            .create_function(sample_create_input("my-func"))
            .await
            .unwrap();

        let err = provider.invoke("my-func", None, b"{}", false).unwrap_err();
        assert!(matches!(err, LambdaServiceError::DockerNotAvailable));
    }

    #[tokio::test]
    async fn test_should_error_on_nonexistent_function() {
        let provider = test_provider();
        let err = provider.get_function("nonexistent", None).unwrap_err();
        assert!(matches!(err, LambdaServiceError::FunctionNotFound { .. }));
    }

    #[tokio::test]
    async fn test_should_reject_create_without_code() {
        let provider = test_provider();
        let input = CreateFunctionInput {
            function_name: "my-func".to_owned(),
            runtime: Some("python3.12".to_owned()),
            role: "arn:aws:iam::000000000000:role/test-role".to_owned(),
            handler: Some("index.handler".to_owned()),
            code: ruststack_lambda_model::types::FunctionCode::default(),
            ..Default::default()
        };
        let err = provider.create_function(input).await.unwrap_err();
        assert!(matches!(err, LambdaServiceError::InvalidParameter { .. }));
    }

    #[tokio::test]
    async fn test_should_reject_create_without_runtime_for_zip() {
        use base64::Engine;
        let provider = test_provider();
        let zip_data = base64::engine::general_purpose::STANDARD.encode(b"PK\x03\x04fake");
        let input = CreateFunctionInput {
            function_name: "my-func".to_owned(),
            runtime: None,
            role: "arn:aws:iam::000000000000:role/test-role".to_owned(),
            handler: Some("index.handler".to_owned()),
            code: ruststack_lambda_model::types::FunctionCode {
                zip_file: Some(zip_data),
                ..Default::default()
            },
            ..Default::default()
        };
        let err = provider.create_function(input).await.unwrap_err();
        assert!(matches!(err, LambdaServiceError::InvalidParameter { .. }));
    }

    #[tokio::test]
    async fn test_should_reject_update_code_without_source() {
        let provider = test_provider();
        provider
            .create_function(sample_create_input("my-func"))
            .await
            .unwrap();

        let input = UpdateFunctionCodeInput::default();
        let err = provider
            .update_function_code("my-func", input)
            .await
            .unwrap_err();
        assert!(matches!(err, LambdaServiceError::InvalidParameter { .. }));
    }

    #[tokio::test]
    async fn test_should_publish_on_create() {
        let provider = test_provider();
        let mut input = sample_create_input("my-func");
        input.publish = Some(true);

        let config = provider.create_function(input).await.unwrap();
        // When publish=true, the returned config should be version "1".
        assert_eq!(config.version, Some("1".to_owned()));

        // The function should have version 1 in versions.
        let versions = provider
            .list_versions_by_function("my-func", None, None)
            .unwrap();
        assert_eq!(versions.versions.as_ref().unwrap().len(), 2); // $LATEST + 1
    }

    #[tokio::test]
    async fn test_should_reject_alias_to_nonexistent_version() {
        let provider = test_provider();
        provider
            .create_function(sample_create_input("my-func"))
            .await
            .unwrap();

        let input = CreateAliasInput {
            name: "prod".to_owned(),
            function_version: "99".to_owned(),
            ..Default::default()
        };
        let err = provider.create_alias("my-func", input).unwrap_err();
        assert!(matches!(err, LambdaServiceError::VersionNotFound { .. }));
    }

    #[tokio::test]
    async fn test_should_reject_duplicate_permission_sid() {
        let provider = test_provider();
        provider
            .create_function(sample_create_input("my-func"))
            .await
            .unwrap();

        let input = AddPermissionInput {
            statement_id: Some("stmt-1".to_owned()),
            action: Some("lambda:InvokeFunction".to_owned()),
            principal: Some("s3.amazonaws.com".to_owned()),
            ..Default::default()
        };
        provider.add_permission("my-func", None, &input).unwrap();

        // Adding same SID again should fail.
        let err = provider
            .add_permission("my-func", None, &input)
            .unwrap_err();
        assert!(matches!(err, LambdaServiceError::ResourceConflict { .. }));
    }

    #[tokio::test]
    async fn test_should_generate_local_function_url() {
        let provider = test_provider();
        provider
            .create_function(sample_create_input("my-func"))
            .await
            .unwrap();

        let input = CreateFunctionUrlConfigInput {
            auth_type: "NONE".to_owned(),
            ..Default::default()
        };
        let url_config = provider
            .create_function_url_config("my-func", None, input)
            .unwrap();
        let url = url_config.function_url.unwrap();
        assert!(
            url.starts_with("http://localhost:4566/lambda-url/my-func/"),
            "URL should use local format, got: {url}"
        );
    }

    #[tokio::test]
    async fn test_should_paginate_versions() {
        let provider = test_provider();
        provider
            .create_function(sample_create_input("my-func"))
            .await
            .unwrap();

        // Publish 3 versions.
        for _ in 0..3 {
            provider
                .publish_version("my-func", &PublishVersionInput::default())
                .unwrap();
        }

        // Get first page of 2.
        let output = provider
            .list_versions_by_function("my-func", None, Some(2))
            .unwrap();
        assert_eq!(output.versions.as_ref().unwrap().len(), 2);
        assert!(output.next_marker.is_some());

        // Get next page.
        let output2 = provider
            .list_versions_by_function("my-func", output.next_marker.as_deref(), Some(2))
            .unwrap();
        assert_eq!(output2.versions.as_ref().unwrap().len(), 2);
    }
}
