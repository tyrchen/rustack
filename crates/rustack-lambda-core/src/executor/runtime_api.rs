//! In-process Lambda Runtime API server.
//!
//! Each warm runtime instance (process or container) gets its own socket bound
//! on `127.0.0.1:0`. The bootstrap inside the instance speaks the standard
//! Lambda Runtime API to it:
//!
//! - `GET  /2018-06-01/runtime/invocation/next` — long-poll for the next job.
//! - `POST /2018-06-01/runtime/invocation/{id}/response` — deliver success.
//! - `POST /2018-06-01/runtime/invocation/{id}/error` — deliver function error.
//! - `POST /2018-06-01/runtime/init/error` — fatal init failure.
//!
//! Per AWS, an instance handles one invocation at a time. The receiver is
//! locked once per `/next` call so concurrent polls (which shouldn't happen
//! in practice) serialize cleanly.

use std::{
    convert::Infallible,
    net::SocketAddr,
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use bytes::Bytes;
use dashmap::DashMap;
use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    Method, Request, Response, StatusCode, body::Incoming, server::conn::http1, service::service_fn,
};
use hyper_util::rt::TokioIo;
use tokio::{
    net::TcpListener,
    sync::{Mutex, mpsc, oneshot, watch},
    task::{JoinHandle, JoinSet},
};
use tracing::{debug, warn};

/// One unit of work for a runtime instance: payload in, result out.
#[derive(Debug)]
pub struct Job {
    /// Synthetic AWS request id (UUID).
    pub request_id: String,
    /// Function ARN reported via `Lambda-Runtime-Invoked-Function-Arn`.
    pub function_arn: String,
    /// Wall-clock deadline (used for `Lambda-Runtime-Deadline-Ms`).
    pub deadline: Instant,
    /// Body delivered as the invocation event.
    pub payload: Bytes,
    /// Where to send the bootstrap's response (or error).
    pub response_tx: oneshot::Sender<RuntimeResult>,
}

/// Result of a runtime round-trip.
#[derive(Debug, Clone)]
pub enum RuntimeResult {
    /// Bootstrap posted to `/response`.
    Success(Bytes),
    /// Bootstrap posted to `/error`.
    Error(Bytes),
    /// Bootstrap posted to `/init/error` before invocation.
    InitError(Bytes),
}

/// Handle to a running runtime API socket. Drop or call `shutdown` to stop.
#[derive(Debug)]
pub struct RuntimeApiHandle {
    addr: SocketAddr,
    job_tx: mpsc::Sender<Job>,
    shutdown_tx: watch::Sender<bool>,
    accept_task: Option<JoinHandle<()>>,
    /// Optional channel that the orchestrator subscribes to in order to be
    /// notified of `/init/error` failures.
    init_error_rx: Mutex<Option<oneshot::Receiver<Bytes>>>,
}

impl RuntimeApiHandle {
    /// Bound socket address.
    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Submit a new invocation job. Fails if the runtime task has stopped.
    pub async fn submit(&self, job: Job) -> Result<(), &'static str> {
        self.job_tx
            .send(job)
            .await
            .map_err(|_| "runtime api task is gone")
    }

    /// Take the init-error receiver. Returns `Some` exactly once.
    pub async fn take_init_error_rx(&self) -> Option<oneshot::Receiver<Bytes>> {
        self.init_error_rx.lock().await.take()
    }

    /// Stop the accept loop and wait for it to exit.
    pub async fn shutdown(&mut self) {
        let _ = self.shutdown_tx.send(true);
        if let Some(task) = self.accept_task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for RuntimeApiHandle {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(true);
        if let Some(task) = self.accept_task.take() {
            task.abort();
        }
    }
}

/// State shared across connection handlers.
#[derive(Debug)]
struct State {
    /// Single-consumer queue of pending jobs.
    job_rx: Mutex<mpsc::Receiver<Job>>,
    /// `request_id` -> `oneshot::Sender<RuntimeResult>` for the in-flight job.
    pending: DashMap<String, oneshot::Sender<RuntimeResult>>,
    /// One-shot to signal an init-error to the orchestrator.
    init_error_tx: Mutex<Option<oneshot::Sender<Bytes>>>,
}

/// Bind a fresh runtime API socket and return a handle.
///
/// The bootstrap process should be started **after** this returns, with
/// `AWS_LAMBDA_RUNTIME_API` set to `handle.addr()`.
pub async fn start() -> std::io::Result<RuntimeApiHandle> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let addr = listener.local_addr()?;

    let (job_tx, job_rx) = mpsc::channel::<Job>(8);
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let (init_err_tx, init_err_rx) = oneshot::channel::<Bytes>();

    let state = Arc::new(State {
        job_rx: Mutex::new(job_rx),
        pending: DashMap::new(),
        init_error_tx: Mutex::new(Some(init_err_tx)),
    });

    let accept_task = tokio::spawn(async move {
        let mut connections = JoinSet::new();
        loop {
            tokio::select! {
                result = connections.join_next(), if !connections.is_empty() => { if let Some(Err(error)) = result { warn!(%error, "Runtime API connection task failed"); } }
                _ = shutdown_rx.changed() => {
                    debug!("runtime api: shutdown signalled, exiting accept loop");
                    break;
                }
                accepted = listener.accept() => {
                    let (stream, _peer) = match accepted {
                        Ok(c) => c,
                        Err(e) => {
                            warn!(error = %e, "runtime api accept failed");
                            continue;
                        }
                    };
                    if connections.len() >= 4 { drop(stream); continue; }
                    let state = Arc::clone(&state);
                    connections.spawn(async move {
                        let io = TokioIo::new(stream);
                        let svc = service_fn(move |req| {
                            let state = Arc::clone(&state);
                            async move { handle(state, req).await }
                        });
                        if let Err(e) = http1::Builder::new()
                            .serve_connection(io, svc)
                            .await
                        {
                            // EOF / reset is expected when the bootstrap exits.
                            debug!(error = %e, "runtime api connection closed");
                        }
                    });
                }
            }
        }
        connections.abort_all();
        while let Some(result) = connections.join_next().await {
            if let Err(error) = result {
                debug!(%error, "Runtime API connection stopped");
            }
        }
    });

    Ok(RuntimeApiHandle {
        addr,
        job_tx,
        shutdown_tx,
        accept_task: Some(accept_task),
        init_error_rx: Mutex::new(Some(init_err_rx)),
    })
}

/// Service entry point.
async fn handle(
    state: Arc<State>,
    req: Request<Incoming>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let method = req.method().clone();
    let path = req.uri().path().to_owned();
    debug!(%method, %path, "runtime api request");

    let result = match (method, path.as_str()) {
        (Method::GET, "/2018-06-01/runtime/invocation/next") => handle_next(&state).await,
        (Method::POST, p)
            if p.starts_with("/2018-06-01/runtime/invocation/") && p.ends_with("/response") =>
        {
            handle_complete(&state, &path, req, RuntimeResult::Success).await
        }
        (Method::POST, p)
            if p.starts_with("/2018-06-01/runtime/invocation/") && p.ends_with("/error") =>
        {
            handle_complete(&state, &path, req, RuntimeResult::Error).await
        }
        (Method::POST, "/2018-06-01/runtime/init/error") => handle_init_error(&state, req).await,
        _ => Ok(simple(StatusCode::NOT_FOUND, b"not found".as_ref())),
    };

    Ok(result.unwrap_or_else(|status| simple(status, b"".as_ref())))
}

/// Long-poll for the next job. Returns once a job is queued or shutdown
/// drops the channel.
async fn handle_next(state: &Arc<State>) -> Result<Response<Full<Bytes>>, StatusCode> {
    let mut rx = state.job_rx.lock().await;
    let Some(job) = rx.recv().await else {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    };
    drop(rx); // release before doing any work

    let Job {
        request_id,
        function_arn,
        deadline,
        payload,
        response_tx,
    } = job;

    state.pending.insert(request_id.clone(), response_tx);

    let deadline_ms = deadline_unix_ms(deadline);
    // X-Ray-shaped trace id for runtime SDKs that expect the header to be
    // present. Format isn't validated downstream — a UUID-derived value is
    // fine for local emulation.
    let trace_id = format!(
        "Root=1-{:08x}-{};Parent={};Sampled=1",
        unix_secs_lower32(),
        uuid::Uuid::new_v4().simple(),
        &uuid::Uuid::new_v4().simple().to_string()[..16],
    );

    let resp = Response::builder()
        .status(StatusCode::OK)
        .header("Lambda-Runtime-Aws-Request-Id", &request_id)
        .header("Lambda-Runtime-Deadline-Ms", deadline_ms.to_string())
        .header("Lambda-Runtime-Invoked-Function-Arn", &function_arn)
        .header("Lambda-Runtime-Trace-Id", trace_id)
        .header("Content-Type", "application/json")
        .body(Full::new(payload))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(resp)
}

/// Handle `/response` or `/error` — fire the matching oneshot, return 202.
async fn handle_complete<F>(
    state: &Arc<State>,
    path: &str,
    req: Request<Incoming>,
    map_result: F,
) -> Result<Response<Full<Bytes>>, StatusCode>
where
    F: FnOnce(Bytes) -> RuntimeResult,
{
    let request_id = parse_request_id(path).ok_or(StatusCode::BAD_REQUEST)?;
    let body = collect(req.into_body()).await?;
    if let Some((_, sender)) = state.pending.remove(&request_id) {
        let _ = sender.send(map_result(body));
        Ok(simple(StatusCode::ACCEPTED, b"".as_ref()))
    } else {
        Ok(simple(
            StatusCode::BAD_REQUEST,
            b"unknown request id".as_ref(),
        ))
    }
}

async fn handle_init_error(
    state: &Arc<State>,
    req: Request<Incoming>,
) -> Result<Response<Full<Bytes>>, StatusCode> {
    let body = collect(req.into_body()).await?;
    if let Some(tx) = state.init_error_tx.lock().await.take() {
        let _ = tx.send(body);
    }
    Ok(simple(StatusCode::ACCEPTED, b"".as_ref()))
}

fn parse_request_id(path: &str) -> Option<String> {
    let after = path.strip_prefix("/2018-06-01/runtime/invocation/")?;
    let id = after.split('/').next()?;
    if id.is_empty() {
        None
    } else {
        Some(id.to_owned())
    }
}

fn deadline_unix_ms(deadline: Instant) -> u64 {
    let now = Instant::now();
    let remaining = deadline.saturating_duration_since(now);
    let now_ms = u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX);
    now_ms.saturating_add(u64::try_from(remaining.as_millis()).unwrap_or(0))
}

#[allow(clippy::cast_possible_truncation)]
fn unix_secs_lower32() -> u32 {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Wrapping is fine — the trace id is opaque to consumers.
    (secs & 0xFFFF_FFFF) as u32
}

async fn collect(body: Incoming) -> Result<Bytes, StatusCode> {
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        Limited::new(body, 6 * 1024 * 1024).collect(),
    )
    .await
    .map_err(|_| StatusCode::REQUEST_TIMEOUT)?
    .map(http_body_util::Collected::to_bytes)
    .map_err(|error| {
        if error.is::<http_body_util::LengthLimitError>() {
            StatusCode::PAYLOAD_TOO_LARGE
        } else {
            StatusCode::BAD_REQUEST
        }
    })
}

fn simple(status: StatusCode, body: &[u8]) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .body(Full::new(Bytes::copy_from_slice(body)))
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::from_static(b""))))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpStream,
    };

    use super::*;

    /// Send a raw HTTP/1.1 request, return (status_line, headers_block, body_bytes).
    async fn http_request(
        addr: SocketAddr,
        method: &str,
        path: &str,
        body: &[u8],
    ) -> (String, String, Vec<u8>) {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Length: {len}\r\nConnection: \
             close\r\n\r\n",
            len = body.len(),
        );
        stream.write_all(req.as_bytes()).await.unwrap();
        if !body.is_empty() {
            stream.write_all(body).await.unwrap();
        }
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();

        // Split status / headers / body manually.
        let split = buf
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("header terminator");
        let head_text = String::from_utf8_lossy(&buf[..split]).to_string();
        let body_bytes = buf[split + 4..].to_vec();
        let mut lines = head_text.split("\r\n");
        let status = lines.next().unwrap_or("").to_owned();
        let headers = lines.collect::<Vec<_>>().join("\r\n");
        (status, headers, body_bytes)
    }

    fn header_value(headers: &str, name: &str) -> Option<String> {
        let needle = name.to_ascii_lowercase();
        headers.split("\r\n").find_map(|line| {
            let (k, v) = line.split_once(':')?;
            if k.trim().to_ascii_lowercase() == needle {
                Some(v.trim().to_owned())
            } else {
                None
            }
        })
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_should_round_trip_invocation_via_runtime_api() {
        let handle = start().await.unwrap();
        let addr = handle.addr();

        let (resp_tx, resp_rx) = oneshot::channel();
        let job = Job {
            request_id: "req-1".to_owned(),
            function_arn: "arn:fake".to_owned(),
            deadline: Instant::now() + Duration::from_secs(3),
            payload: Bytes::from_static(b"{\"hi\":1}"),
            response_tx: resp_tx,
        };
        handle.submit(job).await.unwrap();

        let bootstrap = tokio::spawn(async move {
            let (status, headers, body) =
                http_request(addr, "GET", "/2018-06-01/runtime/invocation/next", b"").await;
            assert!(status.starts_with("HTTP/1.1 200"), "status: {status}");
            let request_id =
                header_value(&headers, "Lambda-Runtime-Aws-Request-Id").expect("request id header");
            assert_eq!(request_id, "req-1");
            assert_eq!(&body[..], b"{\"hi\":1}");

            let path = format!("/2018-06-01/runtime/invocation/{request_id}/response");
            let (status, _h, _b) = http_request(addr, "POST", &path, b"{\"echo\":1}").await;
            assert!(status.starts_with("HTTP/1.1 202"), "post status: {status}");
        });

        let result = tokio::time::timeout(Duration::from_secs(3), resp_rx)
            .await
            .expect("oneshot timed out")
            .unwrap();
        match result {
            RuntimeResult::Success(b) => assert_eq!(&b[..], b"{\"echo\":1}"),
            other => panic!("expected Success, got {other:?}"),
        }
        bootstrap.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_should_propagate_function_error() {
        let handle = start().await.unwrap();
        let addr = handle.addr();

        let (resp_tx, resp_rx) = oneshot::channel();
        handle
            .submit(Job {
                request_id: "req-2".to_owned(),
                function_arn: "arn:fake".to_owned(),
                deadline: Instant::now() + Duration::from_secs(3),
                payload: Bytes::from_static(b"{}"),
                response_tx: resp_tx,
            })
            .await
            .unwrap();

        tokio::spawn(async move {
            let (_s, headers, _b) =
                http_request(addr, "GET", "/2018-06-01/runtime/invocation/next", b"").await;
            let id = header_value(&headers, "Lambda-Runtime-Aws-Request-Id").unwrap();
            let path = format!("/2018-06-01/runtime/invocation/{id}/error");
            http_request(
                addr,
                "POST",
                &path,
                br#"{"errorMessage":"boom","errorType":"TestError"}"#,
            )
            .await;
        });

        let result = tokio::time::timeout(Duration::from_secs(3), resp_rx)
            .await
            .unwrap()
            .unwrap();
        match result {
            RuntimeResult::Error(b) => {
                let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
                assert_eq!(v["errorType"], "TestError");
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_should_capture_init_error() {
        let handle = start().await.unwrap();
        let addr = handle.addr();
        let init_rx = handle.take_init_error_rx().await.unwrap();

        tokio::spawn(async move {
            http_request(
                addr,
                "POST",
                "/2018-06-01/runtime/init/error",
                br#"{"errorMessage":"init failed"}"#,
            )
            .await;
        });
        let body = tokio::time::timeout(Duration::from_secs(3), init_rx)
            .await
            .unwrap()
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["errorMessage"], "init failed");
    }
}
