//! Main keeps one session above all startup/save/final-cleanup observations.

use std::fmt;

use anyhow::{Result, bail};
use tokio::time::Instant;

use super::{RuntimeProviders, SnapshotConfig, SnapshotTask, lease::LeaseAcquisition};

#[derive(Debug)]
enum Operation {
    Load(SnapshotTask),
    Save(SnapshotTask),
}
impl Operation {
    async fn wait(&mut self, deadline: Instant) -> Result<()> {
        match self {
            Self::Load(task) | Self::Save(task) => task.wait(deadline).await,
        }
    }
}
#[derive(Debug)]
struct Named {
    config: SnapshotConfig,
    lease: LeaseAcquisition,
    transaction: Option<Operation>,
    loaded: bool,
}
pub(crate) struct SnapshotSession {
    named: Option<Named>,
}
impl fmt::Debug for SnapshotSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SnapshotSession(<redacted>)")
    }
}
impl SnapshotSession {
    pub(crate) fn new(config: Option<SnapshotConfig>) -> Self {
        Self {
            named: config.map(|config| Named {
                lease: config.begin_lease(),
                config,
                transaction: None,
                loaded: false,
            }),
        }
    }
    pub(crate) async fn load(
        &mut self,
        providers: &RuntimeProviders,
        deadline: Instant,
    ) -> Result<()> {
        let Some(named) = &mut self.named else {
            return Ok(());
        };
        if named.loaded {
            return Ok(());
        }
        let lease = named.lease.wait(deadline).await?;
        if named.transaction.is_none() {
            if Instant::now() >= deadline {
                bail!("snapshot startup observation expired");
            }
            tracing::info!(coverage = ?providers.coverage(), "snapshot includes only declared state; other services and SQS messages are not persisted");
            named.transaction = Some(Operation::Load(SnapshotTask::start(
                named.config.clone(),
                providers.clone(),
                lease,
                None,
                deadline,
            )));
        }
        match &mut named.transaction {
            Some(Operation::Load(task)) => task.wait(deadline).await?,
            _ => bail!("invalid snapshot startup transaction"),
        }
        named.loaded = true;
        named.transaction = None;
        Ok(())
    }
    pub(crate) async fn save(
        &mut self,
        providers: &RuntimeProviders,
        version: &'static str,
        deadline: Instant,
    ) -> Result<()> {
        let Some(named) = &mut self.named else {
            return Ok(());
        };
        if !named.loaded {
            bail!("cannot publish snapshot after failed startup");
        }
        let lease = named.lease.wait(deadline).await?;
        if named.transaction.is_none() {
            if Instant::now() >= deadline {
                bail!("snapshot save observation expired");
            }
            named.transaction = Some(Operation::Save(SnapshotTask::start(
                named.config.clone(),
                providers.clone(),
                lease,
                Some(version),
                deadline,
            )));
        }
        match &mut named.transaction {
            Some(Operation::Save(task)) => task.wait(deadline).await,
            _ => bail!("invalid snapshot save transaction"),
        }
    }
    pub(crate) async fn finish(&mut self, deadline: Instant) -> Result<()> {
        let Some(named) = &mut self.named else {
            return Ok(());
        };
        let lease = named.lease.wait(deadline).await;
        if let Some(operation) = &mut named.transaction {
            operation.wait(deadline).await?;
        }
        lease.map(|_| ())
    }
    pub(crate) fn quarantine(&mut self) {
        if let Some(named) = &mut self.named {
            named.lease.quarantine();
        }
    }
}

#[cfg(test)]
mod tests;
