//! Disabled execution backend. Never executes or fabricates a successful result.
use async_trait::async_trait;

use super::{Executor, ExecutorError, InvokeRequest, InvokeResponse};

/// Explicit unavailable backend for metadata-only operation.
#[derive(Debug, Default, Clone)]
pub struct NoopExecutor {
    unsupported_docker: bool,
}
impl NoopExecutor {
    /// Construct a disabled executor.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    pub(crate) fn docker_unavailable() -> Self {
        Self {
            unsupported_docker: true,
        }
    }
    fn error(&self) -> ExecutorError {
        if self.unsupported_docker {
            ExecutorError::Unsupported("Docker execution is not implemented in this build".into())
        } else {
            ExecutorError::Disabled
        }
    }
}
#[async_trait]
impl Executor for NoopExecutor {
    fn available(&self) -> Result<(), ExecutorError> {
        Err(self.error())
    }
    async fn invoke(&self, _req: InvokeRequest) -> Result<InvokeResponse, ExecutorError> {
        Err(self.error())
    }
    async fn shutdown(&self) {}
}
