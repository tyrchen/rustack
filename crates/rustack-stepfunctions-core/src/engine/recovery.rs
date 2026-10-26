//! First-matching policy selection, bounded backoff and unbiased jitter.

use std::time::{Duration, Instant};

use serde_json::{Map, Value};

use super::{
    ClockSample, Continuation, DriverStep, ExecutionDriver, ExecutionFailure, ExecutionOutcome,
    ExecutionSource, Phase,
};
use crate::{AslError, CatchResult, Jitter, Payload, RetryTiming, StateId, StateView, Transition};

enum Recovery {
    Retry { index: usize, until: Instant },
    Catch { output: Payload, next: StateId },
    Fail,
    Limit,
}
impl ExecutionDriver {
    pub(super) fn recover<S: ExecutionSource + ?Sized>(
        &mut self,
        failure: ExecutionFailure,
        now: ClockSample,
        source: &mut S,
    ) -> DriverStep {
        match self.recovery_plan(&failure, now, source) {
            Ok(Recovery::Retry { index, until }) => {
                let Some(count) = self.retries.get_mut(index) else {
                    return self.evaluation_failure(AslError::InvalidStateId, now, source, false);
                };
                *count = count.saturating_add(1);
                self.retry_count = self.retry_count.saturating_add(1);
                self.phase = Phase::Waiting {
                    until,
                    continuation: Continuation::Retry,
                };
                DriverStep::Park(until)
            }
            Ok(Recovery::Catch { output, next }) => self.transfer(output, Transition::Next(next)),
            Ok(Recovery::Limit) => self.transition_limit(),
            Ok(Recovery::Fail) => self.settle(ExecutionOutcome::Failed(failure)),
            Err(
                error @ (AslError::Merge
                | AslError::DataLimit
                | AslError::Json(_)
                | AslError::Encoding(_)),
            ) => {
                // A failing catcher merge is an interpreter failure, never a
                // second pass through this same recovery policy.
                self.evaluation_failure(error, now, source, false)
            }
            Err(error) => {
                // Recovery entropy/invariant failures are internal, not user errors.
                self.diagnostic = Some(error);
                self.settle(ExecutionOutcome::Failed(ExecutionFailure::named(
                    "Rustack.InternalError",
                    "Task recovery failed",
                )))
            }
        }
    }
    fn recovery_plan<S: ExecutionSource + ?Sized>(
        &mut self,
        failure: &ExecutionFailure,
        now: ClockSample,
        source: &mut S,
    ) -> Result<Recovery, AslError> {
        let machine = self.machine.clone();
        let StateView::Task { recovery, .. } = machine.state(self.state)? else {
            return Err(AslError::InvalidStateId);
        };
        self.retries.resize(recovery.retries().len(), 0);
        let name = failure.error().ok_or(AslError::InvalidFailure)?;
        for (index, retrier) in recovery.retries().iter().enumerate() {
            if !retrier.matches(name) {
                continue;
            }
            let count = *self.retries.get(index).ok_or(AslError::InvalidStateId)?;
            if count < retrier.max_attempts() {
                if self.remaining == 0 {
                    return Ok(Recovery::Limit);
                }
                let remaining = self.deadline.saturating_duration_since(now.monotonic);
                let delay = backoff(retrier.timing(), count, remaining, source)?;
                return Ok(Recovery::Retry {
                    index,
                    until: now
                        .monotonic
                        .checked_add(delay)
                        .ok_or(AslError::InvalidExecution)?,
                });
            }
            break;
        }
        for catcher in recovery.catches() {
            if !catcher.matches(name) {
                continue;
            }
            let mut object = Map::new();
            if let Some(error) = failure.error() {
                object.insert("Error".into(), Value::String(error.into()));
            }
            if let Some(cause) = failure.cause() {
                object.insert("Cause".into(), Value::String(cause.into()));
            }
            let error = Payload::from_value(&Value::Object(object))?;
            let output = match catcher.result_path() {
                CatchResult::Replace => error,
                CatchResult::Discard => self.input.clone(),
                CatchResult::Path(path) => path.merge(&self.input, &error)?,
            };
            return Ok(Recovery::Catch {
                output,
                next: catcher.next(),
            });
        }
        Ok(Recovery::Fail)
    }
}

fn backoff<S: ExecutionSource + ?Sized>(
    timing: &RetryTiming,
    prior: u32,
    remaining: Duration,
    source: &mut S,
) -> Result<Duration, AslError> {
    let cap = timing.max_delay_seconds().map_or(remaining, |s| {
        remaining.min(Duration::from_secs(u64::from(s.get())))
    });
    let seconds =
        f64::from(timing.interval_seconds().get()) * timing.backoff_rate().powf(f64::from(prior));
    let delay = if !seconds.is_finite() || seconds >= cap.as_secs_f64() {
        cap
    } else {
        Duration::try_from_secs_f64(seconds)
            .map_err(|_| AslError::InvalidExecution)?
            .min(cap)
    };
    if timing.jitter() == Jitter::None {
        return Ok(delay);
    }
    jitter(delay, source)
}
fn jitter<S: ExecutionSource + ?Sized>(
    cap: Duration,
    source: &mut S,
) -> Result<Duration, AslError> {
    let cap = u64::try_from(cap.as_nanos()).map_err(|_| AslError::InvalidExecution)?;
    let width = cap.checked_add(1).ok_or(AslError::InvalidExecution)?;
    let threshold = width.wrapping_neg() % width;
    for _ in 0..128 {
        let draw = source.random_u64()?;
        if draw >= threshold {
            return Ok(Duration::from_nanos(draw % width));
        }
    }
    Err(AslError::WorkLimit)
}
