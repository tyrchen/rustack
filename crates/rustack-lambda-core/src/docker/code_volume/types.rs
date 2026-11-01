//! Validated code ownership inputs and revocable metadata-only Ready leases.

use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use tokio::{
    sync::{OwnedSemaphorePermit, oneshot, watch},
    time::{Instant, timeout_at},
};
use typed_builder::TypedBuilder;
use uuid::Uuid;

use crate::{
    artifact::{ImageLiveness, Sha256Digest},
    docker::{
        DockerError, DockerKey, check_deadline,
        models::{Mount, MountType, MountVolumeOptions},
    },
};

/// Immutable identities shared by the code volume and its eventual environment.
#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CodeVolumeIdentity {
    pub(super) environment: Uuid,
    pub(super) resource: Uuid,
    pub(super) revision: Uuid,
}
impl CodeVolumeIdentity {
    /// Validate resource/revision UUIDs and allocate a fresh environment identity.
    ///
    /// # Errors
    /// Returns Configuration for a nil identity.
    ///
    /// ```
    /// let id = rustack_lambda_core::docker::CodeVolumeIdentity::new(uuid::Uuid::new_v4(),uuid::Uuid::new_v4())?;
    /// # Ok::<(),rustack_lambda_core::docker::DockerError>(())
    /// ```
    pub fn new(resource: Uuid, revision: Uuid) -> Result<Self, DockerError> {
        if [resource, revision].iter().any(Uuid::is_nil) {
            return Err(DockerError::Configuration);
        }
        Ok(Self {
            environment: Uuid::new_v4(),
            resource,
            revision,
        })
    }
    /// Environment UUID used in volume/loader names.
    ///
    /// ```no_run
    /// # fn inspect(id:rustack_lambda_core::docker::CodeVolumeIdentity) { assert!(!id.environment_id().is_nil()); }
    /// ```
    #[must_use]
    pub fn environment_id(&self) -> Uuid {
        self.environment
    }
}

/// Independent bounded resource, preparation and wire budgets.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct CodeVolumeConfig {
    pub(super) volumes: u8,
    pub(super) jobs: u8,
    pub(super) io: Duration,
    pub(super) upload: Duration,
}
impl Default for CodeVolumeConfig {
    fn default() -> Self {
        Self {
            volumes: 32,
            jobs: 2,
            io: Duration::from_secs(5),
            upload: Duration::from_secs(60),
        }
    }
}
impl CodeVolumeConfig {
    /// Validate volume1..32/job1..2 admission and IO1..60s/upload1..600s.
    ///
    /// # Errors
    /// Returns Configuration for out-of-range or jobs-exceed-volumes settings.
    ///
    /// ```
    /// let config=rustack_lambda_core::docker::CodeVolumeConfig::new(32,2,std::time::Duration::from_secs(5),std::time::Duration::from_secs(60))?;
    /// # Ok::<(),rustack_lambda_core::docker::DockerError>(())
    /// ```
    pub fn new(volumes: u8, jobs: u8, io: Duration, upload: Duration) -> Result<Self, DockerError> {
        if !(1..=32).contains(&volumes)
            || !(1..=2).contains(&jobs)
            || jobs > volumes
            || !(Duration::from_secs(1)..=Duration::from_secs(60)).contains(&io)
            || !(Duration::from_secs(1)..=Duration::from_secs(600)).contains(&upload)
        {
            return Err(DockerError::Configuration);
        }
        Ok(Self {
            volumes,
            jobs,
            io,
            upload,
        })
    }
    pub(super) fn deadline(self) -> Instant {
        Instant::now() + self.io
    }
}

/// Retained ownership counts, independent of Docker observation/admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct CodeVolumeStatus {
    /// Queued, preparing, live or quarantined volume reservations.
    pub volumes: usize,
    /// Preparation reservations retained until consumption or physical cleanup.
    pub preparing: usize,
    /// Reapers retaining unresolved physical/durable effects.
    pub quarantined: usize,
    /// Whether the owner permits new preparations (capacity may still be full).
    pub accepting: bool,
    /// A permanent supervision failure closed this owner.
    pub worker_failed: bool,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum Consumption {
    Pending,
    Consumed(Instant),
    Expired,
}
#[derive(Debug)]
pub(super) struct Handoff {
    pub(super) state: Consumption,
    pub(super) slot: Option<OwnedSemaphorePermit>,
}
pub(super) type Acceptance = watch::Sender<Handoff>;

/// Metadata-only physical/journal retirement observation; it owns no resources.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CodeRetirement {
    settled: watch::Receiver<bool>,
}
impl CodeRetirement {
    pub(super) fn new(settled: watch::Receiver<bool>) -> Self {
        Self { settled }
    }
    /// Observe the same cleanup without cancelling or restarting it.
    ///
    /// # Errors
    /// Returns Deadline on observer expiry, or Worker if ownership vanished
    /// without a positive retirement proof.
    ///
    /// ```no_run
    /// # async fn observe(mut receipt:rustack_lambda_core::docker::CodeRetirement)->Result<(),rustack_lambda_core::docker::DockerError> {receipt.wait(tokio::time::Instant::now()+std::time::Duration::from_secs(30)).await?;Ok(())}
    /// ```
    pub async fn wait(&mut self, deadline: Instant) -> Result<(), DockerError> {
        check_deadline(deadline)?;
        let result = timeout_at(deadline, async {
            loop {
                if *self.settled.borrow() {
                    return Ok(());
                }
                if self.settled.changed().await.is_err() {
                    return if *self.settled.borrow() {
                        Ok(())
                    } else {
                        Err(DockerError::Worker)
                    };
                }
            }
        })
        .await
        .map_err(|_| DockerError::Deadline)?;
        check_deadline(deadline)?;
        result
    }
}

/// Accepted result/cancellation observation with a separate physical receipt.
#[derive(Debug)]
#[non_exhaustive]
pub struct CodePreparation {
    pub(super) result: oneshot::Receiver<Result<PinnedCode, DockerError>>,
    pub(super) _cancel: watch::Sender<()>,
    pub(super) retired: CodeRetirement,
    pub(super) deadline: Instant,
}
impl CodePreparation {
    /// Retain cleanup evidence before observing a result which may fail/expire.
    ///
    /// ```no_run
    /// # fn inspect(preparation:&rustack_lambda_core::docker::CodePreparation) {let retired=preparation.retirement();}
    /// ```
    #[must_use]
    pub fn retirement(&self) -> CodeRetirement {
        self.retired.clone()
    }
    /// Consume an on-time Ready result; dropping this observer requests cancel.
    ///
    /// # Errors
    /// Returns the preparation failure, expiry, cancellation or supervisor error.
    /// None proves physical retirement; observe the separate receipt for that.
    ///
    /// ```no_run
    /// # async fn ready(preparation:rustack_lambda_core::docker::CodePreparation)->Result<(),rustack_lambda_core::docker::DockerError> {let code=preparation.wait().await?;assert!(code.is_live());Ok(())}
    /// ```
    pub async fn wait(mut self) -> Result<PinnedCode, DockerError> {
        let ready = timeout_at(self.deadline, &mut self.result)
            .await
            .map_err(|_| DockerError::Deadline)?
            .map_err(|_| DockerError::Worker)??;
        ready.accept()?;
        Ok(ready)
    }
}

/// Revocable code metadata; source and physical admission live in the actor.
#[derive(Clone)]
#[non_exhaustive]
pub struct PinnedCode(Arc<CodeLease>);
#[derive(Debug, TypedBuilder)]
pub(super) struct CodeLease {
    identity: Arc<CodeVolumeIdentity>,
    volume: DockerKey,
    image: Sha256Digest,
    source: ImageLiveness,
    release: watch::Sender<()>,
    active: Arc<AtomicBool>,
    stop: watch::Receiver<bool>,
    handoff: Acceptance,
    deadline: Instant,
    retired: watch::Receiver<bool>,
}
impl Drop for CodeLease {
    fn drop(&mut self) {
        self.release.send_replace(());
    }
}
impl fmt::Debug for PinnedCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PinnedCode")
            .field("live", &self.is_live())
            .finish_non_exhaustive()
    }
}
impl PinnedCode {
    pub(super) fn from_lease(lease: CodeLease) -> Self {
        Self(Arc::new(lease))
    }
    pub(super) fn accept(&self) -> Result<(), DockerError> {
        let mut accepted = false;
        self.0.handoff.send_modify(|handoff| {
            let at = Instant::now();
            if matches!(handoff.state, Consumption::Pending)
                && at < self.0.deadline
                && self.is_live()
            {
                handoff.state = Consumption::Consumed(at);
                handoff.slot.take();
                accepted = true;
            }
        });
        if accepted {
            Ok(())
        } else if Instant::now() >= self.0.deadline {
            Err(DockerError::Deadline)
        } else {
            Err(DockerError::Stopped)
        }
    }
    /// Whether this code and its source/manager still permit consumption.
    ///
    /// ```no_run
    /// # fn inspect(code:&rustack_lambda_core::docker::PinnedCode) { let live=code.is_live(); }
    /// ```
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.0.active.load(Ordering::Acquire) && !*self.0.stop.borrow() && self.0.source.is_live()
    }
    /// Exact immutable environment/resource/revision identity.
    ///
    /// ```no_run
    /// # fn inspect(code:&rustack_lambda_core::docker::PinnedCode) { let id=code.identity(); }
    /// ```
    #[must_use]
    pub fn identity(&self) -> &CodeVolumeIdentity {
        &self.0.identity
    }
    /// Immutable parent image ID whose pin the actor retains through retirement.
    ///
    /// ```no_run
    /// # fn inspect(code:&rustack_lambda_core::docker::PinnedCode) { let id=code.base_image_id(); }
    /// ```
    #[must_use]
    pub fn base_image_id(&self) -> &Sha256Digest {
        &self.0.image
    }
    /// Named volume for diagnostics; observation does not confer ownership.
    ///
    /// ```no_run
    /// # fn inspect(code:&rustack_lambda_core::docker::PinnedCode) { let name=code.volume_name(); }
    /// ```
    #[must_use]
    pub fn volume_name(&self) -> &DockerKey {
        &self.0.volume
    }
    /// Observe retirement after releasing all Ready clones and their consumers.
    ///
    /// ```no_run
    /// # async fn release(code:rustack_lambda_core::docker::PinnedCode)->Result<(),rustack_lambda_core::docker::DockerError> {let mut retired=code.retirement();drop(code);retired.wait(tokio::time::Instant::now()+std::time::Duration::from_secs(30)).await?;Ok(())}
    /// ```
    #[must_use]
    pub fn retirement(&self) -> CodeRetirement {
        CodeRetirement::new(self.0.retired.clone())
    }

    /// Return only the checked RO code mount, retaining this lease in its consumer.
    ///
    /// # Errors
    /// Returns Stopped if this lease, manager or source has been revoked.
    ///
    /// ```no_run
    /// # fn inspect(code:&rustack_lambda_core::docker::PinnedCode)->Result<(),rustack_lambda_core::docker::DockerError> { let mount=code.mount()?;assert_eq!(mount.read_only,Some(true));Ok(()) }
    /// ```
    pub fn mount(&self) -> Result<Mount, DockerError> {
        if !self.is_live() {
            return Err(DockerError::Stopped);
        }
        Ok(Mount {
            typ: Some(MountType::VOLUME),
            source: Some(self.0.volume.as_str().into()),
            target: Some("/var/task".into()),
            read_only: Some(true),
            volume_options: Some(MountVolumeOptions {
                no_copy: Some(true),
                ..MountVolumeOptions::default()
            }),
            ..Mount::default()
        })
    }
}
