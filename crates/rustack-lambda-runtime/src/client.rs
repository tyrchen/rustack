//! Ordered host-side relay protocol; external resources remain host-owned.
//!
//! Partial framing lives in retained stream workers. Selecting and cancelling
//! `next()` never cancels those workers or replays a business invocation.

use std::time::Duration;

use tokio::{
    io::{AsyncRead, AsyncWrite},
    time::{Instant, timeout_at},
};

use crate::{
    metadata::deadline_unix_ms,
    protocol::{
        Acknowledgement, Architecture, AttemptId, EnvironmentId, Frame, Message, Outcome, Payload,
    },
    relay::RelayError,
    session::{Invocation, RuntimeResult},
    transport::{InputEnd, Settings, Transport, after},
};

/// Validated host identity, expected architecture and control IO budget.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct ClientConfig {
    environment: EnvironmentId,
    architecture: Architecture,
    io_budget: Duration,
}

impl ClientConfig {
    /// Validate a nonzero control budget of at most sixty seconds.
    ///
    /// # Errors
    /// Returns Configuration when the budget is outside its allowed range.
    ///
    /// ```
    /// # use rustack_lambda_runtime::{client::ClientConfig, protocol::{Architecture, EnvironmentId}};
    /// let config = ClientConfig::new(EnvironmentId::default(), Architecture::Arm64,
    ///     std::time::Duration::from_secs(5))?;
    /// # Ok::<(), rustack_lambda_runtime::relay::RelayError>(())
    /// ```
    pub fn new(
        environment: EnvironmentId,
        architecture: Architecture,
        io_budget: Duration,
    ) -> Result<Self, RelayError> {
        if io_budget.is_zero() || io_budget > Duration::from_secs(60) {
            return Err(RelayError::Configuration);
        }
        Ok(Self {
            environment,
            architecture,
            io_budget,
        })
    }
}

/// One validated relay event, with payload diagnostics redacted.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClientEvent {
    /// Hello/architecture and Ready were received in order.
    RelayReady,
    /// The function issued its first valid Runtime API poll.
    BootstrapReady,
    /// One matching attempt completed; `FunctionError` retires this session.
    InvocationResult {
        /// Correlated invocation identity.
        attempt: AttemptId,
        /// Raw success or function error, without an implicit retry.
        result: RuntimeResult,
    },
    /// Initialization failed; this environment cannot be reused.
    InitError(Payload),
    /// The relay invalidated the attempt; external code may still be running.
    Cancelled(AttemptId),
    /// Close was acknowledged after the peer joined its Runtime API.
    Closed,
}

#[derive(Debug)]
enum Phase {
    AwaitHello,
    AwaitReady,
    Initializing,
    Idle,
    Busy(AttemptId),
    Cancelling {
        attempt: AttemptId,
        outcome_seen: bool,
    },
    Retiring,
    Closed,
    Failed(RelayError),
}

#[derive(Debug)]
struct State {
    phase: Phase,
    closing: bool,
    writing: bool,
    delivered: bool,
}

/// One host control session with retained IO and fail-closed send semantics.
///
/// Drop signals its stream supervisor. Normal environment shutdown must call
/// shutdown and separately terminate/remove every external resource.
#[derive(Debug)]
#[non_exhaustive]
pub struct ControlClient {
    config: ClientConfig,
    transport: Transport,
    state: State,
    last_attempt: Option<AttemptId>,
    settlement: Option<Result<(), RelayError>>,
}

impl ControlClient {
    /// Own asynchronously cancellable streams and start their retained workers.
    ///
    /// # Errors
    /// Returns Worker if no Tokio runtime is entered.
    ///
    /// ```no_run
    /// # async fn example(config: rustack_lambda_runtime::client::ClientConfig) -> Result<(), rustack_lambda_runtime::relay::RelayError> {
    /// let (reader, _) = tokio::io::duplex(1024);
    /// let (writer, _) = tokio::io::duplex(1024);
    /// let mut client = rustack_lambda_runtime::client::ControlClient::start(config, reader, writer)?;
    /// let first = client.next(tokio::time::Instant::now() + std::time::Duration::from_secs(5)).await;
    /// # Ok(()) }
    /// ```
    pub fn start<R, W>(config: ClientConfig, reader: R, writer: W) -> Result<Self, RelayError>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        Ok(Self {
            config,
            transport: Transport::start(
                reader,
                writer,
                Settings {
                    silence: Duration::from_hours(1),
                    io_budget: config.io_budget,
                    input_end: InputEnd::CloseAcknowledgement,
                },
            )?,
            state: State {
                phase: Phase::AwaitHello,
                closing: false,
                writing: false,
                delivered: false,
            },
            last_attempt: None,
            settlement: None,
        })
    }

    /// Whether a healthy, bootstrapped session can accept a new attempt now.
    ///
    /// ```no_run
    /// # fn inspect(client: &rustack_lambda_runtime::client::ControlClient) {
    /// let may_invoke = client.is_idle();
    /// # }
    /// ```
    #[must_use]
    pub fn is_idle(&self) -> bool {
        matches!(self.state.phase, Phase::Idle)
            && !self.state.closing
            && !self.state.writing
            && !self.transport.finished()
    }

    /// Receive one validated event; cancelling this future preserves framing.
    ///
    /// # Errors
    /// Returns a cached protocol/IO failure or invalidates the session at the
    /// absolute deadline. It rejects calls after an interrupted send.
    ///
    /// ```no_run
    /// # async fn receive(client: &mut rustack_lambda_runtime::client::ControlClient) {
    /// let event = client.next(tokio::time::Instant::now() + std::time::Duration::from_secs(5)).await;
    /// # }
    /// ```
    pub async fn next(&mut self, deadline: Instant) -> Result<ClientEvent, RelayError> {
        self.check()?;
        loop {
            if Instant::now() >= deadline {
                return Err(self.fail(RelayError::Deadline));
            }
            let received = timeout_at(deadline, self.transport.recv()).await;
            if Instant::now() >= deadline {
                return Err(self.fail(RelayError::Deadline));
            }
            let frame = match received {
                Ok(Ok(frame)) => frame,
                Ok(Err(error)) => return Err(self.fail(error)),
                Err(_) => return Err(self.fail(RelayError::Deadline)),
            };
            if frame.environment() != self.config.environment {
                return Err(self.fail(RelayError::Scope));
            }
            match self.accept(frame.into_message()) {
                Ok(Some(event)) => return Ok(event),
                Ok(None) => {}
                Err(error) => return Err(self.fail(error)),
            }
        }
    }

    /// Flush one attempt after first-poll readiness; never reconnect or replay.
    ///
    /// # Errors
    /// Returns invalid order/identity, expired metadata or transport failure.
    /// Dropping this send future poisons later business operations.
    ///
    /// ```no_run
    /// # async fn invoke(client: &mut rustack_lambda_runtime::client::ControlClient, invocation: rustack_lambda_runtime::session::Invocation) {
    /// let sent = client.invoke(invocation, tokio::time::Instant::now() + std::time::Duration::from_secs(5)).await;
    /// # }
    /// ```
    pub async fn invoke(
        &mut self,
        invocation: Invocation,
        deadline: Instant,
    ) -> Result<(), RelayError> {
        self.check()?;
        let (attempt, call_deadline, function_arn, trace_id, payload) = invocation.parts();
        if self.state.closing
            || !matches!(self.state.phase, Phase::Idle)
            || self.last_attempt == Some(attempt)
        {
            return Err(self.fail(RelayError::UnexpectedMessage));
        }
        let deadline_unix_ms = match deadline_unix_ms(call_deadline) {
            Ok(deadline)
                if call_deadline.saturating_duration_since(Instant::now())
                    <= Duration::from_hours(1) =>
            {
                deadline
            }
            _ => return Err(self.fail(RelayError::Deadline)),
        };
        self.last_attempt = Some(attempt);
        self.state.phase = Phase::Busy(attempt);
        self.send(
            Message::Invoke {
                attempt,
                deadline_unix_ms,
                function_arn: function_arn.clone(),
                trace_id: trace_id.clone(),
                payload: payload.clone(),
            },
            deadline.min(call_deadline),
        )
        .await
    }

    /// Invalidate the active or just-completed last attempt; await its event next.
    ///
    /// # Errors
    /// Rejects another identity, overlapping cancellation or a closing session.
    /// IO failure invalidates the connection without asserting process exit.
    ///
    /// ```no_run
    /// # async fn cancel(client: &mut rustack_lambda_runtime::client::ControlClient, id: rustack_lambda_runtime::protocol::AttemptId) {
    /// let sent = client.cancel(id, tokio::time::Instant::now() + std::time::Duration::from_secs(5)).await;
    /// # }
    /// ```
    pub async fn cancel(
        &mut self,
        attempt: AttemptId,
        deadline: Instant,
    ) -> Result<(), RelayError> {
        self.check()?;
        if self.state.closing || self.last_attempt != Some(attempt) {
            return Err(self.fail(RelayError::UnexpectedMessage));
        }
        let outcome_seen = match self.state.phase {
            Phase::Busy(active) if active == attempt => false,
            Phase::Idle | Phase::Retiring => true,
            _ => return Err(self.fail(RelayError::UnexpectedMessage)),
        };
        self.state.phase = Phase::Cancelling {
            attempt,
            outcome_seen,
        };
        self.send(Message::Cancel { attempt }, deadline).await
    }

    /// Flush Close while retaining validation of preceding peer events.
    ///
    /// # Errors
    /// Requires ordered Hello/Ready and a healthy, uninterrupted transport.
    ///
    /// ```no_run
    /// # async fn close(client: &mut rustack_lambda_runtime::client::ControlClient) {
    /// let sent = client.begin_close(tokio::time::Instant::now() + std::time::Duration::from_secs(5)).await;
    /// # }
    /// ```
    pub async fn begin_close(&mut self, deadline: Instant) -> Result<(), RelayError> {
        self.check()?;
        if matches!(self.state.phase, Phase::AwaitHello | Phase::AwaitReady) {
            return Err(self.fail(RelayError::UnexpectedMessage));
        }
        if !self.state.closing {
            self.state.closing = true;
            self.send(Message::Close, deadline).await?;
        }
        Ok(())
    }

    /// Complete Close/Ack and join IO, retaining ownership across dropped waiters.
    ///
    /// # Errors
    /// Caches protocol, deadline, uncertain-write and worker failures. A failed
    /// handshake still stops/joins stream workers; external cleanup is separate.
    ///
    /// ```no_run
    /// # async fn shutdown(client: &mut rustack_lambda_runtime::client::ControlClient) {
    /// let clean = client.shutdown(tokio::time::Instant::now() + std::time::Duration::from_secs(5)).await.is_ok();
    /// # }
    /// ```
    pub async fn shutdown(&mut self, deadline: Instant) -> Result<(), RelayError> {
        if let Some(result) = &self.settlement {
            return result.clone();
        }
        let outcome = async {
            if matches!(self.state.phase, Phase::Closed) {
                return Ok(());
            }
            self.check()?;
            while matches!(self.state.phase, Phase::AwaitHello | Phase::AwaitReady) {
                self.next(deadline).await?;
            }
            self.begin_close(deadline).await?;
            while !matches!(self.state.phase, Phase::Closed) {
                self.next(deadline).await?;
            }
            Ok(())
        }
        .await;
        let io = self.transport.shutdown().await;
        let outcome = outcome.and(io);
        self.settlement = Some(outcome.clone());
        outcome
    }

    fn check(&mut self) -> Result<(), RelayError> {
        if let Phase::Failed(error) = &self.state.phase {
            return Err(error.clone());
        }
        if self.state.writing {
            return Err(self.fail(RelayError::Output));
        }
        if matches!(self.state.phase, Phase::Closed) {
            return Err(RelayError::UnexpectedMessage);
        }
        Ok(())
    }

    fn fail(&mut self, error: RelayError) -> RelayError {
        self.state.phase = Phase::Failed(error.clone());
        self.transport.stop();
        error
    }

    async fn send(&mut self, message: Message, deadline: Instant) -> Result<(), RelayError> {
        let deadline = deadline.min(after(self.config.io_budget)?);
        if Instant::now() >= deadline {
            return Err(self.fail(RelayError::Deadline));
        }
        self.state.writing = true;
        let result = self
            .transport
            .send(Frame::new(self.config.environment, message), deadline)
            .await;
        match result {
            Ok(()) => {
                self.state.writing = false;
                Ok(())
            }
            Err(error) => Err(self.fail(error)),
        }
    }

    fn accept(&mut self, message: Message) -> Result<Option<ClientEvent>, RelayError> {
        match message {
            Message::Hello { architecture }
                if matches!(self.state.phase, Phase::AwaitHello)
                    && architecture == self.config.architecture =>
            {
                self.state.phase = Phase::AwaitReady;
                Ok(None)
            }
            Message::Ready if matches!(self.state.phase, Phase::AwaitReady) => {
                self.state.phase = Phase::Initializing;
                Ok(Some(ClientEvent::RelayReady))
            }
            Message::BootstrapReady if matches!(self.state.phase, Phase::Initializing) => {
                self.state.phase = Phase::Idle;
                Ok(Some(ClientEvent::BootstrapReady))
            }
            Message::InvocationResult {
                attempt,
                outcome,
                payload,
            } => match &mut self.state.phase {
                Phase::Busy(active) if *active == attempt => {
                    self.state.delivered = true;
                    let result = match outcome {
                        Outcome::Success => {
                            self.state.phase = Phase::Idle;
                            RuntimeResult::Success(payload)
                        }
                        Outcome::FunctionError => {
                            self.state.phase = Phase::Retiring;
                            RuntimeResult::FunctionError(payload)
                        }
                    };
                    Ok(Some(ClientEvent::InvocationResult { attempt, result }))
                }
                Phase::Cancelling {
                    attempt: active,
                    outcome_seen,
                } if *active == attempt && !*outcome_seen => {
                    self.state.delivered = true;
                    *outcome_seen = true;
                    Ok(None)
                }
                _ => Err(RelayError::UnexpectedMessage),
            },
            Message::InitError { payload } if !self.state.delivered => {
                match &mut self.state.phase {
                    Phase::Initializing | Phase::Idle | Phase::Busy(_) => {
                        self.state.phase = Phase::Retiring;
                        Ok(Some(ClientEvent::InitError(payload)))
                    }
                    Phase::Cancelling { outcome_seen, .. } if !*outcome_seen => {
                        *outcome_seen = true;
                        Ok(None)
                    }
                    _ => Err(RelayError::UnexpectedMessage),
                }
            }
            Message::Ack(Acknowledgement::Cancel(attempt)) if matches!(self.state.phase, Phase::Cancelling { attempt: active, .. } if active == attempt) =>
            {
                self.state.phase = Phase::Retiring;
                Ok(Some(ClientEvent::Cancelled(attempt)))
            }
            Message::Ack(Acknowledgement::Close)
                if self.state.closing && !matches!(self.state.phase, Phase::Cancelling { .. }) =>
            {
                self.state.phase = Phase::Closed;
                Ok(Some(ClientEvent::Closed))
            }
            _ => Err(RelayError::UnexpectedMessage),
        }
    }
}

#[cfg(test)]
mod tests;
