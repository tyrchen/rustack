//! Bounded synchronous ownership of a complete execution's lexical frame tree.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
    time::Instant,
};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::types::FailureOrigin;
use crate::{
    AslError, AttemptId, ExecutionDriver, ExecutionFailure, ExecutionOutcome, FanoutFailure,
    FanoutPlan, Payload,
};

mod budget;
mod control;
mod drive;
mod lifecycle;
#[cfg(test)]
mod tests;
mod types;

use budget::{Blocked, Slot};
pub use budget::{FrameCapacity, FrameLimits};
pub use types::{FrameCallId, FrameError, FrameId, FrameParent, FrameTransition, FrameUpdate};

const ROOT: FrameId = FrameId(0);
const MAX_CALLS: usize = 1024;
const SCAN_LIMIT: usize = 64;
const RESULT_OVERHEAD: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CallState {
    Active,
    CancelQueued,
    CancelIssued,
}
#[derive(Debug)]
struct ResultSlot {
    payload: Payload,
    _global: OwnedSemaphorePermit,
    _local: OwnedSemaphorePermit,
}
#[derive(Debug)]
struct Round {
    plan: FanoutPlan,
    next: usize,
    running: BTreeSet<FrameId>,
    outputs: BTreeMap<usize, ResultSlot>,
    bytes: usize,
    failure: Option<FanoutFailure>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Notification {
    Waiting,
    Queued,
    Emitted,
}
#[derive(Debug)]
struct Notice {
    child: FrameId,
    parent: FrameParent,
    outcome: Arc<ExecutionOutcome>,
}
#[derive(Debug)]
struct Frame {
    driver: ExecutionDriver,
    parent: Option<FrameParent>,
    children: BTreeSet<FrameId>,
    round: Option<Round>,
    calls: BTreeMap<AttemptId, CallState>,
    notification: Notification,
    cancelling: bool,
    _slot: Slot,
}

/// One execution's bounded frame tree; the caller independently owns IO resources.
///
/// Retain this owner across observation timeouts. It emits effects only; callers
/// must join accepted calls and acknowledge retirement before releasing ownership.
#[derive(Debug)]
#[non_exhaustive]
pub struct FrameScheduler {
    frames: BTreeMap<FrameId, Frame>,
    ready: VecDeque<FrameId>,
    queued: BTreeSet<FrameId>,
    blocked: BTreeMap<FrameId, Blocked>,
    cancels: VecDeque<FrameCallId>,
    pending: VecDeque<Notice>,
    capacity: Arc<FrameCapacity>,
    local_frames: Arc<Semaphore>,
    local_results: Arc<Semaphore>,
    deadline: Instant,
    observed: Instant,
    remaining: u32,
    next: u64,
    calls: usize,
    outcome: Option<Arc<ExecutionOutcome>>,
}
impl FrameScheduler {
    /// Admit an unstarted root driver without spawning or issuing a call.
    ///
    /// # Errors
    /// Returns Invalid for a started driver, or Capacity before root registration.
    ///
    /// ```
    /// # fn admit(driver:rustack_stepfunctions_core::ExecutionDriver) {
    /// let owner=rustack_stepfunctions_core::FrameScheduler::new(driver,std::sync::Arc::new(rustack_stepfunctions_core::FrameCapacity::default()),rustack_stepfunctions_core::FrameLimits::default());
    /// # }
    /// ```
    pub fn new(
        driver: ExecutionDriver,
        capacity: Arc<FrameCapacity>,
        limits: FrameLimits,
    ) -> Result<Self, FrameError> {
        if driver.started {
            return Err(FrameError::Invalid);
        }
        let local_frames = Arc::new(Semaphore::new(limits.frames));
        let slot = Slot::acquire(&capacity, &local_frames).map_err(|_| FrameError::Capacity)?;
        let deadline = driver.deadline;
        let observed = driver.observed;
        let remaining = driver.remaining;
        let frame = Frame {
            driver,
            parent: None,
            children: BTreeSet::new(),
            round: None,
            calls: BTreeMap::new(),
            notification: Notification::Waiting,
            cancelling: false,
            _slot: slot,
        };
        Ok(Self {
            frames: BTreeMap::from([(ROOT, frame)]),
            ready: VecDeque::from([ROOT]),
            queued: BTreeSet::from([ROOT]),
            blocked: BTreeMap::new(),
            cancels: VecDeque::new(),
            pending: VecDeque::new(),
            capacity,
            local_frames,
            local_results: Arc::new(Semaphore::new(limits.results)),
            deadline,
            observed,
            remaining,
            next: 1,
            calls: 0,
            outcome: None,
        })
    }
    /// Read a live/retiring frame without mutating its interpreter.
    ///
    /// ```
    /// # fn inspect(owner:&rustack_stepfunctions_core::FrameScheduler,id:rustack_stepfunctions_core::FrameId) { let driver=owner.driver(id); }
    /// ```
    #[must_use]
    pub fn driver(&self, id: FrameId) -> Option<&ExecutionDriver> {
        self.frames.get(&id).map(|frame| &frame.driver)
    }
    /// Read the stable root outcome, independently of resource retirement.
    ///
    /// ```
    /// # fn inspect(owner:&rustack_stepfunctions_core::FrameScheduler) { let outcome=owner.outcome(); }
    /// ```
    #[must_use]
    pub fn outcome(&self) -> Option<&Arc<ExecutionOutcome>> {
        self.outcome.as_ref()
    }
    /// Whether root termination and every descendant/call retirement are complete.
    ///
    /// ```
    /// # fn inspect(owner:&rustack_stepfunctions_core::FrameScheduler) { let clean=owner.cleanup_ready(); }
    /// ```
    #[must_use]
    pub fn cleanup_ready(&self) -> bool {
        self.outcome.is_some() && self.frames.is_empty() && self.pending.is_empty()
    }
    /// Read charged frame count, including terminal frames awaiting call retirement.
    ///
    /// ```
    /// # fn inspect(owner:&rustack_stepfunctions_core::FrameScheduler) { let count=owner.frame_count(); }
    /// ```
    #[must_use]
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    fn enqueue(&mut self, id: FrameId, front: bool) {
        if self.frames.get(&id).is_some_and(|f| !f.cancelling) && self.queued.insert(id) {
            if front {
                self.ready.push_front(id);
            } else {
                self.ready.push_back(id);
            }
        }
    }
    fn internal(&mut self) {
        let source = Arc::new(AslError::InvalidStateId);
        let mut failure =
            ExecutionFailure::named("Rustack.InternalError", "Frame owner invariant failed");
        failure.origin = FailureOrigin::Evaluation(source);
        self.terminate(Arc::new(ExecutionOutcome::Failed(failure)));
    }
    fn safety(&mut self, name: &str) {
        self.terminate(Arc::new(ExecutionOutcome::Failed(
            ExecutionFailure::safety(name, "Execution frame budget exhausted"),
        )));
    }
}
