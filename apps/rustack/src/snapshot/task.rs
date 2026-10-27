//! An observing deadline never aborts the snapshot transaction or its IO joins.

use std::{error::Error, fmt, mem, sync::Arc};

use anyhow::{Context, Result};
use tokio::{
    task::JoinHandle,
    time::{Instant, timeout_at},
};

use super::{RuntimeProviders, SnapshotConfig, SnapshotLease, worker::WorkerFailure};

pub(crate) struct SnapshotTask {
    join: Option<JoinHandle<Result<()>>>,
    failure: Option<Failure>,
}
impl fmt::Debug for SnapshotTask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SnapshotTask(<redacted>)")
    }
}
#[derive(Clone)]
pub(super) struct Failure(Arc<anyhow::Error>);
impl Failure {
    pub(super) fn new(error: anyhow::Error) -> Self {
        Self(Arc::new(error))
    }
}
impl fmt::Debug for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("snapshot transaction failed")
    }
}
impl Error for Failure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.0.as_ref().as_ref())
    }
}
struct Guard {
    lease: Option<Arc<SnapshotLease>>,
    providers: Option<RuntimeProviders>,
    finished: bool,
}
impl fmt::Debug for Guard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SnapshotGuard(<redacted>)")
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        if !self.finished {
            // A panic/cancellation cannot prove sibling filesystem retirement.
            if let Some(lease) = self.lease.take() {
                mem::forget(lease);
            }
            if let Some(providers) = self.providers.take() {
                mem::forget(providers);
            }
        }
    }
}
impl SnapshotTask {
    pub(crate) fn start(
        config: SnapshotConfig,
        providers: RuntimeProviders,
        lease: Arc<SnapshotLease>,
        version: Option<&'static str>,
    ) -> Self {
        let join = tokio::spawn(async move {
            let mut guard = Guard {
                lease: Some(lease),
                providers: Some(providers),
                finished: false,
            };
            let providers = guard
                .providers
                .as_ref()
                .context("snapshot transaction providers missing")?;
            let result = match version {
                Some(version) => config.save(providers, version).await,
                None => config.load(providers).await,
            };
            guard.finished = true;
            result
        });
        Self {
            join: Some(join),
            failure: None,
        }
    }
    pub(crate) async fn wait(&mut self, deadline: Instant) -> Result<()> {
        if let Some(join) = &mut self.join {
            let result = timeout_at(deadline, join)
                .await
                .context("snapshot transaction observation expired")?;
            self.join = None;
            let result = result
                .map_err(WorkerFailure::new)
                .map_err(anyhow::Error::from)
                .and_then(|result| result);
            if let Err(error) = result {
                self.failure = Some(Failure::new(error));
            }
            if Instant::now() >= deadline {
                anyhow::bail!("snapshot transaction observation expired");
            }
        }
        self.failure
            .as_ref()
            .map_or(Ok(()), |error| Err(error.clone().into()))
    }
}

#[cfg(test)]
mod tests;
