//! Retained actor-owned resource revisions, execution names and immutable reads.
//!
//! Accept compiled/validated inputs, independent of HTTP. The embedding API must
//! own bounded preparation and pagination. No command waiter owns execution life.

use std::{fmt, sync::Arc};

use tokio::{
    runtime::Handle,
    sync::{Notify, OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch},
    task::JoinHandle,
    time::{Instant, timeout_at},
};

use crate::{
    CompiledMachine, ExecutionLimits, ExecutionSupervisor, Payload, StateSupport, TenantScope,
};

mod actor;
mod callback;
mod child;
mod domain;
mod error;
#[cfg(test)]
mod tests;
mod types;

pub use callback::{
    CallbackAck, CallbackAttempt, CallbackCommand, CallbackError, CallbackInvocation,
    CallbackInvocationMetadata, CallbackInvocationReporter, CallbackLimits, CallbackMint,
    CallbackOutcome, CallbackRegistry, CallbackStatus, CallbackSupervisor, CallbackTask,
    CallbackTimeout, CallbackToken,
};
use child::owner::Inbox as ChildInbox;
pub use child::{
    ChildError, ChildExecution, ChildExecutionPort, ChildParent, ChildPattern,
    ChildStartParameters, ChildSubmission,
};
pub use domain::{ExecutionArn, ExecutionName, RoleArn, StateMachineArn, StateMachineName, Tags};
pub use error::RegistryError;
pub use types::{
    DefinitionRevision, ExecutionSnapshot, MachineSnapshot, MachineStatus, PreparedDefinition,
    RegistryCapacities, RegistryLimits, StopReason,
};

/// Validated domain operations; preparing wire DTOs remains the API adapter's job.
#[non_exhaustive]
pub enum RegistryCommand {
    /// Create under the supported effective STANDARD/OFF profile.
    Create(StateMachineName, PreparedDefinition, RoleArn, Tags),
    /// Publish a new revision, changing definition and/or role.
    Update(StateMachineArn, Option<PreparedDefinition>, Option<RoleArn>),
    /// Enter deletion, preserving already accepted executions.
    Delete(StateMachineArn),
    /// Read the current mutable resource.
    Machine(StateMachineArn),
    /// Capture all current machines in deterministic order.
    Machines,
    /// Start or replay an identical raw-input RUNNING execution.
    Start(
        StateMachineArn,
        Option<ExecutionName>,
        Payload,
        Option<String>,
    ),
    /// Read captured identity/revision and one atomic execution view.
    Execution(ExecutionArn),
    /// Retain a Stop signal even after the reply observer is dropped.
    Stop(ExecutionArn, StopReason),
    /// Capture retained executions for this ARN, including prior incarnations.
    Executions(StateMachineArn),
    /// Atomically add/replace tags after all validation.
    Tag(StateMachineArn, Tags),
    /// Atomically remove keys; values in the validated Tags map are ignored.
    Untag(StateMachineArn, Tags),
}
impl fmt::Debug for RegistryCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RegistryCommand(<redacted>)")
    }
}
/// One immutable command result, captured by the registry owner.
#[non_exhaustive]
pub enum RegistryResult {
    /// Current resource capture.
    Machine(MachineSnapshot),
    /// Current execution capture.
    Execution(ExecutionSnapshot),
    /// Stable resource-list boundary for a later pagination owner.
    Machines(Vec<MachineSnapshot>),
    /// Stable execution-list boundary for a later pagination owner.
    Executions(Vec<ExecutionSnapshot>),
    /// Mutable deletion accepted.
    Deleted,
    /// Updated bounded tags.
    Tags(Tags),
}
impl fmt::Debug for RegistryResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RegistryResult(<redacted>)")
    }
}
struct Envelope {
    command: RegistryCommand,
    reply: oneshot::Sender<Reply>,
    permit: OwnedSemaphorePermit,
}
struct Reply {
    result: Result<RegistryResult, RegistryError>,
    permit: OwnedSemaphorePermit,
}
#[derive(Debug, Clone, Copy, Default)]
struct Profile {
    execution: ExecutionLimits,
    states: StateSupport,
}
struct Shared {
    child_sender: mpsc::Sender<child::Request>,
    child_capacity: Arc<Semaphore>,
    child_wake: Arc<Notify>,
    scope: TenantScope,
    states: StateSupport,
    callbacks: bool,
    sender: mpsc::Sender<Envelope>,
    capacity: Arc<Semaphore>,
    closing: watch::Sender<bool>,
    status: watch::Sender<RegistryStatus>,
}
impl fmt::Debug for Shared {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RegistryShared(<redacted>)")
    }
}
#[derive(Debug)]
struct Admission {
    shared: Arc<Shared>,
    channels: (mpsc::Receiver<Envelope>, ChildInbox),
}
fn admission(
    scope: TenantScope,
    limits: RegistryLimits,
    support: StateSupport,
    child_limit: usize,
    callbacks: bool,
) -> Admission {
    let (sender, receiver) = mpsc::channel(limits.commands);
    let (child_sender, children) = mpsc::channel(child_limit);
    let (closing, _) = watch::channel(false);
    let (status, _) = watch::channel(RegistryStatus {
        ready: true,
        owned_executions: 0,
    });
    let shared = Arc::new(Shared {
        child_sender,
        child_capacity: Arc::new(Semaphore::new(child_limit)),
        child_wake: Arc::new(Notify::new()),
        scope,
        states: support,
        callbacks,
        sender,
        capacity: Arc::new(Semaphore::new(limits.commands)),
        closing,
        status,
    });
    Admission {
        channels: (
            receiver,
            ChildInbox {
                requests: children,
                registry: shared.clone(),
            },
        ),
        shared,
    }
}
/// Admission-independent registry supervisor observation.
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct RegistryStatus {
    /// False after close/root loss or quarantined execution cleanup.
    pub ready: bool,
    /// All live or quarantined execution owners, including terminal cleanup.
    pub owned_executions: usize,
}
/// Cloneable bounded command handle; caller loss never rolls back acceptance.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Registry {
    shared: Arc<Shared>,
}
/// Resumable observer of one accepted command; no mutation can be replayed.
#[non_exhaustive]
pub struct RegistryReceipt {
    receiver: Option<oneshot::Receiver<Reply>>,
}
impl fmt::Debug for RegistryReceipt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RegistryReceipt(<redacted>)")
    }
}
impl RegistryReceipt {
    /// Await the same reply, preserving its receiver on observation timeout.
    ///
    /// # Errors
    /// Returns a domain failure, Deadline or Unavailable/consumed receipt.
    ///
    /// ```no_run
    /// # async fn observe(receipt:&mut rustack_stepfunctions_core::RegistryReceipt,deadline:tokio::time::Instant) { let result=receipt.wait(deadline).await; }
    /// ```
    pub async fn wait(&mut self, deadline: Instant) -> Result<RegistryResult, RegistryError> {
        if Instant::now() >= deadline {
            return Err(RegistryError::Deadline);
        }
        let receiver = self.receiver.as_mut().ok_or(RegistryError::Unavailable)?;
        let reply = timeout_at(deadline, receiver)
            .await
            .map_err(|_| RegistryError::Deadline)?;
        // Closure also consumes a oneshot: never repoll its completed receiver.
        self.receiver = None;
        let reply = reply.map_err(|_| RegistryError::Unavailable)?;
        // A committed command remains committed even if this observer is late.
        let result = reply.result;
        drop(reply.permit);
        result
    }
}
impl Registry {
    /// Spawn one bounded owner around an existing real execution supervisor.
    ///
    /// # Errors
    /// Requires an active Tokio runtime. All limits are already validated.
    ///
    /// ```no_run
    /// # fn run(runtime:rustack_stepfunctions_core::ExecutionSupervisor)->Result<(),rustack_stepfunctions_core::RegistryError>{
    /// let (registry,owner)=rustack_stepfunctions_core::Registry::new(Default::default(),Default::default(),runtime,Default::default())?;
    /// # Ok(()) }
    /// ```
    pub fn new(
        scope: TenantScope,
        limits: RegistryLimits,
        runtime: ExecutionSupervisor,
        execution: ExecutionLimits,
    ) -> Result<(Self, RegistrySupervisor), RegistryError> {
        Self::new_with_state_support(scope, limits, runtime, execution, StateSupport::Sequential)
    }
    /// Compose the registry with an immutable installed state-instruction profile.
    ///
    /// The API must use `state_support()` when compiling definitions.
    ///
    /// # Errors
    /// Requires an active Tokio runtime; all supplied limits are validated.
    ///
    /// ```no_run
    /// # fn compose(runtime:rustack_stepfunctions_core::ExecutionSupervisor)->Result<(),rustack_stepfunctions_core::RegistryError>{
    /// let (registry,owner)=rustack_stepfunctions_core::Registry::new_with_state_support(Default::default(),Default::default(),runtime,Default::default(),rustack_stepfunctions_core::StateSupport::Structured)?;
    /// # Ok(()) }
    /// ```
    pub fn new_with_state_support(
        scope: TenantScope,
        limits: RegistryLimits,
        runtime: ExecutionSupervisor,
        execution: ExecutionLimits,
        support: StateSupport,
    ) -> Result<(Self, RegistrySupervisor), RegistryError> {
        Handle::try_current().map_err(|_| RegistryError::Unavailable)?;
        if runtime
            .callback_scope()
            .is_some_and(|installed| !domain::same_scope(installed, &scope))
        {
            return Err(RegistryError::Validation);
        }
        let callbacks = runtime.callback_scope().is_some();
        let Admission { shared, channels } = admission(
            scope.clone(),
            limits,
            support,
            runtime.open_limit(),
            callbacks,
        );
        let closing = shared.closing.clone();
        let stop = closing.subscribe();
        let status = shared.status.clone();
        // Install Drop protection before spawn: an unpolled root still owns data.
        let owner = actor::Owner::new(
            scope,
            limits,
            runtime,
            Profile {
                execution,
                states: support,
            },
            channels,
            closing.clone(),
            status,
        );
        let join = tokio::spawn(owner.run(stop));
        Ok((
            Self { shared },
            RegistrySupervisor {
                closing,
                join: Some(join),
                result: None,
            },
        ))
    }
    /// Read readiness/owned execution counts without using command admission.
    ///
    /// ```
    /// # fn inspect(registry:&rustack_stepfunctions_core::Registry){let state=registry.status();}
    /// ```
    #[must_use]
    pub fn status(&self) -> RegistryStatus {
        let mut status = *self.shared.status.borrow();
        status.ready &= !*self.shared.closing.borrow();
        status
    }
    /// Wait for retained execution owners to settle through a grace cutoff.
    ///
    /// This does not close intake; the caller must first join external API intake.
    ///
    /// # Errors
    /// Returns Deadline while executions remain, or Unavailable after root failure.
    ///
    /// ```no_run
    /// # async fn grace(registry:&rustack_stepfunctions_core::Registry,deadline:tokio::time::Instant){let result=registry.wait_idle(deadline).await;}
    /// ```
    pub async fn wait_idle(&self, deadline: Instant) -> Result<(), RegistryError> {
        let mut status = self.shared.status.subscribe();
        let mut closing = self.shared.closing.subscribe();
        loop {
            let value = *status.borrow_and_update();
            if !value.ready || *self.shared.closing.borrow() {
                return Err(RegistryError::Unavailable);
            }
            if value.owned_executions == 0 {
                return Ok(());
            }
            timeout_at(deadline, async {
                tokio::select! {
                    result = status.changed() => result,
                    result = closing.changed() => result,
                }
            })
            .await
            .map_err(|_| RegistryError::Deadline)?
            .map_err(|_| RegistryError::Unavailable)?;
        }
    }
    /// Read the immutable trusted scope of this registry.
    ///
    /// ```
    /// # fn inspect(registry:&rustack_stepfunctions_core::Registry){let scope=registry.scope();}
    /// ```
    #[must_use]
    pub fn scope(&self) -> &TenantScope {
        &self.shared.scope
    }
    /// Read the immutable installed instruction profile for compilation.
    ///
    /// ```
    /// # fn inspect(registry:&rustack_stepfunctions_core::Registry){let states=registry.state_support();}
    /// ```
    #[must_use]
    pub fn state_support(&self) -> StateSupport {
        self.shared.states
    }
    /// Read callback compilation support from the installed execution owner.
    ///
    /// ```
    /// # fn inspect(registry:&rustack_stepfunctions_core::Registry){let enabled=registry.callback_support();}
    /// ```
    #[must_use]
    pub fn callback_support(&self) -> bool {
        self.shared.callbacks
    }
    /// Validate and synchronously transfer one command to retained ownership.
    ///
    /// # Errors
    /// Rejects scope/metadata, closed ownership or saturated command admission.
    /// An error means no command was accepted.
    ///
    /// ```no_run
    /// # fn submit(registry:&rustack_stepfunctions_core::Registry)->Result<(),rustack_stepfunctions_core::RegistryError>{
    /// let receipt=registry.submit(rustack_stepfunctions_core::RegistryCommand::Machines)?;
    /// # Ok(()) }
    /// ```
    pub fn submit(&self, command: RegistryCommand) -> Result<RegistryReceipt, RegistryError> {
        command.check(
            &self.shared.scope,
            self.shared.states,
            self.shared.callbacks,
        )?;
        if *self.shared.closing.borrow() {
            return Err(RegistryError::Unavailable);
        }
        let permit = self
            .shared
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| RegistryError::Capacity)?;
        let (reply, receiver) = oneshot::channel();
        self.shared
            .sender
            .try_send(Envelope {
                command,
                reply,
                permit,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => RegistryError::Capacity,
                mpsc::error::TrySendError::Closed(_) => RegistryError::Unavailable,
            })?;
        Ok(RegistryReceipt {
            receiver: Some(receiver),
        })
    }
    /// Signal Stop and observe termination outside the actor command lane.
    ///
    /// # Errors
    /// Rejects oversized fields, missing execution, admission or observer expiry.
    /// Stop is retained after observer loss and preserves omitted/empty fields.
    ///
    /// ```no_run
    /// # async fn stop(registry:&rustack_stepfunctions_core::Registry,arn:rustack_stepfunctions_core::ExecutionArn,deadline:tokio::time::Instant) { let outcome=registry.stop(arn,None,None,deadline).await; }
    /// ```
    pub async fn stop(
        &self,
        arn: ExecutionArn,
        error: Option<String>,
        cause: Option<String>,
        deadline: Instant,
    ) -> Result<ExecutionSnapshot, RegistryError> {
        let reason = StopReason::new(error, cause)?;
        let mut receipt = self.submit(RegistryCommand::Stop(arn, reason))?;
        let RegistryResult::Execution(mut snapshot) = receipt.wait(deadline).await? else {
            return Err(RegistryError::Unavailable);
        };
        let mut handle = snapshot.handle();
        snapshot.view = handle.wait(deadline).await?;
        Ok(snapshot)
    }
}
impl RegistryCommand {
    fn check(
        &self,
        scope: &TenantScope,
        states: StateSupport,
        callbacks: bool,
    ) -> Result<(), RegistryError> {
        let check_machine = |machine: &CompiledMachine| {
            if machine.is_root_definition()
                && (callbacks || !machine.uses_callbacks())
                && domain::same_scope(machine.scope(), scope)
                && (states == StateSupport::Structured || !machine.requires_structured_execution())
            {
                Ok(())
            } else {
                Err(RegistryError::Validation)
            }
        };
        match self {
            Self::Create(_, definition, role, _) => {
                check_machine(&definition.machine)?;
                role.check(scope)
            }
            Self::Update(arn, definition, role) => {
                arn.check(scope)?;
                if definition.is_none() && role.is_none() {
                    return Err(RegistryError::Validation);
                }
                if let Some(d) = definition {
                    check_machine(&d.machine)?;
                }
                if let Some(r) = role {
                    r.check(scope)?;
                }
                Ok(())
            }
            Self::Delete(arn)
            | Self::Machine(arn)
            | Self::Executions(arn)
            | Self::Tag(arn, _)
            | Self::Untag(arn, _) => arn.check(scope),
            Self::Start(arn, _, _, trace) => {
                arn.check(scope)?;
                if trace
                    .as_ref()
                    .is_some_and(|v| v.len() > 256 || !v.is_ascii())
                {
                    return Err(RegistryError::Validation);
                }
                Ok(())
            }
            Self::Execution(arn) | Self::Stop(arn, _) => arn.machine().check(scope),
            Self::Machines => Ok(()),
        }
    }
}
/// Unique shutdown controller; observation timeout retains the exact root join.
#[derive(Debug)]
#[non_exhaustive]
pub struct RegistrySupervisor {
    closing: watch::Sender<bool>,
    join: Option<JoinHandle<Result<(), RegistryError>>>,
    result: Option<Result<(), Arc<RegistryError>>>,
}
impl RegistrySupervisor {
    /// Reject new commands and request joined shutdown once.
    ///
    /// ```
    /// # fn close(owner:&rustack_stepfunctions_core::RegistrySupervisor){owner.close();}
    /// ```
    pub fn close(&self) {
        self.closing.send_replace(true);
    }
    /// Observe joined shutdown through one absolute deadline; resumes the same join.
    ///
    /// # Errors
    /// Returns Deadline without dropping the root, or cached worker/cleanup failure.
    ///
    /// ```no_run
    /// # async fn join(owner:&mut rustack_stepfunctions_core::RegistrySupervisor,deadline:tokio::time::Instant){let result=owner.shutdown(deadline).await;}
    /// ```
    #[tracing::instrument(name = "stepfunctions.registry.shutdown", skip_all)]
    pub async fn shutdown(&mut self, deadline: Instant) -> Result<(), RegistryError> {
        self.close();
        if self.result.is_none() {
            let join = self.join.as_mut().ok_or(RegistryError::Unavailable)?;
            let result = timeout_at(deadline, join)
                .await
                .map_err(|_| RegistryError::Deadline)?;
            self.join = None;
            self.result = Some(match result {
                Ok(result) => result.map_err(Arc::new),
                Err(error) => Err(Arc::new(RegistryError::Worker(error))),
            });
        }
        match self.result.as_ref() {
            Some(Ok(())) => Ok(()),
            Some(Err(error)) => Err(RegistryError::Owner(error.clone())),
            None => Err(RegistryError::Unavailable),
        }
    }
}
impl Drop for RegistrySupervisor {
    fn drop(&mut self) {
        self.close(); /* Root owns drain after controller loss; never abort or replay it. */
    }
}
