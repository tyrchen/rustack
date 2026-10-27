//! Object-safe bounded task admission and independent result/cleanup receipts.
//!
//! Admission is synchronous: an adapter must transfer its start future to a
//! retained manager before returning a call. Concrete waits need no async-trait.
//! Dropping a call only signals cancellation; adapter ownership is independent.

use std::{fmt, sync::Arc};

use thiserror::Error;
use tokio::sync::{mpsc, watch};

use crate::{
    CallbackError, CallbackInvocationReporter, CompiledMachine, FrameCallId, Integration, Payload,
    StateView, TaskDispatch, TaskFailure,
};

/// Immutable captured instruction and evaluated parameters for one attempt.
#[derive(Debug)]
#[non_exhaustive]
pub struct LambdaTaskCommand {
    machine: Arc<CompiledMachine>,
    dispatch: TaskDispatch,
    mode: LambdaTaskMode,
}
#[derive(Debug)]
enum LambdaTaskMode {
    Response,
    Observed(Option<CallbackInvocationReporter>),
}
impl LambdaTaskCommand {
    pub(crate) fn new(machine: Arc<CompiledMachine>, dispatch: TaskDispatch) -> Self {
        Self {
            machine,
            dispatch,
            mode: LambdaTaskMode::Response,
        }
    }
    /// Bind one real optimized invocation to its unique callback attempt.
    ///
    /// # Errors
    /// Rejects another scope/frame/generation, a nonoptimized Task or expired control.
    ///
    /// ```
    /// # fn bind(m:std::sync::Arc<rustack_stepfunctions_core::CompiledMachine>,d:rustack_stepfunctions_core::TaskDispatch,c:rustack_stepfunctions_core::FrameCallId,r:rustack_stepfunctions_core::CallbackInvocationReporter){let command=rustack_stepfunctions_core::LambdaTaskCommand::observed_callback(m,d,c,r);}
    /// ```
    pub fn observed_callback(
        machine: Arc<CompiledMachine>,
        mut dispatch: TaskDispatch,
        call: FrameCallId,
        reporter: CallbackInvocationReporter,
    ) -> Result<Self, CallbackError> {
        let deadline = reporter.bind(machine.scope(), call)?;
        if dispatch.attempt() != call.attempt()
            || !matches!(
                machine.state(dispatch.state()),
                Ok(StateView::Task {
                    resource: Integration::LambdaInvoke,
                    ..
                })
            )
        {
            return Err(CallbackError::Validation);
        }
        dispatch.deadline = dispatch.deadline.min(deadline.into_std());
        Ok(Self {
            machine,
            dispatch,
            mode: LambdaTaskMode::Observed(Some(reporter)),
        })
    }
    /// Identify explicit callback observation even after its reporter transfers.
    ///
    /// ```
    /// # fn inspect(c:&rustack_stepfunctions_core::LambdaTaskCommand){let callback=c.is_observed_callback();}
    /// ```
    #[must_use]
    pub fn is_observed_callback(&self) -> bool {
        matches!(self.mode, LambdaTaskMode::Observed(_))
    }
    /// Transfer the real invocation report capability once, after validating a plan.
    ///
    /// # Errors
    /// Rejects consumed, expired or revoked callback control. Ordinary commands
    /// return `None` and keep their response mapping.
    ///
    /// ```
    /// # fn take(c:&mut rustack_stepfunctions_core::LambdaTaskCommand){let reporter=c.take_callback_reporter();}
    /// ```
    pub fn take_callback_reporter(
        &mut self,
    ) -> Result<Option<CallbackInvocationReporter>, CallbackError> {
        match &mut self.mode {
            LambdaTaskMode::Response => Ok(None),
            LambdaTaskMode::Observed(reporter) => {
                reporter
                    .as_ref()
                    .ok_or(CallbackError::Progress)?
                    .check_live()?;
                Ok(Some(reporter.take().ok_or(CallbackError::Progress)?))
            }
        }
    }
    /// Read the captured artifact; the dispatch's state ID belongs to this machine.
    ///
    /// ```
    /// # fn inspect(c:&rustack_stepfunctions_core::LambdaTaskCommand) { let ir=c.machine(); }
    /// ```
    #[must_use]
    pub fn machine(&self) -> &Arc<CompiledMachine> {
        &self.machine
    }
    /// Read parameters, attempt identity and the absolute admission/call deadline.
    ///
    /// ```
    /// # fn inspect(c:&rustack_stepfunctions_core::LambdaTaskCommand) { let attempt=c.dispatch().attempt(); }
    /// ```
    #[must_use]
    pub fn dispatch(&self) -> &TaskDispatch {
        &self.dispatch
    }
}

/// Adapter installed by the composition root, never a fake production fallback.
pub trait LambdaTaskPort: fmt::Debug + Send + Sync {
    /// Admit exactly one call without blocking or losing ownership of its start.
    ///
    /// The adapter must check the command deadline before accepting/issuing work,
    /// retain accepted start/cleanup futures, and never retry a business call.
    ///
    /// # Errors
    /// Returns a bounded service/admission failure before transferring a call.
    ///
    /// ```
    /// # fn admit(p:&dyn rustack_stepfunctions_core::LambdaTaskPort,c:rustack_stepfunctions_core::LambdaTaskCommand) {
    /// let call=p.start(c);
    /// # }
    /// ```
    fn start(&self, command: LambdaTaskCommand) -> Result<TaskCall, TaskFailure>;
}

/// Cleanup truth, separate from delivery of the business result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TaskSettlement {
    /// Still owned; no cleanup proof.
    Pending,
    /// Resource cleanup is confirmed by its retained adapter owner.
    Clean,
    /// An uncertain resource remains owned and charged.
    Quarantined,
}
/// At most one start and one result per call; no payload is printed by Debug.
#[derive(Debug)]
#[non_exhaustive]
pub enum TaskEvent {
    /// The adapter actually admitted the underlying invocation.
    Started,
    /// A real accepted nested job identity, before its terminal result.
    Submitted(Payload),
    /// A business result; consult settlement separately before releasing capacity.
    Completed(Result<Payload, TaskFailure>),
}
/// Static call-channel/lifecycle failures; closed channels never imply cleanup.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TaskChannelError {
    /// The adapter terminated without the promised event/settlement.
    #[error("Lambda task owner lost")]
    OwnerLost,
    /// Adapter attempted an invalid or duplicate progress transition.
    #[error("invalid Lambda task progress")]
    InvalidProgress,
}

/// The execution owner's cancellable result/settlement receipt.
#[derive(Debug)]
#[non_exhaustive]
pub struct TaskCall {
    events: mpsc::Receiver<TaskEvent>,
    cancellation: watch::Sender<bool>,
    settlement: watch::Receiver<TaskSettlement>,
}
/// The adapter's completion capability; keep beside the actual invocation handle.
#[derive(Debug)]
#[non_exhaustive]
pub struct TaskResponder {
    events: mpsc::Sender<TaskEvent>,
    cancellation: watch::Receiver<bool>,
    settlement: watch::Sender<TaskSettlement>,
    started: bool,
    submitted: bool,
}
/// Adapter-owned uncertain cleanup capability; Drop never fabricates settlement.
#[derive(Debug)]
#[non_exhaustive]
pub struct QuarantinedTask(TaskResponder);

#[derive(Debug)]
pub(crate) struct TaskCompletion(TaskResponder);
impl TaskCompletion {
    pub(crate) fn settle(self) {
        self.0.settlement.send_replace(TaskSettlement::Clean);
    }
}

impl TaskCall {
    /// Create bounded channels before admission; the adapter owns the responder.
    ///
    /// ```
    /// let (call,owner)=rustack_stepfunctions_core::TaskCall::channel();
    /// assert_eq!(call.settlement(),rustack_stepfunctions_core::TaskSettlement::Pending);
    /// ```
    #[must_use]
    pub fn channel() -> (Self, TaskResponder) {
        let (events, receiver) = mpsc::channel(4);
        let (cancellation, cancelled) = watch::channel(false);
        let (settlement, settled) = watch::channel(TaskSettlement::Pending);
        (
            Self {
                events: receiver,
                cancellation,
                settlement: settled,
            },
            TaskResponder {
                events,
                cancellation: cancelled,
                settlement,
                started: false,
                submitted: false,
            },
        )
    }
    /// Receive progress/result without spawning or aborting the adapter's worker.
    ///
    /// # Errors
    /// Returns `OwnerLost` if the adapter closes without another event.
    ///
    /// ```no_run
    /// # async fn next(call:&mut rustack_stepfunctions_core::TaskCall) {
    /// let event=call.next().await;
    /// # }
    /// ```
    pub async fn next(&mut self) -> Result<TaskEvent, TaskChannelError> {
        self.events.recv().await.ok_or(TaskChannelError::OwnerLost)
    }
    pub(crate) fn cancellation_sender(&self) -> watch::Sender<bool> {
        self.cancellation.clone()
    }

    /// Signal cancellation while preserving the settlement receipt.
    ///
    /// ```
    /// let (call,owner)=rustack_stepfunctions_core::TaskCall::channel();
    /// call.cancel();
    /// ```
    pub fn cancel(&self) {
        self.cancellation.send_replace(true);
    }
    /// Read cleanup truth; channel closure is never mapped to Clean.
    ///
    /// ```
    /// let (call,owner)=rustack_stepfunctions_core::TaskCall::channel();
    /// assert_ne!(call.settlement(),rustack_stepfunctions_core::TaskSettlement::Clean);
    /// ```
    #[must_use]
    pub fn settlement(&self) -> TaskSettlement {
        *self.settlement.borrow()
    }
    /// Await Clean; cancelling this waiter preserves the same receipt and owner.
    ///
    /// # Errors
    /// Returns `OwnerLost` if the owner disappears before confirming cleanup.
    ///
    /// ```no_run
    /// # async fn join(call:&mut rustack_stepfunctions_core::TaskCall) {
    /// let cleanup=call.settled().await;
    /// # }
    /// ```
    pub async fn settled(&mut self) -> Result<(), TaskChannelError> {
        self.settlement
            .wait_for(|state| *state == TaskSettlement::Clean)
            .await
            .map(|_| ())
            .map_err(|_| TaskChannelError::OwnerLost)
    }
}
impl Drop for TaskCall {
    fn drop(&mut self) {
        self.cancel();
    }
}
impl TaskResponder {
    /// Subscribe to cancellation without transferring actual resource ownership.
    ///
    /// ```
    /// let (call,owner)=rustack_stepfunctions_core::TaskCall::channel();
    /// let cancellation=owner.cancellation();
    /// call.cancel();
    /// assert!(*cancellation.borrow());
    /// ```
    #[must_use]
    pub fn cancellation(&self) -> watch::Receiver<bool> {
        self.cancellation.clone()
    }
    /// Report actual underlying invocation start once; a lost observer is harmless.
    ///
    /// # Errors
    /// Rejects duplicate starts or impossible bounded-channel overflow.
    ///
    /// ```
    /// let (call,mut owner)=rustack_stepfunctions_core::TaskCall::channel();
    /// owner.started()?;
    /// # Ok::<(),rustack_stepfunctions_core::TaskChannelError>(())
    /// ```
    pub fn started(&mut self) -> Result<(), TaskChannelError> {
        if self.started {
            return Err(TaskChannelError::InvalidProgress);
        }
        self.started = true;
        match self.events.try_send(TaskEvent::Started) {
            Ok(()) | Err(mpsc::error::TrySendError::Closed(_)) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => Err(TaskChannelError::InvalidProgress),
        }
    }
    /// Publish one actual accepted job envelope after Started.
    ///
    /// # Errors
    /// Rejects duplicate, out-of-order or overflowing progress.
    ///
    /// ```
    /// let (_, mut owner) = rustack_stepfunctions_core::TaskCall::channel();
    /// owner.started()?;
    /// owner.submitted(rustack_stepfunctions_core::Payload::parse(b"{}")?)?;
    /// # Ok::<(),Box<dyn std::error::Error>>(())
    /// ```
    pub fn submitted(&mut self, output: Payload) -> Result<(), TaskChannelError> {
        if !self.started || self.submitted {
            return Err(TaskChannelError::InvalidProgress);
        }
        self.submitted = true;
        match self.events.try_send(TaskEvent::Submitted(output)) {
            Ok(()) | Err(mpsc::error::TrySendError::Closed(_)) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => Err(TaskChannelError::InvalidProgress),
        }
    }
    pub(crate) fn complete(self, result: Result<Payload, TaskFailure>) -> TaskCompletion {
        let _ = self.events.try_send(TaskEvent::Completed(result));
        TaskCompletion(self)
    }
    /// Confirm cleanup and deliver one result, even if its observer has departed.
    ///
    /// Only the adapter that has joined the real resource may call this method.
    ///
    /// ```
    /// let (call,owner)=rustack_stepfunctions_core::TaskCall::channel();
    /// owner.finish(Ok(rustack_stepfunctions_core::Payload::parse(b"true")?));
    /// assert_eq!(call.settlement(),rustack_stepfunctions_core::TaskSettlement::Clean);
    /// # Ok::<(),rustack_stepfunctions_core::AslError>(())
    /// ```
    pub fn finish(self, result: Result<Payload, TaskFailure>) {
        self.settlement.send_replace(TaskSettlement::Clean);
        let _ = self.events.try_send(TaskEvent::Completed(result));
    }
    /// Deliver an uncertain business failure and retain cleanup ownership.
    ///
    /// Store the returned receipt beside the resource until actual joined cleanup.
    ///
    /// ```
    /// let (call,owner)=rustack_stepfunctions_core::TaskCall::channel();
    /// let failure=rustack_stepfunctions_core::TaskFailure::new("Lambda.ServiceException",None)?;
    /// let retained=owner.quarantine(failure);
    /// assert_eq!(call.settlement(),rustack_stepfunctions_core::TaskSettlement::Quarantined);
    /// # Ok::<(),rustack_stepfunctions_core::AslError>(())
    /// ```
    #[must_use]
    pub fn quarantine(self, failure: TaskFailure) -> QuarantinedTask {
        self.settlement.send_replace(TaskSettlement::Quarantined);
        let _ = self.events.try_send(TaskEvent::Completed(Err(failure)));
        QuarantinedTask(self)
    }
}
impl QuarantinedTask {
    /// Read cancellation while uncertain cleanup remains owned.
    ///
    /// ```
    /// # fn inspect(owner:&rustack_stepfunctions_core::QuarantinedTask) { let stop=owner.cancellation(); }
    /// ```
    #[must_use]
    pub fn cancellation(&self) -> watch::Receiver<bool> {
        self.0.cancellation()
    }
    /// Confirm actual joined cleanup without redelivering a business result.
    ///
    /// ```
    /// # fn joined(owner:rustack_stepfunctions_core::QuarantinedTask) { owner.settle(); }
    /// ```
    pub fn settle(self) {
        self.0.settlement.send_replace(TaskSettlement::Clean);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::time::{Instant, timeout};

    use super::*;
    #[tokio::test]
    async fn test_should_preserve_started_and_result_order_with_fast_completion() {
        let (mut call, mut owner) = TaskCall::channel();
        owner.started().unwrap();
        assert!(owner.started().is_err());
        owner.finish(Ok(Payload::parse(b"true").unwrap()));
        assert!(matches!(call.next().await, Ok(TaskEvent::Started)));
        assert!(matches!(call.next().await, Ok(TaskEvent::Completed(Ok(_)))));
        call.settled().await.unwrap();
    }
    #[tokio::test]
    async fn test_should_resume_same_cleanup_receipt_after_waiter_timeout() {
        let (mut call, owner) = TaskCall::channel();
        let cancelled = owner.cancellation();
        call.cancel();
        assert!(*cancelled.borrow());
        let pending = owner.quarantine(TaskFailure::new("Lambda.ServiceException", None).unwrap());
        assert!(
            timeout(Duration::from_millis(10), call.settled())
                .await
                .is_err()
        );
        pending.settle();
        call.settled().await.unwrap();
        assert!(matches!(
            call.next().await,
            Ok(TaskEvent::Completed(Err(_)))
        ));
    }
    #[tokio::test]
    async fn test_should_not_treat_closed_channels_as_clean_settlement() {
        let (mut call, owner) = TaskCall::channel();
        drop(owner);
        assert!(matches!(
            call.next().await,
            Err(TaskChannelError::OwnerLost)
        ));
        assert!(matches!(
            call.settled().await,
            Err(TaskChannelError::OwnerLost)
        ));
        assert_eq!(call.settlement(), TaskSettlement::Pending);
        let (call, owner) = TaskCall::channel();
        let cancelled = owner.cancellation();
        drop(call);
        assert!(*cancelled.borrow());
    }
    #[tokio::test]
    async fn test_should_keep_quarantine_after_owner_receipt_is_lost() {
        let (mut call, owner) = TaskCall::channel();
        let retained = owner.quarantine(TaskFailure::new("Lambda.ServiceException", None).unwrap());
        drop(retained);
        assert!(matches!(
            call.settled().await,
            Err(TaskChannelError::OwnerLost)
        ));
        assert_eq!(call.settlement(), TaskSettlement::Quarantined);
        assert!(Instant::now().elapsed() < Duration::from_secs(1));
    }
}
