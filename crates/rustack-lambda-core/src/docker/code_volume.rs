//! Retained code-volume staging and physical retirement, separate from readiness.

mod base;
mod entry;
mod owner;
mod stage;
mod types;

pub(in crate::docker) use base::validate_base;
pub use owner::{CodeVolumeManager, CodeVolumePort};
pub use types::{
    CodePreparation, CodeRetirement, CodeVolumeConfig, CodeVolumeIdentity, CodeVolumeStatus,
    PinnedCode,
};

#[cfg(test)]
pub(in crate::docker) mod tests;
