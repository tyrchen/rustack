//! Unique retained receipts; panic/drop never asserts cleanup or releases charges.

use std::sync::Arc;

use tokio::{
    sync::{OwnedSemaphorePermit, watch},
    task::{JoinHandle, spawn_blocking},
    time::{Instant, sleep_until},
};

use super::{
    CallbackError, Context, DispatchLease, PREPARATION_BYTES, PREPARATION_OVERHEAD, TaskDataBudget,
    limit_failure, unknown,
};
use crate::{
    AslError, CallbackHistory, CallbackMint, CallbackOutcome, CallbackTask, CallbackToken,
    SystemUuidSource, TaskCall, TaskDispatch, TaskFailure, TaskSettlement,
    engine::scheduler::TaskBytes,
};

#[derive(Debug)]
pub(super) struct Prepared {
    pub(super) dispatch: TaskDispatch,
    pub(super) history: CallbackHistory,
    pub(super) charge: TaskBytes,
}
#[derive(Debug, Default)]
pub(super) struct TokenWork {
    pub(super) mint: Option<CallbackMint>,
    pub(super) task: Option<CallbackTask>,
    pub(super) cpu: Option<JoinHandle<Result<Prepared, AslError>>>,
}
#[derive(Debug, Default)]
pub(super) struct Physical {
    pub(super) call: Option<TaskCall>,
    pub(super) charge: Option<TaskBytes>,
    pub(super) permit: Option<OwnedSemaphorePermit>,
    pub(super) completed: bool,
}
#[derive(Debug)]
pub(super) struct Receipts {
    pub(super) token: TokenWork,
    pub(super) physical: Physical,
    pub(super) lease: Arc<DispatchLease>,
}
#[derive(Debug)]
pub(super) struct Owner(pub(super) Option<Box<Receipts>>);
impl Drop for Owner {
    fn drop(&mut self) {
        if let Some(receipts) = self.0.take() {
            receipts.cancel();
            // The root port/token actors retain accepted IO independently. Keep
            // these exact receipts, physical permit and frame/data charges too.
            let _retained = Box::leak(receipts);
        }
    }
}
impl Receipts {
    fn cancel(&self) {
        if let Some(mint) = &self.token.mint {
            mint.revoke();
        }
        if let Some(task) = &self.token.task {
            task.revoke();
        }
        if let Some(call) = &self.physical.call {
            call.cancel();
        }
    }
    pub(super) fn release_clean(&mut self) {
        if self
            .physical
            .call
            .as_ref()
            .is_some_and(|call| call.settlement() == TaskSettlement::Clean)
        {
            self.physical.charge = None;
            self.physical.permit = None;
        }
    }
}
impl Owner {
    pub(super) fn new(lease: Arc<DispatchLease>) -> Self {
        Self(Some(Box::new(Receipts {
            token: TokenWork::default(),
            physical: Physical::default(),
            lease,
        })))
    }
    pub(super) fn receipts(&mut self) -> Result<&mut Receipts, CallbackError> {
        self.0.as_deref_mut().ok_or(CallbackError::OwnerLost)
    }
    pub(super) fn cancel(&self) {
        if let Some(receipts) = &self.0 {
            receipts.cancel();
        }
    }
    pub(super) fn clean(&mut self) {
        self.0 = None;
    }
    pub(super) async fn prepare(
        &mut self,
        context: &Context,
        budget: &TaskDataBudget,
        cancel: &mut watch::Receiver<bool>,
    ) -> Result<Result<Prepared, TaskFailure>, CallbackError> {
        let deadline = Instant::from_std(context.effect.deadline());
        if super::progress::stopped(cancel, deadline) {
            return Err(CallbackError::TaskTimedOut);
        }
        let permit = tokio::select! {biased;_=cancel.changed()=>return Err(CallbackError::TaskTimedOut),()=sleep_until(deadline)=>return Err(CallbackError::TaskTimedOut),permit=context.shared.pools.cpu.clone().acquire_owned()=>permit.map_err(|_|CallbackError::Unavailable)?,};
        if super::progress::stopped(cancel, deadline) {
            return Err(CallbackError::TaskTimedOut);
        }
        let Ok(charge) = budget.reserve(PREPARATION_BYTES) else {
            return Ok(Err(limit_failure()));
        };
        let effect = context.effect.clone();
        let receipts = self.receipts()?;
        let token: CallbackToken = receipts
            .token
            .task
            .as_ref()
            .ok_or(CallbackError::OwnerLost)?
            .token()
            .expose()
            .parse()?;
        let lease = receipts.lease.clone();
        #[cfg(test)]
        let hook = context.shared.callback_hook.clone();
        receipts.token.cpu = Some(spawn_blocking(move || {
            let _permit = permit;
            let _lease = lease;
            #[cfg(test)]
            if let Some(hook) = &hook {
                hook.run(true);
            }
            let (dispatch, history) =
                effect.prepare_with_source(&token, &mut SystemUuidSource::default())?;
            let bytes = dispatch
                .parameters()
                .original_bytes()
                .len()
                .checked_add(history.payload().original_bytes().len())
                .and_then(|value| value.checked_add(PREPARATION_OVERHEAD))
                .ok_or(AslError::DataLimit)?;
            let charge = charge
                .shrink(bytes)
                .map_err(|_| AslError::InvalidExecution)?;
            #[cfg(test)]
            if let Some(hook) = &hook {
                hook.run(false);
            }
            Ok(Prepared {
                dispatch,
                history,
                charge,
            })
        }));
        let job = receipts
            .token
            .cpu
            .as_mut()
            .ok_or(CallbackError::OwnerLost)?;
        let joined = tokio::select! {biased;_=cancel.changed()=>{if let Some(task)=&receipts.token.task{task.revoke();}job.await},()=sleep_until(deadline)=>{if let Some(task)=&receipts.token.task{task.revoke();}job.await},result=&mut *job=>result,};
        receipts.token.cpu = None;
        let value = joined.map_err(CallbackError::Worker)?;
        Ok(value.map_err(TaskFailure::evaluation))
    }
    pub(super) async fn cleanup(&mut self) -> Result<(), CallbackError> {
        self.cancel();
        let receipts = self.receipts()?;
        let token = receipts.token.cleanup().await;
        let physical = if let Some(call) = receipts.physical.call.as_mut() {
            call.cancel();
            call.settled().await.map_err(|_| CallbackError::OwnerLost)
        } else {
            Ok(())
        };
        if physical.is_ok() {
            receipts.physical = Physical::default();
        }
        token.and(physical)
    }
}

impl TokenWork {
    async fn cleanup(&mut self) -> Result<(), CallbackError> {
        if let Some(job) = self.cpu.as_mut() {
            let result = job.await;
            self.cpu = None;
            let _result = result.map_err(CallbackError::Worker)?;
        }
        if let Some(mint) = self.mint.as_mut() {
            let result = mint.resolve().await;
            if result.as_ref().is_err_and(unknown) {
                return result;
            }
            let mint = self.mint.take().ok_or(CallbackError::OwnerLost)?;
            if result.is_ok() {
                self.task = Some(mint.into_task()?);
            }
        }
        if let Some(task) = self.task.as_mut() {
            task.revoke();
            let terminal = task.resolved().await?;
            if matches!(terminal.as_ref(), CallbackOutcome::Unavailable) {
                return Err(CallbackError::Unavailable);
            }
            self.task = None;
        }
        Ok(())
    }
}
