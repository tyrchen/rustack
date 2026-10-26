//! Global/local frame and aggregate-slot admission; permit drops signal capacity.

use std::sync::Arc;

use tokio::{
    sync::{Notify, OwnedSemaphorePermit, Semaphore},
    time::{Instant, timeout_at},
};

use super::types::FrameError;

/// Validated local frame and aggregate-result slot ceilings.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct FrameLimits {
    pub(super) frames: usize,
    pub(super) results: usize,
}
impl FrameLimits {
    /// Validate1..512 frames and4KiB..128MiB aggregate-slot resource bytes.
    ///
    /// # Errors
    /// Returns Invalid for out-of-range capacities.
    ///
    /// ```
    /// let limits=rustack_stepfunctions_core::FrameLimits::new(512,32*1024*1024)?;
    /// # Ok::<(),rustack_stepfunctions_core::FrameError>(())
    /// ```
    pub fn new(frames: usize, results: usize) -> Result<Self, FrameError> {
        if !(1..=512).contains(&frames) || !(4096..=128 * 1024 * 1024).contains(&results) {
            return Err(FrameError::Invalid);
        }
        Ok(Self { frames, results })
    }
}
impl Default for FrameLimits {
    fn default() -> Self {
        Self {
            frames: 512,
            results: 32 * 1024 * 1024,
        }
    }
}

/// Shared global frame and intermediate aggregate-slot budgets for all executions.
#[derive(Debug)]
#[non_exhaustive]
pub struct FrameCapacity {
    pub(super) frames: Arc<Semaphore>,
    pub(super) results: Arc<Semaphore>,
    changed: Notify,
}
impl FrameCapacity {
    /// Validate1..4,096 frames and4KiB..1GiB global aggregate-slot bytes.
    ///
    /// # Errors
    /// Returns Invalid for out-of-range values.
    ///
    /// ```
    /// let capacity=rustack_stepfunctions_core::FrameCapacity::new(4096,256*1024*1024)?;
    /// # Ok::<(),rustack_stepfunctions_core::FrameError>(())
    /// ```
    pub fn new(frames: usize, results: usize) -> Result<Self, FrameError> {
        if !(1..=4096).contains(&frames)
            || !(4096..=1024 * 1024 * 1024).contains(&results)
            || results > Semaphore::MAX_PERMITS
        {
            return Err(FrameError::Invalid);
        }
        Ok(Self {
            frames: Arc::new(Semaphore::new(frames)),
            results: Arc::new(Semaphore::new(results)),
            changed: Notify::new(),
        })
    }
    /// Read currently unreserved frame slots; callers must still try admission.
    ///
    /// ```
    /// let capacity=rustack_stepfunctions_core::FrameCapacity::default();
    /// assert_eq!(capacity.available_frames(),4096);
    /// ```
    #[must_use]
    pub fn available_frames(&self) -> usize {
        self.frames.available_permits()
    }

    /// Wait for a potentially available global slot without reserving it.
    ///
    /// # Errors
    /// Returns Deadline on observer expiry; no execution or resource is cancelled.
    ///
    /// ```no_run
    /// # async fn wait(capacity:&rustack_stepfunctions_core::FrameCapacity) {
    /// let result=capacity.wait_for_capacity(tokio::time::Instant::now()+std::time::Duration::from_secs(1)).await;
    /// # }
    /// ```
    pub async fn wait_for_capacity(&self, deadline: Instant) -> Result<(), FrameError> {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if Instant::now() >= deadline {
                return Err(FrameError::Deadline);
            }
            if self.available_frames() > 0 {
                return Ok(());
            }
            timeout_at(deadline, changed)
                .await
                .map_err(|_| FrameError::Deadline)?;
        }
    }
}
impl Default for FrameCapacity {
    fn default() -> Self {
        Self {
            frames: Arc::new(Semaphore::new(4096)),
            results: Arc::new(Semaphore::new(256 * 1024 * 1024)),
            changed: Notify::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Blocked {
    Local,
    Global,
}
#[derive(Debug)]
pub(super) struct Slot {
    global: Option<OwnedSemaphorePermit>,
    local: Option<OwnedSemaphorePermit>,
    capacity: Arc<FrameCapacity>,
}
impl Slot {
    pub(super) fn acquire(
        capacity: &Arc<FrameCapacity>,
        local: &Arc<Semaphore>,
    ) -> Result<Self, Blocked> {
        let local = local
            .clone()
            .try_acquire_owned()
            .map_err(|_| Blocked::Local)?;
        let global = capacity
            .frames
            .clone()
            .try_acquire_owned()
            .map_err(|_| Blocked::Global)?;
        Ok(Self {
            global: Some(global),
            local: Some(local),
            capacity: capacity.clone(),
        })
    }
}
impl Drop for Slot {
    fn drop(&mut self) {
        drop(self.global.take());
        drop(self.local.take());
        self.capacity.changed.notify_waiters();
    }
}

/// An immutable charged frame slot retaining its complete ancestor lineage.
#[derive(Debug)]
#[non_exhaustive]
pub struct FrameLease {
    _slot: Slot,
    _parent: Option<Arc<FrameLease>>,
}
impl FrameLease {
    pub(super) fn child(slot: Slot, parent: Option<Arc<Self>>) -> Arc<Self> {
        Arc::new(Self {
            _slot: slot,
            _parent: parent,
        })
    }
    pub(crate) fn root(
        capacity: &Arc<FrameCapacity>,
        limits: FrameLimits,
    ) -> Result<Arc<Self>, FrameError> {
        let local = Arc::new(Semaphore::new(limits.frames));
        Slot::acquire(capacity, &local)
            .map(|slot| Self::child(slot, None))
            .map_err(|_| FrameError::Capacity)
    }
}
