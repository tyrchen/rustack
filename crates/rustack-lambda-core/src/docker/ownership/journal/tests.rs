use std::{
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    path::PathBuf,
    time::Duration,
};

use super::*;
use crate::docker::ownership::NAMESPACE;

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
fn record(run: Uuid) -> ContainerRecord {
    ContainerRecord {
        run,
        environment: Uuid::new_v4(),
        resource: Uuid::new_v4(),
        function: true,
        image: [2; 32],
        known: None,
    }
}
async fn lease(root: &tempfile::TempDir) -> DirectoryLease {
    DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap()
}
fn path(root: &tempfile::TempDir, file: &str) -> PathBuf {
    root.path().join(NAMESPACE).join(file)
}

#[tokio::test]
async fn test_should_persist_unknown_known_and_settled_across_reacquisition() {
    let root = tempfile::tempdir().unwrap();
    let first = lease(&root).await;
    let entry = record(first.run_id());
    first
        .prepare_container(entry.clone(), deadline())
        .await
        .unwrap();
    let bytes = tokio::fs::read(path(&root, PUBLISHED)).await.unwrap();
    assert_eq!(bytes.len(), 171);
    assert_eq!(
        decode(&bytes, first.owner_id()).unwrap(),
        vec![entry.clone()]
    );
    drop(first);
    let second = lease(&root).await;
    assert_ne!(second.run_id(), entry.run);
    assert_eq!(
        second.container_records(deadline()).await.unwrap(),
        vec![entry.clone()]
    );
    second
        .know_container(entry.clone(), "a".repeat(64).parse().unwrap(), deadline())
        .await
        .unwrap();
    drop(second);
    let third = lease(&root).await;
    assert_eq!(
        third.container_records(deadline()).await.unwrap()[0].known,
        Some([0xaa; 32])
    );
    third
        .settle_container(entry.clone(), deadline())
        .await
        .unwrap();
    third.settle_container(entry, deadline()).await.unwrap();
    drop(third);
    assert_eq!(
        lease(&root)
            .await
            .container_records(deadline())
            .await
            .unwrap(),
        []
    );
}
#[tokio::test]
async fn test_should_serialize_concurrent_snapshots_without_losing_records() {
    let root = tempfile::tempdir().unwrap();
    let lease = lease(&root).await;
    let mut jobs = tokio::task::JoinSet::new();
    for _ in 0..32 {
        let lease = lease.clone();
        jobs.spawn(async move {
            lease
                .prepare_container(record(lease.run_id()), deadline())
                .await
        });
    }
    while let Some(result) = jobs.join_next().await {
        result.unwrap().unwrap();
    }
    assert_eq!(lease.container_records(deadline()).await.unwrap().len(), 32);
    let metadata = tokio::fs::metadata(path(&root, PUBLISHED)).await.unwrap();
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    assert_eq!(metadata.nlink(), 1);
}
#[tokio::test]
async fn test_should_bound_records_and_reject_cross_run_or_changed_identity() {
    let root = tempfile::tempdir().unwrap();
    let lease = lease(&root).await;
    let first = record(lease.run_id());
    lease
        .prepare_container(first.clone(), deadline())
        .await
        .unwrap();
    let mut changed = first.clone();
    changed.image = [9; 32];
    assert!(
        lease
            .prepare_container(changed.clone(), deadline())
            .await
            .is_err()
    );
    assert!(lease.settle_container(changed, deadline()).await.is_err());
    assert!(
        lease
            .prepare_container(record(Uuid::new_v4()), deadline())
            .await
            .is_err()
    );
    lease
        .know_container(first.clone(), "a".repeat(64).parse().unwrap(), deadline())
        .await
        .unwrap();
    assert!(
        lease
            .know_container(first, "b".repeat(64).parse().unwrap(), deadline())
            .await
            .is_err()
    );
    for _ in 1..64 {
        lease
            .prepare_container(record(lease.run_id()), deadline())
            .await
            .unwrap();
    }
    assert_eq!(lease.container_records(deadline()).await.unwrap().len(), 64);
    assert!(
        lease
            .prepare_container(record(lease.run_id()), deadline())
            .await
            .is_err()
    );
}
#[test]
fn test_should_reject_truncated_corrupted_foreign_duplicate_and_invalid_encoding() {
    let owner = Uuid::new_v4();
    let entry = record(Uuid::new_v4());
    let bytes = encode(owner, std::slice::from_ref(&entry)).unwrap();
    for length in 0..bytes.len() {
        assert!(decode(&bytes[..length], owner).is_err());
    }
    for position in 0..bytes.len() {
        let mut bad = bytes.clone();
        bad[position] ^= 1;
        assert!(decode(&bad, owner).is_err());
    }
    assert!(decode(&bytes, Uuid::new_v4()).is_err());
    assert!(encode(owner, &[entry.clone(), entry.clone()]).is_err());
    let mut bad = entry.clone();
    bad.run = Uuid::nil();
    assert!(encode(owner, &[bad]).is_err());
    let mut relay = entry.clone();
    relay.function = false;
    relay.resource = Uuid::new_v4();
    assert!(encode(owner, &[entry, relay]).is_err());
    assert!(encode(owner, &vec![record(Uuid::new_v4()); 65]).is_err());
}
#[rstest::rstest]
#[case(PUBLISHED)]
#[case(STAGING)]
#[tokio::test]
async fn test_should_reject_unsafe_snapshot_inodes(#[case] file: &str) {
    let root = tempfile::tempdir().unwrap();
    let lease = lease(&root).await;
    let target = path(&root, file);
    let outside = root.path().join("outside");
    tokio::fs::write(&outside, b"keep").await.unwrap();
    symlink(&outside, &target).unwrap();
    assert!(lease.container_records(deadline()).await.is_err());
    tokio::fs::remove_file(&target).await.unwrap();
    tokio::fs::hard_link(&outside, &target).await.unwrap();
    assert!(lease.container_records(deadline()).await.is_err());
    tokio::fs::remove_file(&target).await.unwrap();
    tokio::fs::write(&target, b"").await.unwrap();
    tokio::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644))
        .await
        .unwrap();
    assert!(lease.container_records(deadline()).await.is_err());
    tokio::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600))
        .await
        .unwrap();
    tokio::fs::write(&target, vec![0; MAX_BYTES + 1])
        .await
        .unwrap();
    assert!(lease.container_records(deadline()).await.is_err());
    assert_eq!(tokio::fs::read(outside).await.unwrap(), b"keep");
}
#[tokio::test]
async fn test_should_ignore_only_bounded_safe_interrupted_staging() {
    let root = tempfile::tempdir().unwrap();
    let lease = lease(&root).await;
    let entry = record(lease.run_id());
    lease
        .prepare_container(entry.clone(), deadline())
        .await
        .unwrap();
    tokio::fs::write(path(&root, STAGING), b"partial")
        .await
        .unwrap();
    tokio::fs::set_permissions(path(&root, STAGING), std::fs::Permissions::from_mode(0o600))
        .await
        .unwrap();
    assert_eq!(
        lease.container_records(deadline()).await.unwrap(),
        vec![entry]
    );
    tokio::fs::write(path(&root, PUBLISHED), b"partial")
        .await
        .unwrap();
    assert!(lease.container_records(deadline()).await.is_err());
}
#[tokio::test]
async fn test_should_fail_before_publication_with_expired_budget() {
    let root = tempfile::tempdir().unwrap();
    let lease = lease(&root).await;
    assert!(matches!(
        lease
            .prepare_container(record(lease.run_id()), Instant::now())
            .await,
        Err(DirectoryLeaseError::Deadline)
    ));
    assert!(!tokio::fs::try_exists(path(&root, PUBLISHED)).await.unwrap());
    assert!(
        lease
            .container_records(Instant::now() + Duration::from_secs(61))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn test_should_keep_transaction_and_lease_owned_after_waiter_abandonment() {
    let root = tempfile::tempdir().unwrap();
    let lease = lease(&root).await;
    let (entered, observed) = tokio::sync::oneshot::channel();
    let (release, barrier) = std::sync::mpsc::channel();
    let entry = record(lease.run_id());
    let worker_lease = lease.clone();
    let task = tokio::spawn(async move {
        worker_lease
            .journal(deadline(), move |records| {
                records.push(entry);
                entered.send(()).unwrap();
                barrier.recv().unwrap();
                Ok(())
            })
            .await
    });
    observed.await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let check = lease.clone();
    let mut read = tokio::spawn(async move { check.container_records(deadline()).await });
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut read)
            .await
            .is_err()
    );
    drop(lease);
    assert!(matches!(
        DirectoryLease::acquire(root.path().into(), deadline()).await,
        Err(DirectoryLeaseError::InUse)
    ));
    release.send(()).unwrap();
    // The cancelled worker must finish/dispose its transaction before this read.
    assert_eq!(read.await.unwrap().unwrap(), []);
    let _next = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
}

#[tokio::test]
async fn test_should_keep_authoritative_record_when_erase_cannot_publish() {
    let root = tempfile::tempdir().unwrap();
    let lease = lease(&root).await;
    let entry = record(lease.run_id());
    lease
        .prepare_container(entry.clone(), deadline())
        .await
        .unwrap();
    let outside = root.path().join("outside");
    tokio::fs::write(&outside, b"keep").await.unwrap();
    symlink(&outside, path(&root, STAGING)).unwrap();
    assert!(
        lease
            .settle_container(entry.clone(), deadline())
            .await
            .is_err()
    );
    let bytes = tokio::fs::read(path(&root, PUBLISHED)).await.unwrap();
    assert_eq!(
        decode(&bytes, lease.owner_id()).unwrap(),
        vec![entry.clone()]
    );
    tokio::fs::remove_file(path(&root, STAGING)).await.unwrap();
    lease.settle_container(entry, deadline()).await.unwrap();
    assert_eq!(lease.container_records(deadline()).await.unwrap(), []);
}

#[tokio::test]
async fn test_should_reject_overlong_budget_before_waiting_for_transaction() {
    let root = tempfile::tempdir().unwrap();
    let lease = lease(&root).await;
    let _held = lease.0.transaction.acquire().await.unwrap();
    let result = tokio::time::timeout(
        Duration::from_millis(50),
        lease.container_records(Instant::now() + Duration::from_secs(61)),
    )
    .await;
    assert!(matches!(
        result,
        Ok(Err(DirectoryLeaseError::Configuration))
    ));
}
