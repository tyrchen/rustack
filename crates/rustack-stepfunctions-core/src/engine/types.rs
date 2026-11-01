//! Validated execution settings and redacted effect/outcome types.

use std::{
    fmt,
    sync::Arc,
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use rustack_core::json::JsonError;

use crate::{
    AslError, CallbackDispatch, FanoutPlan, Payload, StateId, SystemUuidSource, UuidSource,
};

/// One paired wall/monotonic observation supplied by the execution owner.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct ClockSample {
    pub(crate) monotonic: Instant,
    pub(crate) wall: DateTime<Utc>,
}
impl ClockSample {
    /// Pair clock observations; subsequent samples must not regress monotonically.
    ///
    /// ```
    /// let sample=rustack_stepfunctions_core::ClockSample::new(std::time::Instant::now(),chrono::Utc::now());
    /// ```
    #[must_use]
    pub fn new(monotonic: Instant, wall: DateTime<Utc>) -> Self {
        Self { monotonic, wall }
    }
}

/// Validated local duration and transition ceilings, independent of AWS history.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct ExecutionLimits {
    pub(crate) duration: Duration,
    pub(crate) transitions: u32,
}
impl ExecutionLimits {
    /// Validate positive duration up to365 days and1..=1,000,000 transitions.
    ///
    /// # Errors
    /// Rejects zero, excessive or subsecond-only duration settings.
    ///
    /// ```
    /// let limits=rustack_stepfunctions_core::ExecutionLimits::new(std::time::Duration::from_secs(60),1000)?;
    /// # Ok::<(),rustack_stepfunctions_core::AslError>(())
    /// ```
    pub fn new(duration: Duration, transitions: u32) -> Result<Self, AslError> {
        if duration < Duration::from_secs(1)
            || duration > Duration::from_hours(8760)
            || !(1..=1_000_000).contains(&transitions)
        {
            return Err(AslError::InvalidExecution);
        }
        Ok(Self {
            duration,
            transitions,
        })
    }
}
impl Default for ExecutionLimits {
    fn default() -> Self {
        Self {
            duration: Duration::from_secs(3600),
            transitions: 100_000,
        }
    }
}

/// Opaque attempt identity; meaningful only within its owning execution driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub struct AttemptId(pub(crate) u64);

#[derive(Debug, Clone)]
pub(crate) enum FailureOrigin {
    Business,
    Evaluation(Arc<AslError>),
    Safety,
}

/// Bounded failure fields, preserving omission in a Fail state's terminal record.
#[derive(Clone)]
#[non_exhaustive]
pub struct ExecutionFailure {
    pub(crate) error: Option<String>,
    pub(crate) cause: Option<String>,
    pub(crate) origin: FailureOrigin,
}
impl fmt::Debug for ExecutionFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ExecutionFailure(<redacted>)")
    }
}
impl ExecutionFailure {
    /// Read the supplied error field, including a supplied empty string.
    ///
    /// ```
    /// # fn inspect(f:&rustack_stepfunctions_core::ExecutionFailure) { let error=f.error(); }
    /// ```
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    /// Read the optional bounded cause.
    ///
    /// ```
    /// # fn inspect(f:&rustack_stepfunctions_core::ExecutionFailure) { let cause=f.cause(); }
    /// ```
    #[must_use]
    pub fn cause(&self) -> Option<&str> {
        self.cause.as_deref()
    }
    pub(crate) fn evaluation(error: Arc<AslError>) -> Self {
        let name = match error.as_ref() {
            AslError::DataLimit
            | AslError::Json(JsonError::Bytes | JsonError::String | JsonError::Key) => {
                "States.DataLimitExceeded"
            }
            AslError::Entropy(_)
            | AslError::Encoding(_)
            | AslError::Allocation(_)
            | AslError::InvalidStateId
            | AslError::InvalidFanoutResult
            | AslError::InvalidExecution => "Rustack.InternalError",
            _ => "States.Runtime",
        };
        let mut failure = Self::named(name, "State evaluation failed");
        failure.origin = FailureOrigin::Evaluation(error);
        failure
    }
    pub(crate) fn fields(error: Option<String>, cause: Option<String>) -> Self {
        Self {
            error,
            cause,
            origin: FailureOrigin::Business,
        }
    }
    pub(crate) fn safety(error: &str, cause: &str) -> Self {
        Self {
            error: Some(error.into()),
            cause: Some(cause.into()),
            origin: FailureOrigin::Safety,
        }
    }
    pub(crate) fn named(error: &str, cause: &str) -> Self {
        Self {
            error: Some(error.into()),
            cause: Some(cause.into()),
            origin: FailureOrigin::Business,
        }
    }
}

/// A validated external Task failure, distinct from an omitted Fail field.
#[derive(Clone)]
#[non_exhaustive]
pub struct TaskFailure(pub(crate) ExecutionFailure);
impl fmt::Debug for TaskFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TaskFailure(<redacted>)")
    }
}
impl TaskFailure {
    pub(crate) fn evaluation(error: AslError) -> Self {
        Self(ExecutionFailure::evaluation(Arc::new(error)))
    }
    pub(crate) fn safety(name: &str, cause: &str) -> Self {
        Self(ExecutionFailure::safety(name, cause))
    }

    /// Read bounded failure fields without exposing mutable construction.
    ///
    /// ```
    /// let failure=rustack_stepfunctions_core::TaskFailure::new("BusinessError",None)?;
    /// assert_eq!(failure.details().error(),Some("BusinessError"));
    /// # Ok::<(),rustack_stepfunctions_core::AslError>(())
    /// ```
    #[must_use]
    pub fn details(&self) -> &ExecutionFailure {
        &self.0
    }
    /// Build a bounded standard service failure for task-port admission/ownership.
    ///
    /// ```
    /// let failure=rustack_stepfunctions_core::TaskFailure::service_exception();
    /// ```
    #[must_use]
    pub fn service_exception() -> Self {
        Self(ExecutionFailure::named(
            "Lambda.ServiceException",
            "Lambda task service unavailable",
        ))
    }
    /// Build a bounded standard concurrency rejection without fallible user fields.
    ///
    /// ```
    /// let failure=rustack_stepfunctions_core::TaskFailure::throttled();
    /// ```
    #[must_use]
    pub fn throttled() -> Self {
        Self(ExecutionFailure::named(
            "Lambda.TooManyRequestsException",
            "Lambda task capacity exhausted",
        ))
    }
    /// Validate a nonempty256-byte/control-free name and optional32,768-byte cause.
    ///
    /// # Errors
    /// Rejects invalid names or oversized fields; never truncates caller data.
    ///
    /// ```
    /// let failure=rustack_stepfunctions_core::TaskFailure::new("ExampleError",Some("failed"))?;
    /// # Ok::<(),rustack_stepfunctions_core::AslError>(())
    /// ```
    pub fn new(error: &str, cause: Option<&str>) -> Result<Self, AslError> {
        if error.is_empty()
            || error.len() > 256
            || error.chars().any(char::is_control)
            || cause.is_some_and(|c| c.len() > 32_768)
        {
            return Err(AslError::InvalidFailure);
        }
        Ok(Self(ExecutionFailure {
            error: Some(error.into()),
            cause: cause.map(str::to_owned),
            origin: FailureOrigin::Business,
        }))
    }
}

/// Stable terminal outcome; cloning its Arc does not duplicate payload trees.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ExecutionOutcome {
    /// Successful final transformed payload.
    Succeeded(Payload),
    /// Failed state, resource or interpreter.
    Failed(ExecutionFailure),
    /// Absolute execution deadline; cannot be recovered by a Task Catch.
    TimedOut,
    /// Explicit Stop or parent/shutdown cancellation.
    Aborted(TaskFailure),
}

/// Parameters and deadline for exactly one external Task attempt.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct TaskDispatch {
    pub(crate) attempt: AttemptId,
    pub(crate) state: StateId,
    pub(crate) parameters: Payload,
    pub(crate) deadline: Instant,
}
impl TaskDispatch {
    /// Read the execution-local generation used to reject late results.
    ///
    /// ```
    /// # fn inspect(call:&rustack_stepfunctions_core::TaskDispatch) { let id=call.attempt(); }
    /// ```
    #[must_use]
    pub fn attempt(&self) -> AttemptId {
        self.attempt
    }
    /// Read the state index in the driver's captured machine.
    ///
    /// ```
    /// # fn inspect(call:&rustack_stepfunctions_core::TaskDispatch) { let state=call.state(); }
    /// ```
    #[must_use]
    pub fn state(&self) -> StateId {
        self.state
    }
    /// Read evaluated parameters; aliases remain unresolved until dispatch.
    ///
    /// ```
    /// # fn inspect(call:&rustack_stepfunctions_core::TaskDispatch) { let input=call.parameters(); }
    /// ```
    #[must_use]
    pub fn parameters(&self) -> &Payload {
        &self.parameters
    }
    /// Read the minimum Task/execution monotonic deadline.
    ///
    /// ```
    /// # fn inspect(call:&rustack_stepfunctions_core::TaskDispatch) { let deadline=call.deadline(); }
    /// ```
    #[must_use]
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
}

/// Effect returned by a bounded driver operation; it performs no IO itself.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum DriverStep {
    /// Another pure transition can be driven; yield after at most64 per actor tick.
    Continue,
    /// Start exactly once and retain the accepted call under supervisor ownership.
    Dispatch(TaskDispatch),
    /// Register a token for this identity before materializing invocation/history.
    Callback(CallbackDispatch),
    /// Admit indexed children lazily, then join this generation after cleanup.
    FanOut(FanoutPlan),
    /// Arm a monotonic wakeup (also returned while waiting for a Task result).
    Park(Instant),
    /// An immutable outcome; does not certify resource cleanup or history commit.
    Settled(Arc<ExecutionOutcome>),
}

impl DriverStep {
    pub(crate) fn task_attempt(&self) -> Option<AttemptId> {
        match self {
            Self::Dispatch(dispatch) => Some(dispatch.attempt()),
            Self::Callback(dispatch) => Some(dispatch.attempt()),
            _ => None,
        }
    }
}

/// A transition and its independent cleanup obligation.
///
/// The owner must process cancellation before allowing another dispatch. A
/// terminal outcome never releases ownership of an uncertain external call.
#[must_use = "process the step and retain/cancel any outstanding resource"]
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DriverUpdate {
    pub(crate) step: DriverStep,
    pub(crate) cancel: Option<AttemptId>,
}
impl DriverUpdate {
    /// Read the next transition or external effect.
    ///
    /// ```
    /// # fn inspect(update:&rustack_stepfunctions_core::DriverUpdate) { let step=update.step(); }
    /// ```
    #[must_use]
    pub fn step(&self) -> &DriverStep {
        &self.step
    }
    /// Read the invalidated call or round whose cleanup the owner must supervise.
    ///
    /// ```
    /// # fn inspect(update:&rustack_stepfunctions_core::DriverUpdate) { let call=update.cancellation(); }
    /// ```
    #[must_use]
    pub fn cancellation(&self) -> Option<AttemptId> {
        self.cancel
    }
}

/// Uniform entropy supplied explicitly, keeping potentially blocking OS IO out
/// of an async scheduler. UUID evaluation uses the same retained worker/source.
pub trait ExecutionSource: UuidSource {
    /// Produce an unbiased uniform64-bit word.
    ///
    /// # Errors
    /// Propagates entropy acquisition failure without deterministic fallback.
    ///
    /// ```
    /// # fn sample(source:&mut impl rustack_stepfunctions_core::ExecutionSource) -> Result<(),rustack_stepfunctions_core::AslError> {
    /// let draw=source.random_u64()?;
    /// # Ok(()) }
    /// ```
    fn random_u64(&mut self) -> Result<u64, AslError>;
}
impl ExecutionSource for SystemUuidSource {
    fn random_u64(&mut self) -> Result<u64, AslError> {
        let mut bytes = [0_u8; 8];
        getrandom::fill(&mut bytes).map_err(AslError::Entropy)?;
        Ok(u64::from_le_bytes(bytes))
    }
}
