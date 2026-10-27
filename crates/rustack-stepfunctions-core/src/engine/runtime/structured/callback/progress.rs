//! Drive report/result and independent physical settlement without dropping either.

use std::{future::pending, sync::Arc};

use tokio::{
    sync::watch,
    time::{Instant, sleep_until},
};

use super::{CallbackError, Context, owner::Owner};
use crate::{
    CallbackInvocation, CallbackOutcome, CallbackTask, TaskCall, TaskChannelError, TaskEvent,
    TaskResponder, TaskSettlement,
};

#[derive(Debug, Clone, Copy)]
pub(super) enum Stage {
    Invocation,
    Callback,
}
#[derive(Debug)]
pub(super) enum Observed {
    Invocation(Arc<CallbackInvocation>),
    Callback(Arc<CallbackOutcome>),
}
enum Wake {
    Task(Result<Observed, CallbackError>),
    Progress(Result<TaskEvent, TaskChannelError>),
    Settlement(Result<(), watch::error::RecvError>),
    Cancel,
}
async fn event(call: &mut Option<TaskCall>) -> Result<TaskEvent, TaskChannelError> {
    match call {
        Some(call) => call.next().await,
        None => pending().await,
    }
}
async fn settlement(
    receiver: &mut Option<watch::Receiver<TaskSettlement>>,
) -> Result<(), watch::error::RecvError> {
    match receiver {
        Some(receiver) => receiver.changed().await,
        None => pending().await,
    }
}
async fn task(
    task: &mut CallbackTask,
    stage: Stage,
    deadline: Instant,
) -> Result<Observed, CallbackError> {
    match stage {
        Stage::Invocation => task
            .wait_invocation(deadline)
            .await
            .map(Observed::Invocation),
        Stage::Callback => task.resolved().await.map(Observed::Callback),
    }
}
pub(super) fn stopped(cancel: &watch::Receiver<bool>, deadline: Instant) -> bool {
    *cancel.borrow() || cancel.has_changed().is_err() || Instant::now() >= deadline
}
pub(super) async fn observe(
    owner: &mut Owner,
    context: &Context,
    stage: Stage,
    responder: &mut TaskResponder,
    cancel: &mut watch::Receiver<bool>,
) -> Result<Observed, CallbackError> {
    let deadline = Instant::from_std(context.effect.deadline());
    let mut settled = owner
        .receipts()?
        .physical
        .call
        .as_ref()
        .map(TaskCall::settlement_receiver);
    loop {
        if stopped(cancel, deadline) {
            owner.cancel();
            return Err(CallbackError::TaskTimedOut);
        }
        let receipts = owner.receipts()?;
        receipts.release_clean();
        let active = receipts.physical.call.is_some();
        let task_owner = receipts
            .token
            .task
            .as_mut()
            .ok_or(CallbackError::OwnerLost)?;
        let wake = tokio::select! {
            biased;
            _=cancel.changed()=>Wake::Cancel,
            ()=sleep_until(deadline)=>Wake::Cancel,
            result=event(&mut receipts.physical.call),if active&&!receipts.physical.completed=>Wake::Progress(result),
            result=task(task_owner,stage,deadline)=>Wake::Task(result),
            result=settlement(&mut settled),if active&&receipts.physical.call.as_ref().is_some_and(|call|call.settlement()!=TaskSettlement::Clean)=>Wake::Settlement(result),
        };
        if stopped(cancel, deadline) {
            owner.cancel();
            return Err(CallbackError::TaskTimedOut);
        }
        match wake {
            Wake::Cancel => {
                owner.cancel();
                return Err(CallbackError::TaskTimedOut);
            }
            Wake::Progress(Ok(TaskEvent::Started)) => {
                responder.started().map_err(|_| CallbackError::Progress)?;
            }
            Wake::Progress(Ok(TaskEvent::Completed(_))) => {
                owner.receipts()?.physical.completed = true;
            }
            Wake::Task(Err(CallbackError::TaskTimedOut | CallbackError::Deadline))
            | Wake::Progress(Ok(TaskEvent::Submitted(_)) | Err(_)) => {
                return Err(CallbackError::OwnerLost);
            }
            Wake::Task(result) => return result,
            Wake::Settlement(Ok(())) => {}
            Wake::Settlement(Err(_)) => {
                owner.receipts()?.release_clean();
                if owner
                    .receipts()?
                    .physical
                    .call
                    .as_ref()
                    .is_some_and(|call| call.settlement() != TaskSettlement::Clean)
                {
                    return Err(CallbackError::OwnerLost);
                }
                settled = None;
            }
        }
    }
}
