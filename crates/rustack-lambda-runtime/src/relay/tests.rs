use std::{
    future::Future,
    num::NonZeroU64,
    task::{Context, Waker},
};

use bytes::Bytes;
use tokio::{
    io::{AsyncWriteExt, DuplexStream, duplex},
    net::TcpStream,
};

use super::*;
use crate::{
    http::tests::{Reply, request},
    metadata::{deadline_unix_ms, local_trace_id},
    protocol::{FunctionArn, Payload},
};

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(3)
}

struct Harness {
    owner: RelayHandle,
    environment: EnvironmentId,
    input: DuplexStream,
    output: DuplexStream,
    api: std::net::SocketAddr,
}

impl Harness {
    async fn new() -> Self {
        let api = crate::http::start().await.unwrap();
        let address = api.addr();
        let environment = EnvironmentId::default();
        let config = RelayConfig::new(
            environment,
            Architecture::Arm64,
            Duration::from_secs(600),
            Duration::from_secs(1),
            Duration::from_secs(2),
        )
        .unwrap();
        let (input, reader) = duplex(128);
        let (writer, output) = duplex(128);
        let owner = start(api, config, reader, writer).unwrap();
        let mut harness = Self {
            owner,
            environment,
            input,
            output,
            api: address,
        };
        assert!(matches!(
            harness.next().await,
            Message::Hello {
                architecture: Architecture::Arm64
            }
        ));
        assert_eq!(harness.next().await, Message::Ready);
        harness
    }

    async fn next(&mut self) -> Message {
        let frame = read_frame(&mut self.output, deadline())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(frame.environment(), self.environment);
        frame.into_message()
    }

    async fn send(&mut self, message: Message) {
        write_frame(
            &mut self.input,
            &Frame::new(self.environment, message),
            deadline(),
        )
        .await
        .unwrap();
    }

    async fn close(mut self) {
        self.send(Message::Close).await;
        assert_eq!(self.next().await, Message::Ack(Acknowledgement::Close));
        timeout_at(deadline(), self.owner.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(
            read_frame(&mut self.output, deadline())
                .await
                .unwrap()
                .is_none()
        );
        assert!(TcpStream::connect(self.api).await.is_err());
    }

    async fn bootstrap(&mut self) -> JoinHandle<Reply> {
        let api = self.api;
        let poll = tokio::spawn(async move {
            request(api, "GET", "/2018-06-01/runtime/invocation/next", "", b"").await
        });
        assert_eq!(self.next().await, Message::BootstrapReady);
        poll
    }
}

fn invoke() -> (AttemptId, Message) {
    let attempt = AttemptId::default();
    (
        attempt,
        Message::Invoke {
            attempt,
            deadline_unix_ms: deadline_unix_ms(Instant::now() + Duration::from_secs(10)).unwrap(),
            function_arn: "arn:aws:lambda:us-east-1:000000000000:function:echo:prod"
                .parse::<FunctionArn>()
                .unwrap(),
            trace_id: local_trace_id().unwrap(),
            payload: Payload::try_from(Bytes::from_static(b"secret-event")).unwrap(),
        },
    )
}

#[tokio::test]
async fn test_should_handshake_forward_http_results_and_reuse_one_session() {
    let mut harness = Harness::new().await;
    let first = harness.bootstrap().await;
    for (index, poll) in [(0, Some(first)), (1, None)] {
        let poll = poll.unwrap_or_else(|| {
            let api = harness.api;
            tokio::spawn(async move {
                request(api, "GET", "/2018-06-01/runtime/invocation/next", "", b"").await
            })
        });
        let (attempt, message) = invoke();
        harness.send(message).await;
        let event = poll.await.unwrap();
        assert_eq!(event.status, 200);
        assert_eq!(event.body, b"secret-event");
        let computed = format!("{{\"computed\":{},\"iteration\":{index}}}", 6 * 7);
        assert_eq!(
            request(
                harness.api,
                "POST",
                &format!("/2018-06-01/runtime/invocation/{attempt}/response"),
                "",
                computed.as_bytes()
            )
            .await
            .status,
            202
        );
        assert!(
            matches!(harness.next().await, Message::InvocationResult { attempt: id, outcome: Outcome::Success, payload } if id == attempt && payload.as_ref() == computed.as_bytes())
        );
    }
    harness.close().await;
}

#[tokio::test]
async fn test_should_ack_cancel_while_busy_and_after_completion_without_replaying_a_result() {
    for completed in [false, true] {
        let mut harness = Harness::new().await;
        let poll = harness.bootstrap().await;
        let (attempt, message) = invoke();
        harness.send(message).await;
        assert_eq!(poll.await.unwrap().status, 200);
        if completed {
            assert_eq!(
                request(
                    harness.api,
                    "POST",
                    &format!("/2018-06-01/runtime/invocation/{attempt}/response"),
                    "",
                    b"done"
                )
                .await
                .status,
                202
            );
            assert!(matches!(
                harness.next().await,
                Message::InvocationResult {
                    outcome: Outcome::Success,
                    ..
                }
            ));
        }
        harness.send(Message::Cancel { attempt }).await;
        assert_eq!(
            harness.next().await,
            Message::Ack(Acknowledgement::Cancel(attempt))
        );
        harness.send(Message::Cancel { attempt }).await;
        assert_eq!(
            harness.next().await,
            Message::Ack(Acknowledgement::Cancel(attempt))
        );
        harness.close().await;
    }
}

#[tokio::test]
async fn test_should_distinguish_function_and_initialization_errors_and_allow_close_ack() {
    let mut harness = Harness::new().await;
    assert_eq!(
        request(
            harness.api,
            "POST",
            "/2018-06-01/runtime/init/error",
            "",
            b"secret-init-error"
        )
        .await
        .status,
        202
    );
    assert!(
        matches!(harness.next().await, Message::InitError { payload } if payload.as_ref() == b"secret-init-error")
    );
    harness.close().await;
    let mut harness = Harness::new().await;
    let poll = harness.bootstrap().await;
    let (attempt, message) = invoke();
    harness.send(message).await;
    poll.await.unwrap();
    assert_eq!(
        request(
            harness.api,
            "POST",
            &format!("/2018-06-01/runtime/invocation/{attempt}/error"),
            "",
            b"secret-function-error"
        )
        .await
        .status,
        202
    );
    assert!(
        matches!(harness.next().await, Message::InvocationResult { outcome: Outcome::FunctionError, payload, .. } if payload.as_ref() == b"secret-function-error")
    );
    harness.close().await;
}

#[tokio::test]
async fn test_should_reject_foreign_scope_wrong_direction_and_invoke_before_bootstrap() {
    for variant in 0..3 {
        let mut harness = Harness::new().await;
        match variant {
            0 => write_frame(
                &mut harness.input,
                &Frame::new(EnvironmentId::default(), Message::Close),
                deadline(),
            )
            .await
            .unwrap(),
            1 => harness.send(Message::Ready).await,
            _ => harness.send(invoke().1).await,
        }
        assert!(
            timeout_at(deadline(), harness.owner.wait())
                .await
                .unwrap()
                .is_err()
        );
        assert!(harness.owner.wait().await.is_err());
        assert!(TcpStream::connect(harness.api).await.is_err());
    }
}

#[tokio::test]
async fn test_should_retain_shutdown_after_dropping_its_waiter_and_interrupt_idle_input() {
    let mut harness = Harness::new().await;
    let mut waiting = Box::pin(harness.owner.shutdown());
    assert!(
        waiting
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    drop(waiting);
    assert!(harness.owner.task.is_some());
    timeout_at(deadline(), harness.owner.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert!(TcpStream::connect(harness.api).await.is_err());
}

#[tokio::test]
async fn test_should_cleanup_api_after_eof_and_truncated_control_frames() {
    for truncated in [false, true] {
        let mut harness = Harness::new().await;
        if truncated {
            harness.input.write_all(&[0, 0]).await.unwrap();
        }
        harness.input.shutdown().await.unwrap();
        assert!(
            timeout_at(deadline(), harness.owner.wait())
                .await
                .unwrap()
                .is_err()
        );
        assert!(TcpStream::connect(harness.api).await.is_err());
    }
}

#[tokio::test]
async fn test_should_reject_bad_clock_and_overlapping_invocations_without_success() {
    for overlapping in [false, true] {
        let mut harness = Harness::new().await;
        let mut poll = Some(harness.bootstrap().await);
        let (_, mut message) = invoke();
        if overlapping {
            harness.send(message).await;
            poll.take().unwrap().await.unwrap();
            message = invoke().1;
        } else if let Message::Invoke {
            deadline_unix_ms, ..
        } = &mut message
        {
            *deadline_unix_ms = NonZeroU64::new(1).unwrap();
        }
        harness.send(message).await;
        if let Some(poll) = poll {
            let _ = poll.await;
        }
        assert!(
            timeout_at(deadline(), harness.owner.wait())
                .await
                .unwrap()
                .is_err()
        );
    }
}

#[tokio::test]
async fn test_should_close_a_probe_before_any_function_has_polled() {
    Harness::new().await.close().await;
}

#[tokio::test(start_paused = true)]
async fn test_should_bound_partial_frame_and_blocked_output_io_and_join_the_api() {
    let mut harness = Harness::new().await;
    harness.input.write_all(&[0]).await.unwrap();
    assert!(
        matches!(harness.owner.wait().await, Err(RelayError::Protocol(error)) if matches!(&*error, ProtocolError::Deadline))
    );
    assert!(TcpStream::connect(harness.api).await.is_err());

    let api = crate::http::start().await.unwrap();
    let address = api.addr();
    let config = RelayConfig::new(
        EnvironmentId::default(),
        Architecture::Arm64,
        Duration::from_secs(600),
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .unwrap();
    let (_host_input, reader) = duplex(1);
    let (writer, _host_output) = duplex(1);
    let mut owner = start(api, config, reader, writer).unwrap();
    assert!(matches!(owner.wait().await, Err(RelayError::Output)));
    assert!(TcpStream::connect(address).await.is_err());
}

#[tokio::test]
async fn test_should_preserve_a_completed_io_failure_when_priority_stop_wins() {
    let api = crate::http::start().await.unwrap();
    let address = api.addr();
    let config = RelayConfig::new(
        EnvironmentId::default(),
        Architecture::Arm64,
        Duration::from_secs(600),
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .unwrap();
    let (_input, commands) = mpsc::channel(MAILBOX_CAPACITY);
    let (output, mut frames) = mpsc::channel::<Outbound>(MAILBOX_CAPACITY);
    let written = tokio::spawn(async move {
        for _ in 0..2 {
            frames.recv().await.unwrap().written.send(true).unwrap();
        }
    });
    let mut workers = JoinSet::new();
    let failed = workers.spawn(async { Err(RelayError::Eof) });
    while !failed.is_finished() {
        tokio::task::yield_now().await;
    }
    let (_stop, stopped) = watch::channel(true);
    let actor = RelayActor::builder()
        .api(api)
        .config(config)
        .input(commands)
        .output(output)
        .workers(workers)
        .stop(stopped)
        .initialization_error(None)
        .build();
    assert!(matches!(actor.run().await, Err(RelayError::Eof)));
    written.await.unwrap();
    assert!(TcpStream::connect(address).await.is_err());
}

#[test]
fn test_should_bound_relay_config() {
    let environment = EnvironmentId::default();
    assert!(
        RelayConfig::new(
            environment,
            Architecture::Arm64,
            Duration::ZERO,
            Duration::from_secs(1),
            Duration::from_secs(1)
        )
        .is_err()
    );
    assert!(
        RelayConfig::new(
            environment,
            Architecture::Arm64,
            Duration::from_secs(600),
            Duration::from_secs(61),
            Duration::from_secs(1)
        )
        .is_err()
    );
}
