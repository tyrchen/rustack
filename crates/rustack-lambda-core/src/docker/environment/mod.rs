//! Retained two-container environments; executor pooling and startup readiness are separate.

mod actor;
mod cleanup;
mod io;
mod worker;

use std::{
    fmt,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use actor::{Actor, Budget, Envelope, Health, Signals};
use bytes::Bytes;
use rustack_lambda_runtime::{
    protocol::{EnvironmentId, Payload},
    relay::RelayError,
    session::{Invocation, RuntimeResult},
};
use tokio::{
    runtime::Handle,
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
    time::{Instant, timeout_at},
};
use typed_builder::TypedBuilder;

use super::{DirectoryLease, DockerError, DockerPort, EnvironmentPlan, check_deadline};
use crate::invocation::CleanupTask;

/// Raw operator budgets; convert to `EnvironmentConfig` before starting ownership.
#[derive(Debug, Clone, Copy, TypedBuilder)]
#[non_exhaustive]
pub struct EnvironmentSettings {
    /// Count of allocated environments, including quarantine, in1..32.
    #[builder(default = 32)]
    pub environments: u8,
    /// Aggregate function+relay memory,256MiB..1TiB.
    #[builder(default = 8 * 1024 * 1024 * 1024)]
    pub memory_bytes: u64,
    /// Independent Docker/control IO budget,1..60s.
    #[builder(default = Duration::from_secs(5))]
    pub io: Duration,
    /// Cold startup budget,1..60s, also bounded by caller admission deadline.
    #[builder(default = Duration::from_secs(15))]
    pub start: Duration,
    /// Idle lifetime,1..86400s.
    #[builder(default = Duration::from_secs(600))]
    pub idle: Duration,
    /// Retained bytes per function-output channel,4KiB..1MiB.
    #[builder(default = 262_144)]
    pub log_ring_bytes: usize,
}
impl Default for EnvironmentSettings {
    fn default() -> Self {
        Self::builder().build()
    }
}

/// Closed, validated environment ownership budgets.
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct EnvironmentConfig(EnvironmentSettings);
impl TryFrom<EnvironmentSettings> for EnvironmentConfig {
    type Error = EnvironmentError;
    fn try_from(settings: EnvironmentSettings) -> Result<Self, Self::Error> {
        if !(1..=32).contains(&settings.environments)
            || !(256 * 1024 * 1024..=1024 * 1024 * 1024 * 1024).contains(&settings.memory_bytes)
            || !(4096..=1024 * 1024).contains(&settings.log_ring_bytes)
        {
            return Err(EnvironmentError::Configuration);
        }
        for (duration, maximum) in [
            (settings.io, 60),
            (settings.start, 60),
            (settings.idle, 86400),
        ] {
            if duration.subsec_nanos() != 0 || !(1..=maximum).contains(&duration.as_secs()) {
                return Err(EnvironmentError::Configuration);
            }
        }
        Ok(Self(settings))
    }
}
impl EnvironmentConfig {
    fn deadline(self) -> Instant {
        Instant::now() + self.0.io
    }
}

/// Environment errors retain typed sources and never display peer payload bytes.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum EnvironmentError {
    /// Invalid settings, scope, captured invocation or lifecycle operation.
    #[error("invalid environment configuration")]
    Configuration,
    /// Allocation or command capacity is exhausted.
    #[error("environment capacity exhausted")]
    Busy,
    /// Explicit cancellation or abandoned owner/caller.
    #[error("environment cancelled")]
    Cancelled,
    /// Startup, call or waiting deadline expired.
    #[error("environment deadline exceeded")]
    Deadline,
    /// This owner no longer accepts work.
    #[error("environment is stopped")]
    Stopped,
    /// A worker/root/cleanup supervisor failed.
    #[error("environment supervision failed")]
    Worker,
    /// Function reported a bounded initialization error.
    #[error("function initialization failed")]
    Initialization(Payload),
    /// Function exited before its expected result/readiness.
    #[error("function exited with status {exit_code}; oom={oom}")]
    Exited {
        /// Daemon exit status, without peer text.
        exit_code: i64,
        /// Daemon-reported OOM kill flag.
        oom: bool,
    },
    /// Validated local Docker transport failure.
    #[error(transparent)]
    Docker(#[from] DockerError),
    /// Ordered relay session failure.
    #[error(transparent)]
    Relay(#[from] RelayError),
}

/// Current ownership/lifecycle state, independent of external API readiness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EnvironmentPhase {
    /// An allocation is registered and creating its relay.
    Allocating,
    /// Relay/function initialization is in progress.
    Initializing,
    /// Ready for one invocation.
    Idle,
    /// An admitted invocation is active.
    Busy,
    /// All owned resources are being retired.
    Retiring,
    /// Failed/uncertain cleanup retains its allocation reservation.
    Quarantined,
    /// Containers and retained IO are confirmed settled.
    Removed,
}
/// Redacted lifecycle projection; a business failure does not imply unowned cleanup.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct EnvironmentStatus {
    /// Current owner state.
    pub phase: EnvironmentPhase,
    /// Primary failure, retained across cleanup retries.
    pub error: Option<EnvironmentError>,
}
/// Allocation accounting includes every queued, live and quarantined reservation.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct EnvironmentCapacity {
    /// Number of retained allocation slots.
    pub allocated: usize,
    /// Aggregate reserved memory bytes, including relay overhead.
    pub memory_bytes: u64,
    /// Environments retaining failed cleanup.
    pub quarantined: usize,
    /// Whether admission is open and healthy.
    pub accepting: bool,
    /// A supervision failure permanently closed admission.
    pub worker_failed: bool,
}
/// Actual handler result and bounded optional log output; Debug redacts user bytes.
#[derive(Clone)]
#[non_exhaustive]
pub struct EnvironmentResponse {
    /// Unmodified handler success or `FunctionError` payload.
    pub result: RuntimeResult,
    /// Combined captured final 4 KiB when requested; per-invocation attribution is separate.
    pub logs: Bytes,
}
impl fmt::Debug for EnvironmentResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnvironmentResponse")
            .field("result", &self.result)
            .field("log_bytes", &self.logs.len())
            .finish()
    }
}
struct Call {
    invocation: Invocation,
    capture_logs: bool,
    cancellation: watch::Receiver<bool>,
    reply: oneshot::Sender<Result<EnvironmentResponse, EnvironmentError>>,
}
struct CancelOnDrop {
    call: watch::Sender<bool>,
    environment: watch::Sender<bool>,
    complete: bool,
}
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if !self.complete {
            self.call.send_replace(true);
            self.environment.send_replace(true);
        }
    }
}

/// Exclusive owner-facing environment handle; Drop requests retained retirement.
#[non_exhaustive]
pub struct EnvironmentHandle {
    id: EnvironmentId,
    calls: mpsc::Sender<Call>,
    signals: Signals,
    settlement: watch::Receiver<Option<Result<(), EnvironmentError>>>,
}
impl fmt::Debug for EnvironmentHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnvironmentHandle")
            .field("id", &self.id)
            .field("status", &*self.signals.status.borrow())
            .finish_non_exhaustive()
    }
}
impl EnvironmentHandle {
    /// Read the fresh immutable environment identity.
    ///
    /// ```no_run
    /// # fn id(environment: &rustack_lambda_core::docker::EnvironmentHandle) { let id = environment.id(); }
    /// ```
    #[must_use]
    pub fn id(&self) -> EnvironmentId {
        self.id
    }
    /// Read lifecycle status without waiting for a business mailbox.
    ///
    /// ```no_run
    /// # fn status(environment: &rustack_lambda_core::docker::EnvironmentHandle) { let status = environment.status(); }
    /// ```
    #[must_use]
    pub fn status(&self) -> EnvironmentStatus {
        self.signals.status.borrow().clone()
    }
    /// Request retirement through its priority stop signal.
    ///
    /// ```no_run
    /// # fn stop(environment: &rustack_lambda_core::docker::EnvironmentHandle) { environment.cancel(); }
    /// ```
    pub fn cancel(&self) {
        self.signals.stop.send_replace(true);
    }
    /// Await readiness without cancelling an accepted allocation when dropped.
    ///
    /// # Errors
    /// Returns invalid/expired budget, startup failure or owner failure. The owner
    /// remains responsible for shutdown/Drop and retained cleanup.
    ///
    /// ```no_run
    /// # async fn ready(environment: &rustack_lambda_core::docker::EnvironmentHandle) { let ready = environment.wait_ready(tokio::time::Instant::now()+std::time::Duration::from_secs(15)).await; }
    /// ```
    pub async fn wait_ready(&self, deadline: Instant) -> Result<(), EnvironmentError> {
        check_deadline(deadline)?;
        let mut status = self.signals.status.subscribe();
        let observed = timeout_at(
            deadline,
            status.wait_for(|status| {
                !matches!(
                    status.phase,
                    EnvironmentPhase::Allocating | EnvironmentPhase::Initializing
                )
            }),
        )
        .await
        .map_err(|_| EnvironmentError::Deadline)?
        .map_err(|_| EnvironmentError::Worker)?
        .clone();
        if Instant::now() >= deadline {
            return Err(EnvironmentError::Deadline);
        }
        if matches!(
            observed.phase,
            EnvironmentPhase::Idle | EnvironmentPhase::Busy
        ) {
            Ok(())
        } else {
            Err(observed.error.unwrap_or(EnvironmentError::Stopped))
        }
    }
    /// Submit one runtime invocation to a ready environment, without replay.
    ///
    /// # Errors
    /// Returns bounded admission, deadline/cancellation, protocol or function-exit
    /// errors. Dropping this future retires the environment; shutdown separately
    /// awaits cleanup. The supplied absolute invocation deadline is authoritative.
    ///
    /// ```no_run
    /// # async fn invoke(environment: &mut rustack_lambda_core::docker::EnvironmentHandle, invocation: rustack_lambda_runtime::session::Invocation) { let result = environment.invoke(invocation, true).await; }
    /// ```
    pub async fn invoke(
        &mut self,
        invocation: Invocation,
        capture_logs: bool,
    ) -> Result<EnvironmentResponse, EnvironmentError> {
        let deadline = invocation.parts().1;
        if Instant::now() >= deadline {
            return Err(EnvironmentError::Deadline);
        }
        if self.status().phase != EnvironmentPhase::Idle {
            return Err(EnvironmentError::Stopped);
        }
        let (stop, cancellation) = watch::channel(false);
        let mut cancel = CancelOnDrop {
            call: stop,
            environment: self.signals.stop.clone(),
            complete: false,
        };
        let (reply, response) = oneshot::channel();
        self.calls
            .try_send(Call {
                invocation,
                capture_logs,
                cancellation,
                reply,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => EnvironmentError::Busy,
                mpsc::error::TrySendError::Closed(_) => EnvironmentError::Stopped,
            })?;
        let result = timeout_at(deadline, response)
            .await
            .map_err(|_| EnvironmentError::Deadline)?
            .map_err(|_| EnvironmentError::Worker)?;
        if Instant::now() >= deadline {
            return Err(EnvironmentError::Deadline);
        }
        cancel.complete = result.is_ok();
        result
    }
    /// Request stop and await retained container/IO settlement under one deadline.
    ///
    /// # Errors
    /// Returns timeout while ownership remains retained, or cached supervision
    /// failure. Business failure remains in status, independent of clean retirement.
    ///
    /// ```no_run
    /// # async fn stop(environment: &mut rustack_lambda_core::docker::EnvironmentHandle) { let clean = environment.shutdown(tokio::time::Instant::now()+std::time::Duration::from_secs(10)).await; }
    /// ```
    pub async fn shutdown(&mut self, deadline: Instant) -> Result<(), EnvironmentError> {
        self.cancel();
        check_deadline(deadline)?;
        let settled = timeout_at(deadline, self.settlement.wait_for(Option::is_some))
            .await
            .map_err(|_| EnvironmentError::Deadline)?
            .map_err(|_| EnvironmentError::Worker)?
            .clone()
            .ok_or(EnvironmentError::Worker)?;
        if Instant::now() >= deadline {
            return Err(EnvironmentError::Deadline);
        }
        settled
    }
}
impl Drop for EnvironmentHandle {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Cloneable bounded allocation port; retain its manager through shutdown.
#[derive(Clone, TypedBuilder)]
#[builder(
    builder_method(vis = ""),
    build_method(vis = ""),
    builder_type(vis = "")
)]
#[non_exhaustive]
pub struct EnvironmentPort {
    sender: mpsc::Sender<Envelope>,
    directory: DirectoryLease,
    config: EnvironmentConfig,
    control: watch::Sender<bool>,
    budget: Arc<Budget>,
    health: Arc<Health>,
}
impl fmt::Debug for EnvironmentPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnvironmentPort")
            .field("capacity", &self.status())
            .finish_non_exhaustive()
    }
}
impl EnvironmentPort {
    /// Reserve count/memory and register a plan before any container operation.
    ///
    /// # Errors
    /// Returns invalid scope/deadline, closed/unhealthy owner or bounded capacity.
    /// Startup runs under the earlier of admission deadline and configured budget.
    ///
    /// ```no_run
    /// # fn allocate(port: &rustack_lambda_core::docker::EnvironmentPort, plan: rustack_lambda_core::docker::EnvironmentPlan) { let environment = port.spawn(plan,tokio::time::Instant::now()+std::time::Duration::from_secs(15)); }
    /// ```
    pub fn spawn(
        &self,
        plan: EnvironmentPlan,
        deadline: Instant,
    ) -> Result<EnvironmentHandle, EnvironmentError> {
        let deadline = deadline.min(Instant::now() + self.config.0.start);
        check_deadline(deadline)?;
        if !plan.same_scope(&self.directory) {
            return Err(EnvironmentError::Configuration);
        }
        if *self.control.borrow() || self.health.failed.load(Ordering::Acquire) {
            return Err(EnvironmentError::Stopped);
        }
        if self.health.quarantined.load(Ordering::Acquire) > 0 {
            return Err(DockerError::Quarantined.into());
        }
        let reservation = self.budget.reserve(plan.memory_bytes())?;
        let plan = Arc::new(plan);
        let (calls, mailbox) = mpsc::channel(4);
        let signals = Signals::new();
        let (done, settlement) = watch::channel(None);
        let handle = EnvironmentHandle {
            id: plan.environment_id(),
            calls,
            signals: signals.clone(),
            settlement,
        };
        self.sender
            .try_send(Envelope {
                plan,
                mailbox,
                signals,
                reservation,
                done,
                deadline: deadline.min(Instant::now() + self.config.0.start),
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => EnvironmentError::Busy,
                mpsc::error::TrySendError::Closed(_) => EnvironmentError::Stopped,
            })?;
        Ok(handle)
    }
    /// Read bounded resource accounting, including quarantine reservations.
    ///
    /// ```no_run
    /// # fn status(port: &rustack_lambda_core::docker::EnvironmentPort) { let capacity = port.status(); }
    /// ```
    #[must_use]
    pub fn status(&self) -> EnvironmentCapacity {
        EnvironmentCapacity {
            allocated: self.budget.count(),
            memory_bytes: self.budget.memory.load(Ordering::Acquire),
            quarantined: self.health.quarantined.load(Ordering::Acquire),
            accepting: !*self.control.borrow()
                && !self.health.failed.load(Ordering::Acquire)
                && self.health.quarantined.load(Ordering::Acquire) == 0,
            worker_failed: self.health.failed.load(Ordering::Acquire),
        }
    }
}

/// Retained root owner; startup reconciliation/probes and executor pooling are separate.
#[non_exhaustive]
pub struct EnvironmentManager {
    port: EnvironmentPort,
    task: Option<JoinHandle<Result<(), EnvironmentError>>>,
    primary: Option<Result<(), EnvironmentError>>,
    settled: Option<Result<(), EnvironmentError>>,
    emergency: Arc<CleanupTask>,
}
impl fmt::Debug for EnvironmentManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnvironmentManager")
            .field("port", &self.port)
            .field("settled", &self.settled)
            .finish_non_exhaustive()
    }
}
impl EnvironmentManager {
    /// Start ownership on the current runtime under a real directory capability.
    ///
    /// # Errors
    /// Returns Worker when a Tokio runtime is unavailable. This alone does not
    /// establish historical reconciliation or production Docker readiness.
    ///
    /// ```no_run
    /// # fn start(docker: rustack_lambda_core::docker::DockerPort, lease: rustack_lambda_core::docker::DirectoryLease) { let manager = rustack_lambda_core::docker::EnvironmentManager::start(docker,lease,Default::default()); }
    /// ```
    pub fn start(
        docker: DockerPort,
        directory: DirectoryLease,
        config: EnvironmentConfig,
    ) -> Result<Self, EnvironmentError> {
        let runtime = Handle::try_current().map_err(|_| EnvironmentError::Worker)?;
        let (sender, inbox) = mpsc::channel(32);
        let (control, _) = watch::channel(false);
        let health = Arc::new(Health::new(control.clone()));
        let budget = Arc::new(Budget::new(config));
        let emergency = Arc::new(CleanupTask::default());
        let port = EnvironmentPort {
            sender,
            directory,
            config,
            control: control.clone(),
            budget,
            health: health.clone(),
        };
        let actor = Actor::builder()
            .docker(docker)
            .config(config)
            .inbox(inbox)
            .control(control)
            .health(health)
            .runtime(runtime.clone())
            .emergency(emergency.clone())
            .build();
        Ok(Self {
            port,
            task: Some(runtime.spawn(actor.run())),
            primary: None,
            settled: None,
            emergency,
        })
    }
    /// Clone the allocation port without transferring root shutdown ownership.
    ///
    /// ```no_run
    /// # fn port(manager: &rustack_lambda_core::docker::EnvironmentManager) { let port = manager.port(); }
    /// ```
    #[must_use]
    pub fn port(&self) -> EnvironmentPort {
        self.port.clone()
    }
    /// Stop admission and join normal/emergency cleanup without dropping records.
    ///
    /// # Errors
    /// Returns timeout while cleanup stays owned, or cached worker/root failure.
    /// Must finish before image owner or `DockerWire` shutdown.
    ///
    /// ```no_run
    /// # async fn stop(manager: &mut rustack_lambda_core::docker::EnvironmentManager) { let result = manager.shutdown(tokio::time::Instant::now()+std::time::Duration::from_secs(10)).await; }
    /// ```
    pub async fn shutdown(&mut self, deadline: Instant) -> Result<(), EnvironmentError> {
        self.port.control.send_replace(true);
        check_deadline(deadline)?;
        if let Some(settled) = &self.settled {
            return settled.clone();
        }
        if self.primary.is_none() {
            let result = match self.task.as_mut() {
                Some(task) => timeout_at(deadline, task)
                    .await
                    .map_err(|_| EnvironmentError::Deadline)?
                    .map_err(|_| EnvironmentError::Worker)
                    .and_then(|result| result),
                None => Err(EnvironmentError::Worker),
            };
            self.task.take();
            self.primary = Some(result);
        }
        timeout_at(deadline, self.emergency.wait())
            .await
            .map_err(|_| EnvironmentError::Deadline)?
            .map_err(|_| EnvironmentError::Worker)?;
        if Instant::now() >= deadline {
            return Err(EnvironmentError::Deadline);
        }
        let result = self.primary.clone().ok_or(EnvironmentError::Worker)?;
        self.settled = Some(result.clone());
        result
    }
}
impl Drop for EnvironmentManager {
    fn drop(&mut self) {
        self.port.control.send_replace(true);
    }
}

#[cfg(test)]
mod tests;
