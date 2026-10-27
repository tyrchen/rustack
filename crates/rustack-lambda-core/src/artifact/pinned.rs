//! Live image artifacts cannot be reconstructed from snapshot metadata alone.

use std::{
    fmt,
    num::NonZeroU64,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use serde::{Deserialize, Serialize};
use tokio::sync::{OwnedSemaphorePermit, watch};
use typed_builder::TypedBuilder;

use super::{ArtifactError, MAX_IMAGE_BYTES, ResolvedImage};

/// Positive daemon-reported unpacked image storage, within the local10GiB quota.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
#[non_exhaustive]
pub struct UnpackedImageSize(NonZeroU64);
impl TryFrom<u64> for UnpackedImageSize {
    type Error = ArtifactError;
    fn try_from(value: u64) -> Result<Self, Self::Error> {
        let value = NonZeroU64::new(value)
            .filter(|value| value.get() <= MAX_IMAGE_BYTES)
            .ok_or(ArtifactError::Inspection)?;
        Ok(Self(value))
    }
}
impl From<UnpackedImageSize> for u64 {
    fn from(size: UnpackedImageSize) -> Self {
        size.0.get()
    }
}
impl UnpackedImageSize {
    /// Number of validated unpacked bytes, independent of compressed size.
    ///
    /// ```
    /// let size = rustack_lambda_core::artifact::UnpackedImageSize::try_from(4096)?;
    /// assert_eq!(size.get(),4096);
    /// # Ok::<(),rustack_lambda_core::artifact::ArtifactError>(())
    /// ```
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// Shared live pin lease; cloning retains availability until joined retirement.
#[derive(Clone)]
#[non_exhaustive]
pub struct PinnedImage(Arc<Lease>);

/// Admission retained by the image actor until a consumed handoff or physical retirement.
#[derive(Debug)]
pub(crate) struct PreparationAdmission {
    _image: OwnedSemaphorePermit,
    _deployment: Option<Arc<OwnedSemaphorePermit>>,
}
impl PreparationAdmission {
    pub(crate) fn new(
        image: OwnedSemaphorePermit,
        deployment: Option<Arc<OwnedSemaphorePermit>>,
    ) -> Self {
        Self {
            _image: image,
            _deployment: deployment,
        }
    }
}
/// Shared acceptance receipt; clearing it releases guards retained above workers.
pub(crate) type PreparationReceipt = watch::Sender<Option<PreparationAdmission>>;

#[derive(TypedBuilder)]
struct Lease {
    resolved: ResolvedImage,
    size: UnpackedImageSize,
    release: watch::Sender<()>,
    active: Arc<AtomicBool>,
    stop: watch::Receiver<bool>,
    preparation: Option<PreparationReceipt>,
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.release.send_replace(());
    }
}
impl fmt::Debug for PinnedImage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PinnedImage")
            .field("image_id", &self.0.resolved.metadata().image_id)
            .field("unpacked_bytes", &self.0.size)
            .field("live", &self.is_live())
            .finish_non_exhaustive()
    }
}
impl PinnedImage {
    #[cfg(test)]
    pub(crate) fn fixture(resolved: ResolvedImage) -> Self {
        let (release, _) = watch::channel(());
        let (_, stop) = watch::channel(false);
        let size = UnpackedImageSize::try_from(resolved.metadata().size_bytes)
            .expect("validated unit image size");
        Self::new(
            resolved,
            size,
            release,
            Arc::new(AtomicBool::new(true)),
            stop,
        )
    }

    #[cfg(test)]
    pub(crate) fn new(
        resolved: ResolvedImage,
        size: UnpackedImageSize,
        release: watch::Sender<()>,
        active: Arc<AtomicBool>,
        stop: watch::Receiver<bool>,
    ) -> Self {
        Self::with_receipt(resolved, size, release, active, stop, None)
    }

    pub(crate) fn with_receipt(
        resolved: ResolvedImage,
        size: UnpackedImageSize,
        release: watch::Sender<()>,
        active: Arc<AtomicBool>,
        stop: watch::Receiver<bool>,
        preparation: Option<PreparationReceipt>,
    ) -> Self {
        Self(Arc::new(
            Lease::builder()
                .resolved(resolved)
                .size(size)
                .release(release)
                .active(active)
                .stop(stop)
                .preparation(preparation)
                .build(),
        ))
    }

    /// Confirm a validated consumed artifact or a committed deployment CAS.
    pub(crate) fn accept_preparation(&self) {
        if let Some(receipt) = &self.0.preparation {
            receipt.send_replace(None);
        }
    }

    /// Immutable metadata captured by preparation; never resolve a mutable tag.
    ///
    /// ```no_run
    /// # fn inspect(image:&rustack_lambda_core::artifact::PinnedImage) {
    /// let id = &image.resolved().metadata().image_id;
    /// # }
    /// ```
    #[must_use]
    pub fn resolved(&self) -> &ResolvedImage {
        &self.0.resolved
    }

    /// Proven unpacked size used by deployment quota enforcement.
    ///
    /// ```no_run
    /// # fn inspect(image:&rustack_lambda_core::artifact::PinnedImage) {
    /// let bytes = image.unpacked_size().get();
    /// # }
    /// ```
    #[must_use]
    pub fn unpacked_size(&self) -> UnpackedImageSize {
        self.0.size
    }

    /// Whether the owning manager still permits consumers to use this lease.
    ///
    /// Shutdown revokes leases after the runtime has joined their consumers.
    ///
    /// ```no_run
    /// # fn inspect(image:&rustack_lambda_core::artifact::PinnedImage) {
    /// let available = image.is_live();
    /// # }
    /// ```
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.0.active.load(Ordering::Acquire) && !*self.0.stop.borrow()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::Architecture;

    #[test]
    fn test_should_validate_unpacked_storage_limits_including_deserialization() {
        assert!(UnpackedImageSize::try_from(0).is_err());
        assert!(UnpackedImageSize::try_from(MAX_IMAGE_BYTES + 1).is_err());
        assert_eq!(
            UnpackedImageSize::try_from(MAX_IMAGE_BYTES).unwrap().get(),
            MAX_IMAGE_BYTES
        );
        assert!(serde_json::from_str::<UnpackedImageSize>("0").is_err());
        let size: UnpackedImageSize = serde_json::from_str("4096").unwrap();
        assert_eq!(size.get(), 4096);
    }

    #[tokio::test]
    async fn test_should_keep_release_channel_until_last_clone_and_revoke_on_stop() {
        let (release, mut released) = watch::channel(());
        let (stop, stopping) = watch::channel(false);
        let active = Arc::new(AtomicBool::new(true));
        let image = PinnedImage::new(
            ResolvedImage::fixture(Architecture::Arm64),
            4096.try_into().unwrap(),
            release,
            active.clone(),
            stopping,
        );
        let captured = image.clone();
        drop(image);
        assert!(!released.has_changed().unwrap());
        assert!(captured.is_live());
        assert_eq!(captured.unpacked_size().get(), 4096);
        assert!(!format!("{captured:?}").contains("entrypoint"));
        stop.send_replace(true);
        assert!(!captured.is_live());
        drop(captured);
        let _ = released.changed().await;
        assert!(released.has_changed().is_err());
    }
}
