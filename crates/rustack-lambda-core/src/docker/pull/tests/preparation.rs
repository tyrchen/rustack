use std::time::Duration;

use http::StatusCode;
use tempfile::tempdir;
use tokio::{
    spawn,
    time::{Instant, timeout},
};

use super::policy;
use crate::{
    artifact::{Architecture, ArtifactError},
    docker::{
        DirectoryLease, DockerError, ImageLeaseConfig, ImageLeaseManager, ImagePinScope,
        lease::tests::{FakePins, Mode},
        tests::Daemon,
    },
};

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}

#[tokio::test]
async fn test_should_never_pull_when_a_local_image_is_already_valid() {
    let root = tempdir().unwrap();
    let directory = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    let fake = FakePins::new(Mode::Normal).await;
    let bound = fake
        .wire
        .port()
        .bind_directory(&directory, deadline())
        .await
        .unwrap();
    let mut owner = ImageLeaseManager::start_with_pull(
        bound,
        ImagePinScope::new(directory),
        ImageLeaseConfig::default(),
        policy("registry.example"),
    )
    .unwrap();
    // FakePins only serves inspection/tag/retirement, so an unexpected pull fails.
    let image = owner
        .port()
        .prepare(
            &"worker:v1".parse().unwrap(),
            Architecture::Arm64,
            None,
            deadline(),
        )
        .await
        .unwrap();
    assert!(image.is_live());
    drop(image);
    owner.shutdown(deadline()).await.unwrap();
    fake.close().await;
}

#[tokio::test]
async fn test_should_pull_only_after_initial_missing_root_and_never_publish_invalid_post_pull_image()
 {
    let root = tempdir().unwrap();
    let directory = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let port = wire
        .port()
        .bind_directory(&directory, deadline())
        .await
        .unwrap();
    let mut owner = ImageLeaseManager::start_with_pull(
        port,
        ImagePinScope::new(directory.clone()),
        ImageLeaseConfig::default(),
        policy("registry.example"),
    )
    .unwrap();
    let port = owner.port();
    let reference = "registry.example/fn:v1".parse().unwrap();
    let (result, ()) = tokio::join!(
        port.prepare(&reference, Architecture::Arm64, None, deadline()),
        async {
            let initial = daemon.next().await;
            assert!(initial.head.contains("/images/"));
            initial.reply(404, b"{}").await;
            let pull = daemon.next().await;
            assert!(pull.head.starts_with("POST /v1.53/images/create?"));
            pull.reply(200, b"{\"status\":\"Pull complete\"}\n").await;
            let recheck = daemon.next().await;
            assert!(recheck.head.starts_with("GET /v1.53/images/"));
            recheck.reply(200, b"{\"Id\":\"not-a-digest\"}").await;
        }
    );
    assert!(result.is_err());
    owner.shutdown(deadline()).await.unwrap();
    assert_eq!(port.status().pins, 0);
    assert_eq!(directory.image_records(deadline()).await.unwrap(), []);
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_not_pull_after_nonmissing_initial_failure_or_unapproved_registry() {
    for status in [StatusCode::FORBIDDEN, StatusCode::NOT_FOUND] {
        let root = tempdir().unwrap();
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
        let mut owner = ImageLeaseManager::start_with_pull(
            bound,
            ImagePinScope::new(directory),
            ImageLeaseConfig::default(),
            policy("allowed.example"),
        )
        .unwrap();
        let port = owner.port();
        let reference = "denied.example/fn:v1".parse().unwrap();
        let (result, ()) = tokio::join!(
            port.prepare(&reference, Architecture::Arm64, None, deadline()),
            async {
                daemon.next().await.reply(status.as_u16(), b"{}").await;
            }
        );
        assert!(result.is_err());
        owner.shutdown(deadline()).await.unwrap();
        assert_eq!(port.status().pins, 0);
        wire.shutdown().await.unwrap();
        daemon.close().await;
    }
}

#[tokio::test]
async fn test_should_keep_preparation_owned_until_cancelled_pull_driver_is_joined() {
    let root = tempdir().unwrap();
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
    let mut owner = ImageLeaseManager::start_with_pull(
        bound,
        ImagePinScope::new(directory.clone()),
        ImageLeaseConfig::default(),
        policy("registry.example"),
    )
    .unwrap();
    let port = owner.port();
    let task = spawn(async move {
        port.prepare(
            &"registry.example/fn:v1".parse().unwrap(),
            Architecture::Arm64,
            None,
            deadline(),
        )
        .await
    });
    daemon.next().await.reply(404, b"{}").await;
    let held = daemon.next().await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    drop(held);
    owner.shutdown(deadline()).await.unwrap();
    assert_eq!(owner.port().status().pins, 0);
    assert_eq!(directory.image_records(deadline()).await.unwrap(), []);
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_not_treat_platform_metadata_missing_as_initial_root_missing() {
    let root = tempdir().unwrap();
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
    let mut owner = ImageLeaseManager::start_with_pull(
        bound,
        ImagePinScope::new(directory.clone()),
        ImageLeaseConfig::default(),
        policy("registry.example"),
    )
    .unwrap();
    let port = owner.port();
    let reference = "registry.example/fn:v1".parse().unwrap();
    let (result, ()) = tokio::join!(
        port.prepare(&reference, Architecture::Arm64, None, deadline()),
        async {
            daemon
                .next()
                .await
                .reply(
                    200,
                    format!("{{\"Id\":\"sha256:{}\"}}", "a".repeat(64)).as_bytes(),
                )
                .await;
            let selected = daemon.next().await;
            assert!(selected.head.starts_with("GET /v1.53/images/sha256"));
            assert!(selected.head.contains("platform="));
            selected.reply(404, b"{}").await;
        }
    );
    assert!(matches!(
        result,
        Err(DockerError::Image(ArtifactError::Inspection))
    ));
    assert!(daemon.requests.try_recv().is_err());
    owner.shutdown(deadline()).await.unwrap();
    assert_eq!(directory.image_records(deadline()).await.unwrap(), []);
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_cancel_owned_pull_when_preparation_waiter_expires() {
    let root = tempdir().unwrap();
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
    let mut owner = ImageLeaseManager::start_with_pull(
        bound,
        ImagePinScope::new(directory.clone()),
        ImageLeaseConfig::default(),
        policy("registry.example"),
    )
    .unwrap();
    let port = owner.port();
    let task = spawn(async move {
        port.prepare(
            &"registry.example/fn:v1".parse().unwrap(),
            Architecture::Arm64,
            None,
            Instant::now() + Duration::from_secs(1),
        )
        .await
    });
    daemon.next().await.reply(404, b"{}").await;
    let held = daemon.next().await;
    assert!(matches!(
        timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap(),
        Err(DockerError::Deadline)
    ));
    owner.shutdown(deadline()).await.unwrap();
    assert_eq!(owner.port().status().pins, 0);
    assert_eq!(directory.image_records(deadline()).await.unwrap(), []);
    drop(held);
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
