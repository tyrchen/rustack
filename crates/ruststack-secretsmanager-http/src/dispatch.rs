//! Secrets Manager handler trait and operation dispatch.

use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;

use ruststack_secretsmanager_model::error::SecretsManagerError;
use ruststack_secretsmanager_model::operations::SecretsManagerOperation;

use crate::body::SecretsManagerResponseBody;

/// Trait that the Secrets Manager business logic provider must implement.
///
/// The handler receives a parsed operation enum and the raw JSON body bytes,
/// and returns a complete HTTP response. This trait serves as the boundary
/// between the HTTP transport layer and the business logic layer.
pub trait SecretsManagerHandler: Send + Sync + 'static {
    /// Handle a Secrets Manager operation and produce an HTTP response.
    fn handle_operation(
        &self,
        op: SecretsManagerOperation,
        body: Bytes,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        http::Response<SecretsManagerResponseBody>,
                        SecretsManagerError,
                    >,
                > + Send,
        >,
    >;
}

/// Dispatch a Secrets Manager operation to the handler.
pub async fn dispatch_operation<H: SecretsManagerHandler>(
    handler: &H,
    op: SecretsManagerOperation,
    body: Bytes,
) -> Result<http::Response<SecretsManagerResponseBody>, SecretsManagerError> {
    tracing::debug!(operation = %op, "dispatching Secrets Manager operation");
    handler.handle_operation(op, body).await
}

/// Default handler that returns an error for all operations.
#[derive(Debug, Clone, Default)]
pub struct NotImplementedHandler;

impl SecretsManagerHandler for NotImplementedHandler {
    fn handle_operation(
        &self,
        op: SecretsManagerOperation,
        _body: Bytes,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        http::Response<SecretsManagerResponseBody>,
                        SecretsManagerError,
                    >,
                > + Send,
        >,
    > {
        Box::pin(async move { Err(SecretsManagerError::not_implemented(op.as_str())) })
    }
}
