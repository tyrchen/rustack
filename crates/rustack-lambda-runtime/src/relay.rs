//! Supervised control transport for an environment-local Lambda Runtime API.
//!
//! This owner invalidates the API session; the host environment supervisor
//! remains responsible for stopping/removing the separate function container.

use std::{future::pending, io, sync::Arc, time::Duration};

use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    runtime::Handle,
    sync::{oneshot, watch},
    task::JoinHandle,
    time::{Instant, sleep_until, timeout_at},
};
use tracing::instrument::WithSubscriber;
use typed_builder::TypedBuilder;

use crate::{
    http::{ApiError, RuntimeApiHandle},
    metadata::deadline_from_unix_ms,
    protocol::{
        Acknowledgement, Architecture, AttemptId, EnvironmentId, Frame, Message, Outcome, Payload,
        ProtocolError,
    },
    session::{Invocation, InvocationTicket, RuntimeResult, SessionError},
    transport::{InputEnd, Settings, Transport, after},
};

const CONTROL_SILENCE: Duration = Duration::from_hours(1);

/// Relay errors contain classes and redacted protocol/IO sources.
#[derive(Debug, Clone, Error)]
#[non_exhaustive]
pub enum RelayError {
    /// The process entry point needs Linux and supported CPU architecture.
    #[error("Lambda relay requires Linux x86_64 or arm64")]
    UnsupportedPlatform,
    /// Invalid operator-owned worker configuration.
    #[error("invalid Lambda relay configuration")]
    Configuration,
    /// Unexpected EOF before a normal Close exchange.
    #[error("Lambda relay control EOF")]
    Eof,
    /// Protocol validation/transport failure.
    #[error("Lambda relay protocol failed")]
    Protocol(#[source] Arc<ProtocolError>),
    /// IO while reading the frame's first byte failed.
    #[error("Lambda relay control IO failed")]
    Io(#[source] Arc<io::Error>),
    /// A frame belongs to another environment.
    #[error("Lambda relay environment mismatch")]
    Scope,
    /// Message direction, identity or session phase is invalid.
    #[error("unexpected Lambda relay control message")]
    UnexpectedMessage,
    /// An IO/retirement lease expired.
    #[error("Lambda relay deadline exceeded")]
    Deadline,
    /// Output could not be confirmed flushed.
    #[error("Lambda relay output failed")]
    Output,
    /// An owned worker terminated unexpectedly.
    #[error("Lambda relay worker failed")]
    Worker,
    /// Shared session failed.
    #[error(transparent)]
    Session(#[from] SessionError),
    /// The API could not be cleaned up.
    #[error(transparent)]
    Api(#[from] ApiError),
}

impl From<ProtocolError> for RelayError {
    fn from(error: ProtocolError) -> Self {
        Self::Protocol(Arc::new(error))
    }
}
impl From<io::Error> for RelayError {
    fn from(error: io::Error) -> Self {
        Self::Io(Arc::new(error))
    }
}

/// Validated environment identity and forwarded worker leases.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct RelayConfig {
    environment: EnvironmentId,
    architecture: Architecture,
    idle_lease: Duration,
    io_budget: Duration,
    retirement_budget: Duration,
}

impl RelayConfig {
    /// Validate worker leases without reading/mutating process environment.
    ///
    /// # Errors
    /// Rejects zero leases, idle over24h or IO/retirement over60s.
    ///
    /// ```
    /// # use rustack_lambda_runtime::{protocol::{EnvironmentId, Architecture}, relay::RelayConfig};
    /// # use std::time::Duration;
    /// let config = RelayConfig::new(EnvironmentId::default(), Architecture::Arm64,
    ///     Duration::from_secs(600), Duration::from_secs(5), Duration::from_secs(10))?;
    /// # Ok::<(), rustack_lambda_runtime::relay::RelayError>(())
    /// ```
    pub fn new(
        environment: EnvironmentId,
        architecture: Architecture,
        idle_lease: Duration,
        io_budget: Duration,
        retirement_budget: Duration,
    ) -> Result<Self, RelayError> {
        if idle_lease.is_zero()
            || idle_lease > Duration::from_hours(24)
            || io_budget.is_zero()
            || io_budget > Duration::from_secs(60)
            || retirement_budget.is_zero()
            || retirement_budget > Duration::from_secs(60)
        {
            return Err(RelayError::Configuration);
        }
        Ok(Self {
            environment,
            architecture,
            idle_lease,
            io_budget,
            retirement_budget,
        })
    }
}

/// Owner of the relay supervisor; Drop signals stop without abandoning cleanup.
#[derive(Debug)]
#[non_exhaustive]
pub struct RelayHandle {
    stop: watch::Sender<bool>,
    task: Option<JoinHandle<Result<(), RelayError>>>,
    settlement: Option<Result<(), RelayError>>,
}

impl RelayHandle {
    /// Await natural Close/EOF/failure without requesting shutdown.
    ///
    /// # Errors
    /// Returns cached relay/protocol/supervision failures.
    ///
    /// ```no_run
    /// # async fn wait(relay: &mut rustack_lambda_runtime::relay::RelayHandle) {
    /// let outcome = relay.wait().await;
    /// # }
    /// ```
    pub async fn wait(&mut self) -> Result<(), RelayError> {
        if let Some(result) = &self.settlement {
            return result.clone();
        }
        let result = match self.task.as_mut() {
            Some(task) => task
                .await
                .map_err(|_| RelayError::Worker)
                .and_then(|result| result),
            None => Ok(()),
        };
        self.task.take();
        self.settlement = Some(result.clone());
        result
    }

    /// Signal priority stop and join the owned API/control workers.
    ///
    /// # Errors
    /// Returns protocol, API cleanup or supervision failure.
    ///
    /// ```no_run
    /// # async fn stop(relay: &mut rustack_lambda_runtime::relay::RelayHandle) {
    /// let clean = relay.shutdown().await.is_ok();
    /// # }
    /// ```
    pub async fn shutdown(&mut self) -> Result<(), RelayError> {
        self.stop.send_replace(true);
        self.wait().await
    }
}

impl Drop for RelayHandle {
    fn drop(&mut self) {
        self.stop.send_replace(true);
    }
}

/// Transfer the API and cancel-safe owned streams into one relay supervisor.
///
/// IO must be asynchronously cancellable; the Linux binary supplies nonblocking
/// Unix pipes. A blocking stdin adapter cannot satisfy the shutdown contract.
///
/// # Errors
/// Returns unavailable supervision when no Tokio runtime is entered.
///
/// ```no_run
/// # async fn example(api: rustack_lambda_runtime::http::RuntimeApiHandle, config: rustack_lambda_runtime::relay::RelayConfig) -> Result<(), rustack_lambda_runtime::relay::RelayError> {
/// let (reader, _) = tokio::io::duplex(1024);
/// let (writer, _) = tokio::io::duplex(1024);
/// let mut relay = rustack_lambda_runtime::relay::start(api, config, reader, writer)?;
/// relay.shutdown().await?;
/// # Ok(()) }
/// ```
pub fn start<R, W>(
    mut api: RuntimeApiHandle,
    config: RelayConfig,
    reader: R,
    writer: W,
) -> Result<RelayHandle, RelayError>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let runtime = Handle::try_current().map_err(|_| RelayError::Worker)?;
    let (stop, stopped) = watch::channel(false);
    let transport = Transport::start(
        reader,
        writer,
        Settings {
            silence: config.idle_lease.max(CONTROL_SILENCE),
            io_budget: config.io_budget,
            input_end: InputEnd::Close,
        },
    )?;
    let initialization_error = api.take_initialization_error();
    let actor = RelayActor::builder()
        .api(api)
        .config(config)
        .transport(transport)
        .stop(stopped)
        .initialization_error(initialization_error)
        .build();
    let task = runtime.spawn(actor.run().with_current_subscriber());
    Ok(RelayHandle {
        stop,
        task: Some(task),
        settlement: None,
    })
}

#[derive(Debug, Default)]
enum Phase {
    #[default]
    Initializing,
    Idle,
    Busy(InvocationTicket),
    Retiring(Instant),
}

impl Phase {
    async fn result(&mut self) -> Result<RuntimeResult, SessionError> {
        match self {
            Self::Busy(ticket) => ticket.result().await,
            _ => pending().await,
        }
    }
}

#[derive(Debug, TypedBuilder)]
struct RelayActor {
    api: RuntimeApiHandle,
    config: RelayConfig,
    transport: Transport,
    stop: watch::Receiver<bool>,
    initialization_error: Option<oneshot::Receiver<Payload>>,
    #[builder(default)]
    phase: Phase,
    #[builder(default)]
    bootstrap_announced: bool,
    #[builder(default)]
    last_attempt: Option<AttemptId>,
}

impl RelayActor {
    async fn run(mut self) -> Result<(), RelayError> {
        let outcome = self.control().await;
        self.api.session().close();
        let api = self.api.shutdown().await.map_err(RelayError::from);
        let transport = self.transport.shutdown().await;
        outcome.and(api).and(transport)
    }

    async fn emit(&self, message: Message) -> Result<(), RelayError> {
        let deadline = match self.phase {
            Phase::Retiring(until) => after(self.config.io_budget)?.min(until),
            _ => after(self.config.io_budget)?,
        };
        self.transport
            .send(Frame::new(self.config.environment, message), deadline)
            .await
    }

    async fn control(&mut self) -> Result<(), RelayError> {
        if *self.stop.borrow() {
            return Ok(());
        }
        self.emit(Message::Hello {
            architecture: self.config.architecture,
        })
        .await?;
        self.emit(Message::Ready).await?;
        let session = self.api.session().clone();
        loop {
            let retirement = match self.phase {
                Phase::Retiring(deadline) => Some(deadline),
                _ => None,
            };
            tokio::select! {
                biased;
                () = async { let _ = self.stop.wait_for(|stopped| *stopped).await; } => return Ok(()),
                () = retirement_deadline(retirement) => return Err(RelayError::Deadline),
                frame = self.transport.recv() => {
                    let frame = frame?;
                    if frame.environment() != self.config.environment { return Err(RelayError::Scope); }
                    if self.command(frame.into_message()).await? { return Ok(()); }
                }
                error = initialization_error(&mut self.initialization_error), if self.initialization_error.is_some() && retirement.is_none() => {
                    self.initialization_error = None;
                    if let Some(payload) = error {
                        self.emit(Message::InitError { payload }).await?;
                        self.retire().await?;
                    }
                }
                ready = session.bootstrap_ready(after(CONTROL_SILENCE)?), if !self.bootstrap_announced && !matches!(self.phase, Phase::Retiring(_)) => {
                    ready?;
                    self.bootstrap_announced = true;
                    self.phase = Phase::Idle;
                    self.emit(Message::BootstrapReady).await?;
                }
                result = self.phase.result() => { self.completed(result?).await?; }
                () = session.closed(), if !matches!(self.phase, Phase::Retiring(_)) => return Err(RelayError::Session(SessionError::Unavailable)),
            }
        }
    }

    async fn command(&mut self, message: Message) -> Result<bool, RelayError> {
        match message {
            Message::Invoke {
                attempt,
                deadline_unix_ms,
                function_arn,
                trace_id,
                payload,
            } if matches!(self.phase, Phase::Idle) && self.last_attempt != Some(attempt) => {
                let deadline =
                    deadline_from_unix_ms(deadline_unix_ms).map_err(|_| RelayError::Deadline)?;
                let invocation =
                    Invocation::new(attempt, deadline, function_arn, trace_id, payload)?;
                let ticket = self
                    .api
                    .session()
                    .submit(invocation, after(self.config.io_budget)?.min(deadline))
                    .await?;
                self.last_attempt = Some(attempt);
                self.phase = Phase::Busy(ticket);
            }
            Message::Cancel { attempt } if self.last_attempt == Some(attempt) => {
                self.retire().await?;
                self.emit(Message::Ack(Acknowledgement::Cancel(attempt)))
                    .await?;
            }
            Message::Close => {
                self.api.session().close();
                self.api.shutdown().await?;
                self.emit(Message::Ack(Acknowledgement::Close)).await?;
                return Ok(true);
            }
            _ => return Err(RelayError::UnexpectedMessage),
        }
        Ok(false)
    }

    async fn completed(&mut self, result: RuntimeResult) -> Result<(), RelayError> {
        let attempt = self.last_attempt.ok_or(RelayError::UnexpectedMessage)?;
        self.phase = Phase::Idle;
        match result {
            RuntimeResult::Success(payload) => {
                self.emit(Message::InvocationResult {
                    attempt,
                    outcome: Outcome::Success,
                    payload,
                })
                .await?;
            }
            RuntimeResult::FunctionError(payload) => {
                self.emit(Message::InvocationResult {
                    attempt,
                    outcome: Outcome::FunctionError,
                    payload,
                })
                .await?;
                self.retire().await?;
            }
            RuntimeResult::InitError(payload) => {
                self.initialization_error = None;
                self.emit(Message::InitError { payload }).await?;
                self.retire().await?;
            }
        }
        Ok(())
    }

    async fn retire(&mut self) -> Result<(), RelayError> {
        if !matches!(self.phase, Phase::Retiring(_)) {
            if let Phase::Busy(ticket) = &self.phase {
                ticket.cancel();
            }
            self.api.session().close();
            timeout_at(after(self.config.io_budget)?, self.api.session().closed())
                .await
                .map_err(|_| RelayError::Deadline)?;
            self.phase = Phase::Retiring(after(self.config.retirement_budget)?);
        }
        Ok(())
    }
}

async fn initialization_error(
    receiver: &mut Option<oneshot::Receiver<Payload>>,
) -> Option<Payload> {
    match receiver {
        Some(receiver) => receiver.await.ok(),
        None => pending().await,
    }
}

async fn retirement_deadline(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => sleep_until(deadline).await,
        None => pending().await,
    }
}

#[cfg(test)]
mod tests;
