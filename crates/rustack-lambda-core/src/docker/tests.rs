use std::{
    future::Future,
    task::{Context, Waker},
};

use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, duplex},
    net::{UnixListener, UnixStream},
};

use super::*;

#[derive(Debug, Clone)]
struct LogWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
impl Write for LogWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut output = self.0.lock().unwrap();
        if output.len().saturating_add(bytes.len()) > 2 * MAX_BODY_BYTES {
            return Err(io::Error::other("capture exceeded limit"));
        }
        output.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(3)
}

struct IncomingRequest {
    head: String,
    body: Vec<u8>,
    stream: UnixStream,
}
impl IncomingRequest {
    async fn reply(mut self, status: u16, body: &[u8]) {
        let head = format!(
            "HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        timeout_at(deadline(), async {
            self.stream.write_all(head.as_bytes()).await?;
            self.stream.write_all(body).await
        })
        .await
        .unwrap()
        .unwrap();
    }
    async fn raw(mut self, response: &[u8]) {
        timeout_at(deadline(), self.stream.write_all(response))
            .await
            .unwrap()
            .unwrap();
    }
}

struct Daemon {
    directory: TempDir,
    requests: mpsc::Receiver<IncomingRequest>,
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}
impl Daemon {
    fn new() -> Self {
        Self::with_version("1.53")
    }
    fn with_version(version: &'static str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(directory.path().join("docker.sock")).unwrap();
        let (requests, received) = mpsc::channel(32);
        let (stop, mut stopped) = watch::channel(false);
        let task = tokio::spawn(async move {
            let mut jobs = JoinSet::new();
            loop {
                tokio::select! {
                    biased;
                    () = connection::stopped(&mut stopped) => break,
                    job = jobs.join_next(), if !jobs.is_empty() => { job.unwrap().unwrap(); },
                    stream = listener.accept(), if jobs.len() < 16 => {
                        let requests = requests.clone();
                        let (stream, _) = stream.unwrap();
                        jobs.spawn(async move {
                            // Closing a fixture connection during shutdown is
                            // normal. Panics still fail the joined server task.
                            if let Ok(incoming) = read_request(stream).await {
                                if incoming.head.starts_with("GET /version ") {
                                    incoming.reply(200, format!(r#"{{"ApiVersion":"{version}","MinAPIVersion":"1.44"}}"#).as_bytes()).await;
                                } else { let _ = requests.send(incoming).await; }
                            }
                        });
                    }
                }
            }
            jobs.abort_all();
            while let Some(result) = jobs.join_next().await {
                if let Err(error) = result {
                    assert!(error.is_cancelled());
                }
            }
        });
        Self {
            directory,
            requests: received,
            stop,
            task,
        }
    }
    fn socket(&self) -> DockerSocket {
        format!(
            "unix://{}",
            self.directory.path().join("docker.sock").display()
        )
        .parse()
        .unwrap()
    }
    async fn wire(&self) -> DockerWire {
        match DockerWire::connect(self.socket(), deadline()).await {
            Ok(wire) => wire,
            Err(DockerError::Http(error)) => panic!("fixture HTTP startup failed: {error:?}"),
            Err(error) => panic!("fixture startup failed: {error}"),
        }
    }
    async fn next(&mut self) -> IncomingRequest {
        timeout_at(deadline(), self.requests.recv())
            .await
            .unwrap()
            .unwrap()
    }
    async fn close(self) {
        self.stop.send_replace(true);
        self.task.await.unwrap();
    }
}

async fn read_request(mut stream: UnixStream) -> io::Result<IncomingRequest> {
    timeout_at(deadline(), async {
        let mut input = Vec::new();
        let boundary = loop {
            if let Some(index) = input.windows(4).position(|value| value == b"\r\n\r\n") {
                break index + 4;
            }
            if input.len() > 16 * 1024 {
                return Err(io::Error::other("fixture header exceeded cap"));
            }
            let mut block = [0; 512];
            let count = stream.read(&mut block).await?;
            if count == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            input.extend_from_slice(block.get(..count).unwrap());
        };
        let head =
            String::from_utf8(input.get(..boundary).unwrap().to_vec()).map_err(io::Error::other)?;
        let length = head
            .lines()
            .find_map(|line| {
                line.split_once(':')
                    .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .map(|(_, value)| value.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        if length > MAX_BODY_BYTES {
            return Err(io::Error::other("fixture body exceeded cap"));
        }
        let mut body = input.get(boundary..).unwrap().to_vec();
        let missing = length
            .checked_sub(body.len())
            .ok_or_else(|| io::Error::other("unexpected pipelining"))?;
        let mut remainder = vec![0; missing];
        stream.read_exact(&mut remainder).await?;
        body.extend(remainder);
        Ok(IncomingRequest { head, body, stream })
    })
    .await
    .map_err(|_| io::Error::other("fixture deadline"))?
}

fn record(channel: u8, bytes: &[u8]) -> Vec<u8> {
    let mut output = vec![channel, 0, 0, 0];
    output.extend_from_slice(&u32::try_from(bytes.len()).unwrap().to_be_bytes());
    output.extend_from_slice(bytes);
    output
}

#[tokio::test]
async fn test_should_negotiate_bound_metadata_encode_paths_and_preserve_status() {
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let port = wire.port();
    assert_eq!(port.api_version().to_string(), "1.53");
    let (info, ()) = tokio::join!(port.info(deadline()), async {
        let request = daemon.next().await;
        assert!(request.head.starts_with("GET /v1.53/info "));
        request
            .reply(200, br#"{"ID":"daemon-id","OSType":"linux"}"#)
            .await;
    });
    assert_eq!(info.unwrap().id.as_deref(), Some("daemon-id"));
    let image: ImageReference = "localhost:5000/team/worker:v1".parse().unwrap();
    let (result, ()) = tokio::join!(port.inspect_image(&image, deadline()), async {
        let request = daemon.next().await;
        assert!(
            request
                .head
                .starts_with("GET /v1.53/images/localhost%3A5000%2Fteam%2Fworker%3Av1/json ")
        );
        request.reply(404, b"secret daemon error").await;
    });
    let error = result.unwrap_err();
    assert!(matches!(error, DockerError::Status(StatusCode::NOT_FOUND)));
    assert!(!format!("{error:?} {error}").contains("secret"));
    wire.shutdown().await.unwrap();
    assert!(matches!(
        port.info(deadline()).await,
        Err(DockerError::Stopped)
    ));
    daemon.close().await;
}

#[tokio::test]
async fn test_should_reject_oversized_truncated_chunked_trailer_and_duplicate_metadata() {
    for mode in 0..6 {
        let mut daemon = Daemon::new();
        let mut wire = daemon.wire().await;
        let port = wire.port();
        let (result, ()) = tokio::join!(port.info(deadline()), async {
            let request = daemon.next().await;
            match mode {
                0 => request.raw(b"HTTP/1.1 200 OK\r\nContent-Length: 1048577\r\n\r\n").await,
                1 => request.raw(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n{}").await,
                2 => request.raw(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\nSecret: trailer\r\n\r\n").await,
                3 => request.reply(200, br#"{"ID":"one","ID":"two"}"#).await,
                4 => request.raw(format!("HTTP/1.1 200 OK\r\nSecret: {}\r\nContent-Length: 2\r\n\r\n{{}}", "x".repeat(4097)).as_bytes()).await,
                _ => {
                    let mut response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
                    response.extend_from_slice(b"100001\r\n");
                    response.extend(std::iter::repeat_n(b'x', MAX_BODY_BYTES + 1));
                    response.extend_from_slice(b"\r\n0\r\n\r\n");
                    // The client closes as soon as its byte cap is reached.
                    let mut stream = request.stream;
                    let _ = timeout_at(deadline(), stream.write_all(&response)).await;
                }
            }
        });
        assert!(result.is_err(), "case {mode}");
        wire.shutdown().await.unwrap();
        daemon.close().await;
    }
}

#[tokio::test]
async fn test_should_send_typed_container_create_and_management_operations() {
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let port = wire.port();
    let key: DockerKey = "owned-test".parse().unwrap();
    let body = ContainerCreateBody {
        image: Some("sha256:fixture".into()),
        env: Some(vec!["SECRET=value".into()]),
        ..Default::default()
    };
    let (created, ()) = tokio::join!(port.create_container(&key, &body, deadline()), async {
        let request = daemon.next().await;
        assert!(
            request
                .head
                .starts_with("POST /v1.53/containers/create?name=owned-test ")
        );
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["Env"], serde_json::json!(["SECRET=value"]));
        request
            .reply(201, br#"{"Id":"owned-test","Warnings":[]}"#)
            .await;
    });
    assert_eq!(created.unwrap().id, "owned-test");
    for operation in 0..5 {
        let (result, ()) = tokio::join!(
            async {
                match operation {
                    0 => port.start_container(&key, deadline()).await,
                    1 => port
                        .inspect_container(&key, deadline())
                        .await
                        .map(|inspect| assert_eq!(inspect.id.as_deref(), Some("owned-test"))),
                    2 => port.kill_container(&key, deadline()).await,
                    3 => port
                        .wait_container(&key, deadline())
                        .await
                        .map(|wait| assert_eq!(wait.status_code, 0)),
                    _ => port.remove_container(&key, deadline()).await,
                }
            },
            async {
                let request = daemon.next().await;
                assert!(request.head.contains("/containers/owned%2Dtest"));
                match operation {
                    1 => request.reply(200, br#"{"Id":"owned-test"}"#).await,
                    3 => request.reply(200, br#"{"StatusCode":0}"#).await,
                    _ => request.reply(204, b"").await,
                }
            }
        );
        result.unwrap();
    }
    let filter = OwnerFilter::new(Uuid::new_v4(), Some(Uuid::new_v4()), None).unwrap();
    let (found, ()) = tokio::join!(port.list_containers(filter, deadline()), async {
        let request = daemon.next().await;
        assert!(request.head.contains("rustack.lambda.ownerId"));
        request.reply(200, b"[]").await;
    });
    assert_eq!(found.unwrap(), Vec::<ContainerSummary>::new());
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_bound_request_serialization_and_redact_request_debug() {
    let body = ContainerCreateBody {
        env: Some(vec!["TOP_SECRET=value".into()]),
        ..Default::default()
    };
    let request = RawRequest {
        operation: Operation::Create,
        method: Method::POST,
        target: "/secret-target".into(),
        body: encode(&body).unwrap(),
        expected: StatusCode::CREATED,
    };
    let shown = format!("{request:?}");
    assert!(!shown.contains("TOP_SECRET") && !shown.contains("secret-target"));
    let oversized = ContainerCreateBody {
        env: Some(vec!["x".repeat(MAX_BODY_BYTES)]),
        ..Default::default()
    };
    assert!(matches!(encode(&oversized), Err(DockerError::TooLarge)));
    assert!(
        !format!(
            "{:?}",
            Encoded {
                bytes: b"secret".to_vec(),
                failure: None
            }
        )
        .contains("secret")
    );
}

#[tokio::test]
async fn test_should_upgrade_handoff_and_decode_fragmented_binary_output_without_a_management_slot()
{
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let port = wire.port();
    let key: DockerKey = "relay-test".parse().unwrap();
    let (attached, mut server) = tokio::join!(port.attach(&key, deadline()), async {
        let mut request = daemon.next().await;
        assert!(request.head.contains("logs=false"));
        request
            .stream
            .write_all(
                b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: tcp\r\n\r\n",
            )
            .await
            .unwrap();
        request.stream
    });
    let attach = attached.unwrap();
    assert!(!format!("{attach:?}").contains("secret"));
    let (mut reader, mut writer) = attach.split(Duration::from_secs(1)).unwrap();
    let mut records = record(1, b"binary\0secret");
    records.extend(record(2, b"stderr secret"));
    let (chunks, ()) = tokio::join!(
        async {
            let first = reader.next(deadline()).await.unwrap().unwrap();
            let second = reader.next(deadline()).await.unwrap().unwrap();
            (first, second)
        },
        async {
            for byte in records {
                server.write_all(&[byte]).await.unwrap();
            }
        }
    );
    assert_eq!(chunks.0.bytes.as_ref(), b"binary\0secret");
    assert_eq!(chunks.1.channel, OutputChannel::Stderr);
    assert!(!format!("{:?} {reader:?}", chunks.0).contains("secret"));
    writer.write_all(b"raw stdin").await.unwrap();
    let mut stdin = [0; 9];
    server.read_exact(&mut stdin).await.unwrap();
    assert_eq!(&stdin, b"raw stdin");
    // Exported attach has no management driver left to block shutdown.
    wire.shutdown().await.unwrap();
    drop(reader);
    drop(writer);
    drop(server);
    daemon.close().await;
}

#[tokio::test]
async fn test_should_keep_cleanup_capacity_while_business_calls_and_mailbox_are_full() {
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let port = wire.port();
    let mut calls = JoinSet::new();
    for _ in 0..6 {
        let port = port.clone();
        calls.spawn(async move { port.info(deadline()).await });
    }
    let mut held = Vec::new();
    for _ in 0..6 {
        held.push(daemon.next().await);
    }
    let mut queued = Vec::new();
    for _ in 0..32 {
        let (reply, response) = oneshot::channel();
        port.raw
            .business
            .try_send(Envelope {
                request: RawRequest {
                    operation: Operation::Info,
                    method: Method::GET,
                    target: "/v1.53/info".into(),
                    body: Bytes::new(),
                    expected: StatusCode::OK,
                },
                deadline: deadline(),
                reply,
            })
            .unwrap();
        queued.push(response);
    }
    assert!(matches!(
        port.info(deadline()).await,
        Err(DockerError::Busy)
    ));
    let key: DockerKey = "owned-container".parse().unwrap();
    let (removed, ()) = tokio::join!(port.remove_container(&key, deadline()), async {
        let request = daemon.next().await;
        assert!(
            request
                .head
                .starts_with("DELETE /v1.53/containers/owned%2Dcontainer?")
        );
        request.reply(204, b"").await;
    });
    removed.unwrap();
    wire.shutdown().await.unwrap();
    for response in queued {
        assert!(matches!(response.await.unwrap(), Err(DockerError::Stopped)));
    }
    while let Some(result) = calls.join_next().await {
        assert!(result.unwrap().is_err());
    }
    for mut request in held {
        let mut byte = [0; 1];
        assert_eq!(
            timeout_at(deadline(), request.stream.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
    }
    daemon.close().await;
}

#[tokio::test]
async fn test_should_admit_kill_while_two_lifecycle_waits_are_blocked() {
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let port = wire.port();
    let mut calls = JoinSet::new();
    let key: DockerKey = "running-container".parse().unwrap();
    for _ in 0..2 {
        let port = port.clone();
        let key = key.clone();
        calls.spawn(async move { port.wait_container(&key, deadline()).await });
    }
    let mut held = Vec::new();
    for _ in 0..2 {
        let request = daemon.next().await;
        assert!(request.head.contains("/wait?condition=not-running"));
        held.push(request);
    }
    let (killed, ()) = tokio::join!(
        port.kill_container(&key, Instant::now() + Duration::from_secs(1)),
        async {
            let request = daemon.next().await;
            assert!(request.head.contains("/kill?signal=SIGKILL"));
            request.reply(204, b"").await;
        }
    );
    killed.unwrap();
    wire.shutdown().await.unwrap();
    while let Some(result) = calls.join_next().await {
        assert!(result.unwrap().is_err());
    }
    for mut request in held {
        let mut byte = [0; 1];
        assert_eq!(
            timeout_at(deadline(), request.stream.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
    }
    daemon.close().await;
}

#[tokio::test]
async fn test_should_retain_inflight_calls_after_waiter_drop_and_join_after_cancelled_shutdown() {
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let port = wire.port();
    let caller = tokio::spawn(async move { port.info(deadline()).await });
    let request = daemon.next().await;
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    // The accepted request still owns its driver even though its response
    // waiter is gone. Finish the body before stopping the retained wire owner.
    request.reply(200, b"{}").await;
    let mut shutdown = Box::pin(wire.shutdown());
    assert!(
        shutdown
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    drop(shutdown);
    wire.shutdown().await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_filter_secret_requests_responses_and_partial_trailers_under_trace() {
    let capture = LogWriter(std::sync::Arc::default());
    let writer = capture.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    async {
        tracing::debug!("application_event_is_preserved");
        let dispatch = runtime_dispatch();
        tracing::dispatcher::with_default(&dispatch, || {
            tracing::trace!(target: "bollard::docker", "secret_constructed_bollard");
        });
        let mut daemon = Daemon::new();
        let mut wire = daemon.wire().await;
        let port = wire.port();
        let key = "secret-request-target".parse().unwrap();
        let body = ContainerCreateBody { env: Some(vec!["SECRET_ENV=secret_request_body".into()]), ..Default::default() };
        let (created, ()) = tokio::join!(port.create_container(&key, &body, deadline()), async {
            let request = daemon.next().await;
            request.reply(201, br#"{"Id":"known","Warnings":["secret_response_body"]}"#).await;
        });
        assert_eq!(created.unwrap().id, "known");
        let (result, ()) = tokio::join!(port.info(deadline()), async {
            let request = daemon.next().await;
            let mut stream = request.stream;
            stream.write_all(b"HTTP/1.1 200 OK\r\nSecret: secret_response_header\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\nSecret: secret_partial_trailer").await.unwrap();
            // Force at least one partial trailer-buffer parse before finishing.
            tokio::task::yield_now().await;
            stream.write_all(b"\r\n\r\n").await.unwrap();
        });
        assert!(result.is_err());
        wire.shutdown().await.unwrap();
        daemon.close().await;
    }.with_subscriber(subscriber).await;
    let shown = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    assert!(shown.contains("application_event_is_preserved"));
    for secret in [
        "secret_constructed_bollard",
        "secret-request-target",
        "secret_request_body",
        "secret_response_body",
        "secret_response_header",
        "secret_partial_trailer",
    ] {
        assert!(
            !shown.contains(secret),
            "transport secret escaped the filter"
        );
    }
}

#[tokio::test]
async fn test_should_stream_large_records_in_bounded_chunks_and_reject_bad_outer_framing() {
    let data = record(1, &vec![7; 100_000]);
    let mut reader = DockerOutputReader::new(data.as_slice(), Duration::from_secs(1)).unwrap();
    let mut total = 0;
    while let Some(chunk) = reader.next(deadline()).await.unwrap() {
        assert!(chunk.bytes.len() <= 16 * 1024);
        total += chunk.bytes.len();
    }
    assert_eq!(total, 100_000);
    assert!(reader.next(deadline()).await.unwrap().is_none());
    for bytes in [
        vec![0, 0, 0, 0, 0, 0, 0, 0],
        vec![3, 0, 0, 0, 0, 0, 0, 0],
        vec![1, 1, 0, 0, 0, 0, 0, 0],
        vec![1, 0],
        vec![1, 0, 0, 0, 0xff, 0xff, 0xff, 0xff],
        vec![1, 0, 0, 0, 0, 0, 0, 1],
    ] {
        let mut reader = DockerOutputReader::new(bytes.as_slice(), Duration::from_secs(1)).unwrap();
        assert!(reader.next(deadline()).await.is_err());
        assert!(matches!(
            reader.next(deadline()).await,
            Err(DockerError::Poisoned)
        ));
    }
}

#[tokio::test(start_paused = true)]
async fn test_should_not_extend_partial_record_deadlines_across_chunks_or_restart_cancelled_reads()
{
    let data = record(1, &vec![9; 32 * 1024]);
    let mut reader = DockerOutputReader::new(data.as_slice(), Duration::from_secs(1)).unwrap();
    assert_eq!(
        reader.next(deadline()).await.unwrap().unwrap().bytes.len(),
        16 * 1024
    );
    tokio::time::advance(Duration::from_secs(2)).await;
    assert!(matches!(
        reader.next(deadline()).await,
        Err(DockerError::Deadline)
    ));
    let (mut input, source) = duplex(8);
    input.write_all(&[1]).await.unwrap();
    let mut reader = DockerOutputReader::new(source, Duration::from_secs(1)).unwrap();
    let mut waiting = Box::pin(reader.next(deadline()));
    assert!(
        waiting
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    drop(waiting);
    assert!(matches!(
        reader.next(deadline()).await,
        Err(DockerError::Poisoned)
    ));
}

#[test]
fn test_should_validate_explicit_sockets_identifiers_and_version_negotiation() {
    for socket in [
        "/var/run/docker.sock",
        "tcp://localhost:2375",
        "unix://relative",
        "unix:///bad\0socket",
    ] {
        assert!(socket.parse::<DockerSocket>().is_err());
    }
    for key in ["", "../bad", "path/name", "-bad", "space name"] {
        assert!(key.parse::<DockerKey>().is_err());
    }
    assert!("a".repeat(129).parse::<DockerKey>().is_err());
    assert_eq!(ApiVersion::negotiate("1.99", "1.44").unwrap().minor(), 53);
    assert!(ApiVersion::negotiate("1.43", "1.24").is_err());
    assert!(ApiVersion::negotiate("1.99", "1.54").is_err());
    for version in ["1.053", "2.53", "1.-1", "1.1000"] {
        assert!(version.parse::<ApiVersion>().is_err());
    }
    assert!(OwnerFilter::new(Uuid::nil(), None, None).is_err());
    assert!(OwnerFilter::new(Uuid::new_v4(), None, Some(Uuid::new_v4())).is_err());
}

#[tokio::test]
async fn test_should_close_a_timed_out_body_driver_without_stopping_the_wire() {
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let port = wire.port();
    let (result, ()) = tokio::join!(port.info(Instant::now() + Duration::from_secs(1)), async {
        let mut request = daemon.next().await;
        request
            .stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n")
            .await
            .unwrap();
        let mut byte = [0; 1];
        assert_eq!(
            timeout_at(deadline(), request.stream.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
    });
    assert!(matches!(result, Err(DockerError::Deadline)));
    let (next, ()) = tokio::join!(port.info(deadline()), async {
        daemon.next().await.reply(200, b"{}").await;
    });
    next.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_cache_supervision_failure_when_priority_stop_wins() {
    let (_business, requests) = mpsc::channel(32);
    let (_cleanup, priority) = mpsc::channel(8);
    let (_waits, waits) = mpsc::channel(8);
    let (control, _) = watch::channel(true);
    let mut actor = Actor::builder()
        .socket(PathBuf::from("/unused-fixture-socket"))
        .business(requests)
        .cleanup(priority)
        .waits(waits)
        .control(control.clone())
        .build();
    let failed = actor
        .jobs
        .spawn(async { panic!("controlled wire worker failure") });
    while !failed.is_finished() {
        tokio::task::yield_now().await;
    }
    let task = tokio::spawn(actor.run());
    let mut owner = Owner {
        stop: control,
        task: Some(task),
        settlement: None,
    };
    assert!(matches!(owner.shutdown().await, Err(DockerError::Worker)));
    assert!(matches!(owner.shutdown().await, Err(DockerError::Worker)));
}

#[tokio::test]
async fn test_should_reject_expired_and_excessive_deadlines_before_metadata_publication() {
    assert!(matches!(
        check_deadline(Instant::now()),
        Err(DockerError::Deadline)
    ));
    assert!(matches!(
        check_deadline(Instant::now() + Duration::from_secs(601)),
        Err(DockerError::Configuration)
    ));
    let response = RawReply::Body {
        status: StatusCode::OK,
        bytes: Bytes::from_static(b"{}"),
    };
    let decoded: Result<SystemInfo, DockerError> = decode(response, Instant::now());
    assert!(matches!(decoded, Err(DockerError::Deadline)));
}

#[rstest::rstest]
#[case("1.44", false)]
#[case("1.48", false)]
#[case("1.49", true)]
#[case("1.53", true)]
#[tokio::test]
async fn test_should_resolve_preloaded_images_with_versioned_platform_selection(
    #[case] version: &'static str,
    #[case] platform: bool,
) {
    use crate::artifact::Architecture;
    let mut daemon = Daemon::with_version(version);
    let mut wire = daemon.wire().await;
    let resolver = ImageResolver::new(wire.port());
    let reference = "registry.test:5000/team/worker:v1".parse().unwrap();
    let invalid_override = rustack_lambda_model::types::ImageConfig {
        working_directory: Some("relative".into()),
        ..Default::default()
    };
    assert!(matches!(
        resolver
            .resolve_local(
                &reference,
                Architecture::Arm64,
                Some(&invalid_override),
                deadline()
            )
            .await,
        Err(DockerError::Configuration)
    ));
    assert!(daemon.requests.try_recv().is_err());
    let (image, ()) = tokio::join!(
        resolver.resolve_local(&reference, Architecture::Arm64, None, deadline()),
        async {
            let request = daemon.next().await;
            let target = request.head.split_whitespace().nth(1).unwrap();
            assert_eq!(
                target,
                format!("/v{version}/images/registry%2Etest%3A5000%2Fteam%2Fworker%3Av1/json")
            );
            let id = format!("sha256:{}", "a".repeat(64));
            let body = serde_json::json!({"Id":id,"Os":"linux","Architecture":"arm64","Size":1024,"Config":{"Cmd":["/start"]},"GraphDriver":{"Name":"overlay2","Data":{}}});
            request
                .reply(200, &serde_json::to_vec(&body).unwrap())
                .await;
            if platform {
                let request = daemon.next().await;
                let target = request.head.split_whitespace().nth(1).unwrap();
                let (path, query) = target.split_once('?').unwrap();
                assert_eq!(
                    path,
                    format!("/v{version}/images/sha256%3A{}/json", "a".repeat(64))
                );
                let pairs: Vec<_> = form_urlencoded::parse(query.as_bytes()).collect();
                assert_eq!(pairs.len(), 1);
                assert_eq!(pairs.first().unwrap().0, "platform");
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(&pairs.first().unwrap().1).unwrap(),
                    serde_json::json!({"os":"linux","architecture":"arm64"})
                );
                let mut selected = body;
                selected["Id"] = serde_json::json!(format!("sha256:{}", "b".repeat(64)));
                selected["Descriptor"] = serde_json::json!({"mediaType":"application/vnd.oci.image.manifest.v1+json","digest":format!("sha256:{}","b".repeat(64))});
                request
                    .reply(200, &serde_json::to_vec(&selected).unwrap())
                    .await;
            } else {
                let request = daemon.next().await;
                assert!(request.head.starts_with(&format!("GET /v{version}/info ")));
                request
                    .reply(
                        200,
                        br#"{"Driver":"overlay2","DriverStatus":[["Backing Filesystem","extfs"]]}"#,
                    )
                    .await;
            }
        }
    );
    let image = image.unwrap();
    assert_eq!(image.metadata().architecture, Architecture::Arm64);
    assert_eq!(
        image.metadata().image_id.as_str(),
        format!("sha256:{}", "a".repeat(64))
    );
    assert_eq!(
        image.metadata().execution.plan(None).unwrap().entrypoint(),
        [""]
    );
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
