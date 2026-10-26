//! Validated runtime limits and immutable control/read handles.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::{
    sync::watch,
    time::{Instant, timeout_at},
};

use super::{super::history::ExecutionHistory, ExecutionError};
use crate::{ExecutionFailure, ExecutionOutcome, TaskFailure};

/// Shared runtime admission ceilings, supplied by validated operator configuration.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct RuntimeLimits {
    pub(super) open: usize,
    pub(super) dispatch: usize,
    pub(super) cpu: usize,
}
impl RuntimeLimits {
    /// Validate open executions1..=4,096, dispatches1..=1,024 and workers1..=128.
    ///
    /// # Errors
    /// Rejects zero or excessive capacities.
    ///
    /// ```
    /// let limits=rustack_stepfunctions_core::RuntimeLimits::new(128,32,8)?;
    /// # Ok::<(),rustack_stepfunctions_core::ExecutionError>(())
    /// ```
    pub fn new(open: usize, dispatch: usize, cpu: usize) -> Result<Self, ExecutionError> {
        if !(1..=4096).contains(&open)
            || !(1..=1024).contains(&dispatch)
            || !(1..=128).contains(&cpu)
        {
            return Err(ExecutionError::Invalid);
        }
        Ok(Self {
            open,
            dispatch,
            cpu,
        })
    }
}
impl Default for RuntimeLimits {
    fn default() -> Self {
        Self {
            open: 128,
            dispatch: 32,
            cpu: 8,
        }
    }
}
/// Cleanup lifetime remains independent of the business execution outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CleanupState {
    /// Actor/calls are still owned and joining.
    Pending,
    /// All execution resources are confirmed settled.
    Clean,
    /// An uncertain call or worker retains its capacity and ownership.
    Quarantined,
}
/// Atomically published history/outcome/cleanup boundary.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ExecutionView {
    pub(super) history: ExecutionHistory,
    pub(super) outcome: Option<Arc<ExecutionOutcome>>,
    pub(super) stopped: Option<DateTime<Utc>>,
    pub(super) cleanup: CleanupState,
    pub(super) diagnostic: Option<Arc<ExecutionError>>,
}
impl ExecutionView {
    /// Read an internal typed source; Debug and public wire errors remain redacted.
    ///
    /// ```
    /// # fn inspect(view:&rustack_stepfunctions_core::ExecutionView) { let source=view.diagnostic(); }
    /// ```
    #[must_use]
    pub fn diagnostic(&self) -> Option<&ExecutionError> {
        self.diagnostic.as_deref()
    }
    /// Read a stable history boundary that already includes any terminal outcome.
    ///
    /// ```
    /// # fn inspect(view:&rustack_stepfunctions_core::ExecutionView) { let history=view.history(); }
    /// ```
    #[must_use]
    pub fn history(&self) -> &ExecutionHistory {
        &self.history
    }
    /// Read the single settled outcome, if present.
    ///
    /// ```
    /// # fn inspect(view:&rustack_stepfunctions_core::ExecutionView) { let outcome=view.outcome(); }
    /// ```
    #[must_use]
    pub fn outcome(&self) -> Option<&Arc<ExecutionOutcome>> {
        self.outcome.as_ref()
    }
    /// Read committed termination time; subsequent cleanup cannot rewrite it.
    ///
    /// ```
    /// # fn inspect(view:&rustack_stepfunctions_core::ExecutionView) { let stopped=view.stop_date(); }
    /// ```
    #[must_use]
    pub fn stop_date(&self) -> Option<DateTime<Utc>> {
        self.stopped
    }
    /// Inspect cleanup independently of execution success/failure.
    ///
    /// ```
    /// # fn inspect(view:&rustack_stepfunctions_core::ExecutionView) { let cleanup=view.cleanup(); }
    /// ```
    #[must_use]
    pub fn cleanup(&self) -> CleanupState {
        self.cleanup
    }
}
/// Cloneable observer/control handle; dropping it never cancels accepted work.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ExecutionHandle {
    pub(super) stop: watch::Sender<Option<TaskFailure>>,
    pub(super) view: watch::Receiver<Arc<ExecutionView>>,
}
impl ExecutionHandle {
    /// Capture one coherent history/outcome/read boundary.
    ///
    /// ```
    /// # fn inspect(handle:&rustack_stepfunctions_core::ExecutionHandle) { let view=handle.snapshot(); }
    /// ```
    #[must_use]
    pub fn snapshot(&self) -> Arc<ExecutionView> {
        self.view.borrow().clone()
    }
    /// Request Stop once on its independent lane; first reason wins.
    ///
    /// ```
    /// # fn stop(handle:&rustack_stepfunctions_core::ExecutionHandle)->Result<(),rustack_stepfunctions_core::AslError> {
    /// handle.request_stop(rustack_stepfunctions_core::TaskFailure::new("Stopped",None)?);
    /// # Ok(()) }
    /// ```
    pub fn request_stop(&self, reason: TaskFailure) {
        if self.snapshot().outcome.is_some() {
            return;
        }
        let mut reason = Some(reason);
        self.stop.send_if_modified(|pending| {
            if pending.is_some() {
                false
            } else {
                *pending = reason.take();
                true
            }
        });
    }
    /// Await committed termination within an observer deadline, preserving ownership.
    ///
    /// # Errors
    /// Returns Deadline or Unavailable; never cancels or restarts the execution.
    ///
    /// ```no_run
    /// # async fn join(h:&mut rustack_stepfunctions_core::ExecutionHandle) {
    /// let view=h.wait(tokio::time::Instant::now()+std::time::Duration::from_secs(10)).await;
    /// # }
    /// ```
    pub async fn wait(&mut self, deadline: Instant) -> Result<Arc<ExecutionView>, ExecutionError> {
        if Instant::now() >= deadline {
            return Err(ExecutionError::Deadline);
        }
        let wait = async {
            loop {
                let view = self.snapshot();
                if view.outcome.is_some() {
                    return Ok::<Arc<ExecutionView>, ExecutionError>(view);
                }
                self.view
                    .changed()
                    .await
                    .map_err(|_| ExecutionError::Unavailable)?;
            }
        };
        let view = timeout_at(deadline, wait)
            .await
            .map_err(|_| ExecutionError::Deadline)??;
        if Instant::now() >= deadline {
            return Err(ExecutionError::Deadline);
        }
        Ok(view)
    }
    /// Request Stop and await its single committed terminal result.
    ///
    /// # Errors
    /// Returns observation deadline/unavailability without undoing the request.
    ///
    /// ```no_run
    /// # async fn stop(h:&mut rustack_stepfunctions_core::ExecutionHandle,r:rustack_stepfunctions_core::TaskFailure,d:tokio::time::Instant) {
    /// let view=h.stop(r,d).await;
    /// # }
    /// ```
    pub async fn stop(
        &mut self,
        reason: TaskFailure,
        deadline: Instant,
    ) -> Result<Arc<ExecutionView>, ExecutionError> {
        self.request_stop(reason);
        self.wait(deadline).await
    }
}
pub(super) fn internal_outcome() -> Arc<ExecutionOutcome> {
    Arc::new(ExecutionOutcome::Failed(ExecutionFailure::named(
        "Rustack.InternalError",
        "Execution owner failed",
    )))
}
