//! In-memory storage backend for S3 object body data.
//!
//! Objects below a configurable threshold are kept in memory as [`Bytes`].
//! Objects above the threshold are spilled to temporary files on disk.
//!
//! The [`InMemoryStorage`] type is thread-safe and uses [`DashMap`] for
//! concurrent access to stored objects and multipart parts.
//!
//! # Spillover to Disk
//!
//! When object data exceeds the configured `max_memory_size`, the bytes
//! are written to a temporary file via the [`tempfile`] crate. On-disk data
//! is automatically cleaned up when the entry is removed from the map (via
//! the internal stored data `Drop` implementation).

use std::path::PathBuf;

use bytes::{Bytes, BytesMut};
use dashmap::DashMap;
use tokio::io::AsyncReadExt as _;
use tracing::{debug, trace, warn};

use crate::{checksums, error::S3ServiceError};

/// Composite key identifying a stored object: `(bucket, key, version_id)`.
type StorageKey = (String, String, String);

/// Composite key identifying a multipart part: `(bucket, upload_id, part_number)`.
type PartKey = (String, String, u32);

/// Default maximum object size (in bytes) kept in memory before spilling to
/// disk. Objects larger than this threshold are written to temporary files.
///
/// The default is 512 KiB.
const DEFAULT_MAX_MEMORY_SIZE: usize = 524_288;

// ---------------------------------------------------------------------------
// WriteResult
// ---------------------------------------------------------------------------

/// Result of writing data to storage.
///
/// Contains the computed ETag, data size, and raw MD5 hex digest for the
/// written object or part.
#[derive(Debug, Clone)]
pub struct WriteResult {
    /// The ETag (quoted hex MD5) of the written data.
    pub etag: String,
    /// The size in bytes.
    pub size: u64,
    /// The MD5 hex digest (unquoted).
    pub md5_hex: String,
}

// ---------------------------------------------------------------------------
// StoredData
// ---------------------------------------------------------------------------

/// Internal representation of stored data.
///
/// Small objects are kept in memory as [`Bytes`]. Large objects are spilled
/// to a temporary file on disk. When a [`StoredData::OnDisk`] value is
/// dropped, the temporary file is removed.
enum StoredData {
    /// Small objects kept entirely in memory.
    InMemory {
        /// The raw object bytes.
        data: Bytes,
    },
    /// Large objects spilled to a temp file.
    OnDisk {
        /// Path to the temporary file.
        path: PathBuf,
        /// Size of the stored data in bytes.
        size: u64,
    },
}

impl std::fmt::Debug for StoredData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InMemory { data } => f
                .debug_struct("InMemory")
                .field("size", &data.len())
                .finish(),
            Self::OnDisk { path, size } => f
                .debug_struct("OnDisk")
                .field("path", path)
                .field("size", size)
                .finish(),
        }
    }
}

impl Drop for StoredData {
    fn drop(&mut self) {
        if let Self::OnDisk { path, .. } = self {
            let path = path.clone();
            // Spawn async file removal on the tokio runtime. If no runtime is
            // active (e.g. during test teardown), the file will be cleaned up
            // by the OS temp-dir reaper.
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    if let Err(e) = tokio::fs::remove_file(&path).await {
                        if e.kind() != std::io::ErrorKind::NotFound {
                            warn!(path = %path.display(), error = %e, "failed to remove temp file");
                        }
                    } else {
                        trace!(path = %path.display(), "removed temp file");
                    }
                });
            }
        }
    }
}

impl StoredData {
    /// Read the full data from this stored entry.
    async fn read_all(&self) -> Result<Bytes, S3ServiceError> {
        match self {
            Self::InMemory { data } => Ok(data.clone()),
            Self::OnDisk { path, size } => {
                let mut file = tokio::fs::File::open(path).await.map_err(|e| {
                    S3ServiceError::Internal(anyhow::anyhow!(
                        "failed to open temp file {}: {e}",
                        path.display()
                    ))
                })?;
                let capacity = usize::try_from(*size).unwrap_or(usize::MAX);
                let mut buf = Vec::with_capacity(capacity);
                file.read_to_end(&mut buf).await.map_err(|e| {
                    S3ServiceError::Internal(anyhow::anyhow!(
                        "failed to read temp file {}: {e}",
                        path.display()
                    ))
                })?;
                Ok(Bytes::from(buf))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// InMemoryStorage
// ---------------------------------------------------------------------------

/// In-memory storage with automatic spillover to tempfiles for large objects.
///
/// Thread-safe: uses [`DashMap`] for concurrent access. Objects larger than
/// the configured `max_memory_size` are transparently written to
/// temporary files and read back on demand.
///
/// # Examples
///
/// ```
/// use bytes::Bytes;
/// use rustack_s3_core::storage::InMemoryStorage;
///
/// # tokio_test::block_on(async {
/// let storage = InMemoryStorage::new(1024);
/// let result = storage
///     .write_object("my-bucket", "hello.txt", "null", Bytes::from("hello"))
///     .await
///     .unwrap();
/// assert_eq!(result.size, 5);
///
/// let data = storage
///     .read_object("my-bucket", "hello.txt", "null", None)
///     .await
///     .unwrap();
/// assert_eq!(data.as_ref(), b"hello");
/// # });
/// ```
pub struct InMemoryStorage {
    /// Object data keyed by `(bucket, key, version_id)`.
    objects: DashMap<StorageKey, StoredData>,
    /// Multipart part data keyed by `(bucket, upload_id, part_number)`.
    parts: DashMap<PartKey, StoredData>,
    /// Max size in bytes for in-memory storage before spilling to disk.
    max_memory_size: usize,
}

impl std::fmt::Debug for InMemoryStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryStorage")
            .field("objects_count", &self.objects.len())
            .field("parts_count", &self.parts.len())
            .field("max_memory_size", &self.max_memory_size)
            .finish()
    }
}

impl Default for InMemoryStorage {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_MEMORY_SIZE)
    }
}

impl InMemoryStorage {
    /// Create a new storage backend with the given memory threshold.
    ///
    /// Objects larger than `max_memory_size` bytes are spilled to temporary
    /// files on disk.
    #[must_use]
    pub fn new(max_memory_size: usize) -> Self {
        debug!(max_memory_size, "creating InMemoryStorage");
        Self {
            objects: DashMap::new(),
            parts: DashMap::new(),
            max_memory_size,
        }
    }

    /// Return the default maximum in-memory object size (512 KiB).
    #[must_use]
    pub fn default_max_memory_size() -> usize {
        DEFAULT_MAX_MEMORY_SIZE
    }

    /// Store object data. Computes MD5 and returns a [`WriteResult`].
    ///
    /// If the data exceeds the configured memory threshold, it is spilled
    /// to a temporary file on disk.
    ///
    /// # Errors
    ///
    /// Returns [`S3ServiceError::Internal`] if the temporary file cannot be
    /// created or written.
    pub async fn write_object(
        &self,
        bucket: &str,
        key: &str,
        version_id: &str,
        data: Bytes,
    ) -> Result<WriteResult, S3ServiceError> {
        let md5_hex = checksums::compute_md5(&data);
        let etag = format!("\"{md5_hex}\"");
        let size = data.len() as u64;

        let stored = self.store_data(data).await?;

        trace!(bucket, key, version_id, size, "stored object data");
        self.objects.insert(
            (bucket.to_owned(), key.to_owned(), version_id.to_owned()),
            stored,
        );

        Ok(WriteResult {
            etag,
            size,
            md5_hex,
        })
    }

    /// Read object data. Returns the full [`Bytes`] for the object.
    ///
    /// If `range` is specified as `(start, end)` (inclusive on both ends),
    /// only that byte range is returned.
    ///
    /// # Errors
    ///
    /// - [`S3ServiceError::NoSuchKey`] if the object is not found.
    /// - [`S3ServiceError::InvalidRange`] if the range is out of bounds.
    /// - [`S3ServiceError::Internal`] if the on-disk file cannot be read.
    pub async fn read_object(
        &self,
        bucket: &str,
        key: &str,
        version_id: &str,
        range: Option<(u64, u64)>,
    ) -> Result<Bytes, S3ServiceError> {
        let storage_key = (bucket.to_owned(), key.to_owned(), version_id.to_owned());
        let entry = self
            .objects
            .get(&storage_key)
            .ok_or_else(|| S3ServiceError::NoSuchKey {
                key: key.to_owned(),
            })?;

        let all_data = entry.value().read_all().await?;

        match range {
            Some((start, end)) => {
                let data_len = all_data.len();
                let start_idx = usize::try_from(start).map_err(|_| S3ServiceError::InvalidRange)?;
                let end_idx = usize::try_from(end).map_err(|_| S3ServiceError::InvalidRange)?;
                if start_idx >= data_len || end_idx >= data_len || start_idx > end_idx {
                    return Err(S3ServiceError::InvalidRange);
                }
                Ok(all_data.slice(start_idx..=end_idx))
            }
            None => Ok(all_data),
        }
    }

    /// Copy object data from one location to another.
    ///
    /// Reads the source object data, then writes it to the destination
    /// location. Returns a [`WriteResult`] for the destination object.
    ///
    /// # Errors
    ///
    /// - [`S3ServiceError::NoSuchKey`] if the source object is not found.
    /// - [`S3ServiceError::Internal`] if disk I/O fails.
    pub async fn copy_object(
        &self,
        src_bucket: &str,
        src_key: &str,
        src_version_id: &str,
        dst_bucket: &str,
        dst_key: &str,
        dst_version_id: &str,
    ) -> Result<WriteResult, S3ServiceError> {
        let data = self
            .read_object(src_bucket, src_key, src_version_id, None)
            .await?;

        debug!(
            src_bucket,
            src_key,
            src_version_id,
            dst_bucket,
            dst_key,
            dst_version_id,
            size = data.len(),
            "copying object data"
        );

        self.write_object(dst_bucket, dst_key, dst_version_id, data)
            .await
    }

    /// Delete object data.
    ///
    /// Removes the stored data for the given object key. If the data was on
    /// disk, the temporary file is cleaned up via the [`Drop`] implementation.
    /// This is a no-op if the object does not exist.
    pub fn delete_object(&self, bucket: &str, key: &str, version_id: &str) {
        let storage_key = (bucket.to_owned(), key.to_owned(), version_id.to_owned());
        if self.objects.remove(&storage_key).is_some() {
            trace!(bucket, key, version_id, "deleted object data");
        }
    }

    /// Store a multipart part.
    ///
    /// Computes MD5 and returns a [`WriteResult`]. If the part data exceeds
    /// the memory threshold, it is spilled to a temporary file.
    ///
    /// # Errors
    ///
    /// Returns [`S3ServiceError::Internal`] if the temporary file cannot be
    /// created or written.
    pub async fn write_part(
        &self,
        bucket: &str,
        upload_id: &str,
        part_number: u32,
        data: Bytes,
    ) -> Result<WriteResult, S3ServiceError> {
        let md5_hex = checksums::compute_md5(&data);
        let etag = format!("\"{md5_hex}\"");
        let size = data.len() as u64;

        let stored = self.store_data(data).await?;

        trace!(bucket, upload_id, part_number, size, "stored part data");
        self.parts.insert(
            (bucket.to_owned(), upload_id.to_owned(), part_number),
            stored,
        );

        Ok(WriteResult {
            etag,
            size,
            md5_hex,
        })
    }

    /// Read a multipart part's data.
    ///
    /// # Errors
    ///
    /// - [`S3ServiceError::InvalidPart`] if the part does not exist.
    /// - [`S3ServiceError::Internal`] if the on-disk file cannot be read.
    pub async fn read_part(
        &self,
        bucket: &str,
        upload_id: &str,
        part_number: u32,
    ) -> Result<Bytes, S3ServiceError> {
        let part_key = (bucket.to_owned(), upload_id.to_owned(), part_number);
        let entry = self
            .parts
            .get(&part_key)
            .ok_or(S3ServiceError::InvalidPart)?;

        entry.value().read_all().await
    }

    /// Assemble parts into a final object. Concatenates part data in order.
    ///
    /// Returns a tuple of `(WriteResult, Vec<String>)` where the vector
    /// contains the individual (unquoted) MD5 hex digests for each part.
    /// The [`WriteResult::etag`] is a composite ETag in the format
    /// `"<md5>-<part_count>"`.
    ///
    /// # Errors
    ///
    /// - [`S3ServiceError::InvalidPart`] if any requested part does not exist.
    /// - [`S3ServiceError::Internal`] if disk I/O fails.
    pub async fn complete_multipart(
        &self,
        bucket: &str,
        upload_id: &str,
        key: &str,
        version_id: &str,
        part_numbers: &[u32],
    ) -> Result<(WriteResult, Vec<String>), S3ServiceError> {
        let mut combined = BytesMut::new();
        let mut part_md5_hexes = Vec::with_capacity(part_numbers.len());

        for &part_number in part_numbers {
            let part_data = self.read_part(bucket, upload_id, part_number).await?;
            let md5_hex = checksums::compute_md5(&part_data);
            part_md5_hexes.push(md5_hex);
            combined.extend_from_slice(&part_data);
        }

        let combined_bytes = combined.freeze();
        let size = combined_bytes.len() as u64;

        // Compute composite ETag: MD5-of-concatenated-MD5s with part count suffix.
        let etag = checksums::compute_multipart_etag(&part_md5_hexes, part_numbers.len());

        // Store the assembled object.
        let stored = self.store_data(combined_bytes).await?;
        self.objects.insert(
            (bucket.to_owned(), key.to_owned(), version_id.to_owned()),
            stored,
        );

        // Clean up the parts for this upload.
        self.abort_multipart(bucket, upload_id);

        debug!(
            bucket,
            upload_id,
            key,
            version_id,
            size,
            parts = part_numbers.len(),
            "completed multipart upload"
        );

        // The md5_hex for the composite result is the hash portion of the ETag
        // (without quotes and without the -N suffix).
        let composite_md5 = etag
            .trim_matches('"')
            .split('-')
            .next()
            .unwrap_or_default()
            .to_owned();

        Ok((
            WriteResult {
                etag,
                size,
                md5_hex: composite_md5,
            },
            part_md5_hexes,
        ))
    }

    /// Delete all parts for a multipart upload.
    ///
    /// Removes all stored part data associated with the given upload ID.
    /// Temporary files are cleaned up automatically via [`Drop`].
    pub fn abort_multipart(&self, bucket: &str, upload_id: &str) {
        self.parts.retain(|key, _| {
            let matches = key.0 == bucket && key.1 == upload_id;
            if matches {
                trace!(bucket, upload_id, part_number = key.2, "removing part data");
            }
            !matches
        });
    }

    /// Delete all data (objects and parts) for a bucket.
    ///
    /// This removes both object data and any in-progress multipart part data
    /// associated with the bucket.
    pub fn delete_bucket_data(&self, bucket: &str) {
        let obj_before = self.objects.len();
        self.objects.retain(|key, _| key.0 != bucket);
        let obj_removed = obj_before - self.objects.len();

        let part_removed = self.remove_parts_by_bucket(bucket);

        debug!(
            bucket,
            objects_removed = obj_removed,
            parts_removed = part_removed,
            "deleted all bucket data"
        );
    }

    /// Reset all storage, removing every object and part.
    pub fn reset(&self) {
        debug!("resetting all storage data");
        self.objects.clear();
        self.parts.clear();
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Store data either in memory or on disk, depending on size.
    async fn store_data(&self, data: Bytes) -> Result<StoredData, S3ServiceError> {
        if data.len() > self.max_memory_size {
            self.spill_to_disk(&data).await
        } else {
            Ok(StoredData::InMemory { data })
        }
    }

    /// Write data to a temporary file and return an [`StoredData::OnDisk`].
    async fn spill_to_disk(&self, data: &[u8]) -> Result<StoredData, S3ServiceError> {
        let size = data.len() as u64;

        // Create the temp file synchronously (tempfile::NamedTempFile uses
        // the OS temp directory) then persist it so it is not deleted when
        // the NamedTempFile handle is dropped -- we manage cleanup in Drop.
        let temp = tempfile::NamedTempFile::new().map_err(|e| {
            S3ServiceError::Internal(anyhow::anyhow!("failed to create temp file: {e}"))
        })?;
        let path = temp.path().to_path_buf();

        // Persist the named temp file so it is not auto-deleted.
        temp.persist(&path).map_err(|e| {
            S3ServiceError::Internal(anyhow::anyhow!(
                "failed to persist temp file {}: {e}",
                path.display()
            ))
        })?;

        // Write data asynchronously.
        tokio::fs::write(&path, data).await.map_err(|e| {
            S3ServiceError::Internal(anyhow::anyhow!(
                "failed to write temp file {}: {e}",
                path.display()
            ))
        })?;

        trace!(path = %path.display(), size, "spilled data to disk");
        Ok(StoredData::OnDisk { path, size })
    }

    /// Remove all part entries whose bucket matches `bucket`.
    /// Returns the number of entries removed.
    fn remove_parts_by_bucket(&self, bucket: &str) -> usize {
        let before = self.parts.len();
        self.parts.retain(|key, _| key.0 != bucket);
        before - self.parts.len()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Threshold for tests: 64 bytes. Anything larger spills to disk.
    const TEST_THRESHOLD: usize = 64;

    fn small_data() -> Bytes {
        Bytes::from("hello world")
    }

    fn large_data() -> Bytes {
        Bytes::from(vec![0xAB_u8; TEST_THRESHOLD + 1])
    }

    // -----------------------------------------------------------------------
    // Small object write / read
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_should_write_and_read_small_object() {
        let storage = InMemoryStorage::new(TEST_THRESHOLD);
        let data = small_data();
        let result = storage
            .write_object("bucket", "key", "null", data.clone())
            .await;
        assert!(result.is_ok());

        let wr = result.unwrap_or_else(|e| panic!("write_object failed: {e}"));
        assert_eq!(wr.size, data.len() as u64);
        assert!(wr.etag.starts_with('"'));
        assert!(wr.etag.ends_with('"'));
        assert_eq!(wr.md5_hex, checksums::compute_md5(&data));

        let read_data = storage
            .read_object("bucket", "key", "null", None)
            .await
            .unwrap_or_else(|e| panic!("read_object failed: {e}"));
        assert_eq!(read_data, data);
    }

    // -----------------------------------------------------------------------
    // Large object write / read (spillover)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_should_write_and_read_large_object_on_disk() {
        let storage = InMemoryStorage::new(TEST_THRESHOLD);
        let data = large_data();
        let wr = storage
            .write_object("bucket", "big", "null", data.clone())
            .await
            .unwrap_or_else(|e| panic!("write_object failed: {e}"));

        assert_eq!(wr.size, data.len() as u64);

        let read_data = storage
            .read_object("bucket", "big", "null", None)
            .await
            .unwrap_or_else(|e| panic!("read_object failed: {e}"));
        assert_eq!(read_data, data);
    }

    // -----------------------------------------------------------------------
    // Range reads
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_should_read_object_with_range() {
        let storage = InMemoryStorage::new(TEST_THRESHOLD);
        let data = Bytes::from("hello world");
        storage
            .write_object("bucket", "key", "null", data)
            .await
            .unwrap_or_else(|e| panic!("write failed: {e}"));

        // Read bytes 0..=4 => "hello"
        let range_data = storage
            .read_object("bucket", "key", "null", Some((0, 4)))
            .await
            .unwrap_or_else(|e| panic!("range read failed: {e}"));
        assert_eq!(range_data.as_ref(), b"hello");

        // Read bytes 6..=10 => "world"
        let range_data = storage
            .read_object("bucket", "key", "null", Some((6, 10)))
            .await
            .unwrap_or_else(|e| panic!("range read failed: {e}"));
        assert_eq!(range_data.as_ref(), b"world");
    }

    #[tokio::test]
    async fn test_should_reject_invalid_range() {
        let storage = InMemoryStorage::new(TEST_THRESHOLD);
        storage
            .write_object("bucket", "key", "null", Bytes::from("abc"))
            .await
            .unwrap_or_else(|e| panic!("write failed: {e}"));

        // start > end
        let result = storage
            .read_object("bucket", "key", "null", Some((2, 1)))
            .await;
        assert!(matches!(result, Err(S3ServiceError::InvalidRange)));

        // end out of bounds
        let result = storage
            .read_object("bucket", "key", "null", Some((0, 100)))
            .await;
        assert!(matches!(result, Err(S3ServiceError::InvalidRange)));
    }

    // -----------------------------------------------------------------------
    // Copy object
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_should_copy_object() {
        let storage = InMemoryStorage::new(TEST_THRESHOLD);
        let data = small_data();
        storage
            .write_object("src-bucket", "src-key", "null", data.clone())
            .await
            .unwrap_or_else(|e| panic!("write failed: {e}"));

        let wr = storage
            .copy_object(
                "src-bucket",
                "src-key",
                "null",
                "dst-bucket",
                "dst-key",
                "v1",
            )
            .await
            .unwrap_or_else(|e| panic!("copy failed: {e}"));
        assert_eq!(wr.size, data.len() as u64);

        let dst_data = storage
            .read_object("dst-bucket", "dst-key", "v1", None)
            .await
            .unwrap_or_else(|e| panic!("read dst failed: {e}"));
        assert_eq!(dst_data, data);

        // Source still exists.
        let src_data = storage
            .read_object("src-bucket", "src-key", "null", None)
            .await
            .unwrap_or_else(|e| panic!("read src failed: {e}"));
        assert_eq!(src_data, data);
    }

    #[tokio::test]
    async fn test_should_return_error_on_copy_nonexistent_source() {
        let storage = InMemoryStorage::new(TEST_THRESHOLD);
        let result = storage
            .copy_object("bucket", "missing", "null", "dst", "key", "null")
            .await;
        assert!(matches!(result, Err(S3ServiceError::NoSuchKey { .. })));
    }

    // -----------------------------------------------------------------------
    // Delete object
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_should_delete_object() {
        let storage = InMemoryStorage::new(TEST_THRESHOLD);
        storage
            .write_object("bucket", "key", "null", small_data())
            .await
            .unwrap_or_else(|e| panic!("write failed: {e}"));

        storage.delete_object("bucket", "key", "null");

        let result = storage.read_object("bucket", "key", "null", None).await;
        assert!(matches!(result, Err(S3ServiceError::NoSuchKey { .. })));
    }

    #[tokio::test]
    async fn test_should_not_panic_on_delete_nonexistent() {
        let storage = InMemoryStorage::new(TEST_THRESHOLD);
        // Should be a no-op, not panic.
        storage.delete_object("bucket", "ghost", "null");
    }

    // -----------------------------------------------------------------------
    // Multipart write / read / complete / abort
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_should_write_and_read_part() {
        let storage = InMemoryStorage::new(TEST_THRESHOLD);
        let data = Bytes::from("part-data");
        let wr = storage
            .write_part("bucket", "upload-1", 1, data.clone())
            .await
            .unwrap_or_else(|e| panic!("write_part failed: {e}"));

        assert_eq!(wr.size, data.len() as u64);

        let read = storage
            .read_part("bucket", "upload-1", 1)
            .await
            .unwrap_or_else(|e| panic!("read_part failed: {e}"));
        assert_eq!(read, data);
    }

    #[tokio::test]
    async fn test_should_return_error_on_read_missing_part() {
        let storage = InMemoryStorage::new(TEST_THRESHOLD);
        let result = storage.read_part("bucket", "upload-1", 99).await;
        assert!(matches!(result, Err(S3ServiceError::InvalidPart)));
    }

    #[tokio::test]
    async fn test_should_complete_multipart_upload() {
        let storage = InMemoryStorage::new(TEST_THRESHOLD);

        let part1 = Bytes::from("hello ");
        let part2 = Bytes::from("world");

        storage
            .write_part("bucket", "upload-1", 1, part1.clone())
            .await
            .unwrap_or_else(|e| panic!("write part 1 failed: {e}"));
        storage
            .write_part("bucket", "upload-1", 2, part2.clone())
            .await
            .unwrap_or_else(|e| panic!("write part 2 failed: {e}"));

        let (wr, part_md5s) = storage
            .complete_multipart("bucket", "upload-1", "assembled-key", "null", &[1, 2])
            .await
            .unwrap_or_else(|e| panic!("complete_multipart failed: {e}"));

        // Size should be the sum of parts.
        assert_eq!(wr.size, (part1.len() + part2.len()) as u64);

        // ETag should be a composite (contains "-2").
        assert!(
            wr.etag.contains("-2"),
            "expected composite ETag, got {}",
            wr.etag
        );

        // Part MD5s should have 2 entries.
        assert_eq!(part_md5s.len(), 2);
        assert_eq!(part_md5s[0], checksums::compute_md5(&part1));
        assert_eq!(part_md5s[1], checksums::compute_md5(&part2));

        // The assembled object should be readable.
        let data = storage
            .read_object("bucket", "assembled-key", "null", None)
            .await
            .unwrap_or_else(|e| panic!("read assembled object failed: {e}"));
        assert_eq!(data.as_ref(), b"hello world");

        // Parts should have been cleaned up.
        let part_read = storage.read_part("bucket", "upload-1", 1).await;
        assert!(
            matches!(part_read, Err(S3ServiceError::InvalidPart)),
            "parts should be cleaned up after complete"
        );
    }

    #[tokio::test]
    async fn test_should_return_error_on_complete_with_missing_part() {
        let storage = InMemoryStorage::new(TEST_THRESHOLD);
        storage
            .write_part("bucket", "upload-1", 1, Bytes::from("data"))
            .await
            .unwrap_or_else(|e| panic!("write part failed: {e}"));

        // Part 2 was never uploaded.
        let result = storage
            .complete_multipart("bucket", "upload-1", "key", "null", &[1, 2])
            .await;
        assert!(matches!(result, Err(S3ServiceError::InvalidPart)));
    }

    #[tokio::test]
    async fn test_should_abort_multipart() {
        let storage = InMemoryStorage::new(TEST_THRESHOLD);
        storage
            .write_part("bucket", "upload-1", 1, Bytes::from("a"))
            .await
            .unwrap_or_else(|e| panic!("write part 1 failed: {e}"));
        storage
            .write_part("bucket", "upload-1", 2, Bytes::from("b"))
            .await
            .unwrap_or_else(|e| panic!("write part 2 failed: {e}"));

        // Also write a part for a different upload to verify isolation.
        storage
            .write_part("bucket", "upload-2", 1, Bytes::from("c"))
            .await
            .unwrap_or_else(|e| panic!("write part for upload-2 failed: {e}"));

        storage.abort_multipart("bucket", "upload-1");

        // upload-1 parts should be gone.
        assert!(matches!(
            storage.read_part("bucket", "upload-1", 1).await,
            Err(S3ServiceError::InvalidPart)
        ));
        assert!(matches!(
            storage.read_part("bucket", "upload-1", 2).await,
            Err(S3ServiceError::InvalidPart)
        ));

        // upload-2 part should still exist.
        let data = storage
            .read_part("bucket", "upload-2", 1)
            .await
            .unwrap_or_else(|e| panic!("read part for upload-2 failed: {e}"));
        assert_eq!(data.as_ref(), b"c");
    }

    // -----------------------------------------------------------------------
    // Delete bucket data
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_should_delete_bucket_data() {
        let storage = InMemoryStorage::new(TEST_THRESHOLD);
        storage
            .write_object("target", "obj1", "null", Bytes::from("a"))
            .await
            .unwrap_or_else(|e| panic!("write obj1 failed: {e}"));
        storage
            .write_object("target", "obj2", "null", Bytes::from("b"))
            .await
            .unwrap_or_else(|e| panic!("write obj2 failed: {e}"));
        storage
            .write_part("target", "upload-1", 1, Bytes::from("p"))
            .await
            .unwrap_or_else(|e| panic!("write part failed: {e}"));

        // Also write to a different bucket.
        storage
            .write_object("other", "obj3", "null", Bytes::from("c"))
            .await
            .unwrap_or_else(|e| panic!("write obj3 failed: {e}"));

        storage.delete_bucket_data("target");

        // Target bucket data should be gone.
        assert!(matches!(
            storage.read_object("target", "obj1", "null", None).await,
            Err(S3ServiceError::NoSuchKey { .. })
        ));
        assert!(matches!(
            storage.read_object("target", "obj2", "null", None).await,
            Err(S3ServiceError::NoSuchKey { .. })
        ));
        assert!(matches!(
            storage.read_part("target", "upload-1", 1).await,
            Err(S3ServiceError::InvalidPart)
        ));

        // Other bucket should be untouched.
        let data = storage
            .read_object("other", "obj3", "null", None)
            .await
            .unwrap_or_else(|e| panic!("read obj3 failed: {e}"));
        assert_eq!(data.as_ref(), b"c");
    }

    // -----------------------------------------------------------------------
    // Reset
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_should_reset_all_storage() {
        let storage = InMemoryStorage::new(TEST_THRESHOLD);
        storage
            .write_object("b1", "k1", "null", Bytes::from("data1"))
            .await
            .unwrap_or_else(|e| panic!("write1 failed: {e}"));
        storage
            .write_object("b2", "k2", "null", Bytes::from("data2"))
            .await
            .unwrap_or_else(|e| panic!("write2 failed: {e}"));
        storage
            .write_part("b1", "upload", 1, Bytes::from("part"))
            .await
            .unwrap_or_else(|e| panic!("write part failed: {e}"));

        storage.reset();

        assert!(matches!(
            storage.read_object("b1", "k1", "null", None).await,
            Err(S3ServiceError::NoSuchKey { .. })
        ));
        assert!(matches!(
            storage.read_object("b2", "k2", "null", None).await,
            Err(S3ServiceError::NoSuchKey { .. })
        ));
        assert!(matches!(
            storage.read_part("b1", "upload", 1).await,
            Err(S3ServiceError::InvalidPart)
        ));
    }

    // -----------------------------------------------------------------------
    // Default and Debug impls
    // -----------------------------------------------------------------------

    #[test]
    fn test_should_create_default_storage() {
        let storage = InMemoryStorage::default();
        assert_eq!(
            InMemoryStorage::default_max_memory_size(),
            DEFAULT_MAX_MEMORY_SIZE
        );
        let debug_str = format!("{storage:?}");
        assert!(debug_str.contains("InMemoryStorage"));
    }

    // -----------------------------------------------------------------------
    // On-disk cleanup on overwrite
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_should_clean_up_on_overwrite() {
        let storage = InMemoryStorage::new(TEST_THRESHOLD);
        let data1 = large_data();

        storage
            .write_object("bucket", "key", "null", data1)
            .await
            .unwrap_or_else(|e| panic!("write1 failed: {e}"));

        // Overwrite with new data. The old temp file should be cleaned up
        // via Drop when the DashMap entry is replaced.
        let data2 = Bytes::from("small");
        storage
            .write_object("bucket", "key", "null", data2.clone())
            .await
            .unwrap_or_else(|e| panic!("write2 failed: {e}"));

        let read = storage
            .read_object("bucket", "key", "null", None)
            .await
            .unwrap_or_else(|e| panic!("read failed: {e}"));
        assert_eq!(read, data2);
    }

    // -----------------------------------------------------------------------
    // Large part spillover
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_should_write_and_read_large_part_on_disk() {
        let storage = InMemoryStorage::new(TEST_THRESHOLD);
        let data = large_data();

        let wr = storage
            .write_part("bucket", "upload-big", 1, data.clone())
            .await
            .unwrap_or_else(|e| panic!("write_part failed: {e}"));
        assert_eq!(wr.size, data.len() as u64);

        let read = storage
            .read_part("bucket", "upload-big", 1)
            .await
            .unwrap_or_else(|e| panic!("read_part failed: {e}"));
        assert_eq!(read, data);
    }

    // -----------------------------------------------------------------------
    // Large on-disk object read completeness
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_should_read_all_bytes_from_large_on_disk_object() {
        // Use a 1 MiB object with a 64-byte threshold to force disk spillover.
        // This verifies that read_all returns the complete data rather than
        // only the bytes returned by a single read syscall.
        const SIZE: usize = 1024 * 1024;
        let storage = InMemoryStorage::new(TEST_THRESHOLD);
        let data = Bytes::from(vec![0x42_u8; SIZE]);

        storage
            .write_object("bucket", "big-obj", "null", data.clone())
            .await
            .unwrap_or_else(|e| panic!("write failed: {e}"));

        let read_data = storage
            .read_object("bucket", "big-obj", "null", None)
            .await
            .unwrap_or_else(|e| panic!("read failed: {e}"));

        assert_eq!(
            read_data.len(),
            SIZE,
            "expected {SIZE} bytes, got {}",
            read_data.len()
        );
        assert_eq!(read_data, data);
    }

    // -----------------------------------------------------------------------
    // Read nonexistent object
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_should_return_error_on_read_nonexistent_object() {
        let storage = InMemoryStorage::new(TEST_THRESHOLD);
        let result = storage.read_object("bucket", "ghost", "null", None).await;
        assert!(matches!(result, Err(S3ServiceError::NoSuchKey { .. })));
    }
}
