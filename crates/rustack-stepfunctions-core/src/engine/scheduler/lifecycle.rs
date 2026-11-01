//! Lazy admission, indexed results and failure cleanup barriers.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    Frame, FrameId, FrameLease, FrameParent, FrameScheduler, FrameUpdate, Notification,
    RESULT_OVERHEAD, ResultSlot, Round, budget::Slot, drive::Operation,
};
use crate::{
    AslError, ClockSample, ExecutionDriver, ExecutionOutcome, ExecutionSource, FanoutFailure,
    MAX_PAYLOAD_BYTES, Payload, payload::encode,
};

#[derive(Debug)]
enum StoreError {
    Evaluation(AslError),
    Capacity,
}

impl FrameScheduler {
    pub(super) fn round<S: ExecutionSource + ?Sized>(
        &mut self,
        id: FrameId,
        now: ClockSample,
        source: &mut S,
    ) -> Option<FrameUpdate> {
        let owner = self.frames.get(&id)?;
        let round = owner.round.as_ref()?;
        let attempt = round.plan.attempt();
        let waiting = owner.children.iter().any(|child| {
            self.frames
                .get(child)
                .and_then(|f| f.parent)
                .is_some_and(|parent| parent.round == attempt)
        });
        if round.failure.is_some() && waiting {
            return None;
        }
        if round.failure.is_some() || round.outputs.len() == round.plan.item_count() {
            let round = self.frames.get_mut(&id)?.round.take()?;
            self.blocked.remove(&id);
            return Some(self.join_round(id, round, now, source));
        }
        if round.next >= round.plan.item_count()
            || round.running.len() >= round.plan.max_concurrency().get()
        {
            return None;
        }
        let index = round.next;
        let plan = round.plan.clone();
        let slot = match Slot::acquire(&self.capacity, &self.local_frames) {
            Ok(slot) => slot,
            Err(reason) => {
                self.blocked.insert(id, reason);
                return None;
            }
        };
        self.blocked.remove(&id);
        let branch = match plan.branch(index, source) {
            Ok(branch) => branch,
            Err(error) => {
                self.fail_round(id, error.into());
                return Some(FrameUpdate::Yield);
            }
        };
        let driver = match ExecutionDriver::frame(
            branch.machine().clone(),
            branch.input().clone(),
            branch.context().clone(),
            now,
            plan.deadline(),
            self.remaining,
        ) {
            Ok(driver) => driver,
            Err(error) => {
                self.fail_round(id, error.into());
                return Some(FrameUpdate::Yield);
            }
        };
        let child = FrameId(self.next);
        let Some(next) = self.next.checked_add(1) else {
            self.internal();
            return Some(self.terminal_update());
        };
        self.next = next;
        let parent = FrameParent {
            frame: id,
            round: attempt,
            index,
        };
        let Some(owner) = self.frames.get_mut(&id) else {
            self.internal();
            return Some(self.terminal_update());
        };
        let Some(round) = owner.round.as_mut() else {
            self.internal();
            return Some(self.terminal_update());
        };
        round.next = round.next.saturating_add(1);
        round.running.insert(child);
        owner.children.insert(child);
        let lineage = owner.lease.clone();
        self.frames.insert(
            child,
            Frame {
                driver,
                parent: Some(parent),
                children: BTreeSet::new(),
                round: None,
                calls: BTreeMap::new(),
                notification: Notification::Waiting,
                cancelling: false,
                lease: FrameLease::child(slot, Some(lineage)),
            },
        );
        self.enqueue(id, false);
        self.enqueue(child, true);
        Some(FrameUpdate::ChildStarted { child, parent })
    }
    fn join_round<S: ExecutionSource + ?Sized>(
        &mut self,
        id: FrameId,
        round: Round,
        now: ClockSample,
        source: &mut S,
    ) -> FrameUpdate {
        let result = if let Some(failure) = round.failure {
            Err(failure)
        } else {
            let outputs = round
                .outputs
                .iter()
                .map(|(index, slot)| (*index, slot.payload.clone()))
                .collect::<Vec<_>>();
            round.plan.collect(&outputs).map_err(FanoutFailure::from)
        };
        self.perform(id, Operation::Join(result), now, source)
    }
    pub(super) fn fail_round(&mut self, id: FrameId, failure: FanoutFailure) {
        self.fail_round_from(id, failure, None);
    }
    pub(super) fn fail_round_from(
        &mut self,
        id: FrameId,
        failure: FanoutFailure,
        ignore: Option<FrameId>,
    ) {
        let attempt = if let Some(round) = self.frames.get_mut(&id).and_then(|f| f.round.as_mut()) {
            if round.failure.is_some() {
                return;
            }
            round.failure = Some(failure);
            round.outputs.clear();
            round.plan.attempt()
        } else {
            self.internal();
            return;
        };
        self.blocked.remove(&id);
        self.cancel_round(id, attempt, ignore);
        self.enqueue(id, false);
    }
    pub(super) fn finished(&mut self, id: FrameId) -> FrameUpdate {
        let Some(frame) = self.frames.get_mut(&id) else {
            self.internal();
            return self.terminal_update();
        };
        let Some(parent) = frame.parent else {
            self.internal();
            return self.terminal_update();
        };
        let Some(outcome) = frame.driver.outcome().cloned() else {
            self.internal();
            return self.terminal_update();
        };
        frame.notification = Notification::Emitted;
        let current = self
            .frames
            .get_mut(&parent.frame)
            .and_then(|f| f.round.as_mut());
        let Some(round) = current else {
            self.internal();
            return self.terminal_update();
        };
        if round.plan.attempt() != parent.round || !round.running.remove(&id) {
            self.internal();
            return self.terminal_update();
        }
        match outcome.as_ref() {
            ExecutionOutcome::Succeeded(output) => match self.store(parent, output) {
                Ok(()) => {}
                Err(StoreError::Evaluation(error)) => self.fail_round(parent.frame, error.into()),
                Err(StoreError::Capacity) => self.safety("Rustack.ResultLimitExceeded"),
            },
            ExecutionOutcome::Failed(failure) => {
                self.fail_round(parent.frame, failure.clone().into());
            }
            _ => self.internal(),
        }
        self.prune(id);
        self.enqueue(parent.frame, false);
        FrameUpdate::ChildFinished {
            child: id,
            parent,
            outcome,
        }
    }
    fn store(&mut self, parent: FrameParent, payload: &Payload) -> Result<(), StoreError> {
        let compact = encode(payload.value()).map_err(StoreError::Evaluation)?;
        let round = self
            .frames
            .get_mut(&parent.frame)
            .and_then(|frame| frame.round.as_mut())
            .ok_or(StoreError::Evaluation(AslError::InvalidStateId))?;
        if round.outputs.contains_key(&parent.index) || parent.index >= round.plan.item_count() {
            return Err(StoreError::Evaluation(AslError::InvalidFanoutResult));
        }
        let bytes = round
            .bytes
            .checked_add(compact.len())
            .and_then(|n| n.checked_add(usize::from(!round.outputs.is_empty())))
            .filter(|n| *n <= MAX_PAYLOAD_BYTES)
            .ok_or(StoreError::Evaluation(AslError::DataLimit))?;
        let weight = payload
            .original_bytes()
            .len()
            .checked_add(compact.len())
            .and_then(|n| n.checked_add(RESULT_OVERHEAD))
            .and_then(|n| u32::try_from(n).ok())
            .ok_or(StoreError::Capacity)?;
        let local = self
            .local_results
            .clone()
            .try_acquire_many_owned(weight)
            .map_err(|_| StoreError::Capacity)?;
        let global = self
            .capacity
            .results
            .clone()
            .try_acquire_many_owned(weight)
            .map_err(|_| StoreError::Capacity)?;
        round.bytes = bytes;
        round.outputs.insert(
            parent.index,
            ResultSlot {
                payload: payload.clone(),
                _global: global,
                _local: local,
            },
        );
        Ok(())
    }
}
