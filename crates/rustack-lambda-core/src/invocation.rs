//! Attempt identity, cooperative cancellation and supervised invocation handles.
//!
//! Cancellation signals do not drop backend futures. The work supervisor owns
//! those futures until their resources are settled.

use std::{
    fmt,
    future::{Future, pending},
    sync::Arc,
};

use parking_lot::Mutex as OwnerMutex;
use tokio::{
    runtime::Handle,
    sync::{Mutex, oneshot, watch},
    task::JoinHandle,
    time::{Instant, sleep_until, timeout_at},
};
use uuid::Uuid;

use crate::{
    error::LambdaServiceError,
    executor::{ExecutorError, InvokeResponse},
};

/// Why an invocation must stop accepting a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancellationReason {
    /// Explicit cancellation or loss of the attached caller.
    Requested,
    /// The absolute invocation budget expired.
    Deadline,
}

impl CancellationReason {
    pub(crate) fn error(self) -> ExecutorError {
        match self {
            Self::Requested => ExecutorError::Cancelled,
            Self::Deadline => ExecutorError::DeadlineExceeded,
        }
    }
}

/// Immutable attempt metadata and a cooperative cancellation receiver.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct InvocationContext {
    id: Uuid,
    deadline: Instant,
    cancellation: watch::Receiver<bool>,
    external: Option<watch::Receiver<bool>>,
}

impl InvocationContext {
    #[cfg(test)]
    pub(crate) fn channel(deadline: Instant) -> (watch::Sender<bool>, Self) {
        Self::channel_with_cancellation(deadline, None)
    }

    pub(crate) fn channel_with_cancellation(
        deadline: Instant,
        external: Option<watch::Receiver<bool>>,
    ) -> (watch::Sender<bool>, Self) {
        let (sender, cancellation) = watch::channel(false);
        (
            sender,
            Self {
                id: Uuid::new_v4(),
                deadline,
                cancellation,
                external,
            },
        )
    }

    /// Attempt identity shared with the Runtime API and invocation handle.
    ///
    /// ```
    /// # fn inspect(context: &rustack_lambda_core::invocation::InvocationContext) {
    /// assert!(!context.id().is_nil());
    /// # }
    /// ```
    #[must_use]
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// Absolute monotonic budget, including queue and cold start.
    ///
    /// ```
    /// # fn inspect(context: &rustack_lambda_core::invocation::InvocationContext) {
    /// let deadline = context.deadline();
    /// # }
    /// ```
    #[must_use]
    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Check cancellation before allocating a runtime resource.
    ///
    /// # Errors
    /// Returns cancellation or deadline expiry, without cancelling cleanup.
    ///
    /// ```
    /// # fn inspect(context: &rustack_lambda_core::invocation::InvocationContext) {
    /// let still_active = context.check().is_ok();
    /// # }
    /// ```
    pub fn check(&self) -> Result<(), ExecutorError> {
        if Instant::now() >= self.deadline {
            return Err(ExecutorError::DeadlineExceeded);
        }
        if *self.cancellation.borrow()
            || self.cancellation.has_changed().is_err()
            || self
                .external
                .as_ref()
                .is_some_and(|signal| *signal.borrow() || signal.has_changed().is_err())
        {
            return Err(ExecutorError::Cancelled);
        }
        Ok(())
    }

    /// Wait for an explicit signal or the absolute deadline.
    ///
    /// ```no_run
    /// # async fn stop(context: &rustack_lambda_core::invocation::InvocationContext) {
    /// let reason = context.cancelled().await;
    /// # }
    /// ```
    pub async fn cancelled(&self) -> CancellationReason {
        let mut cancellation = self.cancellation.clone();
        let mut external = self.external.clone();
        match self.check() {
            Err(ExecutorError::DeadlineExceeded) => return CancellationReason::Deadline,
            Err(_) => return CancellationReason::Requested,
            Ok(()) => {}
        }
        tokio::select! {
            biased;
            () = sleep_until(self.deadline) => CancellationReason::Deadline,
            _ = cancellation.wait_for(|cancelled| *cancelled) => {
                if Instant::now() >= self.deadline { CancellationReason::Deadline } else { CancellationReason::Requested }
            }
            () = external_cancelled(&mut external) => {
                if Instant::now() >= self.deadline { CancellationReason::Deadline } else { CancellationReason::Requested }
            }
        }
    }
}

async fn external_cancelled(external: &mut Option<watch::Receiver<bool>>) {
    if let Some(signal) = external {
        let _ = signal.wait_for(|cancelled| *cancelled).await;
    } else {
        pending::<()>().await;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Settlement {
    Pending,
    Clean,
    Uncertain,
}

/// Attached ownership of an admitted invocation; Drop requests cancellation.
#[non_exhaustive]
pub struct InvocationHandle {
    context: InvocationContext,
    cancellation: watch::Sender<bool>,
    response: Option<oneshot::Receiver<Result<InvokeResponse, ExecutorError>>>,
    settlement: watch::Receiver<Settlement>,
    cancel_on_drop: bool,
}

impl fmt::Debug for InvocationHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InvocationHandle")
            .field("id", &self.context.id)
            .field("deadline", &self.context.deadline)
            .field("settlement", &*self.settlement.borrow())
            .finish_non_exhaustive()
    }
}

impl InvocationHandle {
    pub(crate) fn new(
        context: InvocationContext,
        cancellation: watch::Sender<bool>,
        response: oneshot::Receiver<Result<InvokeResponse, ExecutorError>>,
        settlement: watch::Receiver<Settlement>,
    ) -> Self {
        Self {
            context,
            cancellation,
            response: Some(response),
            settlement,
            cancel_on_drop: true,
        }
    }

    /// UUID used for this admitted attempt.
    ///
    /// ```
    /// # fn inspect(handle: &rustack_lambda_core::invocation::InvocationHandle) {
    /// assert!(!handle.id().is_nil());
    /// # }
    /// ```
    #[must_use]
    pub fn id(&self) -> Uuid {
        self.context.id
    }

    /// Signal cancellation while retaining the ability to await cleanup.
    ///
    /// ```
    /// # fn stop(handle: &rustack_lambda_core::invocation::InvocationHandle) {
    /// handle.cancel();
    /// # }
    /// ```
    pub fn cancel(&self) {
        self.cancellation.send_replace(true);
    }

    /// Await the backend outcome within the admitted invocation deadline.
    ///
    /// # Errors
    /// Returns backend errors, deadline expiry, or an uncertain worker failure.
    /// Deadline failure signals cancellation; the supervisor still owns cleanup.
    ///
    /// ```no_run
    /// # async fn finish(handle: rustack_lambda_core::invocation::InvocationHandle) {
    /// let response = handle.wait().await;
    /// # }
    /// ```
    pub async fn wait(mut self) -> Result<InvokeResponse, ExecutorError> {
        let result = self.wait_response().await;
        if self.response.is_none() {
            self.cancel_on_drop = false;
        }
        result
    }

    /// Read the business response while retaining the same cleanup handle.
    ///
    /// A dropped waiter can resume. A consumed result cannot be read twice.
    ///
    /// # Errors
    /// Returns cancellation/deadline/backend failure, lost owner or consumed result.
    ///
    /// ```no_run
    /// # async fn read(handle:&mut rustack_lambda_core::invocation::InvocationHandle) {
    /// let response=handle.wait_response().await;
    /// # }
    /// ```
    pub async fn wait_response(&mut self) -> Result<InvokeResponse, ExecutorError> {
        if let Err(error) = self.context.check() {
            self.cancel();
            return Err(error);
        }
        let Some(receiver) = &mut self.response else {
            return Err(ExecutorError::CleanupFailed(
                "Invocation response already consumed".into(),
            ));
        };
        let response = tokio::select! {
            biased;
            reason=self.context.cancelled()=>{self.cancel();return Err(reason.error());},
            result=receiver=>result,
        };
        self.response = None;
        if let Err(error) = self.context.check() {
            self.cancel();
            return Err(error);
        }
        response.map_err(|_| {
            ExecutorError::CleanupFailed("Invocation worker terminated without settlement".into())
        })?
    }

    /// Await confirmed cleanup independently of response/deadline/cancellation.
    ///
    /// Expiry or dropping this waiter leaves the handle resumable and owned.
    ///
    /// # Errors
    /// Returns `CleanupFailed` on expiry, uncertainty or a closed settlement channel.
    ///
    /// ```no_run
    /// # async fn join(handle:&mut rustack_lambda_core::invocation::InvocationHandle,deadline:tokio::time::Instant) {
    /// let cleanup=handle.wait_settlement(deadline).await;
    /// # }
    /// ```
    pub async fn wait_settlement(&mut self, deadline: Instant) -> Result<(), ExecutorError> {
        timeout_at(deadline, self.wait_cleanup())
            .await
            .map_err(|_| {
                ExecutorError::CleanupFailed("Invocation cleanup observer expired".into())
            })?
    }

    /// Retain ownership while joining cleanup without an observation deadline.
    ///
    /// This waits only on the supervisor's state channel; backend IO retains its
    /// own deadlines. Use `wait_settlement` for a bounded observer of the same work.
    ///
    /// # Errors
    /// Returns `CleanupFailed` if the owner closes or reports uncertain settlement.
    ///
    /// ```no_run
    /// # async fn join(handle:&mut rustack_lambda_core::invocation::InvocationHandle) {
    /// let settled=handle.wait_cleanup().await;
    /// # }
    /// ```
    pub async fn wait_cleanup(&mut self) -> Result<(), ExecutorError> {
        match self
            .settlement
            .wait_for(|state| *state != Settlement::Pending)
            .await
        {
            Ok(state) if *state == Settlement::Clean => {
                self.cancel_on_drop = false;
                Ok(())
            }
            _ => Err(ExecutorError::CleanupFailed(
                "Invocation cleanup is not confirmed".into(),
            )),
        }
    }

    /// Request cancellation and await confirmed cleanup, independently of the call deadline.
    ///
    /// # Errors
    /// Returns `CleanupFailed` when the cleanup waiter expires or settlement is uncertain.
    /// Cleanup remains owned after this method returns an error or is dropped.
    ///
    /// ```no_run
    /// # async fn stop(handle: rustack_lambda_core::invocation::InvocationHandle) {
    /// let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    /// let settled = handle.cancel_and_wait(deadline).await;
    /// # }
    /// ```
    pub async fn cancel_and_wait(mut self, deadline: Instant) -> Result<(), ExecutorError> {
        self.cancel();
        self.wait_settlement(deadline).await
    }

    /// Transfer Event invocation ownership to the supervisor after successful admission.
    ///
    /// ```
    /// # fn accept(handle: rustack_lambda_core::invocation::InvocationHandle) {
    /// handle.detach();
    /// # }
    /// ```
    pub fn detach(mut self) {
        self.cancel_on_drop = false;
    }
}

impl Drop for InvocationHandle {
    fn drop(&mut self) {
        if self.cancel_on_drop {
            self.cancel();
        }
    }
}

/// Owned queued submission, retaining cleanup even when admission observation fails.
#[non_exhaustive]
pub struct InvocationSubmission {
    handle: InvocationHandle,
    accepted: Option<oneshot::Receiver<Result<(), LambdaServiceError>>>,
    admitted: bool,
}
impl fmt::Debug for InvocationSubmission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InvocationSubmission")
            .field("handle", &self.handle)
            .field("admitted", &self.admitted)
            .finish_non_exhaustive()
    }
}
impl InvocationSubmission {
    pub(crate) fn new(
        handle: InvocationHandle,
        accepted: oneshot::Receiver<Result<(), LambdaServiceError>>,
    ) -> Self {
        Self {
            handle,
            accepted: Some(accepted),
            admitted: false,
        }
    }
    /// Await execution admission while keeping the cleanup receipt on every path.
    ///
    /// # Errors
    /// Returns service rejection, cancellation/deadline or a lost admission owner.
    /// Dropping this waiter does not drop the submission or restart admission.
    ///
    /// ```no_run
    /// # async fn accepted(submission:&mut rustack_lambda_core::invocation::InvocationSubmission) {
    /// let result=submission.admitted().await;
    /// # }
    /// ```
    pub async fn admitted(&mut self) -> Result<(), LambdaServiceError> {
        if self.admitted {
            return Ok(());
        }
        let Some(receiver) = &mut self.accepted else {
            return Err(admission_error(
                "Invocation admission result already consumed",
            ));
        };
        let result = tokio::select! {
            biased;
            reason=self.handle.context.cancelled()=>{self.handle.cancel();return Err(reason.error().into());},
            result=receiver=>result,
        };
        self.accepted = None;
        result.map_err(|_| admission_error("Invocation admission owner lost"))??;
        self.handle
            .context
            .check()
            .map_err(LambdaServiceError::from)?;
        self.admitted = true;
        Ok(())
    }
    /// Signal cancellation before admission completes without consuming ownership.
    ///
    /// ```
    /// # fn cancel(submission:&rustack_lambda_core::invocation::InvocationSubmission) {submission.cancel();}
    /// ```
    pub fn cancel(&self) {
        self.handle.cancel();
    }
    /// Transfer the invocation handle even after admission rejection or timeout.
    ///
    /// ```
    /// # fn transfer(submission:rustack_lambda_core::invocation::InvocationSubmission) {
    /// let retained=submission.into_handle();
    /// # }
    /// ```
    #[must_use]
    pub fn into_handle(self) -> InvocationHandle {
        self.handle
    }
}
fn admission_error(message: &str) -> LambdaServiceError {
    LambdaServiceError::ResourceNotReady {
        message: message.into(),
    }
}

/// Single cleanup operation whose future survives cancellation of any waiter.
#[derive(Default)]
pub(crate) struct CleanupTask {
    state: OwnerMutex<Option<Arc<Mutex<CleanupState>>>>,
}

enum CleanupState {
    Running(JoinHandle<Result<(), ExecutorError>>),
    Complete(Result<(), ExecutorError>),
}

impl fmt::Debug for CleanupTask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CleanupTask")
            .field("started", &self.state.lock().is_some())
            .finish()
    }
}
impl fmt::Debug for CleanupState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Running(_) => "Running",
            Self::Complete(Ok(())) => "Clean",
            Self::Complete(Err(_)) => "Failed",
        })
    }
}

impl CleanupTask {
    #[cfg(test)]
    pub(crate) fn start<F, Fut>(&self, factory: F) -> bool
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(), ExecutorError>> + Send + 'static,
    {
        self.start_on(&Handle::current(), factory)
    }

    pub(crate) fn start_on<F, Fut>(&self, runtime: &Handle, factory: F) -> bool
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(), ExecutorError>> + Send + 'static,
    {
        let mut state = self.state.lock();
        if state.is_some() {
            return false;
        }
        // Retain the JoinHandle before returning; dropping a waiter never drops this future.
        *state = Some(Arc::new(Mutex::new(CleanupState::Running(
            runtime.spawn(factory()),
        ))));
        true
    }

    pub(crate) async fn wait(&self) -> Result<(), ExecutorError> {
        let state = self.state.lock().clone();
        let Some(state) = state else {
            return Ok(());
        };
        let mut state = state.lock().await;
        if let CleanupState::Running(task) = &mut *state {
            let result = task.await.unwrap_or_else(|_| {
                Err(ExecutorError::CleanupFailed(
                    "Cleanup supervisor task failed".into(),
                ))
            });
            *state = CleanupState::Complete(result);
        }
        match &*state {
            CleanupState::Complete(result) => result.clone(),
            CleanupState::Running(_) => Err(ExecutorError::CleanupFailed(
                "Cleanup supervisor is unsettled".into(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bytes::Bytes;
    use tokio::sync::Semaphore;

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn test_should_prioritize_absolute_expiry_and_notice_closed_cancellation_owner() {
        let deadline = Instant::now() + Duration::from_secs(2);
        let (cancel, context) = InvocationContext::channel(deadline);
        assert!(context.check().is_ok());
        cancel.send_replace(true);
        assert_eq!(context.cancelled().await, CancellationReason::Requested);
        tokio::time::advance(Duration::from_secs(2)).await;
        assert_eq!(context.cancelled().await, CancellationReason::Deadline);
        assert!(matches!(
            context.check(),
            Err(ExecutorError::DeadlineExceeded)
        ));

        let (cancel, context) = InvocationContext::channel(Instant::now() + Duration::from_secs(2));
        drop(cancel);
        assert_eq!(context.cancelled().await, CancellationReason::Requested);
        assert!(matches!(context.check(), Err(ExecutorError::Cancelled)));
    }

    #[tokio::test(start_paused = true)]
    async fn test_should_discard_a_result_received_after_the_attempt_deadline() {
        let (cancel, context) = InvocationContext::channel(Instant::now() + Duration::from_secs(1));
        let (response, received) = oneshot::channel();
        let (_, settled) = watch::channel(Settlement::Clean);
        let handle = InvocationHandle::new(context, cancel.clone(), received, settled);
        response
            .send(Ok(InvokeResponse::success(Bytes::new(), "$LATEST".into())))
            .unwrap();
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(matches!(
            handle.wait().await,
            Err(ExecutorError::DeadlineExceeded)
        ));
        assert!(
            *cancel.borrow(),
            "expiry must request owned backend cleanup"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_should_wait_for_cleanup_after_expiry_and_detach_only_explicitly() {
        let (cancel, context) = InvocationContext::channel(Instant::now());
        let (_, received) = oneshot::channel();
        let (settlement, settled) = watch::channel(Settlement::Pending);
        let handle = InvocationHandle::new(context, cancel.clone(), received, settled);
        settlement.send_replace(Settlement::Clean);
        handle
            .cancel_and_wait(Instant::now() + Duration::from_secs(2))
            .await
            .unwrap();
        assert!(*cancel.borrow());

        let (cancel, context) = InvocationContext::channel(Instant::now() + Duration::from_secs(2));
        let (_, received) = oneshot::channel();
        let (_, settled) = watch::channel(Settlement::Pending);
        InvocationHandle::new(context, cancel.clone(), received, settled).detach();
        assert!(!*cancel.borrow(), "Event detach preserves admitted work");
    }

    #[tokio::test]
    async fn test_should_retain_cleanup_resources_when_a_waiter_is_aborted() {
        let cleanup = Arc::new(CleanupTask::default());
        let resources = Arc::new(Semaphore::new(1));
        let reservation = resources.clone().acquire_owned().await.unwrap();
        let release = Arc::new(Semaphore::new(0));
        let ready = Arc::new(Semaphore::new(0));
        let gate = Arc::clone(&release);
        let started = Arc::clone(&ready);
        assert!(cleanup.start(move || async move {
            let _reservation = reservation;
            started.add_permits(1);
            gate.acquire().await.unwrap().forget();
            Ok(())
        }));
        ready.acquire().await.unwrap().forget();
        let owner = Arc::clone(&cleanup);
        let waiter = tokio::spawn(async move { owner.wait().await });
        tokio::task::yield_now().await;
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        assert_eq!(resources.available_permits(), 0);
        assert!(!cleanup.start(|| async { Ok(()) }));

        release.add_permits(1);
        let (first, second) = tokio::join!(cleanup.wait(), cleanup.wait());
        first.unwrap();
        second.unwrap();
        assert_eq!(resources.available_permits(), 1);
        cleanup.wait().await.unwrap();
    }

    #[tokio::test]
    async fn test_should_preserve_failed_cleanup_and_supervised_panics_for_all_waiters() {
        let cleanup = CleanupTask::default();
        assert!(cleanup.start(|| async { Err(ExecutorError::CleanupFailed("failed".into())) }));
        for _ in 0..2 {
            assert!(
                matches!(cleanup.wait().await, Err(ExecutorError::CleanupFailed(message)) if message == "failed")
            );
        }
        let cleanup = CleanupTask::default();
        assert!(cleanup.start(|| async { panic!("intentional cleanup task panic") }));
        for _ in 0..2 {
            assert!(matches!(
                cleanup.wait().await,
                Err(ExecutorError::CleanupFailed(_))
            ));
        }
    }
}

#[cfg(test)]
mod submission_tests {
    use std::time::Duration;

    use bytes::Bytes;

    use super::*;
    #[tokio::test(start_paused = true)]
    async fn test_should_propagate_external_cancellation_and_keep_deadline_precedence() {
        let (external, signal) = watch::channel(false);
        let (_own, context) = InvocationContext::channel_with_cancellation(
            Instant::now() + Duration::from_secs(1),
            Some(signal),
        );
        external.send_replace(true);
        assert_eq!(context.cancelled().await, CancellationReason::Requested);
        tokio::time::advance(Duration::from_secs(1)).await;
        assert_eq!(context.cancelled().await, CancellationReason::Deadline);
        let (external, signal) = watch::channel(false);
        let (_own, context) = InvocationContext::channel_with_cancellation(
            Instant::now() + Duration::from_secs(1),
            Some(signal),
        );
        drop(external);
        assert!(matches!(context.check(), Err(ExecutorError::Cancelled)));
        assert_eq!(context.cancelled().await, CancellationReason::Requested);
    }
    #[tokio::test(start_paused = true)]
    async fn test_should_keep_cleanup_receipt_after_rejected_admission_and_waiter_timeout() {
        let (cancel, context) =
            InvocationContext::channel(Instant::now() + Duration::from_secs(10));
        let (_response, receiver) = oneshot::channel();
        let (settlement, settled) = watch::channel(Settlement::Pending);
        let handle = InvocationHandle::new(context, cancel, receiver, settled);
        let (admitted, accepted) = oneshot::channel();
        let mut submission = InvocationSubmission::new(handle, accepted);
        admitted
            .send(Err(LambdaServiceError::TooManyRequests))
            .unwrap();
        assert!(matches!(
            submission.admitted().await,
            Err(LambdaServiceError::TooManyRequests)
        ));
        let mut handle = submission.into_handle();
        assert!(
            handle
                .wait_settlement(Instant::now() + Duration::from_secs(1))
                .await
                .is_err()
        );
        settlement.send_replace(Settlement::Clean);
        handle.wait_cleanup().await.unwrap();
    }
    #[tokio::test(start_paused = true)]
    async fn test_should_resume_response_wait_and_reject_second_consumption_without_panic() {
        let (cancel, context) =
            InvocationContext::channel(Instant::now() + Duration::from_secs(10));
        let (response, receiver) = oneshot::channel();
        let (settlement, settled) = watch::channel(Settlement::Pending);
        let mut handle = InvocationHandle::new(context, cancel.clone(), receiver, settled);
        assert!(
            timeout_at(
                Instant::now() + Duration::from_secs(1),
                handle.wait_response()
            )
            .await
            .is_err()
        );
        assert!(!*cancel.borrow());
        response
            .send(Ok(InvokeResponse::success(
                Bytes::from_static(b"5"),
                "$LATEST".into(),
            )))
            .unwrap();
        assert_eq!(
            handle.wait_response().await.unwrap().payload,
            Bytes::from_static(b"5")
        );
        assert!(matches!(
            handle.wait_response().await,
            Err(ExecutorError::CleanupFailed(_))
        ));
        settlement.send_replace(Settlement::Clean);
        handle.wait_cleanup().await.unwrap();
    }
    #[tokio::test(start_paused = true)]
    async fn test_should_not_certify_uncertain_or_lost_cleanup_from_owned_submission() {
        for state in [Settlement::Pending, Settlement::Uncertain] {
            let (cancel, context) =
                InvocationContext::channel(Instant::now() + Duration::from_secs(10));
            let (_response, receiver) = oneshot::channel();
            let (settlement, settled) = watch::channel(state);
            let mut handle = InvocationHandle::new(context, cancel, receiver, settled);
            drop(settlement);
            assert!(matches!(
                handle.wait_cleanup().await,
                Err(ExecutorError::CleanupFailed(_))
            ));
        }
    }
}
