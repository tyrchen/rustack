//! STS HTTP service implementing the hyper `Service` trait.
//!
//! STS uses the `awsQuery` protocol where the request body is
//! `application/x-www-form-urlencoded` and the response is `text/xml`.
//! The `Action=` form parameter routes to the appropriate operation.

use std::{convert::Infallible, future::Future, pin::Pin, sync::Arc};

use bytes::Bytes;
use hyper::body::Incoming;
use rustack_sts_model::error::StsError;

use crate::{
    body::StsResponseBody,
    dispatch::{StsHandler, dispatch_operation},
    request::{extract_access_key_from_auth, parse_form_params},
    response::{CONTENT_TYPE, error_to_response},
    router::resolve_operation,
};

/// Configuration for the STS HTTP service.
#[derive(Clone)]
pub struct StsHttpConfig {
    /// Whether to skip AWS signature validation.
    pub skip_signature_validation: bool,
    /// The AWS region this service is running in.
    pub region: String,
    /// Credential provider for signature validation.
    pub credential_provider: Option<Arc<dyn rustack_auth::CredentialProvider>>,
}

impl std::fmt::Debug for StsHttpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StsHttpConfig")
            .field("skip_signature_validation", &self.skip_signature_validation)
            .field("region", &self.region)
            .field(
                "credential_provider",
                &self.credential_provider.as_ref().map(|_| "..."),
            )
            .finish()
    }
}

impl Default for StsHttpConfig {
    fn default() -> Self {
        Self {
            skip_signature_validation: true,
            region: "us-east-1".to_owned(),
            credential_provider: None,
        }
    }
}

/// Hyper `Service` implementation for STS.
///
/// Wraps an [`StsHandler`] implementation and routes incoming HTTP
/// requests to the appropriate STS operation handler.
#[derive(Debug)]
pub struct StsHttpService<H: StsHandler> {
    handler: Arc<H>,
    config: Arc<StsHttpConfig>,
}

impl<H: StsHandler> StsHttpService<H> {
    /// Create a new `StsHttpService`.
    pub fn new(handler: Arc<H>, config: StsHttpConfig) -> Self {
        Self {
            handler,
            config: Arc::new(config),
        }
    }
}

impl<H: StsHandler> Clone for StsHttpService<H> {
    fn clone(&self) -> Self {
        Self {
            handler: Arc::clone(&self.handler),
            config: Arc::clone(&self.config),
        }
    }
}

impl<H: StsHandler> hyper::service::Service<http::Request<Incoming>> for StsHttpService<H> {
    type Response = http::Response<StsResponseBody>;
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

/// Process a single STS HTTP request through the full pipeline.
///
/// Pipeline:
/// 1. Verify POST method
/// 2. Collect body
/// 3. Parse form params from body
/// 4. Resolve operation from `Action=` param
/// 5. Extract caller access key from Authorization header
/// 6. Authenticate (if enabled)
/// 7. Dispatch to handler
async fn process_request<H: StsHandler>(
    req: http::Request<Incoming>,
    handler: &H,
    config: &StsHttpConfig,
    request_id: &str,
) -> http::Response<StsResponseBody> {
    let (parts, incoming) = req.into_parts();

    // 1. Verify POST method.
    if parts.method != http::Method::POST {
        let err =
            StsError::invalid_action(format!("STS requires POST method, got {}", parts.method));
        return error_to_response(&err, request_id);
    }

    // 2. Collect body.
    let body = match collect_body(incoming).await {
        Ok(body) => body,
        Err(err) => return error_to_response(&err, request_id),
    };

    // 3. Parse form params.
    let params = parse_form_params(&body);

    // 4. Resolve operation.
    let op = match resolve_operation(&params) {
        Ok(op) => op,
        Err(err) => return error_to_response(&err, request_id),
    };

    // 5. Extract caller access key from Authorization header.
    let caller_access_key = parts
        .headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(extract_access_key_from_auth);

    // 6. Authenticate (if enabled).
    if let Err(auth_err) = rustack_auth::AuthMode::resolve(
        config.skip_signature_validation,
        config.credential_provider.as_deref(),
    )
    .and_then(|mode| mode.verify(&parts, &rustack_auth::hash_payload(&body)))
    {
        let err = StsError::invalid_client_token_id(auth_err.to_string());
        return error_to_response(&err, request_id);
    }

    // 7. Dispatch to handler.
    match dispatch_operation(handler, op, body, caller_access_key, request_id).await {
        Ok(response) => response,
        Err(err) => error_to_response(&err, request_id),
    }
}

/// Collect the incoming body into a single `Bytes` buffer.
async fn collect_body(incoming: Incoming) -> Result<Bytes, StsError> {
    rustack_core::http::collect_body(incoming, rustack_core::http::BodyBudget::control())
        .await
        .map_err(|e| {
            let mut err = StsError::internal_error(e.to_string());
            err.status_code = e.status_code();
            err
        })
}

/// Add common response headers to every STS response.
fn add_common_headers(
    mut response: http::Response<StsResponseBody>,
    request_id: &str,
) -> http::Response<StsResponseBody> {
    let headers = response.headers_mut();

    if let Ok(hv) = http::HeaderValue::from_str(request_id) {
        headers.entry("x-amzn-requestid").or_insert(hv);
    }

    headers
        .entry("content-type")
        .or_insert(http::HeaderValue::from_static(CONTENT_TYPE));

    headers.insert("server", http::HeaderValue::from_static("Rustack"));

    // CORS headers.
    headers.insert(
        "access-control-allow-origin",
        http::HeaderValue::from_static("*"),
    );

    response
}
