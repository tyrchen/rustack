use std::{collections::BTreeMap, sync::Arc, time::Duration};

use base64::{Engine, engine::general_purpose::URL_SAFE};
use http::StatusCode;
use rstest::rstest;
use serde_json::{Value, from_slice};
use tempfile::{TempDir, tempdir};
use tokio::{
    spawn,
    sync::watch,
    time::{Instant, sleep, timeout},
};
use tracing::{Level, instrument::WithSubscriber};

use super::policy;
use crate::{
    artifact::Architecture,
    docker::{
        DirectoryLease, DockerError, DockerPort, DockerWire, ImagePullPolicy, RegistryCredential,
        RegistryCredentials,
        tests::{Daemon, LogWriter},
    },
};

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
async fn fixture() -> (TempDir, Daemon, DockerWire, DockerPort) {
    let root = tempdir().unwrap();
    let directory = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    let daemon = Daemon::new();
    let wire = daemon.wire().await;
    let port = wire
        .port()
        .bind_directory(&directory, deadline())
        .await
        .unwrap();
    (root, daemon, wire, port)
}

#[rstest]
#[case("registry.example/team/worker:v1", "v1")]
#[case("registry.example/team/worker", "latest")]
#[case(
    "registry.example/team/worker@sha256:\
     aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
)]
#[tokio::test]
async fn test_should_encode_explicit_tag_digest_platform_and_selected_secret(
    #[case] source: &str,
    #[case] tag: &str,
) {
    let (_root, mut daemon, mut wire, port) = fixture().await;
    let host = "registry.example".parse().unwrap();
    let credentials = RegistryCredentials::try_from(vec![(
        host,
        RegistryCredential::basic("PRIVATE-USER".into(), "PRIVATE-PASSWORD".into()).unwrap(),
    )])
    .unwrap();
    let policy = ImagePullPolicy::if_missing(
        vec!["registry.example".parse().unwrap()],
        credentials,
        Duration::from_secs(10),
    )
    .unwrap();
    let reference = source.parse().unwrap();
    let (_cancel, cancel) = watch::channel(());
    let (_stop, stop) = watch::channel(false);
    let (result, ()) = tokio::join!(
        policy.pull(&port, &reference, Architecture::Arm64, cancel, stop),
        async {
            let request = daemon.next().await;
            let target = request
                .head
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            let (path, query) = target.split_once('?').unwrap();
            assert_eq!(path, "/v1.53/images/create");
            let query = form_urlencoded::parse(query.as_bytes())
                .into_owned()
                .collect::<BTreeMap<_, _>>();
            assert_eq!(query.len(), 3);
            assert_eq!(query["fromImage"], "registry.example/team/worker");
            assert_eq!(query["tag"], tag);
            assert_eq!(query["platform"], "linux/arm64");
            let auth = request
                .head
                .lines()
                .find_map(|line| {
                    line.split_once(':')
                        .filter(|(name, _)| name.eq_ignore_ascii_case("x-registry-auth"))
                        .map(|(_, value)| value.trim())
                })
                .unwrap();
            let credentials: Value = from_slice(&URL_SAFE.decode(auth).unwrap()).unwrap();
            assert_eq!(credentials["username"], "PRIVATE-USER");
            assert_eq!(credentials["password"], "PRIVATE-PASSWORD");
            request
                .reply(200, b"{\"status\":\"Pull complete\"}\n")
                .await;
        }
    );
    result.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[rstest]
#[case(200,b"{\"errorDetail\":{\"message\":\"SECRET-PEER\"}}\n".as_slice())]
#[case(200,b"{}".as_slice())]
#[case(200,b"".as_slice())]
#[case(404,b"{\"message\":\"SECRET-PEER\"}".as_slice())]
#[tokio::test]
async fn test_should_reject_failed_empty_or_truncated_streams_without_peer_text(
    #[case] status: u16,
    #[case] body: &[u8],
) {
    let (_root, mut daemon, mut wire, port) = fixture().await;
    let policy = policy("registry.example");
    let reference = "registry.example/fn:v1".parse().unwrap();
    let (_cancel, cancel) = watch::channel(());
    let (_stop, stop) = watch::channel(false);
    let (result, ()) = tokio::join!(
        policy.pull(&port, &reference, Architecture::Arm64, cancel, stop),
        async {
            daemon.next().await.reply(status, body).await;
        }
    );
    let error = result.unwrap_err();
    assert!(!format!("{error:?}: {error}").contains("SECRET-PEER"));
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_bound_running_and_queued_pulls_without_starving_cleanup() {
    let (_root, mut daemon, mut wire, port) = fixture().await;
    let policy = policy("registry.example");
    let (cancel, cancelled) = watch::channel(());
    let (_stop, stopped) = watch::channel(false);
    let mut tasks = Vec::new();
    let mut held = Vec::new();
    for _ in 0..4 {
        let (policy, port, cancel, stop) = (
            policy.clone(),
            port.clone(),
            cancelled.clone(),
            stopped.clone(),
        );
        tasks.push(spawn(async move {
            policy
                .pull(
                    &port,
                    &"registry.example/fn:v1".parse().unwrap(),
                    Architecture::Arm64,
                    cancel,
                    stop,
                )
                .await
        }));
        held.push(daemon.next().await);
    }
    for _ in 0..8 {
        let (policy, port, cancel, stop) = (
            policy.clone(),
            port.clone(),
            cancelled.clone(),
            stopped.clone(),
        );
        tasks.push(spawn(async move {
            policy
                .pull(
                    &port,
                    &"registry.example/fn:v1".parse().unwrap(),
                    Architecture::Arm64,
                    cancel,
                    stop,
                )
                .await
        }));
    }
    timeout(Duration::from_secs(3), async {
        while port.raw.pulls.capacity() != 0 {
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        policy
            .pull(
                &port,
                &"registry.example/fn:v1".parse().unwrap(),
                Architecture::Arm64,
                cancelled.clone(),
                stopped.clone()
            )
            .await,
        Err(DockerError::Busy)
    ));
    let key = "a".repeat(64).parse().unwrap();
    let (result, ()) = tokio::join!(port.inspect_container(&key, deadline()), async {
        let request = daemon.next().await;
        assert!(request.head.contains("/containers/"));
        request.reply(404, b"{}").await;
    });
    assert!(matches!(
        result,
        Err(DockerError::Status(StatusCode::NOT_FOUND))
    ));
    drop(cancel);
    drop(held);
    for task in tasks {
        assert!(task.await.unwrap().is_err());
    }
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_cancel_and_join_an_active_pull_before_terminal_receipt() {
    let (root, mut daemon, mut wire, port) = fixture().await;
    let policy = policy("registry.example");
    let (cancel, cancelled) = watch::channel(());
    let (_stop, stopped) = watch::channel(false);
    let call = port.clone();
    let task = spawn(async move {
        policy
            .pull(
                &call,
                &"registry.example/fn:v1".parse().unwrap(),
                Architecture::Arm64,
                cancelled,
                stopped,
            )
            .await
    });
    let held = daemon.next().await;
    drop(cancel);
    assert!(matches!(task.await.unwrap(), Err(DockerError::Cancelled)));
    drop(held);
    drop(port);
    wire.shutdown().await.unwrap();
    drop(wire);
    let _next = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    daemon.close().await;
}

#[rstest]
#[case::deadline(false)]
#[case::owner_stop(true)]
#[tokio::test]
async fn test_should_join_an_active_pull_on_deadline_or_owner_stop(#[case] stopping: bool) {
    let (root, mut daemon, mut wire, port) = fixture().await;
    let policy = ImagePullPolicy::if_missing(
        vec!["registry.example".parse().unwrap()],
        RegistryCredentials::default(),
        Duration::from_secs(1),
    )
    .unwrap();
    let (_cancel, cancel) = watch::channel(());
    let (stop, stopped) = watch::channel(false);
    let task = spawn(async move {
        policy
            .pull(
                &port,
                &"registry.example/fn:v1".parse().unwrap(),
                Architecture::Arm64,
                cancel,
                stopped,
            )
            .await
    });
    let held = daemon.next().await;
    if stopping {
        stop.send_replace(true);
    }
    let error = timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(if stopping {
        matches!(error, DockerError::Stopped)
    } else {
        matches!(error, DockerError::Deadline)
    });
    drop(held);
    wire.shutdown().await.unwrap();
    drop(wire);
    let _directory = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    daemon.close().await;
}

#[rstest]
#[case::abandoned_waiter(false)]
#[case::failed_root(true)]
#[tokio::test]
async fn test_should_retain_accepted_pull_after_waiter_or_root_is_abandoned(#[case] failed: bool) {
    let (root, mut daemon, mut wire, port) = fixture().await;
    let policy = policy("registry.example");
    let (_cancel, cancel) = watch::channel(());
    let (_stop, stop) = watch::channel(false);
    let task = spawn(async move {
        policy
            .pull(
                &port,
                &"registry.example/fn:v1".parse().unwrap(),
                Architecture::Arm64,
                cancel,
                stop,
            )
            .await
    });
    let held = daemon.next().await;
    if failed {
        wire.owner.task.as_ref().unwrap().abort();
        assert!(task.await.unwrap().is_err());
    } else {
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(
            DirectoryLease::acquire(root.path().into(), deadline())
                .await
                .is_err()
        );
    }
    let result = wire.shutdown().await;
    assert!(if failed {
        matches!(result, Err(DockerError::Worker))
    } else {
        result.is_ok()
    });
    drop(held);
    drop(wire);
    let _directory = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_filter_pull_auth_target_progress_and_errors_under_trace() {
    let capture = LogWriter(Arc::default());
    let writer = capture.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(Level::TRACE)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    async {
        tracing::debug!("pull_application_event");
        let (_root, mut daemon, mut wire, port) = fixture().await;
        let host = "private-registry.example".parse().unwrap();
        let credential =
            RegistryCredential::basic("PRIVATE-USERNAME".into(), "PRIVATE-PASSWORD".into())
                .unwrap();
        let credentials = RegistryCredentials::try_from(vec![(host, credential)]).unwrap();
        let policy = ImagePullPolicy::if_missing(
            vec!["private-registry.example".parse().unwrap()],
            credentials,
            Duration::from_secs(10),
        )
        .unwrap();
        let (_cancel, cancel) = watch::channel(());
        let (_stop, stop) = watch::channel(false);
        let source = "private-registry.example/private-source:v1"
            .parse()
            .unwrap();
        let (result, ()) = tokio::join!(
            policy.pull(&port, &source, Architecture::Arm64, cancel, stop),
            async {
                daemon
                    .next()
                    .await
                    .reply(
                        200,
                        b"{\"status\":\"PRIVATE-PROGRESS\"}\n{\"error\":\"PRIVATE-ERROR\"}\n",
                    )
                    .await;
            }
        );
        assert!(matches!(result, Err(DockerError::PullFailed)));
        wire.shutdown().await.unwrap();
        daemon.close().await;
    }
    .with_subscriber(subscriber)
    .await;
    let output = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    assert!(output.contains("pull_application_event"));
    for secret in [
        "private-registry.example",
        "private-source",
        "PRIVATE-USERNAME",
        "PRIVATE-PASSWORD",
        "PRIVATE-PROGRESS",
        "PRIVATE-ERROR",
        "X-Registry-Auth",
        "x-registry-auth",
    ] {
        assert!(!output.contains(secret));
    }
}
