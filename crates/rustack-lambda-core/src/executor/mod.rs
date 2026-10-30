//! Lambda function execution engine.
//!
//! Routes execution through an object-safe backend with explicit availability.
//!
//! - [`NoopExecutor`] rejects execution in Disabled/unqualified Docker modes.
//! - [`AutoExecutor`] explicitly opts into Squib on macOS Zip functions, native otherwise.
//! - [`NativeExecutor`] runs trusted host-matching bootstraps without isolation.
//! - [`SquibExecutor`] runs arm64 Zip functions through a microVM guest agent.
//!
//! On Unix, `DockerExecutor` composes the retained image runtime using a trusted
//! readiness port. Ordinary CLI Docker selection requires the startup coordinator.
//!
//! Native instances expose a host Runtime API socket; Squib delegates the
//! bootstrap-side protocol to its guest agent.
//!
//! `async-trait` is required because `RustackLambda` stores the executor as
//! `Arc<dyn Executor>` for backend swapping at startup; the trait must be
//! object-safe.

mod auto;
mod error;
mod instance;
mod native;
mod noop;
pub mod runtime_api;
mod squib;
mod types;

use async_trait::async_trait;
pub use auto::AutoExecutor;
pub use error::ExecutorError;
pub use native::NativeExecutor;
pub use noop::NoopExecutor;
pub use squib::{SquibExecutor, SquibExecutorConfig};
pub use types::{ExecutionRevision, ExecutorBackend, InvokeRequest, InvokeResponse};

#[cfg(unix)]
pub use crate::docker::{
    DockerExecutor, DockerExecutorConfig, DockerExecutorError, DockerExecutorSettings,
    DockerReadiness,
};
use crate::{artifact::Architecture, invocation::InvocationContext};

/// Backend that turns an [`InvokeRequest`] into an [`InvokeResponse`].
#[async_trait]
pub trait Executor: std::fmt::Debug + Send + Sync + 'static {
    /// Verify image execution readiness before admitting asynchronous deployment.
    ///
    /// Backends must explicitly opt in after their Docker runtime probes succeed.
    ///
    /// # Errors
    /// The default rejects image deployment for unsupported or disabled backends.
    ///
    /// ```
    /// # fn check(executor: &dyn rustack_lambda_core::executor::Executor) {
    /// let ready = executor.image_available().is_ok();
    /// # }
    /// ```
    fn image_available(&self) -> Result<(), ExecutorError> {
        Err(ExecutorError::Unsupported(
            "Image execution is unavailable".into(),
        ))
    }

    /// Check optional provided-runtime ZIP support for a captured architecture.
    ///
    /// # Errors
    /// The default rejects capability which this backend has not installed.
    ///
    /// ```
    /// # fn check(executor:&dyn rustack_lambda_core::executor::Executor) {
    /// let available=executor.zip_available(rustack_lambda_core::artifact::Architecture::Arm64).is_ok();
    /// # }
    /// ```
    fn zip_available(&self, _architecture: Architecture) -> Result<(), ExecutorError> {
        Err(ExecutorError::Unsupported(
            "ZIP execution is unavailable".into(),
        ))
    }

    /// Check whether execution is enabled before accepting asynchronous work.
    ///
    /// # Errors
    /// Returns an unavailable-backend or incompatible-artifact/configuration error.
    ///
    /// ```
    /// # use rustack_lambda_core::executor::{Executor, InvokeRequest};
    /// # fn probe(executor: &dyn Executor, request: &InvokeRequest) {
    /// let available = executor.available(request).is_ok();
    /// # }
    /// ```
    fn available(&self, _request: &InvokeRequest) -> Result<(), ExecutorError> {
        Ok(())
    }

    /// Run the function and return its response.
    ///
    /// The work supervisor retains this future through cancellation and cleanup.
    ///
    /// # Errors
    /// Returns invocation, initialization, transport or cleanup errors. A clean
    /// outcome requires backend resources to be reusable or confirmed retired.
    ///
    /// ```no_run
    /// # use rustack_lambda_core::{executor::{Executor, InvokeRequest}, invocation::InvocationContext};
    /// # async fn invoke(executor: &dyn Executor, request: InvokeRequest, context: InvocationContext) {
    /// let result = executor.invoke(request, context).await;
    /// # }
    /// ```
    async fn invoke(
        &self,
        req: InvokeRequest,
        context: InvocationContext,
    ) -> Result<InvokeResponse, ExecutorError>;

    /// Stop all warm instances and release resources.
    ///
    /// # Errors
    /// Returns `CleanupFailed` if owned resource cleanup cannot be confirmed.
    /// Dropping a shutdown waiter must retain the underlying cleanup operation.
    ///
    /// ```no_run
    /// # async fn stop(executor: &dyn rustack_lambda_core::executor::Executor) {
    /// let clean = executor.shutdown().await.is_ok();
    /// # }
    /// ```
    async fn shutdown(&self) -> Result<(), ExecutorError>;
}
