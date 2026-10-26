//! Loopback Runtime API adapter backed by the shared session actor.
//!
//! One supervisor owns the session, listener and bounded HTTP connection tasks.
//! Infrastructure diagnostics never include peer targets, headers or payloads.

use std::{
    convert::Infallible,
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use ::http::{
    HeaderMap, HeaderValue, Method, Request, Response, StatusCode, Uri,
    header::{CONNECTION, CONTENT_LENGTH, CONTENT_TYPE},
};
use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::{body::Incoming, server::conn::http1, service::service_fn};
use hyper_util::rt::{TokioIo, TokioTimer};
use thiserror::Error;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{oneshot, watch},
    task::{JoinHandle, JoinSet},
    time::{Instant, timeout, timeout_at},
};
use tracing::instrument::WithSubscriber;

use crate::{
    diagnostics::runtime_dispatch,
    metadata::deadline_unix_ms,
    protocol::{AttemptId, MAX_PAYLOAD_BYTES, Outcome, Payload},
    session::{Invocation, RuntimeSession, SessionError, SessionHandle},
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const HEADER_BYTES: usize = 16 * 1024;
const HEADER_COUNT: usize = 64;
const HEADER_VALUE_BYTES: usize = 256;
const TARGET_BYTES: usize = 256;
const CONNECTIONS: usize = 4;
const INVOCATION_ID: &str = "lambda-runtime-invocation-id";

/// Adapter/supervision failures without invalid peer data in their diagnostics.
#[derive(Debug, Clone, Error)]
#[non_exhaustive]
pub enum ApiError {
    /// Listener IO failure.
    #[error("runtime API listener IO failed")]
    Io(#[source] Arc<io::Error>),
    /// Shared session failed.
    #[error(transparent)]
    Session(#[from] SessionError),
    /// A supervised HTTP task failed or exceeded graceful shutdown.
    #[error("runtime API connection failed")]
    ConnectionFailed,
    /// The server task failed to join.
    #[error("runtime API supervisor failed")]
    SupervisorFailed,
    /// Runtime API listeners must remain environment-local.
    #[error("runtime API requires a loopback address")]
    NonLoopback,
}

impl From<io::Error> for ApiError {
    fn from(error: io::Error) -> Self {
        Self::Io(Arc::new(error))
    }
}

#[derive(Debug)]
struct ServerTask {
    stop: watch::Sender<bool>,
    task: Option<JoinHandle<Result<(), ApiError>>>,
    settlement: Option<Result<(), ApiError>>,
}

impl ServerTask {
    async fn shutdown(&mut self) -> Result<(), ApiError> {
        self.stop.send_replace(true);
        if let Some(result) = &self.settlement {
            return result.clone();
        }
        let result = match self.task.as_mut() {
            Some(task) => task
                .await
                .map_err(|_| ApiError::SupervisorFailed)
                .and_then(|result| result),
            None => Ok(()),
        };
        self.task.take();
        self.settlement = Some(result.clone());
        result
    }
}

impl Drop for ServerTask {
    fn drop(&mut self) {
        // Fallback: the retained supervisor still owns its session/connections.
        // Normal shutdown joins this handle before the parent Tokio runtime stops.
        self.stop.send_replace(true);
    }
}

/// Owned loopback listener and shared-session command port.
#[derive(Debug)]
#[non_exhaustive]
pub struct RuntimeApiHandle {
    addr: SocketAddr,
    session: SessionHandle,
    server: ServerTask,
    initialization_error: Option<oneshot::Receiver<Payload>>,
}

impl RuntimeApiHandle {
    /// Bound listener address for `AWS_LAMBDA_RUNTIME_API`.
    ///
    /// ```
    /// # fn inspect(api: &rustack_lambda_runtime::http::RuntimeApiHandle) {
    /// assert!(api.addr().ip().is_loopback());
    /// # }
    /// ```
    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Shared session port used by the executor or control relay.
    ///
    /// ```
    /// # fn inspect(api: &rustack_lambda_runtime::http::RuntimeApiHandle) {
    /// let session = api.session().clone();
    /// # }
    /// ```
    #[must_use]
    pub fn session(&self) -> &SessionHandle {
        &self.session
    }

    /// Transfer the initialization-error receiver at most once.
    ///
    /// ```
    /// # fn inspect(api: &mut rustack_lambda_runtime::http::RuntimeApiHandle) {
    /// let notification = api.take_initialization_error();
    /// assert!(api.take_initialization_error().is_none());
    /// # }
    /// ```
    pub fn take_initialization_error(&mut self) -> Option<oneshot::Receiver<Payload>> {
        self.initialization_error.take()
    }

    /// Stop admission and join all connection/session tasks.
    ///
    /// # Errors
    /// Returns cached IO/session/supervisor failures; dropping a waiter retains ownership.
    ///
    /// ```no_run
    /// # async fn stop(api: &mut rustack_lambda_runtime::http::RuntimeApiHandle) {
    /// let clean = api.shutdown().await.is_ok();
    /// # }
    /// ```
    pub async fn shutdown(&mut self) -> Result<(), ApiError> {
        self.server.shutdown().await
    }
}

/// Start the default Native loopback adapter with a ten-minute idle lease.
///
/// # Errors
/// Returns listener/session startup failures.
///
/// ```no_run
/// # async fn example() -> Result<(), rustack_lambda_runtime::http::ApiError> {
/// let mut api = rustack_lambda_runtime::http::start().await?;
/// api.shutdown().await?;
/// # Ok(()) }
/// ```
pub async fn start() -> Result<RuntimeApiHandle, ApiError> {
    start_at(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        Duration::from_mins(10),
    )
    .await
}

/// Bind an explicit loopback endpoint, shared by Native and the relay.
///
/// # Errors
/// Rejects non-loopback addresses or listener/session startup failures.
///
/// ```no_run
/// # async fn example() -> Result<(), rustack_lambda_runtime::http::ApiError> {
/// let mut api = rustack_lambda_runtime::http::start_at("127.0.0.1:9001".parse().map_err(|_| rustack_lambda_runtime::http::ApiError::NonLoopback)?, std::time::Duration::from_secs(60)).await?;
/// api.shutdown().await?;
/// # Ok(()) }
/// ```
#[tracing::instrument(skip_all)]
pub async fn start_at(
    addr: SocketAddr,
    idle_lease: Duration,
) -> Result<RuntimeApiHandle, ApiError> {
    if !addr.ip().is_loopback() {
        return Err(ApiError::NonLoopback);
    }
    let listener = timeout(REQUEST_TIMEOUT, TcpListener::bind(addr))
        .await
        .map_err(|_| ApiError::ConnectionFailed)??;
    let addr = listener.local_addr()?;
    let mut owner = RuntimeSession::start(idle_lease)?;
    let session = owner.handle().clone();
    let initialization_error = owner.take_initialization_error();
    let (stop, _) = watch::channel(false);
    let task = tokio::spawn(
        serve(listener, owner, idle_lease, stop.clone()).with_subscriber(runtime_dispatch()),
    );
    Ok(RuntimeApiHandle {
        addr,
        session,
        server: ServerTask {
            stop,
            task: Some(task),
            settlement: None,
        },
        initialization_error,
    })
}

async fn serve(
    listener: TcpListener,
    mut owner: RuntimeSession,
    idle_lease: Duration,
    stop: watch::Sender<bool>,
) -> Result<(), ApiError> {
    let session = owner.handle().clone();
    let mut stopped = stop.subscribe();
    let mut connections = JoinSet::new();
    let mut failure = None;
    loop {
        tokio::select! {
            biased;
            () = async { let _ = stopped.wait_for(|stopped| *stopped).await; } => break,
            () = session.closed() => break,
            completed = connections.join_next(), if !connections.is_empty() => {
                if !matches!(completed, Some(Ok(Ok(())))) { failure = Some(ApiError::ConnectionFailed); break; }
            }
            accepted = listener.accept() => {
                match accepted {
                    Ok((socket, _)) if connections.len() < CONNECTIONS => {
                        let session = session.clone();
                        let stop = stop.subscribe();
                        connections.spawn(connection(socket, session, idle_lease, stop).with_current_subscriber());
                    }
                    Ok(_) => {}
                    Err(error) => { failure = Some(ApiError::from(error)); break; }
                }
            }
        }
    }
    drop(listener);
    stop.send_replace(true);
    session.close();
    while let Some(result) = connections.join_next().await {
        if !matches!(result, Ok(Ok(()))) {
            failure.get_or_insert(ApiError::ConnectionFailed);
        }
    }
    if let Err(error) = owner.shutdown().await {
        failure.get_or_insert(ApiError::Session(error));
    }
    failure.map_or(Ok(()), Err)
}

#[tracing::instrument(skip_all)]
async fn connection(
    socket: TcpStream,
    session: SessionHandle,
    idle_lease: Duration,
    mut stop: watch::Receiver<bool>,
) -> Result<(), ApiError> {
    let service_session = session.clone();
    let service = service_fn(move |request| handle(service_session.clone(), idle_lease, request));
    let mut builder = http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(REQUEST_TIMEOUT)
        .max_headers(HEADER_COUNT)
        .max_buf_size(HEADER_BYTES);
    let connection = builder.serve_connection(TokioIo::new(socket), service);
    tokio::pin!(connection);
    let result = tokio::select! {
        biased;
        () = async { let _ = stop.wait_for(|stopped| *stopped).await; } => {
            connection.as_mut().graceful_shutdown();
            timeout(REQUEST_TIMEOUT, &mut connection).await.map_err(|_| ApiError::ConnectionFailed)?
        }
        result = &mut connection => result,
    };
    // Idle keepalive/header timeouts close only that connection. The session's
    // own deadline bounds long polls and writes; other IO failures invalidate it.
    match result {
        Ok(()) => Ok(()),
        Err(error) if error.is_timeout() => Ok(()),
        Err(_) => {
            session.close();
            Err(ApiError::ConnectionFailed)
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Route {
    Next,
    Complete {
        attempt: AttemptId,
        outcome: Outcome,
    },
    InitError,
}

fn route(method: &Method, target: &Uri) -> Result<Route, StatusCode> {
    if target
        .path_and_query()
        .is_some_and(|value| value.as_str().len() > TARGET_BYTES)
        || target.scheme().is_some()
        || target.authority().is_some()
        || target.query().is_some()
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let path = target.path();
    if path == "/2018-06-01/runtime/invocation/next" {
        return if method == Method::GET {
            Ok(Route::Next)
        } else {
            Err(StatusCode::METHOD_NOT_ALLOWED)
        };
    }
    if path == "/2018-06-01/runtime/init/error" {
        return if method == Method::POST {
            Ok(Route::InitError)
        } else {
            Err(StatusCode::METHOD_NOT_ALLOWED)
        };
    }
    let rest = path
        .strip_prefix("/2018-06-01/runtime/invocation/")
        .ok_or(StatusCode::NOT_FOUND)?;
    let (attempt, suffix) = rest.split_once('/').ok_or(StatusCode::NOT_FOUND)?;
    let outcome = match suffix {
        "response" => Outcome::Success,
        "error" => Outcome::FunctionError,
        _ => return Err(StatusCode::NOT_FOUND),
    };
    if method != Method::POST {
        return Err(StatusCode::METHOD_NOT_ALLOWED);
    }
    Ok(Route::Complete {
        attempt: attempt.parse().map_err(|_| StatusCode::BAD_REQUEST)?,
        outcome,
    })
}

fn validate_headers(headers: &HeaderMap) -> Result<(), StatusCode> {
    if headers.len() > HEADER_COUNT {
        return Err(StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE);
    }
    let mut total = 0usize;
    for (name, value) in headers {
        if value.as_bytes().len() > HEADER_VALUE_BYTES
            || !value
                .as_bytes()
                .iter()
                .all(|byte| (32..=126).contains(byte))
        {
            return Err(StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE);
        }
        total = total
            .checked_add(name.as_str().len())
            .and_then(|n| n.checked_add(value.as_bytes().len()))
            .ok_or(StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE)?;
        if total > HEADER_BYTES {
            return Err(StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE);
        }
    }
    Ok(())
}

async fn handle(
    session: SessionHandle,
    idle_lease: Duration,
    request: Request<Incoming>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    Ok(dispatch(&session, idle_lease, request)
        .await
        .unwrap_or_else(error_response))
}

async fn dispatch(
    session: &SessionHandle,
    idle_lease: Duration,
    request: Request<Incoming>,
) -> Result<Response<Full<Bytes>>, StatusCode> {
    validate_headers(request.headers())?;
    let route = route(request.method(), request.uri())?;
    let invocation_id = if matches!(route, Route::Complete { .. }) {
        if request.headers().get_all(INVOCATION_ID).iter().count() > 1 {
            return Ok(invalid_invocation_id());
        }
        match request.headers().get(INVOCATION_ID) {
            None => None,
            Some(value) => match value
                .to_str()
                .ok()
                .and_then(|value| value.parse::<AttemptId>().ok())
            {
                Some(id) => Some(id),
                None => return Ok(invalid_invocation_id()),
            },
        }
    } else {
        None
    };
    if matches!(route, Route::Complete { attempt, .. } if invocation_id.is_some_and(|id| id != attempt))
    {
        return Ok(invalid_invocation_id());
    }
    let limit = if matches!(route, Route::Next) {
        0
    } else {
        MAX_PAYLOAD_BYTES
    };
    if request
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length > limit as u64)
    {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let deadline = Instant::now()
        .checked_add(REQUEST_TIMEOUT)
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    let body = timeout_at(deadline, Limited::new(request.into_body(), limit).collect())
        .await
        .map_err(|_| StatusCode::REQUEST_TIMEOUT)?
        .map_err(|error| {
            if error.is::<http_body_util::LengthLimitError>() {
                StatusCode::PAYLOAD_TOO_LARGE
            } else {
                StatusCode::BAD_REQUEST
            }
        })?;
    if body.trailers().is_some() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let payload = Payload::try_from(body.to_bytes()).map_err(|_| StatusCode::PAYLOAD_TOO_LARGE)?;
    match route {
        Route::Next => {
            let deadline = Instant::now()
                .checked_add(idle_lease)
                .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
            let invocation = session.next(deadline).await.map_err(status)?;
            match invocation_response(&invocation) {
                Ok(response) => Ok(response),
                Err(error) => {
                    session.close();
                    Err(error)
                }
            }
        }
        Route::Complete { attempt, outcome } => {
            let completed = session
                .complete(attempt, invocation_id, outcome, payload, deadline)
                .await;
            if matches!(completed, Err(SessionError::UnknownAttempt)) && invocation_id.is_some() {
                return Ok(invalid_invocation_id());
            }
            completed.map_err(status)?;
            Ok(response(StatusCode::ACCEPTED, Bytes::new(), false))
        }
        Route::InitError => {
            session
                .initialization_error(payload, deadline)
                .await
                .map_err(status)?;
            Ok(response(StatusCode::ACCEPTED, Bytes::new(), false))
        }
    }
}

fn invocation_response(invocation: &Invocation) -> Result<Response<Full<Bytes>>, StatusCode> {
    let (attempt, deadline, arn, trace, payload) = invocation.parts();
    let deadline = deadline_unix_ms(deadline).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Response::builder()
        .status(StatusCode::OK)
        .header("Lambda-Runtime-Aws-Request-Id", attempt.to_string())
        .header(INVOCATION_ID, attempt.to_string())
        .header("Lambda-Runtime-Deadline-Ms", deadline.get().to_string())
        .header("Lambda-Runtime-Invoked-Function-Arn", arn.as_ref())
        .header("Lambda-Runtime-Trace-Id", trace.as_ref())
        .header(CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from(payload.clone())))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

fn status(error: SessionError) -> StatusCode {
    match error {
        SessionError::Busy => StatusCode::CONFLICT,
        SessionError::UnknownAttempt | SessionError::InvalidMetadata => StatusCode::BAD_REQUEST,
        SessionError::Deadline => StatusCode::REQUEST_TIMEOUT,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

fn response(status: StatusCode, body: Bytes, close: bool) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(body));
    *response.status_mut() = status;
    if close {
        response
            .headers_mut()
            .insert(CONNECTION, HeaderValue::from_static("close"));
    }
    response
}

fn error_response(status: StatusCode) -> Response<Full<Bytes>> {
    response(status, Bytes::new(), true)
}

fn invalid_invocation_id() -> Response<Full<Bytes>> {
    response(StatusCode::BAD_REQUEST, Bytes::from_static(br#"{"errorType":"InvalidInvocationId","errorMessage":"Invocation ID does not match active invocation"}"#), true)
}

#[cfg(test)]
mod tests;
