use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream, ReadBuf, duplex},
    time::{advance, timeout},
};

use super::*;

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}
fn logs(reader: DuplexStream) -> FunctionLogs {
    FunctionLogs::start_reader(
        DockerOutputReader::new(reader, Duration::from_secs(1)).unwrap(),
        TAIL,
    )
    .unwrap()
}
fn record(channel: u8, bytes: &[u8]) -> Vec<u8> {
    let mut frame = vec![channel, 0, 0, 0];
    frame.extend_from_slice(&u32::try_from(bytes.len()).unwrap().to_be_bytes());
    frame.extend_from_slice(bytes);
    frame
}
async fn closed(logs: &FunctionLogs) -> LogSnapshot {
    timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = logs.snapshot(deadline()).await.unwrap();
            if snapshot.closed || snapshot.failure.is_some() {
                break snapshot;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn test_should_capture_binary_channels_in_observed_order_and_retain_tail_after_eof() {
    let (mut writer, reader) = duplex(1024);
    let mut logs = logs(reader);
    writer.write_all(&record(1, b"stdout\0\xff")).await.unwrap();
    writer.write_all(&record(2, b"stderr\n")).await.unwrap();
    writer.shutdown().await.unwrap();
    let snapshot = closed(&logs).await;
    assert_eq!(snapshot.tail.as_ref(), b"stdout\0\xffstderr\n");
    assert_eq!(snapshot.stdout_discarded, 0);
    assert_eq!(snapshot.stderr_discarded, 0);
    assert!(snapshot.failure.is_none());
    assert!(!format!("{snapshot:?} {logs:?}").contains("stdout\\0"));
    assert!(!format!("{snapshot:?}").contains("stderr\\n"));
    assert_eq!(logs.snapshot(deadline()).await.unwrap().tail, snapshot.tail);
    assert_eq!(logs.shutdown().await.unwrap().tail, snapshot.tail);
    assert_eq!(logs.shutdown().await.unwrap().tail, snapshot.tail);
}

#[tokio::test]
async fn test_should_bound_independent_rings_and_saturate_discard_counts() {
    let mut capture = Capture::new(TAIL).unwrap();
    capture
        .push(&OutputChunk {
            channel: OutputChannel::Stdout,
            bytes: Bytes::from(vec![1; TAIL + 17]),
        })
        .unwrap();
    capture
        .push(&OutputChunk {
            channel: OutputChannel::Stderr,
            bytes: Bytes::from(vec![2; TAIL + 9]),
        })
        .unwrap();
    capture
        .push(&OutputChunk {
            channel: OutputChannel::Stdout,
            bytes: Bytes::from_static(b"final"),
        })
        .unwrap();
    assert_eq!(capture.stdout.bytes.len(), TAIL);
    assert_eq!(capture.stderr.bytes.len(), TAIL);
    assert_eq!(capture.stdout.discarded, 22);
    assert_eq!(capture.stderr.discarded, 9);
    let snapshot = capture.snapshot();
    assert_eq!(snapshot.tail.len(), TAIL);
    assert!(
        snapshot
            .tail
            .get(..TAIL - 5)
            .unwrap()
            .iter()
            .all(|byte| *byte == 2)
    );
    assert_eq!(snapshot.tail.get(TAIL - 5..), Some(b"final".as_slice()));
    capture.stdout.discarded = u64::MAX - 1;
    capture.stdout.push(b"more").unwrap();
    assert_eq!(capture.stdout.discarded, u64::MAX);
}

#[rstest::rstest]
#[case(0)]
#[case(TAIL - 1)]
#[case(1024 * 1024 + 1)]
#[tokio::test]
async fn test_should_reject_invalid_log_capacity(#[case] capacity: usize) {
    let (_, reader) = duplex(1);
    assert!(matches!(
        FunctionLogs::start_reader(
            DockerOutputReader::new(reader, Duration::from_secs(1)).unwrap(),
            capacity
        ),
        Err(DockerError::Configuration)
    ));
}

#[tokio::test]
async fn test_should_keep_a_fragmented_record_alive_through_cancelled_and_completed_snapshots() {
    let (mut writer, reader) = duplex(1);
    let mut logs = logs(reader);
    let frame = record(1, b"secret-frame");
    writer.write_all(frame.get(..4).unwrap()).await.unwrap();
    for _ in 0..16 {
        assert!(logs.snapshot(deadline()).await.unwrap().tail.is_empty());
    }
    {
        let snapshot = logs.snapshot(deadline());
        tokio::pin!(snapshot);
        assert!(
            snapshot
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }
    writer.write_all(frame.get(4..).unwrap()).await.unwrap();
    writer.shutdown().await.unwrap();
    assert_eq!(closed(&logs).await.tail.as_ref(), b"secret-frame");
    assert!(logs.shutdown().await.is_ok());
}

#[tokio::test(start_paused = true)]
async fn test_should_not_extend_the_partial_record_deadline_with_snapshot_traffic() {
    let (mut writer, reader) = duplex(1);
    let mut logs = logs(reader);
    writer.write_all(&[1, 0, 0, 0]).await.unwrap();
    for _ in 0..12 {
        let _ = logs.snapshot(deadline()).await.unwrap();
        advance(Duration::from_millis(100)).await;
    }
    let failed = logs.snapshot(deadline()).await.unwrap();
    assert!(matches!(failed.failure, Some(DockerError::Deadline)));
    assert!(matches!(logs.shutdown().await, Err(DockerError::Deadline)));
    assert!(matches!(logs.shutdown().await, Err(DockerError::Deadline)));
}

#[tokio::test(start_paused = true)]
async fn test_should_bound_first_byte_silence_independently_of_snapshot_requests() {
    let (_writer, reader) = duplex(1);
    let mut logs = logs(reader);
    logs.snapshot(deadline()).await.unwrap();
    advance(SILENCE + Duration::from_secs(1)).await;
    assert!(matches!(
        logs.snapshot(deadline()).await.unwrap().failure,
        Some(DockerError::Deadline)
    ));
    assert!(matches!(logs.shutdown().await, Err(DockerError::Deadline)));
}

#[tokio::test]
async fn test_should_retain_shutdown_ownership_after_its_waiter_is_cancelled() {
    let (mut writer, reader) = duplex(1);
    let mut logs = logs(reader);
    {
        let shutdown = logs.shutdown();
        tokio::pin!(shutdown);
        assert!(
            shutdown
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }
    assert!(
        timeout(Duration::from_secs(1), logs.shutdown())
            .await
            .unwrap()
            .is_ok()
    );
    assert!(writer.write_all(b"x").await.is_err());
}

#[tokio::test]
async fn test_should_signal_quiet_socket_cleanup_on_drop_outside_an_entered_runtime() {
    let (mut writer, reader) = duplex(1);
    let logs = logs(reader);
    let thread = std::thread::spawn(move || {
        assert!(Handle::try_current().is_err());
        drop(logs);
    });
    let task = tokio::task::spawn_blocking(move || thread.join().unwrap());
    timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
    let mut byte = [0];
    assert_eq!(
        timeout(Duration::from_secs(1), writer.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn test_should_serve_snapshots_and_stop_while_output_floods() {
    let (mut writer, reader) = duplex(64 * 1024);
    let mut logs = logs(reader);
    let producer = tokio::spawn(async move {
        let frame = record(1, &vec![b'x'; 16 * 1024]);
        while writer.write_all(&frame).await.is_ok() {
            tokio::task::yield_now().await;
        }
    });
    timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = logs.snapshot(deadline()).await.unwrap();
            assert!(snapshot.tail.len() <= TAIL);
            if snapshot.stdout_discarded > 64 * 1024 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        timeout(Duration::from_secs(1), logs.shutdown())
            .await
            .unwrap()
            .is_ok()
    );
    timeout(Duration::from_secs(1), producer)
        .await
        .unwrap()
        .unwrap();
}

#[rstest::rstest]
#[case::bad_channel(vec![3, 0, 0, 0, 0, 0, 0, 0])]
#[case::reserved(vec![1, 1, 0, 0, 0, 0, 0, 0])]
#[case::truncated_header(vec![1, 0])]
#[case::truncated_payload(vec![1, 0, 0, 0, 0, 0, 0, 2, 1])]
#[tokio::test]
async fn test_should_preserve_malformed_stream_failure_through_shutdown(#[case] frame: Vec<u8>) {
    let (mut writer, reader) = duplex(64);
    let mut logs = logs(reader);
    writer.write_all(&frame).await.unwrap();
    writer.shutdown().await.unwrap();
    assert!(closed(&logs).await.failure.is_some());
    let mut byte = [0];
    assert_eq!(
        timeout(Duration::from_secs(1), writer.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert!(logs.shutdown().await.is_err());
    assert!(logs.shutdown().await.is_err());
}

struct Panics;
impl AsyncRead for Panics {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        _: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        panic!("log worker failure")
    }
}
#[tokio::test]
async fn test_should_cache_log_actor_join_failure() {
    let mut logs = FunctionLogs::start_reader(
        DockerOutputReader::new(Panics, Duration::from_secs(1)).unwrap(),
        TAIL,
    )
    .unwrap();
    while !logs.task.as_ref().unwrap().is_finished() {
        tokio::task::yield_now().await;
    }
    assert!(matches!(
        logs.snapshot(deadline()).await,
        Err(DockerError::Worker)
    ));
    assert!(matches!(logs.shutdown().await, Err(DockerError::Worker)));
    assert!(matches!(logs.shutdown().await, Err(DockerError::Worker)));
}

#[tokio::test]
async fn test_should_bound_snapshot_admission_and_discard_abandoned_requests() {
    let (_writer, reader) = duplex(1);
    let mut logs = logs(reader);
    {
        let mut requests = Vec::new();
        for _ in 0..8 {
            let mut request = Box::pin(logs.snapshot(deadline()));
            assert!(
                request
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop()))
                    .is_pending()
            );
            requests.push(request);
        }
        assert!(matches!(
            logs.snapshot(deadline()).await,
            Err(DockerError::Busy)
        ));
        drop(requests);
    }
    assert!(
        timeout(Duration::from_secs(1), logs.shutdown())
            .await
            .unwrap()
            .is_ok()
    );
}

#[tokio::test]
async fn test_should_use_output_only_attach_without_replay_and_transfer_driver_ownership() {
    let mut daemon = super::super::tests::Daemon::new();
    let mut wire = daemon.wire().await;
    let port = wire.port();
    let attach = tokio::spawn(async move {
        port.attach_logs(&"function".parse().unwrap(), deadline())
            .await
    });
    let request = daemon.next().await;
    assert!(request.head.starts_with(
        "POST /v1.53/containers/function/attach?stdin=false&stdout=true&stderr=true&stream=true&\
         logs=false HTTP/1.1\r\n"
    ));
    let mut reply =
        b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: tcp\r\n\r\n".to_vec();
    reply.extend_from_slice(&record(2, b"wire-secret-output"));
    request.raw(&reply).await;
    let mut logs =
        FunctionLogs::start(attach.await.unwrap().unwrap(), Duration::from_secs(1), TAIL).unwrap();
    wire.shutdown().await.unwrap();
    assert_eq!(closed(&logs).await.tail.as_ref(), b"wire-secret-output");
    assert!(logs.shutdown().await.is_ok());
    daemon.close().await;
}

async fn observed(logs: &FunctionLogs, suffix: &[u8]) {
    timeout(Duration::from_secs(3), async {
        while !logs
            .snapshot(deadline())
            .await
            .unwrap()
            .tail
            .ends_with(suffix)
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
async fn probe(sender: &mpsc::Sender<Request>) -> LogSnapshot {
    let (reply, response) = oneshot::channel();
    sender
        .try_send(Request {
            action: Action::Snapshot,
            deadline: deadline(),
            reply,
        })
        .unwrap_or_else(|_| panic!("test probe admission"));
    timeout(Duration::from_secs(3), response)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
}
#[tokio::test]
async fn test_should_keep_diagnostics_separate_and_never_seed_a_silent_invocation_with_prior_logs()
{
    let (mut writer, reader) = duplex(8192);
    let mut logs = logs(reader);
    writer.write_all(&record(1, b"idle-before")).await.unwrap();
    observed(&logs, b"idle-before").await;
    let first = logs.begin(AttemptId::default(), deadline()).await.unwrap();
    writer.write_all(&record(1, b"first\0\xff")).await.unwrap();
    writer.write_all(&record(2, b"stderr")).await.unwrap();
    observed(&logs, b"stderr").await;
    let first = logs.finish(first, deadline()).await.unwrap();
    assert_eq!(first.tail.as_ref(), b"first\0\xffstderr");
    writer.write_all(&record(2, b"idle-after")).await.unwrap();
    observed(&logs, b"idle-after").await;
    let silent = logs.begin(AttemptId::default(), deadline()).await.unwrap();
    assert!(
        logs.finish(silent, deadline())
            .await
            .unwrap()
            .tail
            .is_empty()
    );
    let third = logs.begin(AttemptId::default(), deadline()).await.unwrap();
    writer.write_all(&record(1, b"third")).await.unwrap();
    observed(&logs, b"third").await;
    assert_eq!(
        logs.finish(third, deadline()).await.unwrap().tail.as_ref(),
        b"third"
    );
    assert!(
        logs.snapshot(deadline())
            .await
            .unwrap()
            .tail
            .starts_with(b"idle-before")
    );
    logs.shutdown().await.unwrap();
}
#[tokio::test]
async fn test_should_bound_binary_window_tail_and_allow_matching_finish_after_clean_eof() {
    let (mut writer, reader) = duplex(16384);
    let mut logs = logs(reader);
    let window = logs.begin(AttemptId::default(), deadline()).await.unwrap();
    writer
        .write_all(&record(1, &vec![0xff; TAIL + 17]))
        .await
        .unwrap();
    writer.write_all(&record(2, b"end\0")).await.unwrap();
    writer.shutdown().await.unwrap();
    let diagnostic = closed(&logs).await;
    let finished = logs.finish(window, deadline()).await.unwrap();
    assert_eq!(finished.tail.len(), TAIL);
    assert_eq!(finished.tail, diagnostic.tail);
    assert!(finished.tail.ends_with(b"end\0"));
    assert_eq!(finished.stdout_discarded, 17);
    assert!(finished.closed);
    assert!(!format!("{finished:?}").contains("end\\0"));
    assert!(matches!(
        logs.begin(AttemptId::default(), deadline()).await,
        Err(DockerError::Stopped)
    ));
    logs.shutdown().await.unwrap();
}
#[tokio::test]
async fn test_should_revoke_capture_and_close_its_quiet_socket_when_an_open_capability_is_dropped()
{
    let (mut writer, reader) = duplex(8);
    let mut logs = logs(reader);
    let window = logs.begin(AttemptId::default(), deadline()).await.unwrap();
    drop(window);
    assert_eq!(
        timeout(Duration::from_secs(1), writer.read(&mut [0]))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert!(matches!(
        closed(&logs).await.failure,
        Some(DockerError::Protocol)
    ));
    assert!(logs.begin(AttemptId::default(), deadline()).await.is_err());
    assert!(logs.shutdown().await.is_err());
    assert!(logs.shutdown().await.is_err());
}
#[tokio::test]
async fn test_should_reject_cross_owner_capabilities_and_revoke_the_originating_window() {
    let (_first_writer, first_reader) = duplex(8);
    let (_second_writer, second_reader) = duplex(8);
    let mut first = logs(first_reader);
    let mut second = logs(second_reader);
    let attempt = AttemptId::default();
    let foreign = first.begin(attempt, deadline()).await.unwrap();
    let own = second.begin(attempt, deadline()).await.unwrap();
    assert!(matches!(
        second.finish(foreign, deadline()).await,
        Err(DockerError::Protocol)
    ));
    assert!(matches!(
        closed(&first).await.failure,
        Some(DockerError::Protocol)
    ));
    assert!(matches!(
        second.begin(AttemptId::default(), deadline()).await,
        Err(DockerError::Stopped)
    ));
    drop(own);
    assert!(closed(&second).await.failure.is_some());
    assert!(first.shutdown().await.is_err());
    assert!(second.shutdown().await.is_err());
}
#[tokio::test]
async fn test_should_reject_a_stale_finish_without_closing_the_current_window() {
    let (_writer, reader) = duplex(8);
    let mut logs = logs(reader);
    let old = logs.begin(AttemptId::default(), deadline()).await.unwrap();
    let key = old.key;
    logs.finish(old, deadline()).await.unwrap();
    let current = logs.begin(AttemptId::default(), deadline()).await.unwrap();
    let (_guard, receipt) = ReceiptGuard::new(deadline());
    assert!(matches!(
        logs.request(Action::Finish(key, receipt), deadline()).await,
        Err(DockerError::Protocol)
    ));
    assert!(
        logs.finish(current, deadline())
            .await
            .unwrap()
            .tail
            .is_empty()
    );
    logs.shutdown().await.unwrap();
}
#[tokio::test]
async fn test_should_poison_window_mutation_after_an_overlapping_begin() {
    let (_writer, reader) = duplex(8);
    let mut logs = logs(reader);
    let first = logs.begin(AttemptId::default(), deadline()).await.unwrap();
    assert!(matches!(
        logs.begin(AttemptId::default(), deadline()).await,
        Err(DockerError::Busy)
    ));
    assert!(matches!(
        logs.finish(first, deadline()).await,
        Err(DockerError::Stopped)
    ));
    assert!(closed(&logs).await.failure.is_some());
    assert!(logs.shutdown().await.is_err());
}
#[tokio::test]
async fn test_should_preserve_partial_record_io_across_begin_and_finish_commands() {
    let (mut writer, reader) = duplex(1);
    let mut logs = logs(reader);
    let frame = record(1, b"fragmented");
    writer.write_all(frame.get(..4).unwrap()).await.unwrap();
    let first = logs.begin(AttemptId::default(), deadline()).await.unwrap();
    assert!(
        logs.finish(first, deadline())
            .await
            .unwrap()
            .tail
            .is_empty()
    );
    let second = logs.begin(AttemptId::default(), deadline()).await.unwrap();
    writer.write_all(frame.get(4..).unwrap()).await.unwrap();
    writer.shutdown().await.unwrap();
    closed(&logs).await;
    assert_eq!(
        logs.finish(second, deadline()).await.unwrap().tail.as_ref(),
        b"fragmented"
    );
    logs.shutdown().await.unwrap();
}
#[rstest::rstest]
#[case::begin(false)]
#[case::finish(true)]
#[tokio::test]
async fn test_should_retire_a_mutation_whose_ready_reply_is_abandoned(#[case] finishing: bool) {
    let (mut writer, reader) = duplex(8);
    let mut logs = logs(reader);
    let sender = logs.requests.clone();
    let window = if finishing {
        Some(logs.begin(AttemptId::default(), deadline()).await.unwrap())
    } else {
        None
    };
    if let Some(window) = window {
        let mut finish = Box::pin(logs.finish(window, deadline()));
        assert!(
            finish
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        probe(&sender).await;
        drop(finish);
    } else {
        let mut begin = Box::pin(logs.begin(AttemptId::default(), deadline()));
        assert!(
            begin
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        probe(&sender).await;
        drop(begin);
    }
    assert_eq!(
        timeout(Duration::from_secs(1), writer.read(&mut [0]))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert!(closed(&logs).await.failure.is_some());
    assert!(matches!(
        logs.begin(AttemptId::default(), deadline()).await,
        Err(DockerError::Stopped)
    ));
    assert!(logs.shutdown().await.is_err());
}
#[rstest::rstest]
#[case::begin(false)]
#[case::finish(true)]
#[tokio::test(start_paused = true)]
async fn test_should_expire_a_ready_mutation_receipt_while_its_public_future_remains_unpolled(
    #[case] finishing: bool,
) {
    let (mut writer, reader) = duplex(8);
    let mut logs = logs(reader);
    let sender = logs.requests.clone();
    let until = Instant::now() + Duration::from_secs(1);
    let window = if finishing {
        Some(logs.begin(AttemptId::default(), deadline()).await.unwrap())
    } else {
        None
    };
    if let Some(window) = window {
        let mut finish = Box::pin(logs.finish(window, until));
        assert!(
            finish
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        probe(&sender).await;
        advance(Duration::from_secs(1)).await;
        assert_eq!(
            timeout(Duration::from_secs(1), writer.read(&mut [0]))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        assert!(probe(&sender).await.failure.is_some());
        assert!(matches!(finish.await, Err(DockerError::Deadline)));
    } else {
        let mut begin = Box::pin(logs.begin(AttemptId::default(), until));
        assert!(
            begin
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        probe(&sender).await;
        advance(Duration::from_secs(1)).await;
        assert_eq!(
            timeout(Duration::from_secs(1), writer.read(&mut [0]))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        assert!(probe(&sender).await.failure.is_some());
        assert!(matches!(begin.await, Err(DockerError::Deadline)));
    }
    assert!(matches!(
        logs.begin(AttemptId::default(), deadline()).await,
        Err(DockerError::Stopped)
    ));
    assert!(logs.shutdown().await.is_err());
}
#[tokio::test]
async fn test_should_retain_capture_failure_when_finishing_an_active_window() {
    let (mut writer, reader) = duplex(8);
    let mut logs = logs(reader);
    let window = logs.begin(AttemptId::default(), deadline()).await.unwrap();
    writer.write_all(&[3, 0, 0, 0, 0, 0, 0, 0]).await.unwrap();
    assert!(closed(&logs).await.failure.is_some());
    assert!(matches!(
        logs.finish(window, deadline()).await,
        Err(DockerError::Protocol)
    ));
    assert!(logs.shutdown().await.is_err());
    assert!(logs.shutdown().await.is_err());
}

#[tokio::test(start_paused = true)]
async fn test_should_preserve_an_on_time_consumption_receipt_processed_after_its_deadline() {
    let mut state = Capture::new(TAIL).unwrap();
    let (window, active) = LogWindow::new(AttemptId::default());
    let until = Instant::now() + Duration::from_secs(1);
    let (mut guard, receipt) = ReceiptGuard::new(until);
    let (reply, response) = oneshot::channel();
    state.reply(Request {
        action: Action::Begin(active, receipt),
        deadline: until,
        reply,
    });
    response.await.unwrap().unwrap();
    guard.consume(until).unwrap();
    advance(Duration::from_secs(2)).await;
    assert!(!state.reconcile());
    assert!(state.failure.is_none());
    assert!(state.active.is_some());
    drop(window);
    assert!(state.reconcile());
}
#[rstest::rstest]
#[case::expired(false)]
#[case::abandoned(true)]
#[tokio::test]
async fn test_should_reject_unissued_window_requests_without_mutating_capture(
    #[case] abandoned: bool,
) {
    let mut state = Capture::new(TAIL).unwrap();
    let (window, active) = LogWindow::new(AttemptId::default());
    let until = if abandoned {
        deadline()
    } else {
        Instant::now()
    };
    let (_guard, receipt) = ReceiptGuard::new(until);
    let (reply, response) = oneshot::channel();
    let response = if abandoned {
        drop(response);
        None
    } else {
        Some(response)
    };
    state.reply(Request {
        action: Action::Begin(active, receipt),
        deadline: until,
        reply,
    });
    if let Some(response) = response {
        assert!(matches!(
            response.await.unwrap(),
            Err(DockerError::Deadline)
        ));
    }
    assert!(state.active.is_none());
    assert!(state.receipt.is_none());
    assert!(state.failure.is_none());
    drop(window);
}
#[rstest::rstest]
#[case::begin(false)]
#[case::finish(true)]
#[tokio::test]
async fn test_should_reject_queued_mutations_when_priority_shutdown_wins(#[case] finishing: bool) {
    let (_writer, reader) = duplex(8);
    let mut logs = logs(reader);
    let (window, active) = LogWindow::new(AttemptId::default());
    let own = if finishing {
        Some(logs.begin(AttemptId::default(), deadline()).await.unwrap())
    } else {
        None
    };
    let (_guard, receipt) = ReceiptGuard::new(deadline());
    let action = if let Some(own) = &own {
        Action::Finish(own.key, receipt)
    } else {
        Action::Begin(active, receipt)
    };
    let (reply, response) = oneshot::channel();
    logs.requests
        .try_send(Request {
            action,
            deadline: deadline(),
            reply,
        })
        .unwrap_or_else(|_| panic!("test mutation admission"));
    logs.shutdown().await.unwrap();
    assert!(matches!(response.await.unwrap(), Err(DockerError::Stopped)));
    drop((window, own));
}
#[tokio::test]
async fn test_should_serve_bounded_window_commands_without_starving_flooded_output_or_shutdown() {
    let (mut writer, reader) = duplex(64 * 1024);
    let mut logs = logs(reader);
    let producer = tokio::spawn(async move {
        let frame = record(1, &vec![b'x'; 16384]);
        while writer.write_all(&frame).await.is_ok() {
            tokio::task::yield_now().await;
        }
    });
    for _ in 0..16 {
        let window = logs.begin(AttemptId::default(), deadline()).await.unwrap();
        let tail = logs.finish(window, deadline()).await.unwrap();
        assert!(tail.tail.len() <= TAIL);
    }
    timeout(Duration::from_secs(1), logs.shutdown())
        .await
        .unwrap()
        .unwrap();
    timeout(Duration::from_secs(1), producer)
        .await
        .unwrap()
        .unwrap();
}
