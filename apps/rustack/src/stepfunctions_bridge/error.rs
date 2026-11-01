//! Redacted internal bridge errors; sources never become ASL public causes.

use std::{fmt, sync::Arc};

use rustack_lambda_core::{error::LambdaServiceError, executor::ExecutorError};
use rustack_stepfunctions_core::{AslError, CallbackError};
use thiserror::Error;
use tokio::task::JoinError;

/// Bridge lifecycle/internal errors with bounded static public descriptions.
#[derive(Error)]
#[non_exhaustive]
pub enum BridgeError {
    /// Invalid operator bridge limits.
    #[error("invalid Lambda task bridge configuration")]
    Configuration,
    /// No Tokio runtime is active for owned admission.
    #[error("Lambda task bridge runtime unavailable")]
    Unavailable,
    /// Evaluated optimized parameters violate the supported schema.
    #[error("invalid Lambda task parameters")]
    Parameters,
    /// An observer deadline expired; the manager remains owned.
    #[error("Lambda task bridge observation deadline exceeded")]
    Deadline,
    /// Cleanup ownership remains uncertain and charged.
    #[error("Lambda task bridge cleanup quarantined")]
    Quarantined,
    /// Provider failure, retained without copying its host/business text.
    #[error("Lambda task provider failed")]
    Service(#[source] Arc<LambdaServiceError>),
    /// Execution/cleanup failure from an actual backend.
    #[error("Lambda task execution failed")]
    Executor(#[source] Arc<ExecutorError>),
    /// Bounded JSON/invariant failure.
    #[error("Lambda task data conversion failed")]
    Evaluation(
        #[from]
        #[source]
        AslError,
    ),
    /// Invalid or closed real callback invocation reporting control.
    #[error("Lambda callback invocation report failed")]
    Callback(#[source] Arc<CallbackError>),
    /// A supervised worker panicked or was unexpectedly cancelled.
    #[error("Lambda task bridge worker failed")]
    Worker(#[source] JoinError),
}
impl fmt::Debug for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
