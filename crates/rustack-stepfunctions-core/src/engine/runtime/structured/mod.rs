//! Asynchronous resource ownership around the unique structured frame scheduler.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use chrono::{DateTime, Utc};
use serde_json::Value;
use tokio::{
    sync::{OwnedSemaphorePermit, mpsc, watch},
    task::{Id, JoinError, JoinSet, yield_now},
    time::{Instant, sleep_until},
};

use super::{
    CleanupState, DispatchLease, ExecutionError, ExecutionView, Shared,
    actor::{OwnedCall, RunExit},
    shutdown_reason,
};
use crate::{
    ChildPattern, ClockSample, CompiledMachine, DriverStep, ExecutionFailure, ExecutionOutcome,
    FrameCallId, FrameError, FrameLease, FrameScheduler, FrameTransition, FrameUpdate, Integration,
    LambdaTaskCommand, Payload, StateView, SystemUuidSource, TaskCall, TaskDispatch, TaskFailure,
    TaskTimeout,
    engine::{
        Phase,
        history::{
            Journal,
            events::{self, TaskFailureKind},
        },
        types::FailureOrigin,
    },
};

mod child;
mod history;
mod resource;
use history::CausalHistory;

#[derive(Debug)]
enum Message {
    Started(FrameCallId),
    Submitted(FrameCallId, Payload),
    Completed(FrameCallId, Result<Payload, TaskFailure>),
    Quarantined(FrameCallId),
    Fatal(ExecutionError),
}
#[derive(Debug)]
struct WatchExit {
    id: FrameCallId,
    owned: OwnedCall,
    clean: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Progress {
    Pending,
    Started,
    Submitted,
}
#[derive(Debug)]
struct Call {
    cancel: watch::Sender<bool>,
    lease: Arc<DispatchLease>,
    machine: Arc<CompiledMachine>,
    dispatch: TaskDispatch,
    progress: Progress,
    result_seen: bool,
    cancelled: bool,
    settled: Option<bool>,
}
#[derive(Debug)]
struct Pending {
    expired: bool,
    command: LambdaTaskCommand,
    frame: Arc<FrameLease>,
}
#[derive(Debug)]
enum Operation {
    Advance,
    Complete(FrameCallId, Result<Payload, TaskFailure>, ClockSample),
}
#[derive(Debug)]
struct Evaluated {
    graph: FrameScheduler,
    update: Result<Option<FrameUpdate>, FrameError>,
    sample: ClockSample,
}
#[derive(Debug)]
pub(super) struct Actor {
    graph: Option<FrameScheduler>,
    deadline: Instant,
    history: CausalHistory,
    view: watch::Sender<Arc<ExecutionView>>,
    stop: watch::Receiver<Option<TaskFailure>>,
    ledger: watch::Sender<Arc<Vec<Arc<DispatchLease>>>>,
    shared: Arc<Shared>,
    calls: BTreeMap<FrameCallId, Call>,
    pending: BTreeMap<FrameCallId, Pending>,
    acks: BTreeSet<FrameCallId>,
    leases: Vec<Arc<DispatchLease>>,
    quarantine: Vec<OwnedCall>,
    watch_ids: BTreeMap<Id, FrameCallId>,
    workers: JoinSet<Evaluated>,
    watchers: JoinSet<WatchExit>,
    child_jobs: JoinSet<()>,
    children: Option<child::Binding>,
    sender: mpsc::Sender<Message>,
    receiver: mpsc::Receiver<Message>,
    forced: Option<Arc<ExecutionOutcome>>,
    committed: Option<Arc<ExecutionOutcome>>,
    published: bool,
    uncertain: bool,
    park: Option<(Instant, bool)>,
    reserved: Option<OwnedSemaphorePermit>,
}
impl Actor {
    pub(super) fn new(
        graph: FrameScheduler,
        deadline: Instant,
        journal: Journal,
        view: watch::Sender<Arc<ExecutionView>>,
        stop: watch::Receiver<Option<TaskFailure>>,
        ledger: watch::Sender<Arc<Vec<Arc<DispatchLease>>>>,
        shared: Arc<Shared>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel(64);
        Self {
            graph: Some(graph),
            deadline,
            history: CausalHistory::new(journal),
            view,
            stop,
            ledger,
            shared,
            calls: BTreeMap::new(),
            pending: BTreeMap::new(),
            acks: BTreeSet::new(),
            leases: Vec::new(),
            quarantine: Vec::new(),
            watch_ids: BTreeMap::new(),
            workers: JoinSet::new(),
            watchers: JoinSet::new(),
            child_jobs: JoinSet::new(),
            children: None,
            sender,
            receiver,
            forced: None,
            committed: None,
            published: false,
            uncertain: false,
            park: None,
            reserved: None,
        }
    }
    fn sample() -> ClockSample {
        ClockSample::new(Instant::now().into_std(), Utc::now())
    }
    fn control(&mut self) {
        if self.published || self.forced.is_some() || self.committed.is_some() {
            return;
        }
        let outcome = if Instant::now() >= self.deadline {
            Some(ExecutionOutcome::TimedOut)
        } else {
            self.stop
                .borrow()
                .clone()
                .or_else(|| self.stop.has_changed().is_err().then(shutdown_reason))
                .map(ExecutionOutcome::Aborted)
        };
        if let Some(outcome) = outcome {
            self.force(Arc::new(outcome));
        }
    }
    fn force(&mut self, mut outcome: Arc<ExecutionOutcome>) {
        if self.published {
            return;
        }
        self.forced = Some(outcome.clone());
        self.reserved = None;
        self.pending.clear();
        for call in self.calls.values_mut() {
            call.cancel.send_replace(true);
            call.cancelled = true;
        }
        if let Some(graph) = &mut self.graph {
            graph.supersede_uncommitted(outcome.clone());
        }
        if let Err(error) = self.history.abort(Utc::now()) {
            let name = if matches!(error, ExecutionError::Capacity) {
                "States.HistoryLimitExceeded"
            } else {
                "Rustack.InternalError"
            };
            let mut view = self.view.borrow().as_ref().clone();
            view.diagnostic = Some(Arc::new(error));
            self.view.send_replace(Arc::new(view));
            outcome = Arc::new(ExecutionOutcome::Failed(ExecutionFailure::safety(
                name,
                "Abort history could not be retained",
            )));
            self.forced = Some(outcome.clone());
            if let Some(graph) = &mut self.graph {
                graph.supersede_uncommitted(outcome.clone());
            }
        }
        self.publish_terminal(outcome);
        self.park = None;
    }
    fn fail(&mut self, error: ExecutionError) {
        let mut view = self.view.borrow().as_ref().clone();
        view.diagnostic = Some(Arc::new(error));
        self.view.send_replace(Arc::new(view));
        let name = if matches!(
            self.view.borrow().diagnostic.as_deref(),
            Some(ExecutionError::Capacity)
        ) {
            "States.HistoryLimitExceeded"
        } else {
            "Rustack.InternalError"
        };
        self.force(Arc::new(ExecutionOutcome::Failed(
            ExecutionFailure::safety(name, "Structured execution owner failed"),
        )));
    }
    fn publish_running(&self) {
        let mut view = self.view.borrow().as_ref().clone();
        view.history = self.history.journal.view.clone();
        self.view.send_replace(Arc::new(view));
    }
    fn publish_terminal(&mut self, mut outcome: Arc<ExecutionOutcome>) {
        if self.published {
            return;
        }
        let wall = Utc::now();
        if self.history.terminal(&outcome, wall).is_err() {
            outcome = Arc::new(ExecutionOutcome::Failed(ExecutionFailure::safety(
                "States.HistoryLimitExceeded",
                "Terminal history could not be retained",
            )));
            if self.history.terminal(&outcome, wall).is_err() {
                self.uncertain = true;
                return;
            }
        }
        let mut view = self.view.borrow().as_ref().clone();
        view.history = self.history.journal.view.clone();
        view.outcome = Some(outcome);
        view.stopped = Some(wall);
        self.view.send_replace(Arc::new(view));
        self.published = true;
    }
    fn cleanup(&self, state: CleanupState) {
        let mut view = self.view.borrow().as_ref().clone();
        view.cleanup = state;
        self.view.send_replace(Arc::new(view));
    }
    fn sync_ledger(&self) {
        self.ledger.send_replace(Arc::new(self.leases.clone()));
    }
    fn release(&mut self, lease: &Arc<DispatchLease>) {
        self.leases.retain(|item| !Arc::ptr_eq(item, lease));
        self.sync_ledger();
    }
    fn reconcile(&mut self) {
        let Some(graph) = &mut self.graph else {
            if self.workers.is_empty() && self.forced.is_some() {
                let ids = self.acks.iter().copied().collect::<Vec<_>>();
                for id in ids {
                    self.acks.remove(&id);
                    if let Some(call) = self.calls.remove(&id) {
                        self.release(&call.lease);
                    }
                }
            }
            return;
        };
        let ids = self.acks.iter().copied().collect::<Vec<_>>();
        let mut released = Vec::new();
        for id in ids {
            match graph.retired(id) {
                Ok(_) => {
                    self.acks.remove(&id);
                    if let Some(call) = self.calls.remove(&id) {
                        released.push(call.lease);
                    }
                }
                Err(FrameError::Event) => {}
                Err(error) => {
                    self.fail(ExecutionError::Frame(error));
                    return;
                }
            }
        }
        for lease in released {
            self.release(&lease);
        }
    }
    fn joined(&mut self, result: Result<(Id, WatchExit), JoinError>) {
        match result {
            Ok((task, exit)) => {
                self.watch_ids.remove(&task);
                if let Some(call) = self.calls.get_mut(&exit.id) {
                    call.settled = Some(exit.clean);
                    if exit.clean && (call.result_seen || call.cancelled || self.published) {
                        self.acks.insert(exit.id);
                    }
                }
                if !exit.clean {
                    self.quarantine.push(exit.owned);
                    self.cleanup(CleanupState::Quarantined);
                    self.uncertain = true;
                }
            }
            Err(error) => {
                if let Some(id) = self.watch_ids.remove(&error.id()) {
                    if let Some(call) = self.calls.get_mut(&id) {
                        call.settled = Some(false);
                    }
                }
                self.uncertain = true;
                self.fail(ExecutionError::Worker(error));
            }
        }
        self.reconcile();
    }
    async fn worker(&mut self, operation: Operation) -> bool {
        self.control();
        if self.forced.is_some() {
            return false;
        }
        let permit = loop {
            let task_deadline = self.next_deadline();
            tokio::select! {biased;_=self.stop.changed()=>{self.control();if self.forced.is_some(){return false;}},()=sleep_until(self.deadline)=>{self.control();return false;},()=sleep_until(task_deadline)=>{self.expired();},permit=self.shared.pools.cpu.clone().acquire_owned()=>break permit}
        };
        let Ok(permit) = permit else {
            self.fail(ExecutionError::Unavailable);
            return false;
        };
        self.control();
        if self.forced.is_some() {
            return false;
        }
        let Some(mut graph) = self.graph.take() else {
            self.fail(ExecutionError::Unavailable);
            return false;
        };
        #[cfg(test)]
        let hook = self.shared.worker_hook.clone();
        self.park = None;
        self.workers.spawn_blocking(move || {
            let _permit = permit;
            #[cfg(test)]
            if let Some(hook) = &hook {
                hook.run(true);
            }
            let sample = Self::sample();
            let mut source = SystemUuidSource::default();
            let update = match operation {
                Operation::Advance => Ok(Some(graph.advance(sample, &mut source))),
                Operation::Complete(call, result, observed) => graph
                    .complete(call, result, observed, &mut source)
                    .map(|result| result.map(|t| FrameUpdate::Transition(Box::new(t)))),
            };
            #[cfg(test)]
            if let Some(hook) = &hook {
                hook.run(false);
            }
            Evaluated {
                graph,
                update,
                sample,
            }
        });
        true
    }
    fn evaluated(&mut self, result: Result<Evaluated, JoinError>) {
        match result {
            Ok(mut result) => {
                self.control();
                if let Some(outcome) = &self.forced {
                    result.graph.supersede_uncommitted(outcome.clone());
                    result.update = Ok(None);
                }
                self.graph = Some(result.graph);
                self.reconcile();
                match result.update {
                    Ok(Some(update)) => {
                        if let Err(error) = self.update(update, result.sample.wall) {
                            self.fail(error);
                        }
                    }
                    Ok(None) => {}
                    Err(error) => self.fail(ExecutionError::Frame(error)),
                }
                if self.forced.is_none() && self.committed.is_none() {
                    self.committed = self
                        .graph
                        .as_ref()
                        .and_then(|graph| graph.outcome().cloned());
                    if self.committed.is_some() {
                        self.park = None;
                    }
                }
            }
            Err(error) => {
                self.fail(ExecutionError::Worker(error));
                self.park = Some((self.deadline, false));
            }
        }
    }
    fn schedule(&mut self, t: &FrameTransition, wall: DateTime<Utc>) -> Result<(), ExecutionError> {
        if matches!(t.step(), DriverStep::Callback(_)) {
            return Err(ExecutionError::Unavailable);
        }
        if let DriverStep::Dispatch(dispatch) = t.step() {
            let id = t.call().ok_or(ExecutionError::Invalid)?;
            let graph = self.graph.as_ref().ok_or(ExecutionError::Invalid)?;
            let driver = graph.driver(id.frame()).ok_or(ExecutionError::Invalid)?;
            let timeout = match driver.machine.state(driver.state)? {
                StateView::Task { timeout, .. } => match timeout {
                    TaskTimeout::Omitted => None,
                    TaskTimeout::Seconds(value) => Some(i64::from(value.get())),
                    TaskTimeout::Path(path) => path
                        .lookup(&driver.input, &driver.context)
                        .and_then(Value::as_i64),
                },
                _ => None,
            };
            self.history.event(
                id.frame(),
                events::scheduled(
                    t.machine(),
                    dispatch.state(),
                    dispatch.parameters(),
                    timeout,
                    wall,
                ),
            )?;
            let frame = graph.lease(id.frame()).ok_or(ExecutionError::Invalid)?;
            self.pending.insert(
                id,
                Pending {
                    expired: false,
                    command: LambdaTaskCommand::new(t.machine().clone(), dispatch.clone()),
                    frame,
                },
            );
        }

        Ok(())
    }
    fn update(&mut self, update: FrameUpdate, wall: DateTime<Utc>) -> Result<(), ExecutionError> {
        match update {
            FrameUpdate::Transition(t) => {
                if self.published {
                    return Ok(());
                }
                self.history.transition(&t, wall)?;

                self.schedule(&t, wall)?;
                self.park = None;
                self.publish_running();
            }
            FrameUpdate::ChildStarted { child, parent } => {
                if !self.published {
                    self.history.child_started(
                        child,
                        parent,
                        self.graph.as_ref().ok_or(ExecutionError::Invalid)?,
                        wall,
                    )?;
                    self.publish_running();
                }
                self.park = None;
            }
            FrameUpdate::ChildFinished {
                child,
                parent,
                outcome,
            } => {
                if !self.published {
                    self.history.child_finished(child, parent, &outcome, wall)?;
                    self.publish_running();
                }
                self.park = None;
            }
            FrameUpdate::Cancel(id) => {
                if let Some(call) = self.calls.get_mut(&id) {
                    call.cancel.send_replace(true);
                    call.cancelled = true;
                    if call.settled == Some(true) {
                        self.acks.insert(id);
                    }
                } else {
                    self.pending.remove(&id);
                    self.acks.insert(id);
                }
                self.reconcile();
                self.park = None;
            }
            FrameUpdate::Park {
                until,
                global_capacity,
            } => self.park = Some((Instant::from_std(until), global_capacity)),
            FrameUpdate::Yield => self.park = None,
            FrameUpdate::Settled(outcome) => {
                if !self.published
                    && matches!(
                        outcome.as_ref(),
                        ExecutionOutcome::Failed(failure)
                            if matches!(failure.origin, FailureOrigin::Safety)
                    )
                {
                    self.history.abort(wall)?;
                }
                self.publish_terminal(outcome);
                self.park = Some((self.deadline, false));
            }
        }
        Ok(())
    }
}

impl Actor {
    fn current(&self, id: FrameCallId) -> bool {
        self.graph
            .as_ref()
            .and_then(|graph| graph.driver(id.frame()))
            .is_some_and(|driver| {
                matches!(driver.phase, Phase::Calling { attempt, .. } if attempt == id.attempt())
            })
    }
    fn submitted(&mut self, id: FrameCallId, output: &Payload) {
        let current = self.current(id);
        if let Some(call) = self.calls.get_mut(&id) {
            if call.progress != Progress::Started || call.result_seen {
                self.fail(ExecutionError::Unavailable);
                return;
            }
            call.progress = Progress::Submitted;
            if !self.published
                && current
                && !call.cancelled
                && Instant::now().into_std() < call.dispatch.deadline()
            {
                if let Err(error) = self.history.event(
                    id.frame(),
                    events::task_submitted(
                        &call.machine,
                        call.dispatch.state(),
                        output,
                        Utc::now(),
                    ),
                ) {
                    self.fail(error);
                } else {
                    self.publish_running();
                }
            }
        }
    }
    async fn message(&mut self, message: Message) {
        self.control();
        match message {
            Message::Fatal(error) => self.fail(error),
            Message::Quarantined(id) => {
                if self.calls.contains_key(&id) {
                    self.cleanup(CleanupState::Quarantined);
                }
            }
            Message::Started(id) => {
                let current = self.current(id);
                if let Some(call) = self.calls.get_mut(&id) {
                    call.progress = Progress::Started;
                    if !self.published
                        && current
                        && !call.cancelled
                        && Instant::now().into_std() < call.dispatch.deadline()
                    {
                        let result = self.history.event(
                            id.frame(),
                            events::task_started(&call.machine, call.dispatch.state(), Utc::now()),
                        );
                        if let Err(error) = result {
                            self.fail(error);
                        } else {
                            self.publish_running();
                        }
                    }
                }
            }
            Message::Submitted(id, output) => self.submitted(id, &output),
            Message::Completed(id, result) => {
                let observed = Self::sample();
                let current = self.current(id);
                let Some(call) = self.calls.get_mut(&id) else {
                    return;
                };
                if call.result_seen {
                    return;
                }
                call.result_seen = true;
                if call.settled == Some(true) {
                    self.acks.insert(id);
                }
                if self.published || !current {
                    self.reconcile();
                    return;
                }
                if result.is_ok() && call.progress == Progress::Pending {
                    self.fail(ExecutionError::Unavailable);
                    return;
                }
                if observed.monotonic < call.dispatch.deadline() {
                    let event = match &result {
                        Ok(output) => events::task_success(
                            &call.machine,
                            call.dispatch.state(),
                            output,
                            observed.wall,
                        ),
                        Err(failure) => events::task_failure(
                            &call.machine,
                            call.dispatch.state(),
                            failure,
                            if call.progress == Progress::Pending {
                                TaskFailureKind::Admission
                            } else {
                                TaskFailureKind::Failed
                            },
                            observed.wall,
                        ),
                    };
                    if let Err(error) = self.history.event(id.frame(), event) {
                        self.fail(error);
                        return;
                    }
                    self.publish_running();
                }
                self.park = None;
                self.worker(Operation::Complete(id, result, observed)).await;
            }
        }
    }
    async fn dispatch(&mut self) {
        self.control();
        if self.published {
            return;
        }
        let Some(id) = self
            .pending
            .iter()
            .find_map(|(id, pending)| child_pattern(&pending.command).map(|_| *id))
            .or_else(|| self.pending.keys().next().copied())
        else {
            return;
        };
        if self.pending.get(&id).is_some_and(|pending| {
            Instant::now().into_std() >= pending.command.dispatch().deadline()
        }) {
            self.park = None;
            return;
        }
        let is_child = self
            .pending
            .get(&id)
            .and_then(|p| child_pattern(&p.command))
            .is_some();
        let permit = if is_child {
            self.reserved = None;
            None
        } else if let Some(permit) = self.reserved.take() {
            Some(permit)
        } else {
            let Ok(permit) = self.shared.pools.dispatch.clone().try_acquire_owned() else {
                return;
            };
            Some(permit)
        };
        self.control();
        if self.published {
            return;
        }
        let Some(pending) = self.pending.remove(&id) else {
            return;
        };
        if Instant::now().into_std() >= pending.command.dispatch().deadline() {
            self.pending.insert(id, pending);
            self.park = None;
            return;
        }
        let lease = Arc::new(DispatchLease {
            _permit: permit,
            _frame: Some(pending.frame),
        });
        self.leases.push(lease.clone());
        self.sync_ledger();
        let machine = pending.command.machine().clone();
        let dispatch = pending.command.dispatch().clone();
        if let Some(pattern) = child_pattern(&pending.command) {
            self.start_child(id, pending.command, pattern, lease);
            return;
        }
        let result = self.shared.port.as_ref().map_or_else(
            || Err(TaskFailure::service_exception()),
            |port| port.start(pending.command),
        );
        match result {
            Ok(call) => {
                self.accepted(id, machine, dispatch, call, lease);
            }
            Err(failure) => {
                self.release(&lease);
                let observed = Self::sample();
                self.control();
                if self.published {
                    return;
                }
                if observed.monotonic >= dispatch.deadline() {
                    self.acks.insert(id);
                    self.park = None;
                    return;
                }
                if let Err(error) = self.history.event(
                    id.frame(),
                    events::task_failure(
                        &machine,
                        dispatch.state(),
                        &failure,
                        TaskFailureKind::Admission,
                        observed.wall,
                    ),
                ) {
                    self.fail(error);
                    return;
                }
                self.publish_running();
                self.acks.insert(id);
                self.worker(Operation::Complete(id, Err(failure), observed))
                    .await;
            }
        }
    }
    fn accepted(
        &mut self,
        id: FrameCallId,
        machine: Arc<CompiledMachine>,
        dispatch: TaskDispatch,
        call: TaskCall,
        lease: Arc<DispatchLease>,
    ) {
        let cancel = call.cancellation_sender();
        let cancelled = cancel.subscribe();
        let child = matches!(
            machine.state(dispatch.state()),
            Ok(StateView::Task {
                resource: Integration::StartExecution(_),
                ..
            })
        );
        self.calls.insert(
            id,
            Call {
                cancel,
                lease: lease.clone(),
                machine,
                dispatch,
                progress: Progress::Pending,
                result_seen: false,
                cancelled: false,
                settled: None,
            },
        );
        let task = self.watchers.spawn(resource::run(
            id,
            OwnedCall { call, lease },
            cancelled,
            self.sender.clone(),
            child,
        ));
        self.watch_ids.insert(task.id(), id);
    }
    fn expired(&mut self) {
        let now = Instant::now().into_std();
        for call in self.calls.values_mut() {
            if !call.result_seen && !call.cancelled && now >= call.dispatch.deadline() {
                call.cancel.send_replace(true);
                call.cancelled = true;
                self.park = None;
            }
        }
        for pending in self.pending.values_mut() {
            if !pending.expired && now >= pending.command.dispatch().deadline() {
                pending.expired = true;
                self.park = None;
            }
        }
    }
    fn next_deadline(&self) -> Instant {
        self.calls
            .values()
            .filter(|call| !call.result_seen && !call.cancelled)
            .map(|call| Instant::from_std(call.dispatch.deadline()))
            .chain(
                self.pending
                    .values()
                    .filter(|pending| !pending.expired)
                    .map(|pending| Instant::from_std(pending.command.dispatch().deadline())),
            )
            .fold(self.deadline, Instant::min)
    }
    fn done(&self) -> bool {
        self.published
            && self.workers.is_empty()
            && self.watchers.is_empty()
            && self.child_jobs.is_empty()
            && self.pending.is_empty()
            && self.calls.values().all(|call| call.settled == Some(false))
            && self.park.is_some()
    }
    pub(super) async fn run(mut self) -> RunExit {
        self.register_children().await;
        let mut ticks = 0_u8;
        loop {
            ticks = ticks.saturating_add(1);
            if ticks >= 64 {
                yield_now().await;
                ticks = 0;
            }
            self.control();
            self.expired();
            self.reconcile();
            while let Some(result) = self.child_jobs.try_join_next() {
                if let Err(error) = result {
                    self.uncertain = true;
                    self.fail(ExecutionError::Worker(error));
                }
            }
            while let Some(result) = self.watchers.try_join_next_with_id() {
                self.joined(result);
            }
            if self.done() {
                break;
            }
            if self.workers.is_empty() {
                if self.published || self.committed.is_some() {
                    if let Some(graph) = &mut self.graph {
                        let update =
                            graph.advance(Self::sample(), &mut SystemUuidSource::default());
                        if let Err(error) = self.update(update, Utc::now()) {
                            self.fail(error);
                        }
                    }
                    if self.park.is_none() {
                        continue;
                    }
                } else {
                    if let Ok(message) = self.receiver.try_recv() {
                        self.message(message).await;
                        continue;
                    }
                    self.dispatch().await;
                    if self.workers.is_empty() && self.park.is_none() {
                        self.worker(Operation::Advance).await;
                        continue;
                    }
                }
            }
            if self.done() {
                break;
            }
            let until = self
                .park
                .map_or(self.deadline, |(until, _)| until)
                .min(self.next_deadline());
            let global = self.park.is_some_and(|(_, global)| global);
            tokio::select! {
                biased;
                _=self.stop.changed(),if !self.published=>self.control(),
                ()=sleep_until(until),if !self.published=>{self.control();self.expired();self.park=None;},
                result=self.workers.join_next(),if !self.workers.is_empty()=>if let Some(result)=result{self.evaluated(result);},
                result=self.child_jobs.join_next(),if !self.child_jobs.is_empty()=>if let Some(Err(error))=result{self.uncertain=true;self.fail(ExecutionError::Worker(error));},
                result=self.watchers.join_next_with_id(),if !self.watchers.is_empty()=>if let Some(result)=result{self.joined(result);},
                message=self.receiver.recv(),if self.workers.is_empty()=>if let Some(message)=message{self.message(message).await;},
                permit=self.shared.pools.dispatch.clone().acquire_owned(),if !self.published&&self.workers.is_empty()&&self.pending.values().any(|p| child_pattern(&p.command).is_none())&&self.reserved.is_none()=>{match permit{Ok(permit)=>self.reserved=Some(permit),Err(_)=>self.fail(ExecutionError::Unavailable)}},
                result=self.shared.frames.capacity.wait_for_capacity(until),if !self.published&&self.workers.is_empty()&&global=>{if result.is_ok(){self.park=None;}},
            }
        }
        let uncertain = self.uncertain || !self.leases.is_empty();
        self.cleanup(if uncertain {
            CleanupState::Quarantined
        } else {
            CleanupState::Clean
        });
        RunExit {
            quarantined: self.quarantine,
            uncertain,
        }
    }
}

fn child_pattern(command: &LambdaTaskCommand) -> Option<ChildPattern> {
    match command.machine().state(command.dispatch().state()).ok()? {
        StateView::Task {
            resource: Integration::StartExecution(pattern),
            ..
        } => Some(*pattern),
        _ => None,
    }
}
