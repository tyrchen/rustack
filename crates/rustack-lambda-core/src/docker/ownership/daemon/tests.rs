use std::{
    fs::Permissions,
    os::unix::fs::{PermissionsExt, symlink},
    sync::{Arc, mpsc::channel as blocking_channel},
    time::Duration,
};

use rstest::rstest;
use sha2::{Digest, Sha256};
use tokio::{
    fs::{read, remove_file, set_permissions, write},
    spawn,
    sync::oneshot::channel as oneshot_channel,
    time::{Instant, timeout},
};
use uuid::Uuid;

use super::{BYTES, DaemonCodec};
use crate::docker::{
    DirectoryLease, DirectoryLeaseError, EnvironmentPlan, ImagePinScope,
    binding::DaemonIdentity,
    ownership::{
        ContainerRecord, ImageRecord, NAMESPACE,
        snapshot::{Codec, write as write_snapshot},
    },
    plan::{
        ContainerRole,
        tests::{image_with, policy, revision},
    },
    tests::DAEMON_ID,
};

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
fn identity() -> DaemonIdentity {
    DaemonIdentity::try_from(DAEMON_ID).unwrap()
}

#[rstest]
#[case(0)]
#[case(1)]
#[case(2)]
#[case(3)]
#[case(4)]
#[case(5)]
#[case(6)]
fn test_should_reject_damaged_daemon_binding(#[case] corruption: u8) {
    let owner = Uuid::new_v4();
    let mut bytes = DaemonCodec::encode(owner, &[identity()]).unwrap();
    assert_eq!(bytes.len(), BYTES);
    assert_eq!(DaemonCodec::decode(&bytes, owner).unwrap(), [identity()]);
    match corruption {
        0 => bytes[0] ^= 1,
        1 => bytes[8] ^= 1,
        2 => bytes[24..40].fill(0),
        3 => {
            bytes.pop();
        }
        4 => bytes.push(0),
        5 => bytes[40] ^= 1,
        _ => bytes[8..24].fill(0),
    }
    if matches!(corruption, 0 | 1 | 2 | 6) {
        let sum = Sha256::digest(&bytes[..40]);
        bytes[40..].copy_from_slice(&sum);
    }
    assert!(matches!(
        DaemonCodec::decode(&bytes, owner),
        Err(DirectoryLeaseError::InvalidState)
    ));
}

#[tokio::test]
async fn test_should_persist_immutable_identity_across_reacquisition_and_torn_staging() {
    let root = tempfile::tempdir().unwrap();
    let directory = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    let owner = directory.owner_id();
    directory.bind_daemon(identity(), deadline()).await.unwrap();
    let path = root.path().join(NAMESPACE);
    let before = read(path.join("daemon.v1")).await.unwrap();
    write(path.join("daemon.pending"), b"torn").await.unwrap();
    set_permissions(path.join("daemon.pending"), Permissions::from_mode(0o600))
        .await
        .unwrap();
    directory.bind_daemon(identity(), deadline()).await.unwrap();
    drop(directory);
    let directory = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    assert_eq!(directory.owner_id(), owner);
    directory.bind_daemon(identity(), deadline()).await.unwrap();
    let other = DaemonIdentity::from_uuid(Uuid::new_v4()).unwrap();
    assert!(matches!(
        directory.bind_daemon(other, deadline()).await,
        Err(DirectoryLeaseError::InvalidState)
    ));
    assert_eq!(read(path.join("daemon.v1")).await.unwrap(), before);
}

#[rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn test_should_reject_missing_binding_over_any_journal(#[case] container: bool) {
    let root = tempfile::tempdir().unwrap();
    let directory = DirectoryLease::acquire(root.path().into(), deadline())
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
        directory
            .prepare_container(
                ContainerRecord::from_plan(&plan, ContainerRole::Relay).unwrap(),
                deadline(),
            )
            .await
            .unwrap();
    } else {
        let pin = ImagePinScope::new(directory.clone()).allocate().unwrap();
        let record =
            ImageRecord::new(&pin, &format!("sha256:{}", "a".repeat(64)).parse().unwrap()).unwrap();
        directory.prepare_image(record, deadline()).await.unwrap();
    }
    assert!(matches!(
        directory.bind_daemon(identity(), deadline()).await,
        Err(DirectoryLeaseError::InvalidState)
    ));
    assert!(!root.path().join(NAMESPACE).join("daemon.v1").exists());
}

#[rstest]
#[case("daemon.v1")]
#[case("daemon.pending")]
#[tokio::test]
async fn test_should_refuse_unsafe_or_oversized_binding_files(#[case] name: &str) {
    let root = tempfile::tempdir().unwrap();
    let directory = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    let outside = root.path().join("outside");
    write(&outside, b"preserve").await.unwrap();
    let path = root.path().join(NAMESPACE).join(name);
    symlink(&outside, &path).unwrap();
    assert!(directory.bind_daemon(identity(), deadline()).await.is_err());
    remove_file(&path).await.unwrap();
    write(&path, vec![0; BYTES + 1]).await.unwrap();
    set_permissions(&path, Permissions::from_mode(0o600))
        .await
        .unwrap();
    assert!(directory.bind_daemon(identity(), deadline()).await.is_err());
    assert_eq!(read(outside).await.unwrap(), b"preserve");
}

#[tokio::test]
async fn test_should_retain_published_binding_and_serial_lease_after_observer_loss() {
    let root = tempfile::tempdir().unwrap();
    let directory = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    let retained = directory.clone();
    let (entered, observed) = oneshot_channel();
    let (release, barrier) = blocking_channel();
    let task = spawn(async move {
        retained
            .transaction(deadline(), move |directory, owner, control| {
                write_snapshot::<DaemonCodec>(directory, owner, &[identity()], control)?;
                entered.send(()).unwrap();
                barrier.recv().unwrap();
                Ok(())
            })
            .await
    });
    observed.await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let copy = directory.clone();
    let other = DaemonIdentity::from_uuid(Uuid::new_v4()).unwrap();
    let mut retry = spawn(async move { copy.bind_daemon(other, deadline()).await });
    assert!(
        timeout(Duration::from_millis(30), &mut retry)
            .await
            .is_err()
    );
    drop(directory);
    assert!(matches!(
        DirectoryLease::acquire(root.path().into(), deadline()).await,
        Err(DirectoryLeaseError::InUse)
    ));
    release.send(()).unwrap();
    assert!(matches!(
        retry.await.unwrap(),
        Err(DirectoryLeaseError::InvalidState)
    ));
    let next = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    next.bind_daemon(identity(), deadline()).await.unwrap();
}
