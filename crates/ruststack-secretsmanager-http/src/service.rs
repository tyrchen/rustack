//! Secrets Manager HTTP service implementing the hyper `Service` trait.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::body::Incoming;

use ruststack_secretsmanager_model::error::{SecretsManagerError, SecretsManagerErrorCode};

use crate::body::SecretsManagerResponseBody;
use crate::dispatch::{SecretsManagerHandler, dispatch_operation};
use crate::response::{CONTENT_TYPE, error_to_response};
use crate::router::resolve_operation;

/// Configuration for the Secrets Manager HTTP service.
#[derive(Clone)]
pub struct SecretsManagerHttpConfig {
    /// Whether to skip AWS signature validation.
    pub skip_signature_validation: bool,
    /// The AWS region this service is running in.
    pub region: String,
    /// Credential provider for signature validation.
    pub credential_provider: Option<Arc<dyn ruststack_auth::CredentialProvider>>,
}

impl std::fmt::Debug for SecretsManagerHttpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretsManagerHttpConfig")
            .field("skip_signature_validation", &self.skip_signature_validation)
            .field("region", &self.region)
            .field(
                "credential_provider",
                &self.credential_provider.as_ref().map(|_| "..."),
            )
            .finish()
    }
}

impl Default for SecretsManagerHttpConfig {
    fn default() -> Self {
        Self {
            skip_signature_validation: true,
            region: "us-east-1".to_owned(),
            credential_provider: None,
        }
    }
}

/// Hyper `Service` implementation for Secrets Manager.
///
/// Wraps a [`SecretsManagerHandler`] implementation and routes incoming HTTP
/// requests to the appropriate Secrets Manager operation handler.
#[derive(Debug)]
pub struct SecretsManagerHttpService<H: SecretsManagerHandler> {
    handler: Arc<H>,
    config: Arc<SecretsManagerHttpConfig>,
}

impl<H: SecretsManagerHandler> SecretsManagerHttpService<H> {
    /// Create a new `SecretsManagerHttpService`.
    pub fn new(handler: Arc<H>, config: SecretsManagerHttpConfig) -> Self {
        Self {
            handler,
            config: Arc::new(config),
        }
    }
}

impl<H: SecretsManagerHandler> Clone for SecretsManagerHttpService<H> {
    fn clone(&self) -> Self {
        Self {
            handler: Arc::clone(&self.handler),
            config: Arc::clone(&self.config),
        }
    }
}

impl<H: SecretsManagerHandler> hyper::service::Service<http::Request<Incoming>>
    for SecretsManagerHttpService<H>
{
    type Response = http::Response<SecretsManagerResponseBody>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, req: http::Request<Incoming>) -> Self::Future {
        let handler = Arc::clone(&self.handler);
        let config = Arc::clone(&self.config);
        let request_id = uuid::Uuid::new_v4().to_string();

        Box::pin(async move {
            let response = process_request(req, handler.as_ref(), &config, &request_id).await;
            let response = add_common_headers(response, &request_id);
            Ok(response)
        })
    }
}

/// Process a single Secrets Manager HTTP request through the full pipeline.
async fn process_request<H: SecretsManagerHandler>(
    req: http::Request<Incoming>,
    handler: &H,
    config: &SecretsManagerHttpConfig,
    request_id: &str,
) -> http::Response<SecretsManagerResponseBody> {
    let (parts, incoming) = req.into_parts();

    // 1. Verify POST method (Secrets Manager only accepts POST).
    if parts.method != http::Method::POST {
        let err = SecretsManagerError::with_message(
            SecretsManagerErrorCode::InvalidRequestException,
            format!("Secrets Manager requires POST method, got {}", parts.method),
        );
        return error_to_response(&err, request_id);
    }

    // 2. Route: extract operation from X-Amz-Target header.
    let op = match resolve_operation(&parts.headers) {
        Ok(op) => op,
        Err(err) => return error_to_response(&err, request_id),
    };

    // 3. Collect body.
    let body = match collect_body(incoming).await {
        Ok(body) => body,
        Err(err) => return error_to_response(&err, request_id),
    };

    // 4. Authenticate (if enabled).
    if !config.skip_signature_validation {
        if let Some(ref cred_provider) = config.credential_provider {
            let body_hash = ruststack_auth::hash_payload(&body);
            if let Err(auth_err) =
                ruststack_auth::verify_sigv4(&parts, &body_hash, cred_provider.as_ref())
            {
                let err = SecretsManagerError::with_message(
                    SecretsManagerErrorCode::InvalidRequestException,
                    auth_err.to_string(),
                );
                return error_to_response(&err, request_id);
            }
        }
    }

    // 5. Dispatch to handler.
    match dispatch_operation(handler, op, body).await {
        Ok(response) => response,
        Err(err) => error_to_response(&err, request_id),
    }
}

/// Collect the incoming body into a single `Bytes` buffer.
async fn collect_body(incoming: Incoming) -> Result<Bytes, SecretsManagerError> {
    incoming
        .collect()
        .await
        .map(http_body_util::Collected::to_bytes)
        .map_err(|e| {
            SecretsManagerError::internal_error(format!("Failed to read request body: {e}"))
        })
}

/// Add common response headers to every Secrets Manager response.
fn add_common_headers(
    mut response: http::Response<SecretsManagerResponseBody>,
    request_id: &str,
) -> http::Response<SecretsManagerResponseBody> {
    let headers = response.headers_mut();

    if let Ok(hv) = http::HeaderValue::from_str(request_id) {
        headers.entry("x-amzn-requestid").or_insert(hv);
    }

    headers
        .entry("content-type")
        .or_insert(http::HeaderValue::from_static(CONTENT_TYPE));

    headers.insert("server", http::HeaderValue::from_static("RustStack"));

    // CORS headers.
    headers.insert(
        "access-control-allow-origin",
        http::HeaderValue::from_static("*"),
    );

    response
}
