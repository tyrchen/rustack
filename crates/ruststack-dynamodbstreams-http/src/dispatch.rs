//! DynamoDB Streams handler trait and operation dispatch.

use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;

use ruststack_dynamodbstreams_model::error::DynamoDBStreamsError;
use ruststack_dynamodbstreams_model::operations::DynamoDBStreamsOperation;

use crate::body::DynamoDBStreamsResponseBody;

/// Trait that the DynamoDB Streams business logic provider must implement.
///
/// The handler receives a parsed operation enum and the raw JSON body bytes,
/// and returns a complete HTTP response. This trait serves as the boundary
/// between the HTTP transport layer and the business logic layer.
pub trait DynamoDBStreamsHandler: Send + Sync + 'static {
    /// Handle a DynamoDB Streams operation and produce an HTTP response.
    fn handle_operation(
        &self,
        op: DynamoDBStreamsOperation,
        body: Bytes,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        http::Response<DynamoDBStreamsResponseBody>,
                        DynamoDBStreamsError,
                    >,
                > + Send,
        >,
    >;
}

/// Dispatch a DynamoDB Streams operation to the handler.
pub async fn dispatch_operation<H: DynamoDBStreamsHandler>(
    handler: &H,
    op: DynamoDBStreamsOperation,
    body: Bytes,
) -> Result<http::Response<DynamoDBStreamsResponseBody>, DynamoDBStreamsError> {
    tracing::debug!(operation = %op, "dispatching DynamoDB Streams operation");
    handler.handle_operation(op, body).await
}

/// Default handler that returns an error for all operations.
#[derive(Debug, Clone, Default)]
pub struct NotImplementedHandler;

impl DynamoDBStreamsHandler for NotImplementedHandler {
    fn handle_operation(
        &self,
        op: DynamoDBStreamsOperation,
        _body: Bytes,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        http::Response<DynamoDBStreamsResponseBody>,
                        DynamoDBStreamsError,
                    >,
                > + Send,
        >,
    > {
        Box::pin(async move { Err(DynamoDBStreamsError::unknown_operation(op.as_str())) })
    }
}
