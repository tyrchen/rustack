//! Bounded, supervised invocation lifecycle. No detached Event tasks.
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use dashmap::DashMap;
use tokio::{
    sync::{Mutex, Notify, OwnedSemaphorePermit, Semaphore, oneshot},
    task::{AbortHandle, JoinSet},
};

use crate::{
    error::LambdaServiceError,
    executor::{Executor, InvokeRequest, InvokeResponse},
};

#[derive(Debug)]
pub(crate) struct WorkManager {
    tasks: Mutex<JoinSet<()>>,
    accepting: AtomicBool,
    capacity: Arc<Semaphore>,
    global: Arc<Semaphore>,
    functions: DashMap<String, Arc<FunctionCapacity>>,
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
#[derive(Debug)]
pub(crate) struct CancelOnDrop(pub(crate) AbortHandle);
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
fn throttled() -> LambdaServiceError {
    LambdaServiceError::TooManyRequests
}

impl FunctionCapacity {
    fn try_acquire(self: &Arc<Self>, limit: usize) -> Result<FunctionPermit, LambdaServiceError> {
        self.active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                if count < limit {
                    count.checked_add(1)
                } else {
                    None
                }
            })
            .map_err(|_| throttled())?;
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
            tasks: Mutex::new(JoinSet::new()),
            accepting: AtomicBool::new(true),
            capacity: Arc::new(Semaphore::new(128)),
            global: Arc::new(Semaphore::new(32)),
            functions: DashMap::new(),
        }
    }

    pub(crate) async fn submit(
        &self,
        executor: Arc<dyn Executor>,
        req: InvokeRequest,
        reserved: Option<i32>,
        asynchronous: bool,
    ) -> Result<
        (
            oneshot::Receiver<Result<InvokeResponse, LambdaServiceError>>,
            AbortHandle,
        ),
        LambdaServiceError,
    > {
        executor.available()?;
        let limit = usize::try_from(reserved.unwrap_or(8))
            .map_err(|_| unavailable("Invalid reserved concurrency"))?;
        if limit == 0 {
            return Err(throttled());
        }
        let slot = Arc::clone(&self.capacity)
            .try_acquire_owned()
            .map_err(|_| throttled())?;
        self.functions.retain(|_, capacity| {
            Arc::strong_count(capacity) > 1 || capacity.active.load(Ordering::Acquire) != 0
        });
        let function = Arc::clone(
            self.functions
                .entry(req.function_name.clone())
                .or_insert_with(|| Arc::new(FunctionCapacity::default()))
                .value(),
        );
        let permits = if asynchronous {
            None
        } else {
            Some((
                function.try_acquire(limit)?,
                Arc::clone(&self.global)
                    .try_acquire_owned()
                    .map_err(|_| throttled())?,
            ))
        };
        let mut tasks = self.tasks.lock().await;
        while let Some(result) = tasks.try_join_next() {
            if let Err(error) = result {
                tracing::error!(%error, "Lambda worker failed");
            }
        }
        if !self.accepting.load(Ordering::Acquire) {
            return Err(unavailable("Lambda is quiescing"));
        }
        let (tx, rx) = oneshot::channel();
        let global = Arc::clone(&self.global);
        let handle = tasks.spawn(async move {
            let _slot = slot;
            let result = execute(executor, req, function, global, limit, permits).await;
            match &result {
                Ok(_) => tracing::debug!("Lambda invocation completed"),
                Err(error) => tracing::warn!(%error, "Lambda invocation failed"),
            }
            if tx.send(result).is_err() {
                tracing::debug!("Lambda invocation completed without a waiting caller");
            }
        });
        Ok((rx, handle))
    }

    pub(crate) async fn quiesce(&self, timeout: Duration) -> Result<(), LambdaServiceError> {
        self.accepting.store(false, Ordering::Release);
        // Own the JoinSet across awaits so cancellation of quiesce drops/aborts every task.
        let mut tasks = std::mem::take(&mut *self.tasks.lock().await);
        let drained = tokio::time::timeout(timeout, async {
            while let Some(result) = tasks.join_next().await {
                if let Err(error) = result {
                    return Err(unavailable(&format!("Lambda task failed: {error}")));
                }
            }
            // Includes workers aborted by a previously cancelled quiesce future.
            let _all_capacity = Arc::clone(&self.capacity)
                .acquire_many_owned(128)
                .await
                .map_err(|_| unavailable("Lambda accepted-work capacity closed"))?;
            Ok(())
        })
        .await;
        match drained {
            Ok(Ok(())) => Ok(()),
            result => {
                tasks.abort_all();
                while let Some(result) = tasks.join_next().await {
                    if let Err(error) = result {
                        tracing::warn!(%error, "Lambda task cancelled during quiesce");
                    }
                }
                match result {
                    Ok(Err(error)) => Err(error),
                    _ => Err(unavailable(
                        "Lambda quiesce deadline exceeded; remaining invocations cancelled",
                    )),
                }
            }
        }
    }
}

async fn execute(
    executor: Arc<dyn Executor>,
    req: InvokeRequest,
    function: Arc<FunctionCapacity>,
    global: Arc<Semaphore>,
    limit: usize,
    permits: Option<(FunctionPermit, OwnedSemaphorePermit)>,
) -> Result<InvokeResponse, LambdaServiceError> {
    let (_function, _global) = match permits {
        Some(permits) => permits,
        None => (
            function.acquire(limit).await,
            global
                .acquire_owned()
                .await
                .map_err(|_| unavailable("Lambda execution closed"))?,
        ),
    };
    let deadline = req.timeout.saturating_add(Duration::from_secs(10));
    match tokio::time::timeout(deadline, executor.invoke(req)).await {
        Ok(result) => result.map_err(LambdaServiceError::from),
        Err(_) => Err(unavailable("Lambda invocation deadline exceeded")),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use async_trait::async_trait;
    use bytes::Bytes;

    use super::*;
    use crate::executor::{ExecutorError, PackageType};

    #[derive(Debug)]
    struct WaitingExecutor;
    #[async_trait]
    impl Executor for WaitingExecutor {
        async fn invoke(&self, _request: InvokeRequest) -> Result<InvokeResponse, ExecutorError> {
            std::future::pending().await
        }
        async fn shutdown(&self) {}
    }
    fn request(name: &str) -> InvokeRequest {
        InvokeRequest {
            function_name: name.into(),
            function_arn: "arn".into(),
            qualifier: "$LATEST".into(),
            runtime: None,
            handler: None,
            architectures: vec![],
            package_type: PackageType::Zip,
            code_root: None,
            code_zip: None,
            image_uri: None,
            environment: HashMap::new(),
            timeout: Duration::from_mins(1),
            memory_mb: 128,
            payload: Bytes::new(),
            capture_logs: false,
        }
    }
    #[tokio::test]
    async fn test_should_bound_event_set_before_accepting_and_release_on_cancel() {
        let work = WorkManager::new();
        for _ in 0..128 {
            work.submit(Arc::new(WaitingExecutor), request("bounded"), None, true)
                .await
                .unwrap();
        }
        assert!(matches!(
            work.submit(Arc::new(WaitingExecutor), request("bounded"), None, true)
                .await,
            Err(LambdaServiceError::TooManyRequests)
        ));
        tokio::task::yield_now().await;
        assert!(
            work.functions
                .get("bounded")
                .unwrap()
                .active
                .load(Ordering::Acquire)
                <= 8
        );
        assert!(work.global.available_permits() >= 24);
        assert!(work.quiesce(Duration::ZERO).await.is_err());
        assert_eq!(work.capacity.available_permits(), 128);
        assert_eq!(work.global.available_permits(), 32);
        assert_eq!(
            work.functions
                .get("bounded")
                .unwrap()
                .active
                .load(Ordering::Acquire),
            0
        );
        assert!(
            work.submit(Arc::new(WaitingExecutor), request("bounded"), None, true)
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn test_should_enforce_function_global_and_zero_reserved_execution_limits() {
        let work = WorkManager::new();
        assert!(matches!(
            work.submit(Arc::new(WaitingExecutor), request("zero"), Some(0), true)
                .await,
            Err(LambdaServiceError::TooManyRequests)
        ));
        for name in ["a", "b", "c", "d"] {
            for _ in 0..8 {
                work.submit(Arc::new(WaitingExecutor), request(name), None, false)
                    .await
                    .unwrap();
            }
            assert!(matches!(
                work.submit(Arc::new(WaitingExecutor), request(name), None, false)
                    .await,
                Err(LambdaServiceError::TooManyRequests)
            ));
        }
        assert!(matches!(
            work.submit(Arc::new(WaitingExecutor), request("e"), Some(32), false)
                .await,
            Err(LambdaServiceError::TooManyRequests)
        ));
        assert_eq!(work.global.available_permits(), 0);
        assert!(work.quiesce(Duration::ZERO).await.is_err());
        assert_eq!(work.global.available_permits(), 32);
    }
    #[tokio::test]
    async fn test_should_release_capacity_when_sync_caller_cancels() {
        let work = WorkManager::new();
        let (_response, handle) = work
            .submit(Arc::new(WaitingExecutor), request("cancel"), None, false)
            .await
            .unwrap();
        drop(CancelOnDrop(handle));
        assert!(work.quiesce(Duration::from_secs(1)).await.is_err());
        assert_eq!(work.capacity.available_permits(), 128);
        assert_eq!(work.global.available_permits(), 32);
    }
}
