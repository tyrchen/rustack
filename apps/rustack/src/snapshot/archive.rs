//! Binary `*.ss.zst` archive helpers for runtime snapshots.

use std::{
    collections::BTreeSet,
    io::{Cursor, Read as _},
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use crc32fast::Hasher;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::fs;

const MAGIC: [u8; 8] = *b"RSSNAP\0\x02";
const HEADER_LEN: usize = 36;
const HEADER_LEN_U32: u32 = 36;
const DIRECTORY_ENTRY_LEN: usize = 28;
const FOOTER_LEN: usize = 12;
const MAX_ARCHIVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_SECTION_COUNT: u32 = 4096;
const ZSTD_LEVEL: i32 = 1;

pub(super) const SECTION_MANIFEST_CBOR: u16 = 1;
pub(super) const SECTION_STATE_CBOR: u16 = 1;
const SECTION_DATA_DIRECTORY_CBOR: u16 = 1;
const SECTION_DATA_BYTES: u16 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ArchiveKind {
    Manifest,
    ServiceMeta,
    ServiceData,
}

impl ArchiveKind {
    const fn raw(self) -> u16 {
        match self {
            Self::Manifest => 1,
            Self::ServiceMeta => 2,
            Self::ServiceData => 3,
        }
    }

    fn from_raw(raw: u16) -> Result<Self> {
        match raw {
            1 => Ok(Self::Manifest),
            2 => Ok(Self::ServiceMeta),
            3 => Ok(Self::ServiceData),
            _ => bail!("unsupported snapshot archive kind {raw}"),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct ArchiveSection {
    pub(super) kind: u16,
    pub(super) flags: u16,
    pub(super) row_count: u64,
    pub(super) bytes: Vec<u8>,
}

impl ArchiveSection {
    pub(super) fn new(kind: u16, bytes: Vec<u8>, row_count: u64) -> Self {
        Self {
            kind,
            flags: 0,
            row_count,
            bytes,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ArchiveStats {
    pub(super) compressed_bytes: u64,
    pub(super) uncompressed_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DataDirectoryEntry {
    relative_path: String,
    offset: u64,
    len: u64,
    crc32: u32,
}

#[derive(Debug, Clone)]
struct DirectoryEntry {
    kind: u16,
    flags: u16,
    offset: u64,
    len: u64,
    row_count: u64,
}

pub(super) fn to_cbor<T>(value: &T) -> Result<Vec<u8>>
where
    T: Serialize,
{
    let mut bytes = Vec::new();
    ciborium::into_writer(value, &mut bytes).context("failed to encode snapshot CBOR")?;
    Ok(bytes)
}

pub(super) fn from_cbor<T>(bytes: &[u8]) -> Result<T>
where
    T: DeserializeOwned,
{
    ciborium::from_reader(bytes).context("failed to decode snapshot CBOR")
}

pub(super) async fn write_archive(
    path: &Path,
    kind: ArchiveKind,
    sections: Vec<ArchiveSection>,
) -> Result<ArchiveStats> {
    let uncompressed = encode_archive(kind, sections)?;
    let uncompressed_bytes = usize_to_u64(uncompressed.len())?;
    let compressed = compress(uncompressed).await?;
    let compressed_bytes = usize_to_u64(compressed.len())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create archive dir {}", parent.display()))?;
    }
    fs::write(path, compressed)
        .await
        .with_context(|| format!("failed to write snapshot archive {}", path.display()))?;
    Ok(ArchiveStats {
        compressed_bytes,
        uncompressed_bytes,
    })
}

pub(super) async fn read_archive(
    path: &Path,
    expected_kind: ArchiveKind,
) -> Result<Vec<ArchiveSection>> {
    let metadata = fs::metadata(path)
        .await
        .with_context(|| format!("failed to inspect snapshot archive {}", path.display()))?;
    if !metadata.is_file() {
        bail!("snapshot archive is not a regular file: {}", path.display());
    }
    if metadata.len() > MAX_ARCHIVE_BYTES {
        bail!(
            "snapshot archive {} exceeds maximum size of {MAX_ARCHIVE_BYTES} bytes",
            path.display()
        );
    }
    let compressed = fs::read(path)
        .await
        .with_context(|| format!("failed to read snapshot archive {}", path.display()))?;
    let inner = decompress(compressed).await?;
    parse_archive(&inner, expected_kind)
}

pub(super) fn get_required_section(sections: &[ArchiveSection], kind: u16) -> Result<&[u8]> {
    sections
        .iter()
        .find(|section| section.kind == kind)
        .map(|section| section.bytes.as_slice())
        .ok_or_else(|| anyhow::anyhow!("missing required snapshot archive section {kind}"))
}

pub(super) async fn pack_data_archive(
    source_dir: &Path,
    archive_path: &Path,
) -> Result<Option<ArchiveStats>> {
    if !path_exists(source_dir).await? {
        return Ok(None);
    }
    let files = collect_files(source_dir).await?;
    if files.is_empty() {
        return Ok(None);
    }

    let mut entries = Vec::with_capacity(files.len());
    let mut data = Vec::new();
    for file in files {
        let relative = archive_relative_path(source_dir, &file)?;
        let bytes = fs::read(&file)
            .await
            .with_context(|| format!("failed to read snapshot data file {}", file.display()))?;
        let offset = usize_to_u64(data.len())?;
        let len = usize_to_u64(bytes.len())?;
        let mut hasher = Hasher::new();
        hasher.update(&bytes);
        entries.push(DataDirectoryEntry {
            relative_path: relative,
            offset,
            len,
            crc32: hasher.finalize(),
        });
        data.extend_from_slice(&bytes);
    }

    let directory = to_cbor(&entries)?;
    let stats = write_archive(
        archive_path,
        ArchiveKind::ServiceData,
        vec![
            ArchiveSection::new(
                SECTION_DATA_DIRECTORY_CBOR,
                directory,
                usize_to_u64(entries.len())?,
            ),
            ArchiveSection::new(SECTION_DATA_BYTES, data, 1),
        ],
    )
    .await?;
    Ok(Some(stats))
}

pub(super) async fn unpack_data_archive(archive_path: &Path, target_dir: &Path) -> Result<()> {
    let sections = read_archive(archive_path, ArchiveKind::ServiceData).await?;
    let directory_bytes = get_required_section(&sections, SECTION_DATA_DIRECTORY_CBOR)?;
    let data = get_required_section(&sections, SECTION_DATA_BYTES)?;
    let entries: Vec<DataDirectoryEntry> = from_cbor(directory_bytes)?;

    fs::create_dir_all(target_dir).await.with_context(|| {
        format!(
            "failed to create snapshot data dir {}",
            target_dir.display()
        )
    })?;

    let mut validated_entries = Vec::with_capacity(entries.len());
    let mut seen_paths = BTreeSet::new();
    for entry in entries {
        let relative = validated_relative_path(&entry.relative_path)?;
        if !seen_paths.insert(relative.clone()) {
            bail!(
                "snapshot data archive contains duplicate path: {}",
                entry.relative_path
            );
        }
        let start = u64_to_usize(entry.offset)?;
        let len = u64_to_usize(entry.len)?;
        let end = checked_add_usize(start, len)?;
        let bytes = data.get(start..end).ok_or_else(|| {
            anyhow::anyhow!(
                "snapshot data entry is out of bounds: {}",
                entry.relative_path
            )
        })?;
        let mut hasher = Hasher::new();
        hasher.update(bytes);
        let actual_crc = hasher.finalize();
        if actual_crc != entry.crc32 {
            bail!("snapshot data entry CRC mismatch: {}", entry.relative_path);
        }
        validated_entries.push((relative, start, end));
    }

    for (relative, start, end) in validated_entries {
        let bytes = data
            .get(start..end)
            .ok_or_else(|| anyhow::anyhow!("snapshot data entry is out of bounds"))?;
        let path = target_dir.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await.with_context(|| {
                format!("failed to create snapshot data dir {}", parent.display())
            })?;
        }
        fs::write(&path, bytes)
            .await
            .with_context(|| format!("failed to write snapshot data file {}", path.display()))?;
    }

    Ok(())
}

pub(super) fn validated_relative_path(relative: &str) -> Result<PathBuf> {
    let path = Path::new(relative);
    if path.as_os_str().is_empty() || path.is_absolute() {
        bail!("snapshot path must be a relative child path: {relative}");
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        let Component::Normal(part) = component else {
            bail!("snapshot path must be a relative child path: {relative}");
        };
        normalized.push(part);
    }
    if normalized.as_os_str().is_empty() {
        bail!("snapshot path must be a relative child path: {relative}");
    }
    Ok(normalized)
}

fn encode_archive(kind: ArchiveKind, sections: Vec<ArchiveSection>) -> Result<Vec<u8>> {
    if sections.is_empty() {
        bail!("snapshot archive must contain at least one section");
    }
    let sections = sorted_sections(sections)?;
    let section_count = usize_to_u32(sections.len())?;
    let directory_len = checked_mul_usize(sections.len(), DIRECTORY_ENTRY_LEN)?;
    let payload_start = checked_add_usize(HEADER_LEN, directory_len)?;

    let mut offset = usize_to_u64(payload_start)?;
    let mut directory = Vec::with_capacity(directory_len);
    let mut payload = Vec::new();
    for section in &sections {
        let len = usize_to_u64(section.bytes.len())?;
        push_u16(&mut directory, section.kind);
        push_u16(&mut directory, section.flags);
        push_u64(&mut directory, offset);
        push_u64(&mut directory, len);
        push_u64(&mut directory, section.row_count);
        offset = offset
            .checked_add(len)
            .ok_or_else(|| anyhow::anyhow!("snapshot archive size overflow"))?;
        payload.extend_from_slice(&section.bytes);
    }

    let file_len = offset
        .checked_add(usize_to_u64(FOOTER_LEN)?)
        .ok_or_else(|| anyhow::anyhow!("snapshot archive size overflow"))?;
    let file_len_usize = u64_to_usize(file_len)?;
    let mut output = Vec::with_capacity(file_len_usize);
    output.extend_from_slice(&MAGIC);
    push_u16(&mut output, kind.raw());
    push_u16(&mut output, 0);
    push_u32(&mut output, HEADER_LEN_U32);
    push_u32(&mut output, section_count);
    push_u64(&mut output, file_len);
    push_u64(&mut output, 0);
    output.extend_from_slice(&directory);
    output.extend_from_slice(&payload);

    let mut hasher = Hasher::new();
    hasher.update(&output);
    let crc32 = hasher.finalize();
    let payload_len = usize_to_u64(output.len())?;
    push_u32(&mut output, crc32);
    push_u64(&mut output, payload_len);

    if output.len() != file_len_usize {
        bail!("snapshot archive writer produced an invalid length");
    }

    Ok(output)
}

fn parse_archive(bytes: &[u8], expected_kind: ArchiveKind) -> Result<Vec<ArchiveSection>> {
    let min_len = checked_add_usize(HEADER_LEN, FOOTER_LEN)?;
    if bytes.len() < min_len {
        bail!("snapshot archive is too short");
    }

    let mut cursor = BinaryCursor::new(bytes);
    let magic = cursor.read_array_8()?;
    if magic != MAGIC {
        bail!("snapshot archive magic is invalid");
    }
    let kind = ArchiveKind::from_raw(cursor.read_u16()?)?;
    if kind != expected_kind {
        bail!(
            "snapshot archive kind mismatch: expected {}, got {}",
            expected_kind.raw(),
            kind.raw()
        );
    }
    let flags = cursor.read_u16()?;
    if flags != 0 {
        bail!("snapshot archive has unsupported flags");
    }
    let header_len = cursor.read_u32()?;
    if header_len != HEADER_LEN_U32 {
        bail!("snapshot archive header length is unsupported");
    }
    let section_count = cursor.read_u32()?;
    if section_count == 0 || section_count > MAX_SECTION_COUNT {
        bail!("snapshot archive section count is invalid");
    }
    let file_len = cursor.read_u64()?;
    if file_len != usize_to_u64(bytes.len())? {
        bail!("snapshot archive length does not match header");
    }
    let reserved = cursor.read_u64()?;
    if reserved != 0 {
        bail!("snapshot archive reserved field is non-zero");
    }

    let footer_start = bytes
        .len()
        .checked_sub(FOOTER_LEN)
        .ok_or_else(|| anyhow::anyhow!("snapshot archive is too short"))?;
    validate_footer(bytes, footer_start)?;

    let directory_len = checked_mul_usize(u32_to_usize(section_count)?, DIRECTORY_ENTRY_LEN)?;
    let directory_end = checked_add_usize(HEADER_LEN, directory_len)?;
    if directory_end > footer_start {
        bail!("snapshot archive directory is out of bounds");
    }

    let directory_bytes = bytes
        .get(HEADER_LEN..directory_end)
        .ok_or_else(|| anyhow::anyhow!("snapshot archive directory is out of bounds"))?;
    let mut dir_cursor = BinaryCursor::new(directory_bytes);
    let mut entries = Vec::with_capacity(u32_to_usize(section_count)?);
    let mut seen = BTreeSet::new();
    for _ in 0..section_count {
        let entry = DirectoryEntry {
            kind: dir_cursor.read_u16()?,
            flags: dir_cursor.read_u16()?,
            offset: dir_cursor.read_u64()?,
            len: dir_cursor.read_u64()?,
            row_count: dir_cursor.read_u64()?,
        };
        if entry.flags != 0 {
            bail!("snapshot archive section has unsupported flags");
        }
        if !seen.insert(entry.kind) {
            bail!("snapshot archive contains duplicate section {}", entry.kind);
        }
        entries.push(entry);
    }
    validate_non_overlapping_sections(&mut entries, directory_end, footer_start)?;

    let mut sections = Vec::with_capacity(entries.len());
    for entry in entries {
        let start = u64_to_usize(entry.offset)?;
        let len = u64_to_usize(entry.len)?;
        let end = checked_add_usize(start, len)?;
        let section_bytes = bytes
            .get(start..end)
            .ok_or_else(|| anyhow::anyhow!("snapshot archive section is out of bounds"))?;
        sections.push(ArchiveSection {
            kind: entry.kind,
            flags: entry.flags,
            row_count: entry.row_count,
            bytes: section_bytes.to_vec(),
        });
    }
    Ok(sections)
}

fn sorted_sections(mut sections: Vec<ArchiveSection>) -> Result<Vec<ArchiveSection>> {
    sections.sort_by_key(|section| section.kind);
    let mut seen = BTreeSet::new();
    for section in &sections {
        if section.kind == 0 {
            bail!("snapshot archive section kind 0 is reserved");
        }
        if section.flags != 0 {
            bail!("snapshot archive section flags are unsupported");
        }
        if !seen.insert(section.kind) {
            bail!(
                "snapshot archive contains duplicate section {}",
                section.kind
            );
        }
    }
    Ok(sections)
}

fn validate_non_overlapping_sections(
    entries: &mut [DirectoryEntry],
    directory_end: usize,
    footer_start: usize,
) -> Result<()> {
    entries.sort_by_key(|entry| entry.offset);
    let mut previous_end = directory_end;
    for entry in entries {
        let start = u64_to_usize(entry.offset)?;
        let len = u64_to_usize(entry.len)?;
        let end = checked_add_usize(start, len)?;
        if start < directory_end || end > footer_start || start < previous_end {
            bail!("snapshot archive section range is invalid");
        }
        previous_end = end;
    }
    Ok(())
}

fn validate_footer(bytes: &[u8], footer_start: usize) -> Result<()> {
    let footer = bytes
        .get(footer_start..)
        .ok_or_else(|| anyhow::anyhow!("snapshot archive footer is missing"))?;
    let mut cursor = BinaryCursor::new(footer);
    let expected_crc = cursor.read_u32()?;
    let payload_len = cursor.read_u64()?;
    if payload_len != usize_to_u64(footer_start)? {
        bail!("snapshot archive footer length does not match");
    }
    let payload = bytes
        .get(..footer_start)
        .ok_or_else(|| anyhow::anyhow!("snapshot archive footer range is invalid"))?;
    let mut hasher = Hasher::new();
    hasher.update(payload);
    let actual_crc = hasher.finalize();
    if actual_crc != expected_crc {
        bail!("snapshot archive CRC mismatch");
    }
    Ok(())
}

async fn compress(bytes: Vec<u8>) -> Result<Vec<u8>> {
    tokio::task::spawn_blocking(move || {
        zstd::stream::encode_all(Cursor::new(bytes), ZSTD_LEVEL)
            .context("failed to compress snapshot archive")
    })
    .await
    .context("snapshot compression task failed")?
}

async fn decompress(bytes: Vec<u8>) -> Result<Vec<u8>> {
    tokio::task::spawn_blocking(move || decode_zstd_bounded(bytes))
        .await
        .context("snapshot decompression task failed")?
}

fn decode_zstd_bounded(bytes: Vec<u8>) -> Result<Vec<u8>> {
    let read_limit = MAX_ARCHIVE_BYTES
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("snapshot archive size overflow"))?;
    let decoder = zstd::stream::read::Decoder::new(Cursor::new(bytes))
        .context("failed to create snapshot zstd decoder")?;
    let mut limited = decoder.take(read_limit);
    let mut output = Vec::new();
    limited
        .read_to_end(&mut output)
        .context("failed to decompress snapshot archive")?;
    if usize_to_u64(output.len())? > MAX_ARCHIVE_BYTES {
        bail!("decompressed snapshot archive exceeds maximum size");
    }
    Ok(output)
}

async fn collect_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut pending = vec![root.to_owned()];
    let mut files = Vec::new();
    while let Some(dir) = pending.pop() {
        let mut entries = fs::read_dir(&dir)
            .await
            .with_context(|| format!("failed to read snapshot data dir {}", dir.display()))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .with_context(|| format!("failed to read snapshot data dir {}", dir.display()))?
        {
            let file_type = entry.file_type().await.with_context(|| {
                format!(
                    "failed to inspect snapshot data path {}",
                    entry.path().display()
                )
            })?;
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file() {
                files.push(entry.path());
            }
        }
    }
    files.sort_by(|left, right| left.as_os_str().cmp(right.as_os_str()));
    Ok(files)
}

fn archive_relative_path(root: &Path, file: &Path) -> Result<String> {
    let relative = file.strip_prefix(root).with_context(|| {
        format!(
            "snapshot data file {} is outside {}",
            file.display(),
            root.display()
        )
    })?;
    let relative = relative.to_str().ok_or_else(|| {
        anyhow::anyhow!(
            "snapshot data path is not valid UTF-8: {}",
            relative.display()
        )
    })?;
    let _ = validated_relative_path(relative)?;
    Ok(relative.to_owned())
}

async fn path_exists(path: &Path) -> Result<bool> {
    match fs::metadata(path).await {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error)
            .with_context(|| format!("failed to inspect snapshot path {}", path.display())),
    }
}

fn push_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(output: &mut Vec<u8>, value: u64) {
    output.extend_from_slice(&value.to_le_bytes());
}

#[derive(Debug)]
struct BinaryCursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> BinaryCursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn read_array_8(&mut self) -> Result<[u8; 8]> {
        let bytes = self.read_exact(8)?;
        let mut array = [0_u8; 8];
        array.copy_from_slice(bytes);
        Ok(array)
    }

    fn read_u16(&mut self) -> Result<u16> {
        let bytes = self.read_exact(2)?;
        let mut array = [0_u8; 2];
        array.copy_from_slice(bytes);
        Ok(u16::from_le_bytes(array))
    }

    fn read_u32(&mut self) -> Result<u32> {
        let bytes = self.read_exact(4)?;
        let mut array = [0_u8; 4];
        array.copy_from_slice(bytes);
        Ok(u32::from_le_bytes(array))
    }

    fn read_u64(&mut self) -> Result<u64> {
        let bytes = self.read_exact(8)?;
        let mut array = [0_u8; 8];
        array.copy_from_slice(bytes);
        Ok(u64::from_le_bytes(array))
    }

    fn read_exact(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = checked_add_usize(self.position, len)?;
        let bytes = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| anyhow::anyhow!("snapshot archive ended unexpectedly"))?;
        self.position = end;
        Ok(bytes)
    }
}

fn checked_add_usize(left: usize, right: usize) -> Result<usize> {
    left.checked_add(right)
        .ok_or_else(|| anyhow::anyhow!("snapshot archive size overflow"))
}

fn checked_mul_usize(left: usize, right: usize) -> Result<usize> {
    left.checked_mul(right)
        .ok_or_else(|| anyhow::anyhow!("snapshot archive size overflow"))
}

fn usize_to_u32(value: usize) -> Result<u32> {
    u32::try_from(value).context("snapshot archive value does not fit in u32")
}

fn u32_to_usize(value: u32) -> Result<usize> {
    usize::try_from(value).context("snapshot archive value does not fit in usize")
}

fn usize_to_u64(value: usize) -> Result<u64> {
    u64::try_from(value).context("snapshot archive value does not fit in u64")
}

fn u64_to_usize(value: u64) -> Result<usize> {
    usize::try_from(value).context("snapshot archive value does not fit in usize")
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[tokio::test]
    async fn test_should_round_trip_archive_sections() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("meta.ss.zst");
        let stats = write_archive(
            &path,
            ArchiveKind::ServiceMeta,
            vec![ArchiveSection::new(
                SECTION_STATE_CBOR,
                b"state".to_vec(),
                7,
            )],
        )
        .await?;
        assert!(stats.compressed_bytes > 0);
        assert!(stats.uncompressed_bytes > 0);

        let sections = read_archive(&path, ArchiveKind::ServiceMeta).await?;
        assert_eq!(sections.len(), 1);
        let Some(section) = sections.first() else {
            bail!("archive should contain one section");
        };
        assert_eq!(section.kind, SECTION_STATE_CBOR);
        assert_eq!(section.row_count, 7);
        assert_eq!(section.bytes, b"state");
        Ok(())
    }

    #[tokio::test]
    async fn test_should_reject_archive_kind_mismatch() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("meta.ss.zst");
        write_archive(
            &path,
            ArchiveKind::ServiceMeta,
            vec![ArchiveSection::new(
                SECTION_STATE_CBOR,
                b"state".to_vec(),
                1,
            )],
        )
        .await?;

        let result = read_archive(&path, ArchiveKind::Manifest).await;
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn test_should_reject_corrupt_archive_crc() -> Result<()> {
        let archive = encode_archive(
            ArchiveKind::ServiceMeta,
            vec![ArchiveSection::new(
                SECTION_STATE_CBOR,
                b"state".to_vec(),
                1,
            )],
        )?;
        let mut corrupt = archive;
        let index = corrupt
            .len()
            .checked_sub(1)
            .ok_or_else(|| anyhow::anyhow!("archive should contain footer byte"))?;
        let Some(byte) = corrupt.get_mut(index) else {
            bail!("archive should contain mutable footer byte");
        };
        *byte = byte.wrapping_add(1);
        let result = parse_archive(&corrupt, ArchiveKind::ServiceMeta);
        assert!(result.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn test_should_pack_and_unpack_data_archive() -> Result<()> {
        let dir = tempdir()?;
        let source = dir.path().join("source");
        let nested = source.join("objects");
        fs::create_dir_all(&nested).await?;
        fs::write(nested.join("0.bin"), b"hello").await?;

        let archive = dir.path().join("data.ss.zst");
        let stats = pack_data_archive(&source, &archive)
            .await
            .context("data archive should pack")?
            .ok_or_else(|| anyhow::anyhow!("non-empty archive should be written"))?;
        assert!(stats.compressed_bytes > 0);

        let target = dir.path().join("target");
        unpack_data_archive(&archive, &target).await?;
        let restored = fs::read(target.join("objects").join("0.bin"))
            .await
            .context("restored file should be readable")?;
        assert_eq!(restored, b"hello");
        Ok(())
    }

    #[tokio::test]
    async fn test_should_reject_duplicate_data_archive_paths() -> Result<()> {
        let dir = tempdir()?;
        let archive = dir.path().join("data.ss.zst");
        let mut hasher = Hasher::new();
        hasher.update(b"hello");
        let crc32 = hasher.finalize();
        let directory = to_cbor(&vec![
            DataDirectoryEntry {
                relative_path: "objects/0.bin".to_owned(),
                offset: 0,
                len: 5,
                crc32,
            },
            DataDirectoryEntry {
                relative_path: "objects//0.bin".to_owned(),
                offset: 0,
                len: 5,
                crc32,
            },
        ])?;
        write_archive(
            &archive,
            ArchiveKind::ServiceData,
            vec![
                ArchiveSection::new(SECTION_DATA_DIRECTORY_CBOR, directory, 2),
                ArchiveSection::new(SECTION_DATA_BYTES, b"hello".to_vec(), 1),
            ],
        )
        .await?;

        let result = unpack_data_archive(&archive, &dir.path().join("target")).await;
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn test_should_reject_data_archive_path_traversal() {
        let result = validated_relative_path("../objects/0.bin");
        assert!(result.is_err());
    }
}
