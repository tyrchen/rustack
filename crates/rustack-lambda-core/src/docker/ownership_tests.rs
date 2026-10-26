use std::{
    error::Error,
    os::unix::fs::{PermissionsExt, symlink},
    process::Stdio,
    sync::mpsc,
};

use nix::unistd::mkfifo;
use tempfile::TempDir;
use tokio::{
    process::Command,
    sync::oneshot,
    time::{sleep, timeout},
};

use super::*;
use crate::docker::ImagePinScope;

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
async fn lease(root: &TempDir) -> DirectoryLease {
    DirectoryLease::acquire(root.path().to_path_buf(), deadline())
        .await
        .unwrap()
}
fn managed(root: &TempDir, name: &str) -> PathBuf {
    root.path().join(NAMESPACE).join(name)
}

#[tokio::test]
async fn test_should_persist_owner_and_refresh_run_after_last_clone_closes() {
    let root = tempfile::tempdir().unwrap();
    let first = lease(&root).await;
    let (owner, run) = (first.owner_id(), first.run_id());
    assert!(!owner.is_nil() && !run.is_nil());
    let clone = first.clone();
    drop(first);
    assert!(matches!(
        DirectoryLease::acquire(root.path().into(), deadline()).await,
        Err(DirectoryLeaseError::InUse)
    ));
    drop(clone);
    let second = lease(&root).await;
    assert_eq!(second.owner_id(), owner);
    assert_ne!(second.run_id(), run);
    let bytes = tokio::fs::read(managed(&root, OWNER)).await.unwrap();
    assert_eq!(bytes.len(), OWNER_BYTES);
    assert_eq!(bytes.get(..8), Some(MAGIC.as_slice()));
    assert!(
        tokio::fs::try_exists(managed(&root, PENDING))
            .await
            .is_ok_and(|exists| !exists)
    );
}

#[tokio::test]
async fn test_should_hold_the_directory_through_allocated_pins() {
    let root = tempfile::tempdir().unwrap();
    let owner = lease(&root).await;
    let id = owner.owner_id();
    let scope = ImagePinScope::new(owner);
    let pin = scope.allocate().unwrap();
    let copy = pin.clone();
    drop(scope);
    drop(pin);
    assert!(matches!(
        DirectoryLease::acquire(root.path().into(), deadline()).await,
        Err(DirectoryLeaseError::InUse)
    ));
    drop(copy);
    assert_eq!(lease(&root).await.owner_id(), id);
}

#[rstest::rstest]
#[case(PathBuf::new())]
#[case(PathBuf::from("relative"))]
#[case(PathBuf::from("/tmp/../outside"))]
#[case(PathBuf::from("/tmp/\0bad"))]
#[case(PathBuf::from(format!("/{}", "a".repeat(4096))))]
#[tokio::test]
async fn test_should_reject_invalid_directories_without_io(#[case] path: PathBuf) {
    assert!(matches!(
        DirectoryLease::acquire(path, deadline()).await,
        Err(DirectoryLeaseError::Configuration)
    ));
}

#[tokio::test]
async fn test_should_create_missing_roots_and_keep_descriptor_files_private() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("new/data");
    let owner = DirectoryLease::acquire(root.clone(), deadline())
        .await
        .unwrap();
    for (path, expected) in [
        (root.join(NAMESPACE), 0o700),
        (root.join(NAMESPACE).join(LOCK), 0o600),
        (root.join(NAMESPACE).join(OWNER), 0o600),
    ] {
        let metadata = tokio::fs::metadata(path).await.unwrap();
        assert_eq!(metadata.permissions().mode() & 0o777, expected);
        assert_eq!(metadata.uid(), geteuid().as_raw());
    }
    assert_eq!(format!("{owner:?}"), "DirectoryLease(<held>)");
}

#[rstest::rstest]
#[case("")]
#[case("/")]
#[case("/.")]
#[case("//./")]
#[tokio::test]
async fn test_should_reject_a_symlink_root_without_touching_the_destination(#[case] suffix: &str) {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let link = root.path().join("link");
    symlink(outside.path(), &link).unwrap();
    let spelling = PathBuf::from(format!("{}{suffix}", link.display()));
    assert!(matches!(
        DirectoryLease::acquire(spelling, deadline()).await,
        Err(DirectoryLeaseError::InvalidState)
    ));
    assert!(
        !tokio::fs::try_exists(outside.path().join(NAMESPACE))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn test_should_reject_untrusted_rename_permissions_and_accept_a_sticky_ancestor() {
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("shared");
    tokio::fs::create_dir(&parent).await.unwrap();
    tokio::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o777))
        .await
        .unwrap();
    let path = parent.join("new/data");
    assert!(matches!(
        DirectoryLease::acquire(path.clone(), deadline()).await,
        Err(DirectoryLeaseError::InvalidState)
    ));
    assert!(!tokio::fs::try_exists(parent.join("new")).await.unwrap());
    tokio::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o1777))
        .await
        .unwrap();
    let owner = DirectoryLease::acquire(path.clone(), deadline())
        .await
        .unwrap();
    let id = owner.owner_id();
    drop(owner);
    assert_eq!(
        DirectoryLease::acquire(path, deadline())
            .await
            .unwrap()
            .owner_id(),
        id
    );
}

#[tokio::test]
async fn test_should_repeat_the_ancestor_sync_barrier_after_interrupted_root_creation() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("new/parent/data");
    let test = tokio::task::spawn_blocking(move || {
        let control = Control {
            deadline: deadline(),
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        let mut attempts = 0;
        let first = open_root_with_sync(&path, &control, |directory| {
            attempts += 1;
            if attempts == 2 {
                return Err(io::Error::other("interrupted parent sync"));
            }
            directory.sync_all()
        });
        assert!(matches!(first, Err(DirectoryLeaseError::Io(_))));
        assert!(path.is_dir());
        let parent = symlink_metadata(path.parent().unwrap()).unwrap();
        let mut observed = Vec::new();
        let reopened = open_root_with_sync(&path, &control, |directory| {
            let metadata = directory.metadata()?;
            observed.push((metadata.dev(), metadata.ino()));
            directory.sync_all()
        })
        .unwrap();
        assert!(
            observed.contains(&(parent.dev(), parent.ino())),
            "retry must not skip the already-existing parent"
        );
        drop(reopened);
        let owner = acquire(&DataDirectory::try_from(path).unwrap(), &control).unwrap();
        assert!(!owner.owner_id().is_nil());
    });
    timeout(Duration::from_secs(10), test)
        .await
        .unwrap()
        .unwrap();
}

#[rstest::rstest]
#[case::namespace(NAMESPACE)]
#[case::lock(LOCK)]
#[case::identity(OWNER)]
#[tokio::test]
async fn test_should_reject_managed_symlinks_without_modifying_their_targets(#[case] name: &str) {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let sentinel = outside.path().join("sentinel");
    tokio::fs::write(&sentinel, b"sentinel").await.unwrap();
    if name == NAMESPACE {
        symlink(outside.path(), root.path().join(NAMESPACE)).unwrap();
    } else {
        drop(lease(&root).await);
        tokio::fs::remove_file(managed(&root, name)).await.unwrap();
        symlink(&sentinel, managed(&root, name)).unwrap();
    }
    assert!(matches!(
        DirectoryLease::acquire(root.path().into(), deadline()).await,
        Err(DirectoryLeaseError::InvalidState)
    ));
    assert_eq!(tokio::fs::read(&sentinel).await.unwrap(), b"sentinel");
}

#[rstest::rstest]
#[case::lock(LOCK)]
#[case::identity(OWNER)]
#[tokio::test]
async fn test_should_reject_hardlinked_managed_files(#[case] name: &str) {
    let root = tempfile::tempdir().unwrap();
    drop(lease(&root).await);
    let before = tokio::fs::read(managed(&root, name)).await.unwrap();
    let link = root.path().join("hardlink");
    tokio::fs::hard_link(managed(&root, name), &link)
        .await
        .unwrap();
    assert!(matches!(
        DirectoryLease::acquire(root.path().into(), deadline()).await,
        Err(DirectoryLeaseError::InvalidState)
    ));
    assert_eq!(tokio::fs::read(link).await.unwrap(), before);
}

#[rstest::rstest]
#[case::namespace(NAMESPACE)]
#[case::lock(LOCK)]
#[case::identity(OWNER)]
#[tokio::test]
async fn test_should_reject_group_or_other_access(#[case] name: &str) {
    let root = tempfile::tempdir().unwrap();
    drop(lease(&root).await);
    let path = if name == NAMESPACE {
        root.path().join(name)
    } else {
        managed(&root, name)
    };
    tokio::fs::set_permissions(
        &path,
        std::fs::Permissions::from_mode(if name == NAMESPACE { 0o755 } else { 0o644 }),
    )
    .await
    .unwrap();
    assert!(matches!(
        DirectoryLease::acquire(root.path().into(), deadline()).await,
        Err(DirectoryLeaseError::InvalidState)
    ));
}

#[rstest::rstest]
#[case::lock(LOCK)]
#[case::identity(OWNER)]
#[tokio::test]
async fn test_should_reject_fifos_without_waiting_for_a_peer(#[case] name: &str) {
    let root = tempfile::tempdir().unwrap();
    drop(lease(&root).await);
    tokio::fs::remove_file(managed(&root, name)).await.unwrap();
    mkfifo(&managed(&root, name), Mode::from_bits_truncate(0o600)).unwrap();
    let result = timeout(
        Duration::from_secs(2),
        DirectoryLease::acquire(root.path().into(), deadline()),
    )
    .await
    .unwrap();
    assert!(matches!(result, Err(DirectoryLeaseError::InvalidState)));
}

#[rstest::rstest]
#[case::empty(Vec::new())]
#[case::partial(vec![0; 24])]
#[case::oversized(vec![0; OWNER_BYTES + 1])]
#[case::magic(vec![0; OWNER_BYTES])]
#[tokio::test]
async fn test_should_reject_damaged_published_identity_without_regenerating_it(
    #[case] damaged: Vec<u8>,
) {
    let root = tempfile::tempdir().unwrap();
    drop(lease(&root).await);
    tokio::fs::write(managed(&root, OWNER), &damaged)
        .await
        .unwrap();
    assert!(matches!(
        DirectoryLease::acquire(root.path().into(), deadline()).await,
        Err(DirectoryLeaseError::InvalidState)
    ));
    assert_eq!(
        tokio::fs::read(managed(&root, OWNER)).await.unwrap(),
        damaged
    );
}

#[tokio::test]
async fn test_should_reject_a_nonnil_identity_bit_flip_and_a_checksummed_nil_identity() {
    let root = tempfile::tempdir().unwrap();
    drop(lease(&root).await);
    let mut bytes = tokio::fs::read(managed(&root, OWNER)).await.unwrap();
    *bytes.get_mut(8).unwrap() ^= 1;
    tokio::fs::write(managed(&root, OWNER), &bytes)
        .await
        .unwrap();
    assert!(matches!(
        DirectoryLease::acquire(root.path().into(), deadline()).await,
        Err(DirectoryLeaseError::InvalidState)
    ));
    bytes.get_mut(8..24).unwrap().fill(0);
    let checksum = Sha256::digest(bytes.get(..24).unwrap());
    bytes
        .get_mut(24..)
        .unwrap()
        .copy_from_slice(checksum.as_ref());
    tokio::fs::write(managed(&root, OWNER), &bytes)
        .await
        .unwrap();
    assert!(matches!(
        DirectoryLease::acquire(root.path().into(), deadline()).await,
        Err(DirectoryLeaseError::InvalidState)
    ));
}

#[tokio::test]
async fn test_should_recover_only_a_bounded_pending_initialization() {
    let root = tempfile::tempdir().unwrap();
    drop(lease(&root).await);
    tokio::fs::remove_file(managed(&root, OWNER)).await.unwrap();
    tokio::fs::write(managed(&root, PENDING), b"partial")
        .await
        .unwrap();
    tokio::fs::set_permissions(
        managed(&root, PENDING),
        std::fs::Permissions::from_mode(0o600),
    )
    .await
    .unwrap();
    let recovered = lease(&root).await;
    assert!(!recovered.owner_id().is_nil());
    assert!(
        !tokio::fs::try_exists(managed(&root, PENDING))
            .await
            .unwrap()
    );
    drop(recovered);
    tokio::fs::remove_file(managed(&root, OWNER)).await.unwrap();
    tokio::fs::write(managed(&root, PENDING), vec![0; OWNER_BYTES + 1])
        .await
        .unwrap();
    assert!(matches!(
        DirectoryLease::acquire(root.path().into(), deadline()).await,
        Err(DirectoryLeaseError::InvalidState)
    ));
}

#[tokio::test]
async fn test_should_reject_unsafe_pending_initialization_without_touching_its_target() {
    let root = tempfile::tempdir().unwrap();
    drop(lease(&root).await);
    tokio::fs::remove_file(managed(&root, OWNER)).await.unwrap();
    let sentinel = root.path().join("sentinel");
    tokio::fs::write(&sentinel, b"outside").await.unwrap();
    symlink(&sentinel, managed(&root, PENDING)).unwrap();
    assert!(matches!(
        DirectoryLease::acquire(root.path().into(), deadline()).await,
        Err(DirectoryLeaseError::InvalidState)
    ));
    assert_eq!(tokio::fs::read(sentinel).await.unwrap(), b"outside");
}

#[tokio::test]
async fn test_should_bound_deadlines_and_keep_diagnostics_redacted() {
    let root = tempfile::tempdir().unwrap();
    assert!(matches!(
        DirectoryLease::acquire(root.path().into(), Instant::now()).await,
        Err(DirectoryLeaseError::Deadline)
    ));
    assert!(matches!(
        DirectoryLease::acquire(root.path().into(), Instant::now() + Duration::from_secs(61)).await,
        Err(DirectoryLeaseError::Configuration)
    ));
    let error = DirectoryLeaseError::Io(io::Error::other("secret operator path"));
    assert!(!format!("{error:?} {error}").contains("secret"));
    assert!(error.source().is_some());
}

#[rstest::rstest]
#[case::aborted(false)]
#[case::deadline(true)]
#[tokio::test]
async fn test_should_retain_worker_capacity_and_lock_after_the_waiter_ends(#[case] expires: bool) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().to_path_buf();
    let pool = Box::leak(Box::new(Semaphore::new(1)));
    let (ready, acquired) = oneshot::channel();
    let (release, held) = mpsc::channel();
    let budget = if expires {
        Duration::from_secs(2)
    } else {
        Duration::from_secs(10)
    };
    let task = tokio::spawn(run(pool, Instant::now() + budget, move |control| {
        let lease = acquire(&DataDirectory::try_from(path)?, control)?;
        ready.send(()).unwrap();
        held.recv_timeout(Duration::from_secs(10)).unwrap();
        Ok(lease)
    }));
    acquired.await.unwrap();
    if expires {
        assert!(matches!(
            timeout(Duration::from_secs(5), task)
                .await
                .unwrap()
                .unwrap(),
            Err(DirectoryLeaseError::Deadline)
        ));
    } else {
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
    }
    assert_eq!(pool.available_permits(), 0);
    assert!(matches!(
        DirectoryLease::acquire(root.path().into(), deadline()).await,
        Err(DirectoryLeaseError::InUse)
    ));
    release.send(()).unwrap();
    timeout(Duration::from_secs(5), async {
        loop {
            match DirectoryLease::acquire(root.path().into(), deadline()).await {
                Ok(lease) => {
                    drop(lease);
                    break;
                }
                Err(DirectoryLeaseError::InUse) => sleep(Duration::from_millis(5)).await,
                other => panic!("unexpected reacquisition result: {other:?}"),
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(pool.available_permits(), 1);
}

#[tokio::test]
async fn test_should_release_worker_capacity_after_a_panicking_worker() {
    let pool = Box::leak(Box::new(Semaphore::new(1)));
    let result: Result<(), _> = run(pool, deadline(), |_| panic!("worker failure")).await;
    assert!(matches!(result, Err(DirectoryLeaseError::Worker)));
    assert_eq!(pool.available_permits(), 1);
}

#[tokio::test]
async fn test_should_retain_capacity_with_a_completed_but_unconsumed_lease_result() {
    let root = tempfile::tempdir().unwrap();
    let held = lease(&root).await;
    let pool: &'static Semaphore = Box::leak(Box::new(Semaphore::new(1)));
    let task = spawn_work(
        pool.try_acquire().unwrap(),
        Control {
            deadline: deadline(),
            cancelled: Arc::new(AtomicBool::new(false)),
        },
        |_| Ok(held),
    );
    timeout(Duration::from_secs(5), async {
        while !task.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(pool.available_permits(), 0);
    assert!(matches!(
        DirectoryLease::acquire(root.path().into(), deadline()).await,
        Err(DirectoryLeaseError::InUse)
    ));
    drop(task);
    assert_eq!(pool.available_permits(), 1);
    drop(lease(&root).await);
}

#[tokio::test]
async fn test_should_expire_worker_admission_without_starting_io() {
    let pool: &'static Semaphore = Box::leak(Box::new(Semaphore::new(1)));
    let _held = pool.acquire().await.unwrap();
    let started = Arc::new(AtomicBool::new(false));
    let observed = started.clone();
    let result = run(
        pool,
        Instant::now() + Duration::from_millis(20),
        move |_| {
            started.store(true, Ordering::Release);
            Ok(())
        },
    )
    .await;
    assert!(matches!(result, Err(DirectoryLeaseError::Deadline)));
    assert!(!observed.load(Ordering::Acquire));
}

#[tokio::test]
async fn test_should_enforce_exclusion_and_stable_identity_across_processes() {
    const PATH: &str = "RUSTACK_DIRECTORY_LEASE_CHILD_PATH";
    const MODE: &str = "RUSTACK_DIRECTORY_LEASE_CHILD_MODE";
    if let Some(path) = std::env::var_os(PATH) {
        let result = DirectoryLease::acquire(path.into(), deadline()).await;
        if std::env::var(MODE).unwrap() == "busy" {
            assert!(matches!(result, Err(DirectoryLeaseError::InUse)));
        } else {
            let lease = result.unwrap();
            assert_eq!(
                lease.owner_id().to_string(),
                std::env::var("RUSTACK_DIRECTORY_LEASE_CHILD_OWNER").unwrap()
            );
            assert_ne!(
                lease.run_id().to_string(),
                std::env::var("RUSTACK_DIRECTORY_LEASE_CHILD_RUN").unwrap()
            );
        }
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let first = lease(&root).await;
    let owner = first.owner_id();
    let run = first.run_id();
    assert_child(&root, "busy", owner, run).await;
    drop(first);
    assert_child(&root, "available", owner, run).await;
}

async fn assert_child(root: &TempDir, mode: &str, owner: Uuid, run: Uuid) {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "docker::ownership::tests::test_should_enforce_exclusion_and_stable_identity_across_processes", "--test-threads=1"])
        .env("RUSTACK_DIRECTORY_LEASE_CHILD_PATH", root.path())
        .env("RUSTACK_DIRECTORY_LEASE_CHILD_MODE", mode)
        .env("RUSTACK_DIRECTORY_LEASE_CHILD_OWNER", owner.to_string())
        .env("RUSTACK_DIRECTORY_LEASE_CHILD_RUN", run.to_string())
        .stdout(Stdio::null()).kill_on_drop(true).spawn().unwrap();
    let status = if let Ok(status) = timeout(Duration::from_secs(10), child.wait()).await {
        status.unwrap()
    } else {
        child.start_kill().unwrap();
        timeout(Duration::from_secs(5), child.wait())
            .await
            .unwrap()
            .unwrap();
        panic!("ownership child timed out");
    };
    assert!(status.success());
}
