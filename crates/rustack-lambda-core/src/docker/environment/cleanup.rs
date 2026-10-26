//! Phase-aware ID-only cleanup; uncertain create absence never grants settlement.

use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use http::StatusCode;
use tokio::{
    sync::{Semaphore, watch},
    time::sleep,
};
use typed_builder::TypedBuilder;

use super::{
    EnvironmentConfig, EnvironmentError, EnvironmentPhase,
    actor::{Health, Signals},
    io::IoGuard,
};
use crate::{
    docker::{
        DockerError, DockerKey, DockerPort, EnvironmentPlan, models::ContainerInspectResponse,
        mutation::MutationReceipt, ownership::ContainerRecord, plan::ContainerRole,
    },
    invocation::CleanupTask,
};

#[derive(Debug, Clone)]
pub(super) enum Phase {
    Unissued,
    Uncertain(MutationReceipt),
    Known(DockerKey),
    Clean,
}
#[derive(Debug)]
pub(super) struct Progress {
    pub(super) relay: watch::Sender<Phase>,
    pub(super) function: watch::Sender<Phase>,
}
impl Progress {
    pub(super) fn new() -> Self {
        Self {
            relay: watch::channel(Phase::Unissued).0,
            function: watch::channel(Phase::Unissued).0,
        }
    }
    pub(super) fn phase(&self, role: ContainerRole) -> &watch::Sender<Phase> {
        match role {
            ContainerRole::Relay => &self.relay,
            ContainerRole::Function => &self.function,
        }
    }
    pub(super) fn clean(&self) -> bool {
        [&self.relay, &self.function]
            .iter()
            .all(|phase| matches!(*phase.borrow(), Phase::Unissued | Phase::Clean))
    }
}

#[derive(TypedBuilder)]
pub(super) struct Cleaner {
    pub(super) docker: DockerPort,
    pub(super) config: EnvironmentConfig,
    pub(super) plan: Arc<EnvironmentPlan>,
    pub(super) progress: Arc<Progress>,
    pub(super) health: Arc<Health>,
    pub(super) cleanup: Arc<Semaphore>,
    pub(super) signals: Signals,
}
impl Cleaner {
    async fn settled(&self, role: ContainerRole) -> bool {
        let Ok(record) = ContainerRecord::from_plan(&self.plan, role) else {
            return false;
        };
        if self
            .plan
            .directory
            .settle_container(record, self.config.deadline())
            .await
            .is_err()
        {
            return false;
        }
        self.progress.phase(role).send_replace(Phase::Clean);
        true
    }
    pub(super) async fn container(&self, role: ContainerRole) -> bool {
        let phase = self.progress.phase(role).borrow().clone();
        let id = match phase {
            Phase::Unissued | Phase::Clean => return self.settled(role).await,
            Phase::Known(id) => id,
            Phase::Uncertain(receipt) => {
                if receipt.definitely_unissued() {
                    return self.settled(role).await;
                }

                let Ok(inspected) = self
                    .docker
                    .inspect_container(self.plan.key(role), self.config.deadline())
                    .await
                else {
                    return false;
                };
                let Ok(id) = self.plan.owned(role, &inspected) else {
                    return false;
                };
                let Ok(record) = ContainerRecord::from_plan(&self.plan, role) else {
                    return false;
                };
                if self
                    .plan
                    .directory
                    .know_container(record, id.clone(), self.config.deadline())
                    .await
                    .is_err()
                {
                    return false;
                }
                self.progress
                    .phase(role)
                    .send_replace(Phase::Known(id.clone()));
                id
            }
        };
        let Ok(_permit) = self.cleanup.acquire().await else {
            return false;
        };
        let inspected = match self
            .docker
            .inspect_container(&id, self.config.deadline())
            .await
        {
            Err(DockerError::Status(StatusCode::NOT_FOUND)) => {
                return self.settled(role).await;
            }
            Ok(inspected) => inspected,
            Err(_) => return false,
        };
        if self.plan.owned(role, &inspected).is_err()
            || inspected.id.as_deref() != Some(id.as_str())
        {
            return false;
        }
        let Some(state) = inspected.state else {
            return false;
        };
        match state.running {
            Some(true) => {
                let _ = self
                    .docker
                    .kill_container(&id, self.config.deadline())
                    .await;
                let waited = self
                    .docker
                    .wait_container(&id, self.config.deadline())
                    .await;
                if !matches!(waited,Ok(ref exit) if exit.error.is_none()) {
                    return false;
                }
            }
            Some(false) => {}
            None => return false,
        }
        let _ = self
            .docker
            .remove_container(&id, self.config.deadline())
            .await;
        if matches!(
            self.docker
                .inspect_container(&id, self.config.deadline())
                .await,
            Err(DockerError::Status(StatusCode::NOT_FOUND))
        ) {
            self.settled(role).await
        } else {
            false
        }
    }
    pub(super) async fn retire(&self, io: &CleanupTask, mut streams: Option<&mut IoGuard>) {
        let mut dirty = None;
        let mut delay = Duration::from_secs(1);
        loop {
            let function = self.container(ContainerRole::Function).await;
            if function {
                if let Some(streams) = streams.take() {
                    streams.begin_cleanup();
                }
            }
            let joined = if function { io.wait().await } else { Ok(()) };
            let relay = if function {
                self.container(ContainerRole::Relay).await
            } else {
                false
            };
            if function && relay && joined.is_ok() {
                return;
            }
            if joined.is_err() {
                self.health.fail();
            }
            if dirty.is_none() {
                dirty = Some(Dirty::new(self.health.clone()));
            }
            self.signals.phase(EnvironmentPhase::Quarantined);
            sleep(delay).await;
            delay = delay.saturating_mul(2).min(Duration::from_secs(30));
        }
    }
    pub(super) async fn inspect_health(&self) -> Result<(), EnvironmentError> {
        let phase = self.progress.function.borrow().clone();
        let Phase::Known(id) = phase else {
            return Err(EnvironmentError::Stopped);
        };
        let inspected = self
            .docker
            .inspect_container(&id, self.config.deadline())
            .await?;
        if self.plan.owned(ContainerRole::Function, &inspected)? != id {
            return Err(DockerError::Quarantined.into());
        }
        function_health(&inspected)
    }
}
pub(super) fn function_health(
    inspected: &ContainerInspectResponse,
) -> Result<(), EnvironmentError> {
    let state = inspected.state.as_ref().ok_or(DockerError::Protocol)?;
    match state.running {
        Some(true) => Ok(()),
        Some(false) => Err(EnvironmentError::Exited {
            exit_code: state.exit_code.ok_or(DockerError::Protocol)?,
            oom: state.oom_killed.ok_or(DockerError::Protocol)?,
        }),
        None => Err(DockerError::Protocol.into()),
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
