//! Immutable ZIP-to-TAR conversion with retained transfer-buffer admission.
//!
//! A canceled observer leaves only a bounded, read-only blocking computation.
//! Its owned source and reservation survive until the computation/output drops;
//! conversion performs no filesystem reads or daemon operations. Source-lease
//! retirement retains its normal last-owner cleanup.

use std::{
    fmt,
    io::{self, Cursor, Read, Write},
    path::Path,
    sync::{Arc, LazyLock},
};

use bytes::Bytes;
use tar::{Builder, EntryType, Header};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore},
    task::spawn_blocking,
    time::{Instant, timeout_at},
};
use zip::ZipArchive;

use super::{DockerError, check_deadline};
use crate::{
    artifact::{Architecture, MAX_ZIP_BYTES, ValidatedArtifact},
    storage::{MAX_EXTRACTED_SIZE, ZipLayout, validate_zip_directory, validated_zip_path},
};

pub(super) const MAX_CODE_ARCHIVE_BYTES: usize = 282 * 1024 * 1024;
static CONVERSIONS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(2)));

struct Storage {
    bytes: Bytes,
    architecture: Architecture,
    source: ValidatedArtifact,
    _reservation: OwnedSemaphorePermit,
}

/// Validated provided-runtime code, ready for the fixed Docker upload endpoint.
///
/// Clones share bytes, original code ownership and one global buffer reservation.
/// There is no unchecked byte constructor, host-path bind or TAR extraction.
#[derive(Clone)]
#[non_exhaustive]
pub struct DockerCodeArchive(Arc<Storage>);

impl fmt::Debug for DockerCodeArchive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DockerCodeArchive")
            .field("bytes", &self.len())
            .field("architecture", &self.architecture())
            .finish_non_exhaustive()
    }
}

impl DockerCodeArchive {
    /// Convert original staged ZIP bytes, checking the captured Linux ELF target.
    ///
    /// # Errors
    /// Returns invalid code, byte/allocation limits, expired deadline, closed
    /// admission or worker failure. No partial artifact or daemon effect is
    /// produced. Two global slots remain held until the last transfer owner drops.
    ///
    /// ```no_run
    /// # async fn prepare(source: rustack_lambda_core::artifact::ValidatedArtifact) -> Result<(), rustack_lambda_core::docker::DockerError> {
    /// use rustack_lambda_core::{artifact::Architecture, docker::DockerCodeArchive};
    /// let code = DockerCodeArchive::prepare(source, Architecture::Arm64,
    ///     tokio::time::Instant::now() + std::time::Duration::from_secs(30)).await?;
    /// assert!(!code.is_empty());
    /// # Ok(()) }
    /// ```
    pub async fn prepare(
        source: ValidatedArtifact,
        architecture: Architecture,
        deadline: Instant,
    ) -> Result<Self, DockerError> {
        with_reservation(CONVERSIONS.clone(), deadline, move |reservation| {
            convert(source, architecture, reservation, deadline)
        })
        .await
    }

    /// Encoded TAR bytes, including headers, padding and terminal blocks.
    ///
    /// ```no_run
    /// # fn inspect(code: &rustack_lambda_core::docker::DockerCodeArchive) {
    /// assert!(code.len() <= 282 * 1024 * 1024);
    /// # }
    /// ```
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.bytes.len()
    }

    /// Whether this transfer is empty; successful code archives are never empty.
    ///
    /// ```no_run
    /// # fn inspect(code: &rustack_lambda_core::docker::DockerCodeArchive) {
    /// assert!(!code.is_empty());
    /// # }
    /// ```
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.bytes.is_empty()
    }

    /// The architecture positively checked in the root bootstrap ELF header.
    ///
    /// ```no_run
    /// # fn inspect(code: &rustack_lambda_core::docker::DockerCodeArchive) {
    /// assert!(matches!(code.architecture(),
    ///     rustack_lambda_core::artifact::Architecture::Arm64 |
    ///     rustack_lambda_core::artifact::Architecture::X86_64));
    /// # }
    /// ```
    #[must_use]
    pub fn architecture(&self) -> Architecture {
        self.0.architecture
    }

    /// Base64 SHA-256 of the original ZIP, independent of transfer normalization.
    ///
    /// ```no_run
    /// # fn inspect(code: &rustack_lambda_core::docker::DockerCodeArchive) {
    /// assert_eq!(code.code_sha256().len(), 44);
    /// # }
    /// ```
    #[must_use]
    pub fn code_sha256(&self) -> &str {
        self.0.source.sha256()
    }

    pub(super) fn bytes(&self) -> &Bytes {
        &self.0.bytes
    }
}

async fn with_reservation<T: Send + 'static>(
    admission: Arc<Semaphore>,
    deadline: Instant,
    work: impl FnOnce(OwnedSemaphorePermit) -> Result<T, DockerError> + Send + 'static,
) -> Result<T, DockerError> {
    check_deadline(deadline)?;
    let reservation = timeout_at(deadline, admission.acquire_owned())
        .await
        .map_err(|_| DockerError::Deadline)?
        .map_err(|_| DockerError::Stopped)?;
    check_deadline(deadline)?;
    let result = timeout_at(deadline, spawn_blocking(move || work(reservation)))
        .await
        .map_err(|_| DockerError::Deadline)?
        .map_err(|_| DockerError::Worker)??;
    check_deadline(deadline)?;
    Ok(result)
}

fn convert(
    source: ValidatedArtifact,
    architecture: Architecture,
    reservation: OwnedSemaphorePermit,
    deadline: Instant,
) -> Result<DockerCodeArchive, DockerError> {
    check_deadline(deadline)?;
    if source.zip_bytes().len() > MAX_ZIP_BYTES {
        return Err(DockerError::TooLarge);
    }
    let mut zip = ZipArchive::new(Cursor::new(source.zip_bytes().as_ref()))
        .map_err(|_| DockerError::InvalidCode)?;
    if zip.len() > 10_000 || zip.has_duplicate_names() {
        return Err(DockerError::InvalidCode);
    }
    let mut output = Builder::new(BoundedWriter {
        bytes: Vec::new(),
        deadline,
        limit: MAX_CODE_ARCHIVE_BYTES,
    });
    let mut layout = ZipLayout::default();
    let mut extracted = 0u64;
    let mut bootstrap = false;
    for index in 0..zip.len() {
        check_deadline(deadline)?;
        let mut entry = zip.by_index(index).map_err(|_| DockerError::InvalidCode)?;
        let path = validated_zip_path(&entry).map_err(|_| DockerError::InvalidCode)?;
        if entry.is_dir() {
            validate_zip_directory(&mut entry).map_err(|_| DockerError::InvalidCode)?;
        }
        let directories = layout
            .admit(&path, entry.is_dir())
            .map_err(|_| DockerError::InvalidCode)?;
        for directory in directories {
            let mut header = Header::new_gnu();
            header.set_uid(0);
            header.set_gid(0);
            header.set_mtime(0);
            header.set_size(0);
            header.set_entry_type(EntryType::Directory);
            header.set_mode(0o555);
            output
                .append_data(&mut header, directory, io::empty())
                .map_err(|error| archive_io(error.kind()))?;
        }
        if entry.is_dir() {
            continue;
        }
        let remaining = MAX_EXTRACTED_SIZE.saturating_sub(extracted);
        if entry.size() > remaining {
            return Err(DockerError::TooLarge);
        }
        let executable = entry.unix_mode().is_some_and(|mode| mode & 0o111 != 0);
        let mut prefix = Vec::new();
        if path == Path::new("bootstrap") {
            if !executable {
                return Err(DockerError::InvalidCode);
            }
            prefix.resize(64, 0);
            entry
                .read_exact(&mut prefix)
                .map_err(|_| DockerError::InvalidCode)?;
            validate_elf(&prefix, architecture)?;
            bootstrap = true;
        }
        let declared = entry.size();
        let mut reader = CheckedReader {
            inner: Cursor::new(prefix).chain(&mut entry),
            actual: 0,
            remaining: remaining.saturating_add(1),
            deadline,
        };
        let mut header = Header::new_gnu();
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_size(declared);
        header.set_entry_type(EntryType::Regular);
        header.set_mode(if executable { 0o555 } else { 0o444 });
        output
            .append_data(&mut header, path, &mut reader)
            .map_err(|error| archive_io(error.kind()))?;
        if reader.actual != declared {
            return Err(DockerError::InvalidCode);
        }
        extracted = extracted
            .checked_add(reader.actual)
            .filter(|total| *total <= MAX_EXTRACTED_SIZE)
            .ok_or(DockerError::TooLarge)?;
    }
    if !bootstrap {
        return Err(DockerError::InvalidCode);
    }
    let writer = output
        .into_inner()
        .map_err(|error| archive_io(error.kind()))?;
    check_deadline(deadline)?;
    drop(zip);
    Ok(DockerCodeArchive(Arc::new(Storage {
        bytes: Bytes::from(writer.bytes),
        architecture,
        source,
        _reservation: reservation,
    })))
}

fn validate_elf(header: &[u8], architecture: Architecture) -> Result<(), DockerError> {
    let machine: u16 = match architecture {
        Architecture::X86_64 => 62,
        Architecture::Arm64 => 183,
    };
    if header.len() != 64
        || header.get(..7) != Some(b"\x7fELF\x02\x01\x01")
        || !matches!(header.get(16..18), Some([2 | 3, 0]))
        || header.get(18..20) != Some(machine.to_le_bytes().as_slice())
        || header.get(20..24) != Some(1u32.to_le_bytes().as_slice())
        || header.get(52..54) != Some(64u16.to_le_bytes().as_slice())
    {
        return Err(DockerError::InvalidCode);
    }
    Ok(())
}

struct CheckedReader<R> {
    inner: R,
    actual: u64,
    remaining: u64,
    deadline: Instant,
}
impl<R: Read> Read for CheckedReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        check_io(self.deadline)?;
        let maximum = usize::try_from(self.remaining.min(8192))
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?
            .min(buffer.len());
        if maximum == 0 && !buffer.is_empty() {
            return Err(io::Error::from(io::ErrorKind::FileTooLarge));
        }
        let count = self.inner.read(
            buffer
                .get_mut(..maximum)
                .ok_or(io::ErrorKind::InvalidInput)?,
        )?;
        let count64 = u64::try_from(count).map_err(|_| io::ErrorKind::InvalidData)?;
        self.remaining = self.remaining.saturating_sub(count64);
        self.actual = self
            .actual
            .checked_add(count64)
            .ok_or(io::ErrorKind::InvalidData)?;
        check_io(self.deadline)?;
        Ok(count)
    }
}

struct BoundedWriter {
    bytes: Vec<u8>,
    deadline: Instant,
    limit: usize,
}
impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        check_io(self.deadline)?;
        let needed = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .filter(|total| *total <= self.limit)
            .ok_or(io::ErrorKind::FileTooLarge)?;
        if needed > self.bytes.capacity() {
            let capacity = self
                .bytes
                .capacity()
                .saturating_mul(2)
                .max(needed)
                .min(self.limit);
            self.bytes
                .try_reserve_exact(capacity.saturating_sub(self.bytes.len()))
                .map_err(|_| io::ErrorKind::OutOfMemory)?;
        }
        self.bytes.extend_from_slice(bytes);
        check_io(self.deadline)?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        check_io(self.deadline)
    }
}

fn check_io(deadline: Instant) -> io::Result<()> {
    if Instant::now() >= deadline {
        Err(io::ErrorKind::TimedOut.into())
    } else {
        Ok(())
    }
}

fn archive_io(kind: io::ErrorKind) -> DockerError {
    match kind {
        io::ErrorKind::TimedOut => DockerError::Deadline,
        io::ErrorKind::FileTooLarge => DockerError::TooLarge,
        io::ErrorKind::OutOfMemory => DockerError::Allocation,
        _ => DockerError::InvalidCode,
    }
}

#[cfg(test)]
pub(in crate::docker) mod tests;
