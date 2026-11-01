//! Squib microVM execution backend.
//!
//! This backend owns the host-side integration with Squib. It starts one
//! long-lived microVM from a static Squib config and uses Squib's vsock muxer
//! to send staged Lambda invocations to a guest agent.

use std::{
    collections::HashMap,
    env, fmt,
    future::Future,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use tokio::{
    runtime::Handle,
    sync::{Mutex, watch},
    time::Instant,
};
use uuid::Uuid;

use super::{Executor, ExecutorError, InvokeRequest, InvokeResponse};
use crate::{
    artifact::{Architecture, DeploymentArtifact},
    invocation::{CleanupTask, InvocationContext},
};

const DEFAULT_INSTANCE_ID: &str = "rustack_lambda";
const DEFAULT_STAGE_PORT: u32 = 5003;
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_ATTEMPT_TIMEOUT: Duration = Duration::from_millis(500);
const CONNECT_RETRY_INTERVAL: Duration = Duration::from_millis(100);
const LAMBDA_SYNC_RESPONSE_PAYLOAD_LIMIT_BYTES: usize = 6 * 1024 * 1024;
const RESPONSE_PROTOCOL_OVERHEAD_BYTES: usize = 64 * 1024;
const DEFAULT_RESPONSE_LIMIT_BYTES: usize =
    base64_encoded_len(LAMBDA_SYNC_RESPONSE_PAYLOAD_LIMIT_BYTES) + RESPONSE_PROTOCOL_OVERHEAD_BYTES;
const DEFAULT_RUN_BUDGET: Duration = Duration::from_hours(24);
const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_RESPONSE_LIMIT_BYTES: usize = 128;
const PROTOCOL_VERSION: &str = "rustack.squib.lambda.v1";

/// Configuration for [`SquibExecutor`].
#[derive(Debug, Clone)]
pub struct SquibExecutorConfig {
    /// Squib static config file.
    pub config_file: Option<PathBuf>,
    /// Base UDS path from the Squib `vsock.uds_path` config.
    pub vsock_path: Option<PathBuf>,
    /// Squib instance id.
    pub instance_id: String,
    /// Guest stage-and-invoke vsock port.
    pub stage_port: u32,
    /// Maximum time spent connecting to the Squib muxer.
    pub connect_timeout: Duration,
    /// Maximum response line size accepted from the guest agent.
    pub response_limit_bytes: usize,
    /// Maximum wall-clock runtime budget for the Squib microVM.
    pub run_budget: Duration,
    /// Maximum time to wait for Squib shutdown.
    pub shutdown_timeout: Duration,
}

impl SquibExecutorConfig {
    /// Read Squib executor configuration from environment variables.
    ///
    /// # Errors
    ///
    /// Rejects invalid paths, instance IDs, ports, timeouts, response limits or run budgets.
    pub fn from_env() -> Result<Self, ExecutorError> {
        Self::from_env_reader(|key| match rustack_core::settings::var(key) {
            Ok(value) => Some(value),
            Err(env::VarError::NotPresent) => None,
            Err(_) => Some(String::new()),
        })
    }

    pub(crate) fn from_env_reader(
        mut read: impl FnMut(&str) -> Option<String>,
    ) -> Result<Self, ExecutorError> {
        let mut config = Self::default();
        for (key, target) in [
            ("LAMBDA_SQUIB_CONFIG_FILE", &mut config.config_file),
            ("LAMBDA_SQUIB_VSOCK_PATH", &mut config.vsock_path),
        ] {
            if let Some(value) = read(key) {
                if value.is_empty() || value.len() > 4096 || value.contains('\0') {
                    return Err(ExecutorError::Unsupported(format!("Invalid {key}")));
                }
                *target = Some(PathBuf::from(value));
            }
        }
        if let Some(value) = read("LAMBDA_SQUIB_INSTANCE_ID") {
            if value.is_empty()
                || value.len() > 64
                || !value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            {
                return Err(ExecutorError::Unsupported(
                    "Invalid LAMBDA_SQUIB_INSTANCE_ID".into(),
                ));
            }
            config.instance_id = value;
        }
        let mut number = |key: &str, default: u64, max: u64| -> Result<u64, ExecutorError> {
            let value = match read(key) {
                Some(raw) => raw
                    .parse()
                    .map_err(|_| ExecutorError::Unsupported(format!("Invalid {key}")))?,
                None => default,
            };
            if value == 0 || value > max {
                return Err(ExecutorError::Unsupported(format!("Invalid {key}")));
            }
            Ok(value)
        };
        config.stage_port = u32::try_from(number(
            "LAMBDA_SQUIB_STAGE_PORT",
            5003,
            u64::from(u32::MAX),
        )?)
        .map_err(|error| ExecutorError::Unsupported(error.to_string()))?;
        config.connect_timeout =
            Duration::from_millis(number("LAMBDA_SQUIB_CONNECT_TIMEOUT_MS", 15_000, 900_000)?);
        config.response_limit_bytes = usize::try_from(number(
            "LAMBDA_SQUIB_RESPONSE_LIMIT_BYTES",
            DEFAULT_RESPONSE_LIMIT_BYTES as u64,
            DEFAULT_RESPONSE_LIMIT_BYTES as u64,
        )?)
        .map_err(|error| ExecutorError::Unsupported(error.to_string()))?;
        config.run_budget =
            Duration::from_secs(number("LAMBDA_SQUIB_RUN_BUDGET_SECS", 86_400, 604_800)?);
        config.shutdown_timeout =
            Duration::from_millis(number("LAMBDA_SQUIB_SHUTDOWN_TIMEOUT_MS", 10000, 30000)?);
        Ok(config)
    }

    fn required_config_file(&self) -> Result<&Path, ExecutorError> {
        self.config_file.as_deref().ok_or_else(|| {
            ExecutorError::Unsupported(
                "LAMBDA_EXECUTOR=squib requires LAMBDA_SQUIB_CONFIG_FILE".to_owned(),
            )
        })
    }

    fn required_vsock_path(&self) -> Result<&Path, ExecutorError> {
        self.vsock_path.as_deref().ok_or_else(|| {
            ExecutorError::Unsupported(
                "LAMBDA_EXECUTOR=squib requires LAMBDA_SQUIB_VSOCK_PATH".to_owned(),
            )
        })
    }
}

impl Default for SquibExecutorConfig {
    fn default() -> Self {
        Self {
            config_file: Some(default_config_file()),
            vsock_path: Some(default_vsock_path()),
            instance_id: DEFAULT_INSTANCE_ID.to_owned(),
            stage_port: DEFAULT_STAGE_PORT,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            response_limit_bytes: DEFAULT_RESPONSE_LIMIT_BYTES,
            run_budget: DEFAULT_RUN_BUDGET,
            shutdown_timeout: DEFAULT_SHUTDOWN_TIMEOUT,
        }
    }
}

/// Lambda executor that delegates invocation to a Squib microVM guest agent.
#[derive(Debug)]
pub struct SquibExecutor {
    config: SquibExecutorConfig,
    runtime: Mutex<Option<OwnedVm<squib::Squib>>>,
    poisoned: std::sync::atomic::AtomicBool,
    stop: watch::Sender<bool>,
    cleanup: Arc<CleanupTask>,
}

/// A concrete VM owner; even an idle owner's Drop starts retained shutdown.
struct OwnedVm<R: VmShutdown> {
    runtime: Option<R>,
    cleanup: Arc<CleanupTask>,
    spawner: Handle,
}

trait VmShutdown: Send + 'static {
    fn shutdown(self) -> impl Future<Output = Result<(), ExecutorError>> + Send;
}

impl VmShutdown for squib::Squib {
    async fn shutdown(mut self) -> Result<(), ExecutorError> {
        squib::Squib::shutdown(&mut self)
            .await
            .map_err(|_| ExecutorError::CleanupFailed("Squib VM shutdown failed".into()))
    }
}

impl<R: VmShutdown> OwnedVm<R> {
    fn new(runtime: R, cleanup: Arc<CleanupTask>) -> Self {
        Self {
            runtime: Some(runtime),
            cleanup,
            spawner: Handle::current(),
        }
    }
}

impl<R: VmShutdown> fmt::Debug for OwnedVm<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnedVm").finish_non_exhaustive()
    }
}

impl<R: VmShutdown> Drop for OwnedVm<R> {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            // This owner creates the only VM for this cleanup task. Use the
            // captured runtime even if Drop runs outside an entered Tokio context.
            self.cleanup
                .start_on(&self.spawner, move || runtime.shutdown());
        }
    }
}

#[derive(Debug)]
struct RuntimeLease<'a> {
    runtime: Option<OwnedVm<squib::Squib>>,
    poisoned: &'a std::sync::atomic::AtomicBool,
}
impl Drop for RuntimeLease<'_> {
    fn drop(&mut self) {
        if self.runtime.is_some() {
            self.poisoned
                .store(true, std::sync::atomic::Ordering::Release);
            // OwnedVm transfers shutdown to the retained cleanup task on Drop.
        }
    }
}

impl Drop for SquibExecutor {
    fn drop(&mut self) {
        self.poisoned
            .store(true, std::sync::atomic::Ordering::Release);
        self.stop.send_replace(true);
        // An idle VM must follow the same owned shutdown path as a busy one.
        drop(self.runtime.get_mut().take());
    }
}

impl SquibExecutor {
    /// Build a Squib executor.
    #[must_use]
    pub fn new(config: SquibExecutorConfig) -> Self {
        let (stop, _) = watch::channel(false);
        Self {
            config,
            runtime: Mutex::new(None),
            poisoned: std::sync::atomic::AtomicBool::new(false),
            stop,
            cleanup: Arc::new(CleanupTask::default()),
        }
    }

    async fn start_runtime(&self) -> Result<OwnedVm<squib::Squib>, ExecutorError> {
        let config_file = self.config.required_config_file()?;
        ensure_config_file_exists(config_file).await?;

        let builder = squib::Squib::builder()
            .try_instance_id(self.config.instance_id.as_str())
            .map_err(|err| ExecutorError::Unsupported(format!("invalid Squib instance id: {err}")))?
            .config_file(config_file)
            .without_api_socket()
            .run_budget(self.config.run_budget);

        let squib = builder.spawn().await.map_err(|_| {
            ExecutorError::CleanupFailed(
                "Squib startup did not establish confirmed runtime ownership".into(),
            )
        })?;
        Ok(OwnedVm::new(squib, Arc::clone(&self.cleanup)))
    }

    async fn invoke_guest(
        &self,
        req: &InvokeRequest,
        context: &InvocationContext,
    ) -> Result<InvokeResponse, ExecutorError> {
        #[cfg(unix)]
        {
            let mut stream = connect_stage_socket_with_retry(&self.config).await?;
            let request = SquibInvokeRequestWire::from_request(req, context.id())?;
            let request_bytes = serde_json::to_vec(&request)
                .map_err(|err| ExecutorError::Io(format!("encode Squib request: {err}")))?;
            tokio::time::timeout(req.revision.timeout, async {
                use tokio::io::AsyncWriteExt as _;

                stream
                    .write_all(&request_bytes)
                    .await
                    .map_err(|err| ExecutorError::Io(format!("write Squib request: {err}")))?;
                stream.write_all(b"\n").await.map_err(|err| {
                    ExecutorError::Io(format!("write Squib request newline: {err}"))
                })?;
                let line = read_bounded_line(
                    &mut stream,
                    self.config.response_limit_bytes,
                    "Squib response",
                )
                .await?;
                decode_response(&line, req.revision.qualifier.as_str())
            })
            .await
            .map_err(|_| ExecutorError::Timeout(req.revision.timeout))?
        }

        #[cfg(not(unix))]
        {
            let _ = req;
            Err(ExecutorError::Unsupported(
                "Squib executor requires Unix domain sockets".to_owned(),
            ))
        }
    }
    async fn retire_vm(&self, vm: OwnedVm<squib::Squib>) -> Result<(), ExecutorError> {
        self.poisoned
            .store(true, std::sync::atomic::Ordering::Release);
        drop(vm);
        self.wait_for_cleanup().await
    }

    async fn wait_for_cleanup(&self) -> Result<(), ExecutorError> {
        // Only the waiter has a deadline. CleanupTask retains the VM shutdown
        // future even when this budget expires or the caller drops the waiter.
        tokio::time::timeout(self.config.shutdown_timeout, self.cleanup.wait())
            .await
            .map_err(|_| {
                ExecutorError::CleanupFailed("Squib VM shutdown is not confirmed".into())
            })?
    }
}

#[async_trait]
impl Executor for SquibExecutor {
    fn available(&self, request: &InvokeRequest) -> Result<(), ExecutorError> {
        validate_request(request)?;
        self.config.required_vsock_path()?;
        if self.poisoned.load(std::sync::atomic::Ordering::Acquire) {
            return Err(ExecutorError::Unsupported(
                "Squib guest execution was cancelled; restart the runtime before invoking again"
                    .into(),
            ));
        }
        Ok(())
    }

    async fn invoke(
        &self,
        req: InvokeRequest,
        context: InvocationContext,
    ) -> Result<InvokeResponse, ExecutorError> {
        self.available(&req)?;
        context.check()?;
        let mut runtime = tokio::select! {
            biased;
            reason = context.cancelled() => return Err(reason.error()),
            runtime = self.runtime.lock() => runtime,
        };
        self.available(&req)?;
        // Squib's startup future owns unjoined VMM handles; never drop it on cancellation.
        let vm = match runtime.take() {
            Some(vm) => vm,
            None => match self.start_runtime().await {
                Ok(vm) => vm,
                Err(error) => {
                    if matches!(&error, ExecutorError::CleanupFailed(_)) {
                        self.poisoned
                            .store(true, std::sync::atomic::Ordering::Release);
                    }
                    return Err(error);
                }
            },
        };
        let mut lease = RuntimeLease {
            runtime: Some(vm),
            poisoned: &self.poisoned,
        };
        let mut stop = self.stop.subscribe();
        let mut result = tokio::select! {
            biased;
            reason = context.cancelled() => Err(reason.error()),
            _ = stop.wait_for(|stopped| *stopped) => Err(ExecutorError::Cancelled),
            result = self.invoke_guest(&req, &context) => result,
        };
        if self.poisoned.load(std::sync::atomic::Ordering::Acquire) || *self.stop.borrow() {
            result = Err(ExecutorError::Cancelled);
        }
        if result.is_ok() {
            if let Err(error) = context.check() {
                result = Err(error);
            }
        }
        if result.is_ok() {
            *runtime = lease.runtime.take();
        } else if let Some(vm) = lease.runtime.take() {
            self.retire_vm(vm).await?;
        }
        result
    }

    async fn shutdown(&self) -> Result<(), ExecutorError> {
        self.poisoned
            .store(true, std::sync::atomic::Ordering::Release);
        self.stop.send_replace(true);
        let mut runtime = self.runtime.lock().await;
        if let Some(vm) = runtime.take() {
            self.retire_vm(vm).await?;
        }
        drop(runtime);
        self.wait_for_cleanup().await
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SquibInvokeRequestWire {
    protocol_version: &'static str,
    request_id: String,
    function_arn: String,
    function_name: String,
    qualifier: String,
    runtime: Option<String>,
    handler: Option<String>,
    architectures: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    code_root: Option<String>,
    code_zip_base64: String,
    environment: HashMap<String, String>,
    timeout_ms: u64,
    memory_mb: u32,
    capture_logs: bool,
    payload_base64: String,
}

impl SquibInvokeRequestWire {
    fn from_request(req: &InvokeRequest, request_id: Uuid) -> Result<Self, ExecutorError> {
        let DeploymentArtifact::Zip(artifact) = &req.revision.artifact else {
            return Err(ExecutorError::Unsupported(
                "Squib backend only supports Zip packages".into(),
            ));
        };
        Ok(Self {
            protocol_version: PROTOCOL_VERSION,
            request_id: request_id.to_string(),
            function_arn: req.function_arn.clone(),
            function_name: req.revision.function_name.clone(),
            qualifier: req.revision.qualifier.clone(),
            runtime: req.revision.runtime.clone(),
            handler: req.revision.handler.clone(),
            architectures: vec![req.revision.architecture.as_str().into()],
            code_root: Some(artifact.code_root().display().to_string()),
            code_zip_base64: STANDARD.encode(artifact.zip_bytes()),
            environment: req.revision.environment.clone(),
            timeout_ms: duration_millis(req.revision.timeout),
            memory_mb: req.revision.memory_mb,
            capture_logs: req.capture_logs,
            payload_base64: STANDARD.encode(req.payload.as_ref()),
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SquibInvokeResponseWire {
    protocol_version: String,
    status: u16,
    payload_base64: String,
    #[serde(default)]
    function_error: Option<String>,
    #[serde(default)]
    log_tail_base64: Option<String>,
    #[serde(default)]
    executed_version: Option<String>,
}

fn validate_request(req: &InvokeRequest) -> Result<(), ExecutorError> {
    if !matches!(req.revision.artifact, DeploymentArtifact::Zip(_)) {
        return Err(ExecutorError::Unsupported(
            "Squib backend only supports Zip packages".to_owned(),
        ));
    }
    if req.revision.architecture != Architecture::Arm64 {
        return Err(ExecutorError::Unsupported(
            "Squib backend requires an arm64 Lambda function".to_owned(),
        ));
    }
    Ok(())
}

async fn ensure_config_file_exists(path: &Path) -> Result<(), ExecutorError> {
    let metadata = tokio::fs::metadata(path).await.map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            ExecutorError::Unsupported(format!(
                "LAMBDA_SQUIB_CONFIG_FILE does not exist: {}. Run `make lambda-squib-image` or \
                 set LAMBDA_SQUIB_CONFIG_FILE and LAMBDA_SQUIB_VSOCK_PATH to a Lambda-capable \
                 Squib guest image.",
                path.display()
            ))
        } else {
            ExecutorError::Io(format!("stat Squib config {}: {err}", path.display()))
        }
    })?;
    if !metadata.is_file() {
        return Err(ExecutorError::Unsupported(format!(
            "LAMBDA_SQUIB_CONFIG_FILE is not a file: {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(unix)]
async fn connect_stage_socket_with_retry(
    config: &SquibExecutorConfig,
) -> Result<tokio::net::UnixStream, ExecutorError> {
    let deadline = Instant::now() + config.connect_timeout;
    let mut last_error = None;

    loop {
        let now = Instant::now();
        if now >= deadline {
            return Err(last_error.unwrap_or(ExecutorError::Timeout(config.connect_timeout)));
        }
        let attempt_timeout = deadline
            .saturating_duration_since(now)
            .min(CONNECT_ATTEMPT_TIMEOUT);
        match tokio::time::timeout(attempt_timeout, connect_stage_socket(config)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(error)) if is_retryable_connect_error(&error) => {
                last_error = Some(error);
                sleep_before_retry(deadline).await;
            }
            Ok(Err(error)) => return Err(error),
            Err(_) => {
                last_error = Some(ExecutorError::Timeout(attempt_timeout));
                sleep_before_retry(deadline).await;
            }
        }
    }
}

#[cfg(unix)]
async fn sleep_before_retry(deadline: Instant) {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if !remaining.is_zero() {
        tokio::time::sleep(remaining.min(CONNECT_RETRY_INTERVAL)).await;
    }
}

#[cfg(unix)]
fn is_retryable_connect_error(error: &ExecutorError) -> bool {
    match error {
        ExecutorError::Io(message) => message.contains("connect "),
        ExecutorError::InitFailed(message) => message.contains("RST "),
        ExecutorError::RuntimeExited(message) => message.contains("closed before newline"),
        _ => false,
    }
}

#[cfg(unix)]
async fn connect_stage_socket(
    config: &SquibExecutorConfig,
) -> Result<tokio::net::UnixStream, ExecutorError> {
    use tokio::io::AsyncWriteExt as _;

    let vsock_path = config.required_vsock_path()?;
    let port_path = derive_port_socket_path(vsock_path, config.stage_port);
    let mut stream = tokio::net::UnixStream::connect(&port_path)
        .await
        .map_err(|err| ExecutorError::Io(format!("connect {}: {err}", port_path.display())))?;
    stream
        .write_all(format!("CONNECT {}\n", config.stage_port).as_bytes())
        .await
        .map_err(|err| ExecutorError::Io(format!("write Squib CONNECT preamble: {err}")))?;
    let response =
        read_bounded_line(&mut stream, CONNECT_RESPONSE_LIMIT_BYTES, "Squib CONNECT").await?;
    let response = std::str::from_utf8(&response).map_err(|err| {
        ExecutorError::InitFailed(format!("Squib CONNECT response is not UTF-8: {err}"))
    })?;
    if !response.starts_with("OK ") {
        return Err(ExecutorError::InitFailed(format!(
            "unexpected Squib CONNECT response: {response}"
        )));
    }
    Ok(stream)
}

#[cfg(unix)]
async fn read_bounded_line(
    stream: &mut tokio::net::UnixStream,
    limit: usize,
    context: &str,
) -> Result<Vec<u8>, ExecutorError> {
    use tokio::io::AsyncReadExt as _;

    let mut line = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        let n = stream
            .read(&mut byte)
            .await
            .map_err(|err| ExecutorError::Io(format!("read {context}: {err}")))?;
        if n == 0 {
            return Err(ExecutorError::RuntimeExited(format!(
                "{context} closed before newline"
            )));
        }
        if byte[0] == b'\n' {
            return Ok(line);
        }
        if line.len() >= limit {
            return Err(ExecutorError::RuntimeExited(format!(
                "{context} exceeded {limit} bytes"
            )));
        }
        line.push(byte[0]);
    }
}

fn decode_response(line: &[u8], default_version: &str) -> Result<InvokeResponse, ExecutorError> {
    let response: SquibInvokeResponseWire = serde_json::from_slice(line).map_err(|err| {
        ExecutorError::RuntimeExited(format!("decode Squib response JSON: {err}"))
    })?;
    if response.protocol_version != PROTOCOL_VERSION {
        return Err(ExecutorError::RuntimeExited(format!(
            "unsupported Squib response protocol version: {}",
            response.protocol_version
        )));
    }
    let payload = STANDARD
        .decode(response.payload_base64.as_bytes())
        .map_err(|err| {
            ExecutorError::RuntimeExited(format!("decode Squib response payload: {err}"))
        })?;
    if payload.len() > LAMBDA_SYNC_RESPONSE_PAYLOAD_LIMIT_BYTES {
        let payload_len = payload.len();
        return Err(ExecutorError::RuntimeExited(format!(
            "Squib response payload exceeded {LAMBDA_SYNC_RESPONSE_PAYLOAD_LIMIT_BYTES} bytes: \
             {payload_len}"
        )));
    }
    Ok(InvokeResponse {
        status: response.status,
        payload: Bytes::from(payload),
        function_error: response.function_error,
        log_tail: response.log_tail_base64,
        executed_version: response
            .executed_version
            .unwrap_or_else(|| default_version.to_owned()),
    })
}

fn derive_port_socket_path(base: &Path, port: u32) -> PathBuf {
    let name = base
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("vsock.sock");
    let dir = base.parent().unwrap_or_else(|| Path::new("."));
    dir.join(format!("{name}_{port}"))
}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

const fn base64_encoded_len(input_len: usize) -> usize {
    input_len.div_ceil(3) * 4
}

fn default_config_file() -> PathBuf {
    default_artifact_dir().join("config.json")
}

fn default_vsock_path() -> PathBuf {
    default_artifact_dir().join("vsock.sock")
}

fn default_artifact_dir() -> PathBuf {
    if let Ok(root) = env::var("RUSTACK_WORKSPACE_ROOT") {
        return PathBuf::from(root).join("target/rustack-lambda-squib");
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map_or_else(
            || PathBuf::from("target/rustack-lambda-squib"),
            |root| root.join("target/rustack-lambda-squib"),
        )
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use super::*;

    struct ProbeVm {
        started: Arc<tokio::sync::Semaphore>,
        gate: Arc<tokio::sync::Semaphore>,
        reservation: tokio::sync::OwnedSemaphorePermit,
    }

    impl VmShutdown for ProbeVm {
        async fn shutdown(self) -> Result<(), ExecutorError> {
            self.started.add_permits(1);
            self.gate.acquire().await.unwrap().forget();
            drop(self.reservation);
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_should_transfer_idle_vm_drop_to_owned_shutdown_before_releasing_resources() {
        let cleanup = Arc::new(CleanupTask::default());
        let capacity = Arc::new(tokio::sync::Semaphore::new(1));
        let started = Arc::new(tokio::sync::Semaphore::new(0));
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let owner = OwnedVm::new(
            ProbeVm {
                started: Arc::clone(&started),
                gate: Arc::clone(&gate),
                reservation: capacity.clone().acquire_owned().await.unwrap(),
            },
            Arc::clone(&cleanup),
        );
        // Drop on an ordinary thread with no entered Tokio context.
        std::thread::spawn(move || drop(owner)).join().unwrap();
        started.acquire().await.unwrap().forget();
        assert_eq!(capacity.available_permits(), 0);
        assert!(
            tokio::time::timeout(Duration::ZERO, cleanup.wait())
                .await
                .is_err()
        );
        assert_eq!(capacity.available_permits(), 0);
        gate.add_permits(1);
        cleanup.wait().await.unwrap();
        assert_eq!(capacity.available_permits(), 1);
    }

    #[test]
    fn test_should_parse_squib_config_from_env_reader() {
        let config = SquibExecutorConfig::from_env_reader(|key| match key {
            "LAMBDA_SQUIB_CONFIG_FILE" => Some("/tmp/vm.json".to_owned()),
            "LAMBDA_SQUIB_VSOCK_PATH" => Some("/tmp/vsock.sock".to_owned()),
            "LAMBDA_SQUIB_INSTANCE_ID" => Some("lambda-dev".to_owned()),
            "LAMBDA_SQUIB_STAGE_PORT" => Some("6003".to_owned()),
            "LAMBDA_SQUIB_CONNECT_TIMEOUT_MS" => Some("3500".to_owned()),
            "LAMBDA_SQUIB_RESPONSE_LIMIT_BYTES" => Some("4096".to_owned()),
            "LAMBDA_SQUIB_RUN_BUDGET_SECS" => Some("60".to_owned()),
            "LAMBDA_SQUIB_SHUTDOWN_TIMEOUT_MS" => Some("2500".to_owned()),
            _ => None,
        })
        .unwrap();

        assert_eq!(config.config_file, Some(PathBuf::from("/tmp/vm.json")));
        assert_eq!(config.vsock_path, Some(PathBuf::from("/tmp/vsock.sock")));
        assert_eq!(config.instance_id, "lambda-dev");
        assert_eq!(config.stage_port, 6003);
        assert_eq!(config.connect_timeout, Duration::from_millis(3500));
        assert_eq!(config.response_limit_bytes, 4096);
        assert_eq!(config.run_budget, Duration::from_mins(1));
        assert_eq!(config.shutdown_timeout, Duration::from_millis(2500));
    }

    #[test]
    fn test_should_derive_squib_port_socket_path() {
        let path = derive_port_socket_path(Path::new("/tmp/rustack/vsock.sock"), 5003);
        assert_eq!(path, PathBuf::from("/tmp/rustack/vsock.sock_5003"));
    }

    #[test]
    fn test_should_require_config_file_for_runtime_start() {
        let config = SquibExecutorConfig {
            config_file: None,
            ..SquibExecutorConfig::default()
        };

        assert!(matches!(
            config.required_config_file(),
            Err(ExecutorError::Unsupported(message))
                if message.contains("LAMBDA_SQUIB_CONFIG_FILE")
        ));
    }

    #[test]
    fn test_should_require_vsock_path_for_invocation() {
        let config = SquibExecutorConfig {
            vsock_path: None,
            ..SquibExecutorConfig::default()
        };

        assert!(matches!(
            config.required_vsock_path(),
            Err(ExecutorError::Unsupported(message)) if message.contains("LAMBDA_SQUIB_VSOCK_PATH")
        ));
    }

    #[test]
    fn test_should_encode_request_payload_base64() {
        let mut req = InvokeRequest::fixture();
        req.payload = Bytes::from_static(b"{}");
        req.capture_logs = true;
        let wire = SquibInvokeRequestWire::from_request(&req, Uuid::new_v4()).unwrap();

        assert_eq!(wire.protocol_version, PROTOCOL_VERSION);
        let DeploymentArtifact::Zip(artifact) = &req.revision.artifact else {
            panic!("unit fixture is ZIP");
        };
        assert_eq!(
            wire.code_root,
            Some(artifact.code_root().display().to_string())
        );
        assert_eq!(
            STANDARD.decode(&wire.code_zip_base64).unwrap(),
            *artifact.zip_bytes()
        );
        assert_eq!(wire.timeout_ms, 3000);
        assert_eq!(wire.payload_base64, "e30=");
    }

    #[test]
    fn test_should_decode_guest_response() {
        let line = br#"{"protocolVersion":"rustack.squib.lambda.v1","status":200,"payloadBase64":"eyJvayI6dHJ1ZX0="}"#;

        let response = decode_response(line, "$LATEST").unwrap();

        assert_eq!(response.status, 200);
        assert_eq!(response.payload, Bytes::from_static(br#"{"ok":true}"#));
        assert_eq!(response.executed_version, "$LATEST");
    }

    #[test]
    fn test_should_allow_max_size_guest_response_with_json_overhead() {
        let payload = vec![b'x'; LAMBDA_SYNC_RESPONSE_PAYLOAD_LIMIT_BYTES];
        let wire = SquibInvokeResponseWireTest {
            protocol_version: PROTOCOL_VERSION,
            status: 200,
            payload_base64: STANDARD.encode(&payload),
        };
        let line = serde_json::to_vec(&wire).unwrap();

        assert!(line.len() > base64_encoded_len(LAMBDA_SYNC_RESPONSE_PAYLOAD_LIMIT_BYTES));
        assert!(line.len() <= DEFAULT_RESPONSE_LIMIT_BYTES);
        let response = decode_response(&line, "$LATEST").unwrap();

        assert_eq!(
            response.payload.len(),
            LAMBDA_SYNC_RESPONSE_PAYLOAD_LIMIT_BYTES
        );
    }

    #[test]
    fn test_should_reject_oversized_guest_response_payload() {
        let payload = vec![b'x'; LAMBDA_SYNC_RESPONSE_PAYLOAD_LIMIT_BYTES + 1];
        let wire = SquibInvokeResponseWireTest {
            protocol_version: PROTOCOL_VERSION,
            status: 200,
            payload_base64: STANDARD.encode(&payload),
        };
        let line = serde_json::to_vec(&wire).unwrap();

        assert!(matches!(
            decode_response(&line, "$LATEST"),
            Err(ExecutorError::RuntimeExited(message))
                if message.contains("response payload exceeded")
        ));
    }

    #[derive(Debug, Serialize)]
    #[serde(rename_all = "camelCase")]
    struct SquibInvokeResponseWireTest {
        protocol_version: &'static str,
        status: u16,
        payload_base64: String,
    }

    #[test]
    fn test_should_reject_non_arm64_request() {
        let mut req = InvokeRequest::fixture();
        Arc::make_mut(&mut req.revision).architecture = Architecture::X86_64;

        assert!(matches!(
            validate_request(&req),
            Err(ExecutorError::Unsupported(message)) if message.contains("arm64")
        ));
    }
}
