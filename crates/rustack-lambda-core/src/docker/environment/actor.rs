//! Root registry owns reservations above workers and their recovery jobs.

use std::{
    collections::HashMap,
    mem,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use rustack_lambda_runtime::{diagnostics::runtime_dispatch, protocol::EnvironmentId};
use tokio::{
    runtime::Handle,
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, watch},
    task::{Id, JoinError, JoinSet},
    time::{Instant, sleep},
};
use tracing::instrument::WithSubscriber;
use typed_builder::TypedBuilder;

use super::{
    Call, EnvironmentConfig, EnvironmentError, EnvironmentPhase, EnvironmentStatus,
    cleanup::{Cleaner, Progress},
    worker::Worker,
};
use crate::{
    docker::{DockerPort, EnvironmentPlan},
    executor::ExecutorError,
    invocation::CleanupTask,
};

pub(super) struct Health {
    control: watch::Sender<bool>,
    pub(super) failed: AtomicBool,
    pub(super) quarantined: AtomicUsize,
}
impl Default for Health {
    fn default() -> Self {
        Self::new(watch::channel(false).0)
    }
}
impl Health {
    pub(super) fn new(control: watch::Sender<bool>) -> Self {
        Self {
            control,
            failed: AtomicBool::new(false),
            quarantined: AtomicUsize::new(0),
        }
    }
    pub(super) fn fail(&self) {
        self.failed.store(true, Ordering::Release);
        self.control.send_replace(true);
    }
}
pub(super) struct Budget {
    capacity: Arc<Semaphore>,
    maximum: usize,
    pub(super) memory: AtomicU64,
    limit: u64,
}
impl Budget {
    pub(super) fn new(config: EnvironmentConfig) -> Self {
        let maximum = usize::from(config.0.environments);
        Self {
            capacity: Arc::new(Semaphore::new(maximum)),
            maximum,
            memory: AtomicU64::new(0),
            limit: config.0.memory_bytes,
        }
    }
    pub(super) fn count(&self) -> usize {
        self.maximum
            .saturating_sub(self.capacity.available_permits())
    }
    pub(super) fn reserve(self: &Arc<Self>, memory: u64) -> Result<Reservation, EnvironmentError> {
        let permit = self
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| EnvironmentError::Busy)?;
        self.memory
            .try_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current
                    .checked_add(memory)
                    .filter(|total| *total <= self.limit)
            })
            .map_err(|_| EnvironmentError::Busy)?;
        Ok(Reservation {
            _permit: permit,
            memory,
            budget: self.clone(),
        })
    }
}
pub(super) struct Reservation {
    _permit: OwnedSemaphorePermit,
    memory: u64,
    budget: Arc<Budget>,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.memory.fetch_sub(self.memory, Ordering::AcqRel);
    }
}
#[derive(Clone)]
pub(super) struct Signals {
    pub(super) status: watch::Sender<EnvironmentStatus>,
    pub(super) stop: watch::Sender<bool>,
}
impl Signals {
    pub(super) fn new() -> Self {
        Self {
            status: watch::channel(EnvironmentStatus {
                phase: EnvironmentPhase::Allocating,
                error: None,
            })
            .0,
            stop: watch::channel(false).0,
        }
    }
    pub(super) fn phase(&self, phase: EnvironmentPhase) {
        self.status.send_modify(|status| status.phase = phase);
    }
    pub(super) fn error(&self, error: EnvironmentError) {
        self.status.send_modify(|status| {
            if status.error.is_none() {
                status.error = Some(error);
            }
        });
    }
    fn worker_failed(&self) {
        self.stop.send_replace(true);
        self.status.send_modify(|status| {
            status.phase = EnvironmentPhase::Retiring;
            status.error = Some(EnvironmentError::Worker);
        });
    }
}
#[derive(TypedBuilder)]
pub(super) struct Envelope {
    pub(super) plan: Arc<EnvironmentPlan>,
    pub(super) mailbox: mpsc::Receiver<Call>,
    pub(super) signals: Signals,
    pub(super) reservation: Reservation,
    pub(super) done: watch::Sender<Option<Result<(), EnvironmentError>>>,
    pub(super) deadline: Instant,
}
pub(super) struct Record {
    pub(super) plan: Arc<EnvironmentPlan>,
    pub(super) progress: Arc<Progress>,
    pub(super) io: Arc<CleanupTask>,
    pub(super) signals: Signals,
    pub(super) done: watch::Sender<Option<Result<(), EnvironmentError>>>,
    _reservation: Reservation,
}
impl Record {
    fn recovery(&self) -> Recovery {
        Recovery {
            plan: self.plan.clone(),
            progress: self.progress.clone(),
            io: self.io.clone(),
            signals: self.signals.clone(),
            done: self.done.clone(),
        }
    }
}
pub(super) struct Recovery {
    pub(super) plan: Arc<EnvironmentPlan>,
    pub(super) progress: Arc<Progress>,
    pub(super) io: Arc<CleanupTask>,
    pub(super) signals: Signals,
    pub(super) done: watch::Sender<Option<Result<(), EnvironmentError>>>,
}
impl Recovery {
    async fn run(
        self,
        docker: DockerPort,
        config: EnvironmentConfig,
        health: Arc<Health>,
        cleanup: Arc<Semaphore>,
    ) {
        let cleaner = Cleaner::builder()
            .docker(docker)
            .config(config)
            .plan(self.plan)
            .progress(self.progress)
            .health(health)
            .cleanup(cleanup)
            .signals(self.signals.clone())
            .build();
        cleaner.retire(&self.io, None).await;
        self.signals.phase(EnvironmentPhase::Removed);
        self.done.send_replace(Some(Err(EnvironmentError::Worker)));
    }
}

type Job = (EnvironmentId, Result<(), EnvironmentError>);
type Joined = Result<(Id, Job), JoinError>;
#[derive(TypedBuilder)]
pub(super) struct Actor {
    docker: DockerPort,
    config: EnvironmentConfig,
    inbox: mpsc::Receiver<Envelope>,
    control: watch::Sender<bool>,
    health: Arc<Health>,
    runtime: Handle,
    emergency: Arc<CleanupTask>,
    #[builder(default = Arc::new(Semaphore::new(2)))]
    cleanup: Arc<Semaphore>,
    #[builder(default)]
    jobs: JoinSet<Job>,
    #[builder(default)]
    records: HashMap<EnvironmentId, Record>,
    #[builder(default)]
    index: HashMap<Id, (EnvironmentId, bool)>,
    #[builder(default)]
    settled: bool,
}
impl Actor {
    pub(super) async fn run(mut self) -> Result<(), EnvironmentError> {
        let mut stopped = self.control.subscribe();
        let mut wire = self.docker.raw.stop.subscribe();
        loop {
            if *stopped.borrow() {
                self.inbox.close();
                while let Ok(envelope) = self.inbox.try_recv() {
                    let failed = self.health.failed.load(Ordering::Acquire);
                    envelope.signals.error(if failed {
                        EnvironmentError::Worker
                    } else {
                        EnvironmentError::Stopped
                    });
                    envelope.signals.phase(EnvironmentPhase::Removed);
                    envelope.done.send_replace(Some(if failed {
                        Err(EnvironmentError::Worker)
                    } else {
                        Ok(())
                    }));
                }
                for record in self.records.values() {
                    if self.health.failed.load(Ordering::Acquire) {
                        record.signals.worker_failed();
                    } else {
                        record.signals.stop.send_replace(true);
                    }
                }
                if self.jobs.is_empty() {
                    break;
                }
                if let Some(result) = self.jobs.join_next_with_id().await {
                    self.finished(&result);
                }
                continue;
            }
            tokio::select! {biased;
                _=wire.wait_for(|stop|*stop)=>{self.fail();},
                _=stopped.wait_for(|stop|*stop)=>{},
                result=self.jobs.join_next_with_id(),if !self.jobs.is_empty()=>if let Some(result)=result{self.finished(&result);},
                envelope=self.inbox.recv()=>match envelope{Some(envelope)=>self.start(envelope),None=>{self.control.send_replace(true);}},
            }
        }
        self.settled = true;
        if self.health.failed.load(Ordering::Acquire) {
            Err(EnvironmentError::Worker)
        } else {
            Ok(())
        }
    }
    fn start(&mut self, envelope: Envelope) {
        if *self.control.borrow()
            || *envelope.signals.stop.borrow()
            || Instant::now() >= envelope.deadline
        {
            envelope.signals.error(EnvironmentError::Stopped);
            envelope.signals.phase(EnvironmentPhase::Removed);
            envelope.done.send_replace(Some(Ok(())));
            return;
        }
        let id = envelope.plan.environment_id();
        if self.records.contains_key(&id) {
            self.fail();
            envelope.signals.error(EnvironmentError::Worker);
            envelope
                .done
                .send_replace(Some(Err(EnvironmentError::Worker)));
            return;
        }
        let progress = Arc::new(Progress::new());
        let io = Arc::new(CleanupTask::default());
        let worker = Worker::builder()
            .docker(self.docker.clone())
            .config(self.config)
            .plan(envelope.plan.clone())
            .progress(progress.clone())
            .cleanup(self.cleanup.clone())
            .health(self.health.clone())
            .signals(envelope.signals.clone())
            .global(self.control.clone())
            .io(io.clone())
            .runtime(self.runtime.clone())
            .build();
        self.records.insert(
            id,
            Record {
                plan: envelope.plan,
                progress,
                io,
                signals: envelope.signals,
                done: envelope.done,
                _reservation: envelope.reservation,
            },
        );
        let job = self.jobs.spawn(
            async move { (id, worker.run(envelope.mailbox, envelope.deadline).await) }
                .with_subscriber(runtime_dispatch()),
        );
        self.index.insert(job.id(), (id, false));
    }
    fn finished(&mut self, result: &Joined) {
        let task = match &result {
            Ok((task, _)) => *task,
            Err(error) => error.id(),
        };
        let Some((id, recovering)) = self.index.remove(&task) else {
            self.fail();
            return;
        };
        let clean = self
            .records
            .get(&id)
            .is_some_and(|record| record.progress.clean());
        if matches!(&result,Ok((_,(returned,Ok(())))) if *returned==id) && clean {
            if let Some(record) = self.records.remove(&id) {
                let failed = recovering || self.health.failed.load(Ordering::Acquire);
                if failed {
                    record.signals.worker_failed();
                }
                record.signals.phase(EnvironmentPhase::Removed);
                record.done.send_replace(Some(if failed {
                    Err(EnvironmentError::Worker)
                } else {
                    Ok(())
                }));
            }
            return;
        }
        self.fail();
        let Some(record) = self.records.get(&id) else {
            return;
        };
        record.signals.worker_failed();
        let recovery = record.recovery();
        let docker = self.docker.clone();
        let config = self.config;
        let health = self.health.clone();
        let cleanup = self.cleanup.clone();
        let job = self.jobs.spawn(async move {
            if recovering {
                sleep(Duration::from_secs(1)).await;
            }
            recovery.run(docker, config, health, cleanup).await;
            (id, Ok(()))
        });
        self.index.insert(job.id(), (id, true));
    }
    fn fail(&self) {
        self.health.fail();
        self.control.send_replace(true);
    }
}
impl Drop for Actor {
    fn drop(&mut self) {
        if !self.settled {
            self.fail();
        }
        while let Ok(envelope) = self.inbox.try_recv() {
            envelope.signals.worker_failed();
            envelope.signals.phase(EnvironmentPhase::Removed);
            envelope
                .done
                .send_replace(Some(Err(EnvironmentError::Worker)));
        }
        if self.records.is_empty() {
            return;
        }
        self.fail();
        for record in self.records.values() {
            record.signals.worker_failed();
        }
        let records = mem::take(&mut self.records);
        let mut jobs = mem::take(&mut self.jobs);
        jobs.abort_all();
        let docker = self.docker.clone();
        let config = self.config;
        let health = self.health.clone();
        let cleanup = self.cleanup.clone();
        self.emergency.start_on(&self.runtime, move || async move {
            while jobs.join_next().await.is_some() {}
            recover_all(records, docker, config, health, cleanup).await
        });
    }
}
async fn recover_all(
    mut records: HashMap<EnvironmentId, Record>,
    docker: DockerPort,
    config: EnvironmentConfig,
    health: Arc<Health>,
    cleanup: Arc<Semaphore>,
) -> Result<(), ExecutorError> {
    let mut jobs = JoinSet::new();
    let mut index = HashMap::new();
    for (id, record) in &records {
        let id = *id;
        let recovery = record.recovery();
        let docker = docker.clone();
        let health = health.clone();
        let cleanup = cleanup.clone();
        let job = jobs.spawn(async move {
            recovery.run(docker, config, health, cleanup).await;
            id
        });
        index.insert(job.id(), id);
    }
    while let Some(result) = jobs.join_next_with_id().await {
        let task = match &result {
            Ok((task, _)) => *task,
            Err(error) => error.id(),
        };
        let Some(id) = index.remove(&task) else {
            continue;
        };
        if matches!(result,Ok((_,returned)) if returned==id)
            && records
                .get(&id)
                .is_some_and(|record| record.progress.clean())
        {
            records.remove(&id);
            continue;
        }
        let Some(record) = records.get(&id) else {
            continue;
        };
        let recovery = record.recovery();
        let docker = docker.clone();
        let health = health.clone();
        let cleanup = cleanup.clone();
        let job = jobs.spawn(async move {
            sleep(Duration::from_secs(1)).await;
            recovery.run(docker, config, health, cleanup).await;
            id
        });
        index.insert(job.id(), id);
    }
    if records.is_empty() {
        Ok(())
    } else {
        Err(ExecutorError::CleanupFailed(
            "Environment recovery lost ownership".into(),
        ))
    }
}
