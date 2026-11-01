//! Opaque frame identities and typed actor/history effects.

use std::{fmt, sync::Arc, time::Instant};

use thiserror::Error;

use crate::{AslError, AttemptId, CompiledMachine, DriverStep, ExecutionOutcome, Payload, StateId};

macro_rules! get {
    ($owner:ident,$name:ident,$out:ty,$s:ident=>$body:expr,$doc:literal) => {
        #[doc=$doc]
        #[doc=concat!("\n```\n# fn inspect(value: &rustack_stepfunctions_core::",stringify!($owner),") { let _=value.",stringify!($name),"(); }\n```")]
        #[must_use] pub fn $name(&self)->$out { let $s=self; $body }
    };
}

/// Monotonic scheduler-local identity, never reused for another child or retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub struct FrameId(pub(super) u64);

/// A call identity includes its frame as well as its interpreter-local generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub struct FrameCallId {
    pub(super) frame: FrameId,
    pub(super) attempt: AttemptId,
}
impl FrameCallId {
    get!(FrameCallId,frame,FrameId,s=>s.frame,"Read the frame that owns this call.");
    get!(FrameCallId,attempt,AttemptId,s=>s.attempt,"Read its frame-local attempt.");
}

/// Immutable causal linkage to the enclosing fanout round.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct FrameParent {
    pub(super) frame: FrameId,
    pub(super) round: AttemptId,
    pub(super) index: usize,
}
impl FrameParent {
    get!(FrameParent,frame,FrameId,s=>s.frame,"Read the enclosing frame.");
    get!(FrameParent,round,AttemptId,s=>s.round,"Read the enclosing round generation.");
    get!(FrameParent,index,usize,s=>s.index,"Read the definition/item result index.");
}

/// One interpreter transition, retaining the immutable information for causal history.
#[non_exhaustive]
pub struct FrameTransition {
    pub(super) frame: FrameId,
    pub(super) parent: Option<FrameParent>,
    pub(super) machine: Arc<CompiledMachine>,
    pub(super) state: StateId,
    pub(super) entered: Option<Payload>,
    pub(super) exited: Option<Payload>,
    pub(super) timed_out: Option<FrameCallId>,
    pub(super) step: DriverStep,
    pub(crate) fanout_started: Option<usize>,
    pub(crate) round_finished: Option<bool>,
}
impl fmt::Debug for FrameTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FrameTransition(<redacted>)")
    }
}
impl FrameTransition {
    get!(FrameTransition,frame,FrameId,s=>s.frame,"Read the frame that made this transition.");
    get!(FrameTransition,parent,Option<FrameParent>,s=>s.parent,"Read its enclosing round linkage, if any.");
    get!(FrameTransition,machine,&Arc<CompiledMachine>,s=>&s.machine,"Read the lexical instruction scope.");
    get!(FrameTransition,state,StateId,s=>s.state,"Read the state before this transition.");
    get!(FrameTransition,entered,Option<&Payload>,s=>s.entered.as_ref(),"Read raw input only when this step enters a new state.");
    get!(FrameTransition,exited,Option<&Payload>,s=>s.exited.as_ref(),"Read output when this step exits a state successfully or through Catch.");
    get!(FrameTransition,timed_out_call,Option<FrameCallId>,s=>s.timed_out,"Read a Task timeout requiring a history failure event.");
    get!(FrameTransition,step,&DriverStep,s=>&s.step,"Read the interpreter effect; the scheduler owns any fanout plan.");
    /// Read the complete call identity when the effect dispatches a Task.
    ///
    /// ```
    /// # fn inspect(transition: &rustack_stepfunctions_core::FrameTransition) { let call=transition.call(); }
    /// ```
    #[must_use]
    pub fn call(&self) -> Option<FrameCallId> {
        self.step.task_attempt().map(|attempt| FrameCallId {
            frame: self.frame,
            attempt,
        })
    }
}

/// Bounded frame-tree updates consumed by the asynchronous resource/history owner.
#[derive(Debug)]
#[non_exhaustive]
pub enum FrameUpdate {
    /// Commit history before issuing its Task effect; fanout is already scheduler-owned.
    Transition(Box<FrameTransition>),
    /// One child was admitted; its first state has not yet run.
    ChildStarted {
        /// New non-reused identity.
        child: FrameId,
        /// Enclosing round and result index.
        parent: FrameParent,
    },
    /// One child's business outcome was notified exactly once.
    ChildFinished {
        /// Completed child identity, possibly still awaiting resource retirement.
        child: FrameId,
        /// Enclosing round and result index.
        parent: FrameParent,
        /// Immutable business result.
        outcome: Arc<ExecutionOutcome>,
    },
    /// Retire this resource; acknowledge only after its ownership is known settled.
    Cancel(FrameCallId),
    /// No local work can advance before a resource/timer/capacity event.
    Park {
        /// Earliest local timer, bounded by the root deadline.
        until: Instant,
        /// Also observe global frame-capacity release when true.
        global_capacity: bool,
    },
    /// Yield after bounded queue scanning; try again without sleeping for IO.
    Yield,
    /// Stable root business result; cleanup readiness is an independent query.
    Settled(Arc<ExecutionOutcome>),
}

/// Configuration/admission/protocol errors; business failures are scheduler outcomes.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum FrameError {
    /// Invalid ceilings, already-started driver or invalid local identity.
    #[error("invalid workflow frame configuration")]
    Invalid,
    /// The root could not reserve bounded frame admission.
    #[error("workflow frame capacity exhausted")]
    Capacity,
    /// An active call was acknowledged without requesting retirement.
    #[error("invalid workflow frame retirement event")]
    Event,
    /// Capacity observation timed out without altering ownership.
    #[error("workflow frame observation deadline exceeded")]
    Deadline,
    /// A typed interpreter invariant or validated input failed.
    #[error("workflow frame evaluation failed")]
    Evaluation(#[from] AslError),
}
