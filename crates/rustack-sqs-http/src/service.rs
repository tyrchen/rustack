//! SQS HTTP service implementing the hyper `Service` trait.

use std::{convert::Infallible, future::Future, pin::Pin, sync::Arc};

use bytes::Bytes;
use hyper::body::Incoming;
use rustack_sqs_model::error::SqsError;

use crate::{
    body::SqsResponseBody,
    dispatch::{SqsHandler, dispatch_operation},
    response::{CONTENT_TYPE, error_to_response},
    router::resolve_operation,
};

/// Configuration for the SQS HTTP service.
#[derive(Clone)]
pub struct SqsHttpConfig {
    /// Whether to skip AWS signature validation.
    pub skip_signature_validation: bool,
    /// The AWS region this service is running in.
    pub region: String,
    /// Credential provider for signature validation.
    pub credential_provider: Option<Arc<dyn rustack_auth::CredentialProvider>>,
}

impl std::fmt::Debug for SqsHttpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqsHttpConfig")
            .field("skip_signature_validation", &self.skip_signature_validation)
            .field("region", &self.region)
            .field(
                "credential_provider",
                &self.credential_provider.as_ref().map(|_| "..."),
            )
            .finish()
    }
}

impl Default for SqsHttpConfig {
    fn default() -> Self {
        Self {
            skip_signature_validation: true,
            region: "us-east-1".to_owned(),
            credential_provider: None,
        }
    }
}

/// Hyper `Service` implementation for SQS.
///
/// Wraps an [`SqsHandler`] implementation and routes incoming HTTP
/// requests to the appropriate SQS operation handler.
#[derive(Debug)]
pub struct SqsHttpService<H: SqsHandler> {
    handler: Arc<H>,
    config: Arc<SqsHttpConfig>,
}

impl<H: SqsHandler> SqsHttpService<H> {
    /// Create a new `SqsHttpService`.
    pub fn new(handler: Arc<H>, config: SqsHttpConfig) -> Self {
        Self {
            handler,
            config: Arc::new(config),
        }
    }
}

impl<H: SqsHandler> Clone for SqsHttpService<H> {
    fn clone(&self) -> Self {
        Self {
            handler: Arc::clone(&self.handler),
            config: Arc::clone(&self.config),
        }
    }
}

impl<H: SqsHandler> hyper::service::Service<http::Request<Incoming>> for SqsHttpService<H> {
    type Response = http::Response<SqsResponseBody>;
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

/// Process a single SQS HTTP request through the full pipeline.
async fn process_request<H: SqsHandler, B>(
    req: http::Request<B>,
    handler: &H,
    config: &SqsHttpConfig,
    request_id: &str,
) -> http::Response<SqsResponseBody>
where
    B: http_body::Body<Data = Bytes>,
    B::Error: std::error::Error + Send + Sync + 'static,
{
    let (parts, incoming) = req.into_parts();

    // 1. Verify POST method (SQS only accepts POST).
    if parts.method != http::Method::POST {
        let err = SqsError::new(
            rustack_sqs_model::error::SqsErrorCode::InvalidParameterValue,
            format!("SQS requires POST method, got {}", parts.method),
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
        Err(err) => {
            let error = SqsError::new(
                rustack_sqs_model::error::SqsErrorCode::InvalidParameterValue,
                err.to_string(),
            );
            let mut response = error_to_response(&error, request_id);
            *response.status_mut() = err.status_code();
            return response;
        }
    };

    // 4. Authenticate (if enabled).
    if let Err(auth_err) = rustack_auth::AuthMode::resolve(
        config.skip_signature_validation,
        config.credential_provider.as_deref(),
    )
    .and_then(|mode| mode.verify(&parts, &rustack_auth::hash_payload(&body)))
    {
        let err = SqsError::new(
            rustack_sqs_model::error::SqsErrorCode::InvalidSecurity,
            auth_err.to_string(),
        );
        return error_to_response(&err, request_id);
    }

    // 5. Dispatch to handler.
    match dispatch_operation(handler, op, body).await {
        Ok(response) => response,
        Err(err) => error_to_response(&err, request_id),
    }
}

/// Collect the incoming body into a single `Bytes` buffer.
async fn collect_body<B>(incoming: B) -> Result<Bytes, rustack_core::http::BodyReadError>
where
    B: http_body::Body<Data = Bytes>,
    B::Error: std::error::Error + Send + Sync + 'static,
{
    rustack_core::http::collect_body(incoming, rustack_core::http::BodyBudget::control()).await
}

/// Add common response headers to every SQS response.
fn add_common_headers(
    mut response: http::Response<SqsResponseBody>,
    request_id: &str,
) -> http::Response<SqsResponseBody> {
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use http_body_util::Full;
    use rustack_auth::{
        StaticCredentialProvider,
        canonical::build_canonical_request,
        sigv4::{build_string_to_sign, compute_signature, derive_signing_key},
    };

    use super::*;

    #[derive(Default)]
    struct Counter(AtomicUsize);
    impl SqsHandler for Counter {
        fn handle_operation(
            &self,
            _: rustack_sqs_model::operations::SqsOperation,
            _: Bytes,
        ) -> Pin<Box<dyn Future<Output = Result<http::Response<SqsResponseBody>, SqsError>> + Send>>
        {
            self.0.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                Ok(http::Response::new(SqsResponseBody::from_bytes(
                    Bytes::from_static(b"{}"),
                )))
            })
        }
    }

    fn request(body: &[u8], original: Option<&[u8]>) -> http::Request<Full<Bytes>> {
        let mut request = http::Request::builder()
            .method("POST")
            .uri("/")
            .header("host", "localhost")
            .header("x-amz-date", "20260101T000000Z")
            .header("x-amz-target", "AmazonSQS.SendMessage")
            .body(Full::new(Bytes::copy_from_slice(body)))
            .unwrap();
        if let Some(original) = original {
            let hash = rustack_auth::hash_payload(original);
            let canonical = build_canonical_request(
                "POST",
                "/",
                "",
                &[("host", "localhost"), ("x-amz-date", "20260101T000000Z")],
                &["host", "x-amz-date"],
                &hash,
            );
            let text = build_string_to_sign(
                "20260101T000000Z",
                "20260101/us-east-1/sqs/aws4_request",
                &rustack_auth::hash_payload(canonical.as_bytes()),
            );
            let signature = compute_signature(
                &derive_signing_key("secret", "20260101", "us-east-1", "sqs"),
                &text,
            );
            request.headers_mut().insert(
                "authorization",
                format!(
                    "AWS4-HMAC-SHA256 \
                     Credential=key/20260101/us-east-1/sqs/aws4_request,SignedHeaders=host;\
                     x-amz-date,Signature={signature}"
                )
                .parse()
                .unwrap(),
            );
            request
                .headers_mut()
                .insert("x-amz-content-sha256", hash.parse().unwrap());
        }
        request
    }

    #[tokio::test]
    async fn test_should_never_dispatch_missing_credentials_or_tampered_payload() {
        let handler = Counter::default();
        let mut config = SqsHttpConfig {
            skip_signature_validation: false,
            ..SqsHttpConfig::default()
        };
        let response = process_request(request(b"original", None), &handler, &config, "test").await;
        assert!(!response.status().is_success());
        config.credential_provider = Some(Arc::new(StaticCredentialProvider::new(vec![(
            "key".to_owned(),
            "secret".to_owned(),
        )])));
        for request in [
            request(b"original", None),
            request(b"modified", Some(b"original")),
        ] {
            assert!(
                !process_request(request, &handler, &config, "test")
                    .await
                    .status()
                    .is_success()
            );
        }
        assert_eq!(handler.0.load(Ordering::SeqCst), 0);
        assert!(
            process_request(
                request(b"original", Some(b"original")),
                &handler,
                &config,
                "test"
            )
            .await
            .status()
            .is_success()
        );
        assert_eq!(handler.0.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_should_reject_oversized_body_without_dispatch() {
        let handler = Counter::default();
        let body = vec![0; 16 * 1024 * 1024 + 1];
        let response = process_request(
            request(&body, None),
            &handler,
            &SqsHttpConfig::default(),
            "test",
        )
        .await;
        assert!(!response.status().is_success());
        assert_eq!(handler.0.load(Ordering::SeqCst), 0);
    }
}
