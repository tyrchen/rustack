//! Unique retained OS-lease acquisition survives observing timeout/drop.

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::{io::ErrorKind, mem, sync::Arc};

use anyhow::{Context, Result, bail};
#[cfg(unix)]
use nix::fcntl::OFlag;
#[cfg(test)]
use tokio::time::Duration;
use tokio::{
    task::JoinHandle,
    time::{Instant, timeout_at},
};

use super::{SnapshotConfig, SnapshotLease, task::Failure, worker::WorkerFailure};

#[derive(Debug)]
pub(crate) struct LeaseAcquisition {
    join: Option<JoinHandle<Result<SnapshotLease>>>,
    result: Option<Result<Arc<SnapshotLease>, Failure>>,
}
impl SnapshotConfig {
    /// Start the same blocking acquisition once; keep its owner above timeouts.
    pub(crate) fn begin_lease(&self) -> LeaseAcquisition {
        self.begin_owned_lease(
            #[cfg(test)]
            None,
        )
    }
    #[allow(clippy::disallowed_types, clippy::disallowed_methods)] // Existing lease std-IO boundary; Tokio has no advisory lock equivalent.
    fn begin_owned_lease(&self, #[cfg(test)] gate: Option<Arc<tests::Gate>>) -> LeaseAcquisition {
        let root = self.root.clone();
        let name = self.name.as_str().to_owned();
        let join = tokio::task::spawn_blocking(move || {
            #[cfg(test)]
            if let Some(gate) = &gate {
                gate.enter();
            }
            std::fs::create_dir_all(&root).context("create snapshot root for lease")?;
            let path = root.join(format!(".{name}.lock"));
            match std::fs::symlink_metadata(&path) {
                Ok(metadata) if !metadata.file_type().is_file() => {
                    bail!("snapshot lock must be a regular file")
                }
                Ok(_) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(error).context("inspect snapshot lease path"),
            }
            let mut options = std::fs::OpenOptions::new();
            options.create(true).truncate(false).read(true).write(true);
            #[cfg(unix)]
            {
                options.custom_flags((OFlag::O_NONBLOCK | OFlag::O_NOFOLLOW).bits());
            }
            let file = options.open(&path).context("open snapshot lease")?;
            if !file
                .metadata()
                .context("inspect opened snapshot lease")?
                .is_file()
            {
                bail!("snapshot lock must be a regular file");
            }
            file.try_lock()
                .context("snapshot is already owned by another runtime")?;
            Ok(SnapshotLease { _file: file })
        });
        LeaseAcquisition {
            join: Some(join),
            result: None,
        }
    }
    #[cfg(test)]
    pub(crate) async fn acquire_lease(&self) -> Result<Arc<SnapshotLease>> {
        self.begin_lease()
            .wait(Instant::now() + Duration::from_secs(5))
            .await
    }
}
impl LeaseAcquisition {
    pub(super) fn quarantine(&mut self) {
        if let Some(join) = self.join.take() {
            mem::forget(join);
        }
        if let Some(Ok(lease)) = self.result.take() {
            lease.quarantine();
        }
    }
    pub(crate) async fn wait(&mut self, deadline: Instant) -> Result<Arc<SnapshotLease>> {
        if let Some(join) = &mut self.join {
            let result = timeout_at(deadline, join)
                .await
                .context("snapshot lease acquisition observation expired")?;
            self.join = None;
            let result = result
                .map_err(WorkerFailure::new)
                .map_err(anyhow::Error::from)
                .and_then(|result| result);
            self.result = Some(result.map(Arc::new).map_err(Failure::new));
            if Instant::now() >= deadline {
                bail!("snapshot lease acquisition observation expired");
            }
        }
        match &self.result {
            Some(Ok(lease)) => Ok(lease.clone()),
            Some(Err(error)) => Err(error.clone().into()),
            None => bail!("snapshot lease acquisition result missing"),
        }
    }
}
impl Drop for LeaseAcquisition {
    fn drop(&mut self) {
        // At most one pending lock acquisition exists in main. A lost owner is
        // not proof that its late-acquired OS lock is safe to release.
        if let Some(join) = self.join.take() {
            mem::forget(join);
        }
    }
}

#[cfg(test)]
mod tests;
