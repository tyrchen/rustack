//! Lambda handler trait and operation dispatch.
//!
//! Uses `async_trait` because `LambdaHandler` requires object safety for
//! dynamic dispatch (`Arc<dyn LambdaHandler>`).

use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;

use ruststack_lambda_model::error::LambdaError;
use ruststack_lambda_model::operations::LambdaOperation;

use crate::body::LambdaResponseBody;
use crate::router::PathParams;

/// The boundary between HTTP and business logic for Lambda.
///
/// Implementations receive the resolved operation, extracted path parameters,
/// the raw query string, request headers, and body bytes. They return a
/// complete HTTP response or a [`LambdaError`].
///
/// This trait uses manual `Pin<Box<dyn Future>>` return types instead of
/// `async fn` in the trait because it needs to be object-safe for use with
/// `Arc<dyn LambdaHandler>`.
pub trait LambdaHandler: Send + Sync + 'static {
    /// Handle a Lambda API operation.
    fn handle_operation(
        &self,
        op: LambdaOperation,
        path_params: PathParams,
        query: String,
        headers: http::HeaderMap,
        body: Bytes,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<LambdaResponseBody>, LambdaError>> + Send>>;
}

/// Dispatch a Lambda operation to the handler.
pub async fn dispatch_operation<H: LambdaHandler>(
    handler: &H,
    op: LambdaOperation,
    path_params: PathParams,
    query: String,
    headers: http::HeaderMap,
    body: Bytes,
) -> Result<http::Response<LambdaResponseBody>, LambdaError> {
    tracing::debug!(operation = %op, "dispatching Lambda operation");
    handler
        .handle_operation(op, path_params, query, headers, body)
        .await
}

/// Default handler that returns a "not implemented" error for all operations.
#[derive(Debug, Clone, Default)]
pub struct NotImplementedHandler;

impl LambdaHandler for NotImplementedHandler {
    fn handle_operation(
        &self,
        op: LambdaOperation,
        _path_params: PathParams,
        _query: String,
        _headers: http::HeaderMap,
        _body: Bytes,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<LambdaResponseBody>, LambdaError>> + Send>>
    {
        Box::pin(async move {
            Err(LambdaError::service_error(format!(
                "operation {op} is not implemented",
            )))
        })
    }
}
