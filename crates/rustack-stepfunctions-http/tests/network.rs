//! Actual HTTP framing and ordinary SDK qualification of the transport only.
#![forbid(unsafe_code)]

use std::{
    net::SocketAddr,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use hyper::server::conn::http1;
use hyper_util::rt::TokioIo;
use rustack_auth::StaticCredentialProvider;
use rustack_stepfunctions_http::{
    Request, RequestContext, RequestScope, Response, StepFunctionsHandler, StepFunctionsHttpConfig,
    StepFunctionsHttpService,
};
use rustack_stepfunctions_model::{StepFunctionsError, StepFunctionsErrorCode, output};
use serde_json::Value;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    process::Command,
    sync::watch,
    task::{JoinHandle, JoinSet},
    time::timeout,
};

#[derive(Default)]
struct Fixture {
    calls: AtomicUsize,
    authenticated: AtomicUsize,
}
#[async_trait]
impl StepFunctionsHandler for Fixture {
    async fn handle(
        &self,
        context: RequestContext,
        request: Request,
    ) -> Result<Response, StepFunctionsError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if context.authenticated() {
            self.authenticated.fetch_add(1, Ordering::SeqCst);
        }
        match request {
            Request::ListStateMachines(_) => Ok(output::ListStateMachinesOutput::default().into()),
            Request::StartExecution(value) => {
                assert_eq!(value.input.as_deref(), Some(" {\"n\":7}"));
                assert_eq!(value.trace_header.as_deref(), Some(""));
                let mut response = output::StartExecutionOutput::default();
                response.execution_arn =
                    "arn:aws:states:us-east-1:000000000000:execution:wire:execution".into();
                response.start_date = chrono::DateTime::from_timestamp_millis(-1250).unwrap();
                Ok(response.into())
            }
            Request::DescribeExecution(_) => Err(StepFunctionsError::with_message(
                StepFunctionsErrorCode::ExecutionDoesNotExist,
                "fixture missing",
            )),
            _ => Err(StepFunctionsError::new(
                StepFunctionsErrorCode::UnsupportedOperation,
            )),
        }
    }
}
struct Server {
    address: SocketAddr,
    stop: watch::Sender<bool>,
    task: Option<JoinHandle<Result<()>>>,
}
impl Server {
    async fn start(handler: Arc<Fixture>, strict: bool) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let scope = RequestScope::new("aws".into(), "us-east-1".into(), "000000000000".into())?;
        let credentials = Arc::new(StaticCredentialProvider::new(vec![(
            "local-key".into(),
            "local-secret".into(),
        )]));
        let service = StepFunctionsHttpService::new(
            handler,
            StepFunctionsHttpConfig::new(scope, !strict, Some(credentials))?,
        );
        let (stop, mut stopped) = watch::channel(false);
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            let primary = loop {
                tokio::select! {
                    biased;
                    _=stopped.changed()=>break Ok(()),
                    joined=connections.join_next(),if !connections.is_empty()=>match joined {
                        Some(Ok(Ok(()))) | None=>{},Some(Ok(Err(error)))=>break Err(error),Some(Err(error))=>break Err(error.into()),
                    },
                    accepted=listener.accept(),if connections.len()<8=>match accepted {
                        Ok((socket,_))=>{let service=service.clone();connections.spawn(async move {
                            timeout(Duration::from_secs(15),http1::Builder::new().serve_connection(TokioIo::new(socket),service)).await.context("connection deadline")?.context("HTTP connection")
                        });},
                        Err(error)=>break Err(error.into()),
                    },
                }
            };
            connections.abort_all();
            let mut cleanup = Ok(());
            while let Some(joined) = connections.join_next().await {
                match joined {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => cleanup = Err(error),
                    Err(error) if error.is_cancelled() => {}
                    Err(error) => cleanup = Err(error.into()),
                }
            }
            primary.and(cleanup)
        });
        Ok(Self {
            address,
            stop,
            task: Some(task),
        })
    }
    async fn shutdown(&mut self) -> Result<()> {
        self.stop.send_replace(true);
        let Some(mut task) = self.task.take() else {
            return Ok(());
        };
        match timeout(Duration::from_secs(10), &mut task).await {
            Ok(joined) => joined?,
            Err(error) => {
                // Keep the signaled root and its connection JoinSet alive.
                // A waiter deadline is not permission to lose drain ownership.
                self.task = Some(task);
                Err(error.into())
            }
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.send_replace(true);
    }
}

#[tokio::test(start_paused = true)]
async fn test_should_retain_server_join_ownership_after_cleanup_waiter_expiry() -> Result<()> {
    let (stop, mut stopped) = watch::channel(false);
    let (release, held) = tokio::sync::oneshot::channel();
    let joined = Arc::new(AtomicUsize::new(0));
    let completed = joined.clone();
    let task = tokio::spawn(async move {
        let mut connections = JoinSet::new();
        connections.spawn(std::future::pending::<()>());
        let _ = stopped.changed().await;
        held.await.context("release drain")?;
        connections.abort_all();
        while connections.join_next().await.is_some() {}
        completed.store(1, Ordering::SeqCst);
        Ok(())
    });
    let id = task.id();
    let mut server = Server {
        address: "127.0.0.1:1".parse()?,
        stop,
        task: Some(task),
    };
    ensure!(
        server.shutdown().await.is_err(),
        "held drain did not expire"
    );
    ensure!(
        server.task.as_ref().context("retained root")?.id() == id,
        "root changed after timeout"
    );
    ensure!(
        joined.load(Ordering::SeqCst) == 0,
        "drain completed before release"
    );
    release
        .send(())
        .map_err(|()| anyhow::anyhow!("drain receiver lost"))?;
    server.shutdown().await?;
    ensure!(
        joined.load(Ordering::SeqCst) == 1,
        "connection handles were not joined"
    );
    Ok(())
}

#[tokio::test]
async fn test_should_reject_actual_chunked_trailers_without_dispatching() -> Result<()> {
    let handler = Arc::new(Fixture::default());
    let mut server = Server::start(handler.clone(), false).await?;
    let result = async {
        let mut socket = TcpStream::connect(server.address).await?;
        let request = format!(
            "POST / HTTP/1.1\r\nHost: localhost\r\nContent-Type: \
             application/x-amz-json-1.0\r\nX-Amz-Target: \
             AWSStepFunctions.ListStateMachines\r\nTransfer-Encoding: chunked\r\nTrailer: \
             x-opaque\r\nConnection: close\r\n\r\n2\r\n{{}}\r\n0\r\nx-opaque: {}\r\n\r\n",
            "s".repeat(9000)
        );
        timeout(Duration::from_secs(5), socket.write_all(request.as_bytes())).await??;
        let mut response = Vec::new();
        timeout(
            Duration::from_secs(5),
            socket.take(65536).read_to_end(&mut response),
        )
        .await??;
        let split = response
            .windows(4)
            .position(|bytes| bytes == b"\r\n\r\n")
            .context("HTTP response headers")?;
        ensure!(
            response.starts_with(b"HTTP/1.1 400"),
            "trailer did not receive400"
        );
        let value: Value = serde_json::from_slice(response.get(split + 4..).context("body")?)?;
        ensure!(
            value["__type"] == "MalformedHttpRequestException",
            "wrong trailer error"
        );
        ensure!(
            handler.calls.load(Ordering::SeqCst) == 0,
            "trailers reached handler"
        );
        anyhow::Ok(())
    }
    .await;
    let cleanup = server.shutdown().await;
    result.and(cleanup)
}
#[tokio::test]
#[ignore = "requires uv and pinned ordinary boto3 fixture"]
async fn test_should_exchange_signed_boto3_requests_with_typed_transport() -> Result<()> {
    let handler = Arc::new(Fixture::default());
    let mut server = Server::start(handler.clone(), true).await?;
    let result = async {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut command = Command::new("uv");
        command
            .args([
                "run",
                "--no-project",
                "--python",
                "3.13",
                "--with-requirements",
            ])
            .arg(root.join("tests/workflows/requirements.txt"))
            .arg("python")
            .arg(root.join("tests/workflows/stepfunctions_http_sdk.py"))
            .arg(format!("http://{}", server.address))
            .env("AWS_EC2_METADATA_DISABLED", "true")
            .env("NO_PROXY", "127.0.0.1,localhost")
            .kill_on_drop(true);
        let mut child = command.spawn().context("start pinned SDK")?;
        let status = match timeout(Duration::from_secs(60), child.wait()).await {
            Ok(status) => status?,
            Err(error) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(error.into());
            }
        };
        ensure!(status.success(), "SDK fixture failed: {status}");
        ensure!(
            handler.calls.load(Ordering::SeqCst) == 3,
            "wrong dispatch count"
        );
        ensure!(
            handler.authenticated.load(Ordering::SeqCst) == 3,
            "SDK calls not authenticated"
        );
        anyhow::Ok(())
    }
    .await;
    let cleanup = server.shutdown().await;
    result.and(cleanup)
}
