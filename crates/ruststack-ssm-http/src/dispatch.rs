//! SSM handler trait and operation dispatch.

use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;

use ruststack_ssm_model::error::SsmError;
use ruststack_ssm_model::operations::SsmOperation;

use crate::body::SsmResponseBody;

/// Trait that the SSM business logic provider must implement.
///
/// The handler receives a parsed operation enum and the raw JSON body bytes,
/// and returns a complete HTTP response. This trait serves as the boundary
/// between the HTTP transport layer and the business logic layer.
pub trait SsmHandler: Send + Sync + 'static {
    /// Handle an SSM operation and produce an HTTP response.
    fn handle_operation(
        &self,
        op: SsmOperation,
        body: Bytes,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<SsmResponseBody>, SsmError>> + Send>>;
}

/// Dispatch an SSM operation to the handler.
pub async fn dispatch_operation<H: SsmHandler>(
    handler: &H,
    op: SsmOperation,
    body: Bytes,
) -> Result<http::Response<SsmResponseBody>, SsmError> {
    tracing::debug!(operation = %op, "dispatching SSM operation");
    handler.handle_operation(op, body).await
}

/// Default handler that returns an error for all operations.
#[derive(Debug, Clone, Default)]
pub struct NotImplementedHandler;

impl SsmHandler for NotImplementedHandler {
    fn handle_operation(
        &self,
        op: SsmOperation,
        _body: Bytes,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<SsmResponseBody>, SsmError>> + Send>>
    {
        Box::pin(async move { Err(SsmError::not_implemented(op.as_str())) })
    }
}
