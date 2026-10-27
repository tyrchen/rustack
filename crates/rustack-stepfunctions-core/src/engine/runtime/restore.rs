//! Pure charged terminal-view reconstruction; never starts a runtime job.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use rustack_stepfunctions_model::types::HistoryEvent;
use tokio::sync::watch;

use super::{CleanupState, ExecutionError, ExecutionHandle, ExecutionView};
use crate::{
    ExecutionFailure, ExecutionOutcome, TaskFailure,
    engine::history::{
        HistoryCapacity, HistoryLimits, Journal, TerminalReserve, events, snapshot_charge,
    },
};

#[derive(Debug, Clone)]
pub(crate) struct HistoryRestorer {
    pub(crate) lambda: bool,
    pub(crate) fresh: bool,
    pub(super) limits: HistoryLimits,
    pub(super) capacity: Arc<HistoryCapacity>,
}
impl HistoryRestorer {
    pub(crate) fn used(&self) -> usize {
        self.capacity.used()
    }
    pub(crate) fn estimate(
        events: impl Iterator<Item = usize>,
        terminal: bool,
    ) -> Result<usize, ExecutionError> {
        snapshot_charge(events, terminal)
    }
    pub(crate) fn validate_terminal(
        event: &HistoryEvent,
        outcome: &ExecutionOutcome,
        stopped: DateTime<Utc>,
    ) -> Result<(), ExecutionError> {
        if event.timestamp != stopped {
            return Err(ExecutionError::Invalid);
        }
        let mut expected = events::terminal(outcome, stopped)?;
        expected.id = event.id;
        expected.previous_event_id = event.previous_event_id;
        let expected = serde_json::to_vec(&expected).map_err(|_| ExecutionError::Invalid)?;
        let actual = serde_json::to_vec(event).map_err(|_| ExecutionError::Invalid)?;
        if expected != actual {
            return Err(ExecutionError::Invalid);
        }
        Ok(())
    }
    pub(crate) fn terminal(
        &self,
        events: Vec<HistoryEvent>,
        outcome: Option<ExecutionOutcome>,
        stopped: Option<DateTime<Utc>>,
        wall: DateTime<Utc>,
    ) -> Result<ExecutionHandle, ExecutionError> {
        let interrupted = outcome.is_none();
        let (outcome, stopped) = match (outcome, stopped) {
            (Some(outcome), Some(stopped)) => (outcome, stopped),
            (None, None) => (
                ExecutionOutcome::Aborted(TaskFailure(ExecutionFailure::named(
                    "Rustack.ExecutionInterrupted",
                    "Execution interrupted by local snapshot restore",
                ))),
                wall,
            ),
            _ => return Err(ExecutionError::Invalid),
        };
        let reserve = TerminalReserve::new(&self.capacity)?;
        let mut journal = Journal::new(self.limits, self.capacity.clone(), reserve);
        let mut events = events.into_iter().peekable();
        while let Some(event) = events.next() {
            let previous = u32::try_from(event.previous_event_id.ok_or(ExecutionError::Invalid)?)
                .map_err(|_| ExecutionError::Invalid)?;
            journal.append_causal(event, previous, !interrupted && events.peek().is_none())?;
        }
        if interrupted {
            journal.append(events::terminal(&outcome, wall)?, true)?;
        }
        let (_, view) = watch::channel(Arc::new(ExecutionView {
            history: journal.view,
            outcome: Some(Arc::new(outcome)),
            stopped: Some(stopped),
            cleanup: CleanupState::Clean,
            diagnostic: None,
        }));
        let (stop, _) = watch::channel(None);
        Ok(ExecutionHandle { stop, view })
    }
}
