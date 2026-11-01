//! Retained Step Functions → actual Lambda provider composition.
//!
//! The port queues synchronously. Its independent manager retains each start,
//! response and cleanup receipt; dropping an ASL observer never drops the real
//! invocation future. No SDK, HTTP loopback or disabled-executor fallback is used.

use std::{
    collections::HashMap,
    fmt,
    mem::replace,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use rustack_lambda_core::{
    executor::ExecutorBackend, invocation::InvocationHandle, provider::RustackLambda,
    resolver::function_arn,
};
use rustack_stepfunctions_core::{
    LambdaTaskCommand, LambdaTaskPort, QuarantinedTask, TaskCall, TaskFailure, TaskResponder,
    TenantScope,
};
use tokio::{
    runtime::Handle,
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, watch},
    task::{Id, JoinError, JoinHandle, JoinSet},
    time::{Instant, timeout_at},
};

mod error;
mod mapping;
#[cfg(test)]
mod tests;
mod worker;

pub use error::BridgeError;

/// Validated accepted-work and independent cleanup wait bounds.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct BridgeLimits {
    accepted: usize,
    cleanup: Duration,
}
impl BridgeLimits {
    /// Validate1..=1,024 accepted calls and1..=60s cleanup observation.
    ///
    /// # Errors
    /// Rejects zero/excessive capacities or cleanup durations.
    ///
    /// ```
    /// let limits=rustack_cli::stepfunctions_bridge::BridgeLimits::new(128,std::time::Duration::from_secs(10))?;
    /// # Ok::<(),rustack_cli::stepfunctions_bridge::BridgeError>(())
    /// ```
    pub fn new(accepted: usize, cleanup: Duration) -> Result<Self, BridgeError> {
        if !(1..=1024).contains(&accepted)
            || !(Duration::from_secs(1)..=Duration::from_secs(60)).contains(&cleanup)
        {
            return Err(BridgeError::Configuration);
        }
        Ok(Self { accepted, cleanup })
    }
}
impl Default for BridgeLimits {
    fn default() -> Self {
        Self {
            accepted: 128,
            cleanup: Duration::from_secs(10),
        }
    }
}
struct Context {
    provider: Arc<RustackLambda>,
    scope: TenantScope,
    limits: BridgeLimits,
}
impl fmt::Debug for Context {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LambdaBridgeContext")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}
#[derive(Debug)]
struct Shared {
    context: Context,
    accepting: AtomicBool,
    unhealthy: AtomicBool,
    capacity: Arc<Semaphore>,
    diagnostic: watch::Sender<Option<Arc<BridgeError>>>,
}
#[derive(Debug)]
struct Submission {
    command: LambdaTaskCommand,
    responder: TaskResponder,
    permit: OwnedSemaphorePermit,
}
#[derive(Debug)]
struct Retained {
    _permit: OwnedSemaphorePermit,
    _resource: Option<Uncertain>,
}
#[derive(Debug)]
struct Uncertain {
    _handle: InvocationHandle,
    _receipt: QuarantinedTask,
}
#[derive(Debug)]
struct WorkerExit {
    uncertain: Option<Uncertain>,
    diagnostic: Option<Arc<BridgeError>>,
}
#[derive(Debug)]
struct ManagerState {
    queue: mpsc::Receiver<Submission>,
    stop: watch::Receiver<bool>,
    jobs: JoinSet<WorkerExit>,
    active: HashMap<Id, OwnedSemaphorePermit>,
    quarantine: Vec<Retained>,
}
#[derive(Debug)]
struct ManagerOwner {
    state: ManagerState,
    shared: Arc<Shared>,
}

/// Synchronous bounded adapter, installed only for an explicitly selected backend.
#[derive(Debug)]
#[non_exhaustive]
pub struct StepFunctionsLambdaBridge {
    queue: mpsc::Sender<Submission>,
    shared: Arc<Shared>,
}
/// Independent join controller; a dropped waiter retains its exact manager handle.
#[derive(Debug)]
#[non_exhaustive]
pub struct LambdaBridgeSupervisor {
    stop: watch::Sender<bool>,
    manager: Option<JoinHandle<Result<(), Arc<BridgeError>>>>,
    shared: Arc<Shared>,
    joined: Option<Result<(), Arc<BridgeError>>>,
}
impl StepFunctionsLambdaBridge {
    /// Read actual port readiness without acquiring task admission.
    ///
    /// ```
    /// # fn inspect(port:&rustack_cli::stepfunctions_bridge::StepFunctionsLambdaBridge){let ready=port.is_ready();}
    /// ```
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.shared.accepting.load(Ordering::Acquire)
            && !self.shared.unhealthy.load(Ordering::Acquire)
    }
    /// Start one retained manager before exposing the task port.
    ///
    /// The caller injects an actual provider and its trusted tenant scope. This
    /// exports a composition adapter; it does not register a gateway service.
    ///
    /// # Errors
    /// Returns Unavailable outside an active Tokio runtime.
    ///
    /// ```no_run
    /// # fn build(provider:std::sync::Arc<rustack_lambda_core::provider::RustackLambda>) -> Result<(),rustack_cli::stepfunctions_bridge::BridgeError> {
    /// use rustack_cli::stepfunctions_bridge::{StepFunctionsLambdaBridge,BridgeLimits};
    /// let (port,owner)=StepFunctionsLambdaBridge::new(provider,rustack_stepfunctions_core::TenantScope::default(),BridgeLimits::default())?;
    /// # Ok(()) }
    /// ```
    pub fn new(
        provider: Arc<RustackLambda>,
        scope: TenantScope,
        limits: BridgeLimits,
    ) -> Result<(Arc<Self>, LambdaBridgeSupervisor), BridgeError> {
        let config = provider.config();
        let expected = format!(
            "arn:{}:lambda:{}:{}:function:",
            scope.partition(),
            scope.region(),
            scope.account()
        );
        if config.executor == ExecutorBackend::Disabled
            || (config.executor == ExecutorBackend::Docker
                && provider.executor().image_available().is_err())
            || !function_arn(
                &config.default_region,
                &config.account_id,
                "BridgeScopeProbe",
            )
            .starts_with(&expected)
        {
            return Err(BridgeError::Configuration);
        }
        let runtime = Handle::try_current().map_err(|_| BridgeError::Unavailable)?;
        let (queue, receiver) = mpsc::channel(limits.accepted);
        let (stop, cancelled) = watch::channel(false);
        let (diagnostic, _) = watch::channel(None);
        let shared = Arc::new(Shared {
            context: Context {
                provider,
                scope,
                limits,
            },
            accepting: AtomicBool::new(true),
            unhealthy: AtomicBool::new(false),
            capacity: Arc::new(Semaphore::new(limits.accepted)),
            diagnostic,
        });
        let owner = ManagerOwner {
            state: ManagerState {
                queue: receiver,
                stop: cancelled,
                jobs: JoinSet::new(),
                active: HashMap::new(),
                quarantine: Vec::new(),
            },
            shared: shared.clone(),
        };
        let manager = runtime.spawn(owner.run());
        Ok((
            Arc::new(Self {
                queue,
                shared: shared.clone(),
            }),
            LambdaBridgeSupervisor {
                stop,
                manager: Some(manager),
                shared,
                joined: None,
            },
        ))
    }
}
impl LambdaTaskPort for StepFunctionsLambdaBridge {
    fn start(&self, command: LambdaTaskCommand) -> Result<TaskCall, TaskFailure> {
        let scope = command.machine().scope();
        let configured = &self.shared.context.scope;
        if !self.shared.accepting.load(Ordering::Acquire)
            || self.queue.is_closed()
            || scope.partition() != configured.partition()
            || scope.region() != configured.region()
            || scope.account() != configured.account()
            || Instant::now() >= Instant::from_std(command.dispatch().deadline())
        {
            return Err(TaskFailure::service_exception());
        }
        let permit = self
            .shared
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| TaskFailure::throttled())?;
        let (call, responder) = TaskCall::channel();
        self.queue
            .try_send(Submission {
                command,
                responder,
                permit,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => TaskFailure::throttled(),
                mpsc::error::TrySendError::Closed(_) => TaskFailure::service_exception(),
            })?;
        Ok(call)
    }
}
impl LambdaBridgeSupervisor {
    /// Stop accepting new tasks and request cancellation of all owned workers.
    ///
    /// ```
    /// # fn close(owner:&rustack_cli::stepfunctions_bridge::LambdaBridgeSupervisor) {owner.close();}
    /// ```
    pub fn close(&self) {
        self.shared.accepting.store(false, Ordering::Release);
        self.stop.send_replace(true);
    }
    /// Join the same manager through an absolute deadline, retaining it on expiry.
    ///
    /// Call before quiescing the underlying Lambda provider.
    ///
    /// # Errors
    /// Returns Deadline, a supervised worker source, or retained Quarantined status.
    ///
    /// ```no_run
    /// # async fn drain(owner:&mut rustack_cli::stepfunctions_bridge::LambdaBridgeSupervisor,deadline:tokio::time::Instant) {
    /// let result=owner.shutdown(deadline).await;
    /// # }
    /// ```
    pub async fn shutdown(&mut self, deadline: Instant) -> Result<(), Arc<BridgeError>> {
        self.close();
        if let Some(result) = &self.joined {
            return result.clone();
        }
        let Some(manager) = &mut self.manager else {
            return Err(Arc::new(BridgeError::Unavailable));
        };
        let result = timeout_at(deadline, manager)
            .await
            .map_err(|_| Arc::new(BridgeError::Deadline))?;
        self.manager = None;
        let result = result.unwrap_or_else(|error| Err(Arc::new(BridgeError::Worker(error))));
        if let Err(error) = &result {
            self.shared.diagnostic.send_replace(Some(error.clone()));
        }
        self.joined = Some(result.clone());
        result
    }
    /// Read the most recent redacted internal source; never a public ASL cause.
    ///
    /// ```
    /// # fn inspect(owner:&rustack_cli::stepfunctions_bridge::LambdaBridgeSupervisor) {let source=owner.diagnostic();}
    /// ```
    #[must_use]
    pub fn diagnostic(&self) -> Option<Arc<BridgeError>> {
        self.shared.diagnostic.borrow().clone()
    }
    /// Read accepted/uncertain calls still holding bridge admission capacity.
    ///
    /// ```
    /// # fn inspect(owner:&rustack_cli::stepfunctions_bridge::LambdaBridgeSupervisor) {let active=owner.owned_calls();}
    /// ```
    #[must_use]
    pub fn owned_calls(&self) -> usize {
        self.shared
            .context
            .limits
            .accepted
            .saturating_sub(self.shared.capacity.available_permits())
    }
}
impl Drop for LambdaBridgeSupervisor {
    fn drop(&mut self) {
        self.close();
        // Intentional detached manager: its own Drop guard retains unresolved
        // jobs/slots through public-controller loss and runtime cancellation.
        if let Some(manager) = self.manager.take() {
            drop(manager);
        }
    }
}
impl ManagerOwner {
    async fn run(mut self) -> Result<(), Arc<BridgeError>> {
        loop {
            if signal(&self.state.stop) {
                self.shared.accepting.store(false, Ordering::Release);
                self.state.queue.close();
                self.reject_queued();
            }
            if self.state.queue.is_closed()
                && self.state.queue.is_empty()
                && self.state.jobs.is_empty()
            {
                break;
            }
            tokio::select! {
                biased;
                _=self.state.stop.changed(),if !signal(&self.state.stop)=>{},
                result=self.state.jobs.join_next_with_id(),if !self.state.jobs.is_empty()=>if let Some(result)=result {self.finish(result);},
                submission=self.state.queue.recv(),if !self.state.queue.is_closed() || !self.state.queue.is_empty()=>if let Some(submission)=submission {
                    let Submission {command,responder,permit}=submission;
                    let shared=self.shared.clone();let stop=self.state.stop.clone();
                    let task=self.state.jobs.spawn(worker::invoke(shared,command,responder,stop));self.state.active.insert(task.id(),permit);
                },
            }
        }
        self.shared.accepting.store(false, Ordering::Release);
        if self.state.quarantine.is_empty() {
            Ok(())
        } else {
            Err(Arc::new(BridgeError::Quarantined))
        }
    }
    fn reject_queued(&mut self) {
        while let Ok(submission) = self.state.queue.try_recv() {
            submission
                .responder
                .finish(Err(TaskFailure::service_exception()));
        }
    }
    fn finish(&mut self, result: Result<(Id, WorkerExit), JoinError>) {
        match result {
            Ok((id, exit)) => {
                if let Some(diagnostic) = exit.diagnostic {
                    self.shared.diagnostic.send_replace(Some(diagnostic));
                }
                if let Some(permit) = self.state.active.remove(&id) {
                    if let Some(resource) = exit.uncertain {
                        self.shared.unhealthy.store(true, Ordering::Release);
                        self.state.quarantine.push(Retained {
                            _permit: permit,
                            _resource: Some(resource),
                        });
                    }
                }
            }
            Err(error) => {
                self.shared.unhealthy.store(true, Ordering::Release);
                let id = error.id();
                self.shared
                    .diagnostic
                    .send_replace(Some(Arc::new(BridgeError::Worker(error))));
                if let Some(permit) = self.state.active.remove(&id) {
                    self.state.quarantine.push(Retained {
                        _permit: permit,
                        _resource: None,
                    });
                }
            }
        }
    }
}
impl Drop for ManagerOwner {
    fn drop(&mut self) {
        self.shared.accepting.store(false, Ordering::Release);
        self.state.queue.close();
        self.reject_queued();
        if self.state.jobs.is_empty()
            && self.state.active.is_empty()
            && self.state.quarantine.is_empty()
        {
            return;
        }
        // Retain the bounded ownership graph if normal shutdown cannot prove clean.
        // This guard also covers an unpolled manager and runtime teardown.
        let state = ManagerState {
            queue: mpsc::channel(1).1,
            stop: watch::channel(true).1,
            jobs: JoinSet::new(),
            active: HashMap::new(),
            quarantine: Vec::new(),
        };
        let retained = replace(&mut self.state, state);
        tracing::error!(
            calls = retained.active.len() + retained.quarantine.len(),
            "Lambda task bridge retains uncertain cleanup"
        );
        let _retained = Box::leak(Box::new((self.shared.clone(), retained)));
    }
}
fn signal(receiver: &watch::Receiver<bool>) -> bool {
    *receiver.borrow() || receiver.has_changed().is_err()
}
