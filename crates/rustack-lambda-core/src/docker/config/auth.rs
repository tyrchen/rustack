//! Bounded retained read-only workers; no ambient credentials or parser trees.
//!
//! A timed-out/dropped observer cannot stop a local filesystem syscall. Its
//! read-only blocking job retains the permit, descriptor and zeroizing buffer
//! until it exits. No late result can publish runtime readiness. POSIX checks
//! require operator-controlled ACLs; they do not prove extended-ACL privacy.

use std::{
    collections::BTreeSet,
    fmt, io,
    os::fd::OwnedFd,
    path::{Component, Path, PathBuf},
    str::{Chars, from_utf8},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use nix::{
    errno::Errno,
    fcntl::{OFlag, open, openat},
    sys::stat::{FileStat, Mode, SFlag, fstat},
    unistd::{geteuid, read},
};
use secrecy::{
    ExposeSecret, SecretString,
    zeroize::{Zeroize, Zeroizing},
};
use tokio::{
    runtime::Handle,
    sync::{Semaphore, SemaphorePermit},
    task::{JoinHandle, spawn_blocking},
    time::{Instant, timeout_at},
};
use yaml_rust2::{
    parser::{Event, Parser},
    scanner::TScalarStyle,
};

use crate::docker::{RegistryCredential, RegistryCredentials, RegistryHost};

static WORKERS: Semaphore = Semaphore::const_new(4);
const MAX_BYTES: usize = 65_536;

/// Fixed credential-source failure classes; paths, YAML and secrets are redacted.
#[derive(thiserror::Error)]
#[non_exhaustive]
pub enum RegistryAuthError {
    /// Invalid path, approved-host set or budget.
    #[error("invalid registry credential configuration")]
    Configuration,
    /// Unsafe file kind, owner, links, modes or changed observed metadata.
    #[error("unsafe registry credential file")]
    UnsafeFile,
    /// Malformed or unsupported bounded credential YAML/schema.
    #[error("invalid registry credential document")]
    Document,
    /// Source or parsed collection exceeds its hard ceiling.
    #[error("registry credential byte budget exceeded")]
    TooLarge,
    /// Absolute queue/read/parse observation deadline expired.
    #[error("registry credential deadline exceeded")]
    Deadline,
    /// The requesting observer was abandoned.
    #[error("registry credential read cancelled")]
    Cancelled,
    /// No runtime or retained blocking worker failed.
    #[error("registry credential worker failed")]
    Worker,
    /// Local descriptor IO failed without displaying source/path text.
    #[error("registry credential IO failed")]
    Io(#[source] io::Error),
}
impl fmt::Debug for RegistryAuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl From<Errno> for RegistryAuthError {
    fn from(error: Errno) -> Self {
        match error {
            Errno::ELOOP | Errno::ENOTDIR => Self::UnsafeFile,
            _ => Self::Io(error.into()),
        }
    }
}
struct Control {
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
}
impl fmt::Debug for Control {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RegistryReadControl")
    }
}
impl Control {
    fn check(&self) -> Result<(), RegistryAuthError> {
        if Instant::now() >= self.deadline {
            return Err(RegistryAuthError::Deadline);
        }
        if self.cancelled.load(Ordering::Acquire) {
            return Err(RegistryAuthError::Cancelled);
        }
        Ok(())
    }
}
struct Cancel(Arc<AtomicBool>);
impl Drop for Cancel {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
pub(super) fn valid_deadline(deadline: Instant) -> Result<(), RegistryAuthError> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or(RegistryAuthError::Deadline)?;
    if remaining > Duration::from_secs(60) {
        return Err(RegistryAuthError::Configuration);
    }
    Ok(())
}
pub(super) async fn load(
    path: PathBuf,
    approved: Vec<RegistryHost>,
    deadline: Instant,
) -> Result<RegistryCredentials, RegistryAuthError> {
    read_owned(path, approved, deadline, &WORKERS, |_| Ok(())).await
}
async fn read_owned(
    path: PathBuf,
    approved: Vec<RegistryHost>,
    deadline: Instant,
    pool: &'static Semaphore,
    before: impl FnOnce(&Control) -> Result<(), RegistryAuthError> + Send + 'static,
) -> Result<RegistryCredentials, RegistryAuthError> {
    valid_deadline(deadline)?;
    Handle::try_current().map_err(|_| RegistryAuthError::Worker)?;
    let count = approved.len();
    if count == 0 || count > 32 {
        return Err(RegistryAuthError::Configuration);
    }
    let approved: BTreeSet<_> = approved.into_iter().collect();
    if approved.len() != count {
        return Err(RegistryAuthError::Configuration);
    }
    let permit = timeout_at(deadline, pool.acquire())
        .await
        .map_err(|_| RegistryAuthError::Deadline)?
        .map_err(|_| RegistryAuthError::Worker)?;
    let cancelled = Arc::new(AtomicBool::new(false));
    let _cancel = Cancel(cancelled.clone());
    let task = spawn_read(
        path,
        approved,
        Control {
            deadline,
            cancelled,
        },
        permit,
        before,
    );
    let (credentials, _permit) = timeout_at(deadline, task)
        .await
        .map_err(|_| RegistryAuthError::Deadline)?
        .map_err(|_| RegistryAuthError::Worker)??;
    if Instant::now() >= deadline {
        return Err(RegistryAuthError::Deadline);
    }
    Ok(credentials)
}
fn spawn_read(
    path: PathBuf,
    approved: BTreeSet<RegistryHost>,
    control: Control,
    permit: SemaphorePermit<'static>,
    before: impl FnOnce(&Control) -> Result<(), RegistryAuthError> + Send + 'static,
) -> JoinHandle<Result<(RegistryCredentials, SemaphorePermit<'static>), RegistryAuthError>> {
    spawn_blocking(move || {
        control.check()?;
        before(&control)?;
        control.check()?;
        let value = read_document(&path, &approved, &control)?;
        control.check()?;
        // Ready results remain charged until observed or discarded.
        Ok::<(RegistryCredentials, SemaphorePermit<'static>), RegistryAuthError>((value, permit))
    })
}

fn flags() -> OFlag {
    OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC | OFlag::O_NONBLOCK | OFlag::O_NOCTTY
}
fn trusted_directory(file: &OwnedFd) -> Result<(), RegistryAuthError> {
    let stat = fstat(file)?;
    let owner = stat.st_uid;
    let mode = stat.st_mode;
    if SFlag::from_bits_truncate(mode) != SFlag::S_IFDIR
        || (owner != 0 && owner != geteuid().as_raw())
        || (mode & 0o022 != 0 && !(owner == 0 && mode & 0o1000 != 0))
    {
        return Err(RegistryAuthError::UnsafeFile);
    }
    Ok(())
}
fn private_file(file: &OwnedFd) -> Result<FileStat, RegistryAuthError> {
    let stat = fstat(file)?;
    if SFlag::from_bits_truncate(stat.st_mode) != SFlag::S_IFREG
        || stat.st_uid != geteuid().as_raw()
        || stat.st_nlink != 1
        || stat.st_mode & 0o7177 != 0
    {
        return Err(RegistryAuthError::UnsafeFile);
    }
    if stat.st_size <= 0
        || stat.st_size > i64::try_from(MAX_BYTES).map_err(|_| RegistryAuthError::Configuration)?
    {
        return Err(RegistryAuthError::TooLarge);
    }
    Ok(stat)
}
fn open_private(path: &Path, control: &Control) -> Result<OwnedFd, RegistryAuthError> {
    control.check()?;
    let mut directory = open(Path::new("/"), flags() | OFlag::O_DIRECTORY, Mode::empty())?;
    trusted_directory(&directory)?;
    let mut parts = path.components().peekable();
    while let Some(part) = parts.next() {
        control.check()?;
        match part {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => {
                if parts.peek().is_none() {
                    return Ok(openat(&directory, name, flags(), Mode::empty())?);
                }
                let child = openat(
                    &directory,
                    name,
                    flags() | OFlag::O_DIRECTORY,
                    Mode::empty(),
                )?;
                trusted_directory(&child)?;
                directory = child;
            }
            _ => return Err(RegistryAuthError::Configuration),
        }
    }
    Err(RegistryAuthError::Configuration)
}
fn same(before: &FileStat, after: &FileStat) -> bool {
    before.st_dev == after.st_dev
        && before.st_ino == after.st_ino
        && before.st_uid == after.st_uid
        && before.st_mode == after.st_mode
        && before.st_nlink == after.st_nlink
        && before.st_size == after.st_size
        && before.st_mtime == after.st_mtime
        && before.st_mtime_nsec == after.st_mtime_nsec
        && before.st_ctime == after.st_ctime
        && before.st_ctime_nsec == after.st_ctime_nsec
}
fn read_document(
    path: &Path,
    approved: &BTreeSet<RegistryHost>,
    control: &Control,
) -> Result<RegistryCredentials, RegistryAuthError> {
    let file = open_private(path, control)?;
    let initial = private_file(&file)?;
    let mut bytes = Zeroizing::new(Vec::new());
    bytes
        .try_reserve_exact(MAX_BYTES + 1)
        .map_err(|_| RegistryAuthError::TooLarge)?;
    bytes.resize(MAX_BYTES + 1, 0);
    let mut used = 0_usize;
    loop {
        control.check()?;
        let end = used.saturating_add(1024).min(MAX_BYTES + 1);
        let buffer = bytes
            .get_mut(used..end)
            .ok_or(RegistryAuthError::TooLarge)?;
        if buffer.is_empty() {
            return Err(RegistryAuthError::TooLarge);
        }
        let count = read(&file, buffer)?;
        if count == 0 {
            break;
        }
        used = used.checked_add(count).ok_or(RegistryAuthError::TooLarge)?;
        if used > MAX_BYTES {
            return Err(RegistryAuthError::TooLarge);
        }
    }
    control.check()?;
    validate_observation(&file, &initial, used)?;
    let text = from_utf8(bytes.get(..used).ok_or(RegistryAuthError::TooLarge)?)
        .map_err(|_| RegistryAuthError::Document)?;
    parse(text, approved, control)
}

fn validate_observation(
    file: &OwnedFd,
    initial: &FileStat,
    used: usize,
) -> Result<(), RegistryAuthError> {
    let after = private_file(file)?;
    if !same(initial, &after) || usize::try_from(initial.st_size).ok() != Some(used) {
        return Err(RegistryAuthError::UnsafeFile);
    }
    Ok(())
}

struct Events<'a> {
    parser: Parser<Chars<'a>>,
    count: usize,
    control: &'a Control,
}
impl fmt::Debug for Events<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RegistryYamlEvents(<redacted>)")
    }
}
impl Events<'_> {
    fn next(&mut self) -> Result<Event, RegistryAuthError> {
        self.control.check()?;
        self.count = self
            .count
            .checked_add(1)
            .ok_or(RegistryAuthError::TooLarge)?;
        if self.count > 512 {
            return Err(RegistryAuthError::TooLarge);
        }
        self.parser
            .next_token()
            .map(|(event, _)| event)
            .map_err(|_| RegistryAuthError::Document)
    }
    fn expect(&mut self, event: &Event) -> Result<(), RegistryAuthError> {
        let actual = self.next()?;
        if &actual != event {
            if let Event::Scalar(mut value, _, _, _) = actual {
                value.zeroize();
            }
            return Err(RegistryAuthError::Document);
        }
        Ok(())
    }
}
fn scalar(event: Event, quoted: bool, max: usize) -> Result<SecretString, RegistryAuthError> {
    match event {
        Event::Scalar(value, style, anchor, tag) => {
            let value: SecretString = value.into();
            if anchor != 0
                || tag.is_some()
                || (quoted
                    && !matches!(
                        style,
                        TScalarStyle::SingleQuoted | TScalarStyle::DoubleQuoted
                    ))
                || value.expose_secret().is_empty()
                || value.expose_secret().len() > max
            {
                return Err(RegistryAuthError::Document);
            }
            Ok(value)
        }
        _ => Err(RegistryAuthError::Document),
    }
}
fn credential(events: &mut Events<'_>) -> Result<RegistryCredential, RegistryAuthError> {
    events.expect(&Event::MappingStart(0, None))?;
    let mut user = None;
    let mut password = None;
    let mut token = None;
    loop {
        let event = events.next()?;
        if event == Event::MappingEnd {
            break;
        }
        let key = scalar(event, false, 32)?;
        let target = match key.expose_secret() {
            "username" => (&mut user, 256),
            "password" => (&mut password, 4096),
            "identityToken" => (&mut token, 8192),
            _ => return Err(RegistryAuthError::Document),
        };
        if target.0.is_some() {
            return Err(RegistryAuthError::Document);
        }
        *target.0 = Some(scalar(events.next()?, true, target.1)?);
    }
    match (user, password, token) {
        (Some(user), Some(password), None) => RegistryCredential::basic(user, password),
        (None, None, Some(token)) => RegistryCredential::identity_token(token),
        _ => return Err(RegistryAuthError::Document),
    }
    .map_err(|_| RegistryAuthError::Document)
}
fn parse(
    text: &str,
    approved: &BTreeSet<RegistryHost>,
    control: &Control,
) -> Result<RegistryCredentials, RegistryAuthError> {
    if text.len() > MAX_BYTES {
        return Err(RegistryAuthError::TooLarge);
    }
    let mut events = Events {
        parser: Parser::new_from_str(text),
        count: 0,
        control,
    };
    events.expect(&Event::StreamStart)?;
    events.expect(&Event::DocumentStart)?;
    events.expect(&Event::MappingStart(0, None))?;
    let mut records = Vec::new();
    let mut seen = BTreeSet::new();
    loop {
        let event = events.next()?;
        if event == Event::MappingEnd {
            break;
        }
        if records.len() >= 32 {
            return Err(RegistryAuthError::TooLarge);
        }
        let raw = scalar(event, false, 253)?;
        let host: RegistryHost = raw
            .expose_secret()
            .parse()
            .map_err(|_| RegistryAuthError::Document)?;
        if !approved.contains(&host) || !seen.insert(host.clone()) {
            return Err(RegistryAuthError::Document);
        }
        records.push((host, credential(&mut events)?));
    }
    events.expect(&Event::DocumentEnd)?;
    events.expect(&Event::StreamEnd)?;
    control.check()?;
    RegistryCredentials::try_from(records).map_err(|_| RegistryAuthError::Document)
}

#[cfg(test)]
mod tests;
