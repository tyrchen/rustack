//! aarch64 kernel image loader.
//!
//! Detects compression by magic bytes, decompresses if needed, parses the aarch64 boot
//! header to recover `text_offset` and `image_size`, and returns a [`LoadedKernel`] ready
//! to be copied into guest RAM.
//!
//! Loading happens at config time (when the operator posts `/boot-source`), not at
//! `InstanceStart`, so a malformed kernel image returns a 4xx synchronously rather than
//! detonating in the boot orchestrator.
//!
//! ## Trust boundary
//!
//! Per [70-security.md § 4](../../../specs/70-security.md#4-input-validation), every
//! external string crossing into squib goes through a fallible-constructor newtype before
//! reaching downstream code. For host-filesystem paths the canonical newtype is
//! `squib_api::schemas::common::SafePath`: byte-length cap of 1024, NUL-byte rejection,
//! charset / parent-traversal handled at the API layer.
//!
//! The loader sits below the API layer and accepts a `&Path` for `load_from_path`. To
//! make the trust boundary load-bearing rather than aspirational, this crate also runs a
//! **defence-in-depth** check on the path before opening it (`enforce_path_bounds`):
//! length cap of 1024 bytes, NUL-byte rejection. A buggy caller that bypasses the API
//! layer surfaces a [`LoaderError::PathRejected`] instead of letting a megabyte path or
//! an interior-NUL path reach `std::fs::metadata`.
//!
//! See [13-arch-and-boot.md § 7](../../../specs/13-arch-and-boot.md#7-kernel-loader) and
//! `arm64/booting.rst` for the boot-header contract.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
// Kernel images are loaded once at config-time before vCPUs run; using sync std::fs is
// fine here and keeps the loader trivially testable. The clippy disallowed-method ban on
// `std::fs::*` exists for *runtime* code that should yield to the tokio scheduler — this
// crate's I/O is the synchronous moral equivalent of `getopts`.
#![allow(clippy::disallowed_methods)]
// Hardware identifiers (`text_offset`, `image_size`, `ARM\x64`, etc.) appear in doc text
// in their canonical underscore form; backticking each one bloats the documentation.
#![allow(clippy::doc_markdown)]
// `1 * 1024 * 1024` is a documented constant, not an identity simplification.
#![allow(clippy::identity_op)]

use std::{
    io::{Cursor, Read},
    path::Path,
};

use squib_arch::layout::{DRAM_BASE, DRAM_MAX_END, KERNEL_LOAD_OFFSET};
use thiserror::Error;

/// `MZ` — DOS / PE magic.
pub const MAGIC_PE: [u8; 2] = *b"MZ";
/// `1f 8b` — gzip magic.
pub const MAGIC_GZ: [u8; 2] = [0x1F, 0x8B];
/// `28 b5 2f fd` — zstd magic.
pub const MAGIC_ZST: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];
/// `ARM\x64` — aarch64 Image magic at offset 0x38.
pub const MAGIC_AARCH64_IMAGE: [u8; 4] = *b"ARM\x64";

/// Offset of the aarch64 magic field within the kernel image.
pub const AARCH64_MAGIC_OFFSET: usize = 0x38;

/// Minimum size for any kernel image we accept (the boot header alone is 64 bytes).
pub const MIN_IMAGE_SIZE: usize = 64;

/// Hard upper bound on the *compressed* input size, in bytes.
///
/// Accepting unbounded compressed input is a decompression-bomb vector. 1 GiB is
/// generously larger than any realistic Linux kernel image (typical aarch64 vmlinux is
/// ≤ 32 MiB). Operators with a legitimate need can raise this via the `MaxSizes`
/// builder; the default bounds make the decompression bomb a non-issue.
pub const DEFAULT_MAX_COMPRESSED: u64 = 1024 * 1024 * 1024;

/// Hard upper bound on the *decompressed* image size, in bytes.
pub const DEFAULT_MAX_DECOMPRESSED: u64 = 4 * 1024 * 1024 * 1024;

/// Errors that can surface while loading and parsing a kernel image.
#[derive(Debug, Error)]
pub enum LoaderError {
    /// Reading the image file failed.
    #[error("kernel image I/O error")]
    Io(#[from] std::io::Error),

    /// Image is too small to contain an aarch64 boot header.
    #[error("kernel image is too small ({size} bytes; need ≥ {MIN_IMAGE_SIZE})")]
    ImageTooSmall {
        /// Actual image size in bytes.
        size: usize,
    },

    /// Image format could not be identified by magic bytes.
    #[error("unknown kernel image format (no recognised magic)")]
    UnknownFormat,

    /// aarch64 boot magic missing at offset 0x38 of the (possibly decompressed) image.
    #[error("missing aarch64 'ARM\\x64' magic at offset {AARCH64_MAGIC_OFFSET:#x}")]
    MissingAarch64Magic,

    /// Compressed input exceeds the configured cap.
    #[error("compressed kernel image is too large ({size} bytes; cap {cap})")]
    CompressedTooLarge {
        /// Actual size of the compressed input.
        size: u64,
        /// Cap that was exceeded.
        cap: u64,
    },

    /// Decompressed kernel exceeds the configured cap (decompression-bomb guard).
    #[error("decompressed kernel image is too large ({size} bytes; cap {cap})")]
    DecompressedTooLarge {
        /// Number of bytes read before the cap fired.
        size: u64,
        /// Cap that was exceeded.
        cap: u64,
    },

    /// Kernel image would not fit in the guest's RAM region.
    #[error(
        "kernel image (size {image_size}) would not fit in guest RAM (start {load_addr:#x}, \
         ram_end {ram_end:#x})"
    )]
    DoesNotFitInRam {
        /// Computed load address.
        load_addr: u64,
        /// `image_size` from the boot header.
        image_size: u64,
        /// Configured guest RAM end (exclusive).
        ram_end: u64,
    },

    /// `text_offset` from the boot header overflows when added to `DRAM_BASE`.
    #[error("invalid text_offset {text_offset:#x}: would overflow DRAM_BASE")]
    InvalidTextOffset {
        /// Offset that overflowed.
        text_offset: u64,
    },

    /// Path failed defence-in-depth boundary checks before any I/O ran.
    #[error("kernel image path rejected at boundary: {0}")]
    PathRejected(&'static str),
}

/// Length cap on a host-filesystem path the loader will open.
///
/// Mirrors `squib_api::schemas::common::SafePath::PATH_MAX` so a value that passed the
/// API-layer boundary always passes the loader-layer boundary and a value that bypassed
/// the API layer (a buggy direct caller) still gets rejected with [`LoaderError::PathRejected`].
pub const PATH_MAX: usize = 1024;

/// Defence-in-depth path validation, run before any `std::fs` call.
///
/// The canonical boundary is the API layer's `SafePath` newtype — but a buggy direct
/// caller of [`load_from_path`] could in theory pass an unvalidated path. This function
/// runs the same byte-cap and NUL-byte rejection that `SafePath::new` runs so the loader
/// is correct in isolation. Returns a `'static` reason string suitable for embedding in
/// [`LoaderError::PathRejected`] without leaking the path itself into error text.
fn enforce_path_bounds(path: &Path) -> Result<(), LoaderError> {
    let s = path
        .to_str()
        .ok_or(LoaderError::PathRejected("path is not valid UTF-8"))?;
    if s.is_empty() {
        return Err(LoaderError::PathRejected("path must not be empty"));
    }
    if s.len() > PATH_MAX {
        return Err(LoaderError::PathRejected("path exceeds PATH_MAX bytes"));
    }
    if s.as_bytes().contains(&0) {
        return Err(LoaderError::PathRejected("path contains a NUL byte"));
    }
    Ok(())
}

/// Recognized kernel image format.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum ImageFormat {
    /// Raw aarch64 `Image` (no PE wrapper, no compression).
    Aarch64Image,
    /// Linux EFI PE-wrapped image (`MZ` magic at offset 0; aarch64 header present at 0x38).
    Pe,
    /// gzip-compressed payload.
    Gzip,
    /// zstd-compressed payload.
    Zstd,
}

/// Caps for the decompression / loading paths.
#[derive(Debug, Clone, Copy)]
pub struct MaxSizes {
    /// Maximum compressed size accepted on disk.
    pub compressed: u64,
    /// Maximum decompressed size produced by `flate2` / `zstd`.
    pub decompressed: u64,
}

impl Default for MaxSizes {
    fn default() -> Self {
        Self {
            compressed: DEFAULT_MAX_COMPRESSED,
            decompressed: DEFAULT_MAX_DECOMPRESSED,
        }
    }
}

/// A loaded, decompressed, and validated aarch64 kernel image.
#[derive(Debug, Clone)]
pub struct LoadedKernel {
    /// Decompressed image bytes (i.e. ready to write into guest RAM).
    pub bytes: Vec<u8>,
    /// Format detected at the input.
    pub format: ImageFormat,
    /// `text_offset` from the boot header — load offset from `DRAM_BASE`.
    pub text_offset: u64,
    /// `image_size` from the boot header (`0` means "use the bytes length").
    pub image_size: u64,
}

impl LoadedKernel {
    /// Compute the guest-physical load address for this kernel.
    ///
    /// Per [13-arch-and-boot.md § 7](../../../specs/13-arch-and-boot.md#7-kernel-loader),
    /// the Image goes at `DRAM_BASE + text_offset`. Firecracker reserves the first 2 MiB
    /// for system metadata (`KERNEL_LOAD_OFFSET`); we honour that floor so a kernel with a
    /// tiny `text_offset` (some 5.x kernels still ship `text_offset = 0x80000`) does not
    /// land underneath the reserved system band.
    ///
    /// # Errors
    /// [`LoaderError::InvalidTextOffset`] if `DRAM_BASE + text_offset` would overflow.
    pub fn load_address(&self) -> Result<u64, LoaderError> {
        let raw =
            DRAM_BASE
                .checked_add(self.text_offset)
                .ok_or(LoaderError::InvalidTextOffset {
                    text_offset: self.text_offset,
                })?;
        Ok(raw.max(DRAM_BASE + KERNEL_LOAD_OFFSET))
    }

    /// The number of bytes that must fit in guest RAM starting at [`Self::load_address`].
    /// Equals `image_size` from the header when non-zero, otherwise the raw payload size.
    #[must_use]
    pub fn image_bytes(&self) -> u64 {
        if self.image_size == 0 {
            self.bytes.len() as u64
        } else {
            self.image_size
        }
    }
    // Note: `bytes.len() as u64` always fits because Vec is bounded by isize::MAX.

    /// Validate that the kernel fits in a guest RAM region of length `ram_size_bytes`.
    ///
    /// # Errors
    /// [`LoaderError::DoesNotFitInRam`] when `load_address + image_bytes() > ram_end`.
    pub fn check_fits_in_ram(&self, ram_size_bytes: u64) -> Result<u64, LoaderError> {
        let load_addr = self.load_address()?;
        let ram_end = DRAM_BASE
            .checked_add(ram_size_bytes)
            .filter(|end| *end <= DRAM_MAX_END)
            .ok_or(LoaderError::DoesNotFitInRam {
                load_addr,
                image_size: self.image_bytes(),
                ram_end: DRAM_MAX_END,
            })?;
        let needed = self.image_bytes();
        let end = load_addr
            .checked_add(needed)
            .ok_or(LoaderError::DoesNotFitInRam {
                load_addr,
                image_size: needed,
                ram_end,
            })?;
        if end > ram_end {
            return Err(LoaderError::DoesNotFitInRam {
                load_addr,
                image_size: needed,
                ram_end,
            });
        }
        Ok(load_addr)
    }
}

/// Detect the image format from the input's first bytes.
#[must_use]
pub fn detect_format(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.len() >= 4 && bytes[..4] == MAGIC_ZST {
        return Some(ImageFormat::Zstd);
    }
    if bytes.len() >= 2 {
        if bytes[..2] == MAGIC_GZ {
            return Some(ImageFormat::Gzip);
        }
        if bytes[..2] == MAGIC_PE {
            return Some(ImageFormat::Pe);
        }
    }
    if has_aarch64_magic(bytes) {
        return Some(ImageFormat::Aarch64Image);
    }
    None
}

fn has_aarch64_magic(bytes: &[u8]) -> bool {
    bytes
        .get(AARCH64_MAGIC_OFFSET..AARCH64_MAGIC_OFFSET + 4)
        .is_some_and(|window| window == MAGIC_AARCH64_IMAGE)
}

/// Load and parse an aarch64 kernel image from disk.
///
/// # Errors
/// Surfaces [`LoaderError`] for I/O, format, or size violations.
pub fn load_from_path(path: &Path) -> Result<LoadedKernel, LoaderError> {
    load_from_path_with_caps(path, MaxSizes::default())
}

/// [`load_from_path`] with explicit decompression caps.
///
/// # Errors
/// Surfaces [`LoaderError`] on any of the documented failure modes, plus
/// [`LoaderError::PathRejected`] when the path fails the loader's defence-in-depth
/// boundary check (see crate-level "Trust boundary" docs).
pub fn load_from_path_with_caps(path: &Path, caps: MaxSizes) -> Result<LoadedKernel, LoaderError> {
    enforce_path_bounds(path)?;
    let metadata = std::fs::metadata(path)?;
    if metadata.len() > caps.compressed {
        return Err(LoaderError::CompressedTooLarge {
            size: metadata.len(),
            cap: caps.compressed,
        });
    }
    let bytes = std::fs::read(path)?;
    load_from_bytes_with_caps(&bytes, caps)
}

/// Load and parse an aarch64 kernel image from an in-memory byte slice.
///
/// # Errors
/// Surfaces [`LoaderError`] on any of the documented failure modes.
pub fn load_from_bytes(bytes: &[u8]) -> Result<LoadedKernel, LoaderError> {
    load_from_bytes_with_caps(bytes, MaxSizes::default())
}

/// [`load_from_bytes`] with explicit decompression caps.
///
/// # Errors
/// Surfaces [`LoaderError`] on any of the documented failure modes.
pub fn load_from_bytes_with_caps(
    bytes: &[u8],
    caps: MaxSizes,
) -> Result<LoadedKernel, LoaderError> {
    let bytes_len = bytes.len() as u64;
    if bytes_len > caps.compressed {
        return Err(LoaderError::CompressedTooLarge {
            size: bytes_len,
            cap: caps.compressed,
        });
    }
    let format = detect_format(bytes).ok_or(LoaderError::UnknownFormat)?;
    let payload = match format {
        ImageFormat::Aarch64Image | ImageFormat::Pe => bytes.to_vec(),
        ImageFormat::Gzip => decompress_gzip(bytes, caps.decompressed)?,
        ImageFormat::Zstd => decompress_zstd(bytes, caps.decompressed)?,
    };

    if payload.len() < MIN_IMAGE_SIZE {
        return Err(LoaderError::ImageTooSmall {
            size: payload.len(),
        });
    }
    if !has_aarch64_magic(&payload) {
        return Err(LoaderError::MissingAarch64Magic);
    }

    let text_offset = read_le_u64_at(&payload, 8)?;
    let image_size = read_le_u64_at(&payload, 16)?;

    Ok(LoadedKernel {
        bytes: payload,
        format,
        text_offset,
        image_size,
    })
}

fn decompress_gzip(bytes: &[u8], cap: u64) -> Result<Vec<u8>, LoaderError> {
    let cursor = Cursor::new(bytes);
    let mut decoder = flate2::read::GzDecoder::new(cursor);
    bounded_read_to_end(&mut decoder, cap)
}

fn decompress_zstd(bytes: &[u8], cap: u64) -> Result<Vec<u8>, LoaderError> {
    let mut decoder = zstd::stream::read::Decoder::new(Cursor::new(bytes))?;
    bounded_read_to_end(&mut decoder, cap)
}

fn bounded_read_to_end<R: Read>(reader: &mut R, cap: u64) -> Result<Vec<u8>, LoaderError> {
    // We refuse to allocate the whole cap upfront — bound the chunk and stop the moment
    // we tip over.
    let mut out = Vec::with_capacity(64 * 1024);
    // Boxed to keep the stack frame small (clippy::large_stack_arrays).
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            return Ok(out);
        }
        let next_total = out.len() as u64 + n as u64;
        if next_total > cap {
            return Err(LoaderError::DecompressedTooLarge {
                size: next_total,
                cap,
            });
        }
        out.extend_from_slice(&buf[..n]);
    }
}

fn read_le_u64_at(bytes: &[u8], offset: usize) -> Result<u64, LoaderError> {
    let slice = bytes
        .get(offset..offset + 8)
        .ok_or(LoaderError::ImageTooSmall { size: bytes.len() })?;
    let mut le = [0u8; 8];
    le.copy_from_slice(slice);
    Ok(u64::from_le_bytes(le))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal aarch64 boot header with the supplied text_offset / image_size.
    fn synth_image(text_offset: u64, image_size: u64, payload_len: usize) -> Vec<u8> {
        let mut img = vec![0u8; payload_len.max(MIN_IMAGE_SIZE + 16)];
        img[8..16].copy_from_slice(&text_offset.to_le_bytes());
        img[16..24].copy_from_slice(&image_size.to_le_bytes());
        img[AARCH64_MAGIC_OFFSET..AARCH64_MAGIC_OFFSET + 4].copy_from_slice(&MAGIC_AARCH64_IMAGE);
        img
    }

    #[test]
    fn detects_aarch64_image_by_offset_0x38_magic() {
        let img = synth_image(0x80000, 0, 256);
        assert_eq!(detect_format(&img), Some(ImageFormat::Aarch64Image));
    }

    #[test]
    fn detects_pe_when_mz_at_offset_zero() {
        let mut img = synth_image(0x80000, 0, 256);
        img[..2].copy_from_slice(&MAGIC_PE);
        assert_eq!(detect_format(&img), Some(ImageFormat::Pe));
    }

    #[test]
    fn detects_gzip_and_zstd_by_magic() {
        let mut g = vec![MAGIC_GZ[0], MAGIC_GZ[1]];
        g.extend_from_slice(&[0u8; 100]);
        assert_eq!(detect_format(&g), Some(ImageFormat::Gzip));

        let mut z = vec![MAGIC_ZST[0], MAGIC_ZST[1], MAGIC_ZST[2], MAGIC_ZST[3], 0, 0];
        z.extend_from_slice(&[0u8; 100]);
        assert_eq!(detect_format(&z), Some(ImageFormat::Zstd));
    }

    #[test]
    fn rejects_unknown_format() {
        let bytes = vec![0xFFu8; 256];
        assert!(matches!(
            load_from_bytes(&bytes),
            Err(LoaderError::UnknownFormat)
        ));
    }

    #[test]
    fn rejects_too_small_image() {
        let bytes = vec![0u8; 32];
        // No magic anywhere in 32 bytes — UnknownFormat is the right error before
        // ImageTooSmall.
        let err = load_from_bytes(&bytes).unwrap_err();
        assert!(matches!(err, LoaderError::UnknownFormat));
    }

    #[test]
    fn parses_text_offset_and_image_size() {
        let img = synth_image(0x80_0000, 0x100_0000, 0x100_0000);
        let loaded = load_from_bytes(&img).unwrap();
        assert_eq!(loaded.format, ImageFormat::Aarch64Image);
        assert_eq!(loaded.text_offset, 0x80_0000);
        assert_eq!(loaded.image_size, 0x100_0000);
    }

    #[test]
    fn load_address_floors_at_kernel_load_offset() {
        // text_offset is small; floor at DRAM_BASE + 2 MiB.
        let img = synth_image(0x80000, 0, 0x100_0000);
        let loaded = load_from_bytes(&img).unwrap();
        let addr = loaded.load_address().unwrap();
        assert_eq!(addr, DRAM_BASE + KERNEL_LOAD_OFFSET);
    }

    #[test]
    fn load_address_uses_text_offset_when_above_floor() {
        let img = synth_image(0x80_0000, 0, 0x100_0000);
        let loaded = load_from_bytes(&img).unwrap();
        let addr = loaded.load_address().unwrap();
        assert_eq!(addr, DRAM_BASE + 0x80_0000);
    }

    #[test]
    fn check_fits_in_ram_passes_when_image_fits() {
        let img = synth_image(0x80_0000, 0x0100_0000, 0x0100_0000);
        let loaded = load_from_bytes(&img).unwrap();
        let ram_size = 256 * 1024 * 1024; // 256 MiB
        let load_addr = loaded.check_fits_in_ram(ram_size).unwrap();
        assert_eq!(load_addr, DRAM_BASE + 0x80_0000);
    }

    #[test]
    fn check_fits_in_ram_rejects_when_kernel_overflows() {
        // 256 MiB image into a 128 MiB RAM region — should fail.
        let img = synth_image(0x80_0000, 0x1000_0000, 256);
        let loaded = load_from_bytes(&img).unwrap();
        let err = loaded.check_fits_in_ram(128 * 1024 * 1024).unwrap_err();
        assert!(matches!(err, LoaderError::DoesNotFitInRam { .. }));
    }

    #[test]
    fn invalid_text_offset_overflows_cleanly() {
        let img = synth_image(u64::MAX, 0, 256);
        let loaded = load_from_bytes(&img).unwrap();
        assert!(matches!(
            loaded.load_address(),
            Err(LoaderError::InvalidTextOffset { .. })
        ));
    }

    #[test]
    fn gzip_decompression_round_trips() {
        let inner = synth_image(0x80_0000, 0x100_0000, 0x100_0000);
        let mut compressed = Vec::new();
        {
            use std::io::Write;
            let mut enc =
                flate2::write::GzEncoder::new(&mut compressed, flate2::Compression::default());
            enc.write_all(&inner).unwrap();
            enc.finish().unwrap();
        }
        let loaded = load_from_bytes(&compressed).unwrap();
        assert_eq!(loaded.format, ImageFormat::Gzip);
        assert_eq!(loaded.text_offset, 0x80_0000);
        assert_eq!(loaded.image_size, 0x100_0000);
    }

    #[test]
    fn zstd_decompression_round_trips() {
        let inner = synth_image(0x40_0000, 0x80_0000, 0x80_0000);
        let compressed = zstd::stream::encode_all(Cursor::new(&inner), 0).unwrap();
        let loaded = load_from_bytes(&compressed).unwrap();
        assert_eq!(loaded.format, ImageFormat::Zstd);
        assert_eq!(loaded.text_offset, 0x40_0000);
    }

    #[test]
    fn decompression_bomb_is_rejected() {
        // Compress a 2 MiB image, then load with a 1 MiB cap — must reject.
        let inner = synth_image(0x40_0000, 0x80_0000, 2 * 1024 * 1024);
        let compressed = zstd::stream::encode_all(Cursor::new(&inner), 0).unwrap();
        let caps = MaxSizes {
            compressed: 8 * 1024 * 1024,
            decompressed: 1 * 1024 * 1024, // 1 MiB cap on output
        };
        let err = load_from_bytes_with_caps(&compressed, caps).unwrap_err();
        assert!(matches!(err, LoaderError::DecompressedTooLarge { .. }));
    }

    #[test]
    fn missing_aarch64_magic_after_decompression_rejected() {
        let inner = vec![0u8; 0x100]; // No magic at 0x38.
        let compressed = zstd::stream::encode_all(Cursor::new(&inner), 0).unwrap();
        let err = load_from_bytes(&compressed).unwrap_err();
        assert!(matches!(err, LoaderError::MissingAarch64Magic));
    }
}
