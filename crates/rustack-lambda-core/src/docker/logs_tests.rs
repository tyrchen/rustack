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
