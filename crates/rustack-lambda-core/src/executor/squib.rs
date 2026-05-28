//! Squib microVM execution backend.
//!
//! This backend owns the host-side integration with Squib. It starts one
//! long-lived microVM from a static Squib config and uses Squib's vsock muxer
//! to send staged Lambda invocations to a guest agent.

use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
    time::Duration,
};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::warn;
use uuid::Uuid;

use super::{Executor, ExecutorError, InvokeRequest, InvokeResponse, PackageType};

const DEFAULT_INSTANCE_ID: &str = "rustack-lambda";
const DEFAULT_STAGE_PORT: u32 = 5003;
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const DEFAULT_RESPONSE_LIMIT_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_RUN_BUDGET: Duration = Duration::from_hours(24);
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
}

impl SquibExecutorConfig {
    /// Read Squib executor configuration from environment variables.
    #[must_use]
    pub fn from_env() -> Self {
        Self::from_env_reader(|key| env::var(key).ok())
    }

    pub(crate) fn from_env_reader(mut read: impl FnMut(&str) -> Option<String>) -> Self {
        let instance_id = read("LAMBDA_SQUIB_INSTANCE_ID")
            .as_deref()
            .and_then(non_empty_string)
            .unwrap_or_else(|| DEFAULT_INSTANCE_ID.to_owned());
        Self {
            config_file: read("LAMBDA_SQUIB_CONFIG_FILE")
                .as_deref()
                .and_then(non_empty_string)
                .map(PathBuf::from),
            vsock_path: read("LAMBDA_SQUIB_VSOCK_PATH")
                .as_deref()
                .and_then(non_empty_string)
                .map(PathBuf::from),
            instance_id,
            stage_port: read("LAMBDA_SQUIB_STAGE_PORT")
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_STAGE_PORT),
            connect_timeout: read("LAMBDA_SQUIB_CONNECT_TIMEOUT_MS")
                .and_then(|v| v.parse::<u64>().ok())
                .map_or(DEFAULT_CONNECT_TIMEOUT, Duration::from_millis),
            response_limit_bytes: read("LAMBDA_SQUIB_RESPONSE_LIMIT_BYTES")
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_RESPONSE_LIMIT_BYTES),
            run_budget: read("LAMBDA_SQUIB_RUN_BUDGET_SECS")
                .and_then(|v| v.parse::<u64>().ok())
                .map_or(DEFAULT_RUN_BUDGET, Duration::from_secs),
        }
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
            config_file: None,
            vsock_path: None,
            instance_id: DEFAULT_INSTANCE_ID.to_owned(),
            stage_port: DEFAULT_STAGE_PORT,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            response_limit_bytes: DEFAULT_RESPONSE_LIMIT_BYTES,
            run_budget: DEFAULT_RUN_BUDGET,
        }
    }
}

/// Lambda executor that delegates invocation to a Squib microVM guest agent.
#[derive(Debug)]
pub struct SquibExecutor {
    config: SquibExecutorConfig,
    runtime: Mutex<Option<squib::Squib>>,
}

impl SquibExecutor {
    /// Build a Squib executor.
    #[must_use]
    pub fn new(config: SquibExecutorConfig) -> Self {
        Self {
            config,
            runtime: Mutex::new(None),
        }
    }

    async fn ensure_runtime(&self) -> Result<(), ExecutorError> {
        let mut runtime = self.runtime.lock().await;
        if runtime.is_some() {
            return Ok(());
        }

        let config_file = self.config.required_config_file()?;
        ensure_config_file_exists(config_file).await?;

        let builder = squib::Squib::builder()
            .try_instance_id(self.config.instance_id.as_str())
            .map_err(|err| ExecutorError::Unsupported(format!("invalid Squib instance id: {err}")))?
            .config_file(config_file)
            .without_api_socket()
            .run_budget(self.config.run_budget);

        let squib = builder
            .spawn()
            .await
            .map_err(|err| ExecutorError::InitFailed(format!("start Squib runtime: {err}")))?;
        *runtime = Some(squib);
        Ok(())
    }

    async fn invoke_guest(&self, req: InvokeRequest) -> Result<InvokeResponse, ExecutorError> {
        #[cfg(unix)]
        {
            let mut stream = tokio::time::timeout(
                self.config.connect_timeout,
                connect_stage_socket(&self.config),
            )
            .await
            .map_err(|_| ExecutorError::Timeout(self.config.connect_timeout))??;
            let request = SquibInvokeRequestWire::from_request(&req)?;
            let request_bytes = serde_json::to_vec(&request)
                .map_err(|err| ExecutorError::Io(format!("encode Squib request: {err}")))?;
            tokio::time::timeout(req.timeout, async {
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
                decode_response(&line, req.qualifier.as_str())
            })
            .await
            .map_err(|_| ExecutorError::Timeout(req.timeout))?
        }

        #[cfg(not(unix))]
        {
            let _ = req;
            Err(ExecutorError::Unsupported(
                "Squib executor requires Unix domain sockets".to_owned(),
            ))
        }
    }
}

#[async_trait]
impl Executor for SquibExecutor {
    async fn invoke(&self, req: InvokeRequest) -> Result<InvokeResponse, ExecutorError> {
        validate_request(&req)?;
        self.config.required_vsock_path()?;
        self.ensure_runtime().await?;
        self.invoke_guest(req).await
    }

    async fn shutdown(&self) {
        let mut runtime = self.runtime.lock().await.take();
        if let Some(squib) = runtime.as_mut()
            && let Err(err) = squib.shutdown().await
        {
            warn!(error = %err, "failed to shut down Squib Lambda runtime");
        }
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
    code_root: String,
    environment: HashMap<String, String>,
    timeout_ms: u64,
    memory_mb: u32,
    capture_logs: bool,
    payload_base64: String,
}

impl SquibInvokeRequestWire {
    fn from_request(req: &InvokeRequest) -> Result<Self, ExecutorError> {
        let code_root = req
            .code_root
            .as_ref()
            .ok_or_else(|| ExecutorError::InvalidCode("missing code root".to_owned()))?;
        Ok(Self {
            protocol_version: PROTOCOL_VERSION,
            request_id: Uuid::new_v4().to_string(),
            function_arn: req.function_arn.clone(),
            function_name: req.function_name.clone(),
            qualifier: req.qualifier.clone(),
            runtime: req.runtime.clone(),
            handler: req.handler.clone(),
            architectures: req.architectures.clone(),
            code_root: code_root.display().to_string(),
            environment: req.environment.clone(),
            timeout_ms: duration_millis(req.timeout),
            memory_mb: req.memory_mb,
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
    if req.package_type != PackageType::Zip {
        return Err(ExecutorError::Unsupported(
            "Squib backend only supports Zip packages".to_owned(),
        ));
    }
    if !req.architectures.iter().any(|arch| arch == "arm64") {
        return Err(ExecutorError::Unsupported(
            "Squib backend requires an arm64 Lambda function".to_owned(),
        ));
    }
    if req.code_root.is_none() {
        return Err(ExecutorError::InvalidCode("missing code root".to_owned()));
    }
    Ok(())
}

async fn ensure_config_file_exists(path: &Path) -> Result<(), ExecutorError> {
    let metadata = tokio::fs::metadata(path).await.map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            ExecutorError::Unsupported(format!(
                "LAMBDA_SQUIB_CONFIG_FILE does not exist: {}",
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

fn non_empty_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

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
            _ => None,
        });

        assert_eq!(config.config_file, Some(PathBuf::from("/tmp/vm.json")));
        assert_eq!(config.vsock_path, Some(PathBuf::from("/tmp/vsock.sock")));
        assert_eq!(config.instance_id, "lambda-dev");
        assert_eq!(config.stage_port, 6003);
        assert_eq!(config.connect_timeout, Duration::from_millis(3500));
        assert_eq!(config.response_limit_bytes, 4096);
        assert_eq!(config.run_budget, Duration::from_mins(1));
    }

    #[test]
    fn test_should_derive_squib_port_socket_path() {
        let path = derive_port_socket_path(Path::new("/tmp/rustack/vsock.sock"), 5003);
        assert_eq!(path, PathBuf::from("/tmp/rustack/vsock.sock_5003"));
    }

    #[test]
    fn test_should_require_config_file_for_runtime_start() {
        let config = SquibExecutorConfig::default();

        assert!(matches!(
            config.required_config_file(),
            Err(ExecutorError::Unsupported(message))
                if message.contains("LAMBDA_SQUIB_CONFIG_FILE")
        ));
    }

    #[test]
    fn test_should_require_vsock_path_for_invocation() {
        let config = SquibExecutorConfig::default();

        assert!(matches!(
            config.required_vsock_path(),
            Err(ExecutorError::Unsupported(message)) if message.contains("LAMBDA_SQUIB_VSOCK_PATH")
        ));
    }

    #[test]
    fn test_should_encode_request_payload_base64() {
        let req = InvokeRequest {
            function_arn: "arn:aws:lambda:us-east-1:000000000000:function:echo".to_owned(),
            function_name: "echo".to_owned(),
            qualifier: "$LATEST".to_owned(),
            runtime: Some("provided.al2023".to_owned()),
            handler: Some("bootstrap".to_owned()),
            architectures: vec!["arm64".to_owned()],
            package_type: PackageType::Zip,
            code_root: Some(PathBuf::from("/tmp/code")),
            image_uri: None,
            environment: HashMap::from([("KEY".to_owned(), "VALUE".to_owned())]),
            timeout: Duration::from_secs(3),
            memory_mb: 128,
            payload: Bytes::from_static(b"{}"),
            capture_logs: true,
        };

        let wire = SquibInvokeRequestWire::from_request(&req).unwrap();

        assert_eq!(wire.protocol_version, PROTOCOL_VERSION);
        assert_eq!(wire.code_root, "/tmp/code");
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
    fn test_should_reject_non_arm64_request() {
        let req = InvokeRequest {
            function_arn: String::new(),
            function_name: "echo".to_owned(),
            qualifier: "$LATEST".to_owned(),
            runtime: None,
            handler: None,
            architectures: vec!["x86_64".to_owned()],
            package_type: PackageType::Zip,
            code_root: Some(PathBuf::from("/tmp/code")),
            image_uri: None,
            environment: HashMap::new(),
            timeout: Duration::from_secs(1),
            memory_mb: 128,
            payload: Bytes::new(),
            capture_logs: false,
        };

        assert!(matches!(
            validate_request(&req),
            Err(ExecutorError::Unsupported(message)) if message.contains("arm64")
        ));
    }
}
