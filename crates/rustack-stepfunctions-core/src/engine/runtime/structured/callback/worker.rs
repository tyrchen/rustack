//! Joined callback registration/preparation/issuance and terminal/cleanup sequence.

use std::sync::Arc;

use tokio::{
    sync::{oneshot, watch},
    time::{Instant, sleep_until},
};

use super::{
    CallbackError, Context, DispatchLease, ExecutionError, Message, TaskDataBudget, TaskFailure,
    callback_limit_failure,
    owner::{Owner, Prepared},
    progress::{self, Observed, Stage},
    timeout_failure, unknown,
};
use crate::{
    CallbackAttempt, CallbackOutcome, CallbackTimeout, LambdaTaskCommand, Payload, TaskResponder,
};

pub(super) async fn run(
    context: Context,
    mut responder: TaskResponder,
    lease: Arc<DispatchLease>,
    budget: TaskDataBudget,
) {
    let mut owner = Owner::new(lease);
    let result = execute(&mut owner, &context, &mut responder, &budget).await;
    let result = match result {
        Ok(result) => result,
        Err(error) if !unknown(&error) => Err(match error {
            CallbackError::Data(error) => TaskFailure::evaluation(error),
            error if capacity(&error) => callback_limit_failure(),
            _ => timeout_failure(),
        }),
        Err(error) => {
            owner.cancel();
            let _ = context
                .sender
                .send(Message::Fatal(ExecutionError::Callback(Arc::new(error))))
                .await;
            Err(TaskFailure::service_exception())
        }
    };
    let completion = responder.complete(result);
    match owner.cleanup().await {
        Ok(()) => {
            owner.clean();
            completion.settle();
        }
        Err(error) => {
            let _ = context
                .sender
                .send(Message::Fatal(ExecutionError::Callback(Arc::new(error))))
                .await;
        }
    }
}

async fn execute(
    owner: &mut Owner,
    context: &Context,
    responder: &mut TaskResponder,
    budget: &TaskDataBudget,
) -> Result<Result<Payload, TaskFailure>, CallbackError> {
    let deadline = Instant::from_std(context.effect.deadline());
    let mut cancel = responder.cancellation();
    if progress::stopped(&cancel, deadline) {
        return Err(CallbackError::TaskTimedOut);
    }
    let (registry, parent) = &context.binding;
    owner.receipts()?.token.mint = Some(registry.register(CallbackAttempt::new(
        parent,
        context.id,
        deadline,
        context.effect.heartbeat(),
    )?)?);
    let mint = owner
        .receipts()?
        .token
        .mint
        .as_mut()
        .ok_or(CallbackError::OwnerLost)?;
    let accepted = tokio::select! {biased;_=cancel.changed()=>{mint.revoke();mint.resolve().await},()=sleep_until(deadline)=>{mint.revoke();mint.resolve().await},result=mint.resolve()=>result,};
    if let Err(error) = accepted {
        if !unknown(&error) {
            owner.receipts()?.token.mint = None;
        }
        return Err(error);
    }
    let mint = owner
        .receipts()?
        .token
        .mint
        .take()
        .ok_or(CallbackError::OwnerLost)?;
    owner.receipts()?.token.task = Some(mint.into_task()?);
    if progress::stopped(&cancel, deadline) {
        return Err(CallbackError::TaskTimedOut);
    }
    let prepared = match owner.prepare(context, budget, &mut cancel).await? {
        Ok(prepared) => prepared,
        Err(failure) => return Ok(Err(failure)),
    };
    if let Err(failure) = issue(owner, context, prepared, &mut cancel).await? {
        return Ok(Err(failure));
    }
    let report =
        match progress::observe(owner, context, Stage::Invocation, responder, &mut cancel).await? {
            Observed::Invocation(report) => report,
            Observed::Callback(_) => return Err(CallbackError::Progress),
        };
    if let Some(failure) = report.failure() {
        return Ok(Err(failure.clone()));
    }
    let metadata = report.metadata().ok_or(CallbackError::Progress)?;
    responder
        .submitted(metadata.payload()?)
        .map_err(|_| CallbackError::Progress)?;
    let outcome =
        match progress::observe(owner, context, Stage::Callback, responder, &mut cancel).await? {
            Observed::Callback(outcome) => outcome,
            Observed::Invocation(_) => return Err(CallbackError::Progress),
        };
    match outcome.as_ref() {
        CallbackOutcome::Succeeded(output) => Ok(Ok(output.clone())),
        CallbackOutcome::Failed(failure) | CallbackOutcome::InvocationFailed(failure) => {
            Ok(Err(failure.clone()))
        }
        CallbackOutcome::TimedOut(CallbackTimeout::Heartbeat) => Ok(Err(TaskFailure::new(
            "States.HeartbeatTimeout",
            Some("Callback heartbeat deadline exceeded"),
        )?)),
        CallbackOutcome::TimedOut(CallbackTimeout::Task) => Ok(Err(timeout_failure())),
        CallbackOutcome::Revoked => Err(CallbackError::OwnerLost),
        CallbackOutcome::Unavailable => Err(CallbackError::Unavailable),
        CallbackOutcome::Invoking | CallbackOutcome::Waiting { .. } => Err(CallbackError::Progress),
    }
}
async fn issue(
    owner: &mut Owner,
    context: &Context,
    prepared: Prepared,
    cancel: &mut watch::Receiver<bool>,
) -> Result<Result<(), TaskFailure>, CallbackError> {
    let deadline = Instant::from_std(context.effect.deadline());
    let Prepared {
        dispatch,
        history,
        charge,
    } = prepared;
    owner.receipts()?.physical.charge = Some(charge);
    if progress::stopped(cancel, deadline) {
        return Err(CallbackError::TaskTimedOut);
    }
    let (ack, mut acknowledged) = oneshot::channel();
    let scheduled = Message::CallbackScheduled(context.id, history, ack);
    tokio::select! {biased;_=cancel.changed()=>return Err(CallbackError::TaskTimedOut),()=sleep_until(deadline)=>return Err(CallbackError::TaskTimedOut),result=context.sender.send(scheduled)=>result.map_err(|_|CallbackError::OwnerLost)?,};
    let allowed = tokio::select! {biased;_=cancel.changed()=>return Err(CallbackError::TaskTimedOut),()=sleep_until(deadline)=>return Err(CallbackError::TaskTimedOut),result=&mut acknowledged=>result.map_err(|_|CallbackError::OwnerLost)?,};
    if !allowed || progress::stopped(cancel, deadline) {
        return Err(CallbackError::TaskTimedOut);
    }
    let permit = tokio::select! {biased;_=cancel.changed()=>return Err(CallbackError::TaskTimedOut),()=sleep_until(deadline)=>return Err(CallbackError::TaskTimedOut),permit=context.shared.pools.dispatch.clone().acquire_owned()=>permit.map_err(|_|CallbackError::Unavailable)?,};
    owner.receipts()?.physical.permit = Some(permit);
    if progress::stopped(cancel, deadline) {
        return Err(CallbackError::TaskTimedOut);
    }
    let task = owner
        .receipts()?
        .token
        .task
        .as_mut()
        .ok_or(CallbackError::OwnerLost)?;
    let command = LambdaTaskCommand::observed_callback(
        context.effect.machine().clone(),
        dispatch,
        context.id,
        task.take_invocation_reporter()?,
    )?;
    let call = context
        .shared
        .port
        .as_ref()
        .ok_or(CallbackError::Unavailable)?
        .start(command);
    match call {
        Ok(call) => owner.receipts()?.physical.call = Some(call),
        Err(failure) => {
            // Prequeue rejection is known; report it through the outer business path.
            return Ok(Err(failure));
        }
    }
    Ok(Ok(()))
}

fn capacity(error: &CallbackError) -> bool {
    match error {
        CallbackError::Cached(error) => capacity(error),
        CallbackError::Capacity => true,
        _ => false,
    }
}
