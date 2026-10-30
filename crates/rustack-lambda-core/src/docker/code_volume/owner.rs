//! Bounded producer actor and a transferred, non-aborting cleanup ledger.

use std::{
    collections::HashMap,
    fmt, mem,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use base64::{Engine, engine::general_purpose::STANDARD};
use tokio::{
    runtime::Handle,
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch},
    task::{Id, JoinError, JoinHandle, JoinSet},
    time::{Instant, sleep_until, timeout_at},
};
use typed_builder::TypedBuilder;
use uuid::Uuid;

use super::{
    CodePreparation, CodeRetirement, CodeVolumeConfig, CodeVolumeIdentity, CodeVolumeStatus,
    PinnedCode,
    entry::{Context, Entry, Health, Plan, Progress},
    stage,
    types::{CodeLease, Consumption, Handoff},
};
use crate::{
    artifact::PinnedImage,
    docker::{
        DirectoryLease, DockerCodeArchive, DockerError, DockerPort, check_deadline,
        connection::stopped,
        ownership::codes::{CodeRecord, LoaderState, VolumeState},
    },
    invocation::CleanupTask,
};

#[derive(Debug)]
struct NoEffectRetirement(Option<watch::Sender<bool>>);
impl Drop for NoEffectRetirement {
    fn drop(&mut self) {
        if let Some(retired) = &self.0 {
            retired.send_replace(true);
        }
    }
}

#[derive(Debug, TypedBuilder)]
struct Envelope {
    retirement: NoEffectRetirement,
    identity: CodeVolumeIdentity,
    archive: DockerCodeArchive,
    base: PinnedImage,
    deadline: Instant,
    cancel: watch::Receiver<()>,
    reply: oneshot::Sender<Result<PinnedCode, DockerError>>,
    volume: OwnedSemaphorePermit,
    handoff: watch::Sender<Handoff>,
}
/// Cloneable bounded code preparation port; retain the manager through cleanup.
#[derive(Debug, Clone, TypedBuilder)]
#[non_exhaustive]
pub struct CodeVolumePort {
    sender: mpsc::Sender<Envelope>,
    slots: Arc<Semaphore>,
    volumes: Arc<Semaphore>,
    stop: watch::Sender<bool>,
    health: Arc<Health>,
    directory: DirectoryLease,
    config: CodeVolumeConfig,
}
impl CodeVolumePort {
    #[cfg(test)]
    pub(super) fn set_fault(&self, point: u8) {
        self.health.fault.store(point, Ordering::Release);
    }
    #[cfg(test)]
    pub(super) fn ready_handoffs(&self) -> usize {
        self.health.ready.load(Ordering::Acquire)
    }

    /// Observe retained reservations without Docker IO or mailbox admission.
    ///
    /// ```no_run
    /// # fn inspect(port:&rustack_lambda_core::docker::CodeVolumePort) {let status=port.status();}
    /// ```
    #[must_use]
    pub fn status(&self) -> CodeVolumeStatus {
        let worker_failed = self.health.failed.load(Ordering::Acquire);
        let quarantined = self.health.quarantined.load(Ordering::Acquire);
        CodeVolumeStatus {
            volumes: usize::from(self.config.volumes)
                .saturating_sub(self.volumes.available_permits()),
            preparing: usize::from(self.config.jobs).saturating_sub(self.slots.available_permits()),
            quarantined,
            worker_failed,
            accepting: !worker_failed && quarantined == 0 && !*self.stop.borrow(),
        }
    }
    /// Prepare typed code under the exact source-pin acquisition and deadline.
    ///
    /// Dropping or expiring the observer leaves accepted work and cleanup owned.
    /// The returned lease must outlive every known/uncertain mount consumer.
    ///
    /// # Errors
    /// Returns invalid identity/source/budgets, bounded admission, IO or staging
    /// failures. An error does not establish physical retirement.
    ///
    /// ```no_run
    /// # async fn prepare(port:&rustack_lambda_core::docker::CodeVolumePort,code:rustack_lambda_core::docker::DockerCodeArchive,base:rustack_lambda_core::artifact::PinnedImage)->Result<(),rustack_lambda_core::docker::DockerError> {
    /// let id=rustack_lambda_core::docker::CodeVolumeIdentity::new(uuid::Uuid::new_v4(),uuid::Uuid::new_v4())?;
    /// let lease=port.prepare(id,code,base,tokio::time::Instant::now()+std::time::Duration::from_secs(120)).await?;
    /// assert_eq!(lease.mount()?.read_only,Some(true));
    /// # Ok(()) }
    /// ```
    #[tracing::instrument(skip_all)]
    pub async fn prepare(
        &self,
        identity: CodeVolumeIdentity,
        archive: DockerCodeArchive,
        base: PinnedImage,
        deadline: Instant,
    ) -> Result<PinnedCode, DockerError> {
        self.start_preparation(identity, archive, base, deadline)?
            .wait()
            .await
    }
    /// Accept preparation and expose retirement before its fallible result.
    ///
    /// Parent environments retain their own admission until the retirement
    /// receipt succeeds, including when result observation fails or expires.
    ///
    /// # Errors
    /// Returns source, budget, stopped or bounded admission failures before any
    /// code effect can be issued; a returned handle owns accepted observation.
    ///
    /// ```no_run
    /// # async fn start(port:&rustack_lambda_core::docker::CodeVolumePort,id:rustack_lambda_core::docker::CodeVolumeIdentity,code:rustack_lambda_core::docker::DockerCodeArchive,base:rustack_lambda_core::artifact::PinnedImage)->Result<(),rustack_lambda_core::docker::DockerError> {
    /// let preparation=port.start_preparation(id,code,base,tokio::time::Instant::now()+std::time::Duration::from_secs(120))?;
    /// let mut retired=preparation.retirement();
    /// let result=preparation.wait().await;
    /// drop(result);
    /// retired.wait(tokio::time::Instant::now()+std::time::Duration::from_secs(30)).await?;
    /// # Ok(()) }
    /// ```
    pub fn start_preparation(
        &self,
        identity: CodeVolumeIdentity,
        archive: DockerCodeArchive,
        base: PinnedImage,
        deadline: Instant,
    ) -> Result<CodePreparation, DockerError> {
        self.start_with_receipt(identity, archive, base, deadline, |_| {})
    }
    pub(in crate::docker) fn same_scope(&self, directory: &DirectoryLease) -> bool {
        self.directory.same_instance(directory)
    }
    pub(in crate::docker) fn live(&self) -> bool {
        !*self.stop.borrow()
            && !self.health.failed.load(Ordering::Acquire)
            && !self.sender.is_closed()
    }
    pub(in crate::docker) fn start_with_receipt<F>(
        &self,
        identity: CodeVolumeIdentity,
        archive: DockerCodeArchive,
        base: PinnedImage,
        deadline: Instant,
        publish: F,
    ) -> Result<CodePreparation, DockerError>
    where
        F: FnOnce(CodeRetirement),
    {
        self.health.check()?;
        if *self.stop.borrow() {
            return Err(DockerError::Stopped);
        }
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|v| !v.is_zero())
            .ok_or(DockerError::Deadline)?;
        if remaining > Duration::from_secs(3600)
            || !base.belongs_to(&self.directory)
            || !base.is_live()
            || !base.liveness().is_live()
            || archive.architecture() != base.resolved().metadata().architecture
        {
            return Err(DockerError::Configuration);
        }
        let slot = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| DockerError::Busy)?;
        let volume = self
            .volumes
            .clone()
            .try_acquire_owned()
            .map_err(|_| DockerError::Busy)?;
        let (handoff, _) = watch::channel(Handoff {
            state: Consumption::Pending,
            slot: Some(slot),
        });
        let (cancel, cancelled) = watch::channel(());
        let (reply, response) = oneshot::channel();
        let (retirement, settled) = watch::channel(false);
        let retired = CodeRetirement::new(settled);
        let request = Envelope::builder()
            .retirement(NoEffectRetirement(Some(retirement)))
            .identity(identity)
            .archive(archive)
            .base(base)
            .deadline(deadline)
            .cancel(cancelled)
            .reply(reply)
            .volume(volume)
            .handoff(handoff)
            .build();
        // Publication cannot race accepted work: the envelope is still local.
        publish(retired.clone());
        self.sender.try_send(request).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => DockerError::Busy,
            mpsc::error::TrySendError::Closed(_) => DockerError::Stopped,
        })?;
        Ok(CodePreparation {
            result: response,
            _cancel: cancel,
            retired,
            deadline,
        })
    }
}
/// Retained producer ownership; stop after all code mount consumers have joined.
#[non_exhaustive]
pub struct CodeVolumeManager {
    port: CodeVolumePort,
    task: Option<JoinHandle<Result<(), DockerError>>>,
    primary: Option<Result<(), DockerError>>,
    emergency: Arc<CleanupTask>,
    settlement: Option<Result<(), DockerError>>,
}
impl fmt::Debug for CodeVolumeManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CodeVolumeManager")
            .field("status", &self.port.status())
            .finish_non_exhaustive()
    }
}
impl CodeVolumeManager {
    pub(in crate::docker) fn physically_settled(&self) -> bool {
        self.settlement.is_some() && self.port.health.physical.load(Ordering::Acquire)
    }

    /// Start one producing owner for this acquired code-generation directory.
    ///
    /// Historical namespace recovery must precede production. No readiness or
    /// ordinary ZIP capability is granted by this library constructor.
    ///
    /// # Errors
    /// Rejects missing runtime, foreign/unbound ports, wrong provenance or a
    /// previous producer still retaining this acquisition.
    ///
    /// ```no_run
    /// # fn start(port:rustack_lambda_core::docker::DockerPort,directory:rustack_lambda_core::docker::DirectoryLease)->Result<(),rustack_lambda_core::docker::DockerError> {
    /// let manager=rustack_lambda_core::docker::CodeVolumeManager::start(port,directory,Default::default())?;
    /// # Ok(()) }
    /// ```
    pub fn start(
        docker: DockerPort,
        directory: DirectoryLease,
        config: CodeVolumeConfig,
    ) -> Result<Self, DockerError> {
        docker.require_directory(&directory)?;
        let runtime = Handle::try_current().map_err(|_| DockerError::Worker)?;
        let producer = directory.code_producer()?;
        let (sender, inbox) = mpsc::channel(32);
        let (stop, _) = watch::channel(false);
        let health = Arc::new(Health::default());
        let emergency = Arc::new(CleanupTask::default());
        let port = CodeVolumePort::builder()
            .sender(sender)
            .slots(Arc::new(Semaphore::new(usize::from(config.jobs))))
            .volumes(Arc::new(Semaphore::new(usize::from(config.volumes))))
            .stop(stop.clone())
            .health(health.clone())
            .directory(directory.clone())
            .config(config)
            .build();
        let context = Arc::new(
            Context::builder()
                .docker(docker.owned())
                .directory(directory)
                .config(config)
                .stop(stop)
                .health(health)
                .runtime(runtime.clone())
                .cleanup(Semaphore::new(2))
                .producer(producer)
                .build(),
        );
        let actor = Actor {
            ledger: Ledger::new(context),
            inbox,
            emergency: emergency.clone(),
            settled: false,
        };
        let task = runtime.spawn(actor.run());
        Ok(Self {
            port,
            task: Some(task),
            primary: None,
            emergency,
            settlement: None,
        })
    }
    #[cfg(test)]
    pub(super) fn abort_root(&self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }

    /// Clone a preparation port without transferring manager shutdown ownership.
    ///
    /// ```no_run
    /// # fn inspect(manager:&rustack_lambda_core::docker::CodeVolumeManager) {let port=manager.port();}
    /// ```
    #[must_use]
    pub fn port(&self) -> CodeVolumePort {
        self.port.clone()
    }
    /// Revoke admission/leases and join original stages and all physical cleanup.
    ///
    /// First join every known/uncertain function and its IO, then stop this
    /// manager, then image ownership and the Docker wire. An expired observer
    /// retains the same join task and all outstanding resource reservations.
    ///
    /// # Errors
    /// Returns deadline while retaining work, or cached supervision failure.
    ///
    /// ```no_run
    /// # async fn stop(manager:&mut rustack_lambda_core::docker::CodeVolumeManager) {let result=manager.shutdown(tokio::time::Instant::now()+std::time::Duration::from_secs(30)).await;}
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
impl Drop for CodeVolumeManager {
    fn drop(&mut self) {
        self.port.stop.send_replace(true);
    }
}

type Request = (
    watch::Sender<()>,
    oneshot::Sender<Result<PinnedCode, DockerError>>,
);

type Job = (Uuid, Result<(), DockerError>);
type Joined = Result<(Id, Job), JoinError>;
#[derive(Debug)]
struct Ledger {
    context: Arc<Context>,
    entries: HashMap<Uuid, Arc<Entry>>,
    jobs: JoinSet<Job>,
    index: HashMap<Id, Uuid>,
}
impl Ledger {
    fn new(context: Arc<Context>) -> Self {
        Self {
            context,
            entries: HashMap::new(),
            jobs: JoinSet::new(),
            index: HashMap::new(),
        }
    }
    fn spawn(&mut self, entry: Arc<Entry>, request: Option<Request>) {
        let id = entry.plan.identity.environment;
        let context = self.context.clone();
        let guard = WorkerGuard::new(entry.clone(), context.clone());
        let job = self.jobs.spawn(async move {
            let mut guard = guard;
            if let Some((release, reply)) = request {
                run(entry, context, release, reply).await;
            } else {
                tokio::time::sleep(Duration::from_secs(1)).await;
                entry.retire(&context).await;
            }
            guard.complete = true;
            (id, Ok(()))
        });
        self.index.insert(job.id(), id);
    }
    fn finished(&mut self, result: &Joined) {
        let task = match &result {
            Ok((id, _)) => *id,
            Err(error) => error.id(),
        };
        let indexed = self.index.remove(&task);
        let returned = match &result {
            Ok((_, (id, _))) => Some(*id),
            Err(_) => None,
        };
        let Some(id) = indexed.or(returned) else {
            self.context.fail();
            return;
        };
        let Some(entry) = self.entries.get(&id).cloned() else {
            self.context.fail();
            return;
        };
        if matches!(result,Ok((_,(returned,Ok(())))) if *returned==id)
            && *entry.progress.clean.borrow()
        {
            self.entries.remove(&id);
        } else {
            self.context.fail();
            self.spawn(entry, None);
        }
    }
    fn schedule_missing(&mut self) {
        let missing = self
            .entries
            .iter()
            .filter(|(id, entry)| {
                !entry.progress.running.load(Ordering::Acquire)
                    && !self.index.values().any(|mapped| mapped == *id)
            })
            .map(|(_, entry)| entry.clone())
            .collect::<Vec<_>>();
        for entry in missing {
            self.spawn(entry, None);
        }
    }
    async fn drain(&mut self) {
        loop {
            if self.jobs.is_empty() {
                self.index.clear();
            }
            self.schedule_missing();
            if self.jobs.is_empty() {
                self.index.clear();
                if self.entries.is_empty() {
                    self.context.producer.settle();
                    self.context.health.physical.store(true, Ordering::Release);
                    return;
                }
                for entry in self.entries.values().cloned().collect::<Vec<_>>() {
                    self.spawn(entry, None);
                }
            }
            if let Some(result) = self.jobs.join_next_with_id().await {
                self.finished(&result);
            }
        }
    }
}
#[derive(Debug)]
struct Actor {
    ledger: Ledger,
    inbox: mpsc::Receiver<Envelope>,
    emergency: Arc<CleanupTask>,
    settled: bool,
}
impl Actor {
    async fn run(mut self) -> Result<(), DockerError> {
        let mut stop = self.ledger.context.stop.subscribe();
        let mut wire_stop = self.ledger.context.docker.raw.stop.subscribe();
        loop {
            if *stop.borrow() {
                self.inbox.close();
                while let Ok(request) = self.inbox.try_recv() {
                    let _ = request.reply.send(Err(DockerError::Stopped));
                }
                self.ledger.drain().await;
                self.settled = true;
                return if self.ledger.context.health.failed.load(Ordering::Acquire) {
                    Err(DockerError::Worker)
                } else {
                    Ok(())
                };
            }
            tokio::select! {biased;
                ()=stopped(&mut wire_stop)=>self.ledger.context.fail(),
                ()=stopped(&mut stop)=>{},
                result=self.ledger.jobs.join_next_with_id(),if !self.ledger.jobs.is_empty()=>if let Some(result)=result {self.ledger.finished(&result);},
                request=self.inbox.recv()=>match request {Some(request)=>self.start(request),None=>{self.ledger.context.stop.send_replace(true);}},
            }
        }
    }
    fn start(&mut self, mut request: Envelope) {
        let id = request.identity.environment;
        if self.ledger.context.health.check().is_err()
            || *self.ledger.context.stop.borrow()
            || request.reply.is_closed()
            || request.cancel.has_changed().is_err()
            || Instant::now() >= request.deadline
            || self.ledger.entries.contains_key(&id)
        {
            let _ = request.reply.send(Err(DockerError::Stopped));
            return;
        }
        let record = match record(&request, &self.ledger.context.directory) {
            Ok(record) => record,
            Err(error) => {
                let _ = request.reply.send(Err(error));
                return;
            }
        };
        let Some(retirement) = request.retirement.0.take() else {
            self.ledger.context.fail();
            let _ = request.reply.send(Err(DockerError::Worker));
            return;
        };
        let (archive, _) = watch::channel(Some(request.archive));
        let (outcome, _) = watch::channel(None);
        let (release, released) = watch::channel(());
        let entry = Arc::new(
            Entry::builder()
                .plan(
                    Plan::builder()
                        .identity(Arc::new(request.identity))
                        .base(request.base)
                        .archive(archive)
                        .record(record)
                        .deadline(request.deadline)
                        .cancel(request.cancel)
                        .build(),
                )
                .progress(Progress::new(retirement))
                .stage(CleanupTask::default())
                .outcome(outcome)
                .handoff(request.handoff)
                .released(released)
                ._volume(request.volume)
                .build(),
        );
        self.ledger.entries.insert(id, entry.clone());
        self.ledger.spawn(entry, Some((release, request.reply)));
        #[cfg(test)]
        self.ledger.context.fault(5);
    }
}
impl Drop for Actor {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        self.ledger.context.fail();
        if self.ledger.entries.is_empty() && self.ledger.jobs.is_empty() {
            self.ledger.context.producer.settle();
            self.ledger
                .context
                .health
                .physical
                .store(true, Ordering::Release);
            return;
        }
        // Move every original worker; never abort IO in order to replace its observer.
        let context = self.ledger.context.clone();
        let mut ledger = mem::replace(&mut self.ledger, Ledger::new(context));
        self.emergency
            .start_on(&ledger.context.runtime.clone(), move || async move {
                ledger.drain().await;
                Ok(())
            });
    }
}
#[derive(Debug)]
struct WorkerGuard {
    entry: Arc<Entry>,
    context: Arc<Context>,
    complete: bool,
}
impl WorkerGuard {
    fn new(entry: Arc<Entry>, context: Arc<Context>) -> Self {
        entry.progress.running.store(true, Ordering::Release);
        Self {
            entry,
            context,
            complete: false,
        }
    }
}
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        self.entry.progress.running.store(false, Ordering::Release);
        if !self.complete {
            self.context.fail();
        }
    }
}

fn record(request: &Envelope, directory: &DirectoryLease) -> Result<CodeRecord, DockerError> {
    let digest = |text: &str| -> Result<[u8; 32], DockerError> {
        let mut result = [0; 32];
        hex::decode_to_slice(
            text.strip_prefix("sha256:")
                .ok_or(DockerError::Configuration)?,
            &mut result,
        )
        .map_err(|_| DockerError::Configuration)?;
        Ok(result)
    };
    let zip = STANDARD
        .decode(request.archive.code_sha256())
        .map_err(|_| DockerError::InvalidCode)?
        .try_into()
        .map_err(|_| DockerError::InvalidCode)?;
    Ok(CodeRecord::builder()
        .run(directory.run_id())
        .environment(request.identity.environment)
        .resource(request.identity.resource)
        .revision(request.identity.revision)
        .zip(zip)
        .base(digest(
            request.base.resolved().metadata().image_id.as_str(),
        )?)
        .fingerprint(digest(
            request.base.resolved().metadata().inspection_hash.as_str(),
        )?)
        .volume(VolumeState::Planned)
        .loader(LoaderState::Planned)
        .build())
}
async fn run(
    entry: Arc<Entry>,
    context: Arc<Context>,
    release: watch::Sender<()>,
    reply: oneshot::Sender<Result<PinnedCode, DockerError>>,
) {
    #[cfg(test)]
    context.fault(1);
    stage::start(&entry, context.clone());
    let joined = entry.stage.wait().await;
    let staged = if joined.is_err() {
        context.fail();
        Err(DockerError::Worker)
    } else {
        entry
            .outcome
            .borrow()
            .clone()
            .unwrap_or(Err(DockerError::Worker))
    };
    #[cfg(test)]
    context.fault(2);
    let staged = staged.and_then(|()| entry.check(&context));
    match staged {
        Ok(()) => {
            entry.progress.active.store(true, Ordering::Release);
            let ready = CodeLease::builder()
                .identity(entry.plan.identity.clone())
                .volume(match entry.plan.record.volume_key() {
                    Ok(key) => key,
                    Err(error) => {
                        let _ = reply.send(Err(error));
                        entry.retire(&context).await;
                        return;
                    }
                })
                .image(entry.plan.base.resolved().metadata().image_id.clone())
                .source(entry.plan.base.liveness())
                .release(release)
                .active(entry.progress.active.clone())
                .stop(context.stop.subscribe())
                .handoff(entry.handoff.clone())
                .deadline(entry.plan.deadline)
                .retired(entry.progress.clean.subscribe())
                .build();
            if reply.send(Ok(PinnedCode::from_lease(ready))).is_ok()
                && consumed(&entry, &context).await
            {
                let mut released = entry.released.clone();
                let mut stop = context.stop.subscribe();
                let source = entry.plan.base.liveness();
                tokio::select! {biased;()=stopped(&mut stop)=>{},()=source.revoked()=>{},_ = released.changed()=>{}}
            }
        }
        Err(error) => {
            let _ = reply.send(Err(error));
        }
    }
    entry.retire(&context).await;
}
#[cfg(test)]
#[derive(Debug)]
struct ReadyProbe(Arc<Health>);
#[cfg(test)]
impl Drop for ReadyProbe {
    fn drop(&mut self) {
        self.0.ready.fetch_sub(1, Ordering::AcqRel);
    }
}

async fn consumed(entry: &Entry, context: &Context) -> bool {
    #[cfg(test)]
    let _probe = {
        context.health.ready.fetch_add(1, Ordering::AcqRel);
        ReadyProbe(context.health.clone())
    };
    let mut changed = entry.handoff.subscribe();
    let mut released = entry.released.clone();
    let mut cancelled = entry.plan.cancel.clone();
    let mut stop = context.stop.subscribe();
    let source = entry.plan.base.liveness();
    let mut abandoned = false;
    loop {
        let mut decision = None;
        entry
            .handoff
            .send_if_modified(|handoff| match handoff.state {
                Consumption::Consumed(at) if at < entry.plan.deadline => {
                    decision = Some(true);
                    false
                }
                Consumption::Expired => {
                    decision = Some(false);
                    false
                }
                _ if abandoned
                    || Instant::now() >= entry.plan.deadline
                    || *stop.borrow()
                    || !source.is_live()
                    || cancelled.has_changed().is_err() =>
                {
                    handoff.state = Consumption::Expired;
                    decision = Some(false);
                    true
                }
                _ => false,
            });
        if let Some(decision) = decision {
            return decision;
        }
        tokio::select! {biased;
            ()=stopped(&mut stop)=>{},()=source.revoked()=>{},_ = cancelled.changed()=>{},
            _ = released.changed()=>{abandoned=true;},_ = changed.changed()=>{},()=sleep_until(entry.plan.deadline)=>{},
        }
    }
}
