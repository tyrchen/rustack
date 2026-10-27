use std::time::Duration;

use bytes::Bytes;
use http::{Method, StatusCode};
use rstest::rstest;
use tokio::{
    fs::read,
    spawn,
    time::{Instant, timeout},
};

use super::DaemonIdentity;
use crate::docker::{
    DirectoryLease, DirectoryLeaseError, DockerError, EnvironmentConfig, EnvironmentManager,
    ImageLeaseConfig, ImageLeaseManager, ImagePinScope, Operation, RawRequest,
    mutation::MutationGuard,
    reconcile_containers, reconcile_image_pins,
    tests::{DAEMON_ID, Daemon},
};

mod connection;
mod real;
mod recovery;

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}

#[rstest]
#[case("")]
#[case("00000000-0000-0000-0000-000000000000")]
#[case("44CA5378-c28c-4bac-b6ed-2b298f02ecb5")]
#[case("44ca5378c28c4bacb6ed2b298f02ecb5")]
#[case("{44ca5378-c28c-4bac-b6ed-2b298f02ecb5}")]
#[case("44ca5378-c28c-4bac-b6ed-2b298f02ecb5\n")]
#[case("33Z5:BE33:KSMC:S4GY:GRDO:EH5E")]
#[case("éééééééééééééééééé")]
fn test_should_reject_noncanonical_or_unproven_identity(#[case] value: &str) {
    assert!(matches!(
        DaemonIdentity::try_from(value),
        Err(DockerError::Protocol)
    ));
}

#[test]
fn test_should_accept_and_redact_canonical_installation_identity() {
    let identity = DaemonIdentity::try_from(DAEMON_ID).unwrap();
    assert_eq!(identity.uuid().to_string(), DAEMON_ID);
    assert!(!format!("{identity:?}").contains(DAEMON_ID));
    assert_eq!(
        format!("{:?}", DockerError::DaemonBinding),
        "Docker daemon binding failed"
    );
}

#[tokio::test]
async fn test_should_require_exact_bound_lease_for_runtime_and_recovery() {
    let root = tempfile::tempdir().unwrap();
    let directory = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    let other_root = tempfile::tempdir().unwrap();
    let other = DirectoryLease::acquire(other_root.path().into(), deadline())
        .await
        .unwrap();
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let raw = wire.port();
    assert!(matches!(
        ImageLeaseManager::start(
            raw.clone(),
            ImagePinScope::new(directory.clone()),
            ImageLeaseConfig::default()
        ),
        Err(DockerError::DaemonBinding)
    ));
    assert!(
        EnvironmentManager::start(raw.clone(), directory.clone(), EnvironmentConfig::default())
            .is_err()
    );
    assert!(
        reconcile_containers(&directory, &raw, deadline())
            .await
            .is_err()
    );
    assert!(matches!(
        reconcile_image_pins(&directory, &raw, deadline()).await,
        Err(DockerError::DaemonBinding)
    ));
    let bound = raw.bind_directory(&directory, deadline()).await.unwrap();
    assert!(bound.require_directory(&other).is_err());
    assert!(
        reconcile_containers(&other, &bound, deadline())
            .await
            .is_err()
    );
    assert!(
        reconcile_image_pins(&other, &bound, deadline())
            .await
            .is_err()
    );
    assert_eq!(
        reconcile_containers(&directory, &bound, deadline())
            .await
            .unwrap()
            .remaining,
        0
    );
    assert_eq!(
        reconcile_image_pins(&directory, &bound, deadline())
            .await
            .unwrap()
            .remaining,
        0
    );
    assert!(
        timeout(Duration::from_millis(20), daemon.requests.recv())
            .await
            .is_err()
    );
    let mut images = ImageLeaseManager::start(
        bound.clone(),
        ImagePinScope::new(directory.clone()),
        ImageLeaseConfig::default(),
    )
    .unwrap();
    images.shutdown(deadline()).await.unwrap();
    drop(images);
    let mut environments = EnvironmentManager::start(
        bound.clone(),
        directory.clone(),
        EnvironmentConfig::default(),
    )
    .unwrap();
    environments.shutdown(deadline()).await.unwrap();
    drop(environments);
    drop(directory);
    assert!(matches!(
        DirectoryLease::acquire(root.path().into(), deadline()).await,
        Err(DirectoryLeaseError::InUse)
    ));
    drop(bound);
    let next = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    let bound = raw.bind_directory(&next, deadline()).await.unwrap();
    bound.require_directory(&next).unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[rstest]
#[case("44ca5378-c28c-4bac-b6ed-2b298f02ecb6", false)]
#[case("invalid", true)]
#[tokio::test]
async fn test_should_refuse_changed_peer_before_mutation_issuance(
    #[case] id: &str,
    #[case] malformed: bool,
) {
    let root = tempfile::tempdir().unwrap();
    let directory = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let bound = wire
        .port()
        .bind_directory(&directory, deadline())
        .await
        .unwrap();
    let before = read(root.path().join(".rustack-lambda-docker/daemon.v1"))
        .await
        .unwrap();
    daemon.identity.send_replace(id.to_owned());
    let guard = MutationGuard::new();
    let receipt = guard.receipt();
    let result = bound
        .raw
        .request(
            RawRequest {
                binding: None,
                mutation: Some(guard),
                operation: Operation::TagPin,
                method: Method::POST,
                target: "/v1.53/images/test/tag".into(),
                body: Bytes::new(),
                expected: StatusCode::CREATED,
            },
            deadline(),
        )
        .await;
    assert!(matches!(result, Err(DockerError::IdentityCheck(source)) if
        if malformed { matches!(source.as_ref(), DockerError::Protocol) }
        else { matches!(source.as_ref(), DockerError::DaemonBinding) }));
    assert!(receipt.definitely_unissued());
    assert!(
        timeout(Duration::from_millis(20), daemon.requests.recv())
            .await
            .is_err()
    );
    assert_eq!(
        read(root.path().join(".rustack-lambda-docker/daemon.v1"))
            .await
            .unwrap(),
        before
    );
    daemon.identity.send_replace(DAEMON_ID.into());
    let key = "a".repeat(64).parse().unwrap();
    let (result, ()) = tokio::join!(bound.inspect_container(&key, deadline()), async {
        daemon.next().await.reply(404, b"{}").await;
    });
    assert!(matches!(
        result,
        Err(DockerError::Status(StatusCode::NOT_FOUND))
    ));
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_refuse_changed_installation_before_rebinding_directory() {
    let root = tempfile::tempdir().unwrap();
    let directory = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    let daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    drop(
        wire.port()
            .bind_directory(&directory, deadline())
            .await
            .unwrap(),
    );
    drop(directory);
    let next = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    daemon
        .identity
        .send_replace("44ca5378-c28c-4bac-b6ed-2b298f02ecb6".into());
    assert!(matches!(
        wire.port().bind_directory(&next, deadline()).await,
        Err(DockerError::Ownership(_))
    ));
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_retain_exact_directory_through_abandoned_inflight_request() {
    let root = tempfile::tempdir().unwrap();
    let directory = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let bound = wire
        .port()
        .bind_directory(&directory, deadline())
        .await
        .unwrap();
    let task = spawn(async move {
        bound
            .inspect_container(&"a".repeat(64).parse().unwrap(), deadline())
            .await
    });
    let held = daemon.next().await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    drop(directory);
    assert!(matches!(
        DirectoryLease::acquire(root.path().into(), deadline()).await,
        Err(DirectoryLeaseError::InUse)
    ));
    held.reply(404, b"{}").await;
    wire.shutdown().await.unwrap();
    let _next = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    daemon.close().await;
}
