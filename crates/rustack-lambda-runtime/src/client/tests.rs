use std::{
    future::Future,
    task::{Context, Waker},
};

use bytes::Bytes;
use tokio::{
    io::{AsyncWriteExt, DuplexStream, duplex},
    net::TcpStream,
    task::JoinHandle,
};

use super::*;
use crate::{
    http::tests::{Reply, request},
    metadata::local_trace_id,
    protocol::{encode, read_frame, write_frame},
    relay::{RelayConfig, RelayHandle, start},
};

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(3)
}

fn invocation() -> Invocation {
    Invocation::new(
        AttemptId::default(),
        Instant::now() + Duration::from_secs(30),
        "arn:aws:lambda:us-east-1:000000000000:function:test:prod"
            .parse()
            .unwrap(),
        local_trace_id().unwrap(),
        Payload::try_from(Bytes::from_static(b"event-secret")).unwrap(),
    )
    .unwrap()
}

struct Peer {
    client: ControlClient,
    environment: EnvironmentId,
    input: DuplexStream,
    output: DuplexStream,
}

impl Peer {
    fn new(output_size: usize) -> Self {
        Self::with_limits(16, output_size, Duration::from_secs(1))
    }

    fn with_limits(input_size: usize, output_size: usize, budget: Duration) -> Self {
        let environment = EnvironmentId::default();
        let config = ClientConfig::new(environment, Architecture::Arm64, budget).unwrap();
        let (reader, output) = duplex(input_size);
        let (input, writer) = duplex(output_size);
        let client = ControlClient::start(config, reader, writer).unwrap();
        Self {
            client,
            environment,
            input,
            output,
        }
    }

    async fn send(&mut self, message: Message) {
        write_frame(
            &mut self.output,
            &Frame::new(self.environment, message),
            deadline(),
        )
        .await
        .unwrap();
    }

    async fn read(&mut self) -> Message {
        let frame = read_frame(&mut self.input, deadline())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(frame.environment(), self.environment);
        frame.into_message()
    }

    async fn ready(&mut self, bootstrap: bool) {
        self.send(Message::Hello {
            architecture: Architecture::Arm64,
        })
        .await;
        self.send(Message::Ready).await;
        assert_eq!(
            self.client.next(deadline()).await.unwrap(),
            ClientEvent::RelayReady
        );
        if bootstrap {
            self.send(Message::BootstrapReady).await;
            assert_eq!(
                self.client.next(deadline()).await.unwrap(),
                ClientEvent::BootstrapReady
            );
        }
    }

    async fn close(mut self) {
        self.client.begin_close(deadline()).await.unwrap();
        assert_eq!(self.read().await, Message::Close);
        self.send(Message::Ack(Acknowledgement::Close)).await;
        self.client.shutdown(deadline()).await.unwrap();
        self.client.shutdown(deadline()).await.unwrap();
        assert!(!self.client.is_idle());
    }

    async fn failed(mut self) {
        assert!(self.client.shutdown(deadline()).await.is_err());
        assert!(self.client.shutdown(deadline()).await.is_err());
        assert!(!self.client.is_idle());
    }
}

struct Harness {
    client: ControlClient,
    relay: RelayHandle,
    api: std::net::SocketAddr,
}

impl Harness {
    async fn new() -> Self {
        let api = crate::http::start().await.unwrap();
        let address = api.addr();
        let environment = EnvironmentId::default();
        let relay_config = RelayConfig::new(
            environment,
            Architecture::Arm64,
            Duration::from_secs(600),
            Duration::from_secs(1),
            Duration::from_secs(2),
        )
        .unwrap();
        let config =
            ClientConfig::new(environment, Architecture::Arm64, Duration::from_secs(1)).unwrap();
        let (writer, relay_reader) = duplex(24);
        let (relay_writer, reader) = duplex(24);
        let relay = start(api, relay_config, relay_reader, relay_writer).unwrap();
        let mut client = ControlClient::start(config, reader, writer).unwrap();
        assert_eq!(
            client.next(deadline()).await.unwrap(),
            ClientEvent::RelayReady
        );
        assert!(!client.is_idle());
        Self {
            client,
            relay,
            api: address,
        }
    }

    async fn bootstrap(&mut self) -> JoinHandle<Reply> {
        let address = self.api;
        let poll = tokio::spawn(async move {
            request(
                address,
                "GET",
                "/2018-06-01/runtime/invocation/next",
                "",
                b"",
            )
            .await
        });
        assert_eq!(
            self.client.next(deadline()).await.unwrap(),
            ClientEvent::BootstrapReady
        );
        assert!(self.client.is_idle());
        poll
    }

    async fn close(mut self) {
        self.client.shutdown(deadline()).await.unwrap();
        self.relay.wait().await.unwrap();
        assert!(TcpStream::connect(self.api).await.is_err());
    }
}

#[tokio::test]
async fn test_should_pair_client_with_real_relay_http_and_reuse_matching_attempts() {
    let mut harness = Harness::new().await;
    let mut poll = Some(harness.bootstrap().await);
    for index in 0..3 {
        let poll = poll.take().unwrap_or_else(|| {
            let address = harness.api;
            tokio::spawn(async move {
                request(
                    address,
                    "GET",
                    "/2018-06-01/runtime/invocation/next",
                    "",
                    b"",
                )
                .await
            })
        });
        let call = invocation();
        let attempt = call.parts().0;
        harness.client.invoke(call, deadline()).await.unwrap();
        let event = poll.await.unwrap();
        assert_eq!(event.status, 200);
        assert_eq!(event.body, b"event-secret");
        let suffix = if index == 2 { "error" } else { "response" };
        assert_eq!(
            request(
                harness.api,
                "POST",
                &format!("/2018-06-01/runtime/invocation/{attempt}/{suffix}"),
                "",
                b"computed-secret"
            )
            .await
            .status,
            202
        );
        let expected = Payload::try_from(Bytes::from_static(b"computed-secret")).unwrap();
        let result = if index == 2 {
            RuntimeResult::FunctionError(expected)
        } else {
            RuntimeResult::Success(expected)
        };
        assert_eq!(
            harness.client.next(deadline()).await.unwrap(),
            ClientEvent::InvocationResult { attempt, result }
        );
        assert_eq!(harness.client.is_idle(), index != 2);
    }
    harness.close().await;
}

#[tokio::test]
async fn test_should_forward_init_failure_before_bootstrap_and_close_both_owners() {
    let mut harness = Harness::new().await;
    assert_eq!(
        request(
            harness.api,
            "POST",
            "/2018-06-01/runtime/init/error",
            "",
            b"init-secret"
        )
        .await
        .status,
        202
    );
    assert!(
        matches!(harness.client.next(deadline()).await.unwrap(), ClientEvent::InitError(payload) if payload.as_ref() == b"init-secret")
    );
    assert!(!harness.client.is_idle());
    harness.close().await;
}

#[tokio::test]
async fn test_should_reject_foreign_scope_architecture_and_out_of_order_handshakes() {
    for mode in 0..6 {
        let mut peer = Peer::new(8192);
        match mode {
            0 => write_frame(
                &mut peer.output,
                &Frame::new(EnvironmentId::default(), Message::Ready),
                deadline(),
            )
            .await
            .unwrap(),
            1 => {
                peer.send(Message::Hello {
                    architecture: Architecture::X86_64,
                })
                .await;
            }
            2 => peer.send(Message::Ready).await,
            3 => peer.send(Message::BootstrapReady).await,
            4 => peer.send(Message::Close).await,
            _ => {
                peer.send(Message::Hello {
                    architecture: Architecture::Arm64,
                })
                .await;
                peer.send(Message::Hello {
                    architecture: Architecture::Arm64,
                })
                .await;
            }
        }
        assert!(peer.client.next(deadline()).await.is_err());
        peer.failed().await;
    }
}

#[tokio::test]
async fn test_should_reject_foreign_and_duplicate_results_and_cancel_ack_ids() {
    for mode in 0..3 {
        let mut peer = Peer::new(8192);
        peer.ready(true).await;
        let call = invocation();
        let attempt = call.parts().0;
        peer.client.invoke(call, deadline()).await.unwrap();
        assert!(matches!(peer.read().await, Message::Invoke { attempt: id, .. } if id == attempt));
        let outcome = Message::InvocationResult {
            attempt,
            outcome: Outcome::Success,
            payload: Payload::try_from(Bytes::from_static(b"result")).unwrap(),
        };
        match mode {
            0 => {
                peer.send(Message::InvocationResult {
                    attempt: AttemptId::default(),
                    outcome: Outcome::Success,
                    payload: Payload::try_from(Bytes::new()).unwrap(),
                })
                .await;
            }
            1 => {
                peer.send(outcome.clone()).await;
                assert!(matches!(
                    peer.client.next(deadline()).await.unwrap(),
                    ClientEvent::InvocationResult { .. }
                ));
                peer.send(outcome).await;
            }
            _ => {
                peer.client.cancel(attempt, deadline()).await.unwrap();
                assert_eq!(peer.read().await, Message::Cancel { attempt });
                peer.send(Message::Ack(Acknowledgement::Cancel(AttemptId::default())))
                    .await;
            }
        }
        assert!(matches!(
            peer.client.next(deadline()).await,
            Err(RelayError::UnexpectedMessage)
        ));
        peer.failed().await;
    }
}

#[tokio::test]
async fn test_should_discard_only_one_late_result_during_cancellation_and_require_matching_ack() {
    for completed in [false, true] {
        let mut peer = Peer::new(8192);
        peer.ready(true).await;
        let call = invocation();
        let attempt = call.parts().0;
        peer.client.invoke(call, deadline()).await.unwrap();
        peer.read().await;
        peer.send(Message::InvocationResult {
            attempt,
            outcome: Outcome::Success,
            payload: Payload::try_from(Bytes::from_static(b"late")).unwrap(),
        })
        .await;
        if completed {
            assert!(matches!(
                peer.client.next(deadline()).await.unwrap(),
                ClientEvent::InvocationResult { .. }
            ));
        }
        peer.client.cancel(attempt, deadline()).await.unwrap();
        assert_eq!(peer.read().await, Message::Cancel { attempt });
        peer.send(Message::Ack(Acknowledgement::Cancel(attempt)))
            .await;
        assert_eq!(
            peer.client.next(deadline()).await.unwrap(),
            ClientEvent::Cancelled(attempt)
        );
        assert!(!peer.client.is_idle());
        peer.client.cancel(attempt, deadline()).await.unwrap();
        assert_eq!(peer.read().await, Message::Cancel { attempt });
        peer.send(Message::Ack(Acknowledgement::Cancel(attempt)))
            .await;
        assert_eq!(
            peer.client.next(deadline()).await.unwrap(),
            ClientEvent::Cancelled(attempt)
        );
        peer.close().await;
    }
}

#[tokio::test]
async fn test_should_validate_pending_events_before_close_and_reject_early_close_ack() {
    for mode in 0..4 {
        let mut peer = Peer::new(8192);
        peer.ready(mode != 0).await;
        let call = invocation();
        let attempt = call.parts().0;
        if mode != 0 {
            peer.client.invoke(call, deadline()).await.unwrap();
            peer.read().await;
        }
        if mode >= 2 {
            peer.client.cancel(attempt, deadline()).await.unwrap();
            peer.read().await;
        }
        peer.client.begin_close(deadline()).await.unwrap();
        assert_eq!(peer.read().await, Message::Close);
        let preceding = match mode {
            0 => Message::BootstrapReady,
            1 => Message::InvocationResult {
                attempt,
                outcome: Outcome::Success,
                payload: Payload::try_from(Bytes::new()).unwrap(),
            },
            _ => Message::Ack(Acknowledgement::Cancel(attempt)),
        };
        if mode == 3 {
            peer.send(Message::Ack(Acknowledgement::Close)).await;
            assert!(peer.client.next(deadline()).await.is_err());
            peer.failed().await;
        } else {
            peer.send(preceding).await;
            peer.send(Message::Ack(Acknowledgement::Close)).await;
            peer.client.shutdown(deadline()).await.unwrap();
        }
    }
}

#[tokio::test]
async fn test_should_keep_partial_framing_when_a_next_waiter_is_cancelled() {
    let mut peer = Peer::new(8192);
    peer.ready(false).await;
    let bytes = encode(&Frame::new(peer.environment, Message::BootstrapReady)).unwrap();
    peer.output
        .write_all(bytes.get(..2).unwrap())
        .await
        .unwrap();
    let mut waiting = Box::pin(peer.client.next(deadline()));
    assert!(
        waiting
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    drop(waiting);
    peer.output
        .write_all(bytes.get(2..).unwrap())
        .await
        .unwrap();
    assert_eq!(
        peer.client.next(deadline()).await.unwrap(),
        ClientEvent::BootstrapReady
    );
    peer.close().await;
}

#[tokio::test]
async fn test_should_fail_closed_after_a_cancelled_send_without_losing_io_ownership() {
    let mut peer = Peer::new(1);
    peer.ready(true).await;
    let mut sending = Box::pin(peer.client.invoke(invocation(), deadline()));
    assert!(
        sending
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    drop(sending);
    assert!(matches!(
        peer.client.next(deadline()).await,
        Err(RelayError::Output)
    ));
    peer.failed().await;
}

#[tokio::test]
async fn test_should_resume_close_after_its_waiter_is_dropped() {
    let mut peer = Peer::new(8192);
    peer.ready(false).await;
    peer.client.begin_close(deadline()).await.unwrap();
    assert_eq!(peer.read().await, Message::Close);
    let mut waiting = Box::pin(peer.client.shutdown(deadline()));
    assert!(
        waiting
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    drop(waiting);
    peer.send(Message::Ack(Acknowledgement::Close)).await;
    peer.client.shutdown(deadline()).await.unwrap();
    peer.client.shutdown(deadline()).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn test_should_bound_partial_frames_and_cache_receive_deadline_failures() {
    for partial in [false, true] {
        let mut peer = Peer::new(8192);
        peer.ready(false).await;
        if partial {
            peer.output.write_all(&[0]).await.unwrap();
        }
        assert!(peer.client.next(deadline()).await.is_err());
        peer.failed().await;
    }
}

#[tokio::test]
async fn test_should_reject_local_invocation_before_bootstrap_and_reused_attempt_identity() {
    for bootstrap in [false, true] {
        let mut peer = Peer::new(8192);
        peer.ready(bootstrap).await;
        let call = invocation();
        if bootstrap {
            let attempt = call.parts().0;
            peer.client.invoke(call.clone(), deadline()).await.unwrap();
            peer.read().await;
            peer.send(Message::InvocationResult {
                attempt,
                outcome: Outcome::Success,
                payload: Payload::try_from(Bytes::new()).unwrap(),
            })
            .await;
            peer.client.next(deadline()).await.unwrap();
        }
        assert!(peer.client.invoke(call, deadline()).await.is_err());
        peer.failed().await;
    }
}

#[tokio::test(start_paused = true)]
async fn test_should_reject_a_queued_frame_when_a_pending_waiter_resumes_after_its_deadline() {
    let mut peer = Peer::with_limits(1, 8192, Duration::from_secs(60));
    peer.ready(false).await;
    let expires = Instant::now() + Duration::from_secs(1);
    let mut waiting = Box::pin(peer.client.next(expires));
    assert!(
        waiting
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    write_frame(
        &mut peer.output,
        &Frame::new(peer.environment, Message::BootstrapReady),
        deadline(),
    )
    .await
    .unwrap();
    // Capacity one forces the reader to finish the preceding frame before this
    // byte fits. Its next partial-frame budget is sixty seconds, beyond the
    // caller's one-second deadline, so a valid queued frame wins an inner poll.
    peer.output.write_all(&[0]).await.unwrap();
    tokio::time::advance(Duration::from_secs(2)).await;
    assert!(matches!(waiting.await, Err(RelayError::Deadline)));
    peer.failed().await;
}

#[tokio::test]
async fn test_should_reject_initialization_errors_after_a_result_proves_delivery() {
    for mode in 0..4 {
        let mut peer = Peer::new(8192);
        peer.ready(true).await;
        let call = invocation();
        let mut attempt = call.parts().0;
        peer.client.invoke(call, deadline()).await.unwrap();
        peer.read().await;
        peer.send(Message::InvocationResult {
            attempt,
            outcome: Outcome::Success,
            payload: Payload::try_from(Bytes::new()).unwrap(),
        })
        .await;
        peer.client.next(deadline()).await.unwrap();
        if mode == 1 || mode == 3 {
            let next = invocation();
            attempt = next.parts().0;
            peer.client.invoke(next, deadline()).await.unwrap();
            peer.read().await;
        }
        if mode >= 2 {
            peer.client.cancel(attempt, deadline()).await.unwrap();
            peer.read().await;
        }
        peer.send(Message::InitError {
            payload: Payload::try_from(Bytes::from_static(b"late-init-secret")).unwrap(),
        })
        .await;
        assert!(matches!(
            peer.client.next(deadline()).await,
            Err(RelayError::UnexpectedMessage)
        ));
        peer.failed().await;
    }
}

#[test]
fn test_should_validate_client_io_budget() {
    for budget in [Duration::ZERO, Duration::from_secs(61)] {
        assert!(ClientConfig::new(EnvironmentId::default(), Architecture::Arm64, budget).is_err());
    }
}

#[tokio::test]
async fn test_should_preserve_transport_worker_failure_after_a_prior_scope_error() {
    let environment = EnvironmentId::default();
    let (reader, mut peer) = duplex(4096);
    let mut client = ControlClient::start(
        ClientConfig::new(environment, Architecture::Arm64, Duration::from_secs(1)).unwrap(),
        reader,
        tokio::io::sink(),
    )
    .unwrap();
    peer.write_all(
        &encode(&Frame::new(
            EnvironmentId::default(),
            Message::Hello {
                architecture: Architecture::Arm64,
            },
        ))
        .unwrap(),
    )
    .await
    .unwrap();
    assert!(matches!(
        client.next(deadline()).await,
        Err(RelayError::Scope)
    ));
    client.transport.abort_owner();
    assert!(matches!(
        client.shutdown(deadline()).await,
        Err(RelayError::Worker)
    ));
    assert!(matches!(
        client.shutdown(deadline()).await,
        Err(RelayError::Worker)
    ));
}
