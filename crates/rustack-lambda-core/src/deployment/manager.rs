//! Bounded deployment admission, retained child joins and keyed failure recovery.
//!
//! Owner Drop intentionally leaves its signalled root running on the captured
//! runtime to drain. The root owns child joins; abnormal root Drop transfers
//! them and their reservations to retained emergency cleanup. Joined shutdown,
//! not diagnostic counts, proves this owner's settlement. The separate image
//! owner still owns critical mutations after a preparation future is cancelled.

use std::{
    collections::HashMap,
    future::Future,
    mem,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use http::StatusCode;
use tokio::{
    runtime::Handle,
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, watch},
    task::{Id, JoinError, JoinSet},
    time::{Instant, sleep_until, timeout_at},
};
use typed_builder::TypedBuilder;
use uuid::Uuid;

use super::{ImageDeployment, ImageDeploymentFailure};
use crate::{
    artifact::{Architecture, ImageReference, PinnedImage},
    docker::{DockerError, ImageLeasePort},
    error::LambdaServiceError,
    executor::ExecutorError,
    invocation::CleanupTask,
    storage::{FunctionRecord, FunctionStore},
};

/// Validated accepted-deployment capacity and absolute preparation budget.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct ImageDeploymentConfig {
    jobs: u8,
    budget: Duration,
}
impl Default for ImageDeploymentConfig {
    fn default() -> Self {
        Self {
            jobs: 8,
            budget: Duration::from_secs(300),
        }
    }
}
impl ImageDeploymentConfig {
    /// Validate one bounded deployment owner; capacity includes queued and running jobs.
    ///
    /// # Errors
    /// Rejects capacity outside1..32 or a budget outside1..3,600 seconds.
    ///
    /// ```
    /// let config = rustack_lambda_core::deployment::ImageDeploymentConfig::new(
    ///     8, std::time::Duration::from_secs(300))?;
    /// # Ok::<(),rustack_lambda_core::error::LambdaServiceError>(())
    /// ```
    pub fn new(jobs: u8, budget: Duration) -> Result<Self, LambdaServiceError> {
        if !(1..=32).contains(&jobs)
            || budget < Duration::from_secs(1)
            || budget > Duration::from_secs(3600)
        {
            return Err(invalid("Invalid deployment capacity or deadline"));
        }
        Ok(Self { jobs, budget })
    }
}

/// Diagnostic counts; use joined quiescence/shutdown for settlement evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ImageDeploymentStatus {
    /// Queued/running jobs whose capacity has not yet been reaped.
    pub accepted: usize,
    /// Whether both owners currently permit new preparation.
    pub accepting: bool,
    /// A deployment supervisor failure permanently closed this owner.
    pub failed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Mode {
    Running,
    Drain,
    Cancel,
}
#[derive(Debug, Default)]
struct Health {
    failed: AtomicBool,
    #[cfg(test)]
    crash_root: AtomicBool,
    #[cfg(test)]
    crash_worker: AtomicBool,
}
fn signal(control: &watch::Sender<Mode>, mode: Mode) {
    control.send_if_modified(|current| {
        if *current < mode {
            *current = mode;
            true
        } else {
            false
        }
    });
}
fn unavailable() -> LambdaServiceError {
    LambdaServiceError::ResourceNotReady {
        message: "Image deployment owner is unavailable".into(),
    }
}
fn invalid(message: &str) -> LambdaServiceError {
    LambdaServiceError::InvalidParameter {
        message: message.into(),
    }
}
fn failed() -> ExecutorError {
    ExecutorError::CleanupFailed("Image deployment supervisor failed".into())
}

#[derive(Debug, TypedBuilder)]
struct Pending {
    job: ImageDeployment,
    store: Arc<FunctionStore>,
    health: Arc<Health>,
    control: watch::Sender<Mode>,
    #[builder(default)]
    settled: AtomicBool,
}
impl Pending {
    fn finish(
        &self,
        outcome: Result<PinnedImage, ImageDeploymentFailure>,
    ) -> Result<(), LambdaServiceError> {
        if !self.settled.load(Ordering::Acquire) {
            self.store.settle_image_deployment(&self.job, outcome)?;
            self.settled.store(true, Ordering::Release);
        }
        Ok(())
    }
    fn fail_owner(&self) {
        self.health.failed.store(true, Ordering::Release);
        signal(&self.control, Mode::Cancel);
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        if self.settled.load(Ordering::Acquire) {
            return;
        }
        let reason = if self.health.failed.load(Ordering::Acquire)
            || *self.control.borrow() != Mode::Cancel
        {
            self.fail_owner();
            ImageDeploymentFailure::SupervisorFailed
        } else {
            ImageDeploymentFailure::Cancelled
        };
        if self.finish(Err(reason)).is_err() {
            self.fail_owner();
            tracing::error!("Abandoned image deployment could not be settled");
        }
    }
}

#[derive(Debug)]
struct Envelope {
    id: Uuid,
    pending: Arc<Pending>,
    slot: OwnedSemaphorePermit,
    deadline: Instant,
}
#[derive(Debug)]
struct Row {
    pending: Arc<Pending>,
    _slot: OwnedSemaphorePermit,
}
#[derive(Debug)]
struct Reservation {
    queue: mpsc::OwnedPermit<Envelope>,
    slot: OwnedSemaphorePermit,
    deadline: Instant,
}

/// Own accepted image deployments through actual worker joins and keyed settlement.
#[derive(Debug, TypedBuilder)]
#[builder(
    builder_method(vis = ""),
    builder_type(vis = ""),
    build_method(vis = "")
)]
#[non_exhaustive]
pub struct ImageDeploymentManager {
    store: Arc<FunctionStore>,
    preparation: ImageLeasePort,
    config: ImageDeploymentConfig,
    capacity: Arc<Semaphore>,
    sender: mpsc::Sender<Envelope>,
    control: watch::Sender<Mode>,
    health: Arc<Health>,
    task: CleanupTask,
    emergency: Arc<CleanupTask>,
}
impl ImageDeploymentManager {
    /// Start a retained owner using an already qualified image preparation port.
    ///
    /// # Errors
    /// Rejects unavailable preparation, excess capacity or lack of an entered runtime.
    /// This does not probe or authorize the Docker execution backend.
    ///
    /// ```no_run
    /// # fn start(store:std::sync::Arc<rustack_lambda_core::storage::FunctionStore>, port:rustack_lambda_core::docker::ImageLeasePort) -> Result<(),rustack_lambda_core::error::LambdaServiceError> {
    /// let owner = rustack_lambda_core::deployment::ImageDeploymentManager::start(store, port,
    ///     rustack_lambda_core::deployment::ImageDeploymentConfig::default())?;
    /// # Ok(()) }
    /// ```
    pub fn start(
        store: Arc<FunctionStore>,
        preparation: ImageLeasePort,
        config: ImageDeploymentConfig,
    ) -> Result<Self, LambdaServiceError> {
        preparation
            .available()
            .map_err(|_| LambdaServiceError::DockerNotAvailable)?;
        if config.jobs > preparation.preparation_jobs() {
            return Err(invalid("Deployment jobs exceed image preparation capacity"));
        }
        let runtime = Handle::try_current().map_err(|_| unavailable())?;
        let (sender, inbox) = mpsc::channel(usize::from(config.jobs));
        let (control, _) = watch::channel(Mode::Running);
        let health = Arc::new(Health::default());
        let emergency = Arc::new(CleanupTask::default());
        let actor = Actor::builder()
            .inbox(inbox)
            .control(control.clone())
            .preparation(preparation.clone())
            .health(health.clone())
            .runtime(runtime.clone())
            .emergency(emergency.clone())
            .build();
        let task = CleanupTask::default();
        task.start_on(&runtime, move || actor.run());
        Ok(Self::builder()
            .store(store)
            .preparation(preparation)
            .config(config)
            .capacity(Arc::new(Semaphore::new(usize::from(config.jobs))))
            .sender(sender)
            .control(control)
            .health(health)
            .task(task)
            .emergency(emergency)
            .build())
    }

    fn available(&self) -> Result<(), LambdaServiceError> {
        if *self.control.borrow() != Mode::Running || self.health.failed.load(Ordering::Acquire) {
            return Err(unavailable());
        }
        self.preparation
            .available()
            .map_err(|_| LambdaServiceError::DockerNotAvailable)
    }
    fn reserve(&self) -> Result<Reservation, LambdaServiceError> {
        self.available()?;
        let deadline = Instant::now()
            .checked_add(self.config.budget)
            .ok_or_else(unavailable)?;
        let slot = self
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| LambdaServiceError::TooManyRequests)?;
        let queue = self
            .sender
            .clone()
            .try_reserve_owned()
            .map_err(|_| unavailable())?;
        self.available()?;
        Ok(Reservation {
            queue,
            slot,
            deadline,
        })
    }
    fn accept(
        &self,
        reservation: Reservation,
        job: ImageDeployment,
        accepted: FunctionRecord,
    ) -> FunctionRecord {
        let pending = Arc::new(
            Pending::builder()
                .job(job)
                .store(self.store.clone())
                .health(self.health.clone())
                .control(self.control.clone())
                .build(),
        );
        reservation.queue.send(Envelope {
            id: Uuid::new_v4(),
            pending,
            slot: reservation.slot,
            deadline: reservation.deadline,
        });
        accepted
    }

    /// Reserve work before inserting an initial Pending image function.
    ///
    /// # Errors
    /// Rejects invalid/duplicate resources, unavailable owners or exhausted capacity.
    /// The returned snapshot is captured before enqueue; dropping it does not cancel accepted work.
    ///
    /// ```no_run
    /// # fn create(owner:&rustack_lambda_core::deployment::ImageDeploymentManager, record:rustack_lambda_core::storage::FunctionRecord) -> Result<(),rustack_lambda_core::error::LambdaServiceError> {
    /// let pending = owner.create(record)?;
    /// # Ok(()) }
    /// ```
    pub fn create(&self, record: FunctionRecord) -> Result<FunctionRecord, LambdaServiceError> {
        let reservation = self.reserve()?;
        let (job, accepted) = self.store.reserve_image_create(record)?;
        Ok(self.accept(reservation, job, accepted))
    }

    /// Reserve an update while preserving previous code and configuration.
    ///
    /// # Errors
    /// Rejects stale resources/revisions, invalid settings, pending work or unavailable capacity.
    ///
    /// ```no_run
    /// # fn update(owner:&rustack_lambda_core::deployment::ImageDeploymentManager, before:&rustack_lambda_core::storage::FunctionRecord) -> Result<(),rustack_lambda_core::error::LambdaServiceError> {
    /// let reference = "worker:v2".parse().map_err(|_| rustack_lambda_core::error::LambdaServiceError::DockerNotAvailable)?;
    /// owner.update(before, reference, rustack_lambda_core::artifact::Architecture::Arm64)?;
    /// # Ok(()) }
    /// ```
    pub fn update(
        &self,
        before: &FunctionRecord,
        reference: ImageReference,
        architecture: Architecture,
    ) -> Result<FunctionRecord, LambdaServiceError> {
        self.update_with_revision(before, reference, architecture, None)
    }

    pub(crate) fn update_with_revision(
        &self,
        before: &FunctionRecord,
        reference: ImageReference,
        architecture: Architecture,
        requested: Option<&str>,
    ) -> Result<FunctionRecord, LambdaServiceError> {
        super::check_revision(requested, &before.latest.revision_id)?;
        let reservation = self.reserve()?;
        let (job, accepted) = self.store.reserve_image_update(
            &before.name,
            before.resource_id,
            &before.latest.revision_id,
            reference,
            architecture,
            requested,
        )?;
        Ok(self.accept(reservation, job, accepted))
    }

    /// Observe bounded admission; counts alone never prove clean settlement.
    ///
    /// ```no_run
    /// # fn inspect(owner:&rustack_lambda_core::deployment::ImageDeploymentManager) {
    /// let count = owner.status().accepted;
    /// # }
    /// ```
    #[must_use]
    pub fn status(&self) -> ImageDeploymentStatus {
        ImageDeploymentStatus {
            accepted: usize::from(self.config.jobs)
                .saturating_sub(self.capacity.available_permits()),
            accepting: self.available().is_ok(),
            failed: self.health.failed.load(Ordering::Acquire),
        }
    }

    pub(crate) fn cancel(&self) {
        signal(&self.control, Mode::Cancel);
    }

    async fn join(&self, deadline: Instant) -> Result<(), ExecutorError> {
        if Instant::now() >= deadline {
            return Err(ExecutorError::DeadlineExceeded);
        }
        let result = timeout_at(deadline, async {
            let primary = self.task.wait().await;
            let emergency = self.emergency.wait().await;
            primary.and(emergency)
        })
        .await
        .map_err(|_| ExecutorError::DeadlineExceeded)?;
        if Instant::now() >= deadline {
            return Err(ExecutorError::DeadlineExceeded);
        }
        result
    }
    /// Close admission and drain accepted workers without cancelling preparation.
    ///
    /// # Errors
    /// Returns deadline or cached supervision failure while retaining joins.
    /// The separate image owner must also join its critical mutations and cleanup.
    ///
    /// ```no_run
    /// # async fn stop(owner:&rustack_lambda_core::deployment::ImageDeploymentManager) {
    /// let result = owner.quiesce(tokio::time::Instant::now()+std::time::Duration::from_secs(10)).await;
    /// # }
    /// ```
    pub async fn quiesce(&self, deadline: Instant) -> Result<(), ExecutorError> {
        signal(&self.control, Mode::Drain);
        self.join(deadline).await
    }

    /// Cancel accepted preparations, settle records and join this owner's workers.
    ///
    /// # Errors
    /// Returns deadline or cached supervision failure; cancelled waiters retain joins.
    ///
    /// ```no_run
    /// # async fn stop(owner:&rustack_lambda_core::deployment::ImageDeploymentManager) {
    /// let result = owner.shutdown(tokio::time::Instant::now()+std::time::Duration::from_secs(10)).await;
    /// # }
    /// ```
    pub async fn shutdown(&self, deadline: Instant) -> Result<(), ExecutorError> {
        signal(&self.control, Mode::Cancel);
        self.join(deadline).await
    }
}
impl Drop for ImageDeploymentManager {
    fn drop(&mut self) {
        signal(&self.control, Mode::Cancel);
    }
}

#[derive(Debug, TypedBuilder)]
struct Actor {
    inbox: mpsc::Receiver<Envelope>,
    preparation: ImageLeasePort,
    control: watch::Sender<Mode>,
    health: Arc<Health>,
    runtime: Handle,
    emergency: Arc<CleanupTask>,
    #[builder(default)]
    jobs: JoinSet<Result<Uuid, ExecutorError>>,
    #[builder(default)]
    rows: HashMap<Uuid, Row>,
    #[builder(default)]
    index: HashMap<Id, Uuid>,
    #[builder(default)]
    settled: bool,
}
impl Actor {
    fn fail(&self) {
        self.health.failed.store(true, Ordering::Release);
        signal(&self.control, Mode::Cancel);
    }
    async fn run(mut self) -> Result<(), ExecutorError> {
        let mut control = self.control.subscribe();
        let mut closed = false;
        loop {
            if *control.borrow() != Mode::Running {
                self.inbox.close();
            }
            if closed && self.jobs.is_empty() {
                break;
            }
            tokio::select! {
                biased;
                _ = control.changed() => {},
                joined = self.jobs.join_next_with_id(), if !self.jobs.is_empty() => if let Some(joined) = joined { self.reap(&joined); },
                envelope = self.inbox.recv(), if !closed => if let Some(envelope) = envelope {
                    if *control.borrow() == Mode::Cancel {
                        if envelope.pending.finish(Err(ImageDeploymentFailure::Cancelled)).is_err() { self.fail(); }
                    } else { self.start(envelope); }
                } else {
                    closed = true;
                    if *control.borrow() == Mode::Running { signal(&self.control, Mode::Cancel); }
                },
            }
        }
        self.settled = true;
        if self.health.failed.load(Ordering::Acquire) {
            Err(failed())
        } else {
            Ok(())
        }
    }
    fn start(&mut self, envelope: Envelope) {
        #[cfg(test)]
        assert!(
            !self.health.crash_root.swap(false, Ordering::AcqRel),
            "injected deployment root failure"
        );
        let Envelope {
            id,
            pending,
            slot,
            deadline,
        } = envelope;
        self.rows.insert(
            id,
            Row {
                pending: pending.clone(),
                _slot: slot,
            },
        );
        let preparation = self.preparation.clone();
        let control = self.control.subscribe();
        #[cfg(test)]
        let health = self.health.clone();
        let worker = self.jobs.spawn(async move {
            #[cfg(test)]
            assert!(
                !health.crash_worker.swap(false, Ordering::AcqRel),
                "injected deployment worker failure"
            );
            let outcome = prepare(&pending, preparation, control, deadline).await;
            let outcome = if pending.health.failed.load(Ordering::Acquire) {
                Err(ImageDeploymentFailure::SupervisorFailed)
            } else {
                outcome
            };
            if pending.finish(outcome).is_err()
                && pending
                    .finish(Err(ImageDeploymentFailure::InvalidImage))
                    .is_err()
            {
                return Err(failed());
            }
            Ok(id)
        });
        self.index.insert(worker.id(), id);
    }
    fn reap(&mut self, joined: &Result<(Id, Result<Uuid, ExecutorError>), JoinError>) {
        let task = match joined {
            Ok((id, _)) => *id,
            Err(error) => error.id(),
        };
        let Some(id) = self.index.remove(&task) else {
            self.fail();
            return;
        };
        let Some(row) = self.rows.remove(&id) else {
            self.fail();
            return;
        };
        if !matches!(joined, Ok((_, Ok(returned))) if *returned == id)
            || !row.pending.settled.load(Ordering::Acquire)
        {
            self.fail();
            if row
                .pending
                .finish(Err(ImageDeploymentFailure::SupervisorFailed))
                .is_err()
            {
                tracing::error!("Failed image worker could not be settled");
            }
        }
    }
}
impl Drop for Actor {
    fn drop(&mut self) {
        if !self.settled {
            self.fail();
        }
        if self.rows.is_empty() {
            return;
        }
        self.fail();
        let rows = mem::take(&mut self.rows);
        let mut jobs = mem::take(&mut self.jobs);
        jobs.abort_all();
        self.emergency.start_on(&self.runtime, move || async move {
            while jobs.join_next().await.is_some() {}
            let mut clean = true;
            for row in rows.into_values() {
                clean &= row
                    .pending
                    .finish(Err(ImageDeploymentFailure::SupervisorFailed))
                    .is_ok();
            }
            if clean { Ok(()) } else { Err(failed()) }
        });
    }
}

async fn prepare(
    pending: &Pending,
    preparation: ImageLeasePort,
    mut control: watch::Receiver<Mode>,
    deadline: Instant,
) -> Result<PinnedImage, ImageDeploymentFailure> {
    let mut changes = pending.store.changes();
    let preparation = preparation.prepare(
        pending.job.reference(),
        pending.job.architecture(),
        pending.job.image_config(),
        deadline,
    );
    tokio::pin!(preparation);
    let result = loop {
        check(pending, &control, deadline)?;
        match next(preparation.as_mut(), &mut control, &mut changes, deadline).await? {
            Progress::Prepared(result) => break result.map_err(|error| map_failure(&error)),
            Progress::Changed => {}
        }
    };
    check(pending, &control, deadline)?;
    result
}
#[derive(Debug)]
enum Progress {
    Prepared(Result<PinnedImage, DockerError>),
    Changed,
}
async fn next(
    preparation: Pin<&mut impl Future<Output = Result<PinnedImage, DockerError>>>,
    control: &mut watch::Receiver<Mode>,
    changes: &mut watch::Receiver<()>,
    deadline: Instant,
) -> Result<Progress, ImageDeploymentFailure> {
    tokio::select! {
        biased;
        () = sleep_until(deadline) => Err(ImageDeploymentFailure::DeadlineExceeded),
        _ = control.wait_for(|mode| *mode == Mode::Cancel) => Err(ImageDeploymentFailure::Cancelled),
        result = preparation => Ok(Progress::Prepared(result)),
        notice = changes.changed() => notice.map(|()| Progress::Changed).map_err(|_| ImageDeploymentFailure::SupervisorFailed),
    }
}

fn check(
    pending: &Pending,
    control: &watch::Receiver<Mode>,
    deadline: Instant,
) -> Result<(), ImageDeploymentFailure> {
    if Instant::now() >= deadline {
        return Err(ImageDeploymentFailure::DeadlineExceeded);
    }
    if *control.borrow() == Mode::Cancel {
        return Err(ImageDeploymentFailure::Cancelled);
    }
    if !pending
        .job
        .is_current(&pending.store)
        .map_err(|_| ImageDeploymentFailure::SupervisorFailed)?
    {
        return Err(ImageDeploymentFailure::Cancelled);
    }
    Ok(())
}
fn map_failure(error: &DockerError) -> ImageDeploymentFailure {
    match error {
        DockerError::Status(StatusCode::NOT_FOUND) => ImageDeploymentFailure::ImageNotFound,
        DockerError::Deadline => ImageDeploymentFailure::DeadlineExceeded,
        DockerError::Cancelled => ImageDeploymentFailure::Cancelled,
        DockerError::Worker => ImageDeploymentFailure::SupervisorFailed,
        DockerError::Configuration
        | DockerError::Schema
        | DockerError::Image(_)
        | DockerError::Json(_)
        | DockerError::Protocol
        | DockerError::TooLarge => ImageDeploymentFailure::InvalidImage,
        _ => ImageDeploymentFailure::RuntimeUnavailable,
    }
}

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;
