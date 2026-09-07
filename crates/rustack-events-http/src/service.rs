//! EventBridge HTTP service implementing the hyper `Service` trait.

use std::{convert::Infallible, future::Future, pin::Pin, sync::Arc};

use bytes::Bytes;
use hyper::body::Incoming;
use rustack_events_model::error::EventsError;

use crate::{
    body::EventsResponseBody,
    dispatch::{EventsHandler, dispatch_operation},
    response::{CONTENT_TYPE, error_to_response},
    router::resolve_operation,
};

/// Configuration for the EventBridge HTTP service.
#[derive(Clone)]
pub struct EventsHttpConfig {
    /// Whether to skip AWS signature validation.
    pub skip_signature_validation: bool,
    /// The AWS region this service is running in.
    pub region: String,
    /// Credential provider for signature validation.
    pub credential_provider: Option<Arc<dyn rustack_auth::CredentialProvider>>,
}

impl std::fmt::Debug for EventsHttpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventsHttpConfig")
            .field("skip_signature_validation", &self.skip_signature_validation)
            .field("region", &self.region)
            .field(
                "credential_provider",
                &self.credential_provider.as_ref().map(|_| "..."),
            )
            .finish()
    }
}

impl Default for EventsHttpConfig {
    fn default() -> Self {
        Self {
            skip_signature_validation: true,
            region: "us-east-1".to_owned(),
            credential_provider: None,
        }
    }
}

/// Hyper `Service` implementation for EventBridge.
///
/// Wraps an [`EventsHandler`] implementation and routes incoming HTTP
/// requests to the appropriate EventBridge operation handler.
#[derive(Debug)]
pub struct EventsHttpService<H: EventsHandler> {
    handler: Arc<H>,
    config: Arc<EventsHttpConfig>,
}

impl<H: EventsHandler> EventsHttpService<H> {
    /// Create a new `EventsHttpService`.
    pub fn new(handler: Arc<H>, config: EventsHttpConfig) -> Self {
        Self {
            handler,
            config: Arc::new(config),
        }
    }
}

impl<H: EventsHandler> Clone for EventsHttpService<H> {
    fn clone(&self) -> Self {
        Self {
            handler: Arc::clone(&self.handler),
            config: Arc::clone(&self.config),
        }
    }
}

impl<H: EventsHandler> hyper::service::Service<http::Request<Incoming>> for EventsHttpService<H> {
    type Response = http::Response<EventsResponseBody>;
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

/// Process a single EventBridge HTTP request through the full pipeline.
async fn process_request<H: EventsHandler>(
    req: http::Request<Incoming>,
    handler: &H,
    config: &EventsHttpConfig,
    request_id: &str,
) -> http::Response<EventsResponseBody> {
    let (parts, incoming) = req.into_parts();

    // 1. Verify POST method (EventBridge only accepts POST).
    if parts.method != http::Method::POST {
        let err = EventsError::validation(format!(
            "EventBridge requires POST method, got {}",
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
    if let Err(auth_err) = rustack_auth::AuthMode::resolve(
        config.skip_signature_validation,
        config.credential_provider.as_deref(),
    )
    .and_then(|mode| mode.verify(&parts, &rustack_auth::hash_payload(&body)))
    {
        let mut err = EventsError::with_message(
            rustack_events_model::error::EventsErrorCode::ValidationException,
            auth_err.to_string(),
        );
        err.status_code = http::StatusCode::FORBIDDEN;
        return error_to_response(&err, request_id);
    }

    // 5. Dispatch to handler.
    match dispatch_operation(handler, op, body).await {
        Ok(response) => response,
        Err(err) => error_to_response(&err, request_id),
    }
}

/// Collect the incoming body into a single `Bytes` buffer.
async fn collect_body(incoming: Incoming) -> Result<Bytes, EventsError> {
    rustack_core::http::collect_body(incoming, rustack_core::http::BodyBudget::control())
        .await
        .map_err(|e| {
            let mut err = EventsError::internal_error(e.to_string());
            err.status_code = e.status_code();
            err
        })
}

/// Add common response headers to every EventBridge response.
fn add_common_headers(
    mut response: http::Response<EventsResponseBody>,
    request_id: &str,
) -> http::Response<EventsResponseBody> {
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
