//! Bounded callback token ownership, provisional completion and exact timers.

use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use tokio::{
    runtime::Handle,
    sync::{Notify, OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch},
    task::JoinHandle,
    time::{Instant, timeout_at},
};

use super::domain;
use crate::{TaskFailure, TenantScope};

mod actor;
mod receipts;
#[cfg(test)]
mod tests;
mod token;
mod types;

pub use receipts::{CallbackAck, CallbackMint, CallbackTask};
pub use token::CallbackToken;
use token::TokenHash;
use types::Action;
pub use types::{
    CallbackAttempt, CallbackCommand, CallbackError, CallbackLimits, CallbackOutcome,
    CallbackStatus, CallbackTimeout,
};

#[derive(Debug)]
struct Control {
    revoked: AtomicBool,
    wake: Arc<Notify>,
}
impl Control {
    fn revoke(&self) {
        if !self.revoked.swap(true, Ordering::AcqRel) {
            self.wake.notify_one();
        }
    }
    fn revoked(&self) -> bool {
        self.revoked.load(Ordering::Acquire)
    }
}
#[derive(Debug, Clone)]
struct Invocation {
    outcome: Result<(), TaskFailure>,
    observed: Instant,
}
#[derive(Debug)]
struct Reservations {
    live: OwnedSemaphorePermit,
    retained: OwnedSemaphorePermit,
    command: OwnedSemaphorePermit,
}
#[derive(Debug)]
struct MintRequest {
    attempt: CallbackAttempt,
    control: Arc<Control>,
    invocation: watch::Sender<Option<Invocation>>,
    invoked: watch::Receiver<Option<Invocation>>,
    reply: oneshot::Sender<MintReply>,
    reservations: Reservations,
}
#[derive(Debug)]
struct MintReply {
    result: Result<CallbackTask, Arc<CallbackError>>,
    _permit: OwnedSemaphorePermit,
}
#[derive(Debug)]
struct AckReply {
    result: Result<(), Arc<CallbackError>>,
    _permit: OwnedSemaphorePermit,
}
#[derive(Debug)]
struct ApiRequest {
    digest: TokenHash,
    action: Action,
    reply: oneshot::Sender<AckReply>,
    permit: OwnedSemaphorePermit,
}
#[derive(Debug)]
enum Request {
    #[cfg(test)]
    Inspect(oneshot::Sender<(usize, usize, usize)>),
    #[cfg(test)]
    Panic,
    Mint(Box<MintRequest>),
    Api(ApiRequest),
}
#[derive(Debug)]
struct Shared {
    #[cfg(test)]
    entropy_hook: std::sync::OnceLock<Arc<tests::EntropyHook>>,
    scope: TenantScope,
    sender: mpsc::Sender<Request>,
    live: Arc<Semaphore>,
    retained: Arc<Semaphore>,
    commands: Arc<Semaphore>,
    closing: watch::Sender<bool>,
    wake: Arc<Notify>,
    status: watch::Receiver<CallbackStatus>,
}
/// Scoped bounded callback admission; accepted mutations outlive API observers.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct CallbackRegistry {
    shared: Arc<Shared>,
}
impl CallbackRegistry {
    /// Start the unique callback actor with bounded entropy workers and commands.
    ///
    /// # Errors
    /// Requires an active Tokio runtime; trusted scope/limits are prevalidated.
    ///
    /// ```no_run
    /// # async fn start()->Result<(),rustack_stepfunctions_core::CallbackError>{
    /// let (callbacks,mut owner)=rustack_stepfunctions_core::CallbackRegistry::new(Default::default(),Default::default())?;
    /// owner.shutdown(tokio::time::Instant::now()+std::time::Duration::from_secs(5)).await?;
    /// # Ok(()) }
    /// ```
    pub fn new(
        scope: TenantScope,
        limits: CallbackLimits,
    ) -> Result<(Self, CallbackSupervisor), CallbackError> {
        if Handle::try_current().is_err() {
            return Err(CallbackError::Unavailable);
        }
        let (sender, receiver) = mpsc::channel(limits.commands);
        let (closing, stop) = watch::channel(false);
        let (status, reader) = watch::channel(CallbackStatus {
            ready: true,
            live: 0,
            retained: 0,
        });
        let shared = Arc::new(Shared {
            #[cfg(test)]
            entropy_hook: std::sync::OnceLock::new(),
            scope,
            sender,
            live: Arc::new(Semaphore::new(limits.live)),
            retained: Arc::new(Semaphore::new(limits.retained)),
            commands: Arc::new(Semaphore::new(limits.commands)),
            closing: closing.clone(),
            wake: Arc::new(Notify::new()),
            status: reader,
        });
        let state = actor::Owner::new(shared.clone(), limits, receiver, status);
        let join = tokio::spawn(state.run(stop));
        Ok((
            Self { shared },
            CallbackSupervisor {
                closing,
                join: Some(join),
                result: None,
            },
        ))
    }
    /// Reserve all bounded ownership before queuing one trusted mint attempt.
    ///
    /// An error before return means no mint request was accepted.
    ///
    /// # Errors
    /// Rejects foreign/closed parents, expired deadlines and exhausted/closed lanes.
    ///
    /// ```no_run
    /// # fn register(r:&rustack_stepfunctions_core::CallbackRegistry,a:rustack_stepfunctions_core::CallbackAttempt){let receipt=r.register(a);}
    /// ```
    pub fn register(&self, attempt: CallbackAttempt) -> Result<CallbackMint, CallbackError> {
        self.accepting()?;
        if !attempt.scoped(&self.shared.scope) {
            return Err(CallbackError::Validation);
        }
        attempt.live(Instant::now())?;
        let reservations = Reservations {
            command: self.command_permit()?,
            live: self
                .shared
                .live
                .clone()
                .try_acquire_owned()
                .map_err(|_| CallbackError::Capacity)?,
            retained: self
                .shared
                .retained
                .clone()
                .try_acquire_owned()
                .map_err(|_| CallbackError::Capacity)?,
        };
        let control = Arc::new(Control {
            revoked: AtomicBool::new(false),
            wake: self.shared.wake.clone(),
        });
        let (invocation, invoked) = watch::channel(None);
        let (reply, receiver) = oneshot::channel();
        self.send(Request::Mint(Box::new(MintRequest {
            attempt,
            control: control.clone(),
            invocation,
            invoked,
            reply,
            reservations,
        })))?;
        Ok(CallbackMint::new(receiver, control))
    }
    /// Accept one prepared mutation using trusted authenticated caller scope.
    ///
    /// Dropping the acknowledgment waiter never undoes this accepted action.
    ///
    /// # Errors
    /// Rejects foreign scope, exhausted command capacity or closed ownership.
    ///
    /// ```no_run
    /// # fn submit(r:&rustack_stepfunctions_core::CallbackRegistry,c:rustack_stepfunctions_core::CallbackCommand){let receipt=r.submit(&Default::default(),c);}
    /// ```
    pub fn submit(
        &self,
        scope: &TenantScope,
        command: CallbackCommand,
    ) -> Result<CallbackAck, CallbackError> {
        if !domain::same_scope(scope, &self.shared.scope) {
            return Err(CallbackError::InvalidToken);
        }
        self.accepting()?;
        let permit = self.command_permit()?;
        let digest = command.token.digest();
        let (reply, receiver) = oneshot::channel();
        self.send(Request::Api(ApiRequest {
            digest,
            action: command.action,
            reply,
            permit,
        }))?;
        Ok(CallbackAck::new(receiver))
    }
    /// Capture root health and issued live/retained counts without credentials.
    ///
    /// ```
    /// # fn inspect(r:&rustack_stepfunctions_core::CallbackRegistry){let status=r.status();}
    /// ```
    #[must_use]
    pub fn status(&self) -> CallbackStatus {
        let mut status = *self.shared.status.borrow();
        status.ready &= !*self.shared.closing.borrow() && self.shared.status.has_changed().is_ok();
        status
    }
    fn accepting(&self) -> Result<(), CallbackError> {
        if self.status().ready {
            Ok(())
        } else {
            Err(CallbackError::Unavailable)
        }
    }
    fn command_permit(&self) -> Result<OwnedSemaphorePermit, CallbackError> {
        self.shared
            .commands
            .clone()
            .try_acquire_owned()
            .map_err(|_| CallbackError::Capacity)
    }
    fn send(&self, request: Request) -> Result<(), CallbackError> {
        self.shared
            .sender
            .try_send(request)
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => CallbackError::Capacity,
                mpsc::error::TrySendError::Closed(_) => CallbackError::Unavailable,
            })
    }
}
/// Unique joined callback root controller; Drop closes without aborting workers.
#[non_exhaustive]
pub struct CallbackSupervisor {
    closing: watch::Sender<bool>,
    join: Option<JoinHandle<Result<(), CallbackError>>>,
    result: Option<Result<(), Arc<CallbackError>>>,
}
impl fmt::Debug for CallbackSupervisor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CallbackSupervisor(<redacted>)")
    }
}
impl CallbackSupervisor {
    /// Reject new work and request root drain exactly once.
    ///
    /// ```
    /// # fn close(owner:&rustack_stepfunctions_core::CallbackSupervisor){owner.close();}
    /// ```
    pub fn close(&self) {
        self.closing.send_replace(true);
    }
    /// Join the same root through an observer deadline without aborting entropy.
    ///
    /// # Errors
    /// Returns Deadline while the join is retained, or a cached fatal worker error.
    ///
    /// ```no_run
    /// # async fn close(owner:&mut rustack_stepfunctions_core::CallbackSupervisor,deadline:tokio::time::Instant){let result=owner.shutdown(deadline).await;}
    /// ```
    pub async fn shutdown(&mut self, deadline: Instant) -> Result<(), CallbackError> {
        self.close();
        if Instant::now() >= deadline {
            return Err(CallbackError::Deadline);
        }
        if self.result.is_none() {
            let join = self.join.as_mut().ok_or(CallbackError::Unavailable)?;
            let result = timeout_at(deadline, join)
                .await
                .map_err(|_| CallbackError::Deadline)?;
            self.join = None;
            self.result = Some(match result {
                Ok(result) => result.map_err(Arc::new),
                Err(error) => Err(Arc::new(CallbackError::Worker(error))),
            });
        }
        if Instant::now() >= deadline {
            return Err(CallbackError::Deadline);
        }
        match self.result.as_ref() {
            Some(Ok(())) => Ok(()),
            Some(Err(error)) => Err(CallbackError::Cached(error.clone())),
            None => Err(CallbackError::Unavailable),
        }
    }
}
impl Drop for CallbackSupervisor {
    fn drop(&mut self) {
        self.close();
    }
}
