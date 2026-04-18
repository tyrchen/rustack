//! CloudFront hyper `Service`.

use std::{convert::Infallible, future::Future, pin::Pin, sync::Arc};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::{Body, Frame, Incoming};

use crate::{
    dispatch::{CloudFrontHandler, dispatch as dispatch_op},
    response::error_response,
    router::resolve,
};

/// Response body type for the CloudFront HTTP service.
#[derive(Debug)]
pub struct HttpBody {
    inner: Full<Bytes>,
}

impl Default for HttpBody {
    fn default() -> Self {
        Self {
            inner: Full::new(Bytes::new()),
        }
    }
}

impl From<String> for HttpBody {
    fn from(s: String) -> Self {
        Self {
            inner: Full::new(Bytes::from(s)),
        }
    }
}

impl From<Bytes> for HttpBody {
    fn from(b: Bytes) -> Self {
        Self {
            inner: Full::new(b),
        }
    }
}

impl Body for HttpBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        // SAFETY: we never move `inner` out.
        let inner = unsafe { self.map_unchecked_mut(|s| &mut s.inner) };
        inner.poll_frame(cx)
    }
}

/// Configuration for the CloudFront HTTP service.
#[derive(Clone)]
pub struct CloudFrontHttpConfig {
    /// Whether to skip SigV4 validation.
    pub skip_signature_validation: bool,
    /// Region string to report to clients.
    pub region: String,
    /// Optional credential provider for SigV4 verification.
    pub credential_provider: Option<Arc<dyn rustack_auth::CredentialProvider>>,
}

impl std::fmt::Debug for CloudFrontHttpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudFrontHttpConfig")
            .field("skip_signature_validation", &self.skip_signature_validation)
            .field("region", &self.region)
            .field(
                "credential_provider",
                &self.credential_provider.as_ref().map(|_| "..."),
            )
            .finish()
    }
}

impl Default for CloudFrontHttpConfig {
    fn default() -> Self {
        Self {
            skip_signature_validation: true,
            region: "us-east-1".to_owned(),
            credential_provider: None,
        }
    }
}

/// CloudFront HTTP service.
#[derive(Debug)]
pub struct CloudFrontHttpService<H: CloudFrontHandler> {
    handler: Arc<H>,
    config: Arc<CloudFrontHttpConfig>,
}

impl<H: CloudFrontHandler> CloudFrontHttpService<H> {
    /// Create a new service.
    pub fn new(handler: Arc<H>, config: CloudFrontHttpConfig) -> Self {
        Self {
            handler,
            config: Arc::new(config),
        }
    }
}

impl<H: CloudFrontHandler> Clone for CloudFrontHttpService<H> {
    fn clone(&self) -> Self {
        Self {
            handler: Arc::clone(&self.handler),
            config: Arc::clone(&self.config),
        }
    }
}

impl<H: CloudFrontHandler> hyper::service::Service<http::Request<Incoming>>
    for CloudFrontHttpService<H>
{
    type Response = http::Response<HttpBody>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, req: http::Request<Incoming>) -> Self::Future {
        let handler = Arc::clone(&self.handler);
        let config = Arc::clone(&self.config);
        let request_id = uuid::Uuid::new_v4().to_string();
        Box::pin(async move { Ok(serve(req, handler.as_ref(), &config, request_id).await) })
    }
}

async fn serve<H: CloudFrontHandler>(
    req: http::Request<Incoming>,
    handler: &H,
    _config: &CloudFrontHttpConfig,
    request_id: String,
) -> http::Response<HttpBody> {
    let (parts, body) = req.into_parts();
    let body_bytes = match body.collect().await {
        Ok(c) => c.to_bytes(),
        Err(e) => {
            let err = rustack_cloudfront_model::CloudFrontError::Internal(format!(
                "failed to read body: {e}"
            ));
            return error_response(&err, &request_id);
        }
    };

    let route = match resolve(&parts.method, &parts.uri) {
        Ok(r) => r,
        Err(e) => return error_response(&e, &request_id),
    };

    let if_match = parts
        .headers
        .get(http::header::IF_MATCH)
        .and_then(|v| v.to_str().ok());

    let mut resp = dispatch_op(
        handler,
        route,
        &parts.uri,
        &parts.headers,
        if_match,
        body_bytes,
        &request_id,
    )
    .await;

    if let Ok(hv) = http::HeaderValue::from_str(&request_id) {
        resp.headers_mut().entry("x-amzn-requestid").or_insert(hv);
    }
    resp
}
