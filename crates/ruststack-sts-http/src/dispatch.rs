//! STS handler trait and operation dispatch.

use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;

use ruststack_sts_model::error::StsError;
use ruststack_sts_model::operations::StsOperation;

use crate::body::StsResponseBody;

/// Trait that the STS business logic provider must implement.
///
/// The handler receives a parsed operation enum, the raw form body bytes,
/// and the caller's access key extracted from the SigV4 Authorization header.
pub trait StsHandler: Send + Sync + 'static {
    /// Handle an STS operation and produce an HTTP response.
    fn handle_operation(
        &self,
        op: StsOperation,
        body: Bytes,
        caller_access_key: Option<String>,
        request_id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<StsResponseBody>, StsError>> + Send>>;
}

/// Dispatch an STS operation to the handler.
pub async fn dispatch_operation<H: StsHandler>(
    handler: &H,
    op: StsOperation,
    body: Bytes,
    caller_access_key: Option<String>,
    request_id: &str,
) -> Result<http::Response<StsResponseBody>, StsError> {
    tracing::debug!(operation = %op, "dispatching STS operation");
    handler
        .handle_operation(op, body, caller_access_key, request_id)
        .await
}
