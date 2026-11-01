//! Above-worker guards, durable-state reconciliation and independent reaping.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use tokio::{
    runtime::Handle,
    sync::{OwnedSemaphorePermit, Semaphore, watch},
    time::{Instant, sleep},
};
use typed_builder::TypedBuilder;

use super::{CodeVolumeConfig, CodeVolumeIdentity, types::Acceptance};
use crate::{
    artifact::PinnedImage,
    docker::{
        DirectoryLease, DockerCodeArchive, DockerError, DockerPort,
        mutation::MutationReceipt,
        ownership::{
            CodeProducerGuard,
            codes::{CodeEffect, CodeRecord, LoaderState, VolumeState},
        },
        recovery::code::retire_effects,
    },
    invocation::CleanupTask,
};

#[derive(Debug, Default)]
pub(super) struct Health {
    pub(super) physical: AtomicBool,
    pub(super) failed: AtomicBool,
    pub(super) quarantined: AtomicUsize,
    #[cfg(test)]
    pub(super) ready: AtomicUsize,
    #[cfg(test)]
    pub(super) fault: std::sync::atomic::AtomicU8,
}
impl Health {
    pub(super) fn check(&self) -> Result<(), DockerError> {
        if self.failed.load(Ordering::Acquire) {
            Err(DockerError::Worker)
        } else if self.quarantined.load(Ordering::Acquire) > 0 {
            Err(DockerError::Quarantined)
        } else {
            Ok(())
        }
    }
}
#[derive(Debug, TypedBuilder)]
pub(super) struct Context {
    pub(super) docker: DockerPort,
    pub(super) directory: DirectoryLease,
    pub(super) config: CodeVolumeConfig,
    pub(super) stop: watch::Sender<bool>,
    pub(super) health: Arc<Health>,
    pub(super) runtime: Handle,
    pub(super) cleanup: Semaphore,
    pub(super) producer: CodeProducerGuard,
}
impl Context {
    #[cfg(test)]
    pub(super) fn fault(&self, point: u8) {
        if self
            .health
            .fault
            .compare_exchange(point, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            panic!("deliberate code-volume owner fault");
        }
    }
    pub(super) fn fail(&self) {
        self.health.failed.store(true, Ordering::Release);
        self.stop.send_replace(true);
    }
}
#[derive(Debug, Clone, Default)]
pub(super) struct Receipts {
    pub(super) volume: Option<MutationReceipt>,
    pub(super) loader: Option<MutationReceipt>,
}
impl Receipts {
    fn unissued(&self) -> bool {
        self.volume
            .as_ref()
            .is_none_or(MutationReceipt::definitely_unissued)
            && self
                .loader
                .as_ref()
                .is_none_or(MutationReceipt::definitely_unissued)
    }
}
#[derive(Debug)]
pub(super) struct Progress {
    pub(super) receipts: watch::Sender<Receipts>,
    pub(super) active: Arc<AtomicBool>,
    pub(super) clean: watch::Sender<bool>,
    pub(super) retired: AtomicBool,
    pub(super) running: AtomicBool,
}
impl Progress {
    pub(super) fn new(clean: watch::Sender<bool>) -> Self {
        let (receipts, _) = watch::channel(Receipts::default());
        Self {
            receipts,
            active: Arc::new(AtomicBool::new(false)),
            clean,
            retired: AtomicBool::new(false),
            running: AtomicBool::new(false),
        }
    }
}
#[derive(Debug, TypedBuilder)]
pub(super) struct Plan {
    pub(super) identity: Arc<CodeVolumeIdentity>,
    pub(super) base: PinnedImage,
    pub(super) archive: watch::Sender<Option<DockerCodeArchive>>,
    pub(super) record: CodeRecord,
    pub(super) deadline: Instant,
    pub(super) cancel: watch::Receiver<()>,
}
#[derive(Debug, TypedBuilder)]
pub(super) struct Entry {
    pub(super) plan: Plan,
    pub(super) progress: Progress,
    pub(super) stage: CleanupTask,
    pub(super) outcome: watch::Sender<Option<Result<(), DockerError>>>,
    pub(super) handoff: Acceptance,
    pub(super) released: watch::Receiver<()>,
    pub(super) _volume: OwnedSemaphorePermit,
}
impl Entry {
    pub(super) fn check(&self, context: &Context) -> Result<(), DockerError> {
        if Instant::now() >= self.plan.deadline {
            return Err(DockerError::Deadline);
        }
        if *context.stop.borrow() || !self.plan.base.liveness().is_live() {
            return Err(DockerError::Stopped);
        }
        if self.plan.cancel.has_changed().is_err() {
            return Err(DockerError::Cancelled);
        }
        Ok(())
    }
    pub(super) async fn retire(&self, context: &Context) {
        self.progress.active.store(false, Ordering::Release);
        if self.stage.wait().await.is_err() {
            context.fail();
        }
        self.plan.archive.send_replace(None);
        let mut dirty = None;
        let mut delay = Duration::from_secs(1);
        loop {
            if self.cleanup_step(context).await {
                self.progress.clean.send_replace(true);
                self.handoff.send_modify(|handoff| {
                    handoff.slot.take();
                });
                return;
            }
            if dirty.is_none() {
                dirty = Some(Dirty::new(context.health.clone()));
            }
            sleep(delay).await;
            delay = delay.saturating_mul(2).min(Duration::from_secs(30));
        }
    }
    async fn cleanup_step(&self, context: &Context) -> bool {
        let Ok(_permit) = context.cleanup.acquire().await else {
            return false;
        };
        let deadline = context.config.deadline();
        let Ok(records) = context.directory.code_records(deadline).await else {
            return false;
        };
        let receipts = self.progress.receipts.borrow().clone();
        let Some(mut record) = records
            .into_iter()
            .find(|r| r.environment == self.plan.record.environment)
        else {
            return self.progress.retired.load(Ordering::Acquire) || receipts.unissued();
        };
        if !record.same_identity(&self.plan.record) {
            return receipts.unissued();
        }
        let proof = match (record.volume, record.loader) {
            (VolumeState::Uncertain, LoaderState::Planned) => {
                receipts.volume.as_ref().map(|r| (CodeEffect::Volume, r))
            }
            (VolumeState::Known, LoaderState::Uncertain) => {
                receipts.loader.as_ref().map(|r| (CodeEffect::Loader, r))
            }
            _ => None,
        };
        if let Some((effect, receipt)) = proof.filter(|(_, r)| r.definitely_unissued()) {
            if context
                .directory
                .abandon_code(record.clone(), effect, receipt, deadline)
                .await
                .is_err()
            {
                return false;
            }
            match effect {
                CodeEffect::Volume => {
                    record.volume = VolumeState::Retired;
                    record.loader = LoaderState::Retired;
                }
                CodeEffect::Loader => {
                    record.loader = LoaderState::Retired;
                }
            }
        }
        if !matches!(
            Box::pin(retire_effects(
                &context.directory,
                &context.docker,
                &mut record,
                deadline
            ))
            .await,
            Ok(true)
        ) {
            return false;
        }
        // This witness outlives a worker and is published before erase can commit.
        self.progress.retired.store(true, Ordering::Release);
        #[cfg(test)]
        context.fault(6);
        let erased = context
            .directory
            .settle_code(record, deadline)
            .await
            .is_ok();
        #[cfg(test)]
        if erased
            && context
                .health
                .fault
                .compare_exchange(7, 0, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            return false;
        }
        erased
    }
}
#[derive(Debug)]
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
