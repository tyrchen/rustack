//! Disabled execution backend. Never executes or fabricates a successful result.
use async_trait::async_trait;

use super::{Executor, ExecutorError, InvokeRequest, InvokeResponse};
use crate::invocation::InvocationContext;

/// Explicit unavailable backend for metadata-only operation.
#[derive(Debug, Default, Clone)]
pub struct NoopExecutor {
    unqualified_docker: bool,
}
impl NoopExecutor {
    /// Construct a disabled executor.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    pub(crate) fn without_docker_runtime() -> Self {
        Self {
            unqualified_docker: true,
        }
    }
    fn error(&self) -> ExecutorError {
        if self.unqualified_docker {
            ExecutorError::Unsupported("qualified Docker startup has not been installed".into())
        } else {
            ExecutorError::Disabled
        }
    }
}
#[async_trait]
impl Executor for NoopExecutor {
    fn available(&self, _request: &InvokeRequest) -> Result<(), ExecutorError> {
        Err(self.error())
    }
    async fn invoke(
        &self,
        _req: InvokeRequest,
        _context: InvocationContext,
    ) -> Result<InvokeResponse, ExecutorError> {
        Err(self.error())
    }
    async fn shutdown(&self) -> Result<(), ExecutorError> {
        Ok(())
    }
}
