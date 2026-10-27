//! Retained code-volume staging and physical retirement, separate from readiness.

mod entry;
mod owner;
mod stage;
mod types;

pub use owner::{CodeVolumeManager, CodeVolumePort};
pub use types::{
    CodePreparation, CodeRetirement, CodeVolumeConfig, CodeVolumeIdentity, CodeVolumeStatus,
    PinnedCode,
};

#[cfg(test)]
mod tests;
