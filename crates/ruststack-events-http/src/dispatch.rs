//! EventBridge handler trait and operation dispatch.

use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;

use ruststack_events_model::error::EventsError;
use ruststack_events_model::operations::EventsOperation;

use crate::body::EventsResponseBody;

/// Trait that the EventBridge business logic provider must implement.
///
/// The handler receives a parsed operation enum and the raw JSON body bytes,
/// and returns a complete HTTP response. This trait serves as the boundary
/// between the HTTP transport layer and the business logic layer.
pub trait EventsHandler: Send + Sync + 'static {
    /// Handle an EventBridge operation and produce an HTTP response.
    fn handle_operation(
        &self,
        op: EventsOperation,
        body: Bytes,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<EventsResponseBody>, EventsError>> + Send>>;
}

/// Dispatch an EventBridge operation to the handler.
pub async fn dispatch_operation<H: EventsHandler>(
    handler: &H,
    op: EventsOperation,
    body: Bytes,
) -> Result<http::Response<EventsResponseBody>, EventsError> {
    tracing::debug!(operation = %op, "dispatching EventBridge operation");
    handler.handle_operation(op, body).await
}

/// Default handler that returns an error for all operations.
#[derive(Debug, Clone, Default)]
pub struct NotImplementedHandler;

impl EventsHandler for NotImplementedHandler {
    fn handle_operation(
        &self,
        op: EventsOperation,
        _body: Bytes,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<EventsResponseBody>, EventsError>> + Send>>
    {
        Box::pin(async move { Err(EventsError::not_implemented(op.as_str())) })
    }
}
