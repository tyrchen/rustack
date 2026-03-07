//! Lambda HTTP service implementing the hyper `Service` trait.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::body::Incoming;

use ruststack_lambda_model::error::LambdaError;

use crate::body::LambdaResponseBody;
use crate::dispatch::{LambdaHandler, dispatch_operation};
use crate::response::{CONTENT_TYPE, error_to_response};
use crate::router::resolve_operation;

/// Configuration for the Lambda HTTP service.
#[derive(Clone)]
pub struct LambdaHttpConfig {
    /// Whether to skip AWS signature validation.
    pub skip_signature_validation: bool,
    /// The AWS region this service is running in.
    pub region: String,
    /// Credential provider for signature validation.
    pub credential_provider: Option<Arc<dyn ruststack_auth::CredentialProvider>>,
}

impl std::fmt::Debug for LambdaHttpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LambdaHttpConfig")
            .field("skip_signature_validation", &self.skip_signature_validation)
            .field("region", &self.region)
            .field(
                "credential_provider",
                &self.credential_provider.as_ref().map(|_| "..."),
            )
            .finish()
    }
}

impl Default for LambdaHttpConfig {
    fn default() -> Self {
        Self {
            skip_signature_validation: true,
            region: "us-east-1".to_owned(),
            credential_provider: None,
        }
    }
}

/// Hyper `Service` implementation for Lambda.
///
/// Wraps a [`LambdaHandler`] implementation and routes incoming HTTP
/// requests to the appropriate Lambda operation handler using restJson1
/// URL-based routing.
#[derive(Debug)]
pub struct LambdaHttpService<H: LambdaHandler> {
    handler: Arc<H>,
    config: Arc<LambdaHttpConfig>,
}

impl<H: LambdaHandler> LambdaHttpService<H> {
    /// Create a new `LambdaHttpService`.
    pub fn new(handler: Arc<H>, config: LambdaHttpConfig) -> Self {
        Self {
            handler,
            config: Arc::new(config),
        }
    }
}

impl<H: LambdaHandler> Clone for LambdaHttpService<H> {
    fn clone(&self) -> Self {
        Self {
            handler: Arc::clone(&self.handler),
            config: Arc::clone(&self.config),
        }
    }
}

impl<H: LambdaHandler> hyper::service::Service<http::Request<Incoming>> for LambdaHttpService<H> {
    type Response = http::Response<LambdaResponseBody>;
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

/// Process a single Lambda HTTP request through the full pipeline.
async fn process_request<H: LambdaHandler>(
    req: http::Request<Incoming>,
    handler: &H,
    config: &LambdaHttpConfig,
    request_id: &str,
) -> http::Response<LambdaResponseBody> {
    let (parts, incoming) = req.into_parts();

    // 1. Route: extract operation from method + path.
    let path = parts.uri.path();
    let (op, path_params, success_status) = match resolve_operation(&parts.method, path) {
        Ok(result) => result,
        Err(err) => return wrap_error_response(&err, request_id),
    };

    // 2. Extract query string.
    let query = parts.uri.query().unwrap_or("").to_owned();

    // 3. Collect body.
    let body = match collect_body(incoming).await {
        Ok(body) => body,
        Err(err) => return wrap_error_response(&err, request_id),
    };

    // 4. Authenticate (if enabled).
    if !config.skip_signature_validation {
        if let Some(ref cred_provider) = config.credential_provider {
            let body_hash = ruststack_auth::hash_payload(&body);
            if let Err(auth_err) =
                ruststack_auth::verify_sigv4(&parts, &body_hash, cred_provider.as_ref())
            {
                let err = LambdaError::new(
                    ruststack_lambda_model::error::LambdaErrorCode::InvalidRequestContentException,
                    auth_err.to_string(),
                );
                return wrap_error_response(&err, request_id);
            }
        }
    }

    // 5. Dispatch to handler.
    match dispatch_operation(handler, op, path_params, query, parts.headers, body).await {
        Ok(mut response) => {
            // Override status if the handler returned 200 but the route specifies differently.
            if response.status() == http::StatusCode::OK && success_status != 200 {
                *response.status_mut() =
                    http::StatusCode::from_u16(success_status).unwrap_or(http::StatusCode::OK);
            }
            response
        }
        Err(err) => wrap_error_response(&err, request_id),
    }
}

/// Convert a `LambdaError` into a `LambdaResponseBody`-typed response.
fn wrap_error_response(
    error: &LambdaError,
    request_id: &str,
) -> http::Response<LambdaResponseBody> {
    let bytes_response = error_to_response(error, request_id);
    let (parts, body) = bytes_response.into_parts();
    http::Response::from_parts(parts, LambdaResponseBody::from_bytes(body))
}

/// Collect the incoming body into a single `Bytes` buffer.
async fn collect_body(incoming: Incoming) -> Result<Bytes, LambdaError> {
    incoming
        .collect()
        .await
        .map(http_body_util::Collected::to_bytes)
        .map_err(|e| LambdaError::service_error(format!("Failed to read request body: {e}")))
}

/// Add common response headers to every Lambda response.
fn add_common_headers(
    mut response: http::Response<LambdaResponseBody>,
    request_id: &str,
) -> http::Response<LambdaResponseBody> {
    let is_no_content = response.status() == http::StatusCode::NO_CONTENT;
    let headers = response.headers_mut();

    if let Ok(hv) = http::HeaderValue::from_str(request_id) {
        headers.entry("x-amzn-requestid").or_insert(hv);
    }

    // Only set content-type for responses with a body (not 204 No Content).
    if !is_no_content {
        headers
            .entry("content-type")
            .or_insert(http::HeaderValue::from_static(CONTENT_TYPE));
    }

    headers.insert("server", http::HeaderValue::from_static("RustStack"));

    // CORS headers.
    headers.insert(
        "access-control-allow-origin",
        http::HeaderValue::from_static("*"),
    );

    response
}
