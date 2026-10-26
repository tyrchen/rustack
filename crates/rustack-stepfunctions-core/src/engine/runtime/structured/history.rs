//! Per-frame causal cursors and typed structured lifecycle projection.

use std::{collections::BTreeMap, sync::Arc};

use chrono::{DateTime, Utc};
use rustack_stepfunctions_model::types::{
    HistoryEvent, HistoryEventType, MapIterationEventDetails, MapStateStartedEventDetails,
};

use crate::{
    AslError, CompiledMachine, ExecutionOutcome, FrameId, FrameParent, FrameScheduler,
    FrameTransition, StateId, StateView, TaskFailure,
    engine::{
        history::{Journal, events},
        runtime::ExecutionError,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Plain,
    Map,
    Parallel,
    Task,
    Wait,
}
#[derive(Debug)]
struct Cursor {
    machine: Arc<CompiledMachine>,
    state: StateId,
    last: u32,
    round: Option<u32>,
    kind: Kind,
    active: bool,
    parent: Option<FrameParent>,
}
#[derive(Debug)]
pub(super) struct CausalHistory {
    pub(super) journal: Journal,
    frames: BTreeMap<FrameId, Cursor>,
    root: Option<FrameId>,
}
impl CausalHistory {
    pub(super) fn new(journal: Journal) -> Self {
        Self {
            journal,
            frames: BTreeMap::new(),
            root: None,
        }
    }
    fn append(&mut self, id: FrameId, event: HistoryEvent) -> Result<(), ExecutionError> {
        let previous = self.frames.get(&id).ok_or(ExecutionError::Invalid)?.last;
        self.journal.append_causal(event, previous, false)?;
        let cursor = self.frames.get_mut(&id).ok_or(ExecutionError::Invalid)?;
        cursor.last = self.journal.view.len();
        Ok(())
    }
    pub(super) fn event(
        &mut self,
        id: FrameId,
        event: Result<HistoryEvent, AslError>,
    ) -> Result<(), ExecutionError> {
        self.append(id, event.map_err(ExecutionError::from)?)
    }
    pub(super) fn transition(
        &mut self,
        t: &FrameTransition,
        wall: DateTime<Utc>,
    ) -> Result<(), ExecutionError> {
        if !self.frames.contains_key(&t.frame()) {
            if t.parent().is_some() {
                return Err(ExecutionError::Invalid);
            }
            self.root = Some(t.frame());
            self.frames.insert(
                t.frame(),
                Cursor {
                    machine: t.machine().clone(),
                    state: t.state(),
                    last: self.journal.view.len(),
                    round: None,
                    kind: Kind::Plain,
                    active: false,
                    parent: None,
                },
            );
        }
        if let Some(input) = t.entered() {
            let cursor = self
                .frames
                .get_mut(&t.frame())
                .ok_or(ExecutionError::Invalid)?;
            cursor.machine = t.machine().clone();
            cursor.state = t.state();
            cursor.round = None;
            cursor.active = true;
            cursor.kind = match t.machine().state(t.state())? {
                StateView::Map { .. } => Kind::Map,
                StateView::Parallel { .. } => Kind::Parallel,
                StateView::Task { .. } => Kind::Task,
                StateView::Wait { .. } => Kind::Wait,
                _ => Kind::Plain,
            };
            self.event(
                t.frame(),
                events::entered(t.machine(), t.state(), input, wall),
            )?;
        }
        if let Some(call) = t.timed_out_call() {
            let failure = TaskFailure::new("States.Timeout", Some("Task deadline exceeded"))?;
            self.event(
                call.frame(),
                events::task_failure(
                    t.machine(),
                    t.state(),
                    &failure,
                    events::TaskFailureKind::TimedOut,
                    wall,
                ),
            )?;
        }
        if let Some(count) = t.fanout_started {
            self.round_started(t.frame(), count, wall)?;
        }
        if let Some(success) = t.round_finished {
            let kind = self
                .frames
                .get(&t.frame())
                .ok_or(ExecutionError::Invalid)?
                .kind;
            let event = match (kind, success) {
                (Kind::Map, true) => HistoryEventType::MapStateSucceeded,
                (Kind::Map, false) => HistoryEventType::MapStateFailed,
                (Kind::Parallel, true) => HistoryEventType::ParallelStateSucceeded,
                (Kind::Parallel, false) => HistoryEventType::ParallelStateFailed,
                _ => return Err(ExecutionError::Invalid),
            };
            self.append(t.frame(), events::event(event, wall))?;
        }
        if let Some(output) = t.exited() {
            self.event(
                t.frame(),
                events::exited(t.machine(), t.state(), output, wall),
            )?;
        }
        if t.exited().is_some() {
            if let Some(cursor) = self.frames.get_mut(&t.frame()) {
                cursor.active = false;
            }
        }
        Ok(())
    }
    fn round_started(
        &mut self,
        id: FrameId,
        count: usize,
        wall: DateTime<Utc>,
    ) -> Result<(), ExecutionError> {
        let kind = self.frames.get(&id).ok_or(ExecutionError::Invalid)?.kind;
        let mut event = events::event(
            if kind == Kind::Map {
                HistoryEventType::MapStateStarted
            } else {
                HistoryEventType::ParallelStateStarted
            },
            wall,
        );
        if kind == Kind::Map {
            let mut details = MapStateStartedEventDetails::default();
            details.length = Some(i32::try_from(count).map_err(|_| ExecutionError::Invalid)?);
            event.map_state_started_event_details = Some(details);
        }
        self.append(id, event)?;
        let cursor = self.frames.get_mut(&id).ok_or(ExecutionError::Invalid)?;
        cursor.round = Some(cursor.last);
        Ok(())
    }
    pub(super) fn child_started(
        &mut self,
        id: FrameId,
        parent: FrameParent,
        graph: &FrameScheduler,
        wall: DateTime<Utc>,
    ) -> Result<(), ExecutionError> {
        let parent_cursor = self
            .frames
            .get(&parent.frame())
            .ok_or(ExecutionError::Invalid)?;
        let previous = parent_cursor.round.ok_or(ExecutionError::Invalid)?;
        let map = parent_cursor.kind == Kind::Map;
        let name = parent_cursor
            .machine
            .state_name(parent_cursor.state)?
            .to_owned();
        let child = graph.driver(id).ok_or(ExecutionError::Invalid)?;
        self.frames.insert(
            id,
            Cursor {
                machine: child.machine().clone(),
                state: child.state(),
                last: previous,
                round: None,
                kind: Kind::Plain,
                active: false,
                parent: Some(parent),
            },
        );
        if map {
            self.append(
                id,
                iteration(
                    HistoryEventType::MapIterationStarted,
                    &name,
                    parent.index(),
                    wall,
                )?,
            )?;
        }
        Ok(())
    }
    pub(super) fn child_finished(
        &mut self,
        id: FrameId,
        parent: FrameParent,
        outcome: &ExecutionOutcome,
        wall: DateTime<Utc>,
    ) -> Result<(), ExecutionError> {
        if !self.frames.contains_key(&id) {
            return Ok(());
        }
        if matches!(outcome, ExecutionOutcome::Aborted(_)) {
            self.abort_state(id, wall)?;
        }
        let owner = self
            .frames
            .get(&parent.frame())
            .ok_or(ExecutionError::Invalid)?;
        let kind = owner.kind;
        let name = owner.machine.state_name(owner.state)?.to_owned();
        if kind == Kind::Map {
            let event = match outcome {
                ExecutionOutcome::Succeeded(_) => HistoryEventType::MapIterationSucceeded,
                ExecutionOutcome::Failed(_) => HistoryEventType::MapIterationFailed,
                _ => HistoryEventType::MapIterationAborted,
            };
            self.append(id, iteration(event, &name, parent.index(), wall)?)?;
        }
        let child = self.frames.remove(&id).ok_or(ExecutionError::Invalid)?;
        let owner = self
            .frames
            .get_mut(&parent.frame())
            .ok_or(ExecutionError::Invalid)?;
        owner.last = child.last;
        Ok(())
    }
    fn abort_state(&mut self, id: FrameId, wall: DateTime<Utc>) -> Result<(), ExecutionError> {
        let cursor = self.frames.get(&id).ok_or(ExecutionError::Invalid)?;
        if !cursor.active {
            return Ok(());
        }
        let event = match cursor.kind {
            Kind::Map => Some(HistoryEventType::MapStateAborted),
            Kind::Parallel => Some(HistoryEventType::ParallelStateAborted),
            Kind::Task => Some(HistoryEventType::TaskStateAborted),
            Kind::Wait => Some(HistoryEventType::WaitStateAborted),
            Kind::Plain => None,
        };
        if let Some(event) = event {
            self.append(id, events::event(event, wall))?;
        }
        if let Some(cursor) = self.frames.get_mut(&id) {
            cursor.active = false;
        }
        Ok(())
    }
    pub(super) fn abort(&mut self, wall: DateTime<Utc>) -> Result<(), ExecutionError> {
        let ids = self.frames.keys().rev().copied().collect::<Vec<_>>();
        for id in ids {
            self.abort_state(id, wall)?;
            let parent = self.frames.get(&id).and_then(|cursor| cursor.parent);
            if let Some(parent) = parent {
                let owner = self
                    .frames
                    .get(&parent.frame())
                    .ok_or(ExecutionError::Invalid)?;
                let name = owner.machine.state_name(owner.state)?.to_owned();
                if owner.kind == Kind::Map {
                    self.append(
                        id,
                        iteration(
                            HistoryEventType::MapIterationAborted,
                            &name,
                            parent.index(),
                            wall,
                        )?,
                    )?;
                }
                let last = self.frames.get(&id).ok_or(ExecutionError::Invalid)?.last;
                self.frames
                    .get_mut(&parent.frame())
                    .ok_or(ExecutionError::Invalid)?
                    .last = last;
            }
        }
        Ok(())
    }
    pub(super) fn terminal(
        &mut self,
        outcome: &ExecutionOutcome,
        wall: DateTime<Utc>,
    ) -> Result<(), ExecutionError> {
        let previous = self
            .root
            .and_then(|id| self.frames.get(&id).map(|cursor| cursor.last))
            .unwrap_or(1);
        self.journal
            .append_causal(events::terminal(outcome, wall)?, previous, true)
    }
}
fn iteration(
    kind: HistoryEventType,
    name: &str,
    index: usize,
    wall: DateTime<Utc>,
) -> Result<HistoryEvent, ExecutionError> {
    let mut event = events::event(kind, wall);
    let mut details = MapIterationEventDetails::default();
    details.name = Some(name.into());
    details.index = Some(i32::try_from(index).map_err(|_| ExecutionError::Invalid)?);
    match &event.r#type {
        HistoryEventType::MapIterationStarted => {
            event.map_iteration_started_event_details = Some(details);
        }
        HistoryEventType::MapIterationSucceeded => {
            event.map_iteration_succeeded_event_details = Some(details);
        }
        HistoryEventType::MapIterationFailed => {
            event.map_iteration_failed_event_details = Some(details);
        }
        HistoryEventType::MapIterationAborted => {
            event.map_iteration_aborted_event_details = Some(details);
        }
        _ => return Err(ExecutionError::Invalid),
    }
    Ok(event)
}
