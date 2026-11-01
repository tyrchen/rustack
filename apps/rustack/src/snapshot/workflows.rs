//! One retained archive operation/CPU/IO lane above observer cancellation.

use std::{
    error::Error,
    fmt, mem,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use rustack_stepfunctions_core::{
    PreparedSnapshot, Registry, RegistryError, RegistryReceipt, RegistryResult, RegistrySnapshot,
    SnapshotLimits,
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch},
    task::JoinHandle,
    time::{Instant, timeout_at},
};

use super::{
    SnapshotKind, SnapshotService,
    archive::{read_file_bounded, to_cbor_bounded},
    decode_state,
    worker::WorkerFailure,
};

mod io;
mod owner;

const HISTORY_FILE: &str = "history.cbor";

#[derive(Clone)]
pub(crate) struct WorkflowArchive {
    sender: mpsc::Sender<Envelope>,
    capacity: Arc<Semaphore>,
    closing: watch::Sender<bool>,
    ready: watch::Receiver<bool>,
    bytes: usize,
    io_budget: Duration,
}
impl fmt::Debug for WorkflowArchive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WorkflowArchive(<redacted>)")
    }
}
#[derive(Debug)]
pub(crate) struct ArchiveSupervisor {
    closing: watch::Sender<bool>,
    join: Option<JoinHandle<Result<()>>>,
    failure: Option<Failure>,
}
#[derive(Clone)]
struct Failure(Arc<anyhow::Error>);
impl fmt::Debug for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("workflow archive owner failed")
    }
}
impl Error for Failure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.0.as_ref().as_ref())
    }
}
enum Command {
    Save(PathBuf),
    Load(Vec<u8>, PathBuf),
}
impl fmt::Debug for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ArchiveCommand(<redacted>)")
    }
}
#[derive(Debug)]
struct Envelope {
    command: Option<Command>,
    deadline: Instant,
    reply: oneshot::Sender<Result<Reply>>,
    permit: OwnedSemaphorePermit,
}
struct Reply {
    bytes: Vec<u8>,
    _permit: OwnedSemaphorePermit,
}
impl fmt::Debug for Reply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ArchiveReply(<redacted>)")
    }
}
enum Materialized {
    Saved { metadata: Vec<u8>, history: Vec<u8> },
    Prepared(PreparedSnapshot),
}
impl fmt::Debug for Materialized {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ArchiveMaterialized(<redacted>)")
    }
}
#[derive(Debug)]
struct Owner {
    registry: Registry,
    bytes: usize,
    inbox: mpsc::Receiver<Envelope>,
    closing: watch::Sender<bool>,
    ready: watch::Sender<bool>,
    active: Option<Envelope>,
    cpu: Option<JoinHandle<Result<Materialized>>>,
    io: Option<JoinHandle<Result<io::Output>>>,
    receipt: Option<RegistryReceipt>,
    fatal: bool,
    #[cfg(test)]
    hooks: Option<Arc<tests::Hooks>>,
}

impl WorkflowArchive {
    pub(crate) fn new(
        registry: Registry,
        bytes: usize,
        io_budget: Duration,
    ) -> Result<(Self, ArchiveSupervisor)> {
        Self::new_owned(
            registry,
            bytes,
            io_budget,
            #[cfg(test)]
            None,
        )
    }
    fn new_owned(
        registry: Registry,
        bytes: usize,
        io_budget: Duration,
        #[cfg(test)] hooks: Option<Arc<tests::Hooks>>,
    ) -> Result<(Self, ArchiveSupervisor)> {
        SnapshotLimits::try_from(bytes)?;
        if io_budget.is_zero() {
            bail!("invalid workflow archive IO deadline");
        }
        let (sender, inbox) = mpsc::channel(1);
        let (closing, _) = watch::channel(false);
        let (ready, status) = watch::channel(true);
        let owner = Owner {
            registry,
            bytes,
            inbox,
            closing: closing.clone(),
            ready,
            active: None,
            cpu: None,
            io: None,
            receipt: None,
            fatal: false,
            #[cfg(test)]
            hooks,
        };
        let join = tokio::spawn(owner.run());
        Ok((
            Self {
                sender,
                capacity: Arc::new(Semaphore::new(1)),
                closing: closing.clone(),
                ready: status,
                bytes,
                io_budget,
            },
            ArchiveSupervisor {
                closing,
                join: Some(join),
                failure: None,
            },
        ))
    }
    pub(crate) fn is_ready(&self) -> bool {
        !*self.closing.borrow() && *self.ready.borrow()
    }
    async fn execute(&self, command: Command) -> Result<Vec<u8>> {
        if !self.is_ready() {
            bail!("workflow archive owner unavailable");
        }
        let permit = self
            .capacity
            .clone()
            .try_acquire_owned()
            .context("workflow archive capacity exhausted")?;
        let deadline = Instant::now()
            .checked_add(self.io_budget)
            .context("workflow archive deadline overflow")?;
        let (reply, receipt) = oneshot::channel();
        self.sender
            .try_send(Envelope {
                command: Some(command),
                deadline,
                reply,
                permit,
            })
            .map_err(|_| anyhow::anyhow!("workflow archive admission unavailable"))?;
        let response = receipt
            .await
            .context("workflow archive reply owner lost")??;
        Ok(response.bytes)
    }
}
impl ArchiveSupervisor {
    pub(crate) fn close(&self) {
        self.closing.send_replace(true);
    }
    pub(crate) async fn shutdown(&mut self, deadline: Instant) -> Result<()> {
        self.close();
        if let Some(join) = &mut self.join {
            let result = timeout_at(deadline, join)
                .await
                .context("workflow archive shutdown observation expired")?;
            self.join = None;
            let result = result
                .map_err(WorkerFailure::new)
                .map_err(anyhow::Error::from)
                .and_then(|result| result);
            if let Err(error) = result {
                self.failure = Some(Failure(Arc::new(error)));
            }
        }
        self.failure
            .as_ref()
            .map_or(Ok(()), |error| Err(error.clone().into()))
    }
}
impl Drop for ArchiveSupervisor {
    fn drop(&mut self) {
        // The retained root keeps accepted CPU/IO/commit work until joined completion.
        self.close();
    }
}

#[async_trait]
impl SnapshotService for WorkflowArchive {
    fn service_name(&self) -> &'static str {
        "stepfunctions"
    }
    fn snapshot_kind(&self) -> SnapshotKind {
        SnapshotKind::Data
    }
    fn payload_limit(&self) -> Option<usize> {
        Some(self.bytes)
    }
    fn data_file(&self) -> Option<&'static str> {
        Some(HISTORY_FILE)
    }
    async fn save_meta(&self, staging: &Path) -> Result<Vec<u8>> {
        self.execute(Command::Save(staging.to_owned())).await
    }
    async fn load_meta(&self, metadata: Vec<u8>, staging: &Path) -> Result<()> {
        self.execute(Command::Load(metadata, staging.to_owned()))
            .await?;
        Ok(())
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.closing.send_replace(true);
        self.ready.send_replace(false);
        if self.active.is_some() {
            // A root panic is not a cleanup receipt. Preserve the bounded job graph.
            if let Some(cpu) = self.cpu.take() {
                mem::forget(cpu);
            }
            if let Some(io) = self.io.take() {
                mem::forget(io);
            }
            if let Some(receipt) = self.receipt.take() {
                mem::forget(receipt);
            }
            if let Some(active) = self.active.take() {
                mem::forget(active);
            }
        }
    }
}

#[cfg(test)]
mod tests;
