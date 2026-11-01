//! Owned per-request transport futures; accepted commands remain core-owned.

use std::{convert::Infallible, error::Error, fmt, future::Future, pin::Pin, sync::Arc};

use async_trait::async_trait;
use bytes::Bytes;
use http_body::Body;
use hyper::body::Incoming;
use rustack_auth::{AuthMode, hash_payload};
use rustack_core::http::collect_body;
use rustack_stepfunctions_model::StepFunctionsError;
use tokio::time::{Instant, timeout_at};
use tracing::Instrument;
use uuid::Uuid;

use crate::{
    HttpError, Request, RequestContext, Response, StepFunctionsBody, StepFunctionsHttpConfig,
    protocol, response,
};

/// Object-safe typed command boundary for the runtime's real core adapter.
///
/// async-trait is required because services retain `Arc<dyn StepFunctionsHandler>`.
/// Accepted registry commands survive a dropped/timed-out HTTP future; handlers
/// must not equate reply-channel closure with execution cancellation.
#[async_trait]
pub trait StepFunctionsHandler: Send + Sync + 'static {
    /// Handle one decoded, bounded request under its caller deadline.
    ///
    /// # Errors
    /// Return a typed domain/protocol error; never include tokens or input in diagnostics.
    async fn handle(
        &self,
        context: RequestContext,
        request: Request,
    ) -> Result<Response, StepFunctionsError>;
}

/// Hyper service requiring an explicit real handler and validated configuration.
#[derive(Clone)]
#[non_exhaustive]
pub struct StepFunctionsHttpService {
    handler: Arc<dyn StepFunctionsHandler>,
    config: Arc<StepFunctionsHttpConfig>,
}
impl fmt::Debug for StepFunctionsHttpService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StepFunctionsHttpService(<redacted>)")
    }
}
impl StepFunctionsHttpService {
    /// Compose transport with a trusted typed core adapter.
    ///
    /// ```no_run
    /// # fn compose(handler: std::sync::Arc<dyn rustack_stepfunctions_http::StepFunctionsHandler>, config: rustack_stepfunctions_http::StepFunctionsHttpConfig) {
    /// let service=rustack_stepfunctions_http::StepFunctionsHttpService::new(handler,config);
    /// # }
    /// ```
    #[must_use]
    pub fn new(handler: Arc<dyn StepFunctionsHandler>, config: StepFunctionsHttpConfig) -> Self {
        Self {
            handler,
            config: Arc::new(config),
        }
    }
    /// Process any bounded byte body; network serving uses the same path.
    ///
    /// ```no_run
    /// # async fn serve(service: rustack_stepfunctions_http::StepFunctionsHttpService, request: http::Request<http_body_util::Full<bytes::Bytes>>) {
    /// let response=service.handle(request).await;
    /// # }
    /// ```
    pub async fn handle<B>(&self, request: http::Request<B>) -> http::Response<StepFunctionsBody>
    where
        B: Body<Data = Bytes> + Send + 'static,
        B::Error: Error + Send + Sync + 'static,
    {
        let id = Uuid::new_v4();
        let deadline = Instant::now() + self.config.total;
        let span = tracing::info_span!("stepfunctions.request",request_id=%id);
        let result = timeout_at(
            deadline,
            self.process(request, id, deadline).instrument(span),
        )
        .await;
        match result {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => response::failure(&error, id),
            Err(_) => response::failure(&HttpError::Deadline, id),
        }
    }
    async fn process<B>(
        &self,
        request: http::Request<B>,
        id: Uuid,
        deadline: Instant,
    ) -> Result<http::Response<StepFunctionsBody>, HttpError>
    where
        B: Body<Data = Bytes> + Send + 'static,
        B::Error: Error + Send + Sync + 'static,
    {
        let (parts, body) = request.into_parts();
        let declared = protocol::envelope(&parts)?;
        if declared.is_some_and(|length| {
            u64::try_from(length).map_or(true, |length| length > self.config.body.max_bytes())
        }) {
            return Err(HttpError::Body(rustack_core::http::BodyReadError::TooLarge));
        }
        let bytes = collect_body(crate::body::RequestBody::new(body), self.config.body)
            .await
            .map_err(HttpError::Body)?;
        if declared.is_some_and(|length| length != bytes.len()) {
            return Err(HttpError::Envelope);
        }
        let authenticated = AuthMode::resolve(self.config.skip, self.config.credentials.as_deref())
            .map_err(HttpError::Authentication)?
            .verify(&parts, &hash_payload(&bytes))
            .map_err(HttpError::Authentication)?;
        if let Some(auth) = &authenticated {
            if auth.service != "states"
                || auth.region != self.config.scope.region()
                || !auth
                    .signed_headers
                    .iter()
                    .any(|name| name == "x-amz-target")
            {
                return Err(HttpError::Authentication(
                    rustack_auth::AuthError::InvalidCredential,
                ));
            }
        }
        check(deadline)?;
        let operation = protocol::resolve_operation(&parts.headers)?;
        let request = Request::decode(operation, &bytes)?;
        check(deadline)?;
        let context = RequestContext {
            scope: self.config.scope.clone(),
            id,
            deadline,
            authenticated: authenticated.is_some(),
        };
        let reply = self
            .handler
            .handle(context, request)
            .await
            .map_err(HttpError::Handler)?;
        check(deadline)?;
        if reply.operation() != operation {
            return Err(HttpError::Response);
        }
        let response = response::success(&reply, id)?;
        check(deadline)?;
        Ok(response)
    }
}
fn check(deadline: Instant) -> Result<(), HttpError> {
    if Instant::now() >= deadline {
        Err(HttpError::Deadline)
    } else {
        Ok(())
    }
}
impl hyper::service::Service<http::Request<Incoming>> for StepFunctionsHttpService {
    type Response = http::Response<StepFunctionsBody>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Infallible>> + Send>>;
    fn call(&self, request: http::Request<Incoming>) -> Self::Future {
        let service = self.clone();
        Box::pin(async move { Ok(service.handle(request).await) })
    }
}
