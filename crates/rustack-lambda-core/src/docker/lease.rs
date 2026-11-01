//! Retained pin preparation, shared live leases and phase-aware cleanup recovery.

use std::{
    collections::HashMap,
    fmt, mem,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use http::StatusCode;
use rustack_lambda_model::types::ImageConfig;
use tokio::{
    runtime::Handle,
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch},
    task::{Id, JoinError, JoinHandle, JoinSet},
    time::{Instant, sleep, timeout_at},
};
use typed_builder::TypedBuilder;
use uuid::Uuid;

use super::{
    DockerError, DockerPort, ImagePin, ImagePinScope, ImagePullPolicy, ImageResolver,
    check_deadline,
    connection::stopped,
    mutation::{MutationGuard, MutationReceipt},
    ownership::ImageRecord,
};
use crate::{
    artifact::{
        Architecture, ArtifactError, ImageLeaseContext, ImageReference, PinnedImage,
        PreparationAdmission, PreparationReceipt, ResolvedImage, Sha256Digest, UnpackedImageSize,
    },
    executor::ExecutorError,
    function_config::validate_image_config,
    invocation::CleanupTask,
};

fn check_preparation_deadline(deadline: Instant) -> Result<(), DockerError> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|value| !value.is_zero())
        .ok_or(DockerError::Deadline)?;
    if remaining > Duration::from_secs(3600) {
        return Err(DockerError::Configuration);
    }
    Ok(())
}

/// Validated image-preparation and live-pin admission/IO budgets.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct ImageLeaseConfig {
    pins: u16,
    jobs: u8,
    io: Duration,
}
impl Default for ImageLeaseConfig {
    fn default() -> Self {
        Self {
            pins: 1024,
            jobs: 8,
            io: Duration::from_secs(5),
        }
    }
}
impl ImageLeaseConfig {
    /// Validate bounded pin/job counts and the independent mutation IO budget.
    ///
    /// # Errors
    /// Rejects pins outside1..8192, jobs outside1..32 or exceeding pins, and
    /// zero/over60s IO budgets.
    ///
    /// ```
    /// let config = rustack_lambda_core::docker::ImageLeaseConfig::new(1024,8,std::time::Duration::from_secs(5))?;
    /// # Ok::<(),rustack_lambda_core::docker::DockerError>(())
    /// ```
    pub fn new(pins: u16, jobs: u8, io: Duration) -> Result<Self, DockerError> {
        if !(1..=8192).contains(&pins)
            || !(1..=32).contains(&jobs)
            || u16::from(jobs) > pins
            || io < Duration::from_secs(1)
            || io > Duration::from_secs(60)
        {
            return Err(DockerError::Configuration);
        }
        Ok(Self { pins, jobs, io })
    }
    fn deadline(self) -> Instant {
        Instant::now() + self.io
    }
}

/// Diagnostic counts; successful cleanup is established by joined ownership.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ImageLeaseStatus {
    /// Slots retained by queued, preparing, live or quarantined pins.
    pub pins: usize,
    /// Accepted preparations, including their failed-preparation cleanup.
    pub preparing: usize,
    /// Workers retaining uncertain/mismatching or failed cleanup.
    pub quarantined: usize,
    /// Whether new preparations can be accepted.
    pub accepting: bool,
    /// A worker/root supervision failure permanently closes admission.
    pub worker_failed: bool,
}
#[derive(Debug, Default)]
struct Health {
    failed: AtomicBool,
    quarantined: AtomicUsize,
}

#[derive(Debug)]
struct PinBudget {
    capacity: Arc<Semaphore>,
    count: AtomicUsize,
    changed: watch::Sender<()>,
}
impl PinBudget {
    fn new(max: u16) -> Self {
        let (changed, _) = watch::channel(());
        Self {
            capacity: Arc::new(Semaphore::new(usize::from(max))),
            count: AtomicUsize::new(0),
            changed,
        }
    }
    fn acquire(self: &Arc<Self>) -> Result<PinPermit, DockerError> {
        let permit = self
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| DockerError::Busy)?;
        self.count.fetch_add(permit.num_permits(), Ordering::AcqRel);
        self.changed.send_replace(());
        Ok(PinPermit {
            permit,
            budget: self.clone(),
        })
    }
}
#[derive(Debug)]
struct PinPermit {
    permit: OwnedSemaphorePermit,
    budget: Arc<PinBudget>,
}
impl Drop for PinPermit {
    fn drop(&mut self) {
        self.budget
            .count
            .fetch_sub(self.permit.num_permits(), Ordering::AcqRel);
        self.budget.changed.send_replace(());
    }
}
impl Health {
    fn check(&self) -> Result<(), DockerError> {
        if self.failed.load(Ordering::Acquire) {
            return Err(DockerError::Worker);
        }
        if self.quarantined.load(Ordering::Acquire) > 0 {
            return Err(DockerError::Quarantined);
        }
        Ok(())
    }
}

#[derive(Debug)]
enum PreparationSource {
    Reference {
        reference: ImageReference,
        architecture: Architecture,
    },
    Restore(Box<ResolvedImage>),
}
impl PreparationSource {
    fn architecture(&self) -> Architecture {
        match self {
            Self::Reference { architecture, .. } => *architecture,
            Self::Restore(saved) => saved.metadata().architecture,
        }
    }
}

#[derive(TypedBuilder)]
struct Request {
    source: PreparationSource,
    overrides: Option<ImageConfig>,
    deadline: Instant,
    cancelled: watch::Receiver<()>,
    reply: oneshot::Sender<Result<PinnedImage, DockerError>>,
    slot: Option<OwnedSemaphorePermit>,
    #[builder(default)]
    retained_admission: Option<Arc<OwnedSemaphorePermit>>,
}
impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImagePreparation")
            .field("deadline", &self.deadline)
            .finish_non_exhaustive()
    }
}
#[derive(Debug)]
struct Envelope {
    request: Request,
    pin: ImagePin,
    permit: PinPermit,
}

/// Cloneable bounded preparation port; it does not own manager shutdown.
#[derive(Debug, Clone, TypedBuilder)]
#[non_exhaustive]
pub struct ImageLeasePort {
    #[builder(default)]
    platform: Option<Architecture>,
    sender: mpsc::Sender<Envelope>,
    slots: Arc<Semaphore>,
    pins: Arc<PinBudget>,
    stop: watch::Sender<bool>,
    health: Arc<Health>,
    scope: ImagePinScope,
    config: ImageLeaseConfig,
}
impl ImageLeasePort {
    pub(crate) fn available(&self) -> Result<(), DockerError> {
        self.health.check()?;
        if *self.stop.borrow() {
            return Err(DockerError::Stopped);
        }
        Ok(())
    }
    pub(crate) fn preparation_jobs(&self) -> u8 {
        self.config.jobs
    }

    /// Prepare a live pinned artifact; dropping this future signals cancellation.
    ///
    /// # Errors
    /// Returns invalid settings, bounded admission, deadline, metadata, mutation
    /// or cleanup failure. In-flight mutations and cleanup remain supervised.
    ///
    /// ```no_run
    /// # async fn prepare(port:&rustack_lambda_core::docker::ImageLeasePort) -> Result<(),rustack_lambda_core::docker::DockerError> {
    /// let image = port.prepare(&"worker:v1".parse()?,rustack_lambda_core::artifact::Architecture::Arm64,None,
    ///     tokio::time::Instant::now()+std::time::Duration::from_secs(30)).await?;
    /// assert!(image.is_live());
    /// # Ok(()) }
    /// ```
    pub async fn prepare(
        &self,
        reference: &ImageReference,
        architecture: Architecture,
        overrides: Option<&ImageConfig>,
        deadline: Instant,
    ) -> Result<PinnedImage, DockerError> {
        self.prepare_guarded(
            PreparationSource::Reference {
                reference: reference.clone(),
                architecture,
            },
            overrides,
            deadline,
            None,
        )
        .await
    }

    /// Reacquire local immutable content without following source tags or pulling.
    ///
    /// The saved ID, selected platform and execution fingerprint must match fresh
    /// inspection before a new durable private pin can be created. Saved metadata
    /// alone never grants availability. This does not publish an archive store.
    ///
    /// # Errors
    /// Returns unavailable/exhausted admission, deadline, missing immutable content,
    /// changed inspection metadata or retained mutation/cleanup failure. Cancellation
    /// never discards an uncertain mutation or its ownership.
    ///
    /// ```no_run
    /// # async fn restore(port: &rustack_lambda_core::docker::ImageLeasePort, saved: &rustack_lambda_core::artifact::ResolvedImage) -> Result<(), rustack_lambda_core::docker::DockerError> {
    /// let image = port.restore(saved, None, tokio::time::Instant::now() + std::time::Duration::from_secs(30)).await?;
    /// assert!(image.is_live());
    /// # Ok(()) }
    /// ```
    pub async fn restore(
        &self,
        saved: &ResolvedImage,
        overrides: Option<&ImageConfig>,
        deadline: Instant,
    ) -> Result<PinnedImage, DockerError> {
        self.prepare_guarded(
            PreparationSource::Restore(Box::new(saved.clone())),
            overrides,
            deadline,
            None,
        )
        .await
    }

    pub(crate) async fn prepare_deployment(
        &self,
        reference: &ImageReference,
        architecture: Architecture,
        overrides: Option<&ImageConfig>,
        deadline: Instant,
        admission: Arc<OwnedSemaphorePermit>,
    ) -> Result<PinnedImage, DockerError> {
        self.prepare_guarded(
            PreparationSource::Reference {
                reference: reference.clone(),
                architecture,
            },
            overrides,
            deadline,
            Some(admission),
        )
        .await
    }

    async fn prepare_guarded(
        &self,
        source: PreparationSource,
        overrides: Option<&ImageConfig>,
        deadline: Instant,
        retained_admission: Option<Arc<OwnedSemaphorePermit>>,
    ) -> Result<PinnedImage, DockerError> {
        let deployment = retained_admission.is_some();
        self.available()?;
        if self
            .platform
            .is_some_and(|platform| platform != source.architecture())
        {
            return Err(DockerError::Configuration);
        }
        check_preparation_deadline(deadline)?;
        if let Some(config) = overrides {
            validate_image_config(config).map_err(|_| DockerError::Configuration)?;
        }
        let slot = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| DockerError::Busy)?;
        let permit = self.pins.acquire()?;
        let pin = self.scope.allocate()?;
        let (cancel, cancelled) = watch::channel(());
        let (reply, response) = oneshot::channel();
        let request = Request::builder()
            .source(source)
            .overrides(overrides.cloned())
            .deadline(deadline)
            .cancelled(cancelled)
            .reply(reply)
            .slot(Some(slot))
            .retained_admission(retained_admission)
            .build();
        self.sender
            .try_send(Envelope {
                request,
                pin,
                permit,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => DockerError::Busy,
                mpsc::error::TrySendError::Closed(_) => DockerError::Stopped,
            })?;
        let result = timeout_at(deadline, response)
            .await
            .map_err(|_| DockerError::Deadline)?
            .map_err(|_| DockerError::Worker)?;
        drop(cancel);
        self.available()?;
        if Instant::now() >= deadline {
            return Err(DockerError::Deadline);
        }
        let image = result?;
        if !deployment {
            image.accept_preparation();
        }
        Ok(image)
    }
    /// Observe release of every pin reservation without stopping admission.
    ///
    /// # Errors
    /// Returns expiry or supervision failure. A timeout never drops cleanup.
    /// Concurrent new preparations can delay this observation.
    ///
    /// ```no_run
    /// # async fn empty(port:&rustack_lambda_core::docker::ImageLeasePort) {
    /// let clean=port.wait_empty(tokio::time::Instant::now()+std::time::Duration::from_secs(10)).await;
    /// # }
    /// ```
    pub async fn wait_empty(&self, deadline: Instant) -> Result<(), DockerError> {
        check_deadline(deadline)?;
        let mut changed = self.pins.changed.subscribe();
        while self.pins.count.load(Ordering::Acquire) > 0 {
            timeout_at(deadline, changed.changed())
                .await
                .map_err(|_| DockerError::Deadline)?
                .map_err(|_| DockerError::Worker)?;
        }
        if Instant::now() >= deadline {
            return Err(DockerError::Deadline);
        }
        self.health.check()
    }
    /// Inspect bounded resource counts without logging source image defaults.
    ///
    /// ```no_run
    /// # fn status(port:&rustack_lambda_core::docker::ImageLeasePort) {
    /// let status=port.status();
    /// # }
    /// ```
    #[must_use]
    pub fn status(&self) -> ImageLeaseStatus {
        let worker_failed = self.health.failed.load(Ordering::Acquire);
        let quarantined = self.health.quarantined.load(Ordering::Acquire);
        ImageLeaseStatus {
            pins: self.pins.count.load(Ordering::Acquire),
            preparing: usize::from(self.config.jobs).saturating_sub(self.slots.available_permits()),
            quarantined,
            accepting: !worker_failed && quarantined == 0 && !*self.stop.borrow(),
            worker_failed,
        }
    }
}

/// Owner of every preparation, live alias and retained cleanup worker.
#[derive(Debug)]
#[non_exhaustive]
pub struct ImageLeaseManager {
    port: ImageLeasePort,
    task: Option<JoinHandle<Result<(), DockerError>>>,
    primary: Option<Result<(), DockerError>>,
    settlement: Option<Result<(), DockerError>>,
    emergency: Arc<CleanupTask>,
}
impl ImageLeaseManager {
    pub(super) fn physically_settled(&self) -> bool {
        self.primary.is_some()
            && self.task.is_none()
            && self.emergency.is_finished()
            && self.port.status().pins == 0
            && self.port.status().preparing == 0
    }

    /// Start under the current Tokio runtime with an explicit scoped Docker port.
    ///
    /// # Errors
    /// Rejects absence of a Tokio runtime or a port not bound to this exact
    /// directory lease. The scope is never taken from a function request.
    ///
    /// ```no_run
    /// # fn start(port:rustack_lambda_core::docker::DockerPort,scope:rustack_lambda_core::docker::ImagePinScope) -> Result<(),rustack_lambda_core::docker::DockerError> {
    /// let owner=rustack_lambda_core::docker::ImageLeaseManager::start(port,scope,Default::default())?;
    /// # Ok(()) }
    /// ```
    pub fn start(
        docker: DockerPort,
        scope: ImagePinScope,
        config: ImageLeaseConfig,
    ) -> Result<Self, DockerError> {
        Self::start_with_pull(docker, scope, config, ImagePullPolicy::default())
    }

    /// Start with an explicit validated operator pull policy.
    ///
    /// # Errors
    /// Rejects absent runtime or a port not bound to this exact directory.
    /// No startup readiness is granted by this constructor.
    ///
    /// ```no_run
    /// # fn start(port:rustack_lambda_core::docker::DockerPort,scope:rustack_lambda_core::docker::ImagePinScope)->Result<(),rustack_lambda_core::docker::DockerError> {
    /// let policy=rustack_lambda_core::docker::ImagePullPolicy::if_missing(vec!["registry.example".parse()?],Default::default(),std::time::Duration::from_secs(300))?;
    /// let images=rustack_lambda_core::docker::ImageLeaseManager::start_with_pull(port,scope,Default::default(),policy)?;
    /// # Ok(()) }
    /// ```
    pub fn start_with_pull(
        docker: DockerPort,
        scope: ImagePinScope,
        config: ImageLeaseConfig,
        policy: ImagePullPolicy,
    ) -> Result<Self, DockerError> {
        Self::start_platform(docker, scope, config, policy, None)
    }
    pub(super) fn start_qualified(
        docker: DockerPort,
        scope: ImagePinScope,
        config: ImageLeaseConfig,
        policy: ImagePullPolicy,
        platform: Architecture,
    ) -> Result<Self, DockerError> {
        Self::start_platform(docker, scope, config, policy, Some(platform))
    }
    fn start_platform(
        docker: DockerPort,
        scope: ImagePinScope,
        config: ImageLeaseConfig,
        policy: ImagePullPolicy,
        platform: Option<Architecture>,
    ) -> Result<Self, DockerError> {
        docker.require_directory(scope.directory())?;
        let runtime = Handle::try_current().map_err(|_| DockerError::Worker)?;
        let (sender, requests) = mpsc::channel(32);
        let (stop, _) = watch::channel(false);
        let health = Arc::new(Health::default());
        let emergency = Arc::new(CleanupTask::default());
        let port = ImageLeasePort::builder()
            .platform(platform)
            .sender(sender)
            .slots(Arc::new(Semaphore::new(usize::from(config.jobs))))
            .pins(Arc::new(PinBudget::new(config.pins)))
            .stop(stop.clone())
            .health(health.clone())
            .scope(scope)
            .config(config)
            .build();
        let actor = Actor::builder()
            .docker(docker)
            .policy(policy)
            .config(config)
            .inbox(requests)
            .control(stop)
            .health(health)
            .runtime(runtime.clone())
            .emergency(emergency.clone())
            .build();
        let task = runtime.spawn(actor.run());
        Ok(Self {
            port,
            task: Some(task),
            primary: None,
            settlement: None,
            emergency,
        })
    }
    /// Clone a preparation port; retain this owner through consumer shutdown.
    ///
    /// ```no_run
    /// # fn port(owner:&rustack_lambda_core::docker::ImageLeaseManager) {
    /// let port=owner.port();
    /// # }
    /// ```
    #[must_use]
    pub fn port(&self) -> ImageLeasePort {
        self.port.clone()
    }

    /// Revoke leases, stop admission and join pin cleanup before `DockerWire` stops.
    ///
    /// # Errors
    /// Returns timeout while retaining cleanup, or cached supervision failure.
    /// The runtime must first join invocation/environment consumers of the leases.
    ///
    /// ```no_run
    /// # async fn stop(owner:&mut rustack_lambda_core::docker::ImageLeaseManager) {
    /// let result=owner.shutdown(tokio::time::Instant::now()+std::time::Duration::from_secs(10)).await;
    /// # }
    /// ```
    pub async fn shutdown(&mut self, deadline: Instant) -> Result<(), DockerError> {
        self.port.stop.send_replace(true);
        if let Some(result) = &self.settlement {
            return result.clone();
        }
        check_deadline(deadline)?;
        if self.primary.is_none() {
            let result = match self.task.as_mut() {
                Some(task) => timeout_at(deadline, task)
                    .await
                    .map_err(|_| DockerError::Deadline)?
                    .map_err(|_| DockerError::Worker)
                    .and_then(|result| result),
                None => Err(DockerError::Worker),
            };
            self.primary = Some(result);
            self.task.take();
        }
        let emergency = timeout_at(deadline, self.emergency.wait())
            .await
            .map_err(|_| DockerError::Deadline)?
            .map_err(|_| DockerError::Worker);
        if Instant::now() >= deadline {
            return Err(DockerError::Deadline);
        }
        let result = self
            .primary
            .clone()
            .ok_or(DockerError::Worker)?
            .and(emergency);
        self.settlement = Some(result.clone());
        result
    }
}
impl Drop for ImageLeaseManager {
    fn drop(&mut self) {
        self.port.stop.send_replace(true);
    }
}

#[derive(Debug, Clone)]
enum Phase {
    Unissued,
    Preparing(Sha256Digest),
    Uncertain(Sha256Digest, MutationReceipt),
    Committing(Sha256Digest),
    Known(Sha256Digest),
    NoEffect(Sha256Digest),
    Clean,
}
#[derive(Debug)]
struct Progress {
    phase: watch::Sender<Phase>,
    released: watch::Receiver<()>,
    active: Arc<AtomicBool>,
}
#[derive(Debug)]
struct Record {
    pin: ImagePin,
    progress: Arc<Progress>,
    permit: PinPermit,
    _admission: PreparationReceipt,
}

type PinJob = (Uuid, Result<(), DockerError>);
type PinJoin = Result<(Id, PinJob), JoinError>;

#[derive(Debug, TypedBuilder)]
struct Actor {
    docker: DockerPort,
    #[builder(default)]
    policy: ImagePullPolicy,
    config: ImageLeaseConfig,
    inbox: mpsc::Receiver<Envelope>,
    control: watch::Sender<bool>,
    health: Arc<Health>,
    runtime: Handle,
    emergency: Arc<CleanupTask>,
    #[builder(default = Arc::new(Semaphore::new(2)))]
    cleanup: Arc<Semaphore>,
    #[builder(default)]
    jobs: JoinSet<PinJob>,
    #[builder(default)]
    records: HashMap<Uuid, Record>,
    #[builder(default)]
    index: HashMap<Id, Uuid>,
    #[builder(default)]
    settled: bool,
}
impl Actor {
    async fn run(mut self) -> Result<(), DockerError> {
        let mut stop = self.control.subscribe();
        let mut wire_stop = self.docker.raw.stop.subscribe();
        loop {
            if *stop.borrow() {
                self.inbox.close();
                while let Ok(envelope) = self.inbox.try_recv() {
                    let _ = envelope.request.reply.send(Err(DockerError::Stopped));
                }
                for record in self.records.values() {
                    record.progress.active.store(false, Ordering::Release);
                }
                if self.jobs.is_empty() {
                    break;
                }
                if let Some(result) = self.jobs.join_next_with_id().await {
                    self.finished(&result);
                }
                continue;
            }
            tokio::select! {
                biased;
                ()=stopped(&mut wire_stop)=>{ self.health.failed.store(true,Ordering::Release);self.control.send_replace(true); },
                ()=stopped(&mut stop)=>{},
                result=self.jobs.join_next_with_id(),if !self.jobs.is_empty()=>if let Some(result)=result {self.finished(&result);},
                envelope=self.inbox.recv()=>match envelope {Some(envelope)=>self.start(envelope),None=>{self.control.send_replace(true);}},
            }
        }
        self.settled = true;
        if self.health.failed.load(Ordering::Acquire) {
            Err(DockerError::Worker)
        } else {
            Ok(())
        }
    }
    fn start(&mut self, mut envelope: Envelope) {
        if self.health.check().is_err()
            || *self.control.borrow()
            || envelope.request.reply.is_closed()
            || envelope.request.cancelled.has_changed().is_err()
            || Instant::now() >= envelope.request.deadline
        {
            let _ = envelope.request.reply.send(Err(DockerError::Stopped));
            return;
        }
        let Some(slot) = envelope.request.slot.take() else {
            self.health.failed.store(true, Ordering::Release);
            self.control.send_replace(true);
            let _ = envelope.request.reply.send(Err(DockerError::Worker));
            return;
        };
        let (admission, _) = watch::channel(Some(PreparationAdmission::new(
            slot,
            envelope.request.retained_admission.take(),
        )));
        let (phase, _) = watch::channel(Phase::Unissued);
        let (release, released) = watch::channel(());
        let progress = Arc::new(Progress {
            phase,
            released,
            active: Arc::new(AtomicBool::new(false)),
        });
        let id = envelope.pin.id;
        let worker = Worker::builder()
            .docker(self.docker.clone())
            .policy(self.policy.clone())
            .config(self.config)
            .pin(envelope.pin.clone())
            .progress(progress.clone())
            .control(self.control.clone())
            .health(self.health.clone())
            .cleanup(self.cleanup.clone())
            .build();
        self.records.insert(
            id,
            Record {
                pin: envelope.pin,
                progress,
                permit: envelope.permit,
                _admission: admission.clone(),
            },
        );
        let health = self.health.clone();
        let control = self.control.clone();
        let job = self.jobs.spawn(async move {
            let mut guard = WorkerGuard {
                health,
                control,
                finished: false,
            };
            let result = worker.run(envelope.request, release, admission).await;
            guard.finished = true;
            (id, result)
        });
        self.index.insert(job.id(), id);
    }
    fn finished(&mut self, result: &PinJoin) {
        let task = match result {
            Ok((id, _)) => *id,
            Err(error) => error.id(),
        };
        let Some(id) = self.index.remove(&task) else {
            self.fail();
            return;
        };
        let clean = self.records.get(&id).is_some_and(|record| {
            matches!(
                *record.progress.phase.borrow(),
                Phase::Unissued | Phase::Clean
            )
        });
        if matches!(result, Ok((_, (returned, Ok(())))) if *returned==id) && clean {
            if let Some(record) = self.records.remove(&id) {
                drop(record.permit);
            }
            return;
        }
        self.fail();
        let Some(record) = self.records.get(&id) else {
            return;
        };
        record.progress.active.store(false, Ordering::Release);
        let worker = Worker::builder()
            .docker(self.docker.clone())
            .config(self.config)
            .pin(record.pin.clone())
            .progress(record.progress.clone())
            .control(self.control.clone())
            .health(self.health.clone())
            .cleanup(self.cleanup.clone())
            .build();
        let job = self.jobs.spawn(async move {
            worker.retire().await;
            (id, Ok(()))
        });
        self.index.insert(job.id(), id);
    }
    fn fail(&self) {
        self.health.failed.store(true, Ordering::Release);
        self.control.send_replace(true);
    }
}
impl Drop for Actor {
    fn drop(&mut self) {
        if !self.settled {
            self.fail();
        }
        if self.records.is_empty() {
            return;
        }
        self.fail();
        for record in self.records.values() {
            record.progress.active.store(false, Ordering::Release);
        }
        let records = mem::take(&mut self.records);
        let mut jobs = mem::take(&mut self.jobs);
        jobs.abort_all();
        let docker = self.docker.clone();
        let config = self.config;
        let control = self.control.clone();
        let health = self.health.clone();
        let cleanup = self.cleanup.clone();
        self.emergency.start_on(&self.runtime, move || async move {
            while jobs.join_next().await.is_some() {}
            recover_all(records, docker, config, control, health, cleanup).await
        });
    }
}

async fn recover_all(
    mut records: HashMap<Uuid, Record>,
    docker: DockerPort,
    config: ImageLeaseConfig,
    control: watch::Sender<bool>,
    health: Arc<Health>,
    cleanup: Arc<Semaphore>,
) -> Result<(), ExecutorError> {
    let mut jobs = JoinSet::new();
    let mut index = HashMap::new();
    for (id, record) in &records {
        let worker = Worker::builder()
            .docker(docker.clone())
            .config(config)
            .pin(record.pin.clone())
            .progress(record.progress.clone())
            .control(control.clone())
            .health(health.clone())
            .cleanup(cleanup.clone())
            .build();
        let id = *id;
        let job = jobs.spawn(async move {
            worker.retire().await;
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
        let Some(record) = records.get(&id) else {
            continue;
        };
        if matches!(
            *record.progress.phase.borrow(),
            Phase::Unissued | Phase::Clean
        ) {
            if let Some(record) = records.remove(&id) {
                drop(record.permit);
            }
        } else {
            let worker = Worker::builder()
                .docker(docker.clone())
                .config(config)
                .pin(record.pin.clone())
                .progress(record.progress.clone())
                .control(control.clone())
                .health(health.clone())
                .cleanup(cleanup.clone())
                .build();
            let job = jobs.spawn(async move {
                worker.retire().await;
                id
            });
            index.insert(job.id(), id);
        }
    }
    if records.is_empty() {
        Ok(())
    } else {
        Err(ExecutorError::CleanupFailed(
            "Image pin recovery lost supervision".into(),
        ))
    }
}

struct WorkerGuard {
    health: Arc<Health>,
    control: watch::Sender<bool>,
    finished: bool,
}
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        if !self.finished {
            self.health.failed.store(true, Ordering::Release);
            self.control.send_replace(true);
        }
    }
}
struct Dirty(Arc<Health>);
impl Dirty {
    fn new(health: Arc<Health>) -> Self {
        health.quarantined.fetch_add(1, Ordering::AcqRel);
        Self(health)
    }
}
impl Drop for Dirty {
    fn drop(&mut self) {
        self.0.quarantined.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Debug, TypedBuilder)]
struct Worker {
    docker: DockerPort,
    #[builder(default)]
    policy: ImagePullPolicy,
    config: ImageLeaseConfig,
    pin: ImagePin,
    progress: Arc<Progress>,
    control: watch::Sender<bool>,
    health: Arc<Health>,
    cleanup: Arc<Semaphore>,
}
impl Worker {
    fn check(&self, deadline: Instant, cancel: &watch::Receiver<()>) -> Result<(), DockerError> {
        if Instant::now() >= deadline {
            return Err(DockerError::Deadline);
        }
        if *self.control.borrow() {
            return Err(DockerError::Stopped);
        }
        if cancel.has_changed().is_err() {
            return Err(DockerError::Cancelled);
        }
        Ok(())
    }
    async fn run(
        mut self,
        request: Request,
        release: watch::Sender<()>,
        admission: PreparationReceipt,
    ) -> Result<(), DockerError> {
        let Request {
            source,
            overrides,
            deadline,
            mut cancelled,
            reply,
            slot: _,
            retained_admission: _,
        } = request;
        let staged = self
            .stage(&source, overrides.as_ref(), deadline, &mut cancelled)
            .await;
        drop(source);
        drop(overrides);
        match staged {
            Ok((resolved, size)) => {
                self.progress.active.store(true, Ordering::Release);
                let image = PinnedImage::with_context(
                    resolved,
                    size,
                    release,
                    self.progress.active.clone(),
                    self.control.subscribe(),
                    ImageLeaseContext::Owned {
                        directory: self.pin.directory().clone(),
                        preparation: Some(admission),
                    },
                );
                let delivered = match self.check(deadline, &cancelled) {
                    Ok(()) => reply.send(Ok(image)).is_ok(),
                    Err(error) => {
                        self.progress.active.store(false, Ordering::Release);
                        drop(image);
                        let _ = reply.send(Err(error));
                        false
                    }
                };
                drop(cancelled);
                if !delivered {
                    self.progress.active.store(false, Ordering::Release);
                    self.retire().await;
                    return Ok(());
                }
                let mut released = self.progress.released.clone();
                let mut stop = self.control.subscribe();
                tokio::select! {biased;()=stopped(&mut stop)=>{},_ = released.changed()=>{}}
                self.retire().await;
            }
            Err(error) => {
                let _ = reply.send(Err(error));
                self.retire().await;
            }
        }
        Ok(())
    }
    async fn resolve_source(
        &self,
        source: &PreparationSource,
        overrides: Option<&ImageConfig>,
        deadline: Instant,
        cancel: &mut watch::Receiver<()>,
    ) -> Result<ResolvedImage, DockerError> {
        self.check(deadline, cancel)?;
        let architecture = source.architecture();
        let (reference, lookup) = match source {
            PreparationSource::Reference { reference, .. } => (reference, reference.clone()),
            PreparationSource::Restore(saved) => (
                &saved.metadata().requested_reference,
                saved.metadata().image_id.as_str().parse()?,
            ),
        };
        let resolver = ImageResolver::new(self.docker.clone());
        let mut stop = self.control.subscribe();
        let root = tokio::select! {biased;()=stopped(&mut stop)=>return Err(DockerError::Stopped),_ = cancel.changed()=>return Err(DockerError::Cancelled),result=self.docker.inspect_image(&lookup,deadline.min(self.config.deadline()))=>result,};
        let root = match root {
            Err(DockerError::Status(StatusCode::NOT_FOUND))
                if matches!(source, PreparationSource::Reference { .. }) =>
            {
                self.policy
                    .pull(
                        &self.docker,
                        reference,
                        architecture,
                        cancel.clone(),
                        self.control.subscribe(),
                    )
                    .await?;
                self.check(deadline, cancel)?;
                tokio::select! {biased;()=stopped(&mut stop)=>return Err(DockerError::Stopped),_ = cancel.changed()=>return Err(DockerError::Cancelled),result=self.docker.inspect_image(reference,deadline.min(self.config.deadline()))=>result?,}
            }
            result => result?,
        };
        if let PreparationSource::Restore(saved) = source {
            if root.id.as_deref() != Some(saved.metadata().image_id.as_str()) {
                return Err(ArtifactError::Inspection.into());
            }
        }
        let actual = tokio::select! {biased;()=stopped(&mut stop)=>return Err(DockerError::Stopped),_ = cancel.changed()=>return Err(DockerError::Cancelled),result=resolver.resolve_captured(reference,architecture,overrides,root,deadline.min(self.config.deadline()))=>result?,};
        self.check(deadline, cancel)?;
        match source {
            PreparationSource::Restore(saved) => {
                if actual.metadata().inspection_hash != saved.metadata().inspection_hash {
                    return Err(ArtifactError::Inspection.into());
                }
                Ok(saved.as_ref().clone())
            }
            PreparationSource::Reference { .. } => Ok(actual),
        }
    }

    async fn stage(
        &mut self,
        source: &PreparationSource,
        overrides: Option<&ImageConfig>,
        deadline: Instant,
        cancel: &mut watch::Receiver<()>,
    ) -> Result<(ResolvedImage, UnpackedImageSize), DockerError> {
        let image = self
            .resolve_source(source, overrides, deadline, cancel)
            .await?;
        let resolver = ImageResolver::new(self.docker.clone());
        let mut stop = self.control.subscribe();
        self.check(deadline, cancel)?;
        let expected = image.metadata().image_id.clone();
        self.progress
            .phase
            .send_replace(Phase::Preparing(expected.clone()));
        self.pin
            .directory()
            .prepare_image(
                ImageRecord::new(&self.pin, &expected)?,
                self.config.deadline(),
            )
            .await?;
        self.check(deadline, cancel)?;
        let mutation = MutationGuard::new();
        self.progress
            .phase
            .send_replace(Phase::Uncertain(expected.clone(), mutation.receipt()));
        let tagged = self
            .docker
            .tag_image_pin_tracked(&expected, &self.pin, self.config.deadline(), mutation)
            .await;
        if matches!(
            &tagged,
            Err(DockerError::Busy
                | DockerError::Status(StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND))
        ) {
            self.progress
                .phase
                .send_replace(Phase::NoEffect(expected.clone()));
        }
        tagged?;
        self.persist_commit(&expected).await?;
        self.check(deadline, cancel)?;
        let actual = tokio::select! {biased;()=stopped(&mut stop)=>return Err(DockerError::Stopped),_ = cancel.changed()=>return Err(DockerError::Cancelled),result=self.docker.inspect_image(self.pin.reference(),deadline.min(self.config.deadline()))=>result?,};
        if actual.id.as_deref() != Some(expected.as_str()) {
            return Err(DockerError::Quarantined);
        }
        let size = tokio::select! {biased;()=stopped(&mut stop)=>return Err(DockerError::Stopped),_ = cancel.changed()=>return Err(DockerError::Cancelled),result=resolver.unpacked_size(&image,deadline.min(self.config.deadline()))=>result?,};
        self.check(deadline, cancel)?;
        Ok((image, size))
    }
    async fn retire(self) {
        self.progress.active.store(false, Ordering::Release);
        let mut dirty = matches!(
            *self.progress.phase.borrow(),
            Phase::Uncertain(..) | Phase::Committing(_)
        )
        .then(|| Dirty::new(self.health.clone()));
        let mut delay = Duration::from_secs(1);
        loop {
            if self.cleanup_step().await {
                self.progress.phase.send_replace(Phase::Clean);
                return;
            }
            if dirty.is_none() {
                dirty = Some(Dirty::new(self.health.clone()));
            }
            sleep(delay).await;
            delay = delay.saturating_mul(2).min(Duration::from_secs(30));
        }
    }
    async fn cleanup_step(&self) -> bool {
        let phase = self.progress.phase.borrow().clone();
        let (expected, known) = match phase {
            Phase::Unissued | Phase::Clean => return true,
            Phase::Preparing(id) | Phase::NoEffect(id) => return self.erase(&id).await,
            Phase::Committing(id) => {
                if self.persist_commit(&id).await.is_err() {
                    return false;
                }
                (id, true)
            }
            Phase::Known(id) => (id, true),
            Phase::Uncertain(id, receipt) => {
                if receipt.definitely_unissued() {
                    return self.erase(&id).await;
                }
                (id, false)
            }
        };
        let Ok(_cleanup) = self.cleanup.acquire().await else {
            return false;
        };
        let inspected = self
            .docker
            .inspect_image_pin(&self.pin, self.config.deadline())
            .await;
        match inspected {
            Err(DockerError::Status(StatusCode::NOT_FOUND)) => known && self.erase(&expected).await,
            Ok(image) if image.id.as_deref() == Some(expected.as_str()) => {
                if self.persist_commit(&expected).await.is_err() {
                    return false;
                }
                if !matches!(self.docker.inspect_image_pin(&self.pin, self.config.deadline()).await,
                    Ok(image) if image.id.as_deref() == Some(expected.as_str()))
                {
                    return false;
                }
                let _ = self
                    .docker
                    .remove_image_pin(&self.pin, self.config.deadline())
                    .await;
                matches!(
                    self.docker
                        .inspect_image_pin(&self.pin, self.config.deadline())
                        .await,
                    Err(DockerError::Status(StatusCode::NOT_FOUND))
                ) && self.erase(&expected).await
            }
            _ => false,
        }
    }
    async fn persist_commit(&self, expected: &Sha256Digest) -> Result<(), DockerError> {
        self.progress
            .phase
            .send_replace(Phase::Committing(expected.clone()));
        self.pin
            .directory()
            .commit_image(
                ImageRecord::new(&self.pin, expected)?,
                self.config.deadline(),
            )
            .await?;
        self.progress
            .phase
            .send_replace(Phase::Known(expected.clone()));
        Ok(())
    }
    async fn erase(&self, expected: &Sha256Digest) -> bool {
        let Ok(record) = ImageRecord::new(&self.pin, expected) else {
            return false;
        };
        self.pin
            .directory()
            .settle_image(record, self.config.deadline())
            .await
            .is_ok()
    }
}

#[cfg(test)]
#[path = "lease_tests.rs"]
pub(crate) mod tests;
