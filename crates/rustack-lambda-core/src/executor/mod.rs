//! Lambda function execution engine.
//!
//! Routes execution through an object-safe backend with explicit availability.
//!
//! - [`NoopExecutor`] rejects execution in Disabled/unsupported Docker modes.
//! - [`AutoExecutor`] explicitly opts into Squib on macOS Zip functions, native otherwise.
//! - [`NativeExecutor`] runs trusted host-matching bootstraps without isolation.
//! - [`SquibExecutor`] runs arm64 Zip functions through a microVM guest agent.
//!
//! Docker execution is not supported; it never falls back to native or successful echo.
//!
//! All backends share a single in-process Lambda Runtime API server (Phase 2)
//! so the bootstrap-side protocol is identical to AWS.
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
pub use types::{ExecutorBackend, InvokeRequest, InvokeResponse, PackageType};

/// Backend that turns an [`InvokeRequest`] into an [`InvokeResponse`].
#[async_trait]
pub trait Executor: std::fmt::Debug + Send + Sync + 'static {
    /// Check whether execution is enabled before accepting asynchronous work.
    fn available(&self) -> Result<(), ExecutorError> {
        Ok(())
    }

    /// Run the function and return its response.
    async fn invoke(&self, req: InvokeRequest) -> Result<InvokeResponse, ExecutorError>;

    /// Stop all warm instances and release resources.
    async fn shutdown(&self);
}
