//! Validated, immutable YAML and environment configuration shared by services.
//!
//! The binary installs settings once before constructing providers. Library users
//! may use [`ValidatedSettings::parse`] without modifying process globals. Values
//! from the process environment override YAML, and diagnostics never print values.

pub use std::env::VarError;
use std::{collections::BTreeMap, env, fmt, net::SocketAddr, sync::OnceLock, time::Duration};

use config::{Config, File, FileFormat};
use serde::Deserialize;
use serde_json::Value;
use tokio::io::AsyncReadExt;
use url::Url;

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const SERVICE_NAMES: &[&str] = &[
    "S3",
    "DYNAMODB",
    "DYNAMODBSTREAMS",
    "SQS",
    "SSM",
    "SNS",
    "LAMBDA",
    "STEPFUNCTIONS",
    "EVENTS",
    "LOGS",
    "KMS",
    "KINESIS",
    "SECRETSMANAGER",
    "SES",
    "APIGATEWAYV2",
    "CLOUDWATCH",
    "IAM",
    "STS",
    "CLOUDFRONT",
];
const KEYS: &[&str] = &[
    "GATEWAY_LISTEN",
    "GATEWAY_HOST",
    "GATEWAY_PORT",
    "RUSTACK_ADVERTISED_ENDPOINT",
    "SERVICES",
    "DEFAULT_REGION",
    "AWS_DEFAULT_REGION",
    "DEFAULT_ACCOUNT_ID",
    "ACCOUNT_ID",
    "ACCESS_KEY",
    "SECRET_KEY",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "LOG_LEVEL",
    "RUST_LOG",
    "PERSISTENCE",
    "DATA_DIR",
    "S3_VIRTUAL_HOSTING",
    "S3_DOMAIN",
    "S3_MAX_MEMORY_OBJECT_SIZE",
    "LAMBDA_DOCKER_ENABLED",
    "LAMBDA_DOCKER_SOCKET",
    "LAMBDA_DOCKER_RELAY_IMAGE",
    "LAMBDA_DOCKER_NETWORK",
    "LAMBDA_DOCKER_PULL_POLICY",
    "LAMBDA_DOCKER_REGISTRIES",
    "LAMBDA_DOCKER_REGISTRY_AUTH_FILE",
    "LAMBDA_DOCKER_GATEWAY_ENDPOINT",
    "LAMBDA_DOCKER_SDK_ACCESS_KEY_ID",
    "LAMBDA_DOCKER_SDK_SECRET_ACCESS_KEY",
    "LAMBDA_DOCKER_UID",
    "LAMBDA_DOCKER_GID",
    "LAMBDA_DOCKER_CPU",
    "LAMBDA_DOCKER_LOG_RING_BYTES",
    "LAMBDA_DOCKER_CROSS_ARCHITECTURE",
    "LAMBDA_DOCKER_ZIP_IMAGE",
    "LAMBDA_EXECUTOR",
    "LAMBDA_MAX_WARM_INSTANCES",
    "LAMBDA_IDLE_TIMEOUT_SECS",
    "LAMBDA_INIT_TIMEOUT_SECS",
    "LAMBDA_SQUIB_INSTANCE_ID",
    "LAMBDA_SQUIB_CONFIG_FILE",
    "LAMBDA_SQUIB_VSOCK_PATH",
    "LAMBDA_SQUIB_STAGE_PORT",
    "LAMBDA_SQUIB_CONNECT_TIMEOUT_MS",
    "LAMBDA_SQUIB_RESPONSE_LIMIT_BYTES",
    "LAMBDA_SQUIB_RUN_BUDGET_SECS",
    "LAMBDA_SQUIB_SHUTDOWN_TIMEOUT_MS",
    "RUSTACK_WORKSPACE_ROOT",
    "RUSTACK_SNAPSHOT_DIR",
    "RUSTACK_SNAPSHOT_PERF_FILE",
    "EVENTS_HOST",
    "EVENTS_PORT",
    "LOGS_HOST",
    "LOGS_PORT",
    "CLOUDFRONT_ACCOUNT_ID",
    "CLOUDFRONT_DOMAIN_SUFFIX",
    "CLOUDFRONT_DISTRIBUTION_PROPAGATION_MS",
    "CLOUDFRONT_INVALIDATION_PROPAGATION_MS",
    "CLOUDFRONT_DETERMINISTIC_IDS",
    "CLOUDFRONT_FAIL_ON_FUNCTION",
    "CLOUDFRONT_FORWARD_USER_METADATA",
    "CLOUDFRONT_MAX_UPSTREAM_BODY_BYTES",
    "CLOUDFRONT_HTTP_ORIGIN_TIMEOUT_MS",
    "CLOUDFRONT_DIVERGENCE_LOG_INTERVAL_MS",
    "CLOUDWATCH_MAX_RETENTION_SECONDS",
    "CLOUDWATCH_MAX_POINTS_PER_SERIES",
    "DYNAMODBSTREAMS_MAX_RECORDS_PER_SHARD",
    "DYNAMODBSTREAMS_MAX_RECORD_AGE_SECONDS",
    "SES_REQUIRE_VERIFIED_IDENTITY",
    "SES_MAX_24_HOUR_SEND",
    "SES_MAX_SEND_RATE",
];

static SETTINGS: OnceLock<ValidatedSettings> = OnceLock::new();

/// Positive, bounded runtime resource limits. YAML names are camelCase.
#[derive(Debug, Clone, Deserialize, typed_builder::TypedBuilder)]
#[non_exhaustive]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeBudgets {
    /// Maximum busy, idle and quarantined Docker environments.
    pub lambda_docker_environments: usize,
    /// Aggregate Docker environment and relay memory capacity.
    pub lambda_docker_memory_bytes: u64,
    /// Absolute Docker pull queue and progress deadline.
    pub lambda_docker_pull_seconds: u64,
    /// Accepted queued and running Docker deployments.
    pub lambda_docker_deployment_jobs: usize,
    /// Absolute Docker deployment preparation deadline.
    pub lambda_docker_deployment_seconds: u64,
    /// Retained preparing, live and quarantined private image pins.
    pub lambda_docker_image_pins: usize,
    /// Independent Docker management and control IO deadline.
    pub lambda_docker_io_seconds: u64,
    /// Code archive upload IO budget, independently bounded by cold startup.
    pub lambda_docker_code_upload_seconds: u64,
    /// Docker startup and probe observation deadline.
    pub lambda_docker_start_seconds: u64,
    /// Independent Docker cleanup observation deadline.
    pub lambda_docker_cleanup_seconds: u64,

    /// Accepted workflow API requests, including ready replies.
    pub workflow_api_accepted: usize,
    /// Retained workflow API preparation/response CPU workers.
    pub workflow_api_workers: usize,
    /// Incremental workflow pagination capture bytes.
    pub workflow_pagination_bytes: usize,
    /// Live immutable workflow continuation handles.
    pub workflow_pagination_tokens: usize,
    /// Current workflow state machines.
    pub workflow_registry_machines: usize,
    /// Retained workflow definition and tag versions, independently.
    pub workflow_registry_versions: usize,
    /// Retained workflow execution records.
    pub workflow_registry_records: usize,
    /// Accepted workflow registry commands.
    pub workflow_registry_commands: usize,
    /// Workflow execution name claims retained for ninety days.
    pub workflow_name_tombstones: usize,
    /// Terminal workflow record retention in seconds.
    pub workflow_retention_seconds: u64,
    /// Live and unsettled workflow executions.
    pub workflow_open_executions: usize,
    /// Retained workflow runtime CPU workers.
    pub workflow_evaluations: usize,
    /// Concurrent workflow Task dispatches.
    pub workflow_dispatches: usize,
    /// Global retained frame slots, including root and ancestor leases.
    pub workflow_frames: usize,
    /// Retained frame slots per execution.
    pub workflow_frames_per_execution: usize,
    /// Global intermediate indexed result storage bytes.
    pub workflow_result_bytes: usize,
    /// Intermediate indexed result storage bytes per execution.
    pub workflow_result_bytes_per_execution: usize,
    /// Independent workflow execution duration ceiling.
    pub workflow_max_duration_seconds: u64,
    /// Maximum transitions per workflow execution.
    pub workflow_transitions: u32,
    /// Retained history bytes per workflow execution.
    pub workflow_history_bytes: usize,
    /// Conservative workflow snapshot copy and per-part encoded byte ceiling.
    pub workflow_snapshot_bytes: usize,
    /// Global retained workflow history bytes.
    pub workflow_total_history_bytes: usize,
    /// Accepted and quarantined workflow Lambda bridge calls.
    pub workflow_lambda_accepted_calls: usize,
    /// Initial workflow Lambda cleanup observation budget.
    pub workflow_lambda_cleanup_seconds: u64,
    /// Grace interval before cancelling remaining workflows.
    pub workflow_drain_seconds: u64,
    /// Live callback token attempts, including provisional completion.
    pub workflow_callback_tokens: usize,
    /// Retained active and closed callback classification entries.
    pub workflow_callback_tombstones: usize,
    /// Accepted callback commands, including ready acknowledgments.
    pub workflow_callback_commands: usize,
    /// Retained callback mint entropy workers.
    pub workflow_callback_workers: usize,
    /// Maximum live HTTP connections.
    pub connections: usize,
    /// Maximum concurrently active business requests, including response bodies.
    pub requests: usize,
    /// HTTP header deadline in seconds.
    pub header_seconds: u64,
    /// Control-plane operation deadline in seconds.
    pub request_seconds: u64,
    /// Synchronous Lambda execution deadline in seconds (may exceed 900 while an Invoke runs).
    pub lambda_invoke_seconds: u64,
    /// Total graceful shutdown deadline in seconds.
    pub shutdown_seconds: u64,
    /// Absolute process startup budget, including Docker qualification and snapshots.
    pub startup_seconds: u64,
    /// Maximum aggregate control-plane body bytes.
    pub control_body_bytes: u64,
    /// Maximum Lambda ZIP JSON envelope bytes.
    pub lambda_code_body_bytes: u64,
    /// Maximum upstream response bytes.
    pub upstream_body_bytes: u64,
    /// Body inactivity deadline in seconds.
    pub body_idle_seconds: u64,
    /// Control-plane body total deadline in seconds.
    pub body_total_seconds: u64,
    /// S3 streaming body total deadline in seconds.
    pub s3_body_total_seconds: u64,
    /// Maximum S3 object bytes, without aggregate collection.
    pub s3_object_body_bytes: u64,
}

impl Default for RuntimeBudgets {
    fn default() -> Self {
        Self {
            lambda_docker_environments: 32,
            lambda_docker_memory_bytes: 8_589_934_592,
            lambda_docker_pull_seconds: 300,
            lambda_docker_deployment_jobs: 8,
            lambda_docker_deployment_seconds: 300,
            lambda_docker_image_pins: 1024,
            lambda_docker_io_seconds: 5,
            lambda_docker_code_upload_seconds: 60,
            lambda_docker_start_seconds: 15,
            lambda_docker_cleanup_seconds: 10,

            workflow_api_accepted: 32,
            workflow_api_workers: 8,
            workflow_pagination_bytes: 67_108_864,
            workflow_pagination_tokens: 10_000,
            workflow_registry_machines: 256,
            workflow_registry_versions: 1024,
            workflow_registry_records: 10_000,
            workflow_registry_commands: 128,
            workflow_name_tombstones: 100_000,
            workflow_retention_seconds: 86_400,
            workflow_open_executions: 128,
            workflow_evaluations: 8,
            workflow_dispatches: 32,
            workflow_frames: 4096,
            workflow_frames_per_execution: 512,
            workflow_result_bytes: 268_435_456,
            workflow_result_bytes_per_execution: 33_554_432,
            workflow_max_duration_seconds: 3600,
            workflow_transitions: 100_000,
            workflow_history_bytes: 33_554_432,
            workflow_snapshot_bytes: 536_870_912,
            workflow_total_history_bytes: 268_435_456,
            workflow_lambda_accepted_calls: 128,
            workflow_lambda_cleanup_seconds: 10,
            workflow_drain_seconds: 5,
            workflow_callback_tokens: 4096,
            workflow_callback_tombstones: 100_000,
            workflow_callback_commands: 1024,
            workflow_callback_workers: 8,
            connections: 256,
            requests: 128,
            header_seconds: 5,
            request_seconds: 30,
            lambda_invoke_seconds: 930,
            shutdown_seconds: 30,
            startup_seconds: 600,
            control_body_bytes: 16 * 1024 * 1024,
            lambda_code_body_bytes: 96 * 1024 * 1024,
            upstream_body_bytes: 64 * 1024 * 1024,
            body_idle_seconds: 5,
            body_total_seconds: 30,
            s3_body_total_seconds: 3600,
            s3_object_body_bytes: 5 * 1024 * 1024 * 1024,
        }
    }
}

impl RuntimeBudgets {
    /// Validate budgets before allocating semaphore capacity or timers.
    ///
    /// # Errors
    /// Rejects zero, excessive, or inconsistent resource deadlines and sizes.
    pub fn validate(&self) -> Result<(), SettingsError> {
        self.validate_docker()?;
        self.validate_workflow_api()?;
        self.validate_workflow_execution()?;
        self.validate_workflow_frames()?;
        self.validate_workflow_callbacks()?;
        if self.workflow_api_workers > self.workflow_api_accepted {
            return Err(invalid(
                "budgets.workflowApiWorkers",
                "must not exceed accepted requests",
            ));
        }
        if self.workflow_history_bytes > self.workflow_total_history_bytes {
            return Err(invalid(
                "budgets.workflowHistoryBytes",
                "must not exceed total history bytes",
            ));
        }
        for (key, value, max) in [
            ("budgets.connections", self.connections as u64, 65_536),
            ("budgets.requests", self.requests as u64, 65_536),
            ("budgets.headerSeconds", self.header_seconds, 300),
            ("budgets.requestSeconds", self.request_seconds, 3600),
            (
                "budgets.lambdaInvokeSeconds",
                self.lambda_invoke_seconds,
                3600,
            ),
            ("budgets.shutdownSeconds", self.shutdown_seconds, 3600),
            ("budgets.startupSeconds", self.startup_seconds, 3600),
            (
                "budgets.controlBodyBytes",
                self.control_body_bytes,
                16 * 1024 * 1024,
            ),
            (
                "budgets.lambdaCodeBodyBytes",
                self.lambda_code_body_bytes,
                96 * 1024 * 1024,
            ),
            (
                "budgets.upstreamBodyBytes",
                self.upstream_body_bytes,
                64 * 1024 * 1024,
            ),
            ("budgets.bodyIdleSeconds", self.body_idle_seconds, 300),
            ("budgets.bodyTotalSeconds", self.body_total_seconds, 3600),
            (
                "budgets.s3BodyTotalSeconds",
                self.s3_body_total_seconds,
                3600,
            ),
            (
                "budgets.s3ObjectBodyBytes",
                self.s3_object_body_bytes,
                5 * 1024 * 1024 * 1024,
            ),
        ] {
            if value == 0 || value > max {
                return Err(invalid(key, "outside supported nonzero range"));
            }
        }
        if self.body_idle_seconds > self.body_total_seconds
            || self.body_idle_seconds > self.s3_body_total_seconds
        {
            return Err(invalid(
                "budgets.bodyIdleSeconds",
                "idle deadline must not exceed total deadline",
            ));
        }
        Ok(())
    }
    fn validate_docker(&self) -> Result<(), SettingsError> {
        for (key, value, min, max) in [
            (
                "lambdaDockerEnvironments",
                self.lambda_docker_environments as u64,
                1,
                32,
            ),
            (
                "lambdaDockerMemoryBytes",
                self.lambda_docker_memory_bytes,
                268_435_456,
                1_099_511_627_776,
            ),
            (
                "lambdaDockerPullSeconds",
                self.lambda_docker_pull_seconds,
                1,
                600,
            ),
            (
                "lambdaDockerDeploymentJobs",
                self.lambda_docker_deployment_jobs as u64,
                1,
                32,
            ),
            (
                "lambdaDockerDeploymentSeconds",
                self.lambda_docker_deployment_seconds,
                1,
                3600,
            ),
            (
                "lambdaDockerImagePins",
                self.lambda_docker_image_pins as u64,
                1,
                8192,
            ),
            (
                "lambdaDockerIoSeconds",
                self.lambda_docker_io_seconds,
                1,
                60,
            ),
            (
                "lambdaDockerCodeUploadSeconds",
                self.lambda_docker_code_upload_seconds,
                1,
                600,
            ),
            (
                "lambdaDockerStartSeconds",
                self.lambda_docker_start_seconds,
                1,
                60,
            ),
            (
                "lambdaDockerCleanupSeconds",
                self.lambda_docker_cleanup_seconds,
                1,
                60,
            ),
        ] {
            if !(min..=max).contains(&value) {
                return Err(invalid(
                    &format!("budgets.{key}"),
                    "outside supported range",
                ));
            }
        }
        if self.lambda_docker_deployment_jobs > self.lambda_docker_image_pins {
            return Err(invalid(
                "budgets.lambdaDockerDeploymentJobs",
                "must not exceed image pins",
            ));
        }
        Ok(())
    }
    fn validate_workflow_api(&self) -> Result<(), SettingsError> {
        for (key, value, min, max) in [
            (
                "budgets.workflowApiAccepted",
                self.workflow_api_accepted as u64,
                1,
                128,
            ),
            (
                "budgets.workflowApiWorkers",
                self.workflow_api_workers as u64,
                1,
                32,
            ),
            (
                "budgets.workflowPaginationBytes",
                self.workflow_pagination_bytes as u64,
                8_388_608,
                1_073_741_824,
            ),
            (
                "budgets.workflowPaginationTokens",
                self.workflow_pagination_tokens as u64,
                1,
                100_000,
            ),
            (
                "budgets.workflowRegistryMachines",
                self.workflow_registry_machines as u64,
                1,
                4096,
            ),
            (
                "budgets.workflowRegistryVersions",
                self.workflow_registry_versions as u64,
                1,
                4096,
            ),
            (
                "budgets.workflowRegistryRecords",
                self.workflow_registry_records as u64,
                1,
                100_000,
            ),
            (
                "budgets.workflowRegistryCommands",
                self.workflow_registry_commands as u64,
                1,
                1024,
            ),
            (
                "budgets.workflowNameTombstones",
                self.workflow_name_tombstones as u64,
                1,
                1_000_000,
            ),
            (
                "budgets.workflowRetentionSeconds",
                self.workflow_retention_seconds,
                60,
                7_776_000,
            ),
        ] {
            if !(min..=max).contains(&value) {
                return Err(invalid(key, "outside supported workflow range"));
            }
        }
        Ok(())
    }
    fn validate_workflow_frames(&self) -> Result<(), SettingsError> {
        for (key, value, min, max) in [
            ("budgets.workflowFrames", self.workflow_frames, 1, 4096),
            (
                "budgets.workflowFramesPerExecution",
                self.workflow_frames_per_execution,
                1,
                512,
            ),
            (
                "budgets.workflowResultBytes",
                self.workflow_result_bytes,
                4096,
                1024 * 1024 * 1024,
            ),
            (
                "budgets.workflowResultBytesPerExecution",
                self.workflow_result_bytes_per_execution,
                4096,
                128 * 1024 * 1024,
            ),
        ] {
            if !(min..=max).contains(&value) {
                return Err(invalid(key, "outside supported workflow frame range"));
            }
        }
        if self.workflow_frames_per_execution > self.workflow_frames
            || self.workflow_result_bytes_per_execution > self.workflow_result_bytes
        {
            return Err(invalid(
                "budgets.workflowFrames",
                "per-execution frame/result limits must not exceed global limits",
            ));
        }
        Ok(())
    }
    fn validate_workflow_callbacks(&self) -> Result<(), SettingsError> {
        for (key, value, maximum) in [
            (
                "budgets.workflowCallbackTokens",
                self.workflow_callback_tokens,
                65_536,
            ),
            (
                "budgets.workflowCallbackTombstones",
                self.workflow_callback_tombstones,
                1_000_000,
            ),
            (
                "budgets.workflowCallbackCommands",
                self.workflow_callback_commands,
                100_000,
            ),
            (
                "budgets.workflowCallbackWorkers",
                self.workflow_callback_workers,
                128,
            ),
        ] {
            if !(1..=maximum).contains(&value) {
                return Err(invalid(key, "outside supported callback range"));
            }
        }
        if self.workflow_callback_tombstones < self.workflow_callback_tokens {
            return Err(invalid(
                "budgets.workflowCallbackTombstones",
                "must include live token reservations",
            ));
        }
        Ok(())
    }
    fn validate_workflow_execution(&self) -> Result<(), SettingsError> {
        for (key, value, min, max) in [
            (
                "budgets.workflowOpenExecutions",
                self.workflow_open_executions as u64,
                1,
                4096,
            ),
            (
                "budgets.workflowEvaluations",
                self.workflow_evaluations as u64,
                1,
                128,
            ),
            (
                "budgets.workflowDispatches",
                self.workflow_dispatches as u64,
                1,
                1024,
            ),
            (
                "budgets.workflowMaxDurationSeconds",
                self.workflow_max_duration_seconds,
                1,
                31_536_000,
            ),
            (
                "budgets.workflowTransitions",
                u64::from(self.workflow_transitions),
                1,
                1_000_000,
            ),
            (
                "budgets.workflowHistoryBytes",
                self.workflow_history_bytes as u64,
                1_048_576,
                268_435_456,
            ),
            (
                "budgets.workflowSnapshotBytes",
                self.workflow_snapshot_bytes as u64,
                1_048_576,
                1_073_741_824,
            ),
            (
                "budgets.workflowTotalHistoryBytes",
                self.workflow_total_history_bytes as u64,
                8_388_608,
                4_294_967_296,
            ),
            (
                "budgets.workflowLambdaAcceptedCalls",
                self.workflow_lambda_accepted_calls as u64,
                1,
                1024,
            ),
            (
                "budgets.workflowLambdaCleanupSeconds",
                self.workflow_lambda_cleanup_seconds,
                1,
                60,
            ),
            (
                "budgets.workflowDrainSeconds",
                self.workflow_drain_seconds,
                0,
                60,
            ),
        ] {
            if !(min..=max).contains(&value) {
                return Err(invalid(key, "outside supported workflow range"));
            }
        }
        Ok(())
    }
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct Document {
    environment: BTreeMap<String, Value>,
    budgets: RuntimeBudgets,
    advertised_endpoint: Option<String>,
}

/// Configuration failure without secret-bearing value diagnostics.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    /// Invalid or unknown operator setting.
    #[error("invalid configuration key {key}: {reason}")]
    Invalid {
        /// Name of the invalid key, never its value.
        key: String,
        /// Non-sensitive explanation.
        reason: &'static str,
    },
    /// YAML cannot be parsed according to the configuration schema.
    #[error("invalid YAML configuration schema (values redacted)")]
    Document,
    /// Configuration source IO failed.
    #[error("cannot read configuration source")]
    Io(#[source] std::io::Error),
    /// Config loading exceeded its deadline.
    #[error("configuration read deadline exceeded")]
    Timeout,
    /// Reinitializing the process configuration is not supported.
    #[error("runtime configuration was already installed")]
    AlreadyInstalled,
}

fn invalid(key: &str, reason: &'static str) -> SettingsError {
    SettingsError::Invalid {
        key: key.to_owned(),
        reason,
    }
}

/// Fully validated settings. Debug prints keys, not potentially secret values.
#[derive(Clone)]
pub struct ValidatedSettings {
    values: BTreeMap<String, String>,
    budgets: RuntimeBudgets,
    listen: SocketAddr,
    advertised_endpoint: String,
}

impl fmt::Debug for ValidatedSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ValidatedSettings")
            .field("keys", &self.values.keys().collect::<Vec<_>>())
            .field("budgets", &self.budgets)
            .field("listen", &self.listen)
            .finish_non_exhaustive()
    }
}

impl ValidatedSettings {
    /// Parse YAML and explicit environment overrides, without touching globals.
    ///
    /// # Errors
    /// Rejects unknown YAML keys, invalid scalars, unsafe ranges, and invalid addresses.
    pub fn parse(yaml: &str, overrides: &BTreeMap<String, String>) -> Result<Self, SettingsError> {
        let document: Document = if yaml.trim().is_empty() {
            Document::default()
        } else {
            Config::builder()
                .add_source(File::from_str(yaml, FileFormat::Yaml))
                .build()
                .map_err(|_| SettingsError::Document)?
                .try_deserialize()
                .map_err(|_| SettingsError::Document)?
        };
        document.budgets.validate()?;
        let mut values = BTreeMap::new();
        for (key, value) in document.environment {
            if !known_key(&key) {
                return Err(invalid(&key, "unknown setting"));
            }
            let text = match value {
                Value::String(text) => text,
                Value::Bool(value) => value.to_string(),
                Value::Number(value) => value.to_string(),
                _ => return Err(invalid(&key, "expected a scalar value")),
            };
            values.insert(key, text);
        }
        for (key, value) in overrides {
            if known_key(key) {
                values.insert(key.clone(), value.clone());
            }
        }
        for (key, value) in &mut values {
            validate_value(key, value)?;
        }
        normalize_aliases(
            &mut values,
            &["DEFAULT_REGION", "AWS_DEFAULT_REGION"],
            Some("us-east-1"),
        )?;
        normalize_aliases(
            &mut values,
            &["DEFAULT_ACCOUNT_ID", "ACCOUNT_ID", "CLOUDFRONT_ACCOUNT_ID"],
            Some("000000000000"),
        )?;
        normalize_aliases(&mut values, &["ACCESS_KEY", "AWS_ACCESS_KEY_ID"], None)?;
        normalize_aliases(&mut values, &["SECRET_KEY", "AWS_SECRET_ACCESS_KEY"], None)?;
        let listen: SocketAddr = values
            .get("GATEWAY_LISTEN")
            .map_or("127.0.0.1:4566", String::as_str)
            .parse()
            .map_err(|_| invalid("GATEWAY_LISTEN", "expected IP address and nonzero port"))?;
        if listen.port() == 0 {
            return Err(invalid("GATEWAY_LISTEN", "port must not be zero"));
        }
        values
            .entry("GATEWAY_LISTEN".to_owned())
            .or_insert_with(|| listen.to_string());
        let default_host = if listen.ip().is_unspecified() {
            "localhost".to_owned()
        } else {
            listen.ip().to_string()
        };
        let host = values
            .entry("GATEWAY_HOST".to_owned())
            .or_insert(default_host)
            .clone();
        let port = values
            .entry("GATEWAY_PORT".to_owned())
            .or_insert_with(|| listen.port().to_string())
            .clone();
        let host = if host.contains(':') && !host.starts_with('[') {
            format!("[{host}]")
        } else {
            host
        };
        values.insert("GATEWAY_HOST".to_owned(), host.clone());
        let advertised_endpoint = values
            .get("RUSTACK_ADVERTISED_ENDPOINT")
            .cloned()
            .or(document.advertised_endpoint)
            .unwrap_or_else(|| format!("http://{host}:{port}"));
        let endpoint = validate_endpoint(&advertised_endpoint)?;
        let endpoint_host = endpoint
            .host_str()
            .ok_or_else(|| invalid("advertisedEndpoint", "missing host"))?;
        let endpoint_port = endpoint
            .port_or_known_default()
            .ok_or_else(|| invalid("advertisedEndpoint", "missing port"))?;
        values.insert("GATEWAY_HOST".to_owned(), endpoint_host.to_owned());
        values.insert("GATEWAY_PORT".to_owned(), endpoint_port.to_string());
        let advertised_endpoint = endpoint.origin().ascii_serialization();
        Ok(Self {
            values,
            budgets: document.budgets,
            listen,
            advertised_endpoint,
        })
    }

    /// Effective immutable resource budgets.
    #[must_use]
    pub fn budgets(&self) -> &RuntimeBudgets {
        &self.budgets
    }

    /// Effective bind address, including the validated nonzero port.
    #[must_use]
    pub const fn listen(&self) -> SocketAddr {
        self.listen
    }

    /// Explicit local public endpoint, never derived from an incoming Host header.
    #[must_use]
    pub fn advertised_endpoint(&self) -> &str {
        &self.advertised_endpoint
    }

    /// Lookup a validated value for provider construction.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    /// Reject strict authentication without a complete static credential pair.
    ///
    /// # Errors
    /// Returns a redacted error if any enabled strict service lacks credentials.
    pub fn validate_credentials(&self, enabled: &[String]) -> Result<(), SettingsError> {
        let strict = enabled.iter().any(|service| {
            self.get(&format!(
                "{}_SKIP_SIGNATURE_VALIDATION",
                service.to_ascii_uppercase()
            )) == Some("false")
        });
        if strict {
            let access = self
                .get("ACCESS_KEY")
                .or_else(|| self.get("AWS_ACCESS_KEY_ID"));
            let secret = self
                .get("SECRET_KEY")
                .or_else(|| self.get("AWS_SECRET_ACCESS_KEY"));
            if access.is_none_or(str::is_empty) || secret.is_none_or(str::is_empty) {
                return Err(invalid(
                    "credentials",
                    "strict signature validation requires both access and secret keys",
                ));
            }
        }
        Ok(())
    }
}

fn known_key(key: &str) -> bool {
    KEYS.contains(&key)
        || key
            .strip_suffix("_SKIP_SIGNATURE_VALIDATION")
            .is_some_and(|prefix| SERVICE_NAMES.contains(&prefix))
}

fn validate_value(key: &str, value: &mut String) -> Result<(), SettingsError> {
    if value.is_empty() && key != "SERVICES" {
        return Err(invalid(key, "must not be empty"));
    }
    let maximum = if key == "LAMBDA_DOCKER_REGISTRIES" {
        8127
    } else {
        4096
    };
    if value.len() > maximum || value.chars().any(char::is_control) {
        return Err(invalid(
            key,
            "exceeds byte limit or contains control characters",
        ));
    }
    let boolean = key.ends_with("_SKIP_SIGNATURE_VALIDATION")
        || matches!(
            key,
            "PERSISTENCE"
                | "S3_VIRTUAL_HOSTING"
                | "LAMBDA_DOCKER_ENABLED"
                | "LAMBDA_DOCKER_CROSS_ARCHITECTURE"
                | "CLOUDFRONT_DETERMINISTIC_IDS"
                | "CLOUDFRONT_FAIL_ON_FUNCTION"
                | "CLOUDFRONT_FORWARD_USER_METADATA"
                | "SES_REQUIRE_VERIFIED_IDENTITY"
        );
    if boolean {
        *value = match value.to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "1" => "true".to_owned(),
            "false" | "no" | "off" | "0" => "false".to_owned(),
            _ => return Err(invalid(key, "expected boolean")),
        };
    } else if key == "LAMBDA_EXECUTOR" {
        if !matches!(
            value.as_str(),
            "disabled" | "native" | "auto" | "docker" | "squib"
        ) {
            return Err(invalid(key, "unknown executor"));
        }
    } else if key.ends_with("ACCOUNT_ID") || key == "ACCOUNT_ID" {
        if value.len() != 12 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(invalid(key, "expected 12 ASCII digits"));
        }
    } else if key.ends_with("REGION") {
        if value.len() > 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(invalid(key, "invalid region identifier"));
        }
    } else if key.ends_with("_PORT") {
        let port = value
            .parse::<u16>()
            .map_err(|_| invalid(key, "expected valid port"))?;
        if port == 0 {
            return Err(invalid(key, "port must not be zero"));
        }
    } else if key == "SES_MAX_24_HOUR_SEND" || key == "SES_MAX_SEND_RATE" {
        let number = value
            .parse::<f64>()
            .map_err(|_| invalid(key, "expected finite positive quota"))?;
        if !number.is_finite() || number <= 0.0 || number > 1_000_000_000.0 {
            return Err(invalid(key, "outside supported quota range"));
        }
    } else if key.ends_with("_MS")
        || key.ends_with("_SECS")
        || key.ends_with("_SECONDS")
        || key.contains("_MAX_")
        || key.ends_with("_LIMIT_BYTES")
    {
        let number = value
            .parse::<u64>()
            .map_err(|_| invalid(key, "expected bounded nonnegative integer"))?;
        let zero_allowed = key.contains("PROPAGATION") || key == "S3_MAX_MEMORY_OBJECT_SIZE";
        let max = if key.ends_with("_MS") {
            86_400_000
        } else if key.ends_with("_SECS") || key.ends_with("_SECONDS") {
            31_536_000
        } else {
            5 * 1024 * 1024 * 1024
        };
        if (!zero_allowed && number == 0) || number > max {
            return Err(invalid(key, "outside supported range"));
        }
    } else if key.ends_with("_HOST") || key.ends_with("_DOMAIN") || key.ends_with("_DOMAIN_SUFFIX")
    {
        if value.len() > 253
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b".-:[]".contains(&byte))
        {
            return Err(invalid(key, "invalid hostname or address"));
        }
    }
    Ok(())
}

fn normalize_aliases(
    values: &mut BTreeMap<String, String>,
    keys: &[&str],
    default: Option<&str>,
) -> Result<(), SettingsError> {
    let mut selected = None;
    for key in keys {
        if let Some(value) = values.get(*key) {
            if selected.as_ref().is_some_and(|current| current != value) {
                return Err(invalid(key, "conflicting aliases"));
            }
            selected = Some(value.clone());
        }
    }
    if let Some(selected) = selected.or_else(|| default.map(str::to_owned)) {
        for key in keys {
            values.insert((*key).to_owned(), selected.clone());
        }
    }
    Ok(())
}

fn validate_endpoint(endpoint: &str) -> Result<Url, SettingsError> {
    if endpoint.len() > 300
        || endpoint
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b' ')
    {
        return Err(invalid(
            "advertisedEndpoint",
            "invalid URL length or characters",
        ));
    }
    let parsed = Url::parse(endpoint).map_err(|_| invalid("advertisedEndpoint", "invalid URL"))?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || parsed.port() == Some(0)
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(invalid(
            "advertisedEndpoint",
            "expected http(s) authority without credentials, path, query or fragment",
        ));
    }
    Ok(parsed)
}

/// Read bounded YAML and freeze process configuration before providers start.
///
/// # Errors
/// Returns configuration, IO, timeout, or duplicate-install errors.
pub async fn initialize() -> Result<&'static ValidatedSettings, SettingsError> {
    let yaml = match env::var("RUSTACK_CONFIG") {
        Ok(path) => {
            let read = async {
                let file = tokio::fs::File::open(path)
                    .await
                    .map_err(SettingsError::Io)?;
                let mut text = String::new();
                file.take(MAX_CONFIG_BYTES + 1)
                    .read_to_string(&mut text)
                    .await
                    .map_err(SettingsError::Io)?;
                if text.len() as u64 > MAX_CONFIG_BYTES {
                    return Err(invalid("RUSTACK_CONFIG", "configuration exceeds 1 MiB"));
                }
                Ok(text)
            };
            tokio::time::timeout(Duration::from_secs(5), read)
                .await
                .map_err(|_| SettingsError::Timeout)??
        }
        Err(VarError::NotPresent) => String::new(),
        Err(_) => return Err(invalid("RUSTACK_CONFIG", "expected UTF-8 path")),
    };
    let mut overrides = BTreeMap::new();
    for key in KEYS.iter().map(|key| (*key).to_owned()).chain(
        SERVICE_NAMES
            .iter()
            .map(|key| format!("{key}_SKIP_SIGNATURE_VALIDATION")),
    ) {
        match env::var(&key) {
            Ok(value) => {
                overrides.insert(key, value);
            }
            Err(VarError::NotPresent) => {}
            Err(_) => return Err(invalid(&key, "expected UTF-8 value")),
        }
    }
    let settings = ValidatedSettings::parse(&yaml, &overrides)?;
    SETTINGS
        .set(settings)
        .map_err(|_| SettingsError::AlreadyInstalled)?;
    SETTINGS.get().ok_or(SettingsError::AlreadyInstalled)
}

/// Get an effective value; library use without installation reads the environment.
///
/// # Errors
/// Matches `std::env::var` for absent or non-Unicode values.
pub fn var(key: &str) -> Result<String, VarError> {
    match SETTINGS.get() {
        Some(settings) => settings
            .get(key)
            .map(str::to_owned)
            .ok_or(VarError::NotPresent),
        None => env::var(key),
    }
}

/// Effective budgets, or defaults for independently constructed library services.
#[must_use]
pub fn budgets() -> RuntimeBudgets {
    SETTINGS
        .get()
        .map_or_else(RuntimeBudgets::default, |settings| settings.budgets.clone())
}

/// Installed public endpoint, when configured by the application.
#[must_use]
pub fn advertised_endpoint() -> Option<&'static str> {
    SETTINGS.get().map(ValidatedSettings::advertised_endpoint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_validate_workflow_budgets_and_strict_credentials() -> Result<(), SettingsError> {
        let config = ValidatedSettings::parse(
            "environment:\n  STEPFUNCTIONS_SKIP_SIGNATURE_VALIDATION: false\nbudgets:\n  \
             workflowDrainSeconds: 0\n  workflowApiAccepted: 2\n  workflowApiWorkers: 1",
            &BTreeMap::new(),
        )?;
        assert!(
            config
                .validate_credentials(&["stepfunctions".to_owned()])
                .is_err()
        );
        assert_eq!(config.budgets().workflow_drain_seconds, 0);
        for setting in [
            "workflowApiAccepted: 0",
            "workflowApiWorkers: 33",
            "workflowPaginationBytes: 8388607",
            "workflowPaginationTokens: 100001",
            "workflowRegistryMachines: 4097",
            "workflowRegistryVersions: 0",
            "workflowRegistryRecords: 100001",
            "workflowRegistryCommands: 1025",
            "workflowNameTombstones: 0",
            "workflowRetentionSeconds: 59",
            "workflowOpenExecutions: 0",
            "workflowEvaluations: 129",
            "workflowDispatches: 1025",
            "workflowFrames: 0",
            "workflowFrames: 4097",
            "workflowFramesPerExecution: 513",
            "workflowResultBytes: 4095",
            "workflowResultBytes: 1073741825",
            "workflowResultBytesPerExecution: 134217729",
            "workflowFrames: 1\n  workflowFramesPerExecution: 2",
            "workflowResultBytes: 4096\n  workflowResultBytesPerExecution: 4097",
            "workflowMaxDurationSeconds: 0",
            "workflowTransitions: 1000001",
            "workflowHistoryBytes: 1048575",
            "workflowSnapshotBytes: 1048575",
            "workflowSnapshotBytes: 1073741825",
            "workflowTotalHistoryBytes: 4294967297",
            "workflowLambdaAcceptedCalls: 0",
            "workflowLambdaCleanupSeconds: 61",
            "workflowDrainSeconds: 61",
            "workflowCallbackTokens: 0",
            "workflowCallbackTokens: 65537",
            "workflowCallbackTombstones: 1000001",
            "workflowCallbackTombstones: 1\n  workflowCallbackTokens: 2",
            "workflowCallbackCommands: 0",
            "workflowCallbackCommands: 100001",
            "workflowCallbackWorkers: 0",
            "workflowCallbackWorkers: 129",
            "workflowApiAccepted: 1\n  workflowApiWorkers: 2",
            "workflowHistoryBytes: 33554432\n  workflowTotalHistoryBytes: 8388608",
        ] {
            assert!(
                ValidatedSettings::parse(&format!("budgets:\n  {setting}"), &BTreeMap::new())
                    .is_err(),
                "{setting}"
            );
        }
        Ok(())
    }

    #[test]
    fn test_should_enforce_exact_frame_and_result_configuration_bounds() -> Result<(), SettingsError>
    {
        for (frames, local_frames, results, local_results) in [
            (1, 1, 4096, 4096),
            (4096, 512, 1024 * 1024 * 1024, 128 * 1024 * 1024),
        ] {
            let config = ValidatedSettings::parse(
                &format!(
                    "budgets:\n  workflowFrames: {frames}\n  workflowFramesPerExecution: \
                     {local_frames}\n  workflowResultBytes: {results}\n  \
                     workflowResultBytesPerExecution: {local_results}\n"
                ),
                &BTreeMap::new(),
            )?;
            assert_eq!(config.budgets().workflow_frames, frames);
            assert_eq!(config.budgets().workflow_frames_per_execution, local_frames);
            assert_eq!(config.budgets().workflow_result_bytes, results);
            assert_eq!(
                config.budgets().workflow_result_bytes_per_execution,
                local_results
            );
        }
        Ok(())
    }

    #[test]
    fn test_should_enforce_exact_callback_configuration_bounds_and_defaults()
    -> Result<(), SettingsError> {
        let defaults = RuntimeBudgets::default();
        assert_eq!(
            (
                defaults.workflow_callback_tokens,
                defaults.workflow_callback_tombstones,
                defaults.workflow_callback_commands,
                defaults.workflow_callback_workers
            ),
            (4096, 100_000, 1024, 8)
        );
        for (tokens, retained, commands, workers, seconds) in [
            (1, 1, 1, 1, 60),
            (65_536, 1_000_000, 100_000, 128, 7_776_000),
        ] {
            let config = ValidatedSettings::parse(
                &format!(
                    "budgets:\n  workflowCallbackTokens: {tokens}\n  workflowCallbackTombstones: \
                     {retained}\n  workflowCallbackCommands: {commands}\n  \
                     workflowCallbackWorkers: {workers}\n  workflowRetentionSeconds: {seconds}\n"
                ),
                &BTreeMap::new(),
            )?;
            let budgets = config.budgets();
            assert_eq!(
                (
                    budgets.workflow_callback_tokens,
                    budgets.workflow_callback_tombstones,
                    budgets.workflow_callback_commands,
                    budgets.workflow_callback_workers,
                    budgets.workflow_retention_seconds
                ),
                (tokens, retained, commands, workers, seconds)
            );
        }
        Ok(())
    }

    #[test]
    fn test_should_apply_environment_over_yaml_and_derive_ipv6_endpoint()
    -> Result<(), SettingsError> {
        let overrides = BTreeMap::from([("GATEWAY_LISTEN".to_owned(), "[::1]:4567".to_owned())]);
        let config = ValidatedSettings::parse(
            "environment:\n  GATEWAY_LISTEN: 127.0.0.1:4566\n",
            &overrides,
        )?;
        assert_eq!(config.get("GATEWAY_PORT"), Some("4567"));
        assert_eq!(config.advertised_endpoint(), "http://[::1]:4567");
        Ok(())
    }

    #[test]
    fn test_should_reject_unknown_keys_bad_values_and_zero_budgets() {
        for yaml in [
            "environment:\n  LAMBDA_EXECUTOR: disable",
            "environment:\n  GATEWAY_PORT: 0",
            "environment:\n  S3_SKIP_SIGNATURE_VALIDATION: maybe",
            "environment:\n  DYNAMDB_SKIP_SIGNATURE_VALIDATION: true",
            "budgets:\n  requests: 0",
            "unknown: true",
        ] {
            assert!(ValidatedSettings::parse(yaml, &BTreeMap::new()).is_err());
        }
    }

    #[test]
    fn test_should_fail_closed_without_credentials_and_redact_debug() -> Result<(), SettingsError> {
        let config = ValidatedSettings::parse(
            "environment:\n  SQS_SKIP_SIGNATURE_VALIDATION: false\n  ACCESS_KEY: sample\n  \
             SECRET_KEY: sensitive-value",
            &BTreeMap::new(),
        )?;
        config.validate_credentials(&["sqs".to_owned()])?;
        assert!(!format!("{config:?}").contains("sensitive-value"));
        let missing = ValidatedSettings::parse(
            "environment:\n  SQS_SKIP_SIGNATURE_VALIDATION: false",
            &BTreeMap::new(),
        )?;
        assert!(missing.validate_credentials(&["sqs".to_owned()]).is_err());
        Ok(())
    }
}

#[cfg(test)]
mod docker_tests {
    use std::collections::BTreeMap;

    use super::{RuntimeBudgets, ValidatedSettings};
    #[test]
    fn test_should_accept_docker_yaml_keys_budgets_and_process_precedence() {
        let yaml = "environment:\n  LAMBDA_DOCKER_RELAY_IMAGE: local:v1\n  LAMBDA_DOCKER_NETWORK: \
                    none\n  LAMBDA_DOCKER_PULL_POLICY: never\n  LAMBDA_DOCKER_CROSS_ARCHITECTURE: \
                    yes\nbudgets:\n  lambdaDockerPullSeconds: 1\n  lambdaDockerImagePins: 32\n  \
                    lambdaDockerDeploymentJobs: 32\n";
        let settings = ValidatedSettings::parse(
            yaml,
            &BTreeMap::from([("LAMBDA_DOCKER_RELAY_IMAGE".into(), "override:v2".into())]),
        )
        .unwrap();
        assert_eq!(
            settings.get("LAMBDA_DOCKER_RELAY_IMAGE"),
            Some("override:v2")
        );
        assert_eq!(
            settings.get("LAMBDA_DOCKER_CROSS_ARCHITECTURE"),
            Some("true")
        );
        assert_eq!(settings.budgets().lambda_docker_pull_seconds, 1);
        assert_eq!(RuntimeBudgets::default().lambda_docker_pull_seconds, 300);
    }
    #[test]
    fn test_should_reject_every_docker_budget_outside_its_supported_range() {
        for (key, min, max) in [
            ("lambdaDockerEnvironments", 1_u64, 32_u64),
            ("lambdaDockerMemoryBytes", 268_435_456, 1_099_511_627_776),
            ("lambdaDockerPullSeconds", 1, 600),
            ("lambdaDockerDeploymentJobs", 1, 32),
            ("lambdaDockerDeploymentSeconds", 1, 3600),
            ("lambdaDockerImagePins", 1, 8192),
            ("lambdaDockerIoSeconds", 1, 60),
            ("lambdaDockerCodeUploadSeconds", 1, 600),
            ("lambdaDockerStartSeconds", 1, 60),
            ("lambdaDockerCleanupSeconds", 1, 60),
        ] {
            for value in [min - 1, max + 1] {
                let raw = format!("budgets:\n  {key}: {value}\n");
                assert!(
                    ValidatedSettings::parse(&raw, &BTreeMap::new()).is_err(),
                    "{key}"
                );
            }
            for value in [min, max] {
                let raw = format!("budgets:\n  lambdaDockerDeploymentJobs: 1\n  {key}: {value}\n");
                // Avoid duplicate keys when the row itself is deployment jobs.
                let raw = if key == "lambdaDockerDeploymentJobs" {
                    format!("budgets:\n  {key}: {value}\n")
                } else {
                    raw
                };
                assert!(
                    ValidatedSettings::parse(&raw, &BTreeMap::new()).is_ok(),
                    "{key}"
                );
            }
        }
        assert!(
            ValidatedSettings::parse(
                "budgets:\n  lambdaDockerImagePins: 1\n  lambdaDockerDeploymentJobs: 2",
                &BTreeMap::new()
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod startup_tests {
    use std::collections::BTreeMap;

    use super::{RuntimeBudgets, ValidatedSettings};
    #[test]
    fn test_should_bound_one_process_startup_budget_independently_of_shutdown() {
        assert_eq!(RuntimeBudgets::default().startup_seconds, 600);
        for seconds in [0, 3601] {
            assert!(
                ValidatedSettings::parse(
                    &format!("budgets:\n  startupSeconds: {seconds}\n"),
                    &BTreeMap::new()
                )
                .is_err()
            );
        }
        for seconds in [1, 3600] {
            let settings = ValidatedSettings::parse(
                &format!("budgets:\n  startupSeconds: {seconds}\n  shutdownSeconds: 10\n"),
                &BTreeMap::new(),
            )
            .unwrap();
            assert_eq!(settings.budgets().startup_seconds, seconds);
            assert_eq!(settings.budgets().shutdown_seconds, 10);
        }
    }
}
