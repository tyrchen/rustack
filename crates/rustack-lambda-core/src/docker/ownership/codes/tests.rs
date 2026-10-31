use std::{
    fs::Permissions,
    os::unix::fs::{PermissionsExt, symlink},
    time::Duration,
};

use rstest::rstest;
use tokio::{
    fs::{read, set_permissions, write},
    time::Instant,
};
use uuid::Uuid;

use super::*;
use crate::docker::{
    binding::DaemonIdentity,
    ownership::{LEGACY_MAGIC, MAGIC, NAMESPACE, OWNER},
    tests::DAEMON_ID,
};

pub(in crate::docker) fn record(run: Uuid, volume: VolumeState, loader: LoaderState) -> CodeRecord {
    CodeRecord::builder()
        .run(run)
        .environment(Uuid::new_v4())
        .resource(Uuid::new_v4())
        .revision(Uuid::new_v4())
        .zip([0x11; 32])
        .base([0xaa; 32])
        .fingerprint([0xbb; 32])
        .volume(volume)
        .loader(loader)
        .build()
}
pub(in crate::docker) async fn publish(directory: &DirectoryLease, records: Vec<CodeRecord>) {
    assert!(directory.code_journaled());
    directory
        .snapshot::<CodeCodec, _>(deadline(), move |saved| {
            *saved = records;
            Ok(())
        })
        .await
        .unwrap();
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
fn signed(bytes: &mut [u8]) {
    let end = bytes.len() - 32;
    let sum = Sha256::digest(&bytes[..end]);
    bytes[end..].copy_from_slice(&sum);
}

#[rstest]
#[case(0)]
#[case(8)]
#[case(24)]
#[case(25)]
#[case(41)]
#[case(57)]
#[case(73)]
#[case(185)]
#[case(186)]
#[case(187)]
fn test_should_reject_corrupt_canonical_fields_even_with_recomputed_checksum(
    #[case] offset: usize,
) {
    // Make the former 0xff owner-byte no-op a deterministic regression case.
    let owner = Uuid::from_u128(u128::MAX);
    let row = record(Uuid::new_v4(), VolumeState::Known, LoaderState::Planned);
    let mut bytes = CodeCodec::encode(owner, &[row]).unwrap();
    if matches!(offset, 25 | 41 | 57 | 73) {
        bytes[offset..offset + 16].fill(0);
    } else if offset == 8 {
        bytes[offset] ^= 0xff;
    } else {
        bytes[offset] = 0xff;
    }
    signed(&mut bytes);
    assert!(matches!(
        CodeCodec::decode(&bytes, owner),
        Err(DirectoryLeaseError::InvalidState)
    ));
}
#[rstest]
#[case(0)]
#[case(1)]
#[case(10)]
#[case(24)]
#[case(25)]
#[case(218)]
#[case(250)]
fn test_should_reject_truncation_and_appended_data(#[case] length: usize) {
    let owner = Uuid::new_v4();
    let row = record(
        Uuid::new_v4(),
        VolumeState::Known,
        LoaderState::Known([0xcc; 32]),
    );
    let bytes = CodeCodec::encode(owner, &[row]).unwrap();
    assert_eq!(bytes.len(), 251);
    assert!(CodeCodec::decode(&bytes[..length], owner).is_err());
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(CodeCodec::decode(&extra, owner).is_err());
}
#[test]
fn test_should_bound_records_and_reject_duplicate_environments_and_wrong_owner() {
    let owner = Uuid::new_v4();
    let rows = (0..32)
        .map(|_| record(Uuid::new_v4(), VolumeState::Known, LoaderState::Uncertain))
        .collect::<Vec<_>>();
    let bytes = CodeCodec::encode(owner, &rows).unwrap();
    assert_eq!(bytes.len(), 6265);
    assert_eq!(CodeCodec::decode(&bytes, owner).unwrap(), rows);
    assert!(CodeCodec::decode(&bytes, Uuid::new_v4()).is_err());
    let mut too_many = rows.clone();
    too_many.push(rows[0].clone());
    assert!(CodeCodec::encode(owner, &too_many).is_err());
    let mut duplicate = rows[0].clone();
    duplicate.run = Uuid::new_v4();
    assert!(CodeCodec::encode(owner, &[rows[0].clone(), duplicate]).is_err());
    assert!(CodeCodec::encode(Uuid::nil(), &[]).is_err());
    let mut damaged = bytes;
    damaged[100] ^= 1;
    assert!(CodeCodec::decode(&damaged, owner).is_err());
}
#[rstest]
#[case(VolumeState::Planned, LoaderState::Uncertain)]
#[case(VolumeState::Planned, LoaderState::Retired)]
#[case(VolumeState::Uncertain, LoaderState::Known([1;32]))]
#[case(VolumeState::Retired, LoaderState::Planned)]
#[case(VolumeState::Retired, LoaderState::Known([1;32]))]
fn test_should_reject_impossible_effect_states(
    #[case] volume: VolumeState,
    #[case] loader: LoaderState,
) {
    assert!(CodeCodec::encode(Uuid::new_v4(), &[record(Uuid::new_v4(), volume, loader)]).is_err());
}
#[tokio::test]
async fn test_should_create_explicit_code_provenance_and_reopen_without_rewriting_owner() {
    let root = tempfile::tempdir().unwrap();
    let lease = DirectoryLease::acquire_code(root.path().into(), deadline())
        .await
        .unwrap();
    lease.require_journaled().unwrap();
    assert!(lease.code_journaled());
    let owner = lease.owner_id();
    let run = lease.run_id();
    let path = root.path().join(NAMESPACE);
    let before = read(path.join(OWNER)).await.unwrap();
    assert_eq!(&before[..8], b"RSDKOWN3");
    let row = record(run, VolumeState::Known, LoaderState::Known([0xcc; 32]));
    publish(&lease, vec![row.clone()]).await;
    drop(lease);
    let lease = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    assert_eq!(owner, lease.owner_id());
    assert_ne!(run, lease.run_id());
    assert_eq!(lease.code_records(deadline()).await.unwrap(), [row]);
    assert_eq!(read(path.join(OWNER)).await.unwrap(), before);
}
#[rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn test_should_refuse_old_roots_without_automatic_upgrade(#[case] legacy: bool) {
    let root = tempfile::tempdir().unwrap();
    let lease = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    drop(lease);
    let path = root.path().join(NAMESPACE).join(OWNER);
    let mut bytes = read(&path).await.unwrap();
    if legacy {
        bytes[..8].copy_from_slice(LEGACY_MAGIC);
        signed(&mut bytes);
        write(&path, &bytes).await.unwrap();
    }
    assert!(matches!(
        DirectoryLease::acquire_code(root.path().into(), deadline()).await,
        Err(DirectoryLeaseError::InvalidState)
    ));
    assert_eq!(read(path).await.unwrap(), bytes);
    assert_eq!(&bytes[..8], if legacy { LEGACY_MAGIC } else { MAGIC });
}
#[rstest]
#[case("code.v1")]
#[case("code.pending")]
#[tokio::test]
async fn test_should_refuse_any_code_file_in_image_namespace_on_acquire_and_binding(
    #[case] name: &str,
) {
    let root = tempfile::tempdir().unwrap();
    let lease = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    let identity = DaemonIdentity::try_from(DAEMON_ID).unwrap();
    lease.bind_daemon(identity, deadline()).await.unwrap();
    let path = root.path().join(NAMESPACE).join(name);
    write(&path, []).await.unwrap();
    set_permissions(&path, Permissions::from_mode(0o600))
        .await
        .unwrap();
    assert!(lease.bind_daemon(identity, deadline()).await.is_err());
    assert!(lease.code_records(deadline()).await.is_err());
    drop(lease);
    assert!(
        DirectoryLease::acquire(root.path().into(), deadline())
            .await
            .is_err()
    );
}
#[rstest]
#[case("code.v1")]
#[case("code.pending")]
#[tokio::test]
async fn test_should_refuse_unsafe_code_files_and_preserve_outside_content(#[case] name: &str) {
    let root = tempfile::tempdir().unwrap();
    let lease = DirectoryLease::acquire_code(root.path().into(), deadline())
        .await
        .unwrap();
    let outside = root.path().join("outside");
    write(&outside, b"preserve").await.unwrap();
    let path = root.path().join(NAMESPACE).join(name);
    symlink(&outside, &path).unwrap();
    assert!(lease.code_records(deadline()).await.is_err());
    tokio::fs::remove_file(&path).await.unwrap();
    write(&path, vec![0; MAX_BYTES + 1]).await.unwrap();
    set_permissions(&path, Permissions::from_mode(0o600))
        .await
        .unwrap();
    assert!(lease.code_records(deadline()).await.is_err());
    assert_eq!(read(outside).await.unwrap(), b"preserve");
}
#[tokio::test]
async fn test_should_tolerate_private_torn_staging_but_refuse_missing_daemon_binding_over_code() {
    let root = tempfile::tempdir().unwrap();
    let lease = DirectoryLease::acquire_code(root.path().into(), deadline())
        .await
        .unwrap();
    let row = record(lease.run_id(), VolumeState::Known, LoaderState::Planned);
    publish(&lease, vec![row.clone()]).await;
    let path = root.path().join(NAMESPACE).join("code.pending");
    write(&path, b"torn").await.unwrap();
    set_permissions(&path, Permissions::from_mode(0o600))
        .await
        .unwrap();
    assert_eq!(lease.code_records(deadline()).await.unwrap(), [row]);
    assert!(
        lease
            .bind_daemon(DaemonIdentity::try_from(DAEMON_ID).unwrap(), deadline())
            .await
            .is_err()
    );
    assert!(!root.path().join(NAMESPACE).join("daemon.v1").exists());
}
#[tokio::test]
async fn test_should_compare_and_swap_monotonic_recovery_state_and_reject_early_erasure() {
    let root = tempfile::tempdir().unwrap();
    let lease = DirectoryLease::acquire_code(root.path().into(), deadline())
        .await
        .unwrap();
    let row = record(lease.run_id(), VolumeState::Known, LoaderState::Uncertain);
    publish(&lease, vec![row.clone()]).await;
    assert!(lease.settle_code(row.clone(), deadline()).await.is_err());
    let mut next = row.clone();
    next.loader = LoaderState::Known([0xcc; 32]);
    lease
        .advance_code(row.clone(), next.clone(), deadline())
        .await
        .unwrap();
    lease
        .advance_code(row.clone(), next.clone(), deadline())
        .await
        .unwrap();
    let mut conflict = next.clone();
    conflict.loader = LoaderState::Known([0xdd; 32]);
    assert!(lease.advance_code(row, conflict, deadline()).await.is_err());
    let mut retired = next.clone();
    retired.loader = LoaderState::Retired;
    lease
        .advance_code(next, retired.clone(), deadline())
        .await
        .unwrap();
    let mut done = retired.clone();
    done.volume = VolumeState::Retired;
    lease
        .advance_code(retired, done.clone(), deadline())
        .await
        .unwrap();
    lease.settle_code(done.clone(), deadline()).await.unwrap();
    lease.settle_code(done, deadline()).await.unwrap();
    assert_eq!(lease.code_records(deadline()).await.unwrap(), []);
}

#[rstest]
#[case("code.v1", false)]
#[case("code.pending", false)]
#[case("code.v1", true)]
#[case("code.pending", true)]
#[tokio::test]
async fn test_should_reject_public_mode_and_hardlinked_code_files(
    #[case] name: &str,
    #[case] hardlink: bool,
) {
    let root = tempfile::tempdir().unwrap();
    let lease = DirectoryLease::acquire_code(root.path().into(), deadline())
        .await
        .unwrap();
    let path = root.path().join(NAMESPACE).join(name);
    if hardlink {
        let outside = root.path().join("outside");
        write(&outside, []).await.unwrap();
        set_permissions(&outside, Permissions::from_mode(0o600))
            .await
            .unwrap();
        tokio::fs::hard_link(&outside, &path).await.unwrap();
    } else {
        write(&path, []).await.unwrap();
        set_permissions(&path, Permissions::from_mode(0o644))
            .await
            .unwrap();
    }
    assert!(matches!(
        lease.code_records(deadline()).await,
        Err(DirectoryLeaseError::InvalidState)
    ));
    drop(lease);
    assert!(matches!(
        DirectoryLease::acquire(root.path().into(), deadline()).await,
        Err(DirectoryLeaseError::InvalidState)
    ));
}
