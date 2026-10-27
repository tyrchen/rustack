//! Retained CPU/IO and accepted registry commits outlive individual observers.

use std::{future::Future, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use rustack_stepfunctions_core::{RegistryHistories, RegistryMetadata};
use tokio::{
    task::{spawn, spawn_blocking},
    time::{Instant, timeout_at},
};

use super::{
    Command, Failure, Materialized, Owner, RegistryError, RegistryResult, RegistrySnapshot, Reply,
    SnapshotLimits, WorkerFailure, decode_state, io, to_cbor_bounded,
};

impl Owner {
    pub(super) async fn run(mut self) -> Result<()> {
        let mut closing = self.closing.subscribe();
        loop {
            if *closing.borrow() {
                self.inbox.close();
            }
            let envelope = tokio::select! {
                biased;
                _ = closing.changed(), if !self.inbox.is_closed() => { self.inbox.close(); continue; }
                envelope = self.inbox.recv() => envelope,
            };
            let Some(envelope) = envelope else {
                break;
            };
            self.active = Some(envelope);
            let result = self.process().await;
            let (result, fatal) = if self.fatal {
                let error = match result {
                    Err(error) => error,
                    Ok(_) => anyhow::anyhow!("workflow archive fatal-state invariant failed"),
                };
                let failure = Failure(Arc::new(error));
                (Err(failure.clone().into()), Some(failure))
            } else {
                (result, None)
            };
            let envelope = self
                .active
                .take()
                .context("workflow archive envelope lost")?;
            let reply = result.map(|bytes| Reply {
                bytes,
                _permit: envelope.permit,
            });
            let _observed = envelope.reply.send(reply);
            if let Some(failure) = fatal {
                return Err(failure.into());
            }
        }
        Ok(())
    }
    async fn process(&mut self) -> Result<Vec<u8>> {
        let active = self
            .active
            .as_mut()
            .context("workflow archive envelope missing")?;
        let deadline = active.deadline;
        let command = active
            .command
            .take()
            .context("workflow archive command missing")?;
        Self::check_deadline(deadline)?;
        let registry = self.registry.clone();
        let bytes = self.bytes;
        match command {
            Command::Save(directory) => {
                #[cfg(test)]
                let hooks = self.hooks.clone();
                self.cpu = Some(spawn_blocking(move || {
                    let document = registry.export_snapshot(SnapshotLimits::try_from(bytes)?)?;
                    let (metadata, history) = document.into_archive_parts()?;
                    let result = Ok(Materialized::Saved {
                        metadata: to_cbor_bounded(&metadata, bytes)?,
                        history: to_cbor_bounded(&history, bytes)?,
                    });
                    #[cfg(test)]
                    if let Some(hooks) = hooks {
                        hooks.cpu_wait();
                    }
                    result
                }));
                let Materialized::Saved { metadata, history } = self.join_cpu().await? else {
                    bail!("invalid workflow archive save result");
                };
                Self::check_deadline(deadline)?;
                self.start_io(io::write(directory, history));
                if !matches!(self.join_io(deadline).await?, io::Output::Written) {
                    bail!("invalid workflow archive write result");
                }
                Ok(metadata)
            }
            Command::Load(metadata, directory) => {
                if metadata.len() > bytes {
                    bail!("workflow archive metadata byte limit exceeded");
                }
                self.start_io(io::read(directory, bytes));
                let io::Output::History(history) = self.join_io(deadline).await? else {
                    bail!("invalid workflow archive read result");
                };
                Self::check_deadline(deadline)?;
                #[cfg(test)]
                let hooks = self.hooks.clone();
                self.cpu = Some(spawn_blocking(move || {
                    let metadata: RegistryMetadata = decode_state(&metadata)?;
                    let history: RegistryHistories = decode_state(&history)?;
                    let document = registry.prepare_snapshot(
                        RegistrySnapshot::from_archive_parts(metadata, history)?,
                        SnapshotLimits::try_from(bytes)?,
                    )?;
                    #[cfg(test)]
                    if let Some(hooks) = hooks {
                        hooks.cpu_wait();
                    }
                    Ok(Materialized::Prepared(document))
                }));
                let Materialized::Prepared(prepared) = self.join_cpu().await? else {
                    bail!("invalid workflow archive prepare result");
                };
                Self::check_deadline(deadline)?;
                self.receipt = Some(self.registry.restore_snapshot(prepared)?);
                // Accepted commit ownership is independent of the command observer's deadline.
                loop {
                    let receipt = self
                        .receipt
                        .as_mut()
                        .context("workflow archive restore receipt missing")?;
                    match receipt.wait(Instant::now() + Duration::from_secs(1)).await {
                        Err(RegistryError::Deadline) => {}
                        Ok(RegistryResult::Restored) => {
                            self.receipt = None;
                            return Ok(Vec::new());
                        }
                        Ok(_) => {
                            self.receipt = None;
                            bail!("invalid workflow archive restore acknowledgement");
                        }
                        Err(error) => {
                            self.receipt = None;
                            return Err(error.into());
                        }
                    }
                }
            }
        }
    }
    fn check_deadline(deadline: Instant) -> Result<()> {
        if Instant::now() >= deadline {
            bail!("workflow archive operation deadline exceeded");
        }
        Ok(())
    }
    fn start_io(&mut self, operation: impl Future<Output = Result<io::Output>> + Send + 'static) {
        #[cfg(test)]
        let task = {
            let hooks = self.hooks.clone();
            spawn(async move {
                if let Some(hooks) = hooks {
                    hooks.io_wait().await?;
                }
                operation.await
            })
        };
        #[cfg(not(test))]
        let task = spawn(operation);
        self.io = Some(task);
    }
    async fn join_cpu(&mut self) -> Result<Materialized> {
        let result = self
            .cpu
            .as_mut()
            .context("workflow archive CPU join missing")?
            .await;
        self.cpu = None;
        result.map_err(|error| {
            self.fatal = true;
            anyhow::Error::from(WorkerFailure::new(error))
        })?
    }
    async fn join_io(&mut self, deadline: Instant) -> Result<io::Output> {
        let task = self
            .io
            .as_mut()
            .context("workflow archive IO join missing")?;
        let (expired, result) = if let Ok(result) = timeout_at(deadline, &mut *task).await {
            (false, result)
        } else {
            self.ready.send_replace(false);
            (true, task.await)
        };
        self.io = None;
        let result = result.map_err(|error| {
            self.fatal = true;
            anyhow::Error::from(WorkerFailure::new(error))
        })?;
        if expired {
            self.ready.send_replace(true);
            bail!("workflow archive IO observation expired");
        }
        let output = result?;
        Self::check_deadline(deadline)?;
        Ok(output)
    }
}

#[cfg(test)]
mod tests;
