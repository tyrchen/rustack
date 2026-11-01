//! Root control and explicit call-retirement ownership.

use std::{cmp::Reverse, collections::BTreeSet, sync::Arc};

use super::{
    CallState, FrameCallId, FrameError, FrameId, FrameScheduler, FrameUpdate, Notice, Notification,
    ROOT,
};
use crate::{
    AslError, AttemptId, ClockSample, ExecutionFailure, ExecutionOutcome, TaskFailure,
    engine::{Phase, types::FailureOrigin},
};

impl FrameScheduler {
    pub(super) fn observe(&mut self, now: ClockSample) {
        if self.outcome.is_some() {
            return;
        }
        if now.monotonic < self.observed {
            let mut failure =
                ExecutionFailure::named("Rustack.InternalError", "Monotonic clock regressed");
            failure.origin = FailureOrigin::Evaluation(Arc::new(AslError::InvalidExecution));
            self.terminate(Arc::new(ExecutionOutcome::Failed(failure)));
        } else {
            self.observed = now.monotonic;
            if now.monotonic >= self.deadline {
                self.terminate(Arc::new(ExecutionOutcome::TimedOut));
            }
        }
    }
    /// Stop once, retaining all calls and emitting their retirement obligations.
    ///
    /// ```
    /// # fn stop(owner:&mut rustack_stepfunctions_core::FrameScheduler,reason:rustack_stepfunctions_core::TaskFailure,now:rustack_stepfunctions_core::ClockSample) {
    /// let update=owner.stop(reason,now);
    /// # }
    /// ```
    pub fn stop(&mut self, reason: TaskFailure, now: ClockSample) -> FrameUpdate {
        self.observe(now);
        if self.outcome.is_none() {
            self.terminate(Arc::new(ExecutionOutcome::Aborted(reason)));
        }
        self.terminal_update()
    }
    pub(super) fn terminate(&mut self, outcome: Arc<ExecutionOutcome>) {
        if self.outcome.is_some() {
            return;
        }
        let success = matches!(outcome.as_ref(), ExecutionOutcome::Succeeded(_));

        self.ready.clear();
        self.queued.clear();
        self.blocked.clear();
        let mut calls = Vec::new();
        let mut notices = Vec::new();
        for (id, frame) in &mut self.frames {
            frame.cancelling = true;
            frame.round = None;
            if *id == ROOT {
                frame.driver.phase = Phase::Terminal(outcome.clone());
            } else if !success {
                frame.driver.phase = Phase::Terminal(Arc::new(ExecutionOutcome::Aborted(
                    TaskFailure(ExecutionFailure::named(
                        "Rustack.ParentCancelled",
                        "Enclosing frame cancelled",
                    )),
                )));
            }
            if frame.notification == Notification::Waiting {
                if let Some(parent) = frame.parent {
                    if let Some(outcome) = frame.driver.outcome().cloned() {
                        notices.push(Notice {
                            child: *id,
                            parent,
                            outcome,
                        });
                        frame.notification = Notification::Queued;
                    }
                }
            }
            calls.extend(frame.calls.keys().map(|attempt| FrameCallId {
                frame: *id,
                attempt: *attempt,
            }));
        }
        notices.sort_by_key(|notice| Reverse(notice.child));
        self.pending.extend(notices);
        self.outcome = Some(outcome);
        for call in calls {
            self.cancel(call);
        }
        let ids = self.frames.keys().copied().collect::<Vec<_>>();
        for id in ids {
            self.prune(id);
        }
    }
    pub(super) fn cancel(&mut self, call: FrameCallId) {
        if let Some(state) = self
            .frames
            .get_mut(&call.frame)
            .and_then(|f| f.calls.get_mut(&call.attempt))
        {
            if *state == CallState::Active {
                *state = CallState::CancelQueued;
                self.cancels.push_back(call);
            }
        }
    }
    pub(super) fn next_cancel(&mut self) -> Option<FrameCallId> {
        while let Some(call) = self.cancels.pop_front() {
            if let Some(state) = self
                .frames
                .get_mut(&call.frame)
                .and_then(|f| f.calls.get_mut(&call.attempt))
            {
                if *state == CallState::CancelQueued {
                    *state = CallState::CancelIssued;
                    return Some(call);
                }
            }
        }
        None
    }
    pub(super) fn pop_notice(&mut self) -> Option<FrameUpdate> {
        let Notice {
            child,
            parent,
            outcome,
        } = self.pending.pop_front()?;
        if let Some(frame) = self.frames.get_mut(&child) {
            frame.notification = Notification::Emitted;
        }
        self.prune(child);
        Some(FrameUpdate::ChildFinished {
            child,
            parent,
            outcome,
        })
    }
    pub(super) fn terminal_update(&mut self) -> FrameUpdate {
        if let Some(call) = self.next_cancel() {
            return FrameUpdate::Cancel(call);
        }
        if let Some(update) = self.pop_notice() {
            return update;
        }
        self.outcome.as_ref().map_or(FrameUpdate::Yield, |outcome| {
            FrameUpdate::Settled(outcome.clone())
        })
    }
    /// Acknowledge a requested retirement only after external ownership is settled.
    ///
    /// # Errors
    /// Returns Event for an active call without a prior completion/cancellation.
    /// Stale or repeated acknowledgements return false and change nothing.
    ///
    /// ```
    /// # fn retire(owner:&mut rustack_stepfunctions_core::FrameScheduler,call:rustack_stepfunctions_core::FrameCallId) {
    /// let changed=owner.retired(call);
    /// # }
    /// ```
    pub fn retired(&mut self, call: FrameCallId) -> Result<bool, FrameError> {
        let Some(frame) = self.frames.get_mut(&call.frame) else {
            return Ok(false);
        };
        let Some(state) = frame.calls.get(&call.attempt) else {
            return Ok(false);
        };
        if *state == CallState::Active {
            return Err(FrameError::Event);
        }
        frame.calls.remove(&call.attempt);
        self.calls = self.calls.checked_sub(1).ok_or(FrameError::Invalid)?;
        self.prune(call.frame);
        let blocked = self.blocked.keys().copied().collect::<Vec<_>>();
        for id in blocked {
            self.enqueue(id, false);
        }
        Ok(true)
    }
    pub(super) fn prune(&mut self, id: FrameId) {
        let mut current = Some(id);
        while let Some(id) = current.take() {
            let removable = self.frames.get(&id).is_some_and(|frame| {
                frame.driver.outcome().is_some()
                    && frame.notification != Notification::Queued
                    && (frame.notification == Notification::Emitted
                        || frame.cancelling
                        || id == ROOT && self.outcome.is_some())
                    && frame.calls.is_empty()
                    && frame.children.is_empty()
            });
            if !removable {
                break;
            }
            if let Some(frame) = self.frames.remove(&id) {
                self.queued.remove(&id);
                self.blocked.remove(&id);
                if let Some(parent) = frame.parent {
                    if let Some(owner) = self.frames.get_mut(&parent.frame) {
                        owner.children.remove(&id);
                    }
                    self.enqueue(parent.frame, false);
                    current = Some(parent.frame);
                }
            }
        }
    }
    pub(super) fn cancel_round(
        &mut self,
        parent: FrameId,
        round: AttemptId,
        ignore: Option<FrameId>,
    ) {
        let mut pending = self.frames.get(&parent).map_or_else(Vec::new, |owner| {
            owner
                .children
                .iter()
                .filter(|child| {
                    self.frames
                        .get(child)
                        .and_then(|f| f.parent)
                        .is_some_and(|link| link.round == round)
                })
                .copied()
                .collect()
        });
        let mut visited = BTreeSet::new();
        let mut notices = Vec::new();
        while let Some(id) = pending.pop() {
            if !visited.insert(id) {
                continue;
            }
            let calls = if let Some(frame) = self.frames.get_mut(&id) {
                pending.extend(frame.children.iter().copied());
                frame.cancelling = Some(id) != ignore;
                frame.round = None;
                if frame.driver.outcome().is_none() {
                    frame.driver.phase = Phase::Terminal(Arc::new(ExecutionOutcome::Aborted(
                        TaskFailure(ExecutionFailure::named(
                            "Rustack.ParentCancelled",
                            "Enclosing frame cancelled",
                        )),
                    )));
                }
                if frame.cancelling && frame.notification == Notification::Waiting {
                    if let Some(parent) = frame.parent {
                        if let Some(outcome) = frame.driver.outcome().cloned() {
                            notices.push(Notice {
                                child: id,
                                parent,
                                outcome,
                            });
                            frame.notification = Notification::Queued;
                        }
                    }
                }
                frame
                    .calls
                    .keys()
                    .map(|attempt| FrameCallId {
                        frame: id,
                        attempt: *attempt,
                    })
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            for call in calls {
                self.cancel(call);
            }
            self.prune(id);
        }
        notices.sort_by_key(|notice| Reverse(notice.child));
        self.pending.extend(notices);
    }
}
