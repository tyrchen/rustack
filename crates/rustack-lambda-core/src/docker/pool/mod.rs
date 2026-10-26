//! Bounded warm dispatch; external resource ownership stays with `EnvironmentManager`.

mod actor;
use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[cfg(test)]
pub(super) use actor::testing_start;
use rustack_lambda_runtime::session::Invocation;
use tokio::{
    runtime::Handle,
    sync::{Semaphore, mpsc, oneshot, watch},
    task::JoinHandle,
    time::{Instant, timeout_at},
};
use typed_builder::TypedBuilder;

use super::{
    DockerError, EnvironmentError, EnvironmentPlan, EnvironmentPort, EnvironmentResponse,
    check_deadline,
};
use crate::invocation::CleanupTask;

/// Operator limits for waiting calls and retained idle environments.
#[derive(Debug, Clone, Copy, TypedBuilder)]
#[non_exhaustive]
pub struct PoolSettings {
    /// Globally retained idle environments,0..32.
    #[builder(default = 32)]
    pub warm: u8,
    /// Idle environments per immutable identity,0..32.
    #[builder(default = 1)]
    pub warm_per_revision: u8,
    /// Accepted waiting calls across mailbox and actor queue,1..256.
    #[builder(default = 256)]
    pub queue: u16,
}
impl Default for PoolSettings {
    fn default() -> Self {
        Self::builder().build()
    }
}
/// Validated warm retention and admission limits.
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct PoolConfig(PoolSettings);
impl TryFrom<PoolSettings> for PoolConfig {
    type Error = EnvironmentError;
    fn try_from(settings: PoolSettings) -> Result<Self, Self::Error> {
        if settings.warm > 32
            || settings.warm_per_revision > 32
            || !(1..=256).contains(&settings.queue)
        {
            return Err(EnvironmentError::Configuration);
        }
        Ok(Self(settings))
    }
}
/// Read-only pool projection; environment accounting includes quarantine separately.
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct PoolStatus {
    /// Accepted calls still waiting for dispatch.
    pub queued: usize,
    /// Invocation/retirement jobs; at most32.
    pub active: usize,
    /// Idle handles retained for reuse.
    pub warm: usize,
    /// Whether admission remains open and healthy.
    pub accepting: bool,
    /// A worker/root supervision failure closed admission.
    pub worker_failed: bool,
}
struct Offer {
    result: Result<EnvironmentResponse, EnvironmentError>,
    receipt: Option<oneshot::Sender<()>>,
}
struct Call {
    plan: Arc<EnvironmentPlan>,
    invocation: Invocation,
    capture_logs: bool,
    cancellation: watch::Receiver<bool>,
    reply: oneshot::Sender<Offer>,
    queue: Option<tokio::sync::OwnedSemaphorePermit>,
    settlement: watch::Sender<Option<Result<(), EnvironmentError>>>,
}
impl Call {
    fn deadline(&self) -> Instant {
        self.invocation.parts().1
    }
    fn error(&self) -> Option<EnvironmentError> {
        if Instant::now() >= self.deadline() {
            Some(EnvironmentError::Deadline)
        } else if self.reply.is_closed()
            || *self.cancellation.borrow()
            || self.cancellation.has_changed().is_err()
        {
            Some(EnvironmentError::Cancelled)
        } else {
            None
        }
    }
    fn reject(self, error: EnvironmentError) {
        // No environment has been allocated for a rejected queued call.
        self.settlement.send_replace(Some(Ok(())));
        let _ = self.reply.send(Offer {
            result: Err(error),
            receipt: None,
        });
    }
}
/// Exclusive response and retained cleanup receipt for one accepted pool call.
///
/// Drop requests cancellation; only `cancel_and_wait` proves settlement.
#[non_exhaustive]
pub struct PoolInvocation {
    deadline: Instant,
    stop: watch::Sender<bool>,
    response: Option<oneshot::Receiver<Offer>>,
    settlement: watch::Receiver<Option<Result<(), EnvironmentError>>>,
    failed: Arc<AtomicBool>,
    consumed: bool,
}
impl fmt::Debug for PoolInvocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PoolInvocation")
            .field("deadline", &self.deadline)
            .field("consumed", &self.consumed)
            .field("settled", &self.settlement.borrow().is_some())
            .finish_non_exhaustive()
    }
}
impl PoolInvocation {
    /// Consume one result while retaining the receiver across cancelled waiters.
    ///
    /// # Errors
    /// Returns invocation failure, deadline, or unavailable supervision. A closed
    /// response lane is not cleanup proof; use `cancel_and_wait` before releasing
    /// higher-level work permits after cancellation or uncertain failure.
    ///
    /// ```no_run
    /// # async fn result(call: &mut rustack_lambda_core::docker::PoolInvocation) {
    /// let result = call.wait().await;
    /// # }
    /// ```
    pub async fn wait(&mut self) -> Result<EnvironmentResponse, EnvironmentError> {
        if self.consumed || self.response.is_none() {
            return Err(EnvironmentError::Configuration);
        }
        let response = self
            .response
            .as_mut()
            .ok_or(EnvironmentError::Configuration)?;
        let received = timeout_at(self.deadline, response)
            .await
            .map_err(|_| EnvironmentError::Deadline)?;
        self.response.take();
        let offer = received.map_err(|_| {
            if self.failed.load(Ordering::Acquire) {
                EnvironmentError::Worker
            } else {
                EnvironmentError::Stopped
            }
        })?;
        // Remove a completed receiver even when the deadline check rejects its
        // offer. Dropping that offer revokes any unconsumed warm-return receipt.
        if Instant::now() >= self.deadline {
            return Err(EnvironmentError::Deadline);
        }
        if let Some(receipt) = offer.receipt {
            receipt.send(()).map_err(|()| EnvironmentError::Worker)?;
        }
        self.consumed = true;
        offer.result
    }

    /// Request cancellation and await independently owned physical settlement.
    ///
    /// A cancelled/expired waiter can resume with a fresh cleanup deadline.
    /// Consumed success belongs to the pool and cannot be cancelled by this call.
    ///
    /// # Errors
    /// Returns Deadline while cleanup remains retained, or Worker when recovery
    /// was uncertain or the settlement producer disappeared without proof.
    /// Cleanup deadlines over ten minutes return Configuration after signalling.
    ///
    /// ```no_run
    /// # async fn stop(call: &mut rustack_lambda_core::docker::PoolInvocation) {
    /// let settled = call.cancel_and_wait(tokio::time::Instant::now()
    ///     + std::time::Duration::from_secs(60)).await;
    /// # }
    /// ```
    pub async fn cancel_and_wait(&mut self, deadline: Instant) -> Result<(), EnvironmentError> {
        if !self.consumed {
            self.stop.send_replace(true);
            self.response.take();
        }
        Self::check_cleanup_deadline(deadline)?;
        loop {
            let result = self.settlement.borrow().clone();
            if let Some(result) = result {
                return result;
            }
            timeout_at(deadline, self.settlement.changed())
                .await
                .map_err(|_| EnvironmentError::Deadline)?
                .map_err(|_| EnvironmentError::Worker)?;
            Self::check_cleanup_deadline(deadline)?;
        }
    }
    fn check_cleanup_deadline(deadline: Instant) -> Result<(), EnvironmentError> {
        check_deadline(deadline).map_err(|error| match error {
            DockerError::Deadline => EnvironmentError::Deadline,
            DockerError::Configuration => EnvironmentError::Configuration,
            error => error.into(),
        })
    }
}
impl Drop for PoolInvocation {
    fn drop(&mut self) {
        if !self.consumed {
            self.stop.send_replace(true);
        }
    }
}
/// Cloneable call port; the retained pool owner must outlive its shutdown.
#[derive(Clone)]
#[non_exhaustive]
pub struct PoolPort {
    sender: mpsc::Sender<Call>,
    queue: Arc<Semaphore>,
    maximum: usize,
    stop: watch::Sender<bool>,
    failed: Arc<AtomicBool>,
    status: watch::Receiver<PoolStatus>,
}
impl fmt::Debug for PoolPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PoolPort")
            .field("status", &self.status())
            .finish()
    }
}
impl PoolPort {
    /// Execute once through a matching warm environment or bounded cold allocation.
    ///
    /// # Errors
    /// Returns queue overload, deadline, cancellation, initialization or runtime
    /// failure. Dropping the future signals retained cleanup without replay.
    ///
    /// ```no_run
    /// # async fn invoke(port: rustack_lambda_core::docker::PoolPort, plan: rustack_lambda_core::docker::EnvironmentPlan, call: rustack_lambda_runtime::session::Invocation) {
    /// let result = port.invoke(plan, call, false).await;
    /// # }
    /// ```
    pub async fn invoke(
        &self,
        plan: EnvironmentPlan,
        invocation: Invocation,
        capture_logs: bool,
    ) -> Result<EnvironmentResponse, EnvironmentError> {
        self.start(plan, invocation, capture_logs)?.wait().await
    }
    /// Admit once and retain separate response/cancellation-settlement ownership.
    ///
    /// # Errors
    /// Rejects expired calls, unavailable image pins, overload or closed admission
    /// before enqueueing. Scope validation also occurs before physical allocation.
    ///
    /// ```no_run
    /// # fn start(port: &rustack_lambda_core::docker::PoolPort, plan: rustack_lambda_core::docker::EnvironmentPlan, event: rustack_lambda_runtime::session::Invocation) {
    /// let call = port.start(plan, event, false);
    /// # }
    /// ```
    pub fn start(
        &self,
        plan: EnvironmentPlan,
        invocation: Invocation,
        capture_logs: bool,
    ) -> Result<PoolInvocation, EnvironmentError> {
        let deadline = invocation.parts().1;
        if Instant::now() >= deadline {
            return Err(EnvironmentError::Deadline);
        }
        if !self.status().accepting {
            return Err(self.closed_error());
        }
        plan.live()?;
        let queue = self
            .queue
            .clone()
            .try_acquire_owned()
            .map_err(|_| EnvironmentError::Busy)?;
        let (stop, cancellation) = watch::channel(false);
        let (reply, response) = oneshot::channel();
        let (settlement, observed) = watch::channel(None);
        self.sender
            .try_send(Call {
                plan: Arc::new(plan),
                invocation,
                capture_logs,
                cancellation,
                reply,
                queue: Some(queue),
                settlement,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => EnvironmentError::Busy,
                mpsc::error::TrySendError::Closed(_) => self.closed_error(),
            })?;
        Ok(PoolInvocation {
            deadline,
            stop,
            response: Some(response),
            settlement: observed,
            failed: self.failed.clone(),
            consumed: false,
        })
    }
    fn closed_error(&self) -> EnvironmentError {
        if self.failed.load(Ordering::Acquire) {
            EnvironmentError::Worker
        } else {
            EnvironmentError::Stopped
        }
    }
    /// Read pool counts without waiting for the business mailbox.
    ///
    /// ```no_run
    /// # fn status(port: &rustack_lambda_core::docker::PoolPort) { let counts = port.status(); }
    /// ```
    #[must_use]
    pub fn status(&self) -> PoolStatus {
        let mut status = *self.status.borrow();
        status.queued = self.maximum.saturating_sub(self.queue.available_permits());
        status.worker_failed = self.failed.load(Ordering::Acquire);
        status.accepting = !*self.stop.borrow() && !status.worker_failed;
        status
    }
}
/// Retained pool root, shut down before the environment and image owners.
#[non_exhaustive]
pub struct EnvironmentPool {
    port: PoolPort,
    task: Option<JoinHandle<Result<(), EnvironmentError>>>,
    primary: Option<Result<(), EnvironmentError>>,
    settled: Option<Result<(), EnvironmentError>>,
    emergency: Arc<CleanupTask>,
}
impl fmt::Debug for EnvironmentPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnvironmentPool")
            .field("port", &self.port)
            .field("settled", &self.settled)
            .finish_non_exhaustive()
    }
}
impl EnvironmentPool {
    /// Start bounded checkout over an already-owned environment service.
    ///
    /// # Errors
    /// Returns Worker when no Tokio runtime is available. This does not establish
    /// Docker startup readiness or enable the production executor.
    ///
    /// ```no_run
    /// # fn start(environments: rustack_lambda_core::docker::EnvironmentPort) {
    /// let pool = rustack_lambda_core::docker::EnvironmentPool::start(environments, Default::default());
    /// # }
    /// ```
    pub fn start(
        environments: EnvironmentPort,
        config: PoolConfig,
    ) -> Result<Self, EnvironmentError> {
        let runtime = Handle::try_current().map_err(|_| EnvironmentError::Worker)?;
        let maximum = usize::from(config.0.queue);
        let (sender, inbox) = mpsc::channel(maximum);
        let (stop, _) = watch::channel(false);
        let failed = Arc::new(AtomicBool::new(false));
        let (status, observed) = watch::channel(PoolStatus::default());
        let emergency = Arc::new(CleanupTask::default());
        let actor = actor::Actor::builder()
            .environments(environments)
            .config(config)
            .inbox(inbox)
            .stop(stop.clone())
            .failed(failed.clone())
            .status(status)
            .emergency(emergency.clone())
            .runtime(runtime.clone())
            .build();
        let port = PoolPort {
            sender,
            queue: Arc::new(Semaphore::new(maximum)),
            maximum,
            stop,
            failed,
            status: observed,
        };
        Ok(Self {
            port,
            task: Some(runtime.spawn(actor.run())),
            primary: None,
            settled: None,
            emergency,
        })
    }
    /// Clone the call port without transferring root shutdown ownership.
    ///
    /// ```no_run
    /// # fn port(pool: &rustack_lambda_core::docker::EnvironmentPool) { let port = pool.port(); }
    /// ```
    #[must_use]
    pub fn port(&self) -> PoolPort {
        self.port.clone()
    }
    /// Close admission, retire active/idle environments and join retained cleanup.
    ///
    /// # Errors
    /// Returns timeout while cleanup stays owned, or a cached supervision failure.
    /// A cancelled waiter does not lose the join or emergency cleanup receipt.
    ///
    /// ```no_run
    /// # async fn stop(pool: &mut rustack_lambda_core::docker::EnvironmentPool) {
    /// let clean = pool.shutdown(tokio::time::Instant::now()+std::time::Duration::from_secs(60)).await;
    /// # }
    /// ```
    pub async fn shutdown(&mut self, deadline: Instant) -> Result<(), EnvironmentError> {
        self.port.stop.send_replace(true);
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
impl Drop for EnvironmentPool {
    fn drop(&mut self) {
        self.port.stop.send_replace(true);
    }
}

#[cfg(test)]
mod tests;
