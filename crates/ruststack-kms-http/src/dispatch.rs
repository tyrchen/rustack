//! KMS handler trait and operation dispatch.

use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;

use ruststack_kms_model::error::KmsError;
use ruststack_kms_model::operations::KmsOperation;

use crate::body::KmsResponseBody;

/// Trait that the KMS business logic provider must implement.
///
/// The handler receives a parsed operation enum and the raw JSON body bytes,
/// and returns a complete HTTP response. This trait serves as the boundary
/// between the HTTP transport layer and the business logic layer.
pub trait KmsHandler: Send + Sync + 'static {
    /// Handle a KMS operation and produce an HTTP response.
    fn handle_operation(
        &self,
        op: KmsOperation,
        body: Bytes,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<KmsResponseBody>, KmsError>> + Send>>;
}

/// Dispatch a KMS operation to the handler.
pub async fn dispatch_operation<H: KmsHandler>(
    handler: &H,
    op: KmsOperation,
    body: Bytes,
) -> Result<http::Response<KmsResponseBody>, KmsError> {
    tracing::debug!(operation = %op, "dispatching KMS operation");
    handler.handle_operation(op, body).await
}

/// Default handler that returns an error for all operations.
#[derive(Debug, Clone, Default)]
pub struct NotImplementedHandler;

impl KmsHandler for NotImplementedHandler {
    fn handle_operation(
        &self,
        op: KmsOperation,
        _body: Bytes,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<KmsResponseBody>, KmsError>> + Send>>
    {
        Box::pin(async move { Err(KmsError::not_implemented(op.as_str())) })
    }
}
