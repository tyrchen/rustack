//! Typed redacted runtime/control failures.

use std::{fmt, sync::Arc};

use thiserror::Error;
use tokio::task::JoinError;

use crate::{AslError, CallbackError, FrameError};

/// Runtime admission/control/history errors without payloads or panic text.
#[derive(Error)]
#[non_exhaustive]
pub enum ExecutionError {
    /// A retained callback protocol or ownership source failed.
    #[error("workflow callback owner failed")]
    Callback(#[source] Arc<CallbackError>),
    /// A structured frame protocol or ownership invariant failed.
    #[error("structured workflow frame failed")]
    Frame(#[source] FrameError),
    /// Invalid trusted runtime/history settings or page arguments.
    #[error("invalid workflow runtime configuration or page")]
    Invalid,
    /// Start requires an active Tokio runtime and an accepting supervisor.
    #[error("workflow supervisor unavailable")]
    Unavailable,
    /// A bounded admission/history capacity cannot be reserved.
    #[error("workflow capacity exhausted")]
    Capacity,
    /// The observer's absolute deadline expired; ownership remains unchanged.
    #[error("workflow observation deadline exceeded")]
    Deadline,
    /// External cleanup or worker lifetime is uncertain.
    #[error("workflow cleanup quarantined")]
    Quarantined,
    /// An interpreter/encoding failure, retained as a typed source.
    #[error("workflow runtime evaluation failed")]
    Evaluation(#[source] Arc<AslError>),
    /// A supervised worker panicked or was unexpectedly cancelled.
    #[error("workflow worker failed")]
    Worker(#[source] JoinError),
}
impl fmt::Debug for ExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl From<AslError> for ExecutionError {
    fn from(source: AslError) -> Self {
        Self::Evaluation(Arc::new(source))
    }
}
