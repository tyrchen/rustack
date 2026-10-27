//! Validated callback identity, settings, public command and read boundaries.

use std::{fmt, sync::Arc, time::Duration};

use thiserror::Error;
use tokio::{task::JoinError, time::Instant};

use super::CallbackToken;
use crate::{
    AslError, ExecutionSnapshot, FrameCallId, Payload, StopReason, TaskFailure, TenantScope,
    compiler::TASK_SECONDS_LIMIT, registry::domain,
};

/// Static, redacted callback lifecycle/admission errors with retained sources.
#[derive(Error)]
#[non_exhaustive]
pub enum CallbackError {
    /// Unknown, malformed or foreign-scope credential.
    #[error("invalid callback token")]
    InvalidToken,
    /// A known token has expired, been consumed or been revoked.
    #[error("callback task is closed or timed out")]
    TaskTimedOut,
    /// Invalid trusted limits, identity or heartbeat interval.
    #[error("invalid callback configuration or attempt")]
    Validation,
    /// This captured execution/frame/generation already minted a token.
    #[error("callback attempt already registered")]
    AttemptExists,
    /// No bounded ownership/queue reservation is available.
    #[error("callback capacity exhausted")]
    Capacity,
    /// No accepting root owner is available.
    #[error("callback owner unavailable")]
    Unavailable,
    /// An observer expired; accepted work remains owned.
    #[error("callback observation deadline exceeded")]
    Deadline,
    /// A reply owner disappeared without proving its result.
    #[error("callback reply owner lost")]
    OwnerLost,
    /// Unique Task owner attempted to report invocation completion twice.
    #[error("invalid callback invocation progress")]
    Progress,
    /// Invalid/oversized output JSON or bounded callback data.
    #[error("invalid callback output or failure data")]
    Data(
        #[from]
        #[source]
        AslError,
    ),
    /// Operating system entropy failed; no token was published.
    #[error("callback entropy failed")]
    Entropy(#[source] getrandom::Error),
    /// A retained worker panicked or was unexpectedly cancelled.
    #[error("callback worker failed")]
    Worker(#[source] JoinError),
    /// Retained cached admission or shutdown failure.
    #[error("callback owner operation failed")]
    Cached(#[source] Arc<CallbackError>),
}
impl fmt::Debug for CallbackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
/// Validated actor capacities and execution-relative classification retention.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct CallbackLimits {
    pub(super) live: usize,
    pub(super) retained: usize,
    pub(super) commands: usize,
    pub(super) workers: usize,
    pub(super) retention: Duration,
}
impl CallbackLimits {
    /// Validate live1..65,536, retained live..1,000,000, commands1..100,000,
    /// workers1..128 and retention1s..90d.
    ///
    /// # Errors
    /// Rejects zero, excessive or inconsistent limits.
    ///
    /// ```
    /// let limits=rustack_stepfunctions_core::CallbackLimits::new(4096,100000,1024,8,std::time::Duration::from_secs(86400))?;
    /// # Ok::<(),rustack_stepfunctions_core::CallbackError>(())
    /// ```
    pub fn new(
        live: usize,
        retained: usize,
        commands: usize,
        workers: usize,
        retention: Duration,
    ) -> Result<Self, CallbackError> {
        if !(1..=65_536).contains(&live)
            || !(live..=1_000_000).contains(&retained)
            || !(1..=100_000).contains(&commands)
            || !(1..=128).contains(&workers)
            || !(Duration::from_secs(1)..=Duration::from_hours(2160)).contains(&retention)
        {
            return Err(CallbackError::Validation);
        }
        Ok(Self {
            live,
            retained,
            commands,
            workers,
            retention,
        })
    }
}
impl Default for CallbackLimits {
    fn default() -> Self {
        Self {
            live: 4096,
            retained: 100_000,
            commands: 1024,
            workers: 8,
            retention: Duration::from_hours(24),
        }
    }
}
/// Captured trusted parent/frame/generation and absolute Task timing.
#[non_exhaustive]
pub struct CallbackAttempt {
    pub(super) parent: ExecutionSnapshot,
    pub(super) call: FrameCallId,
    pub(super) deadline: Instant,
    pub(super) heartbeat: Option<Duration>,
}
impl fmt::Debug for CallbackAttempt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CallbackAttempt(<redacted>)")
    }
}
impl CallbackAttempt {
    /// Capture an exact registered execution and opaque current frame attempt.
    ///
    /// The consuming execution owner must supply its own current call identity.
    ///
    /// # Errors
    /// Rejects closed parents, expired deadlines and nonpositive/nonintegral
    /// heartbeat seconds or intervals exceeding99,999,999 seconds.
    ///
    /// ```no_run
    /// # fn register(parent:&rustack_stepfunctions_core::ExecutionSnapshot,call:rustack_stepfunctions_core::FrameCallId,deadline:tokio::time::Instant){
    /// let attempt=rustack_stepfunctions_core::CallbackAttempt::new(parent,call,deadline,None);
    /// # }
    /// ```
    pub fn new(
        parent: &ExecutionSnapshot,
        call: FrameCallId,
        deadline: Instant,
        heartbeat: Option<Duration>,
    ) -> Result<Self, CallbackError> {
        if heartbeat.is_some_and(|value| {
            value.subsec_nanos() != 0
                || !(1..=u64::from(TASK_SECONDS_LIMIT)).contains(&value.as_secs())
        }) {
            return Err(CallbackError::Validation);
        }
        let result = Self {
            parent: parent.clone(),
            call,
            deadline: deadline.min(Instant::from_std(parent.record.deadline)),
            heartbeat,
        };
        result.live(Instant::now())?;
        Ok(result)
    }
    pub(super) fn live(&self, now: Instant) -> Result<(), CallbackError> {
        if now >= self.deadline || self.parent.handle().snapshot().outcome().is_some() {
            Err(CallbackError::TaskTimedOut)
        } else {
            Ok(())
        }
    }
    pub(super) fn scoped(&self, scope: &TenantScope) -> bool {
        domain::same_scope(self.parent.revision().machine().scope(), scope)
    }
}
/// Distinct expiry reasons for ASL Task and heartbeat error mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CallbackTimeout {
    /// Absolute Task deadline, including synchronous invocation.
    Task,
    /// Heartbeat window after successful invocation.
    Heartbeat,
}
/// Coherent callback business boundary; observing it never confirms Lambda cleanup.
#[derive(Clone)]
#[non_exhaustive]
pub enum CallbackOutcome {
    /// The synchronous invocation has not completed successfully yet.
    Invoking,
    /// Awaiting a callback; no heartbeat timer is represented by None.
    Waiting {
        /// Current bounded heartbeat deadline.
        heartbeat_deadline: Option<Instant>,
    },
    /// Exactly one external success committed after successful invocation.
    Succeeded(Payload),
    /// External failure committed after successful invocation.
    Failed(TaskFailure),
    /// Synchronous invocation failed; any provisional callback was discarded.
    InvocationFailed(TaskFailure),
    /// Absolute/heartbeat deadline expired.
    TimedOut(CallbackTimeout),
    /// Unique Task owner or parent revoked the attempt.
    Revoked,
    /// Registry root disappeared or closed before successful completion.
    Unavailable,
}
impl fmt::Debug for CallbackOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invoking => "CallbackOutcome::Invoking",
            Self::Waiting { .. } => "CallbackOutcome::Waiting",
            Self::Succeeded(_) => "CallbackOutcome::Succeeded(<redacted>)",
            Self::Failed(_) => "CallbackOutcome::Failed(<redacted>)",
            Self::InvocationFailed(_) => "CallbackOutcome::InvocationFailed(<redacted>)",
            Self::TimedOut(_) => "CallbackOutcome::TimedOut",
            Self::Revoked => "CallbackOutcome::Revoked",
            Self::Unavailable => "CallbackOutcome::Unavailable",
        })
    }
}
impl CallbackOutcome {
    /// Whether this immutable observation has a terminal business decision.
    ///
    /// ```
    /// assert!(rustack_stepfunctions_core::CallbackOutcome::Revoked.is_terminal());
    /// ```
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Invoking | Self::Waiting { .. })
    }
}
#[derive(Debug)]
pub(super) enum Action {
    Complete(Result<Payload, TaskFailure>),
    Heartbeat,
}
/// Fully prepared callback mutation, with nonserializable credential ownership.
#[non_exhaustive]
pub struct CallbackCommand {
    pub(super) token: CallbackToken,
    pub(super) action: Action,
}
impl fmt::Debug for CallbackCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CallbackCommand(<redacted>)")
    }
}
impl CallbackCommand {
    /// Parse JSON output without double encoding; use bounded CPU admission.
    ///
    /// # Errors
    /// Rejects invalid JSON or payload limits.
    ///
    /// ```
    /// # fn prepare(token:rustack_stepfunctions_core::CallbackToken)->Result<(),rustack_stepfunctions_core::CallbackError>{
    /// let command=rustack_stepfunctions_core::CallbackCommand::success(token,b"{\"answer\":42}")?;
    /// # Ok(()) }
    /// ```
    pub fn success(token: CallbackToken, output: &[u8]) -> Result<Self, CallbackError> {
        Ok(Self {
            token,
            action: Action::Complete(Ok(Payload::parse(output)?)),
        })
    }
    /// Preserve omitted/empty failure fields after validating byte ceilings.
    ///
    /// # Errors
    /// Rejects error above256 bytes or cause above32,768 bytes.
    ///
    /// ```
    /// # fn prepare(token:rustack_stepfunctions_core::CallbackToken)->Result<(),rustack_stepfunctions_core::CallbackError>{
    /// let command=rustack_stepfunctions_core::CallbackCommand::failure(token,None,Some(String::new()))?;
    /// # Ok(()) }
    /// ```
    pub fn failure(
        token: CallbackToken,
        error: Option<String>,
        cause: Option<String>,
    ) -> Result<Self, CallbackError> {
        let failure = StopReason::new(error, cause)
            .map_err(|_| CallbackError::Validation)?
            .0;
        Ok(Self {
            token,
            action: Action::Complete(Err(failure)),
        })
    }
    /// Prepare a heartbeat; the registry enforces timing/closed classification.
    ///
    /// ```
    /// # fn prepare(token:rustack_stepfunctions_core::CallbackToken){let command=rustack_stepfunctions_core::CallbackCommand::heartbeat(token);}
    /// ```
    #[must_use]
    pub fn heartbeat(token: CallbackToken) -> Self {
        Self {
            token,
            action: Action::Heartbeat,
        }
    }
}
/// Public actor health/counts; no credential, payload or execution identity.
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct CallbackStatus {
    /// Root accepts commands and has no known fatal failure.
    pub ready: bool,
    /// Issued, unsettled callback attempts.
    pub live: usize,
    /// Active plus closed classification reservations in the table.
    pub retained: usize,
}
