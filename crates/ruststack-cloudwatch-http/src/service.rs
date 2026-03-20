//! CloudWatch HTTP service implementing the hyper `Service` trait.
//!
//! CloudWatch Metrics uses the `awsQuery` protocol where the request body is
//! `application/x-www-form-urlencoded` and the response is `text/xml`.
//! The `Action=` form parameter determines the operation.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::body::Incoming;

use ruststack_cloudwatch_model::error::CloudWatchError;

use crate::body::CloudWatchResponseBody;
use crate::dispatch::{CloudWatchHandler, dispatch_operation};
use crate::request::parse_form_params;
use crate::response::{CONTENT_TYPE, error_to_response};
use crate::router::resolve_operation;

/// Configuration for the CloudWatch HTTP service.
#[derive(Clone)]
pub struct CloudWatchHttpConfig {
    /// Whether to skip AWS signature validation.
    pub skip_signature_validation: bool,
    /// The AWS region this service is running in.
    pub region: String,
    /// Credential provider for signature validation.
    pub credential_provider: Option<Arc<dyn ruststack_auth::CredentialProvider>>,
}

impl std::fmt::Debug for CloudWatchHttpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudWatchHttpConfig")
            .field("skip_signature_validation", &self.skip_signature_validation)
            .field("region", &self.region)
            .field(
                "credential_provider",
                &self.credential_provider.as_ref().map(|_| "..."),
            )
            .finish()
    }
}

impl Default for CloudWatchHttpConfig {
    fn default() -> Self {
        Self {
            skip_signature_validation: true,
            region: "us-east-1".to_owned(),
            credential_provider: None,
        }
    }
}

/// Hyper `Service` implementation for CloudWatch Metrics.
///
/// Wraps a [`CloudWatchHandler`] implementation and routes incoming HTTP
/// requests to the appropriate CloudWatch operation handler.
#[derive(Debug)]
pub struct CloudWatchHttpService<H: CloudWatchHandler> {
    handler: Arc<H>,
    config: Arc<CloudWatchHttpConfig>,
}

impl<H: CloudWatchHandler> CloudWatchHttpService<H> {
    /// Create a new `CloudWatchHttpService`.
    pub fn new(handler: Arc<H>, config: CloudWatchHttpConfig) -> Self {
        Self {
            handler,
            config: Arc::new(config),
        }
    }
}

impl<H: CloudWatchHandler> Clone for CloudWatchHttpService<H> {
    fn clone(&self) -> Self {
        Self {
            handler: Arc::clone(&self.handler),
            config: Arc::clone(&self.config),
        }
    }
}

impl<H: CloudWatchHandler> hyper::service::Service<http::Request<Incoming>>
    for CloudWatchHttpService<H>
{
    type Response = http::Response<CloudWatchResponseBody>;
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

/// Process a single CloudWatch HTTP request through the full pipeline.
async fn process_request<H: CloudWatchHandler>(
    req: http::Request<Incoming>,
    handler: &H,
    config: &CloudWatchHttpConfig,
    request_id: &str,
) -> http::Response<CloudWatchResponseBody> {
    let (parts, incoming) = req.into_parts();

    // Verify POST method.
    if parts.method != http::Method::POST {
        let err = CloudWatchError::with_message(
            ruststack_cloudwatch_model::error::CloudWatchErrorCode::InvalidParameterValueException,
            format!("CloudWatch requires POST method, got {}", parts.method),
        );
        return error_to_response(&err, request_id);
    }

    // Collect body.
    let body = match collect_body(incoming).await {
        Ok(body) => body,
        Err(err) => return error_to_response(&err, request_id),
    };

    // Parse form params to extract Action for routing.
    let params = parse_form_params(&body);

    // Resolve operation from Action= param.
    let op = match resolve_operation(&params) {
        Ok(op) => op,
        Err(err) => return error_to_response(&err, request_id),
    };

    // Authenticate (if enabled).
    if !config.skip_signature_validation {
        if let Some(ref cred_provider) = config.credential_provider {
            let body_hash = ruststack_auth::hash_payload(&body);
            if let Err(auth_err) =
                ruststack_auth::verify_sigv4(&parts, &body_hash, cred_provider.as_ref())
            {
                let err = CloudWatchError::with_message(
                    ruststack_cloudwatch_model::error::CloudWatchErrorCode::InternalServiceFault,
                    auth_err.to_string(),
                );
                return error_to_response(&err, request_id);
            }
        }
    }

    // Dispatch to handler.
    match dispatch_operation(handler, op, body).await {
        Ok(response) => response,
        Err(err) => error_to_response(&err, request_id),
    }
}

/// Collect the incoming body into a single `Bytes` buffer.
async fn collect_body(incoming: Incoming) -> Result<Bytes, CloudWatchError> {
    incoming
        .collect()
        .await
        .map(http_body_util::Collected::to_bytes)
        .map_err(|e| CloudWatchError::internal_error(format!("Failed to read request body: {e}")))
}

/// Add common response headers to every CloudWatch response.
fn add_common_headers(
    mut response: http::Response<CloudWatchResponseBody>,
    request_id: &str,
) -> http::Response<CloudWatchResponseBody> {
    let headers = response.headers_mut();

    if let Ok(hv) = http::HeaderValue::from_str(request_id) {
        headers.entry("x-amzn-requestid").or_insert(hv);
    }

    headers
        .entry("content-type")
        .or_insert(http::HeaderValue::from_static(CONTENT_TYPE));

    headers.insert("server", http::HeaderValue::from_static("RustStack"));

    headers.insert(
        "access-control-allow-origin",
        http::HeaderValue::from_static("*"),
    );

    response
}
