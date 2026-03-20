//! CloudWatch handler trait and operation dispatch.

use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;

use ruststack_cloudwatch_model::error::CloudWatchError;
use ruststack_cloudwatch_model::operations::CloudWatchOperation;

use crate::body::CloudWatchResponseBody;

/// Trait that the CloudWatch business logic provider must implement.
///
/// The handler receives a parsed operation enum and the raw form body bytes.
/// CloudWatch uses form-urlencoded params (awsQuery protocol).
pub trait CloudWatchHandler: Send + Sync + 'static {
    /// Handle a CloudWatch operation and produce an HTTP response.
    fn handle_operation(
        &self,
        op: CloudWatchOperation,
        body: Bytes,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<http::Response<CloudWatchResponseBody>, CloudWatchError>>
                + Send,
        >,
    >;
}

/// Dispatch a CloudWatch operation to the handler.
pub async fn dispatch_operation<H: CloudWatchHandler>(
    handler: &H,
    op: CloudWatchOperation,
    body: Bytes,
) -> Result<http::Response<CloudWatchResponseBody>, CloudWatchError> {
    tracing::debug!(operation = %op, "dispatching CloudWatch operation");
    handler.handle_operation(op, body).await
}
