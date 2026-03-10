//! Checksum computation for S3 objects.
//!
//! Provides functions to compute MD5, SHA-1, SHA-256, CRC32, and CRC32C
//! checksums used by S3 for ETags and the additional checksum algorithms
//! supported by the `x-amz-checksum-*` headers.
//!
//! # Streaming Hashing
//!
//! For large objects that cannot be buffered entirely in memory, use
//! [`StreamingHasher`] to incrementally feed data and obtain the final
//! results via [`HasherResult`].

use std::fmt;
use std::str::FromStr;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use digest::Digest;

// ---------------------------------------------------------------------------
// ChecksumAlgorithm
// ---------------------------------------------------------------------------

/// S3-supported checksum algorithms (excluding MD5 which is always computed
/// for the ETag).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChecksumAlgorithm {
    /// CRC-32 (IEEE 802.3).
    Crc32,
    /// CRC-32C (Castagnoli).
    Crc32c,
    /// CRC-64/NVME (NVMe polynomial, hardware-accelerated).
    Crc64Nvme,
    /// SHA-1.
    Sha1,
    /// SHA-256.
    Sha256,
}

impl ChecksumAlgorithm {
    /// Return the canonical string representation used in S3 headers.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Crc32 => "CRC32",
            Self::Crc32c => "CRC32C",
            Self::Crc64Nvme => "CRC64NVME",
            Self::Sha1 => "SHA1",
            Self::Sha256 => "SHA256",
        }
    }
}

impl fmt::Display for ChecksumAlgorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when parsing a [`ChecksumAlgorithm`] from a string fails.
#[derive(Debug, Clone, thiserror::Error)]
#[error("unknown checksum algorithm: {0}")]
pub struct ParseChecksumAlgorithmError(String);

impl FromStr for ChecksumAlgorithm {
    type Err = ParseChecksumAlgorithmError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_uppercase().as_str() {
            "CRC32" => Ok(Self::Crc32),
            "CRC32C" => Ok(Self::Crc32c),
            "CRC64NVME" => Ok(Self::Crc64Nvme),
            "SHA1" => Ok(Self::Sha1),
            "SHA256" => Ok(Self::Sha256),
            _ => Err(ParseChecksumAlgorithmError(s.to_owned())),
        }
    }
}

// ---------------------------------------------------------------------------
// ChecksumValue
// ---------------------------------------------------------------------------

/// A base64-encoded checksum value paired with its algorithm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecksumValue {
    /// The algorithm used to compute this checksum.
    pub algorithm: ChecksumAlgorithm,
    /// The base64-encoded checksum.
    pub value: String,
}

// ---------------------------------------------------------------------------
// Standalone checksum functions
// ---------------------------------------------------------------------------

/// Compute the hex-encoded MD5 digest of `data`.
///
/// This is the raw hex digest used internally. For an S3-formatted ETag (quoted),
/// use [`compute_etag`].
///
/// # Examples
///
/// ```
/// use ruststack_s3_core::checksums::compute_md5;
///
/// let digest = compute_md5(b"hello");
/// assert_eq!(digest, "5d41402abc4b2a76b9719d911017c592");
/// ```
#[must_use]
pub fn compute_md5(data: &[u8]) -> String {
    let hash = md5::Md5::digest(data);
    hex::encode(hash)
}

/// Compute the quoted hex-encoded MD5 digest of `data`, suitable for use as
/// an S3 ETag.
///
/// The returned string is surrounded by double quotes, e.g.
/// `"5d41402abc4b2a76b9719d911017c592"`.
///
/// # Examples
///
/// ```
/// use ruststack_s3_core::checksums::compute_etag;
///
/// let etag = compute_etag(b"");
/// assert_eq!(etag, "\"d41d8cd98f00b204e9800998ecf8427e\"");
/// ```
#[must_use]
pub fn compute_etag(data: &[u8]) -> String {
    let md5_hex = compute_md5(data);
    format!("\"{md5_hex}\"")
}

/// Compute a composite ETag for a multipart upload.
///
/// The composite ETag is the MD5 of the concatenated binary MD5 digests of
/// each part, formatted as `"<hex>-<part_count>"`.
///
/// Each entry in `part_md5_hexes` should be the *unquoted* hex MD5 of a part.
///
/// # Examples
///
/// ```
/// use ruststack_s3_core::checksums::compute_multipart_etag;
///
/// let part_hexes = ["5d41402abc4b2a76b9719d911017c592"];
/// let etag = compute_multipart_etag(&part_hexes, 1);
/// assert!(etag.ends_with("-1\""));
/// ```
#[must_use]
pub fn compute_multipart_etag(part_md5_hexes: &[impl AsRef<str>], part_count: usize) -> String {
    let mut combined = Vec::with_capacity(part_md5_hexes.len() * 16);
    for hex_str in part_md5_hexes {
        let hex_str = hex_str.as_ref().trim_matches('"');
        if let Ok(bytes) = hex::decode(hex_str) {
            combined.extend_from_slice(&bytes);
        }
    }
    let final_md5 = hex::encode(md5::Md5::digest(&combined));
    format!("\"{final_md5}-{part_count}\"")
}

/// Compute a base64-encoded checksum for the given algorithm.
///
/// # Examples
///
/// ```
/// use ruststack_s3_core::checksums::{ChecksumAlgorithm, compute_checksum};
///
/// let b64 = compute_checksum(ChecksumAlgorithm::Crc32, b"hello");
/// assert!(!b64.is_empty());
/// ```
#[must_use]
pub fn compute_checksum(algorithm: ChecksumAlgorithm, data: &[u8]) -> String {
    match algorithm {
        ChecksumAlgorithm::Crc32 => {
            let mut hasher = crc32fast::Hasher::new();
            hasher.update(data);
            let value = hasher.finalize();
            BASE64_STANDARD.encode(value.to_be_bytes())
        }
        ChecksumAlgorithm::Crc32c => {
            let value = crc32c::crc32c(data);
            BASE64_STANDARD.encode(value.to_be_bytes())
        }
        ChecksumAlgorithm::Crc64Nvme => {
            let mut hasher = crc64fast_nvme::Digest::new();
            hasher.write(data);
            let value = hasher.sum64();
            BASE64_STANDARD.encode(value.to_be_bytes())
        }
        ChecksumAlgorithm::Sha1 => {
            let hash = sha1::Sha1::digest(data);
            BASE64_STANDARD.encode(hash)
        }
        ChecksumAlgorithm::Sha256 => {
            let hash = sha2::Sha256::digest(data);
            BASE64_STANDARD.encode(hash)
        }
    }
}

/// Compute a composite checksum for a multipart upload.
///
/// The composite checksum is computed by concatenating the raw (decoded)
/// checksums of each part and then computing the checksum of that
/// concatenation. The result is base64-encoded with a `-<part_count>` suffix.
///
/// Each entry in `part_checksums_b64` should be the base64-encoded checksum
/// of a single part (without a part-count suffix).
///
/// # Examples
///
/// ```
/// use ruststack_s3_core::checksums::{ChecksumAlgorithm, compute_checksum, compute_composite_checksum};
///
/// let part1 = compute_checksum(ChecksumAlgorithm::Crc32, b"hello");
/// let composite = compute_composite_checksum(ChecksumAlgorithm::Crc32, &[part1]);
/// assert!(composite.contains("-1"));
/// ```
#[must_use]
pub fn compute_composite_checksum(
    algorithm: ChecksumAlgorithm,
    part_checksums_b64: &[impl AsRef<str>],
) -> String {
    let mut combined = Vec::new();
    for b64 in part_checksums_b64 {
        if let Ok(bytes) = BASE64_STANDARD.decode(b64.as_ref()) {
            combined.extend_from_slice(&bytes);
        }
    }
    let checksum_b64 = compute_checksum(algorithm, &combined);
    format!("{checksum_b64}-{}", part_checksums_b64.len())
}

// ---------------------------------------------------------------------------
// StreamingHasher
// ---------------------------------------------------------------------------

/// Result produced by [`StreamingHasher::finish`].
#[derive(Debug, Clone)]
pub struct HasherResult {
    /// Hex-encoded MD5 digest.
    pub md5_hex: String,
    /// Per-algorithm base64-encoded checksums (only for algorithms that were
    /// requested when the hasher was created).
    pub checksums: Vec<ChecksumValue>,
}

/// Incremental hasher that computes MD5 and optionally additional checksums
/// over a stream of data chunks.
///
/// # Examples
///
/// ```
/// use ruststack_s3_core::checksums::{ChecksumAlgorithm, StreamingHasher};
///
/// let mut hasher = StreamingHasher::new(&[ChecksumAlgorithm::Sha256]);
/// hasher.update(b"hello ");
/// hasher.update(b"world");
/// let result = hasher.finish();
/// assert!(!result.md5_hex.is_empty());
/// assert_eq!(result.checksums.len(), 1);
/// ```
pub struct StreamingHasher {
    md5: md5::Md5,
    sha1: Option<sha1::Sha1>,
    sha256: Option<sha2::Sha256>,
    crc32: Option<crc32fast::Hasher>,
    crc32c: Option<u32>,
    crc64nvme: Option<crc64fast_nvme::Digest>,
    algorithms: Vec<ChecksumAlgorithm>,
}

impl fmt::Debug for StreamingHasher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamingHasher")
            .field("algorithms", &self.algorithms)
            .finish_non_exhaustive()
    }
}

impl StreamingHasher {
    /// Create a new streaming hasher.
    ///
    /// MD5 is always computed. Provide additional algorithms in `algorithms`
    /// to compute extra checksums.
    #[must_use]
    pub fn new(algorithms: &[ChecksumAlgorithm]) -> Self {
        let mut sha1 = None;
        let mut sha256 = None;
        let mut crc32 = None;
        let mut crc32c = None;
        let mut crc64nvme = None;

        for &algo in algorithms {
            match algo {
                ChecksumAlgorithm::Sha1 => {
                    sha1 = Some(<sha1::Sha1 as Digest>::new());
                }
                ChecksumAlgorithm::Sha256 => {
                    sha256 = Some(<sha2::Sha256 as Digest>::new());
                }
                ChecksumAlgorithm::Crc32 => {
                    crc32 = Some(crc32fast::Hasher::new());
                }
                ChecksumAlgorithm::Crc32c => {
                    crc32c = Some(0);
                }
                ChecksumAlgorithm::Crc64Nvme => {
                    crc64nvme = Some(crc64fast_nvme::Digest::new());
                }
            }
        }

        Self {
            md5: <md5::Md5 as Digest>::new(),
            sha1,
            sha256,
            crc32,
            crc32c,
            crc64nvme,
            algorithms: algorithms.to_vec(),
        }
    }

    /// Feed more data into the hasher.
    pub fn update(&mut self, data: &[u8]) {
        Digest::update(&mut self.md5, data);

        if let Some(ref mut h) = self.sha1 {
            Digest::update(h, data);
        }
        if let Some(ref mut h) = self.sha256 {
            Digest::update(h, data);
        }
        if let Some(ref mut h) = self.crc32 {
            h.update(data);
        }
        if let Some(ref mut val) = self.crc32c {
            *val = crc32c::crc32c_append(*val, data);
        }
        if let Some(ref mut h) = self.crc64nvme {
            h.write(data);
        }
    }

    /// Finalize the hasher and return the results.
    ///
    /// This consumes the hasher.
    #[must_use]
    pub fn finish(self) -> HasherResult {
        let md5_hex = hex::encode(Digest::finalize(self.md5));

        let mut checksums = Vec::with_capacity(self.algorithms.len());
        for algo in &self.algorithms {
            let value = match algo {
                ChecksumAlgorithm::Sha1 => {
                    let hash = Digest::finalize(self.sha1.clone().unwrap_or_default());
                    BASE64_STANDARD.encode(hash)
                }
                ChecksumAlgorithm::Sha256 => {
                    let hash = Digest::finalize(self.sha256.clone().unwrap_or_default());
                    BASE64_STANDARD.encode(hash)
                }
                ChecksumAlgorithm::Crc32 => {
                    let val = self
                        .crc32
                        .as_ref()
                        .map_or(0, crc32fast::Hasher::clone_finalize);
                    BASE64_STANDARD.encode(val.to_be_bytes())
                }
                ChecksumAlgorithm::Crc32c => {
                    let val = self.crc32c.unwrap_or(0);
                    BASE64_STANDARD.encode(val.to_be_bytes())
                }
                ChecksumAlgorithm::Crc64Nvme => {
                    let val = self
                        .crc64nvme
                        .as_ref()
                        .map_or(0, crc64fast_nvme::Digest::sum64);
                    BASE64_STANDARD.encode(val.to_be_bytes())
                }
            };
            checksums.push(ChecksumValue {
                algorithm: *algo,
                value,
            });
        }

        HasherResult { md5_hex, checksums }
    }
}

// ---------------------------------------------------------------------------
// crc32fast Hasher clone_finalize helper
// ---------------------------------------------------------------------------

/// Extension trait to finalize a cloned `crc32fast::Hasher` without
/// consuming the original (used by `StreamingHasher::finish`).
trait CloneFinalize {
    /// Clone and finalize, returning the CRC-32 value.
    fn clone_finalize(&self) -> u32;
}

impl CloneFinalize for crc32fast::Hasher {
    fn clone_finalize(&self) -> u32 {
        self.clone().finalize()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // ChecksumAlgorithm
    // -----------------------------------------------------------------------

    #[test]
    fn test_should_display_checksum_algorithm() {
        assert_eq!(ChecksumAlgorithm::Crc32.to_string(), "CRC32");
        assert_eq!(ChecksumAlgorithm::Crc32c.to_string(), "CRC32C");
        assert_eq!(ChecksumAlgorithm::Crc64Nvme.to_string(), "CRC64NVME");
        assert_eq!(ChecksumAlgorithm::Sha1.to_string(), "SHA1");
        assert_eq!(ChecksumAlgorithm::Sha256.to_string(), "SHA256");
    }

    #[test]
    fn test_should_parse_checksum_algorithm() {
        assert_eq!(
            "CRC32".parse::<ChecksumAlgorithm>().ok(),
            Some(ChecksumAlgorithm::Crc32)
        );
        assert_eq!(
            "crc32c".parse::<ChecksumAlgorithm>().ok(),
            Some(ChecksumAlgorithm::Crc32c)
        );
        assert_eq!(
            "CRC64NVME".parse::<ChecksumAlgorithm>().ok(),
            Some(ChecksumAlgorithm::Crc64Nvme)
        );
        assert_eq!(
            "sha1".parse::<ChecksumAlgorithm>().ok(),
            Some(ChecksumAlgorithm::Sha1)
        );
        assert_eq!(
            "SHA256".parse::<ChecksumAlgorithm>().ok(),
            Some(ChecksumAlgorithm::Sha256)
        );
        assert!("unknown".parse::<ChecksumAlgorithm>().is_err());
    }

    // -----------------------------------------------------------------------
    // MD5 / ETag
    // -----------------------------------------------------------------------

    #[test]
    fn test_should_compute_md5_empty() {
        assert_eq!(compute_md5(b""), "d41d8cd98f00b204e9800998ecf8427e");
    }

    #[test]
    fn test_should_compute_md5_hello() {
        assert_eq!(compute_md5(b"hello"), "5d41402abc4b2a76b9719d911017c592");
    }

    #[test]
    fn test_should_compute_etag_empty() {
        assert_eq!(compute_etag(b""), "\"d41d8cd98f00b204e9800998ecf8427e\"");
    }

    #[test]
    fn test_should_compute_etag_with_data() {
        let etag = compute_etag(b"hello");
        assert!(etag.starts_with('"'));
        assert!(etag.ends_with('"'));
        assert_eq!(etag.len(), 34); // 32 hex + 2 quotes
    }

    // -----------------------------------------------------------------------
    // Multipart ETag
    // -----------------------------------------------------------------------

    #[test]
    fn test_should_compute_multipart_etag() {
        let part1_hex = compute_md5(b"hello");
        let part2_hex = compute_md5(b"world");
        let etag = compute_multipart_etag(&[part1_hex, part2_hex], 2);
        assert!(etag.starts_with('"'));
        assert!(etag.ends_with("-2\""));
    }

    #[test]
    fn test_should_compute_multipart_etag_single_part() {
        let part_hex = compute_md5(b"data");
        let etag = compute_multipart_etag(&[part_hex], 1);
        assert!(etag.ends_with("-1\""));
    }

    // -----------------------------------------------------------------------
    // Algorithm-specific checksums
    // -----------------------------------------------------------------------

    #[test]
    fn test_should_compute_crc32_checksum() {
        let b64 = compute_checksum(ChecksumAlgorithm::Crc32, b"hello");
        assert!(!b64.is_empty());
        // Verify round-trip decode
        let decoded = BASE64_STANDARD.decode(&b64);
        assert!(decoded.is_ok());
        assert_eq!(decoded.expect("test decode").len(), 4);
    }

    #[test]
    fn test_should_compute_crc32c_checksum() {
        let b64 = compute_checksum(ChecksumAlgorithm::Crc32c, b"hello");
        assert!(!b64.is_empty());
    }

    #[test]
    fn test_should_compute_crc64nvme_checksum() {
        let b64 = compute_checksum(ChecksumAlgorithm::Crc64Nvme, b"hello");
        assert!(!b64.is_empty());
        let decoded = BASE64_STANDARD.decode(&b64);
        assert!(decoded.is_ok());
        assert_eq!(decoded.expect("test decode").len(), 8);
    }

    #[test]
    fn test_should_compute_sha1_checksum() {
        let b64 = compute_checksum(ChecksumAlgorithm::Sha1, b"hello");
        let decoded = BASE64_STANDARD.decode(&b64);
        assert!(decoded.is_ok());
        assert_eq!(decoded.expect("test decode").len(), 20);
    }

    #[test]
    fn test_should_compute_sha256_checksum() {
        let b64 = compute_checksum(ChecksumAlgorithm::Sha256, b"hello");
        let decoded = BASE64_STANDARD.decode(&b64);
        assert!(decoded.is_ok());
        assert_eq!(decoded.expect("test decode").len(), 32);
    }

    // -----------------------------------------------------------------------
    // Composite checksums
    // -----------------------------------------------------------------------

    #[test]
    fn test_should_compute_composite_checksum() {
        let p1 = compute_checksum(ChecksumAlgorithm::Sha256, b"part1");
        let p2 = compute_checksum(ChecksumAlgorithm::Sha256, b"part2");
        let composite = compute_composite_checksum(ChecksumAlgorithm::Sha256, &[p1, p2]);
        assert!(composite.contains("-2"));
    }

    // -----------------------------------------------------------------------
    // StreamingHasher
    // -----------------------------------------------------------------------

    #[test]
    fn test_should_stream_md5_only() {
        let mut hasher = StreamingHasher::new(&[]);
        hasher.update(b"hello");
        let result = hasher.finish();
        assert_eq!(result.md5_hex, "5d41402abc4b2a76b9719d911017c592");
        assert!(result.checksums.is_empty());
    }

    #[test]
    fn test_should_stream_with_sha256() {
        let mut hasher = StreamingHasher::new(&[ChecksumAlgorithm::Sha256]);
        hasher.update(b"hello ");
        hasher.update(b"world");
        let result = hasher.finish();

        // MD5 of "hello world"
        assert_eq!(result.md5_hex, compute_md5(b"hello world"));

        // SHA-256 of "hello world"
        assert_eq!(result.checksums.len(), 1);
        assert_eq!(result.checksums[0].algorithm, ChecksumAlgorithm::Sha256);
        assert_eq!(
            result.checksums[0].value,
            compute_checksum(ChecksumAlgorithm::Sha256, b"hello world"),
        );
    }

    #[test]
    fn test_should_stream_multiple_algorithms() {
        let algos = [
            ChecksumAlgorithm::Crc32,
            ChecksumAlgorithm::Crc32c,
            ChecksumAlgorithm::Crc64Nvme,
            ChecksumAlgorithm::Sha1,
            ChecksumAlgorithm::Sha256,
        ];
        let mut hasher = StreamingHasher::new(&algos);
        hasher.update(b"test data");
        let result = hasher.finish();

        assert_eq!(result.checksums.len(), 5);
        for (i, algo) in algos.iter().enumerate() {
            assert_eq!(result.checksums[i].algorithm, *algo);
            assert_eq!(
                result.checksums[i].value,
                compute_checksum(*algo, b"test data"),
            );
        }
    }

    #[test]
    fn test_should_match_single_shot_and_streaming_results() {
        let data = b"The quick brown fox jumps over the lazy dog";

        let single_md5 = compute_md5(data);
        let single_sha256 = compute_checksum(ChecksumAlgorithm::Sha256, data);

        let mut hasher = StreamingHasher::new(&[ChecksumAlgorithm::Sha256]);
        // Feed in chunks
        hasher.update(&data[..10]);
        hasher.update(&data[10..30]);
        hasher.update(&data[30..]);
        let result = hasher.finish();

        assert_eq!(result.md5_hex, single_md5);
        assert_eq!(result.checksums[0].value, single_sha256);
    }
}
