//! Fair bounded ready traversal and one-frame interpreter mutation.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    Blocked, CallState, FailureOrigin, FrameCallId, FrameError, FrameId, FrameScheduler,
    FrameTransition, FrameUpdate, MAX_CALLS, Notification, ROOT, Round, SCAN_LIMIT,
};
use crate::{
    AttemptId, ClockSample, DriverStep, ExecutionOutcome, ExecutionSource, FanoutFailure, Payload,
    StateView, TaskFailure,
    engine::{Continuation, Phase},
};

#[derive(Debug)]
pub(super) enum Operation {
    Advance,
    Complete {
        attempt: AttemptId,
        result: Result<Payload, TaskFailure>,
    },
    Join(Result<Payload, FanoutFailure>),
}
impl FrameScheduler {
    /// Advance at most one frame/admission/notification, without IO or clock reads.
    ///
    /// Callers yield on Yield and observe resource/timer/capacity events on Park.
    ///
    /// ```
    /// # fn tick(owner:&mut rustack_stepfunctions_core::FrameScheduler,now:rustack_stepfunctions_core::ClockSample) {
    /// let update=owner.advance(now,&mut rustack_stepfunctions_core::SystemUuidSource::default());
    /// # }
    /// ```
    pub fn advance<S: ExecutionSource + ?Sized>(
        &mut self,
        now: ClockSample,
        source: &mut S,
    ) -> FrameUpdate {
        self.observe(now);
        if let Some(call) = self.next_cancel() {
            return FrameUpdate::Cancel(call);
        }
        if let Some(update) = self.pop_notice() {
            return update;
        }
        if self.outcome.is_some() {
            return self.terminal_update();
        }
        let due = self
            .frames
            .iter()
            .filter_map(|(id, frame)| match frame.driver.phase {
                Phase::Calling { deadline, .. } if now.monotonic >= deadline => Some(*id),
                Phase::Waiting { until, .. } if now.monotonic >= until => Some(*id),
                _ => None,
            })
            .collect::<Vec<_>>();
        for id in due {
            self.enqueue(id, false);
        }
        let blocked = self.blocked.keys().copied().collect::<Vec<_>>();
        for id in blocked {
            self.enqueue(id, false);
        }
        for _ in 0..SCAN_LIMIT {
            let Some(id) = self.ready.pop_front() else {
                break;
            };
            self.queued.remove(&id);
            let Some(frame) = self.frames.get(&id) else {
                continue;
            };
            if frame.cancelling {
                self.prune(id);
                continue;
            }
            if frame.driver.outcome().is_some() && frame.notification == Notification::Waiting {
                return self.finished(id);
            }
            if frame.round.is_some() {
                if let Some(update) = self.round(id, now, source) {
                    return update;
                }
            } else if matches!(
                frame.driver.phase,
                Phase::Ready { .. } | Phase::Calling { .. } | Phase::Waiting { .. }
            ) {
                return self.perform(id, Operation::Advance, now, source);
            }
        }
        if self.outcome.is_some() {
            return self.terminal_update();
        }
        if !self.ready.is_empty() {
            return FrameUpdate::Yield;
        }
        let has_progress = self.calls > 0
            || self.frames.values().any(|frame| {
                matches!(
                    frame.driver.phase,
                    Phase::Calling { .. } | Phase::Waiting { .. }
                )
            });
        if !self.blocked.is_empty() && !has_progress {
            self.safety("Rustack.FrameLimitExceeded");
            return self.terminal_update();
        }
        let until =
            self.frames
                .values()
                .fold(self.deadline, |until, frame| match frame.driver.phase {
                    Phase::Calling { deadline, .. } => until.min(deadline),
                    Phase::Waiting { until: wake, .. } => until.min(wake),
                    _ => until,
                });
        FrameUpdate::Park {
            until,
            global_capacity: self
                .blocked
                .values()
                .any(|reason| *reason == Blocked::Global),
        }
    }
    /// Apply a Task result only to its current frame and generation.
    ///
    /// # Errors
    /// Returns Invalid only for a scheduler invariant; stale/duplicate events are inert.
    ///
    /// ```
    /// # fn complete(owner:&mut rustack_stepfunctions_core::FrameScheduler,call:rustack_stepfunctions_core::FrameCallId,result:Result<rustack_stepfunctions_core::Payload,rustack_stepfunctions_core::TaskFailure>,now:rustack_stepfunctions_core::ClockSample) {
    /// let transition=owner.complete(call,result,now,&mut rustack_stepfunctions_core::SystemUuidSource::default());
    /// # }
    /// ```
    pub fn complete<S: ExecutionSource + ?Sized>(
        &mut self,
        call: FrameCallId,
        result: Result<Payload, TaskFailure>,
        now: ClockSample,
        source: &mut S,
    ) -> Result<Option<FrameTransition>, FrameError> {
        self.observe(now);
        if self.outcome.is_some() {
            return Ok(None);
        }
        let current=self.frames.get(&call.frame).is_some_and(|frame|!frame.cancelling && frame.calls.get(&call.attempt)==Some(&CallState::Active) && matches!(frame.driver.phase,Phase::Calling { attempt,.. } if attempt==call.attempt));
        if !current {
            return Ok(None);
        }
        self.cancel(call);
        match self.perform(
            call.frame,
            Operation::Complete {
                attempt: call.attempt,
                result,
            },
            now,
            source,
        ) {
            FrameUpdate::Transition(transition) => Ok(Some(*transition)),
            _ if self.outcome.is_some() => Ok(None),
            _ => Err(FrameError::Invalid),
        }
    }
    pub(super) fn perform<S: ExecutionSource + ?Sized>(
        &mut self,
        id: FrameId,
        operation: Operation,
        now: ClockSample,
        source: &mut S,
    ) -> FrameUpdate {
        let Some(frame) = self.frames.get_mut(&id) else {
            self.internal();
            return self.terminal_update();
        };
        let state = frame.driver.state;
        let machine = frame.driver.machine.clone();
        let parent = frame.parent;
        let entered = matches!(frame.driver.phase, Phase::Ready { entry: true })
            .then(|| frame.driver.input.clone());
        let timed_out = match frame.driver.phase {
            Phase::Calling { attempt, deadline } if now.monotonic >= deadline => {
                Some(FrameCallId { frame: id, attempt })
            }
            _ => None,
        };
        frame.driver.remaining = self.remaining;
        frame.driver.fanout_started = None;
        let preparing = matches!(frame.driver.phase, Phase::Ready { .. })
            && matches!(
                machine.state(state),
                Ok(StateView::Map { .. } | StateView::Parallel { .. })
            );
        let round_finished = match &operation {
            Operation::Join(result) => Some(result.is_ok()),
            _ => None,
        };
        let update = match operation {
            Operation::Advance => frame.driver.advance(now, source),
            Operation::Complete { attempt, result } => {
                frame.driver.complete(attempt, result, now, source)
            }
            Operation::Join(result) => {
                let Phase::Joining { attempt, .. } = frame.driver.phase else {
                    self.internal();
                    return self.terminal_update();
                };
                frame.driver.complete_fanout(attempt, result, now, source)
            }
        };
        self.remaining = frame.driver.remaining;
        let fanout_started = frame.driver.fanout_started;
        let round_finished =
            round_finished.or(fanout_started.filter(|count| *count == 0).map(|_| true));
        let round_finished = round_finished.or(if preparing && fanout_started.is_none() {
            Some(false)
        } else {
            None
        });
        let round_finished = round_finished.filter(|success| {
            *success
                || !matches!(
                    frame.driver.phase,
                    Phase::Waiting {
                        continuation: Continuation::Retry,
                        ..
                    }
                )
        });
        let exited = match &frame.driver.phase {
            Phase::Ready { entry: true } => Some(frame.driver.input.clone()),
            Phase::Terminal(outcome) => match outcome.as_ref() {
                ExecutionOutcome::Succeeded(output) => Some(output.clone()),
                _ => None,
            },
            _ => None,
        };
        let terminal = frame.driver.outcome().cloned();
        if let Some(attempt) = update.cancel {
            self.cancel(FrameCallId { frame: id, attempt });
        }
        if !self.capture_effect(id, &update.step) {
            return self.terminal_update();
        }
        if let Some(outcome) = terminal {
            self.settle_frame(id, parent, outcome);
        } else if self.frames.get(&id).is_some_and(|frame| {
            matches!(
                frame.driver.phase,
                Phase::Ready { .. } | Phase::Joining { .. }
            )
        }) {
            self.enqueue(id, false);
        }
        FrameUpdate::Transition(Box::new(FrameTransition {
            frame: id,
            parent,
            machine,
            state,
            entered,
            exited,
            timed_out,
            step: update.step,
            fanout_started,
            round_finished,
        }))
    }
    fn capture_effect(&mut self, id: FrameId, step: &DriverStep) -> bool {
        if let Some(attempt) = step.task_attempt() {
            if self.calls >= MAX_CALLS {
                self.safety("Rustack.TaskOwnershipLimitExceeded");
                return false;
            }
            if let Some(frame) = self.frames.get_mut(&id) {
                frame.calls.insert(attempt, CallState::Active);
                self.calls += 1;
            }
        }
        if let DriverStep::FanOut(plan) = step {
            if let Some(frame) = self.frames.get_mut(&id) {
                frame.round = Some(Round {
                    plan: plan.clone(),
                    next: 0,
                    running: BTreeSet::new(),
                    outputs: BTreeMap::new(),
                    bytes: 2,
                    failure: None,
                });
            }
        }
        true
    }
    fn settle_frame(
        &mut self,
        id: FrameId,
        parent: Option<super::FrameParent>,
        outcome: std::sync::Arc<ExecutionOutcome>,
    ) {
        let safety = matches!(outcome.as_ref(),ExecutionOutcome::Failed(failure) if matches!(failure.origin,FailureOrigin::Safety));
        if id == ROOT || safety {
            self.terminate(outcome);
        } else {
            if let (Some(parent), ExecutionOutcome::Failed(failure)) = (parent, outcome.as_ref()) {
                self.fail_round_from(parent.frame, failure.clone().into(), Some(id));
            }
            self.enqueue(id, true);
        }
    }
}
