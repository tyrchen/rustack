//! Retained child protocol and CPU receipts; no Lambda dispatch permit is acquired.

use std::sync::Arc;

use tokio::{
    sync::{mpsc, oneshot, watch},
    task::{JoinHandle, spawn_blocking},
    time::{Instant, sleep_until},
};

use super::{Actor, DispatchLease, ExecutionError, Message, Shared};
use crate::{
    ChildError, ChildExecution, ChildExecutionPort, ChildParent, ChildPattern,
    ChildStartParameters, ChildSubmission, ExecutionSnapshot, FrameCallId, LambdaTaskCommand,
    Payload, TaskCall, TaskFailure, TaskResponder,
};

#[derive(Debug)]
pub(super) enum Binding {
    Pending(ChildExecutionPort, oneshot::Receiver<ExecutionSnapshot>),
    Ready(ChildExecutionPort, ChildParent),
}
impl Actor {
    pub(in crate::engine::runtime) fn with_child_registration(
        mut self,
        registration: Option<(ChildExecutionPort, oneshot::Receiver<ExecutionSnapshot>)>,
    ) -> Self {
        self.children = registration.map(|(port, receiver)| Binding::Pending(port, receiver));
        self
    }
    pub(super) fn start_child(
        &mut self,
        id: FrameCallId,
        command: LambdaTaskCommand,
        pattern: ChildPattern,
        lease: Arc<DispatchLease>,
    ) {
        let Some(Binding::Ready(port, parent)) = &self.children else {
            self.fail(super::ExecutionError::Unavailable);
            return;
        };
        let binding = (port.clone(), parent.clone());
        let machine = command.machine().clone();
        let dispatch = command.dispatch().clone();
        let (call, responder) = TaskCall::channel();
        self.child_jobs.spawn(run(
            binding,
            command,
            pattern,
            responder,
            lease.clone(),
            self.shared.clone(),
            self.sender.clone(),
        ));
        self.accepted(id, machine, dispatch, call, lease);
    }
    pub(super) async fn register_children(&mut self) {
        let Some(Binding::Pending(port, mut receiver)) = self.children.take() else {
            return;
        };
        self.control();
        if self.forced.is_some() {
            return;
        }
        let result = tokio::select! {
            biased;
            _ = self.stop.changed() => { self.control(); return; },
            () = sleep_until(self.deadline) => { self.control(); return; },
            result = &mut receiver => result,
        };
        match result {
            Ok(parent) => {
                self.children = Some(Binding::Ready(port, parent.child_parent()));
                self.parent = Some(parent);
            }
            Err(_) => self.fail(super::ExecutionError::Unavailable),
        }
    }
}

#[derive(Debug)]
enum CpuValue {
    Parsed(Result<ChildStartParameters, ChildError>),
    Mapped(Result<Result<Payload, TaskFailure>, ChildError>),
}
#[derive(Debug)]
struct Receipts {
    submission: Option<ChildSubmission>,
    child: Option<ChildExecution>,
    cpu: Option<JoinHandle<CpuValue>>,
    lease: Arc<DispatchLease>,
    failure: Option<ExecutionError>,
    cancellation: watch::Receiver<bool>,
    deadline: Instant,
}
#[derive(Debug)]
struct Owner(Option<Box<Receipts>>);
impl Drop for Owner {
    fn drop(&mut self) {
        if let Some(receipts) = self.0.take() {
            if let Some(submission) = &receipts.submission {
                submission.cancel();
            }
            if let Some(child) = &receipts.child {
                child.cancel();
            }
            // A lost worker cannot prove issuance or physical cleanup. The registry
            // still owns relations; retain the exact receipts and frame charge too.
            let _retained = Box::leak(receipts);
        }
    }
}
impl Owner {
    fn receipts(&mut self) -> Result<&mut Receipts, ChildError> {
        self.0.as_deref_mut().ok_or(ChildError::OwnerLost)
    }
    async fn cpu(
        &mut self,
        shared: &Shared,
        work: impl FnOnce() -> CpuValue + Send + 'static,
    ) -> Result<CpuValue, ChildError> {
        let receipts = self.receipts()?;
        let mut cancel = receipts.cancellation.clone();
        let permit = if cancelled(&cancel, receipts.deadline) {
            None
        } else {
            tokio::select! {
                biased;
                _ = cancel.changed() => None,
                () = sleep_until(receipts.deadline) => None,
                permit = shared.pools.cpu.clone().acquire_owned() => Some(permit.map_err(|_| ChildError::Unavailable)?),
            }
        };
        let Some(permit) = permit else {
            if let Some(child) = &receipts.child {
                child.cancel();
            }
            return Err(ChildError::Cancelled);
        };
        let lease = receipts.lease.clone();
        #[cfg(test)]
        let panic = shared.child_cpu_panic;
        receipts.cpu = Some(spawn_blocking(move || {
            let _permit = permit;
            let _lease = lease;
            #[cfg(test)]
            assert!(!panic, "controlled child CPU failure");
            work()
        }));
        let job = receipts.cpu.as_mut().ok_or(ChildError::OwnerLost)?;
        let value = tokio::select! {
            biased;
            _ = cancel.changed() => {
                if let Some(child) = &receipts.child { child.cancel(); }
                job.await
            },
            () = sleep_until(receipts.deadline) => {
                if let Some(child) = &receipts.child { child.cancel(); }
                job.await
            },
            value = &mut *job => value,
        };
        // A panic leaves the completed join receipt retained for quarantine.
        let value = match value {
            Ok(value) => value,
            Err(error) => {
                receipts.failure = Some(ExecutionError::Worker(error));
                return Err(ChildError::OwnerLost);
            }
        };
        receipts.cpu = None;
        Ok(value)
    }
    async fn mapped(
        &mut self,
        shared: &Shared,
        start: bool,
    ) -> Result<Result<Payload, TaskFailure>, ChildError> {
        let mapper = self
            .receipts()?
            .child
            .as_ref()
            .ok_or(ChildError::OwnerLost)?
            .mapper(start);
        match self.cpu(shared, move || CpuValue::Mapped(mapper())).await? {
            CpuValue::Mapped(result) => result,
            CpuValue::Parsed(_) => Err(ChildError::OwnerLost),
        }
    }
    fn clean(&mut self) {
        self.0 = None;
    }
}
fn unknown(error: &ChildError) -> bool {
    match error {
        ChildError::Admission(error) => unknown(error),
        ChildError::OwnerLost => true,
        _ => false,
    }
}
fn cancelled(cancel: &watch::Receiver<bool>, deadline: Instant) -> bool {
    *cancel.borrow() || cancel.has_changed().is_err() || Instant::now() >= deadline
}

pub(super) async fn run(
    binding: (ChildExecutionPort, ChildParent),
    command: LambdaTaskCommand,
    pattern: ChildPattern,
    mut responder: TaskResponder,
    lease: Arc<DispatchLease>,
    shared: Arc<Shared>,
    sender: mpsc::Sender<Message>,
) {
    let mut owner = Owner(Some(Box::new(Receipts {
        submission: None,
        child: None,
        cpu: None,
        lease,
        failure: None,
        cancellation: responder.cancellation(),
        deadline: Instant::from_std(command.dispatch().deadline()),
    })));
    let mut result = execute(
        &mut owner,
        binding,
        command,
        pattern,
        &mut responder,
        &shared,
    )
    .await;
    if result.as_ref().is_err_and(|error| !unknown(error)) {
        if let Some(child) = owner.0.as_mut().and_then(|r| r.child.as_mut()) {
            if pattern.owned() {
                child.cancel();
                if let Err(error) = child.settled().await {
                    result = Err(error);
                } else if let Some(receipts) = owner.0.as_mut() {
                    receipts.child = None;
                }
            } else if let Some(receipts) = owner.0.as_mut() {
                receipts.child = None;
            }
        }
    }
    match result {
        Ok(result) => {
            let completion = responder.complete(result);
            let cleanup = if let Some(child) = owner.0.as_mut().and_then(|r| r.child.as_mut()) {
                child.settled().await
            } else {
                Ok(())
            };
            if cleanup.is_ok() {
                owner.clean();
                completion.settle();
            }
        }
        Err(error)
            if !unknown(&error)
                && owner.0.as_ref().is_some_and(|r| {
                    r.submission.is_none() && r.child.is_none() && r.cpu.is_none()
                }) =>
        {
            owner.clean();
            responder.finish(Err(error.failure()));
        }
        Err(error) => {
            if let Some(failure) = owner.0.as_mut().and_then(|r| r.failure.take()) {
                let _ = sender.send(Message::Fatal(failure)).await;
            }
            // Close both channels so the result watcher records uncertainty instead
            // of waiting forever on a fabricated live completion capability.
            drop(responder.quarantine(error.failure()));
        }
    }
}
async fn execute(
    owner: &mut Owner,
    (port, parent): (ChildExecutionPort, ChildParent),
    command: LambdaTaskCommand,
    pattern: ChildPattern,
    responder: &mut TaskResponder,
    shared: &Shared,
) -> Result<Result<Payload, TaskFailure>, ChildError> {
    let deadline = Instant::from_std(command.dispatch().deadline());
    let mut cancel = responder.cancellation();
    if cancelled(&cancel, deadline) {
        return Err(ChildError::Cancelled);
    }
    let scope = command.machine().scope().clone();
    let parameters = command.dispatch().parameters().clone();
    let args = match owner
        .cpu(shared, move || {
            CpuValue::Parsed(ChildStartParameters::parse(&scope, &parameters))
        })
        .await?
    {
        CpuValue::Parsed(result) => result?,
        CpuValue::Mapped(_) => return Err(ChildError::OwnerLost),
    };
    if cancelled(&cancel, deadline) {
        return Err(ChildError::Cancelled);
    }
    owner.receipts()?.submission = Some(port.start(parent, args, pattern, deadline)?);
    let submission = owner
        .receipts()?
        .submission
        .as_mut()
        .ok_or(ChildError::OwnerLost)?;
    let admitted = tokio::select! {
        biased;
        _ = cancel.changed() => { submission.cancel(); submission.resolve().await },
        () = sleep_until(deadline) => { submission.cancel(); submission.resolve().await },
        result = submission.resolve() => result,
    };
    if let Err(error) = admitted {
        if !unknown(&error) {
            owner.receipts()?.submission = None;
        }
        return Err(error);
    }
    let submission = owner
        .receipts()?
        .submission
        .take()
        .ok_or(ChildError::OwnerLost)?;
    owner.receipts()?.child = Some(submission.into_execution()?);
    responder.started().map_err(|_| ChildError::OwnerLost)?;
    let start = owner
        .mapped(shared, true)
        .await?
        .map_err(|_| ChildError::Invalid)?;
    if !pattern.owned() {
        // Registry acceptance detached this child. Its global owner keeps running.
        owner.receipts()?.child = None;
        return Ok(Ok(start));
    }
    responder
        .submitted(start)
        .map_err(|_| ChildError::OwnerLost)?;
    let child = owner
        .receipts()?
        .child
        .as_mut()
        .ok_or(ChildError::OwnerLost)?;
    let terminal = if cancelled(&cancel, deadline) {
        child.cancel();
        false
    } else {
        tokio::select! {
            biased;
            _ = cancel.changed() => { child.cancel(); false },
            () = sleep_until(deadline) => { child.cancel(); false },
            result = child.wait(deadline) => {
                if result.is_err() { child.cancel(); }
                result.is_ok()
            },
        }
    };
    if !terminal {
        // Control won; retain cleanup and capture the eventual aborted outcome.
        child.settled().await?;
    }
    owner.mapped(shared, false).await
}
