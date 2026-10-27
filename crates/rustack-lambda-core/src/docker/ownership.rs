//! Persistent, descriptor-relative ownership of a Docker resource namespace.
//!
//! Standard files are required for OS advisory locks. All synchronous IO runs
//! on bounded blocking workers; lease clones perform no IO and retain the lock.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]

use std::{
    ffi::OsString,
    fmt,
    fs::{File, Metadata, TryLockError, canonicalize, symlink_metadata},
    io::{self, Read, Write},
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use nix::{
    errno::Errno,
    fcntl::{OFlag, open, openat, renameat},
    sys::stat::{Mode, mkdirat},
    unistd::geteuid,
};
use sha2::{Digest, Sha256};
use tokio::{
    runtime::Handle,
    sync::{Semaphore, SemaphorePermit},
    task::{JoinHandle, spawn_blocking},
    time::{Instant, timeout_at},
};
use uuid::Uuid;

pub(super) use self::{images::ImageRecord, journal::ContainerRecord};

mod daemon;
mod images;
mod journal;
mod snapshot;

const NAMESPACE: &str = ".rustack-lambda-docker";
const LOCK: &str = "lease.lock";
const OWNER: &str = "owner.v1";
const PENDING: &str = "owner.pending";
const MAGIC: &[u8; 8] = b"RSDKOWN1";
const OWNER_BYTES: usize = 56;
static WORKERS: Semaphore = Semaphore::const_new(4);

/// Directory acquisition failures; diagnostics never include operator paths.
#[derive(thiserror::Error)]
#[non_exhaustive]
pub enum DirectoryLeaseError {
    /// Invalid operator directory or acquisition budget.
    #[error("invalid Docker ownership configuration")]
    Configuration,
    /// Another descriptor/process owns this data directory.
    #[error("Docker data directory is already owned")]
    InUse,
    /// Unsafe file kind/permissions/links or damaged authoritative identity.
    #[error("invalid Docker ownership state")]
    InvalidState,
    /// The absolute acquisition deadline elapsed.
    #[error("Docker ownership deadline exceeded")]
    Deadline,
    /// The requesting acquisition was abandoned.
    #[error("Docker ownership acquisition cancelled")]
    Cancelled,
    /// No runtime or the blocking worker failed.
    #[error("Docker ownership worker failed")]
    Worker,
    /// A filesystem operation failed, preserving its source without displaying it.
    #[error("Docker ownership IO failed")]
    Io(#[source] io::Error),
}
impl fmt::Debug for DirectoryLeaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}
impl From<io::Error> for DirectoryLeaseError {
    fn from(source: io::Error) -> Self {
        Self::Io(source)
    }
}
impl From<Errno> for DirectoryLeaseError {
    fn from(source: Errno) -> Self {
        match source {
            Errno::ELOOP | Errno::ENOTDIR => Self::InvalidState,
            _ => Self::Io(source.into()),
        }
    }
}

/// Shared proof of an exclusive, durably identified Docker data directory.
#[derive(Clone)]
#[non_exhaustive]
pub struct DirectoryLease(Arc<Lease>);
struct Lease {
    owner: Uuid,
    run: Uuid,
    transaction: Arc<Semaphore>,
    guard: LeaseGuard,
}
enum LeaseGuard {
    Held {
        directory: File,
        _lock: File,
    },
    #[cfg(test)]
    Testing {
        directory: File,
        _root: tempfile::TempDir,
    },
}
impl fmt::Debug for DirectoryLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DirectoryLease(<held>)")
    }
}
impl DirectoryLease {
    pub(super) fn same_instance(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    #[cfg(test)]
    pub(super) fn synthetic(&self) -> bool {
        matches!(self.0.guard, LeaseGuard::Testing { .. })
    }
    /// Acquire a private namespace, retaining exclusion until the last clone drops.
    ///
    /// # Errors
    /// Rejects invalid paths/budgets, competing owners, unsafe/corrupt state,
    /// cancellation/deadline, filesystem failures and unavailable/failed workers.
    /// Acquisition does not establish previous-run reconciliation or readiness.
    ///
    /// ```no_run
    /// # async fn acquire() -> Result<(), rustack_lambda_core::docker::DirectoryLeaseError> {
    /// use rustack_lambda_core::docker::DirectoryLease;
    /// let lease = DirectoryLease::acquire(
    ///     "/tmp/rustack-data".into(),
    ///     tokio::time::Instant::now() + std::time::Duration::from_secs(5),
    /// ).await?;
    /// let scope = rustack_lambda_core::docker::ImagePinScope::new(lease);
    /// # Ok(()) }
    /// ```
    pub async fn acquire(
        directory: PathBuf,
        deadline: Instant,
    ) -> Result<Self, DirectoryLeaseError> {
        let directory = DataDirectory::try_from(directory)?;
        run(&WORKERS, deadline, move |control| {
            acquire(&directory, control)
        })
        .await
    }

    /// Persistent UUID used for resource labels and private image aliases.
    ///
    /// ```no_run
    /// # fn owner(lease: &rustack_lambda_core::docker::DirectoryLease) {
    /// let owner = lease.owner_id();
    /// assert!(!owner.is_nil());
    /// # }
    /// ```
    #[must_use]
    pub fn owner_id(&self) -> Uuid {
        self.0.owner
    }

    /// Fresh UUID distinguishing this acquisition from prior runs.
    ///
    /// ```no_run
    /// # fn run(lease: &rustack_lambda_core::docker::DirectoryLease) {
    /// let run = lease.run_id();
    /// assert!(!run.is_nil());
    /// # }
    /// ```
    #[must_use]
    pub fn run_id(&self) -> Uuid {
        self.0.run
    }

    #[cfg(test)]
    pub(super) fn testing(owner: Uuid, run: Uuid) -> Self {
        Self(Arc::new(Lease {
            owner,
            run,
            transaction: Arc::new(Semaphore::new(1)),
            guard: {
                let root = tempfile::tempdir().unwrap();
                let directory = File::open(root.path()).unwrap();
                LeaseGuard::Testing {
                    directory,
                    _root: root,
                }
            },
        }))
    }
}

struct DataDirectory(PathBuf);
impl TryFrom<PathBuf> for DataDirectory {
    type Error = DirectoryLeaseError;

    fn try_from(value: PathBuf) -> Result<Self, Self::Error> {
        let encoded = value.as_os_str().as_encoded_bytes();
        if encoded.len() > 4096
            || encoded.contains(&0)
            || value.components().count() > 128
            || !value.is_absolute()
            || value
                .components()
                .any(|part| matches!(part, Component::ParentDir))
        {
            return Err(DirectoryLeaseError::Configuration);
        }
        Ok(Self(value.components().collect()))
    }
}

struct Control {
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
}
impl Control {
    fn check(&self) -> Result<(), DirectoryLeaseError> {
        if self.cancelled.load(Ordering::Acquire) {
            Err(DirectoryLeaseError::Cancelled)
        } else if Instant::now() >= self.deadline {
            Err(DirectoryLeaseError::Deadline)
        } else {
            Ok(())
        }
    }
}
struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

async fn run<T: Send + 'static>(
    pool: &'static Semaphore,
    deadline: Instant,
    work: impl FnOnce(&Control) -> Result<T, DirectoryLeaseError> + Send + 'static,
) -> Result<T, DirectoryLeaseError> {
    Handle::try_current().map_err(|_| DirectoryLeaseError::Worker)?;
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or(DirectoryLeaseError::Deadline)?;
    if remaining > Duration::from_secs(60) {
        return Err(DirectoryLeaseError::Configuration);
    }
    let permit = timeout_at(deadline, pool.acquire())
        .await
        .map_err(|_| DirectoryLeaseError::Deadline)?
        .map_err(|_| DirectoryLeaseError::Worker)?;
    let cancelled = Arc::new(AtomicBool::new(false));
    let _cancel = CancelOnDrop(cancelled.clone());
    let task = spawn_work(
        permit,
        Control {
            deadline,
            cancelled,
        },
        work,
    );
    let (value, _permit) = timeout_at(deadline, task)
        .await
        .map_err(|_| DirectoryLeaseError::Deadline)?
        .map_err(|_| DirectoryLeaseError::Worker)??;
    if Instant::now() >= deadline {
        return Err(DirectoryLeaseError::Deadline);
    }
    Ok(value)
}

fn spawn_work<T: Send + 'static>(
    permit: SemaphorePermit<'static>,
    control: Control,
    work: impl FnOnce(&Control) -> Result<T, DirectoryLeaseError> + Send + 'static,
) -> JoinHandle<Result<(T, SemaphorePermit<'static>), DirectoryLeaseError>> {
    spawn_blocking(move || {
        control.check()?;
        let value = work(&control)?;
        control.check()?;
        Ok((value, permit))
    })
}

fn acquire(
    directory: &DataDirectory,
    control: &Control,
) -> Result<DirectoryLease, DirectoryLeaseError> {
    control.check()?;
    let root = open_root(&directory.0, control)?;
    match mkdirat(&root, NAMESPACE, Mode::from_bits_truncate(0o700)) {
        Ok(()) | Err(Errno::EEXIST) => {}
        Err(error) => return Err(error.into()),
    }
    control.check()?;
    let namespace = File::from(openat(
        &root,
        NAMESPACE,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::empty(),
    )?);
    let metadata = namespace.metadata()?;
    if !metadata.is_dir() || !private(&metadata) {
        return Err(DirectoryLeaseError::InvalidState);
    }
    let lock = managed_file(&namespace, LOCK, OFlag::O_RDWR | OFlag::O_CREAT, 0)?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Err(DirectoryLeaseError::InUse),
        Err(TryLockError::Error(error)) => return Err(error.into()),
    }
    control.check()?;
    let owner = match read_owner(&namespace)? {
        Some(owner) => owner,
        None => publish_owner(&namespace, control)?,
    };
    namespace.sync_all()?;
    root.sync_all()?;
    control.check()?;
    Ok(DirectoryLease(Arc::new(Lease {
        owner,
        run: Uuid::new_v4(),
        transaction: Arc::new(Semaphore::new(1)),
        guard: LeaseGuard::Held {
            directory: namespace,
            _lock: lock,
        },
    })))
}

fn open_root(path: &Path, control: &Control) -> Result<File, DirectoryLeaseError> {
    open_root_with_sync(path, control, File::sync_all)
}

fn open_root_with_sync(
    path: &Path,
    control: &Control,
    sync: impl FnMut(&File) -> io::Result<()>,
) -> Result<File, DirectoryLeaseError> {
    let (anchor, missing) = root_anchor(path, control)?;
    let mut root = File::from(open(&anchor, directory_flags(), Mode::empty())?);
    trusted_directory(&root.metadata()?)?;
    for component in missing.into_iter().rev() {
        control.check()?;
        match mkdirat(
            &root,
            component.as_os_str(),
            Mode::from_bits_truncate(0o700),
        ) {
            Ok(()) | Err(Errno::EEXIST) => {}
            Err(error) => return Err(error.into()),
        }
        let child = File::from(openat(
            &root,
            component.as_os_str(),
            directory_flags(),
            Mode::empty(),
        )?);
        trusted_directory(&child.metadata()?)?;
        root = child;
    }
    let resolved = DataDirectory::try_from(canonicalize(path)?)?;
    trusted_chain(&resolved.0, control)?;
    let observed = File::from(open(&resolved.0, directory_flags(), Mode::empty())?);
    let original = root.metadata()?;
    let current = observed.metadata()?;
    if original.dev() != current.dev() || original.ino() != current.ino() {
        return Err(DirectoryLeaseError::InvalidState);
    }
    sync_ancestors(&resolved.0, control, sync)?;
    Ok(root)
}

fn root_anchor(
    path: &Path,
    control: &Control,
) -> Result<(PathBuf, Vec<OsString>), DirectoryLeaseError> {
    let mut missing = Vec::new();
    let mut ancestor = path;
    loop {
        control.check()?;
        match symlink_metadata(ancestor) {
            Ok(metadata) => {
                if ancestor == path && metadata.file_type().is_symlink() {
                    return Err(DirectoryLeaseError::InvalidState);
                }
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                missing.push(
                    ancestor
                        .file_name()
                        .ok_or(DirectoryLeaseError::Configuration)?
                        .to_owned(),
                );
                ancestor = ancestor
                    .parent()
                    .ok_or(DirectoryLeaseError::Configuration)?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    trusted_chain(ancestor, control)?;
    let resolved = DataDirectory::try_from(canonicalize(ancestor)?)?;
    trusted_chain(&resolved.0, control)?;
    Ok((resolved.0, missing))
}

fn directory_flags() -> OFlag {
    OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC
}

fn trusted_directory(metadata: &Metadata) -> Result<(), DirectoryLeaseError> {
    if !metadata.is_dir()
        || !trusted_uid(metadata)
        || (metadata.mode() & 0o022 != 0 && metadata.mode() & 0o1000 == 0)
    {
        return Err(DirectoryLeaseError::InvalidState);
    }
    Ok(())
}

fn trusted_uid(metadata: &Metadata) -> bool {
    metadata.uid() == geteuid().as_raw() || metadata.uid() == 0
}

fn trusted_chain(path: &Path, control: &Control) -> Result<(), DirectoryLeaseError> {
    for ancestor in path.ancestors() {
        control.check()?;
        let metadata = symlink_metadata(ancestor)?;
        if metadata.file_type().is_symlink() {
            if !trusted_uid(&metadata) {
                return Err(DirectoryLeaseError::InvalidState);
            }
        } else {
            trusted_directory(&metadata)?;
        }
    }
    Ok(())
}

fn sync_ancestors(
    path: &Path,
    control: &Control,
    mut sync: impl FnMut(&File) -> io::Result<()>,
) -> Result<(), DirectoryLeaseError> {
    let mut device = None;
    for ancestor in path.ancestors() {
        control.check()?;
        let directory = File::from(open(ancestor, directory_flags(), Mode::empty())?);
        let metadata = directory.metadata()?;
        trusted_directory(&metadata)?;
        if device.is_some_and(|expected| expected != metadata.dev()) {
            break;
        }
        device = Some(metadata.dev());
        sync(&directory)?;
    }
    Ok(())
}

fn private(metadata: &Metadata) -> bool {
    metadata.uid() == geteuid().as_raw() && metadata.mode() & 0o7077 == 0
}

fn managed_file(
    directory: &File,
    name: &str,
    flags: OFlag,
    maximum: u64,
) -> Result<File, DirectoryLeaseError> {
    let file = File::from(openat(
        directory,
        name,
        flags | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC | OFlag::O_NONBLOCK | OFlag::O_NOCTTY,
        Mode::from_bits_truncate(0o600),
    )?);
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || !private(&metadata)
        || metadata.nlink() != 1
        || metadata.len() > maximum
    {
        return Err(DirectoryLeaseError::InvalidState);
    }
    Ok(file)
}

fn read_owner(directory: &File) -> Result<Option<Uuid>, DirectoryLeaseError> {
    let mut file = match managed_file(directory, OWNER, OFlag::O_RDONLY, OWNER_BYTES as u64) {
        Ok(file) => file,
        Err(DirectoryLeaseError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    if file.metadata()?.len() != OWNER_BYTES as u64 {
        return Err(DirectoryLeaseError::InvalidState);
    }
    let mut bytes = [0; OWNER_BYTES];
    file.read_exact(&mut bytes)?;
    let payload = bytes.get(..24).ok_or(DirectoryLeaseError::InvalidState)?;
    let checksum = Sha256::digest(payload);
    let expected: &[u8] = checksum.as_ref();
    if bytes.get(..8) != Some(MAGIC.as_slice()) || bytes.get(24..) != Some(expected) {
        return Err(DirectoryLeaseError::InvalidState);
    }
    let owner = Uuid::from_slice(bytes.get(8..24).ok_or(DirectoryLeaseError::InvalidState)?)
        .map_err(|_| DirectoryLeaseError::InvalidState)?;
    if owner.is_nil() {
        return Err(DirectoryLeaseError::InvalidState);
    }
    Ok(Some(owner))
}

fn publish_owner(directory: &File, control: &Control) -> Result<Uuid, DirectoryLeaseError> {
    control.check()?;
    let owner = Uuid::new_v4();
    let mut bytes = [0; OWNER_BYTES];
    bytes
        .get_mut(..8)
        .ok_or(DirectoryLeaseError::InvalidState)?
        .copy_from_slice(MAGIC);
    bytes
        .get_mut(8..24)
        .ok_or(DirectoryLeaseError::InvalidState)?
        .copy_from_slice(owner.as_bytes());
    let checksum = Sha256::digest(bytes.get(..24).ok_or(DirectoryLeaseError::InvalidState)?);
    bytes
        .get_mut(24..)
        .ok_or(DirectoryLeaseError::InvalidState)?
        .copy_from_slice(checksum.as_ref());
    let mut pending = managed_file(
        directory,
        PENDING,
        OFlag::O_RDWR | OFlag::O_CREAT,
        OWNER_BYTES as u64,
    )?;
    control.check()?;
    pending.set_len(0)?;
    pending.write_all(&bytes)?;
    pending.sync_all()?;
    control.check()?;
    renameat(directory, PENDING, directory, OWNER)?;
    directory.sync_all()?;
    Ok(owner)
}

#[cfg(test)]
#[path = "ownership_tests.rs"]
mod tests;
