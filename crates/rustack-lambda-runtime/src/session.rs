//! A single actor owns each environment's invocation and long-poll state.
//!
//! This module owns only in-process state/channels. Transport adapters own HTTP
//! connections and the environment supervisor owns its process/container/VM.
//! Session cancellation invalidates an attempt; it never proves user-code exit.

use std::{fmt, future::pending, mem::replace, time::Duration};

use thiserror::Error;
use tokio::{
    runtime::Handle,
    sync::{
        mpsc,
        oneshot::{self, error::TryRecvError},
        watch,
    },
    task::JoinHandle,
    time::{Instant, sleep_until, timeout_at},
};
use typed_builder::TypedBuilder;

use crate::protocol::{AttemptId, FunctionArn, Outcome, Payload, TraceId};

const MAILBOX_CAPACITY: usize = 8;
const MAX_IDLE_LEASE: Duration = Duration::from_hours(24);

/// Bounded session errors never include function inputs or peer-supplied values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum SessionError {
    /// Actor stopped or an owner disappeared.
    #[error("runtime session unavailable")]
    Unavailable,
    /// One invocation/poll is already active, or the mailbox is full.
    #[error("runtime session busy")]
    Busy,
    /// Completion does not identify the delivered active attempt.
    #[error("unknown runtime invocation attempt")]
    UnknownAttempt,
    /// Command, idle lease or attempt deadline expired.
    #[error("runtime session deadline exceeded")]
    Deadline,
    /// Caller or environment owner cancelled the session.
    #[error("runtime session cancelled")]
    Cancelled,
    /// A lease/deadline cannot be represented or is outside its range.
    #[error("invalid runtime session metadata")]
    InvalidMetadata,
    /// The owned actor panicked or failed to join.
    #[error("runtime session supervisor failed")]
    SupervisorFailed,
}

/// Captured, validated event and runtime metadata for a single attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Invocation {
    attempt: AttemptId,
    deadline: Instant,
    function_arn: FunctionArn,
    trace_id: TraceId,
    payload: Payload,
}

impl Invocation {
    /// Capture one event and its absolute monotonic deadline.
    ///
    /// # Errors
    /// Returns Deadline if the attempt has already expired.
    ///
    /// ```
    /// # use rustack_lambda_runtime::{protocol::{AttemptId, Payload}, session::Invocation};
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let call = Invocation::new(
    ///     AttemptId::default(), tokio::time::Instant::now() + std::time::Duration::from_secs(5),
    ///     "arn:aws:lambda:us-east-1:000000000000:function:echo".parse()?,
    ///     "Root=1-00000000-000000000000000000000000;Sampled=0".parse()?,
    ///     Payload::try_from(bytes::Bytes::from_static(b"{}"))?,
    /// )?;
    /// # Ok(()) }
    /// ```
    pub fn new(
        attempt: AttemptId,
        deadline: Instant,
        function_arn: FunctionArn,
        trace_id: TraceId,
        payload: Payload,
    ) -> Result<Self, SessionError> {
        check_deadline(deadline)?;
        Ok(Self {
            attempt,
            deadline,
            function_arn,
            trace_id,
            payload,
        })
    }

    /// Borrow all event metadata and payload without copying bytes.
    ///
    /// ```
    /// # fn inspect(call: &rustack_lambda_runtime::session::Invocation) {
    /// let (attempt, deadline, arn, trace, payload) = call.parts();
    /// # }
    /// ```
    #[must_use]
    pub fn parts(&self) -> (AttemptId, Instant, &FunctionArn, &TraceId, &Payload) {
        (
            self.attempt,
            self.deadline,
            &self.function_arn,
            &self.trace_id,
            &self.payload,
        )
    }
}

/// One accepted runtime result; raw payload diagnostics are always redacted.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RuntimeResult {
    /// Function /response.
    Success(Payload),
    /// Function /error.
    FunctionError(Payload),
    /// Fatal initialization error before any event delivery.
    InitError(Payload),
}

/// Attached ownership of a session attempt; Drop invalidates only this attempt.
#[non_exhaustive]
pub struct InvocationTicket {
    attempt: AttemptId,
    deadline: Instant,
    cancellation: watch::Sender<bool>,
    response: oneshot::Receiver<Result<RuntimeResult, SessionError>>,
    attached: bool,
}

impl fmt::Debug for InvocationTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InvocationTicket")
            .field("attempt", &self.attempt)
            .field("deadline", &self.deadline)
            .finish_non_exhaustive()
    }
}

impl InvocationTicket {
    /// Identity used by the delivered Runtime API event.
    ///
    /// ```
    /// # fn inspect(ticket: &rustack_lambda_runtime::session::InvocationTicket) {
    /// let attempt = ticket.attempt();
    /// # }
    /// ```
    #[must_use]
    pub fn attempt(&self) -> AttemptId {
        self.attempt
    }

    /// Request invalidation; the environment supervisor still owns termination.
    ///
    /// ```
    /// # fn stop(ticket: &rustack_lambda_runtime::session::InvocationTicket) {
    /// ticket.cancel();
    /// # }
    /// ```
    pub fn cancel(&self) {
        self.cancellation.send_replace(true);
    }

    /// Await a single result within the attempt's absolute deadline.
    ///
    /// # Errors
    /// Returns cancellation/expiry, session failure or disconnected supervision.
    ///
    /// ```no_run
    /// # async fn finish(ticket: rustack_lambda_runtime::session::InvocationTicket) {
    /// let result = ticket.wait().await;
    /// # }
    /// ```
    pub async fn wait(mut self) -> Result<RuntimeResult, SessionError> {
        check_deadline(self.deadline)?;
        if *self.cancellation.borrow() {
            return Err(SessionError::Cancelled);
        }
        let result = timeout_at(self.deadline, &mut self.response)
            .await
            .map_err(|_| SessionError::Deadline)?
            .map_err(|_| SessionError::Unavailable)?;
        check_deadline(self.deadline)?;
        if *self.cancellation.borrow() {
            return Err(SessionError::Cancelled);
        }
        self.attached = false;
        result
    }
}

impl Drop for InvocationTicket {
    fn drop(&mut self) {
        if self.attached {
            self.cancel();
        }
    }
}

/// Cloneable bounded command port; no mutable invocation state is shared.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SessionHandle {
    inbox: mpsc::Sender<Envelope>,
    stop: watch::Sender<bool>,
    bootstrap: watch::Receiver<bool>,
}

/// Owner of the actor and take-once initialization-error notification.
#[derive(Debug)]
#[non_exhaustive]
pub struct RuntimeSession {
    handle: SessionHandle,
    task: Option<JoinHandle<()>>,
    initialization_error: Option<oneshot::Receiver<Payload>>,
    settlement: Option<Result<(), SessionError>>,
}

impl RuntimeSession {
    /// Start a single-invocation actor with a bounded idle lease.
    ///
    /// # Errors
    /// Rejects a zero/over24h lease, unrepresentable deadline or missing Tokio runtime.
    ///
    /// ```no_run
    /// # async fn example() -> Result<(), rustack_lambda_runtime::session::SessionError> {
    /// let mut session = rustack_lambda_runtime::session::RuntimeSession::start(std::time::Duration::from_secs(60))?;
    /// session.shutdown().await?;
    /// # Ok(()) }
    /// ```
    pub fn start(idle_lease: Duration) -> Result<Self, SessionError> {
        if idle_lease.is_zero() || idle_lease > MAX_IDLE_LEASE {
            return Err(SessionError::InvalidMetadata);
        }
        let runtime = Handle::try_current().map_err(|_| SessionError::Unavailable)?;
        let idle_deadline = Instant::now()
            .checked_add(idle_lease)
            .ok_or(SessionError::InvalidMetadata)?;
        let (inbox, commands) = mpsc::channel(MAILBOX_CAPACITY);
        let (stop, stopped) = watch::channel(false);
        let (bootstrap, initialized) = watch::channel(false);
        let (initialization_error, error) = oneshot::channel();
        let actor = SessionActor::builder()
            .inbox(commands)
            .stop(stopped)
            .bootstrap(bootstrap)
            .initialization_error(Some(initialization_error))
            .idle_lease(idle_lease)
            .idle_deadline(idle_deadline)
            .build();
        Ok(Self {
            handle: SessionHandle {
                inbox,
                stop,
                bootstrap: initialized,
            },
            task: Some(runtime.spawn(actor.run())),
            initialization_error: Some(error),
            settlement: None,
        })
    }

    /// Borrow the actor's command port.
    ///
    /// ```
    /// # fn inspect(session: &rustack_lambda_runtime::session::RuntimeSession) {
    /// let handle = session.handle().clone();
    /// # }
    /// ```
    #[must_use]
    pub fn handle(&self) -> &SessionHandle {
        &self.handle
    }

    /// Transfer the initialization notification exactly once.
    ///
    /// ```
    /// # fn inspect(session: &mut rustack_lambda_runtime::session::RuntimeSession) {
    /// let notification = session.take_initialization_error();
    /// assert!(session.take_initialization_error().is_none());
    /// # }
    /// ```
    pub fn take_initialization_error(&mut self) -> Option<oneshot::Receiver<Payload>> {
        self.initialization_error.take()
    }

    /// Signal stop and join the owned actor; cancelling a waiter retains its handle.
    ///
    /// # Errors
    /// Returns `SupervisorFailed` if the actor panicked or failed to join.
    ///
    /// ```no_run
    /// # async fn stop(session: &mut rustack_lambda_runtime::session::RuntimeSession) {
    /// let clean = session.shutdown().await.is_ok();
    /// # }
    /// ```
    pub async fn shutdown(&mut self) -> Result<(), SessionError> {
        self.handle.close();
        if let Some(result) = self.settlement {
            return result;
        }
        let result = if let Some(task) = self.task.as_mut() {
            task.await.map_err(|_| SessionError::SupervisorFailed)
        } else {
            Ok(())
        };
        self.task.take();
        self.settlement = Some(result);
        result
    }
}

impl Drop for RuntimeSession {
    fn drop(&mut self) {
        self.handle.close();
    }
}

impl SessionHandle {
    fn send(&self, command: Command, deadline: Instant) -> Result<(), SessionError> {
        check_deadline(deadline)?;
        if *self.stop.borrow() {
            return Err(SessionError::Unavailable);
        }
        self.inbox
            .try_send(Envelope { deadline, command })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => SessionError::Busy,
                mpsc::error::TrySendError::Closed(_) => SessionError::Unavailable,
            })
    }

    /// Admit one event; a dropped admission future cancels its own attempt.
    ///
    /// # Errors
    /// Returns Busy, cancellation/expiry or unavailable supervision.
    ///
    /// ```no_run
    /// # async fn submit(handle: &rustack_lambda_runtime::session::SessionHandle, call: rustack_lambda_runtime::session::Invocation) {
    /// let deadline = call.parts().1;
    /// let ticket = handle.submit(call, deadline).await;
    /// # }
    /// ```
    pub async fn submit(
        &self,
        invocation: Invocation,
        deadline: Instant,
    ) -> Result<InvocationTicket, SessionError> {
        check_deadline(deadline)?;
        check_deadline(invocation.deadline)?;
        let (cancellation, cancelled) = watch::channel(false);
        let (response, result) = oneshot::channel();
        let (admitted, accepted) = oneshot::channel();
        let ticket = InvocationTicket {
            attempt: invocation.attempt,
            deadline: invocation.deadline,
            cancellation,
            response: result,
            attached: true,
        };
        self.send(
            Command::Submit {
                invocation,
                cancellation: cancelled,
                response,
                admitted,
            },
            deadline,
        )?;
        receive(accepted, deadline).await??;
        Ok(ticket)
    }

    /// Wait for the next event, allowing exactly one outstanding poll.
    ///
    /// # Errors
    /// Returns Busy for a concurrent poll/active delivery, expiry or closed session.
    ///
    /// ```no_run
    /// # async fn poll(handle: &rustack_lambda_runtime::session::SessionHandle) {
    /// let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
    /// let event = handle.next(deadline).await;
    /// # }
    /// ```
    pub async fn next(&self, deadline: Instant) -> Result<Invocation, SessionError> {
        check_deadline(deadline)?;
        let (reply, response) = oneshot::channel();
        self.send(Command::Next { reply }, deadline)?;
        let delivery = receive(response, deadline).await??;
        delivery
            .receipt
            .send(())
            .map_err(|()| SessionError::Unavailable)?;
        Ok(delivery.invocation)
    }

    /// Validate and settle exactly one delivered attempt.
    ///
    /// # Errors
    /// Rejects stale/mismatched identities, deadline expiry, overload or closure.
    ///
    /// ```no_run
    /// # async fn complete(handle: &rustack_lambda_runtime::session::SessionHandle, attempt: rustack_lambda_runtime::protocol::AttemptId, payload: rustack_lambda_runtime::protocol::Payload) {
    /// let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    /// let result = handle.complete(attempt, None, rustack_lambda_runtime::protocol::Outcome::Success, payload, deadline).await;
    /// # }
    /// ```
    pub async fn complete(
        &self,
        attempt: AttemptId,
        invocation_id: Option<AttemptId>,
        outcome: Outcome,
        payload: Payload,
        deadline: Instant,
    ) -> Result<(), SessionError> {
        check_deadline(deadline)?;
        let (reply, response) = oneshot::channel();
        self.send(
            Command::Complete {
                attempt,
                invocation_id,
                outcome,
                payload,
                reply,
            },
            deadline,
        )?;
        receive(response, deadline).await?
    }

    /// Report a fatal initialization failure before any event delivery.
    ///
    /// # Errors
    /// Returns Busy after delivery, or command deadline/overload/session failure.
    ///
    /// ```no_run
    /// # async fn fail(handle: &rustack_lambda_runtime::session::SessionHandle, payload: rustack_lambda_runtime::protocol::Payload) {
    /// let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    /// let result = handle.initialization_error(payload, deadline).await;
    /// # }
    /// ```
    pub async fn initialization_error(
        &self,
        payload: Payload,
        deadline: Instant,
    ) -> Result<(), SessionError> {
        check_deadline(deadline)?;
        let (reply, response) = oneshot::channel();
        self.send(Command::InitError { payload, reply }, deadline)?;
        receive(response, deadline).await?
    }

    /// Await the first valid GET /next; listener readiness cannot satisfy this.
    ///
    /// # Errors
    /// Returns deadline expiry or closure before the first poll.
    ///
    /// ```no_run
    /// # async fn ready(handle: &rustack_lambda_runtime::session::SessionHandle) {
    /// let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    /// let ready = handle.bootstrap_ready(deadline).await;
    /// # }
    /// ```
    pub async fn bootstrap_ready(&self, deadline: Instant) -> Result<(), SessionError> {
        check_deadline(deadline)?;
        let mut bootstrap = self.bootstrap.clone();
        timeout_at(deadline, bootstrap.wait_for(|ready| *ready))
            .await
            .map_err(|_| SessionError::Deadline)?
            .map_err(|_| SessionError::Unavailable)?;
        check_deadline(deadline)
    }

    /// Signal priority shutdown without waiting for the ordinary command mailbox.
    ///
    /// ```
    /// # fn stop(handle: &rustack_lambda_runtime::session::SessionHandle) {
    /// handle.close();
    /// # }
    /// ```
    pub fn close(&self) {
        self.stop.send_replace(true);
    }

    /// Wait until the actor closes its command receiver.
    ///
    /// ```no_run
    /// # async fn closed(handle: &rustack_lambda_runtime::session::SessionHandle) {
    /// handle.closed().await;
    /// # }
    /// ```
    pub async fn closed(&self) {
        self.inbox.closed().await;
    }

    /// Whether admission is closed or its owner has requested stop.
    ///
    /// ```
    /// # fn inspect(handle: &rustack_lambda_runtime::session::SessionHandle) {
    /// let closed = handle.is_closed();
    /// # }
    /// ```
    #[must_use]
    pub fn is_closed(&self) -> bool {
        *self.stop.borrow() || self.inbox.is_closed()
    }
}

fn check_deadline(deadline: Instant) -> Result<(), SessionError> {
    if Instant::now() >= deadline {
        Err(SessionError::Deadline)
    } else {
        Ok(())
    }
}

async fn receive<T>(response: oneshot::Receiver<T>, deadline: Instant) -> Result<T, SessionError> {
    let result = timeout_at(deadline, response)
        .await
        .map_err(|_| SessionError::Deadline)?
        .map_err(|_| SessionError::Unavailable)?;
    check_deadline(deadline)?;
    Ok(result)
}

#[derive(Debug)]
struct Envelope {
    deadline: Instant,
    command: Command,
}

#[derive(Debug)]
enum Command {
    Submit {
        invocation: Invocation,
        cancellation: watch::Receiver<bool>,
        response: oneshot::Sender<Result<RuntimeResult, SessionError>>,
        admitted: oneshot::Sender<Result<(), SessionError>>,
    },
    Next {
        reply: oneshot::Sender<Result<PollDelivery, SessionError>>,
    },
    Complete {
        attempt: AttemptId,
        invocation_id: Option<AttemptId>,
        outcome: Outcome,
        payload: Payload,
        reply: oneshot::Sender<Result<(), SessionError>>,
    },
    InitError {
        payload: Payload,
        reply: oneshot::Sender<Result<(), SessionError>>,
    },
}

#[derive(Debug)]
struct Pending {
    attempt: AttemptId,
    deadline: Instant,
    cancellation: watch::Receiver<bool>,
    response: oneshot::Sender<Result<RuntimeResult, SessionError>>,
}

impl Pending {
    fn check(&self) -> Result<(), SessionError> {
        check_deadline(self.deadline)?;
        if *self.cancellation.borrow() || self.cancellation.has_changed().is_err() {
            Err(SessionError::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug)]
struct PollDelivery {
    invocation: Invocation,
    receipt: oneshot::Sender<()>,
}

#[derive(Debug)]
enum PhaseEvent {
    Cancelled,
    Receipt(bool),
}

#[derive(Debug, Default)]
enum Phase {
    #[default]
    Idle,
    Queued {
        invocation: Invocation,
        pending: Pending,
    },
    Offering {
        invocation: Invocation,
        pending: Pending,
        receipt: oneshot::Receiver<()>,
    },
    Delivered(Pending),
    Closed,
}

impl Phase {
    fn pending(&self) -> Option<&Pending> {
        match self {
            Self::Queued { pending, .. }
            | Self::Offering { pending, .. }
            | Self::Delivered(pending) => Some(pending),
            _ => None,
        }
    }

    async fn event(&mut self) -> PhaseEvent {
        match self {
            Self::Offering {
                pending, receipt, ..
            } => {
                tokio::select! {
                    biased;
                    () = async { let _ = pending.cancellation.wait_for(|cancelled| *cancelled).await; } => PhaseEvent::Cancelled,
                    result = receipt => PhaseEvent::Receipt(result.is_ok()),
                }
            }
            Self::Queued { pending, .. } | Self::Delivered(pending) => {
                let _ = pending.cancellation.wait_for(|cancelled| *cancelled).await;
                PhaseEvent::Cancelled
            }
            _ => pending().await,
        }
    }
}

#[derive(Debug, TypedBuilder)]
struct SessionActor {
    inbox: mpsc::Receiver<Envelope>,
    stop: watch::Receiver<bool>,
    bootstrap: watch::Sender<bool>,
    initialization_error: Option<oneshot::Sender<Payload>>,
    idle_lease: Duration,
    idle_deadline: Instant,
    #[builder(default)]
    phase: Phase,
    #[builder(default)]
    poll: Option<oneshot::Sender<Result<PollDelivery, SessionError>>>,
    #[builder(default)]
    ever_delivered: bool,
}

impl SessionActor {
    async fn run(mut self) {
        loop {
            let deadline = self
                .phase
                .pending()
                .map_or(self.idle_deadline, |pending| pending.deadline);
            tokio::select! {
                biased;
                () = async { let _ = self.stop.wait_for(|stopped| *stopped).await; } => { self.close(SessionError::Cancelled); break; }
                () = sleep_until(deadline) => { self.close(SessionError::Deadline); break; }
                event = self.phase.event() => {
                    match event {
                        PhaseEvent::Cancelled => { self.close(SessionError::Cancelled); break; }
                        PhaseEvent::Receipt(received) => self.received(received),
                    }
                }
                command = self.inbox.recv() => {
                    let Some(Envelope { command, deadline }) = command else { self.close(SessionError::Unavailable); break; };
                    if let Err(error) = check_deadline(deadline) { reject(command, error); continue; }
                    if self.command(command) {
                        let Some(deadline) = Instant::now().checked_add(self.idle_lease) else { self.close(SessionError::InvalidMetadata); break; };
                        self.idle_deadline = deadline;
                    }
                    if matches!(self.phase, Phase::Closed) { break; }
                    self.deliver();
                }
            }
        }
    }

    fn command(&mut self, command: Command) -> bool {
        self.reconcile_receipt();
        let error = if *self.stop.borrow() {
            Some(SessionError::Cancelled)
        } else if matches!(self.phase, Phase::Closed) {
            Some(SessionError::Unavailable)
        } else {
            self.phase
                .pending()
                .and_then(|pending| pending.check().err())
        };
        if let Some(error) = error {
            self.close(error);
            reject(command, error);
            return false;
        }
        match command {
            Command::Submit {
                invocation,
                cancellation,
                response,
                admitted,
            } => {
                let error = if !matches!(self.phase, Phase::Idle) {
                    Some(SessionError::Busy)
                } else if check_deadline(invocation.deadline).is_err() {
                    Some(SessionError::Deadline)
                } else if *cancellation.borrow() || cancellation.has_changed().is_err() {
                    Some(SessionError::Cancelled)
                } else {
                    None
                };
                if let Some(error) = error {
                    let _ = admitted.send(Err(error));
                    return false;
                }
                let pending = Pending {
                    attempt: invocation.attempt,
                    deadline: invocation.deadline,
                    cancellation,
                    response,
                };
                self.phase = Phase::Queued {
                    invocation,
                    pending,
                };
                let _ = admitted.send(Ok(()));
                true
            }
            Command::Next { reply } => {
                if matches!(self.phase, Phase::Offering { .. } | Phase::Delivered(_))
                    || self.poll.as_ref().is_some_and(|poll| !poll.is_closed())
                {
                    let _ = reply.send(Err(SessionError::Busy));
                    return false;
                }
                if reply.is_closed() {
                    return false;
                }
                self.bootstrap.send_if_modified(|ready| {
                    let changed = !*ready;
                    *ready = true;
                    changed
                });
                self.poll = Some(reply);
                true
            }
            Command::Complete {
                attempt,
                invocation_id,
                outcome,
                payload,
                reply,
            } => self.complete(attempt, invocation_id, outcome, payload, reply),
            Command::InitError { payload, reply } => {
                if let Some(error) = self
                    .phase
                    .pending()
                    .and_then(|pending| pending.check().err())
                {
                    self.close(error);
                    let _ = reply.send(Err(error));
                    return false;
                }
                if self.ever_delivered {
                    let _ = reply.send(Err(SessionError::Busy));
                    return false;
                }
                if let Some(notification) = self.initialization_error.take() {
                    let _ = notification.send(payload.clone());
                }
                if let Some(pending) = self.take_pending() {
                    let _ = pending.response.send(Ok(RuntimeResult::InitError(payload)));
                }
                self.close(SessionError::Unavailable);
                let _ = reply.send(Ok(()));
                true
            }
        }
    }

    fn reconcile_receipt(&mut self) {
        let received = match &mut self.phase {
            Phase::Offering { receipt, .. } => match receipt.try_recv() {
                Ok(()) => Some(true),
                Err(TryRecvError::Closed) => Some(false),
                Err(TryRecvError::Empty) => None,
            },
            _ => None,
        };
        if let Some(received) = received {
            self.received(received);
        }
    }

    fn complete(
        &mut self,
        attempt: AttemptId,
        invocation_id: Option<AttemptId>,
        outcome: Outcome,
        payload: Payload,
        reply: oneshot::Sender<Result<(), SessionError>>,
    ) -> bool {
        let valid = matches!(&self.phase, Phase::Delivered(pending) if pending.attempt == attempt)
            && invocation_id.is_none_or(|id| id == attempt);
        if !valid {
            let _ = reply.send(Err(SessionError::UnknownAttempt));
            return false;
        }
        let Phase::Delivered(pending) = replace(&mut self.phase, Phase::Idle) else {
            let _ = reply.send(Err(SessionError::Unavailable));
            return false;
        };
        let result = if let Err(error) = pending.check() {
            let _ = pending.response.send(Err(error));
            self.close(error);
            Err(error)
        } else {
            let result = match outcome {
                Outcome::Success => RuntimeResult::Success(payload),
                Outcome::FunctionError => RuntimeResult::FunctionError(payload),
            };
            if pending.response.send(Ok(result)).is_err() {
                self.close(SessionError::Cancelled);
                Err(SessionError::Cancelled)
            } else {
                Ok(())
            }
        };
        let accepted = result.is_ok();
        let _ = reply.send(result);
        accepted
    }

    fn deliver(&mut self) {
        if !matches!(self.phase, Phase::Queued { .. }) {
            return;
        }
        let Some(poll) = self.poll.take() else {
            return;
        };
        let Phase::Queued {
            invocation,
            pending,
        } = replace(&mut self.phase, Phase::Idle)
        else {
            return;
        };
        if let Err(error) = pending.check() {
            let _ = poll.send(Err(error));
            let _ = pending.response.send(Err(error));
            self.close(error);
            return;
        }
        let (receipt, received) = oneshot::channel();
        match poll.send(Ok(PollDelivery {
            invocation: invocation.clone(),
            receipt,
        })) {
            Ok(()) => {
                self.phase = Phase::Offering {
                    invocation,
                    pending,
                    receipt: received,
                };
            }
            Err(_) => {
                self.phase = Phase::Queued {
                    invocation,
                    pending,
                };
            }
        }
    }

    fn received(&mut self, received: bool) {
        let Phase::Offering {
            invocation,
            pending,
            ..
        } = replace(&mut self.phase, Phase::Idle)
        else {
            return;
        };
        if let Err(error) = pending.check() {
            let _ = pending.response.send(Err(error));
            self.close(error);
            return;
        }
        if received {
            self.ever_delivered = true;
            self.phase = Phase::Delivered(pending);
        } else {
            self.phase = Phase::Queued {
                invocation,
                pending,
            };
        }
    }

    fn take_pending(&mut self) -> Option<Pending> {
        match replace(&mut self.phase, Phase::Closed) {
            Phase::Queued { pending, .. }
            | Phase::Offering { pending, .. }
            | Phase::Delivered(pending) => Some(pending),
            _ => None,
        }
    }

    fn close(&mut self, error: SessionError) {
        if let Some(pending) = self.take_pending() {
            let _ = pending.response.send(Err(error));
        }
        if let Some(poll) = self.poll.take() {
            let _ = poll.send(Err(error));
        }
        self.inbox.close();
    }
}

fn reject(command: Command, error: SessionError) {
    match command {
        Command::Submit {
            admitted, response, ..
        } => {
            let _ = admitted.send(Err(error));
            let _ = response.send(Err(error));
        }
        Command::Next { reply } => {
            let _ = reply.send(Err(error));
        }
        Command::Complete { reply, .. } | Command::InitError { reply, .. } => {
            let _ = reply.send(Err(error));
        }
    }
}

#[cfg(test)]
mod tests;
