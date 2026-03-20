//! API Gateway v2 handler trait and operation dispatch.
//!
//! Uses manual `Pin<Box<dyn Future>>` return types because `ApiGatewayV2Handler`
//! requires object safety for dynamic dispatch (`Arc<dyn ApiGatewayV2Handler>`).

use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;

use ruststack_apigatewayv2_model::error::ApiGatewayV2Error;
use ruststack_apigatewayv2_model::operations::ApiGatewayV2Operation;

use crate::body::ApiGatewayV2ResponseBody;
use crate::router::PathParams;

/// The boundary between HTTP and business logic for API Gateway v2.
///
/// Implementations receive the resolved operation, extracted path parameters,
/// the raw query string, request headers, and body bytes. They return a
/// complete HTTP response or an [`ApiGatewayV2Error`].
///
/// This trait uses manual `Pin<Box<dyn Future>>` return types instead of
/// `async fn` in the trait because it needs to be object-safe for use with
/// `Arc<dyn ApiGatewayV2Handler>`.
pub trait ApiGatewayV2Handler: Send + Sync + 'static {
    /// Handle an API Gateway v2 operation.
    fn handle_operation(
        &self,
        op: ApiGatewayV2Operation,
        path_params: PathParams,
        query: String,
        headers: http::HeaderMap,
        body: Bytes,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<http::Response<ApiGatewayV2ResponseBody>, ApiGatewayV2Error>>
                + Send,
        >,
    >;
}

/// Dispatch an API Gateway v2 operation to the handler.
pub async fn dispatch_operation<H: ApiGatewayV2Handler>(
    handler: &H,
    op: ApiGatewayV2Operation,
    path_params: PathParams,
    query: String,
    headers: http::HeaderMap,
    body: Bytes,
) -> Result<http::Response<ApiGatewayV2ResponseBody>, ApiGatewayV2Error> {
    tracing::debug!(operation = %op, "dispatching ApiGatewayV2 operation");
    handler
        .handle_operation(op, path_params, query, headers, body)
        .await
}

/// Default handler that returns a "not implemented" error for all operations.
#[derive(Debug, Clone, Default)]
pub struct NotImplementedHandler;

impl ApiGatewayV2Handler for NotImplementedHandler {
    fn handle_operation(
        &self,
        op: ApiGatewayV2Operation,
        _path_params: PathParams,
        _query: String,
        _headers: http::HeaderMap,
        _body: Bytes,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<http::Response<ApiGatewayV2ResponseBody>, ApiGatewayV2Error>>
                + Send,
        >,
    > {
        Box::pin(async move { Err(ApiGatewayV2Error::not_implemented(op.as_str())) })
    }
}
