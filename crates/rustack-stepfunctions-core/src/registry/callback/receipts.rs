//! Unique Task controls and cancellation-safe cached admission/API observations.

use std::sync::Arc;

use tokio::{
    sync::{oneshot, watch},
    time::{Instant, timeout_at},
};

use super::{
    AckReply, CallbackError, CallbackInvocation, CallbackInvocationReporter, CallbackOutcome,
    CallbackToken, Control, MintReply,
};
use crate::TaskFailure;

/// One retained trusted mint request; dropping it revokes even before acceptance.
#[derive(Debug)]
#[non_exhaustive]
pub struct CallbackMint {
    receiver: Option<oneshot::Receiver<MintReply>>,
    reply: Option<MintReply>,
    loss: Option<Arc<CallbackError>>,
    control: Arc<Control>,
    armed: bool,
}
impl CallbackMint {
    pub(super) fn new(receiver: oneshot::Receiver<MintReply>, control: Arc<Control>) -> Self {
        Self {
            receiver: Some(receiver),
            reply: None,
            loss: None,
            control,
            armed: true,
        }
    }
    /// Observe the same minted token owner, caching even a late-ready response.
    ///
    /// # Errors
    /// Returns typed mint failure, observer Deadline or unknown `OwnerLost`.
    ///
    /// ```no_run
    /// # async fn observe(m:&mut rustack_stepfunctions_core::CallbackMint,d:tokio::time::Instant){let task=m.admitted(d).await;}
    /// ```
    pub async fn admitted(&mut self, deadline: Instant) -> Result<&CallbackTask, CallbackError> {
        if Instant::now() >= deadline {
            return Err(CallbackError::Deadline);
        }
        let result = timeout_at(deadline, self.resolve())
            .await
            .map_err(|_| CallbackError::Deadline)?;
        if Instant::now() >= deadline {
            return Err(CallbackError::Deadline);
        }
        result?;
        match self.reply.as_ref().map(|reply| &reply.result) {
            Some(Ok(task)) => Ok(task),
            _ => Err(CallbackError::OwnerLost),
        }
    }
    pub(crate) async fn resolve(&mut self) -> Result<(), CallbackError> {
        if self.reply.is_none() && self.loss.is_none() {
            let result = self
                .receiver
                .as_mut()
                .ok_or(CallbackError::OwnerLost)?
                .await;
            self.receiver = None;
            match result {
                Ok(reply) => self.reply = Some(reply),
                Err(_) => self.loss = Some(Arc::new(CallbackError::OwnerLost)),
            }
        }
        if let Some(loss) = &self.loss {
            return Err(CallbackError::Cached(loss.clone()));
        }
        match self.reply.as_ref().map(|reply| &reply.result) {
            Some(Ok(_)) => Ok(()),
            Some(Err(error)) => Err(CallbackError::Cached(error.clone())),
            None => Err(CallbackError::OwnerLost),
        }
    }

    /// Transfer the cached unique Task owner, without revoking on mint Drop.
    ///
    /// # Errors
    /// Requires a cached successful admission, including a late-ready success.
    ///
    /// ```no_run
    /// # fn transfer(m:rustack_stepfunctions_core::CallbackMint){let task=m.into_task();}
    /// ```
    pub fn into_task(mut self) -> Result<CallbackTask, CallbackError> {
        if let Some(loss) = &self.loss {
            return Err(CallbackError::Cached(loss.clone()));
        }
        let task = self
            .reply
            .take()
            .ok_or(CallbackError::OwnerLost)?
            .result
            .map_err(CallbackError::Cached)?;
        self.armed = false;
        Ok(task)
    }
    /// Revoke the accepted/pending attempt independently of command saturation.
    ///
    /// ```
    /// # fn cancel(m:&rustack_stepfunctions_core::CallbackMint){m.revoke();}
    /// ```
    pub fn revoke(&self) {
        self.control.revoke();
    }
}
impl Drop for CallbackMint {
    fn drop(&mut self) {
        if self.armed {
            self.control.revoke();
        }
    }
}
/// Resumable API acknowledgment; observer loss never undoes its accepted action.
#[derive(Debug)]
#[non_exhaustive]
pub struct CallbackAck {
    receiver: Option<oneshot::Receiver<AckReply>>,
    reply: Option<AckReply>,
    loss: Option<Arc<CallbackError>>,
}
impl CallbackAck {
    pub(super) fn new(receiver: oneshot::Receiver<AckReply>) -> Self {
        Self {
            receiver: Some(receiver),
            reply: None,
            loss: None,
        }
    }
    /// Await/cache one action result without resending it on observation timeout.
    ///
    /// # Errors
    /// Returns protocol/admission failure, observer Deadline or `OwnerLost`.
    ///
    /// ```no_run
    /// # async fn observe(r:&mut rustack_stepfunctions_core::CallbackAck,d:tokio::time::Instant){let result=r.wait(d).await;}
    /// ```
    pub async fn wait(&mut self, deadline: Instant) -> Result<(), CallbackError> {
        if Instant::now() >= deadline {
            return Err(CallbackError::Deadline);
        }
        if self.reply.is_none() && self.loss.is_none() {
            let result = timeout_at(
                deadline,
                self.receiver.as_mut().ok_or(CallbackError::OwnerLost)?,
            )
            .await
            .map_err(|_| CallbackError::Deadline)?;
            self.receiver = None;
            match result {
                Ok(reply) => self.reply = Some(reply),
                Err(_) => self.loss = Some(Arc::new(CallbackError::OwnerLost)),
            }
        }
        if Instant::now() >= deadline {
            return Err(CallbackError::Deadline);
        }
        if let Some(loss) = &self.loss {
            return Err(CallbackError::Cached(loss.clone()));
        }
        match self.reply.as_ref().map(|reply| &reply.result) {
            Some(Ok(())) => Ok(()),
            Some(Err(error)) => Err(CallbackError::Cached(error.clone())),
            None => Err(CallbackError::OwnerLost),
        }
    }
}
/// Unique invocation gate, credential and independent business-result observer.
#[derive(Debug)]
#[non_exhaustive]
pub struct CallbackTask {
    pub(super) token: CallbackToken,
    pub(super) control: Arc<Control>,
    pub(super) reporter: Option<CallbackInvocationReporter>,
    pub(super) invocation: watch::Receiver<Option<Arc<CallbackInvocation>>>,
    pub(super) view: watch::Receiver<Arc<CallbackOutcome>>,
    pub(super) deadline: Instant,
}
impl CallbackTask {
    /// Borrow the secret only for intentional context/outbound credential access.
    ///
    /// ```
    /// # fn send(task:&rustack_stepfunctions_core::CallbackTask){let token=task.token();}
    /// ```
    #[must_use]
    pub fn token(&self) -> &CallbackToken {
        &self.token
    }
    /// Capture one coherent invocation/callback business state.
    ///
    /// ```
    /// # fn inspect(task:&rustack_stepfunctions_core::CallbackTask){let state=task.snapshot();}
    /// ```
    #[must_use]
    pub fn snapshot(&self) -> Arc<CallbackOutcome> {
        self.view.borrow().clone()
    }
    /// Report synchronous invocation outcome once; no business Lambda payload.
    ///
    /// # Errors
    /// Rejects duplicate invocation reports, expired/revoked tasks and root loss.
    ///
    /// ```
    /// # fn report(task:&mut rustack_stepfunctions_core::CallbackTask){let result=task.invocation_complete(Ok(()));}
    /// ```
    pub fn invocation_complete(
        &mut self,
        result: Result<(), TaskFailure>,
    ) -> Result<(), CallbackError> {
        self.reporter_live()?;
        self.reporter
            .as_mut()
            .ok_or(CallbackError::Progress)?
            .complete_unit(result)
    }
    fn reporter_live(&self) -> Result<(), CallbackError> {
        if self
            .reporter
            .as_ref()
            .is_none_or(|reporter| reporter.reported)
        {
            return Err(CallbackError::Progress);
        }
        if matches!(self.snapshot().as_ref(), CallbackOutcome::Unavailable) {
            return Err(CallbackError::Unavailable);
        }
        if self.snapshot().is_terminal()
            || Instant::now() >= self.deadline
            || self.control.revoked()
        {
            return Err(CallbackError::TaskTimedOut);
        }
        Ok(())
    }
    /// Transfer the unique invocation report capability to its real provider owner.
    ///
    /// # Errors
    /// Rejects duplicate/delegated progress, expired/revoked tasks or root loss.
    ///
    /// ```
    /// # fn delegate(t:&mut rustack_stepfunctions_core::CallbackTask){let reporter=t.take_invocation_reporter();}
    /// ```
    pub fn take_invocation_reporter(
        &mut self,
    ) -> Result<CallbackInvocationReporter, CallbackError> {
        self.reporter_live()?;
        self.reporter.take().ok_or(CallbackError::Progress)
    }
    /// Capture the actual return report independently of callback/cleanup state.
    ///
    /// ```
    /// # fn inspect(t:&rustack_stepfunctions_core::CallbackTask){let report=t.invocation();}
    /// ```
    #[must_use]
    pub fn invocation(&self) -> Option<Arc<CallbackInvocation>> {
        self.invocation.borrow().clone()
    }
    /// Observe the same invocation return without resending or consuming cleanup.
    ///
    /// # Errors
    /// Returns observer Deadline, known task closure, root Unavailable or `OwnerLost`.
    ///
    /// ```no_run
    /// # async fn observe(t:&mut rustack_stepfunctions_core::CallbackTask,d:tokio::time::Instant){let report=t.wait_invocation(d).await;}
    /// ```
    pub async fn wait_invocation(
        &mut self,
        deadline: Instant,
    ) -> Result<Arc<CallbackInvocation>, CallbackError> {
        if Instant::now() >= deadline {
            return Err(CallbackError::Deadline);
        }
        let result = timeout_at(deadline, async {
            loop {
                if let Some(report) = self.invocation() { return Ok(report); }
                match self.snapshot().as_ref() {
                    CallbackOutcome::Unavailable => return Err(CallbackError::Unavailable),
                    CallbackOutcome::TimedOut(_) | CallbackOutcome::Revoked => return Err(CallbackError::TaskTimedOut),
                    _ => {},
                }
                tokio::select! {
                    biased;
                    result = self.invocation.changed() => result.map_err(|_| CallbackError::OwnerLost)?,
                    result = self.view.changed() => result.map_err(|_| CallbackError::OwnerLost)?,
                }
            }
        }).await.map_err(|_| CallbackError::Deadline)?;
        if Instant::now() >= deadline {
            return Err(CallbackError::Deadline);
        }
        result
    }
    /// Revoke immediately through independent monotonic control and Notify.
    ///
    /// ```
    /// # fn cancel(task:&rustack_stepfunctions_core::CallbackTask){task.revoke();}
    /// ```
    pub fn revoke(&self) {
        self.control.revoke();
    }
    /// Await a terminal business state; observer expiry never cancels ownership.
    ///
    /// # Errors
    /// Returns Deadline or `OwnerLost` without resending or revoking the callback.
    ///
    /// ```no_run
    /// # async fn observe(task:&mut rustack_stepfunctions_core::CallbackTask,d:tokio::time::Instant){let result=task.wait(d).await;}
    /// ```
    pub async fn wait(&mut self, deadline: Instant) -> Result<Arc<CallbackOutcome>, CallbackError> {
        if Instant::now() >= deadline {
            return Err(CallbackError::Deadline);
        }
        let result = timeout_at(deadline, self.resolved())
            .await
            .map_err(|_| CallbackError::Deadline)?;
        if Instant::now() >= deadline {
            return Err(CallbackError::Deadline);
        }
        result
    }
    pub(crate) async fn resolved(&mut self) -> Result<Arc<CallbackOutcome>, CallbackError> {
        loop {
            let view = self.snapshot();
            if view.is_terminal() {
                return Ok(view);
            }
            self.view
                .changed()
                .await
                .map_err(|_| CallbackError::OwnerLost)?;
        }
    }
}
impl Drop for CallbackTask {
    fn drop(&mut self) {
        self.revoke();
    }
}
