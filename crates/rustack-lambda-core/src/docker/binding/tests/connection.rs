use std::{sync::Arc, time::Duration};

use bytes::Bytes;
use http::{Method, StatusCode};
use rstest::rstest;
use serde_json::{json, to_vec};
use tokio::{
    io::AsyncWriteExt,
    net::UnixListener,
    spawn,
    time::{advance, pause, timeout},
};

use super::deadline;
use crate::docker::{
    DirectoryLease, DockerError, Operation, Owner, RawRequest,
    binding::{DaemonBinding, DaemonIdentity},
    mutation::MutationGuard,
    tests::{DAEMON_ID, read_request},
};

#[rstest]
#[case::same_connection(0)]
#[case::checked_peer_closed(1)]
#[case::invalid_json(2)]
#[case::unavailable(3)]
#[case::expired_before_issue(4)]
#[case::missing_identity(5)]
#[case::null_identity(6)]
#[tokio::test]
async fn test_should_fence_issue_on_the_checked_connection(#[case] mode: u8) {
    let root = tempfile::tempdir().unwrap();
    let directory = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    let identity = DaemonIdentity::try_from(DAEMON_ID).unwrap();
    directory.bind_daemon(identity, deadline()).await.unwrap();
    let path = root.path().join("fixture.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let (mut owner, mut raw) = Owner::start(path);
    raw.binding = Some(Arc::new(DaemonBinding {
        identity,
        directory,
    }));
    let guard = MutationGuard::new();
    let receipt = guard.receipt();
    let request_port = raw.clone();
    let task = spawn(async move {
        request_port
            .request(
                RawRequest {
                    pull: None,
                    binding: None,
                    mutation: Some(guard),
                    operation: Operation::TagPin,
                    method: Method::POST,
                    target: "/v1.53/images/proof/tag".into(),
                    body: Bytes::new(),
                    expected: StatusCode::CREATED,
                },
                deadline(),
            )
            .await
    });
    let (stream, _) = timeout(Duration::from_secs(5), listener.accept())
        .await
        .unwrap()
        .unwrap();
    let mut incoming = read_request(stream).await.unwrap();
    assert!(incoming.head.starts_with("GET /info "));
    assert!(
        incoming
            .head
            .to_ascii_lowercase()
            .contains("connection: keep-alive")
    );
    let body = match mode {
        2 => b"{".to_vec(),
        5 => b"{}".to_vec(),
        6 => br#"{"ID":null}"#.to_vec(),
        _ => to_vec(&json!({"ID": DAEMON_ID})).unwrap(),
    };
    if mode == 4 {
        pause();
        advance(Duration::from_secs(20)).await;
    }
    if mode == 1 {
        incoming.reply(200, &body).await;
    } else {
        let status = if mode == 3 { 500 } else { 200 };
        let head = format!(
            "HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
            body.len()
        );
        // Expiry can close the checked connection before the fixture writes.
        let _ = incoming.stream.write_all(head.as_bytes()).await;
        let _ = incoming.stream.write_all(&body).await;
        if mode == 0 {
            let actual = read_request(incoming.stream).await.unwrap();
            assert!(actual.head.starts_with("POST /v1.53/images/proof/tag "));
            actual.reply(201, b"").await;
        } else {
            assert!(read_request(incoming.stream).await.is_err());
        }
    }
    let result = task.await.unwrap();
    if mode == 0 {
        assert!(result.is_ok());
        assert!(!receipt.definitely_unissued());
    } else {
        assert!(result.is_err());
        assert!(receipt.definitely_unissued());
        if mode == 4 {
            assert!(matches!(result, Err(DockerError::Deadline)));
        }
    }
    assert!(
        timeout(Duration::from_millis(20), listener.accept())
            .await
            .is_err(),
        "checked connection was replayed on another socket"
    );
    owner.shutdown().await.unwrap();
}
