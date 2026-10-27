//! Bounded invocation admission and a single owner for execution/cleanup futures.

use std::{
    collections::HashMap,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use tokio::{
    sync::{Mutex, Notify, OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch},
    task::{Id, JoinHandle, JoinSet},
    time::{Instant, timeout},
};
use typed_builder::TypedBuilder;

use crate::{
    diagnostics::{
        ACCEPTED_LIMIT, DEFAULT_FUNCTION_LIMIT, EXECUTION_LIMIT, InvocationWorkLimits,
        InvocationWorkStatus,
    },
    error::LambdaServiceError,
    executor::{Executor, ExecutorError, InvokeRequest, InvokeResponse},
    invocation::{InvocationContext, InvocationHandle, InvocationSubmission, Settlement},
};

#[derive(Debug)]
pub(crate) struct WorkManager {
    accepting: Arc<AtomicBool>,
    capacity: Arc<Semaphore>,
    global: Arc<Semaphore>,
    drain: Arc<Notify>,
    supervisor: OnceLock<SupervisorHandle>,
}

#[derive(Debug)]
struct SupervisorHandle {
    inbox: mpsc::Sender<Submission>,
    stop: watch::Sender<bool>,
    state: watch::Receiver<SupervisorState>,
    task: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SupervisorState {
    Running,
    Stopped { failed: bool },
}

#[derive(Debug, TypedBuilder)]
struct Submission {
    executor: Arc<dyn Executor>,
    request: InvokeRequest,
    context: InvocationContext,
    cancellation: watch::Sender<bool>,
    reserved: usize,
    asynchronous: bool,
    slot: OwnedSemaphorePermit,
    admitted: oneshot::Sender<Result<(), LambdaServiceError>>,
    response: oneshot::Sender<Result<InvokeResponse, ExecutorError>>,
    settlement: watch::Sender<Settlement>,
}

#[derive(Debug, TypedBuilder)]
struct Supervisor {
    inbox: mpsc::Receiver<Submission>,
    stop: watch::Receiver<bool>,
    drain: Arc<Notify>,
    accepting: Arc<AtomicBool>,
    global: Arc<Semaphore>,
    state: watch::Sender<SupervisorState>,
    #[builder(default)]
    jobs: JoinSet<bool>,
    #[builder(default)]
    cancellations: HashMap<Id, watch::Sender<bool>>,
    #[builder(default)]
    functions: HashMap<String, Arc<FunctionCapacity>>,
}

#[derive(Debug, Default)]
struct FunctionCapacity {
    active: AtomicUsize,
    changed: Notify,
}

#[derive(Debug)]
struct FunctionPermit(Arc<FunctionCapacity>);
impl Drop for FunctionPermit {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::AcqRel);
        self.0.changed.notify_waiters();
    }
}

#[cfg(test)]
#[derive(Debug)]
pub(crate) struct CancelOnDrop(pub(crate) tokio::task::AbortHandle);
#[cfg(test)]
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn unavailable(message: &str) -> LambdaServiceError {
    LambdaServiceError::ResourceNotReady {
        message: message.into(),
    }
}

impl FunctionCapacity {
    fn try_acquire(self: &Arc<Self>, limit: usize) -> Result<FunctionPermit, LambdaServiceError> {
        self.active
            .try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                if count < limit {
                    count.checked_add(1)
                } else {
                    None
                }
            })
            .map_err(|_| LambdaServiceError::TooManyRequests)?;
        Ok(FunctionPermit(Arc::clone(self)))
    }

    async fn acquire(self: &Arc<Self>, limit: usize) -> FunctionPermit {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if let Ok(permit) = self.try_acquire(limit) {
                return permit;
            }
            changed.await;
        }
    }
}

impl WorkManager {
    pub(crate) fn new() -> Self {
        Self {
            accepting: Arc::new(AtomicBool::new(true)),
            capacity: Arc::new(Semaphore::new(ACCEPTED_LIMIT)),
            global: Arc::new(Semaphore::new(EXECUTION_LIMIT)),
            drain: Arc::new(Notify::new()),
            supervisor: OnceLock::new(),
        }
    }

    pub(crate) fn status(&self) -> InvocationWorkStatus {
        InvocationWorkStatus {
            accepted_invocations: ACCEPTED_LIMIT.saturating_sub(self.capacity.available_permits()),
            active_executions: EXECUTION_LIMIT.saturating_sub(self.global.available_permits()),
            accepting: self.accepting.load(Ordering::Acquire),
            limits: InvocationWorkLimits {
                accepted_invocations: ACCEPTED_LIMIT,
                active_executions: EXECUTION_LIMIT,
                default_function_executions: DEFAULT_FUNCTION_LIMIT,
            },
        }
    }

    fn supervisor(&self) -> &SupervisorHandle {
        self.supervisor.get_or_init(|| {
            let (inbox, receiver) = mpsc::channel(ACCEPTED_LIMIT);
            let (stop, cancellation) = watch::channel(false);
            let (state, snapshot) = watch::channel(SupervisorState::Running);
            let supervisor = Supervisor::builder()
                .inbox(receiver)
                .stop(cancellation)
                .drain(Arc::clone(&self.drain))
                .accepting(Arc::clone(&self.accepting))
                .global(Arc::clone(&self.global))
                .state(state)
                .build();
            let task = tokio::spawn(supervisor.run());
            SupervisorHandle {
                inbox,
                stop,
                state: snapshot,
                task: Mutex::new(Some(task)),
            }
        })
    }

    #[cfg(test)]
    pub(crate) async fn submit(
        &self,
        executor: Arc<dyn Executor>,
        request: InvokeRequest,
        reserved: Option<i32>,
        asynchronous: bool,
        caller_deadline: Option<Instant>,
    ) -> Result<InvocationHandle, LambdaServiceError> {
        let mut submission = self.submit_owned(
            executor,
            request,
            reserved,
            asynchronous,
            caller_deadline,
            None,
        )?;
        submission.admitted().await?;
        Ok(submission.into_handle())
    }

    pub(crate) fn submit_owned(
        &self,
        executor: Arc<dyn Executor>,
        request: InvokeRequest,
        reserved: Option<i32>,
        asynchronous: bool,
        caller_deadline: Option<Instant>,
        external: Option<watch::Receiver<bool>>,
    ) -> Result<InvocationSubmission, LambdaServiceError> {
        executor.available(&request)?;
        if !self.accepting.load(Ordering::Acquire) {
            return Err(unavailable("Lambda is quiescing"));
        }
        let reserved = match reserved {
            Some(reserved) => usize::try_from(reserved)
                .map_err(|_| unavailable("Invalid reserved concurrency"))?,
            None => DEFAULT_FUNCTION_LIMIT,
        };
        if reserved == 0 {
            return Err(LambdaServiceError::TooManyRequests);
        }
        let slot = Arc::clone(&self.capacity)
            .try_acquire_owned()
            .map_err(|_| LambdaServiceError::TooManyRequests)?;
        let total = request
            .revision
            .timeout
            .saturating_add(Duration::from_secs(10));
        let deadline = Instant::now()
            .checked_add(total)
            .ok_or_else(|| unavailable("Invalid invocation budget"))?;
        let deadline = caller_deadline.map_or(deadline, |caller| caller.min(deadline));
        let (cancellation, context) =
            InvocationContext::channel_with_cancellation(deadline, external);
        context.check()?;
        let (admitted, accepted) = oneshot::channel();
        let (response, receiver) = oneshot::channel();
        let (settlement, settled) = watch::channel(Settlement::Pending);
        let handle =
            InvocationHandle::new(context.clone(), cancellation.clone(), receiver, settled);
        let submission = Submission::builder()
            .executor(executor)
            .request(request)
            .context(context.clone())
            .cancellation(cancellation)
            .reserved(reserved)
            .asynchronous(asynchronous)
            .slot(slot)
            .admitted(admitted)
            .response(response)
            .settlement(settlement)
            .build();
        self.supervisor()
            .inbox
            .try_send(submission)
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => LambdaServiceError::TooManyRequests,
                mpsc::error::TrySendError::Closed(_) => unavailable("Lambda supervisor stopped"),
            })?;
        Ok(InvocationSubmission::new(handle, accepted))
    }

    pub(crate) fn cancel_all(&self) {
        self.accepting.store(false, Ordering::Release);
        if let Some(supervisor) = self.supervisor.get() {
            supervisor.stop.send_replace(true);
        }
        self.drain.notify_one();
    }

    pub(crate) async fn quiesce(&self, budget: Duration) -> Result<(), LambdaServiceError> {
        self.accepting.store(false, Ordering::Release);
        self.drain.notify_one();
        let Some(supervisor) = self.supervisor.get() else {
            return Ok(());
        };
        let mut state = supervisor.state.clone();
        let clean = {
            let result = timeout(
                budget,
                state.wait_for(|state| matches!(state, SupervisorState::Stopped { .. })),
            )
            .await;
            match result {
                Ok(Ok(state)) => matches!(*state, SupervisorState::Stopped { failed: false }),
                _ => false,
            }
        };
        if !clean {
            supervisor.stop.send_replace(true);
            return Err(unavailable(
                "Lambda quiesce did not confirm all invocation cleanup",
            ));
        }
        let mut task = supervisor.task.lock().await;
        if let Some(task) = task.as_mut() {
            task.await
                .map_err(|_| unavailable("Lambda supervisor failed"))?;
        }
        task.take();
        Ok(())
    }
}

impl Drop for WorkManager {
    fn drop(&mut self) {
        // Fallback only: the detached actor keeps cleanup futures alive until settlement.
        // Normal provider shutdown awaits quiescence before dropping this owner.
        self.cancel_all();
    }
}

impl Supervisor {
    fn cancel_all(&self) {
        for cancellation in self.cancellations.values() {
            cancellation.send_replace(true);
        }
    }

    fn admit(&mut self, submission: Submission) {
        let result = if self.accepting.load(Ordering::Acquire) {
            submission.context.check().map_err(LambdaServiceError::from)
        } else {
            Err(unavailable("Lambda is quiescing"))
        };
        if let Err(error) = result {
            reject(submission, error);
            return;
        }
        self.functions.retain(|_, function| {
            Arc::strong_count(function) > 1 || function.active.load(Ordering::Acquire) != 0
        });
        let function = Arc::clone(
            self.functions
                .entry(submission.request.revision.function_name.clone())
                .or_default(),
        );
        let permits = if submission.asynchronous {
            None
        } else {
            match function
                .try_acquire(submission.reserved)
                .and_then(|function| {
                    Arc::clone(&self.global)
                        .try_acquire_owned()
                        .map(|global| (function, global))
                        .map_err(|_| LambdaServiceError::TooManyRequests)
                }) {
                Ok(permits) => Some(permits),
                Err(error) => {
                    reject(submission, error);
                    return;
                }
            }
        };
        let Submission {
            executor,
            request,
            context,
            cancellation,
            reserved,
            slot,
            admitted,
            response,
            settlement,
            ..
        } = submission;
        let global = Arc::clone(&self.global);
        let worker = self.jobs.spawn(async move {
            let result = execute(
                executor, request, context, function, global, reserved, permits,
            )
            .await;
            let clean = !matches!(&result, Err(ExecutorError::CleanupFailed(_)));
            // Backend cleanup completes before permits and the admission slot are released.
            drop(slot);
            settlement.send_replace(if clean {
                Settlement::Clean
            } else {
                Settlement::Uncertain
            });
            let _ = response.send(result);
            clean
        });
        self.cancellations.insert(worker.id(), cancellation);
        let _ = admitted.send(Ok(()));
    }

    async fn run(mut self) {
        let mut failed = false;
        loop {
            if !self.accepting.load(Ordering::Acquire) || *self.stop.borrow() {
                self.inbox.close();
            }
            if *self.stop.borrow() {
                self.cancel_all();
            }
            if self.inbox.is_closed() && self.inbox.is_empty() && self.jobs.is_empty() {
                break;
            }
            tokio::select! {
                biased;
                _ = self.stop.changed(), if !*self.stop.borrow() => { self.accepting.store(false, Ordering::Release); }
                () = self.drain.notified() => {}
                completed = self.jobs.join_next_with_id(), if !self.jobs.is_empty() => {
                    match completed {
                        Some(Ok((id, clean))) => {
                            self.cancellations.remove(&id);
                            if !clean { failed = true; }
                        }
                        Some(Err(error)) => {
                            self.cancellations.remove(&error.id());
                            failed = true;
                            tracing::error!("Lambda invocation worker terminated without confirmed settlement");
                        }
                        None => {}
                    }
                    if failed { self.accepting.store(false, Ordering::Release); self.cancel_all(); }
                }
                submission = self.inbox.recv(), if !self.inbox.is_closed() || !self.inbox.is_empty() => {
                    if let Some(submission) = submission { self.admit(submission); }
                }
            }
        }
        self.accepting.store(false, Ordering::Release);
        self.state.send_replace(SupervisorState::Stopped { failed });
    }
}

fn reject(submission: Submission, error: LambdaServiceError) {
    let Submission {
        slot,
        admitted,
        response,
        settlement,
        ..
    } = submission;
    drop(slot);
    settlement.send_replace(Settlement::Clean);
    let _ = response.send(Err(ExecutorError::Cancelled));
    let _ = admitted.send(Err(error));
}

async fn execute(
    executor: Arc<dyn Executor>,
    request: InvokeRequest,
    context: InvocationContext,
    function: Arc<FunctionCapacity>,
    global: Arc<Semaphore>,
    limit: usize,
    permits: Option<(FunctionPermit, OwnedSemaphorePermit)>,
) -> Result<InvokeResponse, ExecutorError> {
    let acquired = async {
        match permits {
            Some(permits) => Ok::<_, ExecutorError>(permits),
            None => Ok((
                function.acquire(limit).await,
                global
                    .acquire_owned()
                    .await
                    .map_err(|_| ExecutorError::Cancelled)?,
            )),
        }
    };
    let (_function, _global) = tokio::select! {
        biased;
        reason = context.cancelled() => return Err(reason.error()),
        result = acquired => result?,
    };
    context.check()?;
    executor.invoke(request, context).await
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use async_trait::async_trait;
    use tokio::sync::Semaphore;

    use super::*;

    #[derive(Debug)]
    struct ProbeExecutor {
        started: Semaphore,
        cancelling: Semaphore,
        cleanup: Semaphore,
        hold_cleanup: AtomicBool,
        calls: AtomicUsize,
    }
    impl ProbeExecutor {
        fn new(hold_cleanup: bool) -> Self {
            Self {
                started: Semaphore::new(0),
                cancelling: Semaphore::new(0),
                cleanup: Semaphore::new(0),
                hold_cleanup: AtomicBool::new(hold_cleanup),
                calls: AtomicUsize::new(0),
            }
        }
        async fn started(&self, count: u32) {
            timeout(Duration::from_secs(2), self.started.acquire_many(count))
                .await
                .unwrap()
                .unwrap()
                .forget();
        }
        async fn cancelling(&self) {
            timeout(Duration::from_secs(2), self.cancelling.acquire())
                .await
                .unwrap()
                .unwrap()
                .forget();
        }
    }
    #[async_trait]
    impl Executor for ProbeExecutor {
        async fn invoke(
            &self,
            _request: InvokeRequest,
            context: InvocationContext,
        ) -> Result<InvokeResponse, ExecutorError> {
            self.calls.fetch_add(1, Ordering::AcqRel);
            self.started.add_permits(1);
            let reason = context.cancelled().await;
            self.cancelling.add_permits(1);
            if self.hold_cleanup.load(Ordering::Acquire) {
                self.cleanup
                    .acquire()
                    .await
                    .map_err(|_| ExecutorError::CleanupFailed("Probe cleanup closed".into()))?
                    .forget();
            }
            Err(reason.error())
        }
        async fn shutdown(&self) -> Result<(), ExecutorError> {
            Ok(())
        }
    }
    fn request(name: &str) -> InvokeRequest {
        let mut request = InvokeRequest::fixture();
        let revision = Arc::make_mut(&mut request.revision);
        revision.function_name = name.into();
        revision.timeout = Duration::from_secs(60);
        request
    }

    #[tokio::test]
    async fn test_should_observe_actual_accepted_active_and_retained_cleanup_without_admission() {
        let work = WorkManager::new();
        let idle = work.status();
        assert!(work.supervisor.get().is_none());
        assert_eq!(
            (
                idle.accepted_invocations,
                idle.active_executions,
                idle.accepting
            ),
            (0, 0, true)
        );
        let serialized = serde_json::to_value(idle).unwrap();
        assert_eq!(
            serialized,
            serde_json::json!({"acceptedInvocations":0,"activeExecutions":0,"accepting":true,
            "limits":{"acceptedInvocations":128,"activeExecutions":32,"defaultFunctionExecutions":8}})
        );
        let executor = Arc::new(ProbeExecutor::new(true));
        for _ in 0..3 {
            work.submit(executor.clone(), request("one"), Some(1), true, None)
                .await
                .unwrap()
                .detach();
        }
        executor.started(1).await;
        let active = work.status();
        assert_eq!(
            (active.accepted_invocations, active.active_executions),
            (3, 1)
        );
        work.cancel_all();
        executor.cancelling().await;
        timeout(Duration::from_secs(2), async {
            while work.status().accepted_invocations != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let retained = work.status();
        assert_eq!(
            (
                retained.accepted_invocations,
                retained.active_executions,
                retained.accepting
            ),
            (1, 1, false)
        );
        executor.cleanup.add_permits(1);
        work.quiesce(Duration::from_secs(2)).await.unwrap();
        let settled = work.status();
        assert_eq!(
            (
                settled.accepted_invocations,
                settled.active_executions,
                settled.accepting
            ),
            (0, 0, false)
        );
    }

    #[tokio::test]
    async fn test_should_bound_event_admission_and_cancel_queued_work_without_starting_it() {
        let work = WorkManager::new();
        let executor = Arc::new(ProbeExecutor::new(false));
        for _ in 0..128 {
            work.submit(executor.clone(), request("bounded"), None, true, None)
                .await
                .unwrap()
                .detach();
        }
        executor.started(8).await;
        assert_eq!(
            (
                work.status().accepted_invocations,
                work.status().active_executions
            ),
            (128, 8)
        );
        assert_eq!(executor.calls.load(Ordering::Acquire), 8);
        assert!(matches!(
            work.submit(executor.clone(), request("bounded"), None, true, None)
                .await,
            Err(LambdaServiceError::TooManyRequests)
        ));
        work.cancel_all();
        work.quiesce(Duration::from_secs(2)).await.unwrap();
        assert_eq!(work.capacity.available_permits(), 128);
        assert_eq!(work.global.available_permits(), 32);
        assert_eq!(executor.calls.load(Ordering::Acquire), 8);
        assert!(
            work.submit(executor, request("bounded"), None, true, None)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn test_should_enforce_shared_function_global_and_zero_reserved_execution_limits() {
        let work = WorkManager::new();
        let executor = Arc::new(ProbeExecutor::new(false));
        assert!(matches!(
            work.submit(executor.clone(), request("zero"), Some(0), true, None)
                .await,
            Err(LambdaServiceError::TooManyRequests)
        ));
        let mut handles = Vec::new();
        for name in ["a", "b", "c", "d"] {
            for _ in 0..8 {
                handles.push(
                    work.submit(executor.clone(), request(name), None, false, None)
                        .await
                        .unwrap(),
                );
            }
            assert!(matches!(
                work.submit(executor.clone(), request(name), None, false, None)
                    .await,
                Err(LambdaServiceError::TooManyRequests)
            ));
        }
        assert_eq!(work.global.available_permits(), 0);
        assert!(matches!(
            work.submit(executor, request("e"), Some(32), false, None)
                .await,
            Err(LambdaServiceError::TooManyRequests)
        ));
        drop(handles);
        work.quiesce(Duration::from_secs(2)).await.unwrap();
        assert_eq!(work.global.available_permits(), 32);
    }

    #[tokio::test]
    async fn test_should_hold_permits_until_cancelled_backend_cleanup_is_confirmed() {
        let work = WorkManager::new();
        let executor = Arc::new(ProbeExecutor::new(true));
        let handle = work
            .submit(executor.clone(), request("cleanup"), Some(1), false, None)
            .await
            .unwrap();
        executor.started(1).await;
        handle.cancel();
        executor.cancelling().await;
        assert_eq!(work.capacity.available_permits(), 127);
        assert_eq!(work.global.available_permits(), 31);
        assert!(matches!(
            work.submit(executor.clone(), request("cleanup"), Some(1), false, None)
                .await,
            Err(LambdaServiceError::TooManyRequests)
        ));
        executor.cleanup.add_permits(1);
        handle
            .cancel_and_wait(Instant::now() + Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(work.capacity.available_permits(), 128);
        assert_eq!(work.global.available_permits(), 32);
        work.quiesce(Duration::from_secs(2)).await.unwrap();
    }

    #[tokio::test]
    async fn test_should_keep_cleanup_owned_after_handle_deadline_and_aborted_quiesce_waiter() {
        let work = Arc::new(WorkManager::new());
        let executor = Arc::new(ProbeExecutor::new(true));
        let handle = work
            .submit(executor.clone(), request("retained"), None, false, None)
            .await
            .unwrap();
        executor.started(1).await;
        assert!(matches!(
            handle.cancel_and_wait(Instant::now()).await,
            Err(ExecutorError::CleanupFailed(_))
        ));
        executor.cancelling().await;
        let waiting = Arc::clone(&work);
        let task = tokio::spawn(async move { waiting.quiesce(Duration::from_secs(30)).await });
        tokio::task::yield_now().await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(work.capacity.available_permits(), 127);
        executor.cleanup.add_permits(1);
        work.quiesce(Duration::from_secs(2)).await.unwrap();
        assert_eq!(work.capacity.available_permits(), 128);
    }

    #[tokio::test]
    async fn test_should_cancel_on_handle_drop_and_reject_already_expired_admission() {
        let work = WorkManager::new();
        let executor = Arc::new(ProbeExecutor::new(false));
        assert!(matches!(
            work.submit(
                executor.clone(),
                request("expired"),
                None,
                false,
                Some(Instant::now())
            )
            .await,
            Err(LambdaServiceError::ResourceNotReady { .. })
        ));
        let handle = work
            .submit(executor.clone(), request("drop"), None, false, None)
            .await
            .unwrap();
        executor.started(1).await;
        drop(handle);
        executor.cancelling().await;
        work.quiesce(Duration::from_secs(2)).await.unwrap();
        assert_eq!(work.capacity.available_permits(), 128);
    }

    #[tokio::test(start_paused = true)]
    async fn test_should_return_at_caller_deadline_while_retaining_backend_cleanup_capacity() {
        let work = WorkManager::new();
        let executor = Arc::new(ProbeExecutor::new(true));
        let handle = work
            .submit(
                executor.clone(),
                request("deadline"),
                None,
                false,
                Some(Instant::now() + Duration::from_secs(1)),
            )
            .await
            .unwrap();
        executor.started(1).await;
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(matches!(
            handle.wait().await,
            Err(ExecutorError::DeadlineExceeded)
        ));
        executor.cancelling().await;
        assert_eq!(work.global.available_permits(), 31);
        assert_eq!(work.capacity.available_permits(), 127);
        assert!(work.quiesce(Duration::ZERO).await.is_err());
        executor.cleanup.add_permits(1);
        work.quiesce(Duration::from_secs(2)).await.unwrap();
        assert_eq!(work.global.available_permits(), 32);
        assert_eq!(work.capacity.available_permits(), 128);
    }

    #[derive(Debug)]
    struct PanickingExecutor;
    #[async_trait]
    impl Executor for PanickingExecutor {
        async fn invoke(
            &self,
            _request: InvokeRequest,
            _context: InvocationContext,
        ) -> Result<InvokeResponse, ExecutorError> {
            panic!("intentional supervised panic");
        }
        async fn shutdown(&self) -> Result<(), ExecutorError> {
            Ok(())
        }
    }
    #[tokio::test]
    async fn test_should_never_report_successful_quiescence_after_a_worker_panic() {
        let work = WorkManager::new();
        let handle = work
            .submit(
                Arc::new(PanickingExecutor),
                request("panic"),
                None,
                false,
                None,
            )
            .await
            .unwrap();
        assert!(matches!(
            handle.wait().await,
            Err(ExecutorError::CleanupFailed(_))
        ));
        assert!(work.quiesce(Duration::from_secs(2)).await.is_err());
        assert!(
            work.submit(
                Arc::new(ProbeExecutor::new(false)),
                request("after-panic"),
                None,
                false,
                None
            )
            .await
            .is_err()
        );
    }
}
