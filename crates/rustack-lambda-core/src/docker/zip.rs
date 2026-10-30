//! Provided-runtime sources and retained per-plan code preparation ownership.

#[cfg(test)]
use std::sync::atomic::AtomicU8;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use tokio::{
    runtime::Handle,
    sync::watch,
    time::{Instant, timeout_at},
};
use typed_builder::TypedBuilder;

use super::{
    CodeRetirement, CodeVolumeIdentity, CodeVolumePort, DirectoryLease, DockerCodeArchive,
    DockerError, DockerKey, PinnedCode, check_deadline, connection::stopped, models::Mount,
    ownership::codes::volume_key,
};
use crate::{
    artifact::{PinnedImage, ValidatedArtifact},
    invocation::CleanupTask,
};

/// Exact affiliated OS-image and code-owner inputs; production startup qualifies them.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DockerZipRuntime {
    pub(super) base: PinnedImage,
    pub(super) code: CodeVolumePort,
}
impl DockerZipRuntime {
    /// Bind a live source and producer to the exact code-profile acquisition.
    ///
    /// This performs no IO and confers no production readiness.
    ///
    /// # Errors
    /// Rejects foreign acquisitions, older profiles or unavailable ownership.
    ///
    /// ```no_run
    /// # fn source(directory:rustack_lambda_core::docker::DirectoryLease,base:rustack_lambda_core::artifact::PinnedImage,code:rustack_lambda_core::docker::CodeVolumePort)->Result<(),rustack_lambda_core::docker::DockerError>{
    /// let source=rustack_lambda_core::docker::DockerZipRuntime::new(&directory,base,code)?;
    /// # Ok(()) }
    /// ```
    pub fn new(
        directory: &DirectoryLease,
        base: PinnedImage,
        code: CodeVolumePort,
    ) -> Result<Self, DockerError> {
        let source = Self { base, code };
        if !directory.code_journaled() || !source.same_scope(directory) {
            return Err(DockerError::Configuration);
        }
        source.available()?;
        Ok(source)
    }
    pub(super) fn same_scope(&self, directory: &DirectoryLease) -> bool {
        self.base.belongs_to(directory) && self.code.same_scope(directory)
    }
    pub(super) fn live(&self) -> Result<(), DockerError> {
        if !self.base.liveness().is_live() || !self.code.live() {
            return Err(DockerError::Stopped);
        }
        Ok(())
    }
    pub(super) fn available(&self) -> Result<(), DockerError> {
        self.live()?;
        if !self.code.status().accepting {
            return Err(DockerError::Quarantined);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub(super) enum CodeStageError {
    #[error("code preparation admission saturated")]
    AdmissionBusy,
    #[error(transparent)]
    Docker(#[from] DockerError),
}

#[derive(Debug, Default)]
struct Preparation {
    ready: Option<PinnedCode>,
    receipt: Option<CodeRetirement>,
    outcome: Option<Result<(), CodeStageError>>,
}

#[derive(Debug, TypedBuilder)]
pub(super) struct ZipPlan {
    pub(super) source: DockerZipRuntime,
    identity: watch::Sender<Option<CodeVolumeIdentity>>,
    pub(super) volume: DockerKey,
    #[builder(default)]
    stage: CleanupTask,
    state: watch::Sender<Preparation>,
    #[builder(default)]
    claimed: AtomicBool,
    #[builder(default)]
    started: AtomicBool,
    #[builder(default = AtomicBool::new(true))]
    settled: AtomicBool,
    #[cfg(test)]
    #[builder(default)]
    fault: AtomicU8,
}
impl ZipPlan {
    #[cfg(test)]
    fn fault(&self, point: u8) {
        if self
            .fault
            .compare_exchange(point, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            panic!("deliberate ZIP stage fault");
        }
    }
    pub(super) fn new(
        source: DockerZipRuntime,
        identity: CodeVolumeIdentity,
    ) -> Result<Arc<Self>, DockerError> {
        let volume = volume_key(identity.environment_id())?;
        Ok(Arc::new(
            Self::builder()
                .source(source)
                .identity(watch::channel(Some(identity)).0)
                .volume(volume)
                .state(watch::channel(Preparation::default()).0)
                .build(),
        ))
    }
    pub(super) fn claim(self: &Arc<Self>) -> Result<AllocationClaim, DockerError> {
        self.claimed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| DockerError::Configuration)?;
        Ok(AllocationClaim {
            plan: self.clone(),
            committed: false,
        })
    }
    pub(super) fn live(&self) -> Result<(), DockerError> {
        self.source.live()?;
        if self.started.load(Ordering::Acquire) && self.clean() {
            return Err(DockerError::Stopped);
        }
        if self
            .state
            .borrow()
            .ready
            .as_ref()
            .is_some_and(|ready| !ready.is_live())
        {
            return Err(DockerError::Stopped);
        }
        Ok(())
    }
    pub(super) fn ready_mount(&self) -> Result<Mount, DockerError> {
        self.live()?;
        let state = self.state.borrow();
        let ready = state.ready.as_ref().ok_or(DockerError::Configuration)?;
        if ready.volume_name() != &self.volume
            || ready.base_image_id() != &self.source.base.resolved().metadata().image_id
        {
            return Err(DockerError::Quarantined);
        }
        ready.mount()
    }
    pub(super) fn clean(&self) -> bool {
        self.settled.load(Ordering::Acquire)
    }
    pub(super) fn start(
        self: &Arc<Self>,
        input: StageInput,
        runtime: &Handle,
    ) -> Result<(), DockerError> {
        self.started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| DockerError::Configuration)?;
        self.settled.store(false, Ordering::Release);
        let retained = self.clone();
        self.stage.start_on(runtime, move || async move {
            let outcome = retained.prepare(input).await;
            retained
                .state
                .send_modify(|state| state.outcome = Some(outcome));
            Ok(())
        });
        Ok(())
    }
    async fn prepare(&self, mut input: StageInput) -> Result<(), CodeStageError> {
        input.check()?;
        self.source.available()?;
        let mut identity = None;
        self.identity.send_modify(|slot| identity = slot.take());
        let identity = identity.ok_or(DockerError::Configuration)?;
        let archive = DockerCodeArchive::prepare(
            input.artifact.clone(),
            self.source.base.resolved().metadata().architecture,
            input.deadline,
        )
        .await?;
        input.check()?;
        #[cfg(test)]
        self.fault(1);
        let preparation = self
            .source
            .code
            .start_with_receipt(
                identity,
                archive,
                self.source.base.clone(),
                input.deadline,
                |receipt| {
                    #[cfg(test)]
                    self.fault(2);
                    self.state
                        .send_modify(|state| state.receipt = Some(receipt));
                    #[cfg(test)]
                    self.fault(3);
                },
            )
            .map_err(|error| match error {
                DockerError::Busy => CodeStageError::AdmissionBusy,
                other => CodeStageError::Docker(other),
            })?;
        let ready = tokio::select! {biased;
            ()=stopped(&mut input.global)=>return Err(DockerError::Cancelled.into()),
            ()=stopped(&mut input.stop)=>return Err(DockerError::Cancelled.into()),
            ready=preparation.wait()=>ready?,
        };
        input.check()?;
        if ready.volume_name() != &self.volume
            || ready.base_image_id() != &self.source.base.resolved().metadata().image_id
        {
            return Err(DockerError::Quarantined.into());
        }
        self.state.send_modify(|state| state.ready = Some(ready));
        Ok(())
    }
    pub(super) async fn wait_ready(&self, deadline: Instant) -> Result<(), CodeStageError> {
        timeout_at(deadline, self.stage.wait())
            .await
            .map_err(|_| DockerError::Deadline)?
            .map_err(|_| DockerError::Worker)?;
        check_deadline(deadline)?;
        self.state
            .borrow()
            .outcome
            .clone()
            .ok_or(DockerError::Worker)??;
        self.ready_mount()?;
        Ok(())
    }
    /// Returns a sticky supervisor diagnostic separately from positive retirement.
    pub(super) async fn retire(&self, deadline: Instant) -> Result<bool, DockerError> {
        let joined = timeout_at(deadline, self.stage.wait())
            .await
            .map_err(|_| DockerError::Deadline)?;
        let failed = joined.is_err()
            || matches!(
                self.state.borrow().outcome,
                Some(Err(CodeStageError::Docker(DockerError::Worker)))
            );
        self.state.send_modify(|state| {
            state.ready.take();
        });
        let receipt = self.state.borrow().receipt.clone();
        if let Some(mut receipt) = receipt {
            receipt.wait(deadline).await?;
        }
        // Every accepted preparation publishes above before-enqueue; no receipt
        // after this original stage joins therefore proves no acceptance.
        check_deadline(deadline)?;
        self.settled.store(true, Ordering::Release);
        Ok(failed)
    }
}

#[derive(Debug)]
pub(super) struct AllocationClaim {
    plan: Arc<ZipPlan>,
    committed: bool,
}
impl AllocationClaim {
    pub(super) fn belongs_to(&self, plan: &Arc<ZipPlan>) -> bool {
        Arc::ptr_eq(&self.plan, plan)
    }
    pub(super) fn commit(&mut self) {
        self.committed = true;
    }
}
impl Drop for AllocationClaim {
    fn drop(&mut self) {
        if !self.committed {
            self.plan.claimed.store(false, Ordering::Release);
        }
    }
}

#[derive(Debug, TypedBuilder)]
pub(super) struct StageInput {
    artifact: ValidatedArtifact,
    deadline: Instant,
    global: watch::Receiver<bool>,
    stop: watch::Receiver<bool>,
}
impl StageInput {
    fn check(&self) -> Result<(), DockerError> {
        check_deadline(self.deadline)?;
        if *self.global.borrow()
            || *self.stop.borrow()
            || self.global.has_changed().is_err()
            || self.stop.has_changed().is_err()
        {
            return Err(DockerError::Cancelled);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
