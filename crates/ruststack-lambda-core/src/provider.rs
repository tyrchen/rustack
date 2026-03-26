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

use ruststack_lambda_model::{
    input::{
        AddLayerVersionPermissionInput, AddPermissionInput, CreateAliasInput,
        CreateEventSourceMappingInput, CreateFunctionInput, CreateFunctionUrlConfigInput,
        EventInvokeConfigInput, PublishLayerVersionInput, PublishVersionInput, TagResourceInput,
        UpdateAliasInput, UpdateEventSourceMappingInput, UpdateFunctionCodeInput,
        UpdateFunctionConfigurationInput, UpdateFunctionUrlConfigInput,
    },
    output::{
        AccountLimit, AccountUsage, AddLayerVersionPermissionOutput, AddPermissionOutput,
        GetAccountSettingsOutput, GetFunctionOutput, GetLayerVersionPolicyOutput, GetPolicyOutput,
        ListAliasesOutput, ListEventSourceMappingsOutput, ListFunctionUrlConfigsOutput,
        ListFunctionsOutput, ListLayerVersionsOutput, ListLayersOutput, ListTagsOutput,
        ListVersionsOutput, PublishLayerVersionOutput,
    },
    types::{
        AliasConfiguration, AliasRoutingConfiguration, Concurrency, EnvironmentResponse,
        EphemeralStorage, EventSourceMappingConfiguration, FunctionCodeLocation,
        FunctionConfiguration, FunctionEventInvokeConfig, FunctionUrlConfig, ImageConfigResponse,
        Layer, LayerVersionContentOutput, LayerVersionsListItem, LayersListItem, SnapStartResponse,
        TracingConfigResponse, VpcConfigResponse,
    },
};

use crate::{
    config::LambdaConfig,
    error::LambdaServiceError,
    resolver::{
        alias_arn, function_arn, function_version_arn, layer_arn, layer_version_arn,
        parse_layer_version_arn, resolve_function_ref, resolve_version,
    },
    storage::{
        AliasRecord, EventInvokeConfigRecord, EventSourceMappingRecord, EventSourceMappingStore,
        FunctionRecord, FunctionStore, FunctionUrlConfigRecord, LayerStore, LayerVersionRecord,
        PolicyDocument, PolicyStatement, VersionRecord, compute_sha256,
    },
};

/// Lambda business logic provider.
///
/// Holds the function store, layer store, and service configuration.
/// All operations are implemented as methods that return domain types or errors.
#[derive(Debug)]
pub struct RustStackLambda {
    store: FunctionStore,
    layer_store: LayerStore,
    esm_store: EventSourceMappingStore,
    config: LambdaConfig,
}

impl RustStackLambda {
    /// Create a new Lambda provider with the given store and config.
    #[must_use]
    pub fn with_store(store: FunctionStore, config: LambdaConfig) -> Self {
        Self {
            store,
            layer_store: LayerStore::new(),
            esm_store: EventSourceMappingStore::new(),
            config,
        }
    }

    /// Create a new Lambda provider from config, using a temp directory for code.
    #[must_use]
    pub fn new(config: LambdaConfig) -> Self {
        let code_dir = std::env::temp_dir().join("ruststack-lambda-code");
        let store = FunctionStore::new(code_dir);
        Self {
            store,
            layer_store: LayerStore::new(),
            esm_store: EventSourceMappingStore::new(),
            config,
        }
    }

    /// Returns a reference to the underlying function store.
    #[must_use]
    pub fn store(&self) -> &FunctionStore {
        &self.store
    }

    /// Returns a reference to the layer store.
    #[must_use]
    pub fn layer_store(&self) -> &LayerStore {
        &self.layer_store
    }

    /// Returns a reference to the event source mapping store.
    #[must_use]
    pub fn esm_store(&self) -> &EventSourceMappingStore {
        &self.esm_store
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
            reserved_concurrent_executions: None,
            event_invoke_configs: HashMap::new(),
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
                    "Request payload size {payload_len} exceeds the synchronous invoke limit of \
                     {MAX_SYNC_PAYLOAD} bytes",
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
    // Phase 2b: Lambda Layers
    // ---------------------------------------------------------------

    /// Publish a new layer version.
    pub fn publish_layer_version(
        &self,
        layer_name: &str,
        input: &PublishLayerVersionInput,
    ) -> Result<PublishLayerVersionOutput, LambdaServiceError> {
        if layer_name.is_empty() || layer_name.len() > 140 {
            return Err(LambdaServiceError::InvalidParameter {
                message: "Layer name must be between 1 and 140 characters".to_owned(),
            });
        }

        if let Some(ref license) = input.license_info {
            if license.len() > 512 {
                return Err(LambdaServiceError::InvalidParameter {
                    message: "License info must be at most 512 characters".to_owned(),
                });
            }
        }

        // Process layer code.
        let (code_sha256, code_size) = if let Some(ref content) = input.content {
            if let Some(ref b64) = content.zip_file {
                use base64::Engine;
                let zip_bytes = base64::engine::general_purpose::STANDARD
                    .decode(b64)
                    .map_err(|e| LambdaServiceError::InvalidZipFile {
                        message: format!("Invalid base64 encoding: {e}"),
                    })?;
                let sha256 = compute_sha256(&zip_bytes);
                let size = zip_bytes.len() as u64;
                (sha256, size)
            } else {
                // S3 source accepted but not functional; use empty hash.
                (compute_sha256(b""), 0)
            }
        } else {
            (compute_sha256(b""), 0)
        };

        let now = now_iso8601();
        let la = layer_arn(
            &self.config.default_region,
            &self.config.account_id,
            layer_name,
        );

        // Create a temporary version record; the store will assign the actual version number.
        let version_record = LayerVersionRecord {
            version: 0, // will be overwritten
            description: input.description.clone().unwrap_or_default(),
            compatible_runtimes: input.compatible_runtimes.clone().unwrap_or_default(),
            compatible_architectures: input.compatible_architectures.clone().unwrap_or_default(),
            license_info: input.license_info.clone(),
            code_sha256: code_sha256.clone(),
            code_size,
            created_date: now.clone(),
            layer_arn: la.clone(),
            layer_version_arn: String::new(), // will be overwritten
            policy: PolicyDocument::default(),
        };

        let version_num = self
            .layer_store
            .publish_version(layer_name, &la, version_record);

        // Update the version number and ARN in the stored record.
        let lva = layer_version_arn(
            &self.config.default_region,
            &self.config.account_id,
            layer_name,
            version_num,
        );
        self.layer_store
            .update_version(layer_name, version_num, |ver| {
                ver.version = version_num;
                ver.layer_version_arn.clone_from(&lva);
            })?;

        info!(layer_name = %layer_name, version = %version_num, "published layer version");

        #[allow(clippy::cast_possible_wrap)]
        Ok(PublishLayerVersionOutput {
            content: Some(LayerVersionContentOutput {
                code_sha256: Some(code_sha256),
                code_size: Some(code_size as i64),
                ..Default::default()
            }),
            layer_arn: Some(la),
            layer_version_arn: Some(lva),
            description: input.description.clone(),
            created_date: Some(now),
            version: Some(version_num as i64),
            compatible_runtimes: input.compatible_runtimes.clone(),
            license_info: input.license_info.clone(),
            compatible_architectures: input.compatible_architectures.clone(),
        })
    }

    /// Get a specific layer version.
    pub fn get_layer_version(
        &self,
        layer_name: &str,
        version_number: u64,
    ) -> Result<PublishLayerVersionOutput, LambdaServiceError> {
        let ver = self
            .layer_store
            .get_version(layer_name, version_number)
            .ok_or(LambdaServiceError::InvalidParameter {
                message: format!("Layer version not found: {layer_name}:{version_number}"),
            })?;

        Ok(Self::build_layer_version_output(&ver))
    }

    /// Get a layer version by its full ARN.
    pub fn get_layer_version_by_arn(
        &self,
        arn: &str,
    ) -> Result<PublishLayerVersionOutput, LambdaServiceError> {
        let (name, version) = parse_layer_version_arn(arn)?;
        self.get_layer_version(&name, version)
    }

    /// List versions of a layer.
    pub fn list_layer_versions(
        &self,
        layer_name: &str,
        marker: Option<&str>,
        max_items: Option<usize>,
    ) -> Result<ListLayerVersionsOutput, LambdaServiceError> {
        let versions = self.layer_store.list_versions(layer_name);
        let max = max_items.unwrap_or(50).min(10_000);

        let start = marker
            .and_then(|m| m.parse::<u64>().ok())
            .and_then(|marker_ver| versions.iter().position(|v| v.version > marker_ver))
            .unwrap_or(0);

        #[allow(clippy::cast_possible_wrap)]
        let page: Vec<LayerVersionsListItem> = versions
            .iter()
            .skip(start)
            .take(max)
            .map(|v| LayerVersionsListItem {
                layer_version_arn: Some(v.layer_version_arn.clone()),
                version: Some(v.version as i64),
                description: if v.description.is_empty() {
                    None
                } else {
                    Some(v.description.clone())
                },
                created_date: Some(v.created_date.clone()),
                compatible_runtimes: if v.compatible_runtimes.is_empty() {
                    None
                } else {
                    Some(v.compatible_runtimes.clone())
                },
                license_info: v.license_info.clone(),
                compatible_architectures: if v.compatible_architectures.is_empty() {
                    None
                } else {
                    Some(v.compatible_architectures.clone())
                },
            })
            .collect();

        let next_marker = if start + max < versions.len() {
            page.last()
                .and_then(|v| v.version.map(|ver| ver.to_string()))
        } else {
            None
        };

        Ok(ListLayerVersionsOutput {
            layer_versions: Some(page),
            next_marker,
        })
    }

    /// List all layers.
    #[must_use]
    pub fn list_layers(&self, marker: Option<&str>, max_items: Option<usize>) -> ListLayersOutput {
        let all = self.layer_store.list_layers();
        let max = max_items.unwrap_or(50).min(10_000);

        let start = marker
            .and_then(|m| all.iter().position(|r| r.name.as_str() > m))
            .unwrap_or(0);

        #[allow(clippy::cast_possible_wrap)]
        let page: Vec<LayersListItem> = all
            .iter()
            .skip(start)
            .take(max)
            .map(|r| {
                let latest = r
                    .versions
                    .values()
                    .next_back()
                    .map(|v| LayerVersionsListItem {
                        layer_version_arn: Some(v.layer_version_arn.clone()),
                        version: Some(v.version as i64),
                        description: if v.description.is_empty() {
                            None
                        } else {
                            Some(v.description.clone())
                        },
                        created_date: Some(v.created_date.clone()),
                        compatible_runtimes: if v.compatible_runtimes.is_empty() {
                            None
                        } else {
                            Some(v.compatible_runtimes.clone())
                        },
                        license_info: v.license_info.clone(),
                        compatible_architectures: if v.compatible_architectures.is_empty() {
                            None
                        } else {
                            Some(v.compatible_architectures.clone())
                        },
                    });
                LayersListItem {
                    layer_name: Some(r.name.clone()),
                    layer_arn: Some(r.layer_arn.clone()),
                    latest_matching_version: latest,
                }
            })
            .collect();

        let next_marker = if start + max < all.len() {
            page.last().and_then(|l| l.layer_name.clone())
        } else {
            None
        };

        ListLayersOutput {
            layers: Some(page),
            next_marker,
        }
    }

    /// Delete a layer version.
    pub fn delete_layer_version(
        &self,
        layer_name: &str,
        version_number: u64,
    ) -> Result<(), LambdaServiceError> {
        // AWS silently succeeds even if the version doesn't exist.
        let _ = self.layer_store.delete_version(layer_name, version_number);
        info!(layer_name = %layer_name, version = %version_number, "deleted layer version");
        Ok(())
    }

    /// Add a permission to a layer version's resource policy.
    pub fn add_layer_version_permission(
        &self,
        layer_name: &str,
        version_number: u64,
        input: &AddLayerVersionPermissionInput,
    ) -> Result<AddLayerVersionPermissionOutput, LambdaServiceError> {
        // Validate required fields.
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

        let lva = layer_version_arn(
            &self.config.default_region,
            &self.config.account_id,
            layer_name,
            version_number,
        );

        let statement = PolicyStatement {
            sid: sid.clone(),
            effect: "Allow".to_owned(),
            principal: principal.clone(),
            action: action.clone(),
            resource: lva,
            condition: None,
        };

        let statement_json = serde_json::json!({
            "Sid": sid,
            "Effect": "Allow",
            "Principal": { "Service": principal },
            "Action": action,
            "Resource": statement.resource,
        });

        let revision_id = self.layer_store.update_version(
            layer_name,
            version_number,
            |ver| -> Result<String, LambdaServiceError> {
                if ver.policy.statements.iter().any(|s| s.sid == sid) {
                    return Err(LambdaServiceError::ResourceConflict {
                        message: format!("The statement id ({sid}) provided already exists."),
                    });
                }
                ver.policy.statements.push(statement);
                Ok(uuid::Uuid::new_v4().to_string())
            },
        )??;

        Ok(AddLayerVersionPermissionOutput {
            statement: Some(statement_json.to_string()),
            revision_id: Some(revision_id),
        })
    }

    /// Get the resource policy for a layer version.
    pub fn get_layer_version_policy(
        &self,
        layer_name: &str,
        version_number: u64,
    ) -> Result<GetLayerVersionPolicyOutput, LambdaServiceError> {
        let ver = self
            .layer_store
            .get_version(layer_name, version_number)
            .ok_or(LambdaServiceError::InvalidParameter {
                message: format!("Layer version not found: {layer_name}:{version_number}"),
            })?;

        if ver.policy.statements.is_empty() {
            return Err(LambdaServiceError::PolicyNotFound {
                sid: format!("{layer_name}:{version_number}"),
            });
        }

        let statements: Vec<serde_json::Value> = ver
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

        Ok(GetLayerVersionPolicyOutput {
            policy: Some(policy.to_string()),
            revision_id: Some(uuid::Uuid::new_v4().to_string()),
        })
    }

    /// Remove a permission from a layer version's resource policy.
    pub fn remove_layer_version_permission(
        &self,
        layer_name: &str,
        version_number: u64,
        statement_id: &str,
    ) -> Result<(), LambdaServiceError> {
        self.layer_store.update_version(
            layer_name,
            version_number,
            |ver| -> Result<(), LambdaServiceError> {
                let initial_len = ver.policy.statements.len();
                ver.policy.statements.retain(|s| s.sid != statement_id);
                if ver.policy.statements.len() == initial_len {
                    return Err(LambdaServiceError::PolicyNotFound {
                        sid: statement_id.to_owned(),
                    });
                }
                Ok(())
            },
        )??;

        Ok(())
    }

    // ---------------------------------------------------------------
    // Phase 3: Event Source Mappings
    // ---------------------------------------------------------------

    /// Create an event source mapping.
    ///
    /// Validates the function exists, generates a UUID, and stores the mapping.
    /// Defaults: `batch_size=10`, `enabled=true`.
    ///
    /// # Errors
    ///
    /// Returns `FunctionNotFound` if the specified function does not exist.
    /// Returns `InvalidParameter` if `event_source_arn` is empty.
    pub fn create_event_source_mapping(
        &self,
        input: &CreateEventSourceMappingInput,
    ) -> Result<EventSourceMappingConfiguration, LambdaServiceError> {
        if input.event_source_arn.is_empty() {
            return Err(LambdaServiceError::InvalidParameter {
                message: "eventSourceArn is required".to_owned(),
            });
        }

        // Resolve function name to ARN; validates the function exists.
        let (name, _) = resolve_function_ref(&input.function_name)?;
        let _record = self.get_record(&name)?;
        let func_arn = function_arn(&self.config.default_region, &self.config.account_id, &name);

        let uuid = uuid::Uuid::new_v4().to_string();
        let enabled = input.enabled.unwrap_or(true);
        let batch_size = input.batch_size.unwrap_or(10);
        let max_batching_window = input.maximum_batching_window_in_seconds.unwrap_or(0);
        let state = if enabled { "Enabled" } else { "Disabled" }.to_owned();
        let now = chrono::Utc::now().timestamp();

        let esm_record = EventSourceMappingRecord {
            uuid: uuid.clone(),
            event_source_arn: input.event_source_arn.clone(),
            function_arn: func_arn,
            enabled,
            batch_size,
            maximum_batching_window_in_seconds: max_batching_window,
            starting_position: input.starting_position.clone(),
            starting_position_timestamp: input.starting_position_timestamp.clone(),
            maximum_record_age_in_seconds: input.maximum_record_age_in_seconds,
            bisect_batch_on_function_error: input.bisect_batch_on_function_error,
            maximum_retry_attempts: input.maximum_retry_attempts,
            parallelization_factor: input.parallelization_factor,
            function_response_types: input.function_response_types.clone().unwrap_or_default(),
            state,
            state_transition_reason: "User action".to_owned(),
            last_modified: now,
            last_processing_result: "No records processed".to_owned(),
        };

        let config = Self::record_to_configuration(&esm_record);
        self.esm_store.create(esm_record);

        info!(uuid = %uuid, function_name = %name, "Created event source mapping");

        Ok(config)
    }

    /// Get an event source mapping by UUID.
    ///
    /// # Errors
    ///
    /// Returns `EventSourceMappingNotFound` if the UUID does not exist.
    pub fn get_event_source_mapping(
        &self,
        uuid: &str,
    ) -> Result<EventSourceMappingConfiguration, LambdaServiceError> {
        let record = self.esm_store.get(uuid).ok_or_else(|| {
            LambdaServiceError::EventSourceMappingNotFound {
                uuid: uuid.to_owned(),
            }
        })?;
        Ok(Self::record_to_configuration(&record))
    }

    /// Update an event source mapping.
    ///
    /// # Errors
    ///
    /// Returns `EventSourceMappingNotFound` if the UUID does not exist.
    /// Returns `FunctionNotFound` if a new function name is provided that does not exist.
    pub fn update_event_source_mapping(
        &self,
        uuid: &str,
        input: &UpdateEventSourceMappingInput,
    ) -> Result<EventSourceMappingConfiguration, LambdaServiceError> {
        // If a new function name is provided, validate it exists and resolve the ARN.
        let new_function_arn = if let Some(ref fn_name) = input.function_name {
            let (name, _) = resolve_function_ref(fn_name)?;
            let _record = self.get_record(&name)?;
            Some(function_arn(
                &self.config.default_region,
                &self.config.account_id,
                &name,
            ))
        } else {
            None
        };

        let now = chrono::Utc::now().timestamp();

        let updated = self.esm_store.update(uuid, |record| {
            if let Some(ref arn) = new_function_arn {
                record.function_arn.clone_from(arn);
            }
            if let Some(enabled) = input.enabled {
                record.enabled = enabled;
                if enabled { "Enabled" } else { "Disabled" }.clone_into(&mut record.state);
            }
            if let Some(batch_size) = input.batch_size {
                record.batch_size = batch_size;
            }
            if let Some(max_window) = input.maximum_batching_window_in_seconds {
                record.maximum_batching_window_in_seconds = max_window;
            }
            if let Some(max_age) = input.maximum_record_age_in_seconds {
                record.maximum_record_age_in_seconds = Some(max_age);
            }
            if let Some(bisect) = input.bisect_batch_on_function_error {
                record.bisect_batch_on_function_error = Some(bisect);
            }
            if let Some(retries) = input.maximum_retry_attempts {
                record.maximum_retry_attempts = Some(retries);
            }
            if let Some(factor) = input.parallelization_factor {
                record.parallelization_factor = Some(factor);
            }
            if let Some(ref types) = input.function_response_types {
                record.function_response_types.clone_from(types);
            }
            record.last_modified = now;
            "User action".clone_into(&mut record.state_transition_reason);
            Self::record_to_configuration(record)
        })?;

        info!(uuid = %uuid, "Updated event source mapping");

        Ok(updated)
    }

    /// Delete an event source mapping.
    ///
    /// Returns the final configuration with state set to `Deleting`.
    ///
    /// # Errors
    ///
    /// Returns `EventSourceMappingNotFound` if the UUID does not exist.
    pub fn delete_event_source_mapping(
        &self,
        uuid: &str,
    ) -> Result<EventSourceMappingConfiguration, LambdaServiceError> {
        let record = self.esm_store.delete(uuid).ok_or_else(|| {
            LambdaServiceError::EventSourceMappingNotFound {
                uuid: uuid.to_owned(),
            }
        })?;

        info!(uuid = %uuid, "Deleted event source mapping");

        let mut config = Self::record_to_configuration(&record);
        config.state = Some("Deleting".to_owned());
        Ok(config)
    }

    /// List event source mappings with optional filters and pagination.
    ///
    /// Supports filtering by `function_name` and `event_source_arn`.
    #[must_use]
    pub fn list_event_source_mappings(
        &self,
        function_name: Option<&str>,
        event_source_arn: Option<&str>,
        marker: Option<&str>,
        max_items: Option<usize>,
    ) -> ListEventSourceMappingsOutput {
        let all = self.esm_store.list(function_name, event_source_arn);
        let max = max_items.unwrap_or(100);

        // Find start index from marker.
        let start = marker
            .and_then(|m| all.iter().position(|r| r.uuid == m))
            .map_or(0, |pos| pos + 1);

        let page: Vec<EventSourceMappingConfiguration> = all
            .iter()
            .skip(start)
            .take(max)
            .map(Self::record_to_configuration)
            .collect();

        let next_marker = if start + max < all.len() {
            all.get(start + max - 1).map(|r| r.uuid.clone())
        } else {
            None
        };

        ListEventSourceMappingsOutput {
            event_source_mappings: Some(page),
            next_marker,
        }
    }

    /// Convert an `EventSourceMappingRecord` to an `EventSourceMappingConfiguration`.
    fn record_to_configuration(
        record: &EventSourceMappingRecord,
    ) -> EventSourceMappingConfiguration {
        #[allow(clippy::cast_precision_loss)]
        let last_modified_f64 = record.last_modified as f64;

        EventSourceMappingConfiguration {
            uuid: Some(record.uuid.clone()),
            event_source_arn: Some(record.event_source_arn.clone()),
            function_arn: Some(record.function_arn.clone()),
            state: Some(record.state.clone()),
            state_transition_reason: Some(record.state_transition_reason.clone()),
            last_modified: Some(last_modified_f64),
            last_processing_result: Some(record.last_processing_result.clone()),
            batch_size: Some(record.batch_size),
            maximum_batching_window_in_seconds: Some(record.maximum_batching_window_in_seconds),
            starting_position: record.starting_position.clone(),
            starting_position_timestamp: record.starting_position_timestamp.clone(),
            maximum_record_age_in_seconds: record.maximum_record_age_in_seconds,
            bisect_batch_on_function_error: record.bisect_batch_on_function_error,
            maximum_retry_attempts: record.maximum_retry_attempts,
            parallelization_factor: record.parallelization_factor,
            function_response_types: if record.function_response_types.is_empty() {
                None
            } else {
                Some(record.function_response_types.clone())
            },
        }
    }

    // ---------------------------------------------------------------
    // Internal helpers
    // ---------------------------------------------------------------

    /// Build a `PublishLayerVersionOutput` from a `LayerVersionRecord`.
    #[allow(clippy::cast_possible_wrap)]
    fn build_layer_version_output(ver: &LayerVersionRecord) -> PublishLayerVersionOutput {
        PublishLayerVersionOutput {
            content: Some(LayerVersionContentOutput {
                code_sha256: Some(ver.code_sha256.clone()),
                code_size: Some(ver.code_size as i64),
                ..Default::default()
            }),
            layer_arn: Some(ver.layer_arn.clone()),
            layer_version_arn: Some(ver.layer_version_arn.clone()),
            description: if ver.description.is_empty() {
                None
            } else {
                Some(ver.description.clone())
            },
            created_date: Some(ver.created_date.clone()),
            version: Some(ver.version as i64),
            compatible_runtimes: if ver.compatible_runtimes.is_empty() {
                None
            } else {
                Some(ver.compatible_runtimes.clone())
            },
            license_info: ver.license_info.clone(),
            compatible_architectures: if ver.compatible_architectures.is_empty() {
                None
            } else {
                Some(ver.compatible_architectures.clone())
            },
        }
    }

    // -----------------------------------------------------------------
    // Phase 6: Concurrency
    // -----------------------------------------------------------------

    /// Put (set) reserved concurrency for a function.
    pub fn put_function_concurrency(
        &self,
        function_ref: &str,
        reserved: i32,
    ) -> Result<Concurrency, LambdaServiceError> {
        let (name, _) = resolve_function_ref(function_ref)?;
        let _ = self.get_record(&name)?;

        self.store.update(&name, |rec| {
            rec.reserved_concurrent_executions = Some(reserved);
        })?;

        Ok(Concurrency {
            reserved_concurrent_executions: Some(reserved),
        })
    }

    /// Get reserved concurrency for a function.
    pub fn get_function_concurrency(
        &self,
        function_ref: &str,
    ) -> Result<Concurrency, LambdaServiceError> {
        let (name, _) = resolve_function_ref(function_ref)?;
        let record = self.get_record(&name)?;
        Ok(Concurrency {
            reserved_concurrent_executions: record.reserved_concurrent_executions,
        })
    }

    /// Delete reserved concurrency for a function.
    pub fn delete_function_concurrency(
        &self,
        function_ref: &str,
    ) -> Result<(), LambdaServiceError> {
        let (name, _) = resolve_function_ref(function_ref)?;
        let _ = self.get_record(&name)?;
        self.store.update(&name, |rec| {
            rec.reserved_concurrent_executions = None;
        })?;
        Ok(())
    }

    // -----------------------------------------------------------------
    // Phase 6: Event Invoke Config
    // -----------------------------------------------------------------

    /// Put (create/replace) an event invoke config for a function qualifier.
    pub fn put_function_event_invoke_config(
        &self,
        function_ref: &str,
        qualifier: Option<&str>,
        input: &EventInvokeConfigInput,
    ) -> Result<FunctionEventInvokeConfig, LambdaServiceError> {
        let (name, ref_qualifier) = resolve_function_ref(function_ref)?;
        let qualifier = qualifier.or(ref_qualifier.as_deref()).unwrap_or("$LATEST");
        let record = self.get_record(&name)?;
        let fn_arn = self.build_qualified_arn(&name, qualifier);
        let now = chrono::Utc::now();
        let now_iso = now.format("%Y-%m-%dT%H:%M:%S%.3f%z").to_string();
        let epoch_millis = millis_to_f64(now.timestamp_millis());

        let config_record = EventInvokeConfigRecord {
            function_arn: fn_arn.clone(),
            qualifier: qualifier.to_owned(),
            maximum_retry_attempts: input.maximum_retry_attempts,
            maximum_event_age_in_seconds: input.maximum_event_age_in_seconds,
            last_modified: now_iso,
            destination_config: input.destination_config.clone(),
        };

        drop(record);
        self.store.update(&name, |rec| {
            rec.event_invoke_configs
                .insert(qualifier.to_owned(), config_record.clone());
        })?;

        Ok(build_event_invoke_config(&config_record, epoch_millis))
    }

    /// Get an event invoke config for a function qualifier.
    pub fn get_function_event_invoke_config(
        &self,
        function_ref: &str,
        qualifier: Option<&str>,
    ) -> Result<FunctionEventInvokeConfig, LambdaServiceError> {
        let (name, ref_qualifier) = resolve_function_ref(function_ref)?;
        let qualifier = qualifier.or(ref_qualifier.as_deref()).unwrap_or("$LATEST");
        let record = self.get_record(&name)?;

        let config_record = record.event_invoke_configs.get(qualifier).ok_or_else(|| {
            LambdaServiceError::EventInvokeConfigNotFound {
                function_name: name.clone(),
                qualifier: qualifier.to_owned(),
            }
        })?;

        let epoch_millis = parse_epoch_millis(&config_record.last_modified);

        Ok(build_event_invoke_config(config_record, epoch_millis))
    }

    /// Update (merge) an event invoke config for a function qualifier.
    pub fn update_function_event_invoke_config(
        &self,
        function_ref: &str,
        qualifier: Option<&str>,
        input: &EventInvokeConfigInput,
    ) -> Result<FunctionEventInvokeConfig, LambdaServiceError> {
        let (name, ref_qualifier) = resolve_function_ref(function_ref)?;
        let qualifier = qualifier.or(ref_qualifier.as_deref()).unwrap_or("$LATEST");
        let _ = self.get_record(&name)?;
        let fn_arn = self.build_qualified_arn(&name, qualifier);
        let now = chrono::Utc::now();
        let now_iso = now.format("%Y-%m-%dT%H:%M:%S%.3f%z").to_string();
        let epoch_millis = millis_to_f64(now.timestamp_millis());

        let result = self.store.update(&name, |rec| {
            let entry = rec
                .event_invoke_configs
                .entry(qualifier.to_owned())
                .or_insert_with(|| EventInvokeConfigRecord {
                    function_arn: fn_arn.clone(),
                    qualifier: qualifier.to_owned(),
                    maximum_retry_attempts: None,
                    maximum_event_age_in_seconds: None,
                    last_modified: now_iso.clone(),
                    destination_config: None,
                });

            if let Some(v) = input.maximum_retry_attempts {
                entry.maximum_retry_attempts = Some(v);
            }
            if let Some(v) = input.maximum_event_age_in_seconds {
                entry.maximum_event_age_in_seconds = Some(v);
            }
            if let Some(ref dc) = input.destination_config {
                entry.destination_config = Some(dc.clone());
            }
            entry.last_modified.clone_from(&now_iso);
            entry.clone()
        })?;

        Ok(build_event_invoke_config(&result, epoch_millis))
    }

    /// Delete an event invoke config for a function qualifier.
    pub fn delete_function_event_invoke_config(
        &self,
        function_ref: &str,
        qualifier: Option<&str>,
    ) -> Result<(), LambdaServiceError> {
        let (name, ref_qualifier) = resolve_function_ref(function_ref)?;
        let qualifier = qualifier.or(ref_qualifier.as_deref()).unwrap_or("$LATEST");
        let _ = self.get_record(&name)?;

        self.store.update(&name, |rec| {
            rec.event_invoke_configs.remove(qualifier);
        })?;
        Ok(())
    }

    /// List all event invoke configs for a function.
    pub fn list_function_event_invoke_configs(
        &self,
        function_ref: &str,
    ) -> Result<Vec<FunctionEventInvokeConfig>, LambdaServiceError> {
        let (name, _) = resolve_function_ref(function_ref)?;
        let record = self.get_record(&name)?;

        let configs: Vec<FunctionEventInvokeConfig> = record
            .event_invoke_configs
            .values()
            .map(|cr| {
                let epoch_millis = parse_epoch_millis(&cr.last_modified);
                build_event_invoke_config(cr, epoch_millis)
            })
            .collect();

        Ok(configs)
    }

    /// Build a qualified ARN for a function + qualifier.
    fn build_qualified_arn(&self, function_name: &str, qualifier: &str) -> String {
        function_version_arn(
            &self.config.default_region,
            &self.config.account_id,
            function_name,
            qualifier,
        )
    }

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

/// Convert an `i64` millis timestamp to `f64` for the API response.
///
/// The AWS Lambda API returns `LastModified` as epoch milliseconds in a float.
/// Precision loss is acceptable since timestamps fit well within f64 mantissa range
/// for any reasonable date (up to year ~285,000).
#[allow(clippy::cast_precision_loss)]
fn millis_to_f64(millis: i64) -> f64 {
    millis as f64
}

/// Parse an ISO 8601 timestamp string into epoch millis as `f64`.
fn parse_epoch_millis(iso: &str) -> f64 {
    chrono::DateTime::parse_from_str(iso, "%Y-%m-%dT%H:%M:%S%.3f%z")
        .map(|dt| millis_to_f64(dt.timestamp_millis()))
        .unwrap_or(0.0)
}

/// Convert an `EventInvokeConfigRecord` to the output type.
fn build_event_invoke_config(
    record: &EventInvokeConfigRecord,
    epoch_millis: f64,
) -> FunctionEventInvokeConfig {
    FunctionEventInvokeConfig {
        function_arn: Some(record.function_arn.clone()),
        maximum_retry_attempts: record.maximum_retry_attempts,
        maximum_event_age_in_seconds: record.maximum_event_age_in_seconds,
        last_modified: Some(epoch_millis),
        destination_config: record.destination_config.clone(),
    }
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

    // ---- Layer operation tests ----

    #[test]
    fn test_should_publish_and_get_layer_version() {
        use base64::Engine;
        let provider = test_provider();
        let zip_data = base64::engine::general_purpose::STANDARD.encode(b"PK\x03\x04layer");
        let input = PublishLayerVersionInput {
            description: Some("Test layer".to_owned()),
            content: Some(ruststack_lambda_model::types::LayerVersionContentInput {
                zip_file: Some(zip_data),
                ..Default::default()
            }),
            compatible_runtimes: Some(vec!["python3.12".to_owned()]),
            ..Default::default()
        };

        let output = provider.publish_layer_version("my-layer", &input).unwrap();
        assert_eq!(output.version, Some(1));
        assert_eq!(output.description, Some("Test layer".to_owned()));
        assert!(
            output
                .layer_arn
                .as_ref()
                .unwrap()
                .contains("layer:my-layer")
        );
        assert!(
            output
                .layer_version_arn
                .as_ref()
                .unwrap()
                .contains("layer:my-layer:1")
        );

        // Get the layer version.
        let get_output = provider.get_layer_version("my-layer", 1).unwrap();
        assert_eq!(get_output.version, Some(1));
        assert_eq!(get_output.description, Some("Test layer".to_owned()));
    }

    #[test]
    fn test_should_publish_multiple_layer_versions() {
        let provider = test_provider();
        let input = PublishLayerVersionInput::default();

        let v1 = provider.publish_layer_version("my-layer", &input).unwrap();
        let v2 = provider.publish_layer_version("my-layer", &input).unwrap();

        assert_eq!(v1.version, Some(1));
        assert_eq!(v2.version, Some(2));
    }

    #[test]
    fn test_should_list_layer_versions() {
        let provider = test_provider();
        let input = PublishLayerVersionInput::default();

        provider.publish_layer_version("my-layer", &input).unwrap();
        provider.publish_layer_version("my-layer", &input).unwrap();
        provider.publish_layer_version("my-layer", &input).unwrap();

        let output = provider
            .list_layer_versions("my-layer", None, None)
            .unwrap();
        assert_eq!(output.layer_versions.as_ref().unwrap().len(), 3);
    }

    #[test]
    fn test_should_list_layers() {
        let provider = test_provider();
        let input = PublishLayerVersionInput::default();

        provider
            .publish_layer_version("alpha-layer", &input)
            .unwrap();
        provider
            .publish_layer_version("bravo-layer", &input)
            .unwrap();

        let output = provider.list_layers(None, None);
        let layers = output.layers.as_ref().unwrap();
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0].layer_name, Some("alpha-layer".to_owned()),);
        assert_eq!(layers[1].layer_name, Some("bravo-layer".to_owned()),);
    }

    #[test]
    fn test_should_delete_layer_version() {
        let provider = test_provider();
        let input = PublishLayerVersionInput::default();

        provider.publish_layer_version("my-layer", &input).unwrap();
        provider.publish_layer_version("my-layer", &input).unwrap();

        provider.delete_layer_version("my-layer", 1).unwrap();

        // Version 1 should be gone.
        let err = provider.get_layer_version("my-layer", 1);
        assert!(err.is_err());

        // Version 2 should still exist.
        let output = provider.get_layer_version("my-layer", 2).unwrap();
        assert_eq!(output.version, Some(2));
    }

    #[test]
    fn test_should_delete_nonexistent_layer_version_silently() {
        let provider = test_provider();
        // Should not error even if layer doesn't exist.
        provider.delete_layer_version("nonexistent", 99).unwrap();
    }

    #[test]
    fn test_should_add_and_get_layer_version_policy() {
        let provider = test_provider();
        let input = PublishLayerVersionInput::default();
        provider.publish_layer_version("my-layer", &input).unwrap();

        let perm_input = AddLayerVersionPermissionInput {
            statement_id: Some("stmt-1".to_owned()),
            action: Some("lambda:GetLayerVersion".to_owned()),
            principal: Some("*".to_owned()),
            ..Default::default()
        };
        let output = provider
            .add_layer_version_permission("my-layer", 1, &perm_input)
            .unwrap();
        assert!(output.statement.is_some());

        let policy = provider.get_layer_version_policy("my-layer", 1).unwrap();
        assert!(policy.policy.as_ref().unwrap().contains("stmt-1"));
    }

    #[test]
    fn test_should_reject_duplicate_layer_permission_sid() {
        let provider = test_provider();
        let input = PublishLayerVersionInput::default();
        provider.publish_layer_version("my-layer", &input).unwrap();

        let perm_input = AddLayerVersionPermissionInput {
            statement_id: Some("stmt-1".to_owned()),
            action: Some("lambda:GetLayerVersion".to_owned()),
            principal: Some("*".to_owned()),
            ..Default::default()
        };
        provider
            .add_layer_version_permission("my-layer", 1, &perm_input)
            .unwrap();

        let err = provider
            .add_layer_version_permission("my-layer", 1, &perm_input)
            .unwrap_err();
        assert!(matches!(err, LambdaServiceError::ResourceConflict { .. }));
    }

    #[test]
    fn test_should_remove_layer_version_permission() {
        let provider = test_provider();
        let input = PublishLayerVersionInput::default();
        provider.publish_layer_version("my-layer", &input).unwrap();

        let perm_input = AddLayerVersionPermissionInput {
            statement_id: Some("stmt-1".to_owned()),
            action: Some("lambda:GetLayerVersion".to_owned()),
            principal: Some("*".to_owned()),
            ..Default::default()
        };
        provider
            .add_layer_version_permission("my-layer", 1, &perm_input)
            .unwrap();

        provider
            .remove_layer_version_permission("my-layer", 1, "stmt-1")
            .unwrap();

        // Policy should now be empty.
        let err = provider.get_layer_version_policy("my-layer", 1);
        assert!(err.is_err());
    }

    #[test]
    fn test_should_get_layer_version_by_arn() {
        let provider = test_provider();
        let input = PublishLayerVersionInput::default();
        provider.publish_layer_version("my-layer", &input).unwrap();

        let arn = "arn:aws:lambda:us-east-1:000000000000:layer:my-layer:1";
        let output = provider.get_layer_version_by_arn(arn).unwrap();
        assert_eq!(output.version, Some(1));
    }

    #[test]
    fn test_should_error_on_empty_layer_name() {
        let provider = test_provider();
        let input = PublishLayerVersionInput::default();
        let err = provider.publish_layer_version("", &input).unwrap_err();
        assert!(matches!(err, LambdaServiceError::InvalidParameter { .. }));
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

    // ---- Event Source Mapping tests ----

    fn sample_esm_input() -> CreateEventSourceMappingInput {
        CreateEventSourceMappingInput {
            event_source_arn: "arn:aws:sqs:us-east-1:000000000000:my-queue".to_owned(),
            function_name: "my-func".to_owned(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_should_create_event_source_mapping() {
        let provider = test_provider();
        create_test_function(&provider, "my-func").await;

        let config = provider
            .create_event_source_mapping(&sample_esm_input())
            .unwrap();

        assert!(config.uuid.is_some());
        assert_eq!(
            config.event_source_arn.as_deref(),
            Some("arn:aws:sqs:us-east-1:000000000000:my-queue")
        );
        assert!(config.function_arn.is_some());
        assert_eq!(config.state.as_deref(), Some("Enabled"));
        assert_eq!(config.batch_size, Some(10));
        assert_eq!(config.maximum_batching_window_in_seconds, Some(0));
    }

    #[tokio::test]
    async fn test_should_create_esm_disabled() {
        let provider = test_provider();
        create_test_function(&provider, "my-func").await;

        let mut input = sample_esm_input();
        input.enabled = Some(false);
        input.batch_size = Some(50);

        let config = provider.create_event_source_mapping(&input).unwrap();
        assert_eq!(config.state.as_deref(), Some("Disabled"));
        assert_eq!(config.batch_size, Some(50));
    }

    #[tokio::test]
    async fn test_should_reject_esm_for_nonexistent_function() {
        let provider = test_provider();
        let err = provider
            .create_event_source_mapping(&sample_esm_input())
            .unwrap_err();
        assert!(matches!(err, LambdaServiceError::FunctionNotFound { .. }));
    }

    #[test]
    fn test_should_reject_esm_with_empty_event_source_arn() {
        let provider = test_provider();
        let input = CreateEventSourceMappingInput {
            event_source_arn: String::new(),
            function_name: "my-func".to_owned(),
            ..Default::default()
        };
        let err = provider.create_event_source_mapping(&input).unwrap_err();
        assert!(matches!(err, LambdaServiceError::InvalidParameter { .. }));
    }

    #[tokio::test]
    async fn test_should_get_event_source_mapping() {
        let provider = test_provider();
        create_test_function(&provider, "my-func").await;

        let created = provider
            .create_event_source_mapping(&sample_esm_input())
            .unwrap();
        let uuid = created.uuid.as_ref().unwrap();

        let retrieved = provider.get_event_source_mapping(uuid).unwrap();
        assert_eq!(retrieved.uuid.as_deref(), Some(uuid.as_str()));
        assert_eq!(retrieved.batch_size, Some(10));
    }

    #[test]
    fn test_should_error_on_get_nonexistent_esm() {
        let provider = test_provider();
        let err = provider
            .get_event_source_mapping("no-such-uuid")
            .unwrap_err();
        assert!(matches!(
            err,
            LambdaServiceError::EventSourceMappingNotFound { .. }
        ));
    }

    #[tokio::test]
    async fn test_should_update_event_source_mapping() {
        let provider = test_provider();
        create_test_function(&provider, "my-func").await;

        let created = provider
            .create_event_source_mapping(&sample_esm_input())
            .unwrap();
        let uuid = created.uuid.as_ref().unwrap();

        let update_input = UpdateEventSourceMappingInput {
            batch_size: Some(100),
            enabled: Some(false),
            maximum_retry_attempts: Some(3),
            ..Default::default()
        };

        let updated = provider
            .update_event_source_mapping(uuid, &update_input)
            .unwrap();
        assert_eq!(updated.batch_size, Some(100));
        assert_eq!(updated.state.as_deref(), Some("Disabled"));
        assert_eq!(updated.maximum_retry_attempts, Some(3));
    }

    #[test]
    fn test_should_error_on_update_nonexistent_esm() {
        let provider = test_provider();
        let input = UpdateEventSourceMappingInput::default();
        let err = provider
            .update_event_source_mapping("no-such-uuid", &input)
            .unwrap_err();
        assert!(matches!(
            err,
            LambdaServiceError::EventSourceMappingNotFound { .. }
        ));
    }

    #[tokio::test]
    async fn test_should_delete_event_source_mapping() {
        let provider = test_provider();
        create_test_function(&provider, "my-func").await;

        let created = provider
            .create_event_source_mapping(&sample_esm_input())
            .unwrap();
        let uuid = created.uuid.as_ref().unwrap();

        let deleted = provider.delete_event_source_mapping(uuid).unwrap();
        assert_eq!(deleted.state.as_deref(), Some("Deleting"));

        // Should no longer be findable.
        let err = provider.get_event_source_mapping(uuid).unwrap_err();
        assert!(matches!(
            err,
            LambdaServiceError::EventSourceMappingNotFound { .. }
        ));
    }

    #[test]
    fn test_should_error_on_delete_nonexistent_esm() {
        let provider = test_provider();
        let err = provider
            .delete_event_source_mapping("no-such-uuid")
            .unwrap_err();
        assert!(matches!(
            err,
            LambdaServiceError::EventSourceMappingNotFound { .. }
        ));
    }

    #[tokio::test]
    async fn test_should_list_event_source_mappings() {
        let provider = test_provider();
        create_test_function(&provider, "my-func").await;

        // Create 3 mappings.
        for _ in 0..3 {
            provider
                .create_event_source_mapping(&sample_esm_input())
                .unwrap();
        }

        let output = provider.list_event_source_mappings(None, None, None, None);
        assert_eq!(output.event_source_mappings.as_ref().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn test_should_list_esm_with_function_filter() {
        let provider = test_provider();
        create_test_function(&provider, "func-a").await;
        create_test_function(&provider, "func-b").await;

        let input_a = CreateEventSourceMappingInput {
            event_source_arn: "arn:aws:sqs:us-east-1:000000000000:queue".to_owned(),
            function_name: "func-a".to_owned(),
            ..Default::default()
        };
        let input_b = CreateEventSourceMappingInput {
            event_source_arn: "arn:aws:sqs:us-east-1:000000000000:queue".to_owned(),
            function_name: "func-b".to_owned(),
            ..Default::default()
        };

        provider.create_event_source_mapping(&input_a).unwrap();
        provider.create_event_source_mapping(&input_b).unwrap();

        let output = provider.list_event_source_mappings(Some("func-a"), None, None, None);
        assert_eq!(output.event_source_mappings.as_ref().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_should_list_esm_with_pagination() {
        let provider = test_provider();
        create_test_function(&provider, "my-func").await;

        for _ in 0..5 {
            provider
                .create_event_source_mapping(&sample_esm_input())
                .unwrap();
        }

        let page1 = provider.list_event_source_mappings(None, None, None, Some(2));
        assert_eq!(page1.event_source_mappings.as_ref().unwrap().len(), 2);
        assert!(page1.next_marker.is_some());

        let page2 =
            provider.list_event_source_mappings(None, None, page1.next_marker.as_deref(), Some(2));
        assert_eq!(page2.event_source_mappings.as_ref().unwrap().len(), 2);
    }

    /// Helper to create a test function for ESM tests.
    async fn create_test_function(provider: &RustStackLambda, name: &str) {
        provider
            .create_function(sample_create_input(name))
            .await
            .unwrap();
    }
}
