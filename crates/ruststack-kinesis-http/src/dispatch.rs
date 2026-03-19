//! Kinesis handler trait and operation dispatch.

use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;

use ruststack_kinesis_model::error::KinesisError;
use ruststack_kinesis_model::operations::KinesisOperation;

use crate::body::KinesisResponseBody;

/// Trait that the Kinesis business logic provider must implement.
///
/// The handler receives a parsed operation enum and the raw JSON body bytes,
/// and returns a complete HTTP response. This trait serves as the boundary
/// between the HTTP transport layer and the business logic layer.
pub trait KinesisHandler: Send + Sync + 'static {
    /// Handle a Kinesis operation and produce an HTTP response.
    fn handle_operation(
        &self,
        op: KinesisOperation,
        body: Bytes,
    ) -> Pin<
        Box<dyn Future<Output = Result<http::Response<KinesisResponseBody>, KinesisError>> + Send>,
    >;
}

/// Dispatch a Kinesis operation to the handler.
pub async fn dispatch_operation<H: KinesisHandler>(
    handler: &H,
    op: KinesisOperation,
    body: Bytes,
) -> Result<http::Response<KinesisResponseBody>, KinesisError> {
    tracing::debug!(operation = %op, "dispatching Kinesis operation");
    handler.handle_operation(op, body).await
}

/// Default handler that returns an error for all operations.
#[derive(Debug, Clone, Default)]
pub struct NotImplementedHandler;

impl KinesisHandler for NotImplementedHandler {
    fn handle_operation(
        &self,
        op: KinesisOperation,
        _body: Bytes,
    ) -> Pin<
        Box<dyn Future<Output = Result<http::Response<KinesisResponseBody>, KinesisError>> + Send>,
    > {
        Box::pin(async move { Err(KinesisError::not_implemented(op.as_str())) })
    }
}
