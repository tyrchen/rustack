//! In-guest Rustack Lambda runtime agent.
//!
//! The agent listens on virtio-vsock port 5003, receives a single JSON
//! invocation from the Rustack host executor, stages the uploaded Lambda Zip in
//! `/tmp`, runs `bootstrap`, serves the Lambda Runtime API on loopback, and
//! returns one JSON response over the same vsock stream.

// Archive extraction runs inside `spawn_blocking`, and child stdio redirection
// requires `std::fs::File`.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
#![forbid(unsafe_code)]
#![warn(missing_debug_implementations, rust_2024_compatibility)]

use std::{
    collections::HashMap,
    fs::{self, File},
    io::Cursor,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, Result, anyhow, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpListener, TcpStream},
    process::Command,
    sync::{Mutex, oneshot},
};
use tokio_vsock::{VMADDR_CID_ANY, VsockAddr, VsockListener};
use uuid::Uuid;

const STAGE_PORT: u32 = 5003;
const PROTOCOL_VERSION: &str = "rustack.squib.lambda.v1";
const SHUTDOWN_REQUEST_KIND: &str = "shutdown";
const MAX_REQUEST_BYTES: usize = 80 * 1024 * 1024;
const MAX_HTTP_HEADER_BYTES: usize = 64 * 1024;
const MAX_RUNTIME_BODY_BYTES: usize = 8 * 1024 * 1024;
const BIND_RETRY_INTERVAL: Duration = Duration::from_millis(100);
const BIND_RETRY_ATTEMPTS: usize = 300;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InvokeRequestWire {
    protocol_version: String,
    request_id: String,
    function_arn: String,
    function_name: String,
    qualifier: String,
    runtime: Option<String>,
    handler: Option<String>,
    architectures: Vec<String>,
    code_zip_base64: String,
    environment: HashMap<String, String>,
    timeout_ms: u64,
    memory_mb: u32,
    capture_logs: bool,
    payload_base64: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InvokeResponseWire {
    protocol_version: &'static str,
    status: u16,
    payload_base64: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    function_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    log_tail_base64: Option<String>,
    executed_version: String,
}

#[derive(Debug)]
struct RuntimeOutcome {
    payload: Vec<u8>,
    function_error: Option<String>,
}

#[derive(Debug)]
struct HttpRequest {
    method: String,
    path: String,
    body: Vec<u8>,
}

#[tokio::main]
async fn main() -> Result<()> {
    eprintln!("[rustack-lambda-agent] listening on vsock port {STAGE_PORT}");
    let listener = bind_stage_listener().await?;
    loop {
        let (mut stream, peer) = listener.accept().await.context("accept vsock")?;
        eprintln!("[rustack-lambda-agent] accepted invocation from {peer:?}");
        tokio::spawn(async move {
            let response = match handle_connection(&mut stream).await {
                Ok(response) => response,
                Err(error) => error_response(error),
            };
            match serde_json::to_vec(&response) {
                Ok(bytes) => {
                    let write_result = async {
                        stream.write_all(&bytes).await?;
                        stream.write_all(b"\n").await
                    }
                    .await;
                    if let Err(error) = write_result {
                        eprintln!("[rustack-lambda-agent] write response failed: {error}");
                    }
                }
                Err(error) => {
                    eprintln!("[rustack-lambda-agent] encode response failed: {error}");
                }
            }
        });
    }
}

async fn bind_stage_listener() -> Result<VsockListener> {
    let addr = VsockAddr::new(VMADDR_CID_ANY, STAGE_PORT);
    let mut last_error = None;
    for _ in 0..BIND_RETRY_ATTEMPTS {
        match VsockListener::bind(addr) {
            Ok(listener) => return Ok(listener),
            Err(error) => {
                last_error = Some(error);
                tokio::time::sleep(BIND_RETRY_INTERVAL).await;
            }
        }
    }
    Err(last_error.map_or_else(|| anyhow!("bind stage vsock listener"), anyhow::Error::from))
        .context("bind stage vsock listener")
}

async fn handle_connection(stream: &mut tokio_vsock::VsockStream) -> Result<InvokeResponseWire> {
    let line = read_bounded_line(stream).await?;
    let request_value: serde_json::Value =
        serde_json::from_slice(&line).context("decode request JSON")?;
    if is_shutdown_request(&request_value)? {
        schedule_guest_poweroff();
        return Ok(InvokeResponseWire {
            protocol_version: PROTOCOL_VERSION,
            status: 202,
            payload_base64: STANDARD.encode(b"{}"),
            function_error: None,
            log_tail_base64: None,
            executed_version: "$LATEST".to_owned(),
        });
    }

    let request: InvokeRequestWire =
        serde_json::from_value(request_value).context("decode invoke request JSON")?;
    validate_request(&request)?;

    let payload = STANDARD
        .decode(request.payload_base64.as_bytes())
        .context("decode payload")?;
    let zip_bytes = STANDARD
        .decode(request.code_zip_base64.as_bytes())
        .context("decode code zip")?;
    let work_dir = stage_zip(&request.request_id, &zip_bytes).await?;
    let timeout = Duration::from_millis(request.timeout_ms.max(1));
    let outcome = run_bootstrap(&request, &work_dir, payload, timeout).await?;
    let _ = tokio::fs::remove_dir_all(&work_dir).await;

    Ok(InvokeResponseWire {
        protocol_version: PROTOCOL_VERSION,
        status: 200,
        payload_base64: STANDARD.encode(outcome.payload),
        function_error: outcome.function_error,
        log_tail_base64: None,
        executed_version: request.qualifier,
    })
}

fn is_shutdown_request(request: &serde_json::Value) -> Result<bool> {
    let protocol_version = request
        .get("protocolVersion")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("missing protocolVersion"))?;
    if protocol_version != PROTOCOL_VERSION {
        bail!("unsupported protocol version: {protocol_version}");
    }
    Ok(request
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|kind| kind == SHUTDOWN_REQUEST_KIND))
}

fn schedule_guest_poweroff() {
    tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let result = Command::new("/usr/bin/busybox")
            .args(["poweroff", "-f"])
            .status()
            .await;
        if let Err(error) = result {
            eprintln!("[rustack-lambda-agent] poweroff failed: {error}");
        }
    });
}

fn validate_request(request: &InvokeRequestWire) -> Result<()> {
    if request.protocol_version != PROTOCOL_VERSION {
        bail!("unsupported protocol version: {}", request.protocol_version);
    }
    if !request.architectures.iter().any(|arch| arch == "arm64") {
        bail!("guest only supports arm64 Lambda artifacts");
    }
    if request.code_zip_base64.is_empty() {
        bail!("missing code zip");
    }
    Ok(())
}

async fn read_bounded_line(stream: &mut tokio_vsock::VsockStream) -> Result<Vec<u8>> {
    let mut line = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        let n = stream.read(&mut byte).await.context("read request")?;
        if n == 0 {
            bail!("request closed before newline");
        }
        if byte[0] == b'\n' {
            return Ok(line);
        }
        if line.len() >= MAX_REQUEST_BYTES {
            bail!("request exceeds {} bytes", MAX_REQUEST_BYTES);
        }
        line.push(byte[0]);
    }
}

async fn stage_zip(request_id: &str, zip_bytes: &[u8]) -> Result<PathBuf> {
    let root = PathBuf::from("/tmp/rustack-lambda").join(request_id);
    let extract_root = root.join("code");
    tokio::fs::create_dir_all(&extract_root)
        .await
        .with_context(|| format!("create {}", extract_root.display()))?;
    let zip = zip_bytes.to_vec();
    let target = extract_root.clone();
    tokio::task::spawn_blocking(move || extract_zip(&zip, &target))
        .await
        .context("join zip extraction")??;
    let bootstrap = extract_root.join("bootstrap");
    if !bootstrap.is_file() {
        bail!("zip does not contain executable bootstrap at archive root");
    }
    Ok(extract_root)
}

fn extract_zip(zip_bytes: &[u8], target: &Path) -> Result<()> {
    let reader = Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(reader).context("open zip archive")?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .with_context(|| format!("open zip entry {index}"))?;
        let Some(enclosed) = entry.enclosed_name() else {
            bail!("zip entry has invalid path: {}", entry.name());
        };
        let output = target.join(enclosed);
        if entry.is_dir() {
            fs::create_dir_all(&output)
                .with_context(|| format!("create directory {}", output.display()))?;
            continue;
        }
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create parent directory {}", parent.display()))?;
        }
        let mut out =
            File::create(&output).with_context(|| format!("create {}", output.display()))?;
        std::io::copy(&mut entry, &mut out)
            .with_context(|| format!("write {}", output.display()))?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&output, fs::Permissions::from_mode(mode))
                .with_context(|| format!("chmod {}", output.display()))?;
        }
    }
    Ok(())
}

async fn run_bootstrap(
    request: &InvokeRequestWire,
    code_root: &Path,
    payload: Vec<u8>,
    timeout: Duration,
) -> Result<RuntimeOutcome> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("bind runtime API")?;
    let runtime_addr = listener.local_addr().context("runtime API address")?;
    let (result_tx, result_rx) = oneshot::channel();
    let result_tx = Arc::new(Mutex::new(Some(result_tx)));
    let next_sent = Arc::new(AtomicBool::new(false));
    let request_id = Uuid::new_v4().to_string();
    let server = tokio::spawn(runtime_api_server(
        listener,
        RuntimeContext {
            request_id: request_id.clone(),
            function_arn: request.function_arn.clone(),
            deadline_ms: deadline_ms(timeout),
            payload,
            result_tx,
            next_sent,
        },
    ));
    let stdout_path = code_root.join("bootstrap.stdout");
    let stderr_path = code_root.join("bootstrap.stderr");
    let stdout_file =
        File::create(&stdout_path).with_context(|| format!("create {}", stdout_path.display()))?;
    let stderr_file =
        File::create(&stderr_path).with_context(|| format!("create {}", stderr_path.display()))?;

    let mut cmd = Command::new(code_root.join("bootstrap"));
    cmd.current_dir(code_root)
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file))
        .env("AWS_LAMBDA_RUNTIME_API", runtime_addr.to_string())
        .env("AWS_LAMBDA_FUNCTION_NAME", &request.function_name)
        .env("AWS_LAMBDA_FUNCTION_VERSION", &request.qualifier)
        .env(
            "AWS_LAMBDA_FUNCTION_MEMORY_SIZE",
            request.memory_mb.to_string(),
        )
        .env(
            "_HANDLER",
            request.handler.as_deref().unwrap_or("bootstrap"),
        )
        .env(
            "AWS_EXECUTION_ENV",
            request.runtime.as_deref().unwrap_or("provided.al2023"),
        );
    for (key, value) in &request.environment {
        cmd.env(key, value);
    }
    if request.capture_logs {
        cmd.kill_on_drop(true);
    }
    let mut child = cmd.spawn().context("spawn Lambda bootstrap")?;

    let result = tokio::select! {
        outcome = result_rx => outcome.context("runtime API response channel closed")?,
        wait = child.wait() => {
            let status = wait.context("wait for bootstrap")?;
            let stdout = read_short_file(&stdout_path).await;
            let stderr = read_short_file(&stderr_path).await;
            bail!("bootstrap exited before posting a response: {status}; stdout={stdout:?}; stderr={stderr:?}");
        }
        () = tokio::time::sleep(timeout) => {
            bail!("Lambda timed out after {} ms", timeout.as_millis());
        }
    };

    let _ = child.kill().await;
    server.abort();
    Ok(result)
}

async fn read_short_file(path: &Path) -> String {
    match tokio::fs::read(path).await {
        Ok(bytes) => {
            let start = bytes.len().saturating_sub(4096);
            String::from_utf8_lossy(&bytes[start..]).into_owned()
        }
        Err(error) => format!("read {} failed: {error}", path.display()),
    }
}

#[derive(Debug)]
struct RuntimeContext {
    request_id: String,
    function_arn: String,
    deadline_ms: u64,
    payload: Vec<u8>,
    result_tx: Arc<Mutex<Option<oneshot::Sender<RuntimeOutcome>>>>,
    next_sent: Arc<AtomicBool>,
}

async fn runtime_api_server(listener: TcpListener, context: RuntimeContext) {
    let context = Arc::new(context);
    loop {
        let accepted = listener.accept().await;
        let Ok((stream, _addr)) = accepted else {
            return;
        };
        let context = Arc::clone(&context);
        tokio::spawn(async move {
            if let Err(error) = handle_runtime_connection(stream, context).await {
                eprintln!("[rustack-lambda-agent] runtime API request failed: {error}");
            }
        });
    }
}

async fn handle_runtime_connection(
    mut stream: TcpStream,
    context: Arc<RuntimeContext>,
) -> Result<()> {
    let request = read_http_request(&mut stream).await?;
    if request.method == "GET" && request.path == "/2018-06-01/runtime/invocation/next" {
        if context.next_sent.swap(true, Ordering::SeqCst) {
            write_http_response(&mut stream, 204, &[], &[]).await?;
            return Ok(());
        }
        let headers = vec![
            (
                "Lambda-Runtime-Aws-Request-Id".to_owned(),
                context.request_id.clone(),
            ),
            (
                "Lambda-Runtime-Invoked-Function-Arn".to_owned(),
                context.function_arn.clone(),
            ),
            (
                "Lambda-Runtime-Deadline-Ms".to_owned(),
                context.deadline_ms.to_string(),
            ),
        ];
        write_http_response(&mut stream, 200, &context.payload, &headers).await?;
        return Ok(());
    }

    let response_prefix = format!(
        "/2018-06-01/runtime/invocation/{}/response",
        context.request_id
    );
    if request.method == "POST" && request.path == response_prefix {
        send_runtime_outcome(
            &context,
            RuntimeOutcome {
                payload: request.body,
                function_error: None,
            },
        )
        .await;
        write_http_response(&mut stream, 202, &[], &[]).await?;
        return Ok(());
    }

    let error_prefix = format!(
        "/2018-06-01/runtime/invocation/{}/error",
        context.request_id
    );
    if request.method == "POST" && request.path == error_prefix {
        send_runtime_outcome(
            &context,
            RuntimeOutcome {
                payload: request.body,
                function_error: Some("Unhandled".to_owned()),
            },
        )
        .await;
        write_http_response(&mut stream, 202, &[], &[]).await?;
        return Ok(());
    }

    write_http_response(&mut stream, 404, b"not found", &[]).await?;
    Ok(())
}

async fn send_runtime_outcome(context: &RuntimeContext, outcome: RuntimeOutcome) {
    let mut guard = context.result_tx.lock().await;
    if let Some(sender) = guard.take() {
        let _ = sender.send(outcome);
    }
}

async fn read_http_request(stream: &mut TcpStream) -> Result<HttpRequest> {
    let mut buf = Vec::new();
    let headers_end = loop {
        let mut chunk = [0_u8; 1024];
        let n = stream.read(&mut chunk).await.context("read HTTP request")?;
        if n == 0 {
            bail!("HTTP client closed before headers");
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > MAX_HTTP_HEADER_BYTES + MAX_RUNTIME_BODY_BYTES {
            bail!("HTTP request exceeds size limit");
        }
        if let Some(pos) = find_header_end(&buf) {
            break pos;
        }
        if buf.len() > MAX_HTTP_HEADER_BYTES {
            bail!("HTTP headers exceed size limit");
        }
    };

    let header_bytes = &buf[..headers_end];
    let header_text = std::str::from_utf8(header_bytes).context("HTTP headers are not UTF-8")?;
    let mut lines = header_text.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| anyhow!("missing request line"))?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .ok_or_else(|| anyhow!("missing method"))?
        .to_owned();
    let path = request_parts
        .next()
        .ok_or_else(|| anyhow!("missing path"))?
        .to_owned();
    let mut content_length = 0_usize;
    for line in lines {
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            content_length = value.trim().parse().context("invalid content-length")?;
        }
    }
    if content_length > MAX_RUNTIME_BODY_BYTES {
        bail!("HTTP body exceeds size limit");
    }
    let body_start = headers_end + 4;
    while buf.len() < body_start + content_length {
        let mut chunk = [0_u8; 1024];
        let n = stream.read(&mut chunk).await.context("read HTTP body")?;
        if n == 0 {
            bail!("HTTP client closed before body");
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    Ok(HttpRequest {
        method,
        path,
        body: buf[body_start..body_start + content_length].to_vec(),
    })
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|window| window == b"\r\n\r\n")
}

async fn write_http_response(
    stream: &mut TcpStream,
    status: u16,
    body: &[u8],
    headers: &[(String, String)],
) -> Result<()> {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        404 => "Not Found",
        _ => "OK",
    };
    let mut response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        response.push_str(name);
        response.push_str(": ");
        response.push_str(value);
        response.push_str("\r\n");
    }
    response.push_str("\r\n");
    stream
        .write_all(response.as_bytes())
        .await
        .context("write HTTP headers")?;
    stream.write_all(body).await.context("write HTTP body")?;
    Ok(())
}

fn deadline_ms(timeout: Duration) -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let deadline = now.saturating_add(timeout);
    u64::try_from(deadline.as_millis()).unwrap_or(u64::MAX)
}

fn error_response(error: anyhow::Error) -> InvokeResponseWire {
    eprintln!("[rustack-lambda-agent] invocation failed: {error:#}");
    let body = serde_json::json!({
        "errorMessage": error.to_string(),
        "errorType": "RustackSquibAgentError",
    });
    InvokeResponseWire {
        protocol_version: PROTOCOL_VERSION,
        status: 200,
        payload_base64: STANDARD.encode(body.to_string()),
        function_error: Some("Unhandled".to_owned()),
        log_tail_base64: None,
        executed_version: "$LATEST".to_owned(),
    }
}
