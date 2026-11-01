//! One ordered environment worker; all resources remain registered above its task.

use std::{future::pending, sync::Arc, time::Duration};

use bytes::Bytes;
use http::StatusCode;
use rustack_lambda_runtime::{
    client::ClientEvent,
    protocol::{Architecture as RuntimeArchitecture, EnvironmentId, HandlerTimeout},
    session::RuntimeResult,
};
use tokio::{
    runtime::Handle,
    sync::{Semaphore, mpsc, oneshot, watch},
    time::{Instant, interval, sleep_until},
};
use typed_builder::TypedBuilder;

use super::{
    Call, EnvironmentConfig, EnvironmentError, EnvironmentPhase, EnvironmentResponse,
    actor::{Health, Signals},
    cleanup::{Cleaner, Phase, Progress},
    io::IoGuard,
};
use crate::{
    artifact::{Architecture, DeploymentArtifact},
    docker::{
        DockerError, DockerKey, DockerPort, EnvironmentPlan,
        connection::stopped as wait_for_stop,
        mutation::MutationGuard,
        ownership::ContainerRecord,
        plan::ContainerRole,
        zip::{CodeStageError, StageInput},
    },
    invocation::CleanupTask,
};

#[derive(TypedBuilder)]
pub(super) struct Worker {
    docker: DockerPort,
    config: EnvironmentConfig,
    plan: Arc<EnvironmentPlan>,
    progress: Arc<Progress>,
    cleanup: Arc<Semaphore>,
    health: Arc<Health>,
    signals: Signals,
    global: watch::Sender<bool>,
    io: Arc<CleanupTask>,
    runtime: Handle,
}
pub(super) type FinalReply = (
    oneshot::Sender<Result<EnvironmentResponse, EnvironmentError>>,
    Result<EnvironmentResponse, EnvironmentError>,
);
impl Worker {
    #[cfg(test)]
    pub(super) async fn serve_and_retire_test(
        &self,
        io: &mut IoGuard,
        mailbox: &mut mpsc::Receiver<Call>,
    ) {
        let cleaner = self.cleaner();
        let pending = self.serve(io, &cleaner, mailbox).await;
        self.finish(io, &cleaner, mailbox, pending).await;
    }
    #[cfg(test)]
    pub(super) async fn serve_test(
        &self,
        io: &mut IoGuard,
        cleaner: &Cleaner,
        mailbox: &mut mpsc::Receiver<Call>,
    ) -> Option<FinalReply> {
        self.serve(io, cleaner, mailbox).await
    }

    pub(super) async fn run(
        self,
        mut mailbox: mpsc::Receiver<Call>,
        deadline: Instant,
    ) -> Result<(), EnvironmentError> {
        let mut io = IoGuard::new(
            self.runtime.clone(),
            self.io.clone(),
            self.config,
            self.signals.clone(),
        );
        let cleaner = self.cleaner();
        let pending = match self.initialize(&mut io, deadline).await {
            Ok(()) => {
                self.signals.phase(EnvironmentPhase::Idle);
                self.serve(&mut io, &cleaner, &mut mailbox).await
            }
            Err(error) => {
                self.signals.error(error);
                None
            }
        };
        self.finish(&mut io, &cleaner, &mut mailbox, pending).await;
        Ok(())
    }
    async fn finish(
        &self,
        io: &mut IoGuard,
        cleaner: &Cleaner,
        mailbox: &mut mpsc::Receiver<Call>,
        pending: Option<FinalReply>,
    ) {
        self.signals.phase(EnvironmentPhase::Retiring);
        mailbox.close();
        while let Ok(call) = mailbox.try_recv() {
            let _ = call.reply.send(Err(EnvironmentError::Stopped));
        }
        cleaner.retire(&self.io, Some(io)).await;
        if let Some((reply, result)) = pending {
            let _ = reply.send(result);
        }
    }
    fn cleaner(&self) -> Cleaner {
        Cleaner::builder()
            .docker(self.docker.clone())
            .config(self.config)
            .plan(self.plan.clone())
            .progress(self.progress.clone())
            .health(self.health.clone())
            .cleanup(self.cleanup.clone())
            .signals(self.signals.clone())
            .build()
    }
    fn check(&self, deadline: Instant) -> Result<(), EnvironmentError> {
        if *self.global.borrow() || *self.signals.stop.borrow() {
            Err(EnvironmentError::Cancelled)
        } else if Instant::now() >= deadline {
            Err(EnvironmentError::Deadline)
        } else {
            Ok(())
        }
    }
    async fn create(
        &self,
        role: ContainerRole,
        deadline: Instant,
    ) -> Result<DockerKey, EnvironmentError> {
        self.check(deadline)?;
        let mutation = MutationGuard::new();
        self.progress
            .phase(role)
            .send_replace(Phase::Uncertain(mutation.receipt()));
        let record = ContainerRecord::from_plan(&self.plan, role)?;
        self.plan
            .directory
            .prepare_container(record.clone(), self.config.deadline())
            .await?;
        let created = self
            .plan
            .create_tracked(role, &self.docker, self.config.deadline(), mutation)
            .await;
        if matches!(
            &created,
            Err(DockerError::Busy
                | DockerError::Status(StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND))
        ) {
            self.progress.phase(role).send_replace(Phase::Clean);
        }
        let created = created?;
        let inspected = self
            .docker
            .inspect_container(self.plan.key(role), self.config.deadline())
            .await?;
        let id = self.plan.owned(role, &inspected)?;
        if id.as_str() != created.id {
            return Err(DockerError::Protocol.into());
        }
        self.plan
            .directory
            .know_container(record, id.clone(), self.config.deadline())
            .await?;
        self.progress
            .phase(role)
            .send_replace(Phase::Known(id.clone()));
        self.check(deadline)?;
        Ok(id)
    }
    async fn initialize(
        &self,
        io: &mut IoGuard,
        deadline: Instant,
    ) -> Result<(), EnvironmentError> {
        let initialization_started = Instant::now();
        if let Some(zip) = &self.plan.zip {
            let DeploymentArtifact::Zip(artifact) = &self.plan.revision().artifact else {
                return Err(EnvironmentError::Configuration);
            };
            zip.start(
                StageInput::builder()
                    .artifact(artifact.clone())
                    .deadline(deadline)
                    .global(self.global.subscribe())
                    .stop(self.signals.stop.subscribe())
                    .build(),
                &self.runtime,
            )?;
            if let Err(error) = zip.wait_ready(deadline).await {
                return Err(match error {
                    CodeStageError::AdmissionBusy => EnvironmentError::Busy,
                    CodeStageError::Docker(DockerError::Worker) => {
                        self.health.fail();
                        EnvironmentError::Worker
                    }
                    CodeStageError::Docker(other) => other.into(),
                });
            }
            self.check(deadline)?;
        }
        let relay = self.create(ContainerRole::Relay, deadline).await?;
        self.signals.phase(EnvironmentPhase::Initializing);
        let attached = self.docker.attach(&relay, self.config.deadline()).await?;
        let architecture = match self.plan.revision().architecture {
            Architecture::Arm64 => RuntimeArchitecture::Arm64,
            Architecture::X86_64 => RuntimeArchitecture::X86_64,
        };
        io.control(attached, self.plan.environment_id(), architecture)?;
        self.check(deadline)?;
        self.docker
            .start_container(&relay, self.config.deadline())
            .await?;
        self.check(deadline)?;
        match self.next(io, deadline, None, None).await? {
            ClientEvent::RelayReady => {}
            _ => return Err(DockerError::Protocol.into()),
        }
        let relay_ready = Instant::now();
        let function = self.create(ContainerRole::Function, deadline).await?;
        io.logs(
            self.docker
                .attach_logs(&function, self.config.deadline())
                .await?,
        )?;
        self.check(deadline)?;
        self.docker
            .start_container(&function, self.config.deadline())
            .await?;
        let bootstrap_deadline = deadline.min(Instant::now() + self.config.0.init);
        let ready = self
            .next(io, bootstrap_deadline, Some(&self.cleaner()), None)
            .await
            .map_err(|error| {
                if matches!(error, EnvironmentError::Deadline) && bootstrap_deadline < deadline {
                    EnvironmentError::InitializationTimeout
                } else {
                    error
                }
            })?;
        finish_initialization(
            ready,
            self.plan.zip.is_none().then(|| self.plan.environment_id()),
            initialization_started,
            relay_ready,
        )
    }
    async fn next(
        &self,
        io: &mut IoGuard,
        deadline: Instant,
        health: Option<&Cleaner>,
        mut cancellation: Option<&mut watch::Receiver<bool>>,
    ) -> Result<ClientEvent, EnvironmentError> {
        let mut global = self.global.subscribe();
        let mut stopped = self.signals.stop.subscribe();
        let mut tick = interval(Duration::from_secs(1));
        tick.tick().await;
        loop {
            self.check(deadline)?;
            if cancellation
                .as_ref()
                .is_some_and(|cancel| *cancel.borrow() || cancel.has_changed().is_err())
            {
                return Err(EnvironmentError::Cancelled);
            }
            tokio::select! {biased;
                ()=wait_for_stop(&mut global)=>return Err(EnvironmentError::Cancelled),
                ()=wait_for_stop(&mut stopped)=>return Err(EnvironmentError::Cancelled),
                ()=sleep_until(deadline)=>return Err(EnvironmentError::Deadline),
                ()=async{match &mut cancellation{Some(cancel)=>{let _=cancel.wait_for(|stop|*stop).await;},None=>pending().await,}}=>return Err(EnvironmentError::Cancelled),
                result=io.client()?.next(deadline)=>return result.map_err(EnvironmentError::from),
                _=tick.tick(),if health.is_some()=>{if let Some(health)=health{health.inspect_health().await?;io.healthy().await?;tick.reset();}},
            }
        }
    }
    async fn serve(
        &self,
        io: &mut IoGuard,
        cleaner: &Cleaner,
        mailbox: &mut mpsc::Receiver<Call>,
    ) -> Option<FinalReply> {
        let mut global = self.global.subscribe();
        let mut stopped = self.signals.stop.subscribe();
        let mut tick = interval(Duration::from_secs(1));
        tick.tick().await;
        let mut idle = Instant::now() + self.config.0.idle;
        loop {
            let action = tokio::select! {biased;
                ()=wait_for_stop(&mut global)=>return None,
                ()=wait_for_stop(&mut stopped)=>return None,
                ()=sleep_until(idle)=>return None,
                _=tick.tick()=>{let result=async{cleaner.inspect_health().await?;io.healthy().await}.await;tick.reset();if let Err(error)=result{self.signals.error(error);return None;}continue;},
                result=io.client().ok()?.next(idle)=>{self.signals.error(match result {Ok(event)=>Self::event_failure(event),Err(error)=>error.into()});return None;},
                call=mailbox.recv()=>call,
            };
            let mut call = action?;
            self.signals.phase(EnvironmentPhase::Busy);
            let result = self.call(io, cleaner, &mut call).await;
            let reusable = matches!(
                &result,
                Ok(EnvironmentResponse {
                    result: RuntimeResult::Success(_),
                    ..
                })
            ) && !call.reply.is_closed()
                && io.reusable();
            if reusable {
                self.signals.phase(EnvironmentPhase::Idle);
                let _ = call.reply.send(result);
                idle = Instant::now() + self.config.0.idle;
            } else {
                if let Err(error) = &result {
                    self.signals.error(error.clone());
                }
                return Some((call.reply, result));
            }
        }
    }
    async fn call(
        &self,
        io: &mut IoGuard,
        cleaner: &Cleaner,
        call: &mut Call,
    ) -> Result<EnvironmentResponse, EnvironmentError> {
        let (attempt, deadline, arn, _, _) = call.invocation.parts();
        let name = arn
            .as_ref()
            .split_once(":function:")
            .and_then(|(_, name)| name.split(':').next());
        if name != Some(self.plan.revision().function_name.as_str()) {
            return Err(EnvironmentError::Configuration);
        }
        self.check(deadline)?;
        if *call.cancellation.borrow() || call.reply.is_closed() {
            return Err(EnvironmentError::Cancelled);
        }
        let log_window = io.begin_logs(attempt).await?;
        self.check(deadline)?;
        if *call.cancellation.borrow() || call.reply.is_closed() {
            return Err(EnvironmentError::Cancelled);
        }
        io.client()?
            .invoke(
                call.invocation.clone().with_handler_timeout(
                    HandlerTimeout::try_from(self.plan.revision().timeout)
                        .map_err(|_| EnvironmentError::Configuration)?,
                ),
                deadline.min(self.config.deadline()),
            )
            .await?;
        let result = self
            .next(io, deadline, Some(cleaner), Some(&mut call.cancellation))
            .await;
        match result {
            Ok(ClientEvent::InvocationTimedOut(returned)) if returned == attempt => {
                Err(EnvironmentError::Timeout(self.plan.revision().timeout))
            }
            Ok(ClientEvent::InvocationResult {
                attempt: returned,
                result,
            }) if returned == attempt => {
                let tail = io.finish_logs(log_window).await?;
                let logs = if call.capture_logs {
                    tail
                } else {
                    Bytes::new()
                };
                self.check(deadline)?;
                if *call.cancellation.borrow() || call.reply.is_closed() {
                    return Err(EnvironmentError::Cancelled);
                }
                Ok(EnvironmentResponse { result, logs })
            }
            Ok(event) => Err(Self::event_failure(event)),
            Err(error) => {
                if io
                    .client()?
                    .cancel(attempt, self.config.deadline())
                    .await
                    .is_ok()
                {
                    let _ = io.client()?.next(self.config.deadline()).await;
                }
                Err(error)
            }
        }
    }
    fn event_failure(event: ClientEvent) -> EnvironmentError {
        match event {
            ClientEvent::InitError(payload) if payload.as_ref().len() <= 32768 => {
                EnvironmentError::Initialization(payload)
            }
            _ => DockerError::Protocol.into(),
        }
    }
}

fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos().saturating_add(999) / 1000).unwrap_or(u64::MAX)
}

fn finish_initialization(
    event: ClientEvent,
    image: Option<EnvironmentId>,
    started: Instant,
    relay: Instant,
) -> Result<(), EnvironmentError> {
    match event {
        ClientEvent::BootstrapReady => {
            if let Some(environment) = image {
                tracing::info!(
                    target: "rustack_lambda_core::docker::initialization",
                    environment_id = %environment,
                    relay_micros = micros(relay.duration_since(started)),
                    ric_micros = micros(relay.elapsed()),
                    "Image environment ready"
                );
            }
            Ok(())
        }
        event => Err(Worker::event_failure(event)),
    }
}

#[cfg(test)]
mod timing_tests {
    use std::{
        error::Error,
        io::{self, Write},
        mem,
        sync::mpsc::{SyncSender, sync_channel},
        time::Duration,
    };

    use bytes::Bytes;
    use rustack_lambda_runtime::{
        client::ClientEvent,
        protocol::{EnvironmentId, Payload},
    };
    use tokio::time::{Instant, advance};

    use super::{finish_initialization, micros};

    #[derive(Debug)]
    struct Line {
        bytes: Vec<u8>,
        sender: SyncSender<Vec<u8>>,
    }
    impl Write for Line {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.bytes.len().saturating_add(bytes.len()) > 1024 {
                return Err(io::Error::other("bounded initialization diagnostic"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl Drop for Line {
        fn drop(&mut self) {
            let _ = self.sender.try_send(mem::take(&mut self.bytes));
        }
    }

    #[test]
    fn test_should_round_initialization_durations_up_without_overflow() {
        assert_eq!(micros(Duration::ZERO), 0);
        assert_eq!(micros(Duration::from_nanos(1)), 1);
        assert_eq!(micros(Duration::from_nanos(1000)), 1);
        assert_eq!(micros(Duration::from_nanos(1001)), 2);
        assert_eq!(micros(Duration::MAX), u64::MAX);
    }

    #[tokio::test(start_paused = true)]
    async fn test_should_report_only_successful_image_initialization_without_payloads()
    -> Result<(), Box<dyn Error>> {
        let (sender, received) = sync_channel(4);
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || Line {
                bytes: Vec::new(),
                sender: sender.clone(),
            })
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        let started = Instant::now();
        advance(Duration::from_millis(1)).await;
        let relay = Instant::now();
        advance(Duration::from_millis(2)).await;
        let environment = EnvironmentId::default();
        finish_initialization(
            ClientEvent::BootstrapReady,
            Some(environment),
            started,
            relay,
        )?;
        let line = String::from_utf8(received.try_recv()?)?;
        assert!(line.contains("relay_micros=1000") && line.contains("ric_micros=2000"));
        assert!(line.contains(&environment.to_string()));
        let secret = Payload::try_from(Bytes::from_static(b"runtime-boundary-secret"))?;
        assert!(
            finish_initialization(
                ClientEvent::InitError(secret),
                Some(environment),
                started,
                relay
            )
            .is_err()
        );
        assert!(
            finish_initialization(ClientEvent::RelayReady, Some(environment), started, relay)
                .is_err()
        );
        finish_initialization(ClientEvent::BootstrapReady, None, started, relay)?;
        assert!(received.try_recv().is_err());
        assert!(!line.contains("runtime-boundary-secret"));
        Ok(())
    }
}
