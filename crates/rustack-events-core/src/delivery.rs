//! Typed target delivery and bounded, drainable dispatch.
//!
//! `async-trait` is required for object-safe `Arc<dyn TargetDelivery>` bridges.
use std::{
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
pub use rustack_events_model::types::Target;
use tokio::sync::{mpsc, watch};

/// Explicit target delivery failure.
#[derive(Debug, thiserror::Error)]
pub enum DeliveryError {
    /// Invalid resource identifier.
    #[error("Invalid target ARN: {0}")]
    InvalidArn(String),
    /// Unsupported execution capability or parameter.
    #[error("Unsupported target delivery: {0}")]
    Unsupported(String),
    /// Required runtime service is disabled or unavailable.
    #[error("Target service unavailable: {0}")]
    Unavailable(String),
    /// Target rejected the delivery.
    #[error("Target delivery failed: {0}")]
    TargetError(String),
}

/// Bridge implemented in the application, never by depending on another core.
#[async_trait]
pub trait TargetDelivery: Send + Sync + std::fmt::Debug + 'static {
    /// Validate configuration against the bridge's actual capabilities.
    fn validate(&self, target: &Target) -> Result<(), DeliveryError>;
    /// Deliver the complete target configuration and transformed JSON body.
    async fn deliver(&self, target: &Target, event_json: &str) -> Result<(), DeliveryError>;
}

/// Explicit unavailable dependency, suitable for metadata-only runtime wiring.
#[derive(Debug)]
pub struct UnavailableTargetDelivery;
#[async_trait]
impl TargetDelivery for UnavailableTargetDelivery {
    fn validate(&self, _: &Target) -> Result<(), DeliveryError> {
        Err(DeliveryError::Unavailable("SQS is not enabled".into()))
    }
    async fn deliver(&self, target: &Target, _: &str) -> Result<(), DeliveryError> {
        self.validate(target)
    }
}

/// Cumulative dispatch counters. Accepted counts target attempts, not API calls.
#[derive(Debug, Clone, Copy, Default)]
pub struct DeliveryStats {
    /// Accepted target attempts.
    pub accepted: u64,
    /// Successfully delivered target attempts.
    pub delivered: u64,
    /// Failed attempts, including timeout and panic.
    pub failed: u64,
    /// Rejected event batches (capacity, lifecycle or payload budget).
    pub rejected: u64,
}

#[derive(Debug, Default)]
struct Counters {
    accepted: AtomicU64,
    delivered: AtomicU64,
    failed: AtomicU64,
    rejected: AtomicU64,
}

#[derive(Debug)]
pub(crate) struct DeliveryJob {
    pub target: Target,
    pub body: String,
}

#[cfg(test)]
#[path = "delivery_tests.rs"]
mod tests;

enum Command {
    #[cfg(test)]
    CrashWorker,
    Deliver(Vec<DeliveryJob>),
    Quiesce,
}
struct Worker {
    sender: mpsc::Sender<Command>,
    stopped: watch::Receiver<Option<bool>>,
}

pub(crate) struct DeliveryQueue {
    bridge: Arc<dyn TargetDelivery>,
    worker: OnceLock<Worker>,
    closing: Arc<AtomicBool>,
    counters: Arc<Counters>,
}

impl DeliveryQueue {
    pub fn new(bridge: Arc<dyn TargetDelivery>) -> Self {
        Self {
            bridge,
            worker: OnceLock::new(),
            closing: Arc::new(AtomicBool::new(false)),
            counters: Arc::new(Counters::default()),
        }
    }

    pub fn submit(&self, jobs: Vec<DeliveryJob>) -> Result<(), DeliveryError> {
        if jobs.len() > 128 || jobs.iter().map(|job| job.body.len()).sum::<usize>() > 1024 * 1024 {
            self.counters.rejected.fetch_add(1, Ordering::Relaxed);
            return Err(DeliveryError::TargetError(
                "Event fanout exceeds 128 targets or 1 MiB".into(),
            ));
        }
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| DeliveryError::Unavailable("Async runtime is not running".into()))?;
        let worker = self.worker.get_or_init(|| {
            let (sender, receiver) = mpsc::channel(128);
            let (finished, stopped) = watch::channel(None);
            let bridge = Arc::clone(&self.bridge);
            let counters = Arc::clone(&self.counters);
            let closing = Arc::clone(&self.closing);
            // One supervisor per provider; it observes the worker's panic/result.
            runtime.spawn(async move {
                let result = tokio::spawn(run_worker(receiver, bridge, counters, closing)).await;
                let _ = finished.send(Some(result.is_ok()));
            });
            Worker { sender, stopped }
        });
        let count = u64::try_from(jobs.len()).unwrap_or(u64::MAX);
        if self.closing.load(Ordering::Acquire)
            || worker.sender.try_send(Command::Deliver(jobs)).is_err()
        {
            self.counters.rejected.fetch_add(1, Ordering::Relaxed);
            return Err(DeliveryError::Unavailable(
                "Delivery queue is full or quiescing".into(),
            ));
        }
        self.counters.accepted.fetch_add(count, Ordering::Relaxed);
        Ok(())
    }

    pub fn is_ready(&self) -> bool {
        !self.closing.load(Ordering::Acquire)
            && self.worker.get().is_none_or(|worker| {
                !worker.sender.is_closed()
                    && worker.stopped.borrow().is_none()
                    && worker.stopped.has_changed().is_ok()
            })
    }

    /// Test-only supervision fault injection: crash the worker and observe failure.
    #[cfg(test)]
    pub fn inject_crash(&self) {
        if let Some(worker) = self.worker.get() {
            let _ = worker.sender.try_send(Command::CrashWorker);
        }
    }

    pub fn stats(&self) -> DeliveryStats {
        DeliveryStats {
            accepted: self.counters.accepted.load(Ordering::Relaxed),
            delivered: self.counters.delivered.load(Ordering::Relaxed),
            failed: self.counters.failed.load(Ordering::Relaxed),
            rejected: self.counters.rejected.load(Ordering::Relaxed),
        }
    }

    pub async fn quiesce(&self) -> Result<(), DeliveryError> {
        self.closing.store(true, Ordering::Release);
        let Some(worker) = self.worker.get() else {
            return Ok(());
        };
        // A full channel already wakes the worker, which checks closing before each receive.
        let _ = worker.sender.try_send(Command::Quiesce);
        let mut stopped = worker.stopped.clone();
        loop {
            if let Some(success) = *stopped.borrow_and_update() {
                return if success {
                    Ok(())
                } else {
                    Err(DeliveryError::TargetError(
                        "Delivery worker panicked".into(),
                    ))
                };
            }
            stopped
                .changed()
                .await
                .map_err(|_| DeliveryError::Unavailable("Delivery supervisor stopped".into()))?;
        }
    }
}

async fn run_worker(
    mut receiver: mpsc::Receiver<Command>,
    bridge: Arc<dyn TargetDelivery>,
    counters: Arc<Counters>,
    closing: Arc<AtomicBool>,
) {
    loop {
        if closing.load(Ordering::Acquire) {
            receiver.close();
        }
        let Some(command) = receiver.recv().await else {
            break;
        };
        match command {
            #[cfg(test)]
            Command::CrashWorker => panic!("injected worker panic"),
            Command::Quiesce => receiver.close(),
            Command::Deliver(jobs) => {
                for job in jobs {
                    let bridge = Arc::clone(&bridge);
                    let mut task =
                        tokio::spawn(async move { bridge.deliver(&job.target, &job.body).await });
                    let success =
                        match tokio::time::timeout(Duration::from_secs(5), &mut task).await {
                            Ok(Ok(Ok(()))) => true,
                            Ok(Ok(Err(error))) => {
                                tracing::warn!(error = %error, "Event target delivery failed");
                                false
                            }
                            Ok(Err(error)) => {
                                tracing::error!(error = %error, "Event target task failed");
                                false
                            }
                            Err(_) => {
                                task.abort();
                                let _ = task.await;
                                tracing::warn!("Event target delivery timed out");
                                false
                            }
                        };
                    if success {
                        counters.delivered.fetch_add(1, Ordering::Relaxed);
                    } else {
                        counters.failed.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
    }
}
