//! S3 operation dispatch: routes a resolved operation to the appropriate handler method.
//!
//! This module provides the [`dispatch_operation`] function that bridges the routing layer
//! with the business logic handler. Given a [`RoutingContext`] and HTTP request parts/body,
//! it:
//!
//! 1. Deserializes the HTTP request into the operation's typed Input struct (via
//!    [`crate::FromS3Request`])
//! 2. Calls the appropriate method on the [`S3Handler`] trait
//! 3. Serializes the Output struct into an HTTP response (via [`crate::IntoS3Response`])
//!
//! Phase 3 of the project will implement [`S3Handler`] on the `RustackS3` provider.
//! For now, all operations return `NotImplemented` by default.

use std::{future::Future, pin::Pin};

use bytes::Bytes;
use rustack_s3_model::{S3Operation, error::S3Error};

use crate::{body::S3ResponseBody, router::RoutingContext};

/// Trait that the business logic provider must implement.
///
/// This is the boundary between the HTTP layer and the S3 business logic.
/// In Phase 3, `RustackS3` will implement this trait by dispatching to
/// the appropriate operation handler.
///
/// # Object Safety
///
/// This trait uses `async-trait`-style boxing because it needs to be used
/// with `Arc<dyn S3Handler>` for dynamic dispatch in the service layer.
pub trait S3Handler: Send + Sync + 'static {
    /// Handle an S3 operation and produce an HTTP response.
    ///
    /// The implementor receives the identified operation, raw HTTP parts, and body,
    /// and must return a fully formed HTTP response.
    fn handle_operation(
        &self,
        op: S3Operation,
        parts: http::request::Parts,
        body: Bytes,
        ctx: RoutingContext,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<S3ResponseBody>, S3Error>> + Send>>;
}

/// Dispatch a routed S3 request to the handler.
///
/// This function is called by [`S3HttpService`](crate::service::S3HttpService) after
/// routing and optional authentication. It delegates to the [`S3Handler`] implementation.
pub async fn dispatch_operation<H: S3Handler>(
    handler: &H,
    parts: http::request::Parts,
    body: Bytes,
    ctx: RoutingContext,
) -> Result<http::Response<S3ResponseBody>, S3Error> {
    let op = ctx.operation;
    tracing::debug!(operation = %op, bucket = ?ctx.bucket, key = ?ctx.key, "dispatching S3 operation");
    handler.handle_operation(op, parts, body, ctx).await
}

/// A default handler implementation that returns `NotImplemented` for all operations.
///
/// Useful for testing the HTTP routing and request parsing layers in isolation.
#[derive(Debug, Clone, Default)]
pub struct NotImplementedHandler;

impl S3Handler for NotImplementedHandler {
    fn handle_operation(
        &self,
        op: S3Operation,
        _parts: http::request::Parts,
        _body: Bytes,
        _ctx: RoutingContext,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<S3ResponseBody>, S3Error>> + Send>> {
        Box::pin(async move { Err(S3Error::not_implemented(op.as_str())) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::RoutingContext;

    #[tokio::test]
    async fn test_should_return_not_implemented_for_default_handler() {
        let handler = NotImplementedHandler;
        let req = http::Request::builder()
            .method(http::Method::GET)
            .uri("/mybucket")
            .body(())
            .expect("valid request");
        let (parts, ()) = req.into_parts();
        let ctx = RoutingContext {
            bucket: Some("mybucket".to_owned()),
            key: None,
            operation: S3Operation::ListObjects,
            query_params: vec![],
        };

        let err = dispatch_operation(&handler, parts, Bytes::new(), ctx)
            .await
            .unwrap_err();
        assert_eq!(
            err.code,
            rustack_s3_model::error::S3ErrorCode::NotImplemented
        );
    }
}
