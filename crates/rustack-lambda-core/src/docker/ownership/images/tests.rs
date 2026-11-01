use std::{
    os::unix::fs::{PermissionsExt, symlink},
    sync::Arc,
    time::Duration,
};

use sha2::{Digest, Sha256};
use tokio::time::Instant;
use uuid::Uuid;

use super::{ImageRecord, MAX_BYTES, PUBLISHED, STAGING, decode, encode};
use crate::docker::{
    DirectoryLease, DirectoryLeaseError, EnvironmentPlan, ImagePinScope,
    ownership::{ContainerRecord, NAMESPACE},
    plan::{
        ContainerRole,
        tests::{image_with, policy, revision},
    },
};

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
fn record(run: Uuid) -> ImageRecord {
    ImageRecord {
        run,
        pin: Uuid::new_v4(),
        image: [0xab; 32],
        committed: false,
    }
}
async fn lease(root: &tempfile::TempDir) -> DirectoryLease {
    DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap()
}
fn resign(bytes: &mut [u8]) {
    let length = bytes.len() - 32;
    let checksum = Sha256::digest(&bytes[..length]);
    bytes[length..].copy_from_slice(&checksum);
}
#[test]
fn test_should_roundtrip_full_capacity_and_reject_semantic_corruption() {
    let owner = Uuid::new_v4();
    let run = Uuid::new_v4();
    let records: Vec<_> = (1..=8192)
        .map(|id| ImageRecord {
            run,
            pin: Uuid::from_u128(id),
            image: [1; 32],
            committed: id % 2 == 0,
        })
        .collect();
    let bytes = encode(owner, &records).unwrap();
    assert_eq!(bytes.len(), MAX_BYTES);
    assert_eq!(decode(&bytes, owner).unwrap(), records);
    assert!(encode(owner, &[records.clone(), vec![record(run)]].concat()).is_err());
    assert!(decode(&bytes, Uuid::new_v4()).is_err());
    let single = encode(owner, &[record(run)]).unwrap();
    for length in 0..single.len() {
        assert!(decode(&single[..length], owner).is_err());
    }
    for position in 0..single.len() {
        let mut bad = single.clone();
        bad[position] ^= 1;
        assert!(decode(&bad, owner).is_err());
    }
    for position in [26, 42] {
        let mut bad = single.clone();
        bad[position..position + 16].fill(0);
        resign(&mut bad);
        assert!(decode(&bad, owner).is_err());
    }
    let mut nil_owner = encode(owner, &[]).unwrap();
    nil_owner.get_mut(8..24).unwrap().fill(0);
    resign(&mut nil_owner);
    assert!(decode(&nil_owner, Uuid::nil()).is_err());
    let mut bad = single;
    bad[90] = 2;
    resign(&mut bad);
    assert!(decode(&bad, owner).is_err());
    let entry = record(run);
    assert!(encode(owner, &[entry.clone(), entry]).is_err());
}
#[tokio::test]
async fn test_should_persist_unknown_commitment_and_erasure_under_new_run() {
    let root = tempfile::tempdir().unwrap();
    let first = lease(&root).await;
    let entry = record(first.run_id());
    first
        .prepare_image(entry.clone(), deadline())
        .await
        .unwrap();
    drop(first);
    let second = lease(&root).await;
    assert_ne!(entry.run, second.run_id());
    assert_eq!(
        second.image_records(deadline()).await.unwrap(),
        vec![entry.clone()]
    );
    let pin = entry.pin(&second).unwrap();
    assert_eq!(
        pin.reference().as_str(),
        format!(
            "rustack-internal/lambda-{}/{}:{}",
            second.owner_id().simple(),
            entry.run.simple(),
            entry.pin.simple()
        )
    );
    assert_eq!(
        entry.expected().unwrap().as_str(),
        format!("sha256:{}", "ab".repeat(32))
    );
    second
        .commit_image(entry.clone(), deadline())
        .await
        .unwrap();
    assert!(second.image_records(deadline()).await.unwrap()[0].committed);
    drop(pin);
    drop(second);
    let third = lease(&root).await;
    assert!(third.image_records(deadline()).await.unwrap()[0].committed);
    third.settle_image(entry.clone(), deadline()).await.unwrap();
    third.settle_image(entry, deadline()).await.unwrap();
    assert_eq!(third.image_records(deadline()).await.unwrap(), []);
}
#[tokio::test]
async fn test_should_reject_cross_run_and_changed_target_without_erasing_authority() {
    let root = tempfile::tempdir().unwrap();
    let lease = lease(&root).await;
    let entry = record(lease.run_id());
    lease
        .prepare_image(entry.clone(), deadline())
        .await
        .unwrap();
    let mut changed = entry.clone();
    changed.image = [7; 32];
    assert!(
        lease
            .prepare_image(changed.clone(), deadline())
            .await
            .is_err()
    );
    assert!(
        lease
            .commit_image(changed.clone(), deadline())
            .await
            .is_err()
    );
    assert!(lease.settle_image(changed, deadline()).await.is_err());
    assert!(
        lease
            .prepare_image(record(Uuid::new_v4()), deadline())
            .await
            .is_err()
    );
    assert_eq!(lease.image_records(deadline()).await.unwrap(), vec![entry]);
}
#[rstest::rstest]
#[case(PUBLISHED)]
#[case(STAGING)]
#[tokio::test]
async fn test_should_reject_unsafe_oversized_or_corrupt_pin_files(#[case] name: &str) {
    let root = tempfile::tempdir().unwrap();
    let lease = lease(&root).await;
    let target = root.path().join(NAMESPACE).join(name);
    let outside = root.path().join("outside");
    tokio::fs::write(&outside, b"keep").await.unwrap();
    symlink(&outside, &target).unwrap();
    assert!(lease.image_records(deadline()).await.is_err());
    tokio::fs::remove_file(&target).await.unwrap();
    tokio::fs::hard_link(&outside, &target).await.unwrap();
    assert!(lease.image_records(deadline()).await.is_err());
    tokio::fs::remove_file(&target).await.unwrap();
    tokio::fs::write(&target, b"partial").await.unwrap();
    tokio::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644))
        .await
        .unwrap();
    assert!(lease.image_records(deadline()).await.is_err());
    tokio::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600))
        .await
        .unwrap();
    if name == PUBLISHED {
        assert!(lease.image_records(deadline()).await.is_err());
    } else {
        assert_eq!(lease.image_records(deadline()).await.unwrap(), []);
    }
    tokio::fs::write(&target, vec![0; MAX_BYTES + 1])
        .await
        .unwrap();
    assert!(lease.image_records(deadline()).await.is_err());
    assert_eq!(tokio::fs::read(outside).await.unwrap(), b"keep");
}
#[tokio::test]
async fn test_should_share_transaction_exclusion_and_reject_overlong_budget_before_waiting() {
    let root = tempfile::tempdir().unwrap();
    let lease = lease(&root).await;
    let held = lease.0.transaction.acquire().await.unwrap();
    assert!(matches!(
        tokio::time::timeout(
            Duration::from_millis(50),
            lease.image_records(Instant::now() + Duration::from_secs(61))
        )
        .await,
        Ok(Err(DirectoryLeaseError::Configuration))
    ));
    let copy = lease.clone();
    let mut task =
        tokio::spawn(async move { copy.prepare_image(record(copy.run_id()), deadline()).await });
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut task)
            .await
            .is_err()
    );
    drop(held);
    task.await.unwrap().unwrap();
    assert_eq!(lease.image_records(deadline()).await.unwrap().len(), 1);
}
#[tokio::test]
async fn test_should_retain_record_when_erasure_cannot_publish() {
    let root = tempfile::tempdir().unwrap();
    let lease = lease(&root).await;
    let entry = record(lease.run_id());
    lease
        .prepare_image(entry.clone(), deadline())
        .await
        .unwrap();
    let staging = root.path().join(NAMESPACE).join(STAGING);
    symlink(root.path().join("outside"), &staging).unwrap();
    assert!(lease.settle_image(entry.clone(), deadline()).await.is_err());
    let published = tokio::fs::read(root.path().join(NAMESPACE).join(PUBLISHED))
        .await
        .unwrap();
    assert_eq!(
        decode(&published, lease.owner_id()).unwrap(),
        vec![entry.clone()]
    );
    tokio::fs::remove_file(staging).await.unwrap();
    lease.settle_image(entry, deadline()).await.unwrap();
}
#[tokio::test]
async fn test_should_keep_container_encoding_unchanged_and_allow_independent_records() {
    let root = tempfile::tempdir().unwrap();
    let lease = lease(&root).await;
    let plan = EnvironmentPlan::new(
        lease.clone(),
        image_with("1000", vec![]),
        Arc::new(revision()),
        policy(),
    )
    .unwrap();
    let container = ContainerRecord::from_plan(&plan, ContainerRole::Relay).unwrap();
    lease
        .prepare_container(container, deadline())
        .await
        .unwrap();
    let path = root.path().join(NAMESPACE).join("containers.v1");
    let before = tokio::fs::read(&path).await.unwrap();
    let pin = ImagePinScope::new(lease.clone()).allocate().unwrap();
    let entry =
        ImageRecord::new(&pin, &format!("sha256:{}", "a".repeat(64)).parse().unwrap()).unwrap();
    lease.prepare_image(entry, deadline()).await.unwrap();
    assert_eq!(tokio::fs::read(path).await.unwrap(), before);
    assert_eq!(lease.container_records(deadline()).await.unwrap().len(), 1);
    assert_eq!(lease.image_records(deadline()).await.unwrap().len(), 1);
}
