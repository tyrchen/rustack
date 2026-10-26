//! Errors surfaced by an [`Executor`](super::Executor).

use crate::error::LambdaServiceError;

/// Failure modes for an execution backend.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ExecutorError {
    /// Caller or owner requested cancellation; settlement is tracked separately.
    #[error("invocation cancelled")]
    Cancelled,
    /// Absolute invocation budget expired.
    #[error("invocation deadline exceeded")]
    DeadlineExceeded,
    /// Cleanup could not be confirmed; ownership remains supervised.
    #[error("cleanup not confirmed: {0}")]
    CleanupFailed(String),
    /// No backend is wired up — set `LAMBDA_EXECUTOR=native` (or `docker`).
    #[error("execution is disabled. Set LAMBDA_EXECUTOR=native or LAMBDA_EXECUTOR=docker")]
    Disabled,

    /// The function's code can't run on this host (arch/OS mismatch, missing
    /// `bootstrap`, unsupported runtime, etc.).
    #[error("backend cannot run this function: {0}")]
    Unsupported(String),

    /// The deployment package is missing the file the runtime expects.
    #[error("invalid function code: {0}")]
    InvalidCode(String),

    /// Bootstrap failed to start (or didn't poll `/next` within the init
    /// window).
    #[error("init failed: {0}")]
    InitFailed(String),

    /// Bootstrap exited or stopped responding mid-invocation.
    #[error("runtime exited: {0}")]
    RuntimeExited(String),

    /// Function exceeded its configured timeout.
    #[error("function timed out after {0:?}")]
    Timeout(std::time::Duration),

    /// Underlying I/O / Docker / process error.
    #[error("backend i/o error: {0}")]
    Io(String),
}

impl From<ExecutorError> for LambdaServiceError {
    fn from(e: ExecutorError) -> Self {
        match e {
            ExecutorError::Cancelled | ExecutorError::DeadlineExceeded => Self::ResourceNotReady {
                message: e.to_string(),
            },
            ExecutorError::Disabled => Self::ResourceNotReady {
                message: "Lambda execution is disabled; explicitly select a trusted execution \
                          backend"
                    .into(),
            },
            ExecutorError::Unsupported(m)
            | ExecutorError::InvalidCode(m)
            | ExecutorError::InitFailed(m)
            | ExecutorError::RuntimeExited(m)
            | ExecutorError::Io(m)
            | ExecutorError::CleanupFailed(m) => Self::Internal { message: m },
            ExecutorError::Timeout(d) => Self::Internal {
                message: format!("function timed out after {d:?}"),
            },
        }
    }
}
