//! Object-safe app-owned SQS publisher and bounded synchronous publish lifecycle.
//! `async-trait` is required because providers hold `Arc<dyn SqsPublisher>`.
use std::{
    future::{Future, poll_fn},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    task::Poll,
};

use async_trait::async_trait;
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};

/// Explicit cross-service delivery failure.
#[derive(Debug, thiserror::Error)]
pub enum DeliveryError {
    /// A target implementation panicked; subsequent subscribers are still attempted.
    #[error("Target publisher panicked")]
    TargetPanicked,
    /// SQS target rejected a delivery.
    #[error("SQS delivery failed to {queue_arn}: {reason}")]
    SqsDeliveryFailed {
        /// Target ARN.
        queue_arn: String,
        /// Failure reason.
        reason: String,
    },
    /// Required service is unavailable.
    #[error("Delivery service unavailable: {0}")]
    Unavailable(String),
    /// Execution capability is not implemented.
    #[error("Unsupported delivery: {0}")]
    Unsupported(String),
}

/// Application bridge to a real SQS provider.
#[async_trait]
pub trait SqsPublisher: Send + Sync + 'static {
    /// Validate that the ARN can be delivered by this runtime.
    fn validate(&self, queue_arn: &str) -> Result<(), DeliveryError>;
    /// Deliver one message, preserving FIFO identity.
    async fn send_message(
        &self,
        queue_arn: &str,
        message_body: &str,
        message_group_id: Option<&str>,
        message_deduplication_id: Option<&str>,
    ) -> Result<(), DeliveryError>;
}

/// Isolate each target's future without spawning detached work. After a panic
/// the future is never polled again; its error is counted and fanout continues.
pub(crate) async fn send_guarded(
    publisher: &dyn SqsPublisher,
    arn: &str,
    body: &str,
    group: Option<&str>,
    dedup: Option<&str>,
) -> Result<(), DeliveryError> {
    let delivery = async { publisher.send_message(arn, body, group, dedup).await };
    tokio::pin!(delivery);
    poll_fn(|context| {
        catch_unwind(AssertUnwindSafe(|| delivery.as_mut().poll(context)))
            .unwrap_or(Poll::Ready(Err(DeliveryError::TargetPanicked)))
    })
    .await
}

/// Explicit absent SQS dependency; never claims delivery success.
#[derive(Debug)]
pub struct UnavailableSqsPublisher;
#[async_trait]
impl SqsPublisher for UnavailableSqsPublisher {
    fn validate(&self, _: &str) -> Result<(), DeliveryError> {
        Err(DeliveryError::Unavailable("SQS is not enabled".into()))
    }
    async fn send_message(
        &self,
        arn: &str,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<(), DeliveryError> {
        self.validate(arn)
    }
}

/// Cumulative target-attempt outcomes (not topic Publish counts).
#[derive(Debug, Clone, Copy)]
pub struct DeliveryStats {
    /// Accepted target attempts.
    pub accepted: u64,
    /// Delivered target attempts.
    pub delivered: u64,
    /// Failed/cancelled target attempts.
    pub failed: u64,
    /// Publication admission rejections (capacity or shutdown).
    pub rejected: u64,
}

#[derive(Debug)]
pub(crate) struct PublishLifecycle {
    permits: Arc<Semaphore>,
    wake: Arc<Notify>,
    accepted: AtomicU64,
    delivered: AtomicU64,
    failed: AtomicU64,
    rejected: AtomicU64,
}
impl Default for PublishLifecycle {
    fn default() -> Self {
        Self {
            permits: Arc::new(Semaphore::new(128)),
            wake: Arc::new(Notify::new()),
            accepted: AtomicU64::new(0),
            delivered: AtomicU64::new(0),
            failed: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
        }
    }
}
impl PublishLifecycle {
    pub fn is_ready(&self) -> bool {
        !self.permits.is_closed()
    }
    pub fn admit(&self) -> Result<PublishPermit, DeliveryError> {
        let permit = Arc::clone(&self.permits).try_acquire_owned().map_err(|_| {
            self.rejected.fetch_add(1, Ordering::Relaxed);
            DeliveryError::Unavailable("SNS publish is full or quiescing".into())
        })?;
        Ok(PublishPermit {
            permit: Some(permit),
            wake: Arc::clone(&self.wake),
        })
    }
    pub fn attempt(&self) -> Attempt<'_> {
        self.accepted.fetch_add(1, Ordering::Relaxed);
        Attempt {
            lifecycle: self,
            delivered: false,
        }
    }
    pub fn stats(&self) -> DeliveryStats {
        DeliveryStats {
            accepted: self.accepted.load(Ordering::Relaxed),
            delivered: self.delivered.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
            rejected: self.rejected.load(Ordering::Relaxed),
        }
    }
    pub async fn quiesce(&self) {
        self.permits.close();
        loop {
            let notified = self.wake.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.permits.available_permits() == 128 {
                return;
            }
            notified.await;
        }
    }
}

pub(crate) struct PublishPermit {
    permit: Option<OwnedSemaphorePermit>,
    wake: Arc<Notify>,
}
impl Drop for PublishPermit {
    fn drop(&mut self) {
        drop(self.permit.take());
        self.wake.notify_waiters();
    }
}
pub(crate) struct Attempt<'a> {
    lifecycle: &'a PublishLifecycle,
    delivered: bool,
}
impl Attempt<'_> {
    pub fn delivered(&mut self) {
        self.delivered = true;
    }
}
impl Drop for Attempt<'_> {
    fn drop(&mut self) {
        if self.delivered {
            self.lifecycle.delivered.fetch_add(1, Ordering::Relaxed);
        } else {
            self.lifecycle.failed.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn test_should_bound_publish_and_drain_cancelled_permits() {
        let lifecycle = PublishLifecycle::default();
        let permits: Vec<_> = (0..128).map(|_| lifecycle.admit().unwrap()).collect();
        assert!(lifecycle.admit().is_err());
        assert!(
            tokio::time::timeout(Duration::from_millis(1), lifecycle.quiesce())
                .await
                .is_err()
        );
        assert!(lifecycle.admit().is_err());
        drop(permits);
        tokio::time::timeout(Duration::from_secs(1), lifecycle.quiesce())
            .await
            .unwrap();
    }

    #[test]
    fn test_should_record_cancelled_attempt_as_failure() {
        let lifecycle = PublishLifecycle::default();
        {
            let _attempt = lifecycle.attempt();
        }
        {
            let mut attempt = lifecycle.attempt();
            attempt.delivered();
        }
        assert_eq!(lifecycle.stats().accepted, 2);
        assert_eq!(lifecycle.stats().failed, 1);
        assert_eq!(lifecycle.stats().delivered, 1);
    }

    #[tokio::test]
    async fn test_should_never_report_unavailable_publisher_as_success() {
        assert!(
            UnavailableSqsPublisher
                .validate("arn:aws:sqs:us-east-1:000000000000:q")
                .is_err()
        );
        assert!(
            UnavailableSqsPublisher
                .send_message("arn", "message", None, None)
                .await
                .is_err()
        );
    }
}
