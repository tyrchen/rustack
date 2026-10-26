//! Bounded registry-owned execution and cleanup supervision.

use std::{collections::HashMap, future::pending, mem::take, sync::Arc};

use chrono::Utc;
use tokio::{
    runtime::Handle,
    sync::{OwnedSemaphorePermit, Semaphore, watch},
    task::{Id, JoinError, JoinSet},
    time::{Instant, timeout_at},
};

use super::history::{HistoryCapacity, HistoryLimits, Journal, TerminalReserve, events};
use crate::{ExecutionDriver, ExecutionFailure, LambdaTaskPort, TaskFailure};

mod actor;
mod error;
mod evaluate;
#[cfg(test)]
mod tests;
mod types;

use actor::{Actor, OwnedCall, RunExit};
pub use error::ExecutionError;
pub use types::{CleanupState, ExecutionHandle, ExecutionView, RuntimeLimits};

#[derive(Debug)]
struct Pools {
    open: Arc<Semaphore>,
    dispatch: Arc<Semaphore>,
    cpu: Arc<Semaphore>,
}
#[derive(Debug)]
struct Shared {
    #[cfg(test)]
    worker_hook: Option<Arc<tests::WorkerHook>>,
    pools: Pools,
    port: Option<Arc<dyn LambdaTaskPort>>,
    history: HistorySettings,
}
#[derive(Debug)]
struct HistorySettings {
    limits: HistoryLimits,
    capacity: Arc<HistoryCapacity>,
}
#[derive(Debug)]
struct Slot {
    stop: watch::Sender<Option<TaskFailure>>,
    view: watch::Sender<Arc<ExecutionView>>,
    reserve: Arc<TerminalReserve>,
    leases: watch::Receiver<Arc<Vec<Arc<DispatchLease>>>>,
    _permit: OwnedSemaphorePermit,
}
#[derive(Debug)]
struct DispatchLease {
    _permit: OwnedSemaphorePermit,
}
#[derive(Debug)]
struct Retained {
    _slot: Slot,
    _calls: Vec<OwnedCall>,
}
#[derive(Debug, Default)]
struct OwnerState {
    accepting: bool,
    jobs: JoinSet<RunExit>,
    slots: HashMap<Id, Slot>,
    quarantine: Vec<Retained>,
}

/// Actor-owned supervisor; the registry must reap jobs and drain before shutdown.
#[derive(Debug)]
#[non_exhaustive]
pub struct ExecutionSupervisor {
    shared: Arc<Shared>,
    state: OwnerState,
}
impl ExecutionSupervisor {
    /// Construct bounded pools/history accounting without spawning or enabling APIs.
    ///
    /// # Errors
    /// Rejects global history outside8MiB..=4GiB or below the per-execution cap.
    ///
    /// ```
    /// let owner=rustack_stepfunctions_core::ExecutionSupervisor::new(
    ///     rustack_stepfunctions_core::RuntimeLimits::default(),
    ///     rustack_stepfunctions_core::HistoryLimits::default(),256*1024*1024,None)?;
    /// # Ok::<(),rustack_stepfunctions_core::ExecutionError>(())
    /// ```
    pub fn new(
        limits: RuntimeLimits,
        history: HistoryLimits,
        total_history: usize,
        port: Option<Arc<dyn LambdaTaskPort>>,
    ) -> Result<Self, ExecutionError> {
        let capacity = HistoryCapacity::new(total_history, history)?;
        Ok(Self {
            shared: Arc::new(Shared {
                #[cfg(test)]
                worker_hook: None,
                pools: Pools {
                    open: Arc::new(Semaphore::new(limits.open)),
                    dispatch: Arc::new(Semaphore::new(limits.dispatch)),
                    cpu: Arc::new(Semaphore::new(limits.cpu)),
                },
                port,
                history: HistorySettings {
                    limits: history,
                    capacity,
                },
            }),
            state: OwnerState {
                accepting: true,
                ..OwnerState::default()
            },
        })
    }
    /// Transfer a driver to the supervisor, independent of all observing callers.
    ///
    /// # Errors
    /// Rejects unavailable runtime/closed owner, missing adapter or admission/history
    /// exhaustion before registration; no invocation occurs on rejection.
    ///
    /// ```no_run
    /// # fn start(owner:&mut rustack_stepfunctions_core::ExecutionSupervisor,driver:rustack_stepfunctions_core::ExecutionDriver) {
    /// let handle=owner.start(driver);
    /// # }
    /// ```
    pub fn start(&mut self, driver: ExecutionDriver) -> Result<ExecutionHandle, ExecutionError> {
        if !self.state.accepting || Handle::try_current().is_err() {
            return Err(ExecutionError::Unavailable);
        }
        self.reap_ready();
        if driver.started || (self.shared.port.is_none() && driver.machine().uses_lambda()) {
            return Err(ExecutionError::Unavailable);
        }
        let permit = self
            .shared
            .pools
            .open
            .clone()
            .try_acquire_owned()
            .map_err(|_| ExecutionError::Capacity)?;
        let reserve = TerminalReserve::new(&self.shared.history.capacity)?;
        let mut journal = Journal::new(
            self.shared.history.limits,
            self.shared.history.capacity.clone(),
            reserve.clone(),
        );
        journal.append(
            events::started(driver.input(), Utc::now()).map_err(ExecutionError::from)?,
            false,
        )?;
        let (view, reader) = watch::channel(Arc::new(ExecutionView {
            history: journal.view.clone(),
            outcome: None,
            stopped: None,
            cleanup: CleanupState::Pending,
            diagnostic: None,
        }));
        let (stop, cancelled) = watch::channel(None);
        let (leases, ledger) = watch::channel(Arc::new(Vec::new()));
        let actor = Actor::new(
            driver,
            journal,
            view.clone(),
            cancelled,
            leases,
            self.shared.clone(),
        );
        let task = self.state.jobs.spawn(actor.run());
        self.state.slots.insert(
            task.id(),
            Slot {
                stop: stop.clone(),
                view,
                reserve,
                leases: ledger,
                _permit: permit,
            },
        );
        Ok(ExecutionHandle { stop, view: reader })
    }
    /// Reap already completed jobs without waiting; retained quarantine stays charged.
    ///
    /// ```
    /// # fn reap(owner:&mut rustack_stepfunctions_core::ExecutionSupervisor) { owner.reap_ready(); }
    /// ```
    pub fn reap_ready(&mut self) {
        while let Some(result) = self.state.jobs.try_join_next_with_id() {
            self.finish(result);
        }
    }
    /// Join one job; callers may cancel the waiter and resume the same supervisor.
    ///
    /// ```no_run
    /// # async fn reap(owner:&mut rustack_stepfunctions_core::ExecutionSupervisor) { owner.reap_next().await; }
    /// ```
    pub async fn reap_next(&mut self) {
        if let Some(result) = self.state.jobs.join_next_with_id().await {
            self.finish(result);
        }
    }
    /// Reject starts and request shutdown once; repeated calls preserve Stop reasons.
    ///
    /// ```
    /// # fn close(owner:&mut rustack_stepfunctions_core::ExecutionSupervisor) { owner.close(); }
    /// ```
    pub fn close(&mut self) {
        self.state.accepting = false;
        for slot in self.state.slots.values() {
            slot.stop.send_if_modified(|stop| {
                if stop.is_some() {
                    false
                } else {
                    *stop = Some(shutdown_reason());
                    true
                }
            });
        }
    }
    /// Close and join through one absolute deadline; expiry retains every job/slot.
    ///
    /// # Errors
    /// Returns Deadline while jobs remain live, or Quarantined after uncertain exit.
    ///
    /// ```no_run
    /// # async fn drain(owner:&mut rustack_stepfunctions_core::ExecutionSupervisor,deadline:tokio::time::Instant) {
    /// let result=owner.drain(deadline).await;
    /// # }
    /// ```
    #[tracing::instrument(name="stepfunctions.drain",skip(self),fields(owned=self.owned_executions()))]
    pub async fn drain(&mut self, deadline: Instant) -> Result<(), ExecutionError> {
        self.close();
        while !self.state.jobs.is_empty() {
            timeout_at(deadline, self.reap_next())
                .await
                .map_err(|_| ExecutionError::Deadline)?;
        }
        if self.state.quarantine.is_empty() {
            Ok(())
        } else {
            Err(ExecutionError::Quarantined)
        }
    }
    /// Read all unsettled jobs/quarantine slots; terminal observers do not release them.
    ///
    /// ```
    /// # fn inspect(owner:&rustack_stepfunctions_core::ExecutionSupervisor) { let active=owner.owned_executions(); }
    /// ```
    #[must_use]
    pub fn owned_executions(&self) -> usize {
        self.state.slots.len() + self.state.quarantine.len()
    }
    /// Read retained global history bytes, including views held by observers.
    ///
    /// ```
    /// # fn inspect(owner:&rustack_stepfunctions_core::ExecutionSupervisor) { let bytes=owner.history_bytes(); }
    /// ```
    #[must_use]
    pub fn history_bytes(&self) -> usize {
        self.shared.history.capacity.used()
    }
    fn finish(&mut self, result: Result<(Id, RunExit), JoinError>) {
        finish_owned(&mut self.state, &self.shared, result);
    }
}
fn finish_owned(state: &mut OwnerState, shared: &Shared, result: Result<(Id, RunExit), JoinError>) {
    match result {
        Ok((id, exit)) => {
            if let Some(slot) = state.slots.remove(&id) {
                if exit.uncertain {
                    publish_owner_failure(&slot, shared, None);
                    state.quarantine.push(Retained {
                        _slot: slot,
                        _calls: exit.quarantined,
                    });
                } else {
                    drop(slot);
                }
            }
        }
        Err(error) => {
            if let Some(slot) = state.slots.remove(&error.id()) {
                tracing::error!(
                    dispatches = slot.leases.borrow().len(),
                    "Workflow execution actor failed; retaining its capacity"
                );
                publish_owner_failure(&slot, shared, Some(Arc::new(ExecutionError::Worker(error))));
                state.quarantine.push(Retained {
                    _slot: slot,
                    _calls: Vec::new(),
                });
            }
        }
    }
}

fn shutdown_reason() -> TaskFailure {
    TaskFailure(ExecutionFailure::named(
        "Rustack.Shutdown",
        "Workflow supervisor is shutting down",
    ))
}
fn publish_owner_failure(slot: &Slot, shared: &Shared, diagnostic: Option<Arc<ExecutionError>>) {
    let old = slot.view.borrow().clone();
    if old.outcome.is_some() {
        let mut view = old.as_ref().clone();
        view.cleanup = CleanupState::Quarantined;
        if diagnostic.is_some() {
            view.diagnostic = diagnostic;
        }
        slot.view.send_replace(Arc::new(view));
        return;
    }
    let outcome = types::internal_outcome();
    let wall = Utc::now();
    let mut journal = Journal::resume(
        old.history.clone(),
        shared.history.limits,
        shared.history.capacity.clone(),
        slot.reserve.clone(),
    );
    if let Ok(event) = events::terminal(&outcome, wall) {
        if journal.append(event, true).is_ok() {
            slot.view.send_replace(Arc::new(ExecutionView {
                history: journal.view,
                outcome: Some(outcome),
                stopped: Some(wall),
                cleanup: CleanupState::Quarantined,
                diagnostic: diagnostic.or_else(|| old.diagnostic.clone()),
            }));
        }
    }
}

#[derive(Debug)]
struct FallbackOwner {
    state: OwnerState,
    shared: Arc<Shared>,
}
impl FallbackOwner {
    async fn drain(mut self) {
        while let Some(result) = self.state.jobs.join_next_with_id().await {
            finish_owned(&mut self.state, &self.shared, result);
        }
        if !self.state.quarantine.is_empty() {
            tracing::error!(
                executions = self.state.quarantine.len(),
                "Dropped workflow supervisor retains quarantined resources"
            );
            pending::<()>().await;
        }
    }
}
impl Drop for FallbackOwner {
    fn drop(&mut self) {
        if self.state.jobs.is_empty()
            && self.state.slots.is_empty()
            && self.state.quarantine.is_empty()
        {
            return;
        }
        // Covers cancellation before first poll, panic, and runtime shutdown.
        // The unresolved graph must outlive the executor that stopped polling it.
        let state = take(&mut self.state);
        tracing::error!(
            executions = state.slots.len() + state.quarantine.len(),
            "Workflow fallback lost its runtime; ownership quarantined for process lifetime"
        );
        let _retained = Box::leak(Box::new((self.shared.clone(), state)));
    }
}
impl Drop for ExecutionSupervisor {
    fn drop(&mut self) {
        if self.state.jobs.is_empty() && self.state.quarantine.is_empty() {
            return;
        }
        self.close();
        let retained = FallbackOwner {
            state: take(&mut self.state),
            shared: self.shared.clone(),
        };
        if let Ok(runtime) = Handle::try_current() {
            // Intentional detached fallback owner. Its Drop guard preserves the
            // bounded graph even if this future never polls or is cancelled.
            let fallback = runtime.spawn(retained.drain());
            drop(fallback);
        } else {
            drop(retained);
        }
    }
}
