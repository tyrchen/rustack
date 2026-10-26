//! One ordered environment worker; all resources remain registered above its task.

use std::{future::pending, sync::Arc, time::Duration};

use bytes::Bytes;
use http::StatusCode;
use rustack_lambda_runtime::{
    client::ClientEvent,
    protocol::{Architecture as RuntimeArchitecture, HandlerTimeout},
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
    artifact::Architecture,
    docker::{
        DockerError, DockerKey, DockerPort, EnvironmentPlan, connection::stopped as wait_for_stop,
        mutation::MutationGuard, plan::ContainerRole,
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
        self.signals.phase(EnvironmentPhase::Retiring);
        mailbox.close();
        while let Ok(call) = mailbox.try_recv() {
            let _ = call.reply.send(Err(EnvironmentError::Stopped));
        }
        cleaner.retire(&self.io, Some(&mut io)).await;
        if let Some((reply, result)) = pending {
            let _ = reply.send(result);
        }
        Ok(())
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
        match self.next(io, deadline, Some(&self.cleaner()), None).await? {
            ClientEvent::BootstrapReady => Ok(()),
            ClientEvent::InitError(payload) if payload.as_ref().len() <= 32768 => {
                Err(EnvironmentError::Initialization(payload))
            }
            _ => Err(DockerError::Protocol.into()),
        }
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
                result=io.client().ok()?.next(idle)=>{self.signals.error(result.err().map_or(EnvironmentError::Docker(DockerError::Protocol),EnvironmentError::from));return None;},
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
                let tail = io.tail().await?;
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
            Ok(_) => Err(DockerError::Protocol.into()),
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
}
