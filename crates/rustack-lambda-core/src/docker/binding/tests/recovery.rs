use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use http::StatusCode;
use rstest::rstest;
use tokio::{
    fs::read,
    time::{Instant, sleep, timeout},
};

use super::deadline;
use crate::{
    artifact::Architecture,
    docker::{
        DirectoryLease, DockerError, EnvironmentError, EnvironmentPlan, ImageLeaseConfig,
        ImageLeaseManager, ImagePinScope,
        lease::tests::{FakePins, Mode},
        ownership::{ContainerRecord, ImageRecord},
        plan::{
            ContainerRole,
            tests::{image_with, policy, revision},
        },
        reconcile_containers, reconcile_image_pins,
        tests::Daemon,
    },
};

#[rstest]
#[case::known_container(true)]
#[case::committed_alias(false)]
#[tokio::test]
async fn test_should_never_treat_identity_404_as_resource_absence(#[case] container: bool) {
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
    if container {
        let plan = EnvironmentPlan::new(
            directory.clone(),
            image_with("1000", vec![]),
            Arc::new(revision()),
            policy(),
        )
        .unwrap();
        let record = ContainerRecord::from_plan(&plan, ContainerRole::Relay).unwrap();
        directory
            .prepare_container(record.clone(), deadline())
            .await
            .unwrap();
        directory
            .know_container(record, "a".repeat(64).parse().unwrap(), deadline())
            .await
            .unwrap();
    } else {
        let pin = ImagePinScope::new(directory.clone()).allocate().unwrap();
        let record =
            ImageRecord::new(&pin, &format!("sha256:{}", "a".repeat(64)).parse().unwrap()).unwrap();
        directory
            .prepare_image(record.clone(), deadline())
            .await
            .unwrap();
        directory.commit_image(record, deadline()).await.unwrap();
    }
    let path = root
        .path()
        .join(".rustack-lambda-docker")
        .join(if container {
            "containers.v1"
        } else {
            "pins.v1"
        });
    let before = read(&path).await.unwrap();
    drop(bound);
    drop(directory);
    let next = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    let bound = wire.port().bind_directory(&next, deadline()).await.unwrap();
    daemon.identity_status.send_replace(404);
    if container {
        assert!(
            matches!(reconcile_containers(&next, &bound, deadline()).await,
            Err(EnvironmentError::Docker(DockerError::IdentityCheck(source)))
                if matches!(source.as_ref(), DockerError::Status(StatusCode::NOT_FOUND)))
        );
        assert_eq!(next.container_records(deadline()).await.unwrap().len(), 1);
    } else {
        assert!(
            matches!(reconcile_image_pins(&next, &bound, deadline()).await,
            Err(DockerError::IdentityCheck(source))
                if matches!(source.as_ref(), DockerError::Status(StatusCode::NOT_FOUND)))
        );
        assert_eq!(next.image_records(deadline()).await.unwrap().len(), 1);
    }
    assert_eq!(read(path).await.unwrap(), before);
    assert!(
        timeout(Duration::from_millis(20), daemon.requests.recv())
            .await
            .is_err()
    );
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_retain_live_pin_capacity_when_cleanup_identity_returns_404() {
    let root = tempfile::tempdir().unwrap();
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
    let mut owner = ImageLeaseManager::start(
        bound,
        ImagePinScope::new(directory.clone()),
        ImageLeaseConfig::new(1, 1, Duration::from_secs(1)).unwrap(),
    )
    .unwrap();
    let port = owner.port();
    let image = port
        .prepare(
            &"worker:v1".parse().unwrap(),
            Architecture::Arm64,
            None,
            deadline(),
        )
        .await
        .unwrap();
    assert_eq!(directory.image_records(deadline()).await.unwrap().len(), 1);
    fake.identity_status.send_replace(404);
    drop(image);
    timeout(Duration::from_secs(5), async {
        while port.status().quarantined == 0 {
            assert_eq!(port.status().pins, 1);
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        port.wait_empty(Instant::now() + Duration::from_millis(100))
            .await
            .is_err()
    );
    assert_eq!(port.status().pins, 1);
    assert!(port.status().quarantined > 0);
    assert_eq!(directory.image_records(deadline()).await.unwrap().len(), 1);
    assert_eq!(fake.stats.deletes.load(Ordering::Acquire), 0);
    fake.identity_status.send_replace(200);
    owner.shutdown(deadline()).await.unwrap();
    assert_eq!(port.status().pins, 0);
    assert_eq!(directory.image_records(deadline()).await.unwrap(), []);
    fake.close().await;
}
