//! Owned, validated deployment artifacts shared by captured execution requests.
//!
//! ZIP leases keep successfully staged code alive through invocation cleanup.
//! Images carry immutable daemon identity rather than an unresolved mutable tag.

mod execution;
mod image;
mod pinned;

use std::{
    fmt::{Debug, Formatter, Result as FmtResult},
    fs::{canonicalize, remove_dir_all, symlink_metadata},
    path::{Path, PathBuf},
    sync::Arc,
};

use bytes::Bytes;
pub use execution::{ImageExecutionDefaults, ImageExecutionMetadata, ImageExecutionPlan};
pub use image::{
    Architecture, ArtifactError, ImageMetadata, ImageReference, ResolvedImage, Sha256Digest,
};
pub use pinned::{PinnedImage, UnpackedImageSize};
pub(crate) use pinned::{PreparationAdmission, PreparationReceipt};
use tempfile::TempDir;

use crate::storage::compute_sha256;

pub(crate) const MAX_ZIP_BYTES: usize = 50 * 1024 * 1024;
pub(crate) const MAX_ZIP_BASE64_BYTES: usize = MAX_ZIP_BYTES.div_ceil(3) * 4;

pub(crate) const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024 * 1024;
#[cfg(test)]
use crate::{storage::stage_zip, test_zip::minimal_zip};

/// One unambiguous deployment artifact for an invocation.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum DeploymentArtifact {
    /// Successfully staged ZIP, with shared lifetime ownership.
    Zip(ValidatedArtifact),
    /// Live Linux image lease, retained through backend cleanup.
    Image(PinnedImage),
}

/// Image deployment availability; snapshot metadata never grants a live lease.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub enum ImageArtifact {
    /// No image has been prepared.
    #[default]
    Unresolved,
    /// Immutable restored metadata requiring availability reacquisition.
    Restored(ResolvedImage),
    /// Prepared image with shared availability ownership.
    Pinned(PinnedImage),
}
impl ImageArtifact {
    /// Captured metadata, whether or not runtime availability has been acquired.
    ///
    /// ```
    /// assert!(rustack_lambda_core::artifact::ImageArtifact::Unresolved.resolved().is_none());
    /// ```
    #[must_use]
    pub fn resolved(&self) -> Option<&ResolvedImage> {
        match self {
            Self::Unresolved => None,
            Self::Restored(image) => Some(image),
            Self::Pinned(image) => Some(image.resolved()),
        }
    }

    /// Live artifact ownership, absent for restored metadata alone.
    ///
    /// ```
    /// assert!(rustack_lambda_core::artifact::ImageArtifact::Unresolved.pinned().is_none());
    /// ```
    #[must_use]
    pub fn pinned(&self) -> Option<&PinnedImage> {
        match self {
            Self::Pinned(image) => Some(image),
            Self::Unresolved | Self::Restored(_) => None,
        }
    }
}

/// Shared ownership of a ZIP archive and the extracted code it produced.
///
/// Only the function store can issue a lease after successful staging.
#[derive(Clone)]
#[non_exhaustive]
pub struct ValidatedArtifact(Arc<ZipStorage>);

struct ZipStorage {
    directory: Option<TempDir>,
    root: PathBuf,
    extracted: PathBuf,
    bytes: Bytes,
    sha256: String,
}

impl Debug for ZipStorage {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("ZipStorage")
            .field("sha256", &self.sha256)
            .field("archive_bytes", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

impl Debug for ValidatedArtifact {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("ValidatedArtifact")
            .field("sha256", &self.0.sha256)
            .field("archive_bytes", &self.0.bytes.len())
            .finish_non_exhaustive()
    }
}

impl ValidatedArtifact {
    #[cfg(test)]
    pub(crate) fn fixture() -> Self {
        stage_zip(&std::env::temp_dir(), Bytes::from(minimal_zip(b"fixture")))
            .expect("trusted ZIP unit fixture must stage successfully")
    }
    pub(crate) fn from_staged(
        directory: TempDir,
        root: PathBuf,
        extracted: PathBuf,
        bytes: Bytes,
    ) -> Self {
        let sha256 = compute_sha256(&bytes);
        Self(Arc::new(ZipStorage {
            directory: Some(directory),
            root,
            extracted,
            bytes,
            sha256,
        }))
    }

    /// Absolute root of the extracted code, not a user-controlled logical name.
    ///
    /// ```
    /// # fn inspect(artifact: &rustack_lambda_core::artifact::ValidatedArtifact) {
    /// assert!(artifact.code_root().is_absolute());
    /// # }
    /// ```
    #[must_use]
    pub fn code_root(&self) -> &Path {
        &self.0.extracted
    }

    /// Original bytes from the same successfully staged ZIP.
    ///
    /// ```
    /// # fn inspect(artifact: &rustack_lambda_core::artifact::ValidatedArtifact) {
    /// let shared_bytes = artifact.zip_bytes().clone();
    /// assert_eq!(shared_bytes, *artifact.zip_bytes());
    /// # }
    /// ```
    #[must_use]
    pub fn zip_bytes(&self) -> &Bytes {
        &self.0.bytes
    }

    /// Base64-encoded SHA-256 of the original archive.
    ///
    /// ```
    /// # fn inspect(artifact: &rustack_lambda_core::artifact::ValidatedArtifact) {
    /// assert_eq!(artifact.sha256().len(), 44);
    /// # }
    /// ```
    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.0.sha256
    }

    pub(crate) fn storage_root(&self) -> &Path {
        &self.0.root
    }
}

impl Drop for ZipStorage {
    #[allow(clippy::disallowed_methods)] // Last-lease Drop owns bounded directory cleanup; async IO is unavailable here.
    fn drop(&mut self) {
        let Some(directory) = self.directory.take() else {
            return;
        };
        let path = directory.keep();
        let safe = canonicalize(&self.root).is_ok_and(|root| root == self.root)
            && symlink_metadata(&self.root)
                .is_ok_and(|meta| meta.is_dir() && !meta.file_type().is_symlink())
            && symlink_metadata(&path)
                .is_ok_and(|meta| meta.is_dir() && !meta.file_type().is_symlink());
        if !safe {
            tracing::error!("Refusing cleanup of replaced Lambda artifact root");
            return;
        }
        if let Err(error) = remove_dir_all(path) {
            tracing::error!(%error, "Lambda artifact cleanup failed");
        }
    }
}
