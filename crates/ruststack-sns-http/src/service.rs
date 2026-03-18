//! SNS HTTP service implementing the hyper `Service` trait.
//!
//! SNS uses the `awsQuery` protocol where the request body is
//! `application/x-www-form-urlencoded` and the response is `text/xml`.
//! Unlike SQS/SSM (which use `X-Amz-Target` header), SNS uses the
//! `Action=` form parameter for operation routing.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::body::Incoming;

use ruststack_sns_model::error::SnsError;

use crate::body::SnsResponseBody;
use crate::dispatch::{SnsHandler, dispatch_operation};
use crate::request::parse_form_params;
use crate::response::{CONTENT_TYPE, error_to_response};
use crate::router::resolve_operation;

/// Configuration for the SNS HTTP service.
#[derive(Clone)]
pub struct SnsHttpConfig {
    /// Whether to skip AWS signature validation.
    pub skip_signature_validation: bool,
    /// The AWS region this service is running in.
    pub region: String,
    /// Credential provider for signature validation.
    pub credential_provider: Option<Arc<dyn ruststack_auth::CredentialProvider>>,
}

impl std::fmt::Debug for SnsHttpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SnsHttpConfig")
            .field("skip_signature_validation", &self.skip_signature_validation)
            .field("region", &self.region)
            .field(
                "credential_provider",
                &self.credential_provider.as_ref().map(|_| "..."),
            )
            .finish()
    }
}

impl Default for SnsHttpConfig {
    fn default() -> Self {
        Self {
            skip_signature_validation: true,
            region: "us-east-1".to_owned(),
            credential_provider: None,
        }
    }
}

/// Hyper `Service` implementation for SNS.
///
/// Wraps an [`SnsHandler`] implementation and routes incoming HTTP
/// requests to the appropriate SNS operation handler.
#[derive(Debug)]
pub struct SnsHttpService<H: SnsHandler> {
    handler: Arc<H>,
    config: Arc<SnsHttpConfig>,
}

impl<H: SnsHandler> SnsHttpService<H> {
    /// Create a new `SnsHttpService`.
    pub fn new(handler: Arc<H>, config: SnsHttpConfig) -> Self {
        Self {
            handler,
            config: Arc::new(config),
        }
    }
}

impl<H: SnsHandler> Clone for SnsHttpService<H> {
    fn clone(&self) -> Self {
        Self {
            handler: Arc::clone(&self.handler),
            config: Arc::clone(&self.config),
        }
    }
}

impl<H: SnsHandler> hyper::service::Service<http::Request<Incoming>> for SnsHttpService<H> {
    type Response = http::Response<SnsResponseBody>;
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

/// Process a single SNS HTTP request through the full pipeline.
///
/// Pipeline:
/// 1. Verify POST method (SNS only accepts POST)
/// 2. Collect body
/// 3. Parse form params from body
/// 4. Resolve operation from `Action=` param
/// 5. Authenticate (if enabled)
/// 6. Dispatch to handler (pass raw body bytes)
async fn process_request<H: SnsHandler>(
    req: http::Request<Incoming>,
    handler: &H,
    config: &SnsHttpConfig,
    request_id: &str,
) -> http::Response<SnsResponseBody> {
    let (parts, incoming) = req.into_parts();

    // 1. Verify POST method (SNS only accepts POST).
    if parts.method != http::Method::POST {
        let err =
            SnsError::invalid_parameter(format!("SNS requires POST method, got {}", parts.method));
        return error_to_response(&err, request_id);
    }

    // 2. Collect body.
    let body = match collect_body(incoming).await {
        Ok(body) => body,
        Err(err) => return error_to_response(&err, request_id),
    };

    // 3. Parse form params to extract Action for routing.
    let params = parse_form_params(&body);

    // 4. Resolve operation from Action= param.
    let op = match resolve_operation(&params) {
        Ok(op) => op,
        Err(err) => return error_to_response(&err, request_id),
    };

    // 5. Authenticate (if enabled).
    if !config.skip_signature_validation {
        if let Some(ref cred_provider) = config.credential_provider {
            let body_hash = ruststack_auth::hash_payload(&body);
            if let Err(auth_err) =
                ruststack_auth::verify_sigv4(&parts, &body_hash, cred_provider.as_ref())
            {
                let err = SnsError::invalid_security(auth_err.to_string());
                return error_to_response(&err, request_id);
            }
        }
    }

    // 6. Dispatch to handler (pass raw body so handler can re-parse as needed).
    match dispatch_operation(handler, op, body).await {
        Ok(response) => response,
        Err(err) => error_to_response(&err, request_id),
    }
}

/// Collect the incoming body into a single `Bytes` buffer.
async fn collect_body(incoming: Incoming) -> Result<Bytes, SnsError> {
    incoming
        .collect()
        .await
        .map(http_body_util::Collected::to_bytes)
        .map_err(|e| SnsError::internal_error(format!("Failed to read request body: {e}")))
}

/// Add common response headers to every SNS response.
fn add_common_headers(
    mut response: http::Response<SnsResponseBody>,
    request_id: &str,
) -> http::Response<SnsResponseBody> {
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
