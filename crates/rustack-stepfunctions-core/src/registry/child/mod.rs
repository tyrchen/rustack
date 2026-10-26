//! Bounded atomic child admission; weak registry linkage and retained cleanup.

use std::{
    fmt,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

use rustack_core::json::JsonError;
use thiserror::Error;
use tokio::{
    sync::{Notify, OwnedSemaphorePermit, mpsc, oneshot},
    time::{Instant, timeout_at},
};

use super::{
    ExecutionArn, ExecutionName, ExecutionSnapshot, Registry, RegistryError, Shared,
    StateMachineArn, types::ExecutionRecord,
};
use crate::{AslError, ExecutionError, ExecutionFailure, ExecutionView, Payload, TaskFailure};

mod envelope;
pub(super) mod owner;
mod parameters;

/// Optimized nested integration behavior selected by the trusted caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ChildPattern {
    /// Detach at successful start acceptance.
    RequestResponse,
    /// Await termination; return Input/Output as serialized JSON strings.
    Sync,
    /// Await termination; return Input/Output as JSON values.
    Sync2,
}
impl ChildPattern {
    pub(super) fn owned(self) -> bool {
        match self {
            Self::RequestResponse => false,
            Self::Sync | Self::Sync2 => true,
        }
    }
}
/// Typed child admission/observation failures; public text is static and redacted.
#[derive(Error)]
#[non_exhaustive]
pub enum ChildError {
    /// Malformed evaluated parameters or an invalid captured state.
    #[error("invalid child execution parameters")]
    Invalid,
    /// Parent capability is foreign or no longer RUNNING.
    #[error("child execution parent unavailable")]
    Parent,
    /// The bounded depth or independent child-call pool is exhausted.
    #[error("child execution capacity exhausted")]
    Capacity,
    /// An owned dependency would join itself or form a graph cycle.
    #[error("cyclic child execution dependency")]
    Cycle,
    /// The caller cancelled before issuance.
    #[error("child execution admission cancelled")]
    Cancelled,
    /// Observer expiry never cancels accepted work.
    #[error("child execution observation expired")]
    Deadline,
    /// Closed admission rejected the request before issuance.
    #[error("child execution admission unavailable")]
    Unavailable,
    /// Lost reply ownership cannot prove whether a child was issued.
    #[error("child execution reply owner lost")]
    OwnerLost,
    /// A job result requires observed child termination.
    #[error("child execution is not terminal")]
    Pending,
    /// Registry rejection with a retained typed source.
    #[error("child execution registration failed")]
    Registry(
        #[from]
        #[source]
        RegistryError,
    ),
    /// Cached admission failure, retained across late/expired observations.
    #[error("child execution admission failed")]
    Admission(#[source] Arc<ChildError>),
    /// Invalid JSON, serialization or bounded data failure.
    #[error("child execution data failed")]
    Data(
        #[from]
        #[source]
        AslError,
    ),
    /// Execution observation/cleanup source.
    #[error("child execution observation failed")]
    Execution(
        #[from]
        #[source]
        ExecutionError,
    ),
}
impl fmt::Debug for ChildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl ChildError {
    /// Map admission/data failure to its bounded optimized Task error.
    ///
    /// ```
    /// assert_eq!(rustack_stepfunctions_core::ChildError::Capacity.failure().details().error(),Some("StepFunctions.ExecutionLimitExceeded"));
    /// ```
    #[must_use]
    pub fn failure(&self) -> TaskFailure {
        let name = match self {
            Self::Admission(source) => return source.failure(),
            Self::Data(
                AslError::DataLimit
                | AslError::Json(JsonError::Bytes | JsonError::String | JsonError::Key),
            ) => "States.DataLimitExceeded",
            Self::Invalid
            | Self::Cycle
            | Self::Registry(RegistryError::Validation)
            | Self::Data(_) => "States.Runtime",
            Self::Capacity
            | Self::Registry(
                RegistryError::Capacity | RegistryError::Execution(ExecutionError::Capacity),
            ) => "StepFunctions.ExecutionLimitExceeded",
            Self::Registry(RegistryError::MachineMissing) => {
                "StepFunctions.StateMachineDoesNotExist"
            }
            Self::Registry(RegistryError::ExecutionExists) => {
                "StepFunctions.ExecutionAlreadyExists"
            }
            Self::Registry(RegistryError::Deleting) => "StepFunctions.StateMachineDeleting",
            _ => "StepFunctions.ServiceException",
        };
        TaskFailure(ExecutionFailure::named(
            name,
            "Child execution request failed",
        ))
    }
}
/// Fully validated optimized parameters; fields cannot be constructed unchecked.
#[non_exhaustive]
pub struct ChildStartParameters {
    pub(super) machine: StateMachineArn,
    pub(super) name: Option<ExecutionName>,
    pub(super) input: Payload,
}
impl fmt::Debug for ChildStartParameters {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ChildStartParameters(<redacted>)")
    }
}
/// Opaque exact parent-record capability; cloning retains its registry charge.
#[derive(Clone)]
#[non_exhaustive]
pub struct ChildParent {
    pub(super) record: Arc<ExecutionRecord>,
}
impl fmt::Debug for ChildParent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ChildParent(<redacted>)")
    }
}
impl ExecutionSnapshot {
    /// Capture an opaque capability; admission rechecks identity and liveness.
    ///
    /// ```
    /// # fn inspect(s:&rustack_stepfunctions_core::ExecutionSnapshot){let parent=s.child_parent();}
    /// ```
    #[must_use]
    pub fn child_parent(&self) -> ChildParent {
        ChildParent {
            record: self.record.clone(),
        }
    }
    /// Read nesting depth, with external Start executions at zero.
    ///
    /// ```
    /// # fn inspect(s:&rustack_stepfunctions_core::ExecutionSnapshot){let depth=s.nesting_depth();}
    /// ```
    #[must_use]
    pub fn nesting_depth(&self) -> usize {
        self.record.lineage.0.len()
    }
    /// Read the original creating parent, independent of later idempotent joins.
    ///
    /// ```
    /// # fn inspect(s:&rustack_stepfunctions_core::ExecutionSnapshot){let parent=s.parent_execution_arn();}
    /// ```
    #[must_use]
    pub fn parent_execution_arn(&self) -> Option<&ExecutionArn> {
        self.record.lineage.0.last()
    }
}
#[derive(Debug, Default)]
pub(super) struct Lineage(pub(super) Vec<ExecutionArn>);
impl Lineage {
    pub(super) fn descendant(parent: &ExecutionRecord) -> Result<Self, ChildError> {
        if parent.lineage.0.len() >= 16 {
            return Err(ChildError::Capacity);
        }
        let mut lineage = parent.lineage.0.clone();
        lineage.push(parent.identity.arn.clone());
        Ok(Self(lineage))
    }
}
#[derive(Debug)]
pub(super) struct Control {
    cancelled: AtomicBool,
    wake: Arc<Notify>,
}
impl Control {
    pub(super) fn cancel(&self) {
        if !self.cancelled.swap(true, Ordering::AcqRel) {
            self.wake.notify_one();
        }
    }
    pub(super) fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}
#[derive(Debug)]
pub(super) struct Context {
    pub(super) parent: ChildParent,
    pub(super) parameters: ChildStartParameters,
    pub(super) pattern: ChildPattern,
    pub(super) deadline: Instant,
}
#[derive(Debug)]
pub(super) struct Request {
    pub(super) context: Context,
    pub(super) control: Arc<Control>,
    pub(super) reply: oneshot::Sender<Result<ChildExecution, ChildError>>,
    pub(super) permit: OwnedSemaphorePermit,
}
/// Weak registry admission handle; no service calls or HTTP loopback.
#[derive(Clone)]
#[non_exhaustive]
pub struct ChildExecutionPort {
    registry: Weak<Shared>,
}
impl fmt::Debug for ChildExecutionPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ChildExecutionPort(<redacted>)")
    }
}
impl Registry {
    /// Read the concrete weak child port without retaining a registry cycle.
    ///
    /// ```
    /// # fn inspect(r:&rustack_stepfunctions_core::Registry){let port=r.child_port();}
    /// ```
    #[must_use]
    pub fn child_port(&self) -> ChildExecutionPort {
        ChildExecutionPort {
            registry: Arc::downgrade(&self.shared),
        }
    }
}
impl ChildExecutionPort {
    /// Synchronously transfer one owned start before waiting for admission.
    ///
    /// # Errors
    /// Rejects closed ownership, deadline, scope or independent call saturation.
    ///
    /// ```no_run
    /// # fn start(p:&rustack_stepfunctions_core::ChildExecutionPort,parent:rustack_stepfunctions_core::ChildParent,args:rustack_stepfunctions_core::ChildStartParameters) {
    /// let receipt=p.start(parent,args,rustack_stepfunctions_core::ChildPattern::Sync2,tokio::time::Instant::now()+std::time::Duration::from_secs(30));
    /// # }
    /// ```
    pub fn start(
        &self,
        parent: ChildParent,
        parameters: ChildStartParameters,
        pattern: ChildPattern,
        deadline: Instant,
    ) -> Result<ChildSubmission, ChildError> {
        let shared = self.registry.upgrade().ok_or(ChildError::Unavailable)?;
        if *shared.closing.borrow() {
            return Err(ChildError::Unavailable);
        }
        parameters.machine.check(&shared.scope)?;
        let deadline = deadline.min(Instant::from_std(parent.record.deadline));
        if Instant::now() >= deadline {
            return Err(ChildError::Deadline);
        }
        let permit = shared
            .child_capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| ChildError::Capacity)?;
        let control = Arc::new(Control {
            cancelled: AtomicBool::new(false),
            wake: shared.child_wake.clone(),
        });
        let (reply, receiver) = oneshot::channel();
        let request = Request {
            context: Context {
                parent,
                parameters,
                pattern,
                deadline,
            },
            control: control.clone(),
            reply,
            permit,
        };
        shared
            .child_sender
            .try_send(request)
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => ChildError::Capacity,
                mpsc::error::TrySendError::Closed(_) => ChildError::Unavailable,
            })?;
        Ok(ChildSubmission {
            receiver: Some(receiver),
            accepted: None,
            control,
            armed: true,
        })
    }
}
/// Retained start receipt; Drop requests owned cancellation even before acceptance.
#[non_exhaustive]
pub struct ChildSubmission {
    receiver: Option<oneshot::Receiver<Result<ChildExecution, ChildError>>>,
    accepted: Option<Result<ChildExecution, Arc<ChildError>>>,
    control: Arc<Control>,
    armed: bool,
}
impl fmt::Debug for ChildSubmission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ChildSubmission(<redacted>)")
    }
}
impl ChildSubmission {
    /// Observe the same acceptance once; timeout/cancel retains its receiver.
    ///
    /// # Errors
    /// Returns a typed admission failure, deadline or closed receipt.
    ///
    /// ```no_run
    /// # async fn observe(s:&mut rustack_stepfunctions_core::ChildSubmission,d:tokio::time::Instant){let result=s.admitted(d).await;}
    /// ```
    pub async fn admitted(&mut self, deadline: Instant) -> Result<&ChildExecution, ChildError> {
        if Instant::now() >= deadline {
            return Err(ChildError::Deadline);
        }
        if self.accepted.is_none() {
            let receiver = self.receiver.as_mut().ok_or(ChildError::Unavailable)?;
            let response = timeout_at(deadline, receiver)
                .await
                .map_err(|_| ChildError::Deadline)?;
            self.receiver = None;
            self.accepted = Some(
                response
                    .map_err(|_| ChildError::OwnerLost)
                    .and_then(|result| result)
                    .map_err(Arc::new),
            );
        }
        if Instant::now() >= deadline {
            return Err(ChildError::Deadline);
        }
        match self.accepted.as_ref() {
            Some(Ok(child)) => Ok(child),
            Some(Err(source)) => Err(ChildError::Admission(source.clone())),
            None => Err(ChildError::Unavailable),
        }
    }

    /// Transfer the accepted unique cancellation owner without signalling Drop.
    ///
    /// # Errors
    /// Requires a cached acceptance, including one retained after a late observer
    /// expired; returns a cached rejection or Unavailable otherwise.
    ///
    /// ```no_run
    /// # fn take(s:rustack_stepfunctions_core::ChildSubmission){let child=s.into_execution();}
    /// ```
    pub fn into_execution(mut self) -> Result<ChildExecution, ChildError> {
        let child = self
            .accepted
            .take()
            .ok_or(ChildError::Unavailable)?
            .map_err(ChildError::Admission)?;
        self.armed = false;
        Ok(child)
    }
    /// Request cancellation; accepted detached starts remain independent.
    ///
    /// ```
    /// # fn cancel(s:&rustack_stepfunctions_core::ChildSubmission){s.cancel();}
    /// ```
    pub fn cancel(&self) {
        self.control.cancel();
    }
}
impl Drop for ChildSubmission {
    fn drop(&mut self) {
        if self.armed {
            self.control.cancel();
        }
    }
}
/// Unique owned-child cancellation capability and independent read observations.
#[non_exhaustive]
pub struct ChildExecution {
    pub(super) snapshot: ExecutionSnapshot,
    pub(super) control: Arc<Control>,
    pub(super) pattern: ChildPattern,
}
impl fmt::Debug for ChildExecution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ChildExecution(<redacted>)")
    }
}
impl ChildExecution {
    /// Read the latest captured child business/cleanup observation.
    ///
    /// ```
    /// # fn inspect(c:&rustack_stepfunctions_core::ChildExecution){let snapshot=c.snapshot();}
    /// ```
    #[must_use]
    pub fn snapshot(&self) -> &ExecutionSnapshot {
        &self.snapshot
    }
    /// Observe termination, preserving ownership when this observer expires.
    ///
    /// # Errors
    /// Returns the observer deadline or retained execution error.
    ///
    /// ```no_run
    /// # async fn observe(c:&mut rustack_stepfunctions_core::ChildExecution,d:tokio::time::Instant){let result=c.wait(d).await;}
    /// ```
    pub async fn wait(&mut self, deadline: Instant) -> Result<Arc<ExecutionView>, ChildError> {
        self.snapshot = self.snapshot.wait(deadline).await?;
        Ok(self.snapshot.view.clone())
    }
    /// Observe physical cleanup independently of the terminal business result.
    ///
    /// # Errors
    /// Deadline/uncertainty keeps the same owned child and receipt live.
    ///
    /// ```no_run
    /// # async fn observe(c:&mut rustack_stepfunctions_core::ChildExecution,d:tokio::time::Instant){let result=c.wait_cleanup(d).await;}
    /// ```
    pub async fn wait_cleanup(
        &mut self,
        deadline: Instant,
    ) -> Result<Arc<ExecutionView>, ChildError> {
        let mut handle = self.snapshot.handle();
        let view = handle.wait_cleanup(deadline).await?;
        self.snapshot.view = view.clone();
        Ok(view)
    }
    /// Request cancellation of an owned relationship, never a detached child.
    ///
    /// ```
    /// # fn cancel(c:&rustack_stepfunctions_core::ChildExecution){c.cancel();}
    /// ```
    pub fn cancel(&self) {
        if self.pattern.owned() {
            self.control.cancel();
        }
    }
}
impl Drop for ChildExecution {
    fn drop(&mut self) {
        self.cancel();
    }
}
