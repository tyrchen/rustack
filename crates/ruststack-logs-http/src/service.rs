//! CloudWatch Logs HTTP service implementing the hyper `Service` trait.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::body::Incoming;

use ruststack_logs_model::error::LogsError;

use crate::body::LogsResponseBody;
use crate::dispatch::{LogsHandler, dispatch_operation};
use crate::response::{CONTENT_TYPE, error_to_response};
use crate::router::resolve_operation;

/// Configuration for the CloudWatch Logs HTTP service.
#[derive(Clone)]
pub struct LogsHttpConfig {
    /// Whether to skip AWS signature validation.
    pub skip_signature_validation: bool,
    /// The AWS region this service is running in.
    pub region: String,
    /// Credential provider for signature validation.
    pub credential_provider: Option<Arc<dyn ruststack_auth::CredentialProvider>>,
}

impl std::fmt::Debug for LogsHttpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogsHttpConfig")
            .field("skip_signature_validation", &self.skip_signature_validation)
            .field("region", &self.region)
            .field(
                "credential_provider",
                &self.credential_provider.as_ref().map(|_| "..."),
            )
            .finish()
    }
}

impl Default for LogsHttpConfig {
    fn default() -> Self {
        Self {
            skip_signature_validation: true,
            region: "us-east-1".to_owned(),
            credential_provider: None,
        }
    }
}

/// Hyper `Service` implementation for CloudWatch Logs.
///
/// Wraps a [`LogsHandler`] implementation and routes incoming HTTP
/// requests to the appropriate CloudWatch Logs operation handler.
#[derive(Debug)]
pub struct LogsHttpService<H: LogsHandler> {
    handler: Arc<H>,
    config: Arc<LogsHttpConfig>,
}

impl<H: LogsHandler> LogsHttpService<H> {
    /// Create a new `LogsHttpService`.
    pub fn new(handler: Arc<H>, config: LogsHttpConfig) -> Self {
        Self {
            handler,
            config: Arc::new(config),
        }
    }
}

impl<H: LogsHandler> Clone for LogsHttpService<H> {
    fn clone(&self) -> Self {
        Self {
            handler: Arc::clone(&self.handler),
            config: Arc::clone(&self.config),
        }
    }
}

impl<H: LogsHandler> hyper::service::Service<http::Request<Incoming>> for LogsHttpService<H> {
    type Response = http::Response<LogsResponseBody>;
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

/// Process a single CloudWatch Logs HTTP request through the full pipeline.
async fn process_request<H: LogsHandler>(
    req: http::Request<Incoming>,
    handler: &H,
    config: &LogsHttpConfig,
    request_id: &str,
) -> http::Response<LogsResponseBody> {
    let (parts, incoming) = req.into_parts();

    // 1. Verify POST method (CloudWatch Logs only accepts POST).
    if parts.method != http::Method::POST {
        let err = LogsError::validation(format!(
            "CloudWatch Logs requires POST method, got {}",
            parts.method,
        ));
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
                let err = LogsError::with_message(
                    ruststack_logs_model::error::LogsErrorCode::ValidationException,
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
async fn collect_body(incoming: Incoming) -> Result<Bytes, LogsError> {
    incoming
        .collect()
        .await
        .map(http_body_util::Collected::to_bytes)
        .map_err(|e| LogsError::internal_error(format!("Failed to read request body: {e}")))
}

/// Add common response headers to every CloudWatch Logs response.
fn add_common_headers(
    mut response: http::Response<LogsResponseBody>,
    request_id: &str,
) -> http::Response<LogsResponseBody> {
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
