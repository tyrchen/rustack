//! SES HTTP service implementing the hyper `Service` trait.
//!
//! SES v1 uses the `awsQuery` protocol (form-urlencoded request, XML response).
//! SES v2 uses `restJson1` (JSON request/response, path-based routing).

use std::{convert::Infallible, future::Future, pin::Pin, sync::Arc};

use bytes::Bytes;
use hyper::body::Incoming;
use rustack_ses_model::error::SesError;

use crate::{
    body::SesResponseBody,
    dispatch::{SesHandler, dispatch_operation},
    request::parse_form_params,
    response::{XML_CONTENT_TYPE, error_to_response},
    router::resolve_operation,
};

/// Configuration for the SES HTTP service.
#[derive(Clone)]
pub struct SesHttpConfig {
    /// Whether to skip AWS signature validation.
    pub skip_signature_validation: bool,
    /// The AWS region this service is running in.
    pub region: String,
    /// Credential provider for signature validation.
    pub credential_provider: Option<Arc<dyn rustack_auth::CredentialProvider>>,
}

impl std::fmt::Debug for SesHttpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SesHttpConfig")
            .field("skip_signature_validation", &self.skip_signature_validation)
            .field("region", &self.region)
            .field(
                "credential_provider",
                &self.credential_provider.as_ref().map(|_| "..."),
            )
            .finish()
    }
}

impl Default for SesHttpConfig {
    fn default() -> Self {
        Self {
            skip_signature_validation: true,
            region: "us-east-1".to_owned(),
            credential_provider: None,
        }
    }
}

/// Hyper `Service` implementation for SES v1 (awsQuery).
#[derive(Debug)]
pub struct SesHttpService<H: SesHandler> {
    handler: Arc<H>,
    config: Arc<SesHttpConfig>,
}

impl<H: SesHandler> SesHttpService<H> {
    /// Create a new `SesHttpService`.
    pub fn new(handler: Arc<H>, config: SesHttpConfig) -> Self {
        Self {
            handler,
            config: Arc::new(config),
        }
    }
}

impl<H: SesHandler> Clone for SesHttpService<H> {
    fn clone(&self) -> Self {
        Self {
            handler: Arc::clone(&self.handler),
            config: Arc::clone(&self.config),
        }
    }
}

impl<H: SesHandler> hyper::service::Service<http::Request<Incoming>> for SesHttpService<H> {
    type Response = http::Response<SesResponseBody>;
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

/// Process a single SES v1 HTTP request through the full pipeline.
///
/// Pipeline:
/// 1. Verify POST method (SES v1 only accepts POST)
/// 2. Collect body
/// 3. Parse form params from body
/// 4. Resolve operation from `Action=` param
/// 5. Authenticate (if enabled)
/// 6. Dispatch to handler (pass raw body bytes)
async fn process_request<H: SesHandler>(
    req: http::Request<Incoming>,
    handler: &H,
    config: &SesHttpConfig,
    request_id: &str,
) -> http::Response<SesResponseBody> {
    let (parts, incoming) = req.into_parts();

    // 1. Verify POST method (SES v1 only accepts POST).
    if parts.method != http::Method::POST {
        let err = SesError::invalid_parameter_value(format!(
            "SES requires POST method, got {}",
            parts.method
        ));
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
    if let Err(auth_err) = rustack_auth::AuthMode::resolve(
        config.skip_signature_validation,
        config.credential_provider.as_deref(),
    )
    .and_then(|mode| mode.verify(&parts, &rustack_auth::hash_payload(&body)))
    {
        let mut err = SesError::internal_error(auth_err.to_string());
        err.status_code = http::StatusCode::FORBIDDEN;
        return error_to_response(&err, request_id);
    }

    // 6. Dispatch to handler (pass raw body so handler can re-parse as needed).
    match dispatch_operation(handler, op, body).await {
        Ok(response) => response,
        Err(err) => error_to_response(&err, request_id),
    }
}

/// Collect the incoming body into a single `Bytes` buffer.
async fn collect_body(incoming: Incoming) -> Result<Bytes, SesError> {
    rustack_core::http::collect_body(incoming, rustack_core::http::BodyBudget::control())
        .await
        .map_err(|e| {
            let mut err = SesError::internal_error(e.to_string());
            err.status_code = e.status_code();
            err
        })
}

/// Add common response headers to every SES response.
fn add_common_headers(
    mut response: http::Response<SesResponseBody>,
    request_id: &str,
) -> http::Response<SesResponseBody> {
    let headers = response.headers_mut();

    if let Ok(hv) = http::HeaderValue::from_str(request_id) {
        headers.entry("x-amzn-requestid").or_insert(hv);
    }

    headers
        .entry("content-type")
        .or_insert(http::HeaderValue::from_static(XML_CONTENT_TYPE));

    headers.insert("server", http::HeaderValue::from_static("Rustack"));

    // CORS headers.
    headers.insert(
        "access-control-allow-origin",
        http::HeaderValue::from_static("*"),
    );

    response
}
