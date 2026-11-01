//! Registered token/history/physical-call ownership for deferred callback effects.

use std::sync::Arc;

use chrono::Utc;
use tokio::{
    sync::{mpsc, oneshot},
    time::Instant,
};

use super::{Actor, DispatchLease, ExecutionError, Instruction, Message, Progress, Shared};
use crate::{
    CallbackDispatch, CallbackError, CallbackHistory, CallbackRegistry, ExecutionFailure,
    ExecutionSnapshot, FrameCallId, MAX_PAYLOAD_BYTES, TaskCall, TaskFailure,
    engine::scheduler::TaskDataBudget,
};

mod owner;
mod progress;
mod worker;

const PROTOCOL_BYTES: usize = 65_536;
const PREPARATION_OVERHEAD: usize = 256;
const RESULT_BYTES: usize = MAX_PAYLOAD_BYTES + PROTOCOL_BYTES;
const PREPARATION_BYTES: usize = 2 * MAX_PAYLOAD_BYTES + PREPARATION_OVERHEAD;
#[derive(Debug)]
struct Context {
    id: FrameCallId,
    effect: CallbackDispatch,
    binding: (CallbackRegistry, ExecutionSnapshot),
    shared: Arc<Shared>,
    sender: mpsc::Sender<Message>,
}
impl Actor {
    pub(super) fn start_callback(
        &mut self,
        id: FrameCallId,
        effect: CallbackDispatch,
    ) -> Result<(), ExecutionError> {
        let binding = (
            self.shared
                .callbacks
                .clone()
                .ok_or(ExecutionError::Unavailable)?,
            self.parent.clone().ok_or(ExecutionError::Unavailable)?,
        );
        let graph = self.graph.as_ref().ok_or(ExecutionError::Invalid)?;
        let frame = graph.lease(id.frame()).ok_or(ExecutionError::Invalid)?;
        let budget = graph.task_data_budget();
        let (data, admitted) = match budget.reserve(RESULT_BYTES) {
            Ok(data) => (Some(data), true),
            Err(_) => (None, false),
        };
        let lease = Arc::new(DispatchLease {
            _permit: None,
            _frame: Some(frame),
            _data: data,
        });
        self.leases.push(lease.clone());
        self.sync_ledger();
        let (call, responder) = TaskCall::channel();
        let machine = effect.machine().clone();
        if admitted {
            self.callback_jobs.spawn(worker::run(
                Context {
                    id,
                    effect: effect.clone(),
                    binding,
                    shared: self.shared.clone(),
                    sender: self.sender.clone(),
                },
                responder,
                lease.clone(),
                budget,
            ));
        } else {
            responder.finish(Err(limit_failure()));
        }
        self.accepted_instruction(id, machine, Instruction::Callback(effect), call, lease);
        Ok(())
    }
    pub(super) fn callback_scheduled(
        &mut self,
        id: FrameCallId,
        history: &CallbackHistory,
        ack: oneshot::Sender<bool>,
    ) {
        self.control();
        let current = self.current(id);
        let allowed = !self.published
            && current
            && self.calls.get(&id).is_some_and(|call| {
                !call.cancelled
                    && !call.result_seen
                    && call.progress == Progress::Preparing
                    && Instant::now().into_std() < call.dispatch.deadline()
            });
        if !allowed {
            let _ = ack.send(false);
            return;
        }
        let result =
            self.calls
                .get(&id)
                .ok_or(ExecutionError::Invalid)
                .and_then(|call| match &call.dispatch {
                    Instruction::Callback(effect) => effect
                        .scheduled_event(history, Utc::now())
                        .map_err(ExecutionError::from),
                    Instruction::Task(_) => Err(ExecutionError::Invalid),
                });
        match result.and_then(|event| self.history.event(id.frame(), Ok(event))) {
            Ok(()) => {
                if let Some(call) = self.calls.get_mut(&id) {
                    call.progress = Progress::Pending;
                }
                self.publish_running();
                self.control();
                let allowed = !self.published
                    && self.calls.get(&id).is_some_and(|call| {
                        !call.cancelled && Instant::now().into_std() < call.dispatch.deadline()
                    });
                let _ = ack.send(allowed);
            }
            Err(error) => {
                let _ = ack.send(false);
                self.fail(error);
            }
        }
    }
}
fn limit_failure() -> TaskFailure {
    TaskFailure::safety(
        "Rustack.ResultLimitExceeded",
        "Callback data budget exhausted",
    )
}
fn callback_limit_failure() -> TaskFailure {
    TaskFailure::safety(
        "Rustack.CallbackLimitExceeded",
        "Callback token capacity exhausted",
    )
}
fn timeout_failure() -> TaskFailure {
    TaskFailure(ExecutionFailure::fields(
        Some("States.Timeout".into()),
        None,
    ))
}

fn unknown(error: &CallbackError) -> bool {
    match error {
        CallbackError::Cached(source) => unknown(source),
        CallbackError::TaskTimedOut
        | CallbackError::Deadline
        | CallbackError::Capacity
        | CallbackError::Data(_) => false,
        _ => true,
    }
}

#[cfg(test)]
mod tests;
