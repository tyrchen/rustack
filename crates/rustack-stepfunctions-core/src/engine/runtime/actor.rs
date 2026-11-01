//! One sequential execution owner, including retired-call cleanup lifetime.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use rustack_stepfunctions_model::types::HistoryEvent;
use serde_json::Value;
use tokio::{
    sync::{OwnedSemaphorePermit, watch},
    task::{JoinError, JoinSet, yield_now},
    time::{Instant, sleep_until},
};

use super::{
    super::{
        ClockSample, DriverStep, Phase,
        history::{
            Journal,
            events::{self, TaskFailureKind},
        },
    },
    CleanupState, DispatchLease, ExecutionError, ExecutionView, Shared,
    evaluate::{Evaluated, Operation},
    shutdown_reason,
};
use crate::{
    AslError, ExecutionDriver, ExecutionFailure, ExecutionOutcome, LambdaTaskCommand, Payload,
    StateView, TaskCall, TaskDispatch, TaskEvent, TaskFailure, TaskSettlement, TaskTimeout,
};

#[derive(Debug)]
pub(super) struct OwnedCall {
    pub(super) call: TaskCall,
    pub(super) lease: Arc<DispatchLease>,
}
#[derive(Debug)]
struct CurrentCall {
    owned: OwnedCall,
    started: bool,
}
#[derive(Debug)]
struct Resources {
    active: Option<CurrentCall>,
    retired: JoinSet<Result<Arc<DispatchLease>, OwnedCall>>,
    quarantined: Vec<OwnedCall>,
    leases: Vec<Arc<DispatchLease>>,
    workers: JoinSet<Evaluated>,
}
#[derive(Debug)]
struct Context {
    journal: Journal,
    view: watch::Sender<Arc<ExecutionView>>,
    stop: watch::Receiver<Option<TaskFailure>>,
    ledger: watch::Sender<Arc<Vec<Arc<DispatchLease>>>>,
}
#[derive(Debug)]
pub(super) struct Actor {
    pub(super) driver: ExecutionDriver,
    context: Context,
    resources: Resources,
    shared: Arc<Shared>,
    pending: Option<Operation>,
}
#[derive(Debug)]
pub(super) struct RunExit {
    pub(super) quarantined: Vec<OwnedCall>,
    pub(super) uncertain: bool,
}
impl Actor {
    pub(super) fn new(
        driver: ExecutionDriver,
        journal: Journal,
        view: watch::Sender<Arc<ExecutionView>>,
        stop: watch::Receiver<Option<TaskFailure>>,
        ledger: watch::Sender<Arc<Vec<Arc<DispatchLease>>>>,
        shared: Arc<Shared>,
    ) -> Self {
        Self {
            driver,
            context: Context {
                journal,
                view,
                stop,
                ledger,
            },
            resources: Resources {
                active: None,
                retired: JoinSet::new(),
                quarantined: Vec::new(),
                leases: Vec::new(),
                workers: JoinSet::new(),
            },
            shared,
            pending: None,
        }
    }
    #[tracing::instrument(name = "stepfunctions.execution", skip_all)]
    pub(super) async fn run(mut self) -> RunExit {
        let mut ticks = 0_u8;
        loop {
            self.reap_cleanups();
            if self.driver.outcome().is_some() {
                break;
            }
            let operation = if let Some(pending) = self.pending.take() {
                Some(pending)
            } else {
                self.next_operation().await
            };
            if let Some(operation) = operation {
                match self.evaluate(operation).await {
                    Ok(Some(DriverStep::Dispatch(dispatch))) => self.dispatch(dispatch).await,
                    Ok(_) => {}
                    Err(error) => self.fail(error),
                }
            }
            ticks = ticks.saturating_add(1);
            if ticks >= 64 {
                yield_now().await;
                ticks = 0;
            }
        }
        self.retire_active();
        let publication_failed = self.publish_terminal().is_err();
        // CPU futures remain supervised even when Stop/deadline won earlier.
        while let Some(result) = self.resources.workers.join_next().await {
            if result.is_err() {
                tracing::error!("Workflow evaluation worker failed during drain");
            }
        }
        while let Some(result) = self.resources.retired.join_next().await {
            self.cleanup_result(result);
        }
        let uncertain = publication_failed || !self.resources.leases.is_empty();
        let cleanup = if uncertain {
            CleanupState::Quarantined
        } else {
            CleanupState::Clean
        };
        self.publish_cleanup(cleanup);
        RunExit {
            quarantined: self.resources.quarantined,
            uncertain,
        }
    }
    fn sample() -> ClockSample {
        ClockSample::new(Instant::now().into_std(), Utc::now())
    }
    fn stopped(&self) -> Option<TaskFailure> {
        self.context.stop.borrow().clone()
    }
    fn apply_control(&mut self) -> bool {
        if self.driver.outcome().is_some() {
            return true;
        }
        let now = Self::sample();
        let reason = self.stopped().or_else(|| {
            self.context
                .stop
                .has_changed()
                .is_err()
                .then(shutdown_reason)
        });
        if let Some(reason) = reason {
            let _ = self.driver.stop(reason, now);
            self.retire_active();
            return true;
        }
        if now.monotonic >= self.driver.deadline {
            let _ = self.driver.guard(now);
            self.retire_active();
            return true;
        }
        false
    }
    async fn next_operation(&mut self) -> Option<Operation> {
        loop {
            if self.apply_control() {
                return None;
            }
            match &self.driver.phase {
                Phase::Ready { .. } => return Some(Operation::Advance),
                Phase::Terminal(_) => return None,
                Phase::Joining { .. } => {
                    self.fail(ExecutionError::Unavailable);
                    return None;
                }
                Phase::Waiting { until, .. } => {
                    let until = Instant::from_std((*until).min(self.driver.deadline));
                    tokio::select! {
                        biased;
                        _=self.context.stop.changed()=>{},
                        ()=sleep_until(until)=>return Some(Operation::Advance),
                        result=self.resources.retired.join_next(),if !self.resources.retired.is_empty()=>if let Some(result)=result {self.cleanup_result(result);},
                    }
                }
                Phase::Calling { attempt, deadline } => {
                    let attempt = *attempt;
                    let deadline = *deadline;
                    if Instant::now().into_std() >= deadline {
                        if Instant::now().into_std() < self.driver.deadline {
                            let failure = TaskFailure(ExecutionFailure::fields(
                                Some("States.Timeout".into()),
                                None,
                            ));
                            if !self.record(events::task_failure(
                                self.driver.machine(),
                                self.driver.state,
                                &failure,
                                TaskFailureKind::TimedOut,
                                Utc::now(),
                            )) {
                                return None;
                            }
                        }
                        self.retire_active();
                        return Some(Operation::Advance);
                    }
                    let Some(active) = &mut self.resources.active else {
                        self.fail(ExecutionError::Unavailable);
                        return None;
                    };
                    tokio::select! {
                                           biased;
                                           _=self.context.stop.changed()=>{},
                                           ()=sleep_until(Instant::from_std(deadline))=>{},
                                           result=self.resources.retired.join_next(),if !self.resources.retired.is_empty()=>if let Some(result)=result {self.cleanup_result(result);},
                                           event=active.owned.call.next()=>{
                                               if self.apply_control() {return None;}
                                               let observed=Self::sample();
                                               if observed.monotonic>=deadline {continue;}
                                               match event {
                                                   Ok(TaskEvent::Started)=>{if let Some(active)=&mut self.resources.active {active.started=true;}
                    if !self.record(events::task_started(self.driver.machine(),self.driver.state,observed.wall)) {return None;}},
                                                   Ok(TaskEvent::Submitted(_))=>{self.fail(ExecutionError::Unavailable);},
                                                   Ok(TaskEvent::Completed(result))=>{
                                                       if !self.record_result(&result,observed.wall) {return None;}
                                                       self.retire_active();return Some(Operation::Complete {attempt,result,observed});
                                                   }
                                                   Err(_)=>{
                                                       let result=Err(TaskFailure(ExecutionFailure::named("Lambda.ServiceException","Lambda task owner lost")));
                                                       if !self.record_result(&result,observed.wall) {return None;}
                                                       self.retire_active();return Some(Operation::Complete {attempt,result,observed});
                                                   }
                                               }
                                           }
                                       }
                }
            }
        }
    }
    fn record_result(
        &mut self,
        result: &Result<Payload, TaskFailure>,
        wall: DateTime<Utc>,
    ) -> bool {
        let started = self
            .resources
            .active
            .as_ref()
            .is_some_and(|active| active.started);
        if result.is_ok() && !started {
            self.fail(ExecutionError::Unavailable);
            return false;
        }
        let kind = if started {
            TaskFailureKind::Failed
        } else {
            TaskFailureKind::Admission
        };
        let event = match result {
            Ok(output) => {
                events::task_success(self.driver.machine(), self.driver.state, output, wall)
            }
            Err(failure) => events::task_failure(
                self.driver.machine(),
                self.driver.state,
                failure,
                kind,
                wall,
            ),
        };
        self.record(event)
    }
    pub(super) fn record(&mut self, event: Result<HistoryEvent, AslError>) -> bool {
        match event
            .map_err(ExecutionError::from)
            .and_then(|event| self.context.journal.append(event, false))
        {
            Ok(()) => {
                self.publish_running();
                true
            }
            Err(error) => {
                self.fail(error);
                false
            }
        }
    }
    pub(super) fn fail(&mut self, error: ExecutionError) {
        let outcome = if matches!(error, ExecutionError::Capacity) {
            ExecutionOutcome::Failed(ExecutionFailure::named(
                "States.HistoryLimitExceeded",
                "Execution history limit exceeded",
            ))
        } else {
            ExecutionOutcome::Failed(ExecutionFailure::named(
                "Rustack.InternalError",
                "Execution owner failed",
            ))
        };
        if self.driver.outcome().is_none() {
            self.driver.phase = Phase::Terminal(Arc::new(outcome));
        }
        self.record_diagnostic(Arc::new(error));
        self.retire_active();
    }
    fn record_diagnostic(&self, error: Arc<ExecutionError>) {
        let mut view = self.context.view.borrow().as_ref().clone();
        view.diagnostic = Some(error);
        self.context.view.send_replace(Arc::new(view));
    }
    pub(super) async fn dispatch(&mut self, dispatch: TaskDispatch) {
        let timeout = match self.driver.machine.state(self.driver.state) {
            Ok(StateView::Task { timeout, .. }) => match timeout {
                TaskTimeout::Omitted => None,
                TaskTimeout::Seconds(v) => Some(i64::from(v.get())),
                TaskTimeout::Path(path) => path
                    .lookup(&self.driver.input, &self.driver.context)
                    .and_then(Value::as_i64),
            },
            _ => None,
        };
        if !self.record(events::scheduled(
            self.driver.machine(),
            dispatch.state(),
            dispatch.parameters(),
            timeout,
            Utc::now(),
        )) {
            return;
        }
        let acquired = loop {
            if self.apply_control() {
                return;
            }
            if Instant::now().into_std() >= dispatch.deadline() {
                // No call was issued; the driver still owns this attempt's timer.
                return;
            }
            tokio::select! {
                biased;
                _=self.context.stop.changed()=>{},
                ()=sleep_until(Instant::from_std(dispatch.deadline()))=>return,
                result=self.resources.retired.join_next(),if !self.resources.retired.is_empty()=>if let Some(result)=result {self.cleanup_result(result);},
                permit=self.shared.pools.dispatch.clone().acquire_owned()=>break permit,
            }
        };
        let Ok(permit) = acquired else {
            self.fail(ExecutionError::Unavailable);
            return;
        };
        if self.apply_control() || Instant::now().into_std() >= dispatch.deadline() {
            return;
        }
        let lease = Arc::new(DispatchLease {
            _permit: Some(permit),
            _frame: None,
            _data: None,
        });
        self.resources.leases.push(lease.clone());
        self.sync_ledger();
        let command = LambdaTaskCommand::new(self.driver.machine.clone(), dispatch.clone());
        let result = self.shared.port.as_ref().map_or_else(
            || {
                Err(TaskFailure(ExecutionFailure::named(
                    "Lambda.ServiceException",
                    "Lambda adapter unavailable",
                )))
            },
            |port| port.start(command),
        );
        match result {
            Ok(call) => {
                self.resources.active = Some(CurrentCall {
                    owned: OwnedCall { call, lease },
                    started: false,
                });
            }
            Err(failure) => {
                self.release_lease(&lease);
                let observed = Self::sample();
                if self.apply_control() || observed.monotonic >= dispatch.deadline() {
                    return;
                }
                if !self.record(events::task_failure(
                    self.driver.machine(),
                    dispatch.state(),
                    &failure,
                    TaskFailureKind::Admission,
                    observed.wall,
                )) {
                    return;
                }
                // Drive admission failures through the same checkpoint/evaluation path.
                self.pending = Some(Operation::Complete {
                    attempt: dispatch.attempt(),
                    result: Err(failure),
                    observed,
                });
            }
        }
    }
    fn retire_active(&mut self) {
        if let Some(active) = self.resources.active.take() {
            if active.owned.call.settlement() == TaskSettlement::Quarantined {
                self.publish_cleanup(CleanupState::Quarantined);
            }
            active.owned.call.cancel();
            let owned = active.owned;
            self.resources.retired.spawn(async move {
                let mut owned = owned;
                if owned.call.settled().await.is_ok() {
                    Ok(owned.lease.clone())
                } else {
                    Err(owned)
                }
            });
        }
    }
    fn sync_ledger(&self) {
        self.context
            .ledger
            .send_replace(Arc::new(self.resources.leases.clone()));
    }
    fn release_lease(&mut self, lease: &Arc<DispatchLease>) {
        self.resources
            .leases
            .retain(|current| !Arc::ptr_eq(current, lease));
        self.sync_ledger();
    }
    fn cleanup_result(&mut self, result: Result<Result<Arc<DispatchLease>, OwnedCall>, JoinError>) {
        match result {
            Ok(Ok(lease)) => self.release_lease(&lease),
            Ok(Err(owned)) => {
                self.resources.quarantined.push(owned);
                self.publish_cleanup(CleanupState::Quarantined);
            }
            Err(error) => {
                self.record_diagnostic(Arc::new(ExecutionError::Worker(error)));
                tracing::error!("Workflow cleanup worker failed; dispatch capacity retained");
                self.publish_cleanup(CleanupState::Quarantined);
            }
        }
    }
    fn reap_cleanups(&mut self) {
        while let Some(result) = self.resources.retired.try_join_next() {
            self.cleanup_result(result);
        }
    }
    fn publish_running(&self) {
        let old = self.context.view.borrow().clone();
        self.context.view.send_replace(Arc::new(ExecutionView {
            history: self.context.journal.view.clone(),
            outcome: old.outcome.clone(),
            stopped: old.stopped,
            cleanup: old.cleanup,
            diagnostic: old.diagnostic.clone(),
        }));
    }
    fn publish_cleanup(&self, cleanup: CleanupState) {
        let mut view = self.context.view.borrow().as_ref().clone();
        view.cleanup = cleanup;
        self.context.view.send_replace(Arc::new(view));
    }
    fn publish_terminal(&mut self) -> Result<(), ExecutionError> {
        let Some(outcome) = self.driver.outcome().cloned() else {
            return Err(ExecutionError::Unavailable);
        };
        let wall = Utc::now();
        let first = events::terminal(&outcome, wall)
            .map_err(ExecutionError::from)
            .and_then(|event| self.context.journal.append(event, true));
        let outcome = if let Err(error) = first {
            let name = if matches!(error, ExecutionError::Capacity) {
                "States.HistoryLimitExceeded"
            } else {
                "Rustack.InternalError"
            };
            let outcome = Arc::new(ExecutionOutcome::Failed(ExecutionFailure::named(
                name,
                "Terminal history could not be retained",
            )));
            let event = events::terminal(&outcome, wall).map_err(ExecutionError::from)?;
            self.context.journal.append(event, true)?;
            self.driver.phase = Phase::Terminal(outcome.clone());
            outcome
        } else {
            outcome
        };
        let cleanup = if self.context.view.borrow().cleanup == CleanupState::Quarantined {
            CleanupState::Quarantined
        } else {
            CleanupState::Pending
        };
        let diagnostic = self
            .driver
            .diagnostic
            .as_ref()
            .map(|e| Arc::new(ExecutionError::Evaluation(e.clone())))
            .or_else(|| self.context.view.borrow().diagnostic.clone());
        self.context.view.send_replace(Arc::new(ExecutionView {
            history: self.context.journal.view.clone(),
            outcome: Some(outcome),
            stopped: Some(wall),
            cleanup,
            diagnostic,
        }));
        Ok(())
    }
    pub(super) async fn evaluation_permit(&mut self) -> Option<OwnedSemaphorePermit> {
        loop {
            if self.apply_control() {
                return None;
            }
            tokio::select! {
                biased;
                _=self.context.stop.changed()=>{},
                ()=sleep_until(Instant::from_std(self.driver.deadline))=>{},
                permit=self.shared.pools.cpu.clone().acquire_owned()=>return permit.ok(),
            }
        }
    }
    #[cfg(test)]
    pub(super) fn worker_hook(&self) -> Option<Arc<super::tests::WorkerHook>> {
        self.shared.worker_hook.clone()
    }
    pub(super) fn start_worker(&mut self, worker: impl FnOnce() -> Evaluated + Send + 'static) {
        self.resources.workers.spawn_blocking(worker);
    }
    pub(super) async fn evaluated(&mut self) -> Result<Option<Evaluated>, ExecutionError> {
        loop {
            if self.apply_control() {
                return Ok(None);
            }
            tokio::select! {
                biased;
                _=self.context.stop.changed()=>{},
                ()=sleep_until(Instant::from_std(self.driver.deadline))=>{},
                result=self.resources.workers.join_next()=>return match result {Some(Ok(result))=>Ok(Some(result)),Some(Err(error))=>Err(ExecutionError::Worker(error)),None=>Err(ExecutionError::Unavailable)},
            }
        }
    }
    pub(super) fn commit(&mut self, evaluated: Evaluated) -> Option<DriverStep> {
        if self.apply_control() {
            return None;
        }
        let before_state = self.driver.state;
        if matches!(self.driver.phase, Phase::Ready { entry: true })
            && !self.record(events::entered(
                self.driver.machine(),
                before_state,
                &self.driver.input,
                evaluated.sample.wall,
            ))
        {
            return None;
        }
        let exit = match &evaluated.proposed.phase {
            Phase::Ready { entry: true } => Some(evaluated.proposed.input.clone()),
            Phase::Terminal(outcome) => match outcome.as_ref() {
                ExecutionOutcome::Succeeded(output) => Some(output.clone()),
                _ => None,
            },
            _ => None,
        };
        if let Some(output) = exit {
            if !self.record(events::exited(
                self.driver.machine(),
                before_state,
                &output,
                Utc::now(),
            )) {
                return None;
            }
        }
        self.driver = evaluated.proposed;
        Some(evaluated.step)
    }
}
