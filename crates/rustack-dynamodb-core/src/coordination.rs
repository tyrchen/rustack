//! Cancellation-safe admission for synchronous provider tasks.
//!
//! The semaphore contains no data state. Its permit is moved into the blocking
//! closure so HTTP cancellation cannot make a running operation disappear from
//! the shutdown barrier.
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::panicking,
};

use parking_lot::MutexGuard;
use rustack_dynamodb_model::error::{DynamoDBError, DynamoDBErrorCode};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};

const REQUEST_CAPACITY: usize = 128;

#[derive(Debug)]
pub(crate) struct RequestTracker {
    permits: Arc<Semaphore>,
    wake: Arc<Notify>,
    failed: AtomicBool,
}
impl Default for RequestTracker {
    fn default() -> Self {
        Self {
            permits: Arc::new(Semaphore::new(REQUEST_CAPACITY)),
            wake: Arc::new(Notify::new()),
            failed: AtomicBool::new(false),
        }
    }
}
impl RequestTracker {
    pub fn is_ready(&self) -> bool {
        !self.permits.is_closed()
    }
    pub fn ensure_healthy(&self) -> Result<(), DynamoDBError> {
        if self.failed.load(Ordering::Acquire) {
            Err(DynamoDBError::internal_error("DynamoDB operation panicked"))
        } else {
            Ok(())
        }
    }
    pub fn guard<'a>(
        &'a self,
        guard: MutexGuard<'a, ()>,
    ) -> Result<OperationGuard<'a>, DynamoDBError> {
        self.ensure_open()?;
        Ok(OperationGuard {
            _guard: guard,
            tracker: self,
        })
    }
    pub fn admit(&self) -> Result<RequestPermit, DynamoDBError> {
        let permit = Arc::clone(&self.permits)
            .try_acquire_owned()
            .map_err(|_| Self::unavailable())?;
        Ok(RequestPermit {
            permit: Some(permit),
            wake: Arc::clone(&self.wake),
        })
    }
    pub fn ensure_open(&self) -> Result<(), DynamoDBError> {
        if self.permits.is_closed() {
            Err(Self::unavailable())
        } else {
            Ok(())
        }
    }
    fn unavailable() -> DynamoDBError {
        DynamoDBError::with_message(
            DynamoDBErrorCode::RequestLimitExceeded,
            "DynamoDB is full or quiescing",
        )
    }
    pub async fn quiesce(&self) {
        self.permits.close();
        loop {
            let notified = self.wake.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.permits.available_permits() == REQUEST_CAPACITY {
                return;
            }
            notified.await;
        }
    }
}

pub(crate) struct OperationGuard<'a> {
    _guard: MutexGuard<'a, ()>,
    tracker: &'a RequestTracker,
}
impl Drop for OperationGuard<'_> {
    fn drop(&mut self) {
        if panicking() {
            self.tracker.failed.store(true, Ordering::Release);
            self.tracker.permits.close();
        }
    }
}

pub(crate) struct RequestPermit {
    permit: Option<OwnedSemaphorePermit>,
    wake: Arc<Notify>,
}
impl Drop for RequestPermit {
    fn drop(&mut self) {
        drop(self.permit.take());
        self.wake.notify_waiters();
    }
}
