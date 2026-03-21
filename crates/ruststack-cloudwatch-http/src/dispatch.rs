//! CloudWatch handler trait and operation dispatch.

use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;

use ruststack_cloudwatch_model::error::CloudWatchError;
use ruststack_cloudwatch_model::operations::CloudWatchOperation;

use crate::body::CloudWatchResponseBody;

/// The wire protocol for the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// awsQuery: form-urlencoded request, XML response.
    AwsQuery,
    /// Smithy rpcv2Cbor: CBOR request, CBOR response.
    RpcV2Cbor,
}

/// Trait that the CloudWatch business logic provider must implement.
///
/// The handler receives a parsed operation enum, the raw body bytes,
/// and the wire protocol so it can parse input and serialize output
/// in the correct format.
pub trait CloudWatchHandler: Send + Sync + 'static {
    /// Handle a CloudWatch operation and produce an HTTP response.
    fn handle_operation(
        &self,
        op: CloudWatchOperation,
        body: Bytes,
        protocol: Protocol,
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
    protocol: Protocol,
) -> Result<http::Response<CloudWatchResponseBody>, CloudWatchError> {
    tracing::debug!(operation = %op, ?protocol, "dispatching CloudWatch operation");
    handler.handle_operation(op, body, protocol).await
}
