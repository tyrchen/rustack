//! Lambda function execution engine.
//!
//! Converts the stubbed `Invoke` echo path into real execution by routing
//! every request through an `Executor` trait. Backends include:
//!
//! - [`NoopExecutor`] — preserves the legacy "echo back the payload" behavior; used in unit tests
//!   and when `LAMBDA_EXECUTOR=disabled`.
//! - [`AutoExecutor`] — defaults macOS Zip Lambdas to Squib and otherwise uses native execution.
//! - `NativeExecutor` (Phase 3) — spawns `provided.*` bootstraps directly on the host.
//! - `DockerExecutor` (Phase 4) — runs any supported runtime in an AWS Lambda base image.
//! - `SquibExecutor` — runs `arm64` Zip functions through a Squib microVM guest agent.
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
    /// Run the function and return its response.
    async fn invoke(&self, req: InvokeRequest) -> Result<InvokeResponse, ExecutorError>;

    /// Stop all warm instances and release resources.
    async fn shutdown(&self);
}
