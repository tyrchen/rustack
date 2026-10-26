use std::{
    future::Future,
    task::{Context, Waker},
};

use rstest::rstest;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

use super::*;
use crate::{metadata::local_trace_id, session::RuntimeResult};

#[derive(Debug)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

#[derive(Debug, Clone)]
struct LogWriter(Arc<std::sync::Mutex<Vec<u8>>>);

impl io::Write for LogWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut output = self.0.lock().unwrap();
        if output.len().saturating_add(bytes.len()) > 2 * 1024 * 1024 {
            return Err(io::Error::other("log capture capacity exceeded"));
        }
        output.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find_map(|(key, value)| key.eq_ignore_ascii_case(name).then_some(value.as_str()))
    }
}

async fn raw(addr: SocketAddr, request: Vec<u8>) -> Reply {
    timeout(Duration::from_secs(3), async {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream.write_all(&request).await.unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).await.unwrap();
        let position = bytes
            .windows(4)
            .position(|bytes| bytes == b"\r\n\r\n")
            .unwrap();
        let headers = String::from_utf8(bytes.get(..position).unwrap().to_vec()).unwrap();
        let mut lines = headers.lines();
        let status = lines
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let headers = lines
            .map(|line| {
                let (key, value) = line.split_once(':').unwrap();
                (key.into(), value.trim().into())
            })
            .collect();
        Reply {
            status,
            headers,
            body: bytes.get(position + 4..).unwrap().to_vec(),
        }
    })
    .await
    .unwrap()
}

pub(crate) async fn request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &str,
    body: &[u8],
) -> Reply {
    let mut bytes = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nContent-Length: \
         {}\r\n{headers}\r\n",
        body.len()
    )
    .into_bytes();
    bytes.extend_from_slice(body);
    raw(addr, bytes).await
}

fn invocation() -> Invocation {
    Invocation::new(
        AttemptId::default(),
        Instant::now() + Duration::from_secs(10),
        "arn:aws:lambda:us-east-1:000000000000:function:echo:prod"
            .parse()
            .unwrap(),
        local_trace_id().unwrap(),
        Payload::try_from(Bytes::from_static(b"secret-event\0\xff")).unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn test_should_round_trip_binary_invocations_and_supply_complete_runtime_metadata() {
    let mut api = start().await.unwrap();
    let call = invocation();
    let ticket = api
        .session()
        .submit(call.clone(), call.parts().1)
        .await
        .unwrap();
    let response = request(
        api.addr(),
        "GET",
        "/2018-06-01/runtime/invocation/next",
        "",
        b"",
    )
    .await;
    assert_eq!(response.status, 200);
    assert_eq!(response.body, call.parts().4.as_ref());
    assert_eq!(
        response.header("Lambda-Runtime-Aws-Request-Id").unwrap(),
        ticket.attempt().to_string()
    );
    assert_eq!(
        response.header(INVOCATION_ID).unwrap(),
        ticket.attempt().to_string()
    );
    assert_eq!(
        response.header("Lambda-Runtime-Invoked-Function-Arn"),
        Some("arn:aws:lambda:us-east-1:000000000000:function:echo:prod")
    );
    assert!(
        response
            .header("Lambda-Runtime-Trace-Id")
            .unwrap()
            .ends_with("Sampled=0")
    );
    assert!(
        response
            .header("Lambda-Runtime-Deadline-Ms")
            .unwrap()
            .parse::<u64>()
            .unwrap()
            > 0
    );
    let path = format!(
        "/2018-06-01/runtime/invocation/{}/response",
        ticket.attempt()
    );
    let result = request(
        api.addr(),
        "POST",
        &path,
        &format!("Lambda-Runtime-Invocation-Id: {}\r\n", ticket.attempt()),
        b"secret-result\0\xff",
    )
    .await;
    assert_eq!(result.status, 202);
    assert!(
        matches!(ticket.wait().await.unwrap(), RuntimeResult::Success(payload) if payload.as_ref() == b"secret-result\0\xff")
    );
    api.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_should_preserve_current_attempt_after_unknown_duplicate_or_wrong_header_completions()
{
    let mut api = start().await.unwrap();
    let call = invocation();
    let ticket = api
        .session()
        .submit(call.clone(), call.parts().1)
        .await
        .unwrap();
    request(
        api.addr(),
        "GET",
        "/2018-06-01/runtime/invocation/next",
        "",
        b"",
    )
    .await;
    let path = format!("/2018-06-01/runtime/invocation/{}/error", ticket.attempt());
    let mismatch = request(
        api.addr(),
        "POST",
        &path,
        &format!("Lambda-Runtime-Invocation-Id: {}\r\n", AttemptId::default()),
        b"{}",
    )
    .await;
    assert_eq!(mismatch.status, 400);
    assert!(
        String::from_utf8(mismatch.body)
            .unwrap()
            .contains("InvalidInvocationId")
    );
    let duplicate_headers = request(
        api.addr(),
        "POST",
        &path,
        &format!(
            "Lambda-Runtime-Invocation-Id: {}\r\nLambda-Runtime-Invocation-Id: {}\r\n",
            ticket.attempt(),
            ticket.attempt()
        ),
        b"{}",
    )
    .await;
    assert_eq!(duplicate_headers.status, 400);
    let unknown = request(
        api.addr(),
        "POST",
        &format!(
            "/2018-06-01/runtime/invocation/{}/response",
            AttemptId::default()
        ),
        "",
        b"{}",
    )
    .await;
    assert_eq!(unknown.status, 400);
    assert_eq!(
        request(api.addr(), "POST", &path, "", b"function-error")
            .await
            .status,
        202
    );
    assert!(
        matches!(ticket.wait().await.unwrap(), RuntimeResult::FunctionError(payload) if payload.as_ref() == b"function-error")
    );
    assert_eq!(
        request(api.addr(), "POST", &path, "", b"duplicate")
            .await
            .status,
        400
    );
    api.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_should_acknowledge_initialization_error_before_joining_the_closed_session() {
    let mut api = start().await.unwrap();
    let receiver = api.take_initialization_error().unwrap();
    assert!(api.take_initialization_error().is_none());
    assert_eq!(
        request(
            api.addr(),
            "POST",
            "/2018-06-01/runtime/init/error",
            "",
            b"secret-init-error"
        )
        .await
        .status,
        202
    );
    assert_eq!(receiver.await.unwrap().as_ref(), b"secret-init-error");
    api.shutdown().await.unwrap();
}

#[rstest]
#[case("GET", "/2018-06-01/runtime/invocation/next?secret-query=1", 400)]
#[case("GET", "http://localhost/2018-06-01/runtime/invocation/next", 400)]
#[case("POST", "/2018-06-01/runtime/invocation/next", 405)]
#[case("GET", "/2018-06-01/runtime/init/error", 405)]
#[case("POST", "/2018-06-01/runtime/invocation/not-an-id/response", 400)]
#[case(
    "POST",
    "/2018-06-01/runtime/invocation/00112233-4455-6677-8899-aabbccddeeff/extra/response",
    404
)]
#[tokio::test]
async fn test_should_reject_invalid_routes_before_bootstrap_readiness(
    #[case] method: &str,
    #[case] path: &str,
    #[case] expected: u16,
) {
    let mut api = start().await.unwrap();
    assert_eq!(
        request(api.addr(), method, path, "", b"").await.status,
        expected
    );
    assert_eq!(
        api.session().bootstrap_ready(Instant::now()).await,
        Err(SessionError::Deadline)
    );
    api.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_should_bound_headers_get_bodies_and_declared_post_bodies_before_session_commands() {
    let mut api = start().await.unwrap();
    assert_eq!(
        request(
            api.addr(),
            "GET",
            "/2018-06-01/runtime/invocation/next",
            "",
            b"x"
        )
        .await
        .status,
        413
    );
    assert_eq!(
        request(
            api.addr(),
            "GET",
            "/2018-06-01/runtime/invocation/next",
            &format!("X-Secret: {}\r\n", "s".repeat(257)),
            b""
        )
        .await
        .status,
        431
    );
    let request = format!(
        "POST /2018-06-01/runtime/init/error HTTP/1.1\r\nHost: localhost\r\nConnection: \
         close\r\nContent-Length: {}\r\n\r\n",
        MAX_PAYLOAD_BYTES + 1
    )
    .into_bytes();
    assert_eq!(raw(api.addr(), request).await.status, 413);
    api.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_should_enforce_streamed_chunked_body_limit_without_accepting_init_error() {
    let mut api = start().await.unwrap();
    let mut notification = api.take_initialization_error().unwrap();
    let stream = TcpStream::connect(api.addr()).await.unwrap();
    let (mut reader, mut writer) = stream.into_split();
    let send = tokio::spawn(async move {
        writer.write_all(b"POST /2018-06-01/runtime/init/error HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
        let header = format!("{:x}\r\n", MAX_PAYLOAD_BYTES + 1);
        writer.write_all(header.as_bytes()).await.unwrap();
        let _ = writer.write_all(&vec![0xff; MAX_PAYLOAD_BYTES + 1]).await;
        let _ = writer.write_all(b"\r\n0\r\n\r\n").await;
    });
    let mut reply = Vec::new();
    timeout(Duration::from_secs(3), reader.read_to_end(&mut reply))
        .await
        .unwrap()
        .unwrap();
    send.await.unwrap();
    assert!(reply.starts_with(b"HTTP/1.1 413"));
    assert!(notification.try_recv().is_err());
    api.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_should_keep_shutdown_owned_when_its_waiter_is_dropped_and_close_the_listener() {
    let mut api = start().await.unwrap();
    let addr = api.addr();
    let mut waiting = Box::pin(api.shutdown());
    assert!(
        waiting
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    drop(waiting);
    assert!(api.server.task.is_some());
    api.shutdown().await.unwrap();
    api.shutdown().await.unwrap();
    assert!(TcpStream::connect(addr).await.is_err());
    assert!(matches!(
        start_at("0.0.0.0:0".parse().unwrap(), Duration::from_secs(60)).await,
        Err(ApiError::NonLoopback)
    ));
}

#[tokio::test]
async fn test_should_retain_failed_supervision_on_repeated_shutdown() {
    let mut api = start().await.unwrap();
    api.server.task.as_ref().unwrap().abort();
    assert!(matches!(
        api.shutdown().await,
        Err(ApiError::SupervisorFailed)
    ));
    assert!(matches!(
        api.shutdown().await,
        Err(ApiError::SupervisorFailed)
    ));
}

#[tokio::test]
async fn test_should_omit_payloads_headers_and_targets_from_trace_level_runtime_diagnostics() {
    let logs = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = Arc::clone(&logs);
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_span_events(
            tracing_subscriber::fmt::format::FmtSpan::NEW
                | tracing_subscriber::fmt::format::FmtSpan::CLOSE,
        )
        .with_writer(move || LogWriter(Arc::clone(&captured)))
        .finish();
    // This current-thread runtime keeps its tasks under one thread-local
    // dispatcher, without changing process-wide logging or environment settings.
    let _subscriber = tracing::subscriber::set_default(subscriber);
    let mut api = start().await.unwrap();
    let call = invocation();
    let ticket = api
        .session()
        .submit(call.clone(), call.parts().1)
        .await
        .unwrap();
    assert_eq!(
        request(
            api.addr(),
            "GET",
            "/2018-06-01/runtime/invocation/next",
            "X-Secret: secret-header\r\n",
            b""
        )
        .await
        .status,
        200
    );
    let path = format!(
        "/2018-06-01/runtime/invocation/{}/response",
        ticket.attempt()
    );
    assert_eq!(
        request(api.addr(), "POST", &path, "", b"secret-result")
            .await
            .status,
        202
    );
    ticket.wait().await.unwrap();
    assert_eq!(
        request(
            api.addr(),
            "GET",
            "/2018-06-01/runtime/invocation/next?secret-target",
            "",
            b""
        )
        .await
        .status,
        400
    );
    let mut stream = TcpStream::connect(api.addr()).await.unwrap();
    stream.write_all(b"POST /2018-06-01/runtime/init/error HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n0\r\nX-Secret: secret-fragmented-trailer").await.unwrap();
    // Force another decoder poll with a partially buffered trailer.
    tokio::time::sleep(Duration::from_millis(30)).await;
    stream.write_all(b"\r\n\r\n").await.unwrap();
    let mut reply = Vec::new();
    timeout(Duration::from_secs(3), stream.read_to_end(&mut reply))
        .await
        .unwrap()
        .unwrap();
    assert!(reply.starts_with(b"HTTP/1.1 400"));
    api.shutdown().await.unwrap();
    let output = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
    assert!(
        !output.is_empty(),
        "trace capture must observe runtime spans"
    );
    for secret in [
        "secret-event",
        "secret-header",
        "secret-result",
        "secret-target",
        "secret-fragmented-trailer",
    ] {
        assert!(
            !output.contains(secret),
            "runtime diagnostics leaked {secret}"
        );
    }
}

#[tokio::test]
async fn test_should_reject_all_trailers_before_initialization_or_invocation_mutation() {
    let mut api = start().await.unwrap();
    let mut notification = api.take_initialization_error().unwrap();
    for trailer in [
        "X-Secret: secret-trailer".to_owned(),
        format!("X-Long: {}", "s".repeat(300)),
        format!("Lambda-Runtime-Invocation-Id: {}", AttemptId::default()),
    ] {
        let bytes = format!(
            "POST /2018-06-01/runtime/init/error HTTP/1.1\r\nHost: localhost\r\nConnection: \
             close\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n{trailer}\r\n\r\n"
        )
        .into_bytes();
        assert_eq!(raw(api.addr(), bytes).await.status, 400);
        assert!(notification.try_recv().is_err());
        assert!(!api.session().is_closed());
    }
    api.shutdown().await.unwrap();
}
