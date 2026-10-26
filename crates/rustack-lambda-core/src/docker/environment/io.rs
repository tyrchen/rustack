//! Worker-owned streams transfer to retained cleanup before their guard drops.

use std::{collections::VecDeque, mem::take, sync::Arc, time::Duration};

use bytes::Bytes;
use rustack_lambda_runtime::{
    client::{ClientConfig, ControlClient},
    diagnostics::runtime_dispatch,
    protocol::{Architecture, EnvironmentId},
    relay::RelayError,
};
use tokio::{
    io::{AsyncWriteExt, duplex},
    runtime::Handle,
    sync::watch,
    task::JoinHandle,
    time::{Instant, timeout_at},
};
use tracing::instrument::WithSubscriber;

use super::{EnvironmentConfig, EnvironmentError, actor::Signals};
use crate::{
    docker::{
        DockerAttachment, DockerError, FunctionLogs, OutputChannel,
        connection::stopped as wait_for_stop,
    },
    executor::ExecutorError,
    invocation::CleanupTask,
};

struct Bridge {
    stop: watch::Sender<bool>,
    task: JoinHandle<Result<(), DockerError>>,
    status: watch::Receiver<Option<Result<(), DockerError>>>,
}
#[derive(Default)]
struct Streams {
    client: Option<ControlClient>,
    logs: Option<FunctionLogs>,
    bridge: Option<Bridge>,
}
pub(super) struct IoGuard {
    streams: Streams,
    runtime: Handle,
    cleanup: Arc<CleanupTask>,
    config: EnvironmentConfig,
    signals: Signals,
    closing: bool,
    logs_closed: bool,
}
impl IoGuard {
    #[cfg(test)]
    pub(super) fn test_streams(&mut self, client: ControlClient, logs: FunctionLogs) {
        self.streams.client = Some(client);
        self.streams.logs = Some(logs);
    }

    pub(super) fn new(
        runtime: Handle,
        cleanup: Arc<CleanupTask>,
        config: EnvironmentConfig,
        signals: Signals,
    ) -> Self {
        Self {
            streams: Streams::default(),
            runtime,
            cleanup,
            config,
            signals,
            closing: false,
            logs_closed: false,
        }
    }
    pub(super) fn control(
        &mut self,
        attachment: DockerAttachment,
        environment: EnvironmentId,
        architecture: Architecture,
    ) -> Result<(), EnvironmentError> {
        let (mut output, input) = attachment.split(self.config.0.io)?;
        let (mut bridge, reader) = duplex(64 * 1024);
        self.streams.client = Some(ControlClient::start(
            ClientConfig::new(environment, architecture, self.config.0.io)?,
            reader,
            input,
        )?);
        let (stop, mut stopped) = watch::channel(false);
        let (status, observed) = watch::channel(None);
        let budget = self.config.0.io;
        let mut diagnostics = VecDeque::new();
        diagnostics
            .try_reserve_exact(65536)
            .map_err(|_| DockerError::Allocation)?;
        let task=self.runtime.spawn(async move{
            let result=async{
                loop{
                    let chunk=tokio::select!{biased;()=wait_for_stop(&mut stopped)=>return Ok(()),chunk=output.next(Instant::now()+Duration::from_secs(3600))=>chunk?,};
                    match chunk{
                        None=>return Ok(()),
                        Some(chunk) if chunk.channel==OutputChannel::Stdout=>{timeout_at(Instant::now()+budget,bridge.write_all(&chunk.bytes)).await.map_err(|_|DockerError::Deadline)??;},
                        Some(chunk)=>{let remove=diagnostics.len().saturating_add(chunk.bytes.len()).saturating_sub(65536);diagnostics.drain(..remove.min(diagnostics.len()));diagnostics.extend(chunk.bytes.iter().copied());},
                    }
                }
            }.await;status.send_replace(Some(result.clone()));result
        }.with_subscriber(runtime_dispatch()));
        self.streams.bridge = Some(Bridge {
            stop,
            task,
            status: observed,
        });
        Ok(())
    }
    pub(super) fn client(&mut self) -> Result<&mut ControlClient, EnvironmentError> {
        self.streams.client.as_mut().ok_or(EnvironmentError::Worker)
    }
    pub(super) fn logs(&mut self, attachment: DockerAttachment) -> Result<(), EnvironmentError> {
        self.streams.logs = Some(FunctionLogs::start(
            attachment,
            self.config.0.io,
            self.config.0.log_ring_bytes,
        )?);
        Ok(())
    }
    pub(super) async fn tail(&mut self) -> Result<Bytes, EnvironmentError> {
        let logs = self.streams.logs.as_ref().ok_or(EnvironmentError::Worker)?;
        let snapshot = logs.snapshot(self.config.deadline()).await?;
        if let Some(error) = snapshot.failure {
            return Err(error.into());
        }
        self.logs_closed = snapshot.closed;
        Ok(snapshot.tail)
    }
    pub(super) async fn healthy(&mut self) -> Result<(), EnvironmentError> {
        if let Some(bridge) = &self.streams.bridge {
            if let Some(result) = bridge.status.borrow().as_ref() {
                return match result {
                    Ok(()) => Err(DockerError::Protocol.into()),
                    Err(error) => Err(error.clone().into()),
                };
            }
            if bridge.task.is_finished() && bridge.status.borrow().is_none() {
                return Err(EnvironmentError::Worker);
            }
        }
        self.tail().await?;
        if self.logs_closed {
            Err(DockerError::Protocol.into())
        } else {
            Ok(())
        }
    }
    pub(super) fn reusable(&self) -> bool {
        !self.logs_closed
            && self
                .streams
                .bridge
                .as_ref()
                .is_none_or(|bridge| bridge.status.borrow().is_none())
    }
    pub(super) fn begin_cleanup(&mut self) {
        if self.closing {
            return;
        }
        self.closing = true;
        let streams = take(&mut self.streams);
        let config = self.config;
        let signals = self.signals.clone();
        self.cleanup.start_on(&self.runtime, move || async move {
            close(streams, config, signals).await
        });
    }
}
impl Drop for IoGuard {
    fn drop(&mut self) {
        self.begin_cleanup();
    }
}
async fn close(
    mut streams: Streams,
    config: EnvironmentConfig,
    signals: Signals,
) -> Result<(), ExecutorError> {
    let mut failed = false;
    if let Some(client) = &mut streams.client {
        if let Err(error) = client.shutdown(config.deadline()).await {
            failed |= matches!(error, RelayError::Worker);
            signals.error(error.into());
        }
    }
    if let Some(bridge) = &streams.bridge {
        bridge.stop.send_replace(true);
    }
    if let Some(logs) = &mut streams.logs {
        if let Err(error) = logs.shutdown().await {
            failed |= matches!(error, DockerError::Worker);
            signals.error(error.into());
        }
    }
    if let Some(bridge) = streams.bridge {
        match bridge.task.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                signals.error(error.into());
            }
            Err(_) => failed = true,
        }
    }
    if failed {
        signals.error(EnvironmentError::Worker);
        Err(ExecutorError::CleanupFailed(
            "Environment IO supervision failed".into(),
        ))
    } else {
        Ok(())
    }
}
