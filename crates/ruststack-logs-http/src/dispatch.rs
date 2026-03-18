//! CloudWatch Logs handler trait and operation dispatch.

use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;

use ruststack_logs_model::error::LogsError;
use ruststack_logs_model::operations::LogsOperation;

use crate::body::LogsResponseBody;

/// Trait that the CloudWatch Logs business logic provider must implement.
///
/// The handler receives a parsed operation enum and the raw JSON body bytes,
/// and returns a complete HTTP response. This trait serves as the boundary
/// between the HTTP transport layer and the business logic layer.
pub trait LogsHandler: Send + Sync + 'static {
    /// Handle a CloudWatch Logs operation and produce an HTTP response.
    fn handle_operation(
        &self,
        op: LogsOperation,
        body: Bytes,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<LogsResponseBody>, LogsError>> + Send>>;
}

/// Dispatch a CloudWatch Logs operation to the handler.
pub async fn dispatch_operation<H: LogsHandler>(
    handler: &H,
    op: LogsOperation,
    body: Bytes,
) -> Result<http::Response<LogsResponseBody>, LogsError> {
    tracing::debug!(operation = %op, "dispatching CloudWatch Logs operation");
    handler.handle_operation(op, body).await
}

/// Default handler that returns an error for all operations.
#[derive(Debug, Clone, Default)]
pub struct NotImplementedHandler;

impl LogsHandler for NotImplementedHandler {
    fn handle_operation(
        &self,
        op: LogsOperation,
        _body: Bytes,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<LogsResponseBody>, LogsError>> + Send>>
    {
        Box::pin(async move { Err(LogsError::not_implemented(op.as_str())) })
    }
}
