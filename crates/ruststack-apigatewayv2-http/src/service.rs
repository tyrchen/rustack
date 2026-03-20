//! API Gateway v2 HTTP service implementing the hyper `Service` trait.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use http_body_util::BodyExt;
use hyper::body::Incoming;

use bytes::Bytes;
use ruststack_apigatewayv2_model::error::ApiGatewayV2Error;

use crate::body::ApiGatewayV2ResponseBody;
use crate::dispatch::{ApiGatewayV2Handler, dispatch_operation};
use crate::response::{CONTENT_TYPE, error_to_response};
use crate::router::resolve_operation;

/// Configuration for the API Gateway v2 HTTP service.
#[derive(Clone)]
pub struct ApiGatewayV2HttpConfig {
    /// Whether to skip AWS signature validation.
    pub skip_signature_validation: bool,
    /// The AWS region this service is running in.
    pub region: String,
    /// Credential provider for signature validation.
    pub credential_provider: Option<Arc<dyn ruststack_auth::CredentialProvider>>,
}

impl std::fmt::Debug for ApiGatewayV2HttpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiGatewayV2HttpConfig")
            .field("skip_signature_validation", &self.skip_signature_validation)
            .field("region", &self.region)
            .field(
                "credential_provider",
                &self.credential_provider.as_ref().map(|_| "..."),
            )
            .finish()
    }
}

impl Default for ApiGatewayV2HttpConfig {
    fn default() -> Self {
        Self {
            skip_signature_validation: true,
            region: "us-east-1".to_owned(),
            credential_provider: None,
        }
    }
}

/// Hyper `Service` implementation for API Gateway v2.
///
/// Wraps an [`ApiGatewayV2Handler`] implementation and routes incoming HTTP
/// requests to the appropriate operation handler using restJson1 URL-based
/// routing.
#[derive(Debug)]
pub struct ApiGatewayV2HttpService<H: ApiGatewayV2Handler> {
    handler: Arc<H>,
    config: Arc<ApiGatewayV2HttpConfig>,
}

impl<H: ApiGatewayV2Handler> ApiGatewayV2HttpService<H> {
    /// Create a new `ApiGatewayV2HttpService`.
    pub fn new(handler: Arc<H>, config: ApiGatewayV2HttpConfig) -> Self {
        Self {
            handler,
            config: Arc::new(config),
        }
    }
}

impl<H: ApiGatewayV2Handler> Clone for ApiGatewayV2HttpService<H> {
    fn clone(&self) -> Self {
        Self {
            handler: Arc::clone(&self.handler),
            config: Arc::clone(&self.config),
        }
    }
}

impl<H: ApiGatewayV2Handler> hyper::service::Service<http::Request<Incoming>>
    for ApiGatewayV2HttpService<H>
{
    type Response = http::Response<ApiGatewayV2ResponseBody>;
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

/// Process a single API Gateway v2 HTTP request through the full pipeline.
async fn process_request<H: ApiGatewayV2Handler>(
    req: http::Request<Incoming>,
    handler: &H,
    config: &ApiGatewayV2HttpConfig,
    _request_id: &str,
) -> http::Response<ApiGatewayV2ResponseBody> {
    let (parts, incoming) = req.into_parts();

    // 1. Route: extract operation from method + path.
    let path = parts.uri.path();
    let (op, path_params, success_status) = match resolve_operation(&parts.method, path) {
        Ok(result) => result,
        Err(err) => return wrap_error_response(&err),
    };

    // 2. Extract query string.
    let query = parts.uri.query().unwrap_or("").to_owned();

    // 3. Collect body.
    let body = match collect_body(incoming).await {
        Ok(body) => body,
        Err(err) => return wrap_error_response(&err),
    };

    // 4. Authenticate (if enabled).
    if !config.skip_signature_validation {
        if let Some(ref cred_provider) = config.credential_provider {
            let body_hash = ruststack_auth::hash_payload(&body);
            if let Err(auth_err) =
                ruststack_auth::verify_sigv4(&parts, &body_hash, cred_provider.as_ref())
            {
                let err = ApiGatewayV2Error::with_message(
                    ruststack_apigatewayv2_model::error::ApiGatewayV2ErrorCode::AccessDeniedException,
                    auth_err.to_string(),
                );
                return wrap_error_response(&err);
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
        Err(err) => wrap_error_response(&err),
    }
}

/// Convert an `ApiGatewayV2Error` into an `ApiGatewayV2ResponseBody`-typed response.
///
/// Falls back to a plain-text 500 response if the error response itself
/// cannot be constructed (extremely unlikely).
fn wrap_error_response(error: &ApiGatewayV2Error) -> http::Response<ApiGatewayV2ResponseBody> {
    if let Ok(bytes_response) = error_to_response(error) {
        let (parts, body) = bytes_response.into_parts();
        http::Response::from_parts(parts, ApiGatewayV2ResponseBody::from_bytes(body))
    } else {
        // Fallback: if we cannot even serialize the error, return a minimal 500.
        let (parts, body) = http::Response::builder()
            .status(http::StatusCode::INTERNAL_SERVER_ERROR)
            .body(Bytes::from(r#"{"message":"Internal error"}"#))
            .unwrap_or_default()
            .into_parts();
        http::Response::from_parts(parts, ApiGatewayV2ResponseBody::from_bytes(body))
    }
}

/// Collect the incoming body into a single `Bytes` buffer.
async fn collect_body(incoming: Incoming) -> Result<Bytes, ApiGatewayV2Error> {
    incoming
        .collect()
        .await
        .map(http_body_util::Collected::to_bytes)
        .map_err(|e| ApiGatewayV2Error::internal_error(format!("Failed to read request body: {e}")))
}

/// Add common response headers to every API Gateway v2 response.
fn add_common_headers(
    mut response: http::Response<ApiGatewayV2ResponseBody>,
    request_id: &str,
) -> http::Response<ApiGatewayV2ResponseBody> {
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
