//! Compiled frame instructions; no resource lifetime is hidden in evaluation.

use std::time::{Duration, Instant};

use serde_json::Value;

use super::{
    AttemptId, CallbackDispatch, ClockSample, Continuation, DriverStep, ExecutionDriver,
    ExecutionFailure, ExecutionOutcome, ExecutionSource, Phase, TaskDispatch,
    callback::CallbackIdentity,
};
use crate::{
    AslError, DataFlow, FailureView, Integration, Payload, StateView, TaskTimeout, Transition,
    compiler::TASK_SECONDS_LIMIT,
};

impl ExecutionDriver {
    pub(super) fn execute<S: ExecutionSource + ?Sized>(
        &mut self,
        now: ClockSample,
        source: &mut S,
    ) -> Result<DriverStep, AslError> {
        let machine = self.machine.clone();
        match machine.state(self.state)? {
            StateView::Parallel { .. } | StateView::Map { .. } => self.fanout(source),
            StateView::Pass { flow, result, next } => {
                let prepared = flow.prepare_with_source(&self.input, &self.context, source)?;
                let result = result
                    .map(Payload::from_value)
                    .transpose()?
                    .unwrap_or(prepared);
                let output =
                    flow.finish_with_source(&self.input, &result, &self.context, source)?;
                Ok(self.transfer(output, next))
            }
            StateView::Succeed { flow } => {
                let prepared = flow.prepare_with_source(&self.input, &self.context, source)?;
                let output =
                    flow.finish_with_source(&self.input, &prepared, &self.context, source)?;
                Ok(self.settle(ExecutionOutcome::Succeeded(output)))
            }
            StateView::Fail { error, cause } => {
                let error = self.failure_field(error, 256, source)?;
                let cause = self.failure_field(cause, 32_768, source)?;
                Ok(
                    self.settle(ExecutionOutcome::Failed(ExecutionFailure::fields(
                        error, cause,
                    ))),
                )
            }
            StateView::Choice {
                flow,
                branches,
                default,
            } => {
                let prepared = flow.prepare_with_source(&self.input, &self.context, source)?;
                let mut selected = None;
                for (predicate, target) in branches {
                    if predicate.evaluate(&prepared, &self.context)? {
                        selected = Some(*target);
                        break;
                    }
                }
                let Some(target) = selected.or(default) else {
                    return Ok(
                        self.settle(ExecutionOutcome::Failed(ExecutionFailure::named(
                            "States.NoChoiceMatched",
                            "No Choice rule or default matched",
                        ))),
                    );
                };
                let output =
                    flow.finish_with_source(&self.input, &prepared, &self.context, source)?;
                Ok(self.transfer(output, Transition::Next(target)))
            }
            StateView::Wait {
                flow,
                source: wait,
                next,
            } => {
                let prepared = flow.prepare_with_source(&self.input, &self.context, source)?;
                let duration = wait.resolve(&prepared, &self.context, now.wall)?;
                let output =
                    flow.finish_with_source(&self.input, &prepared, &self.context, source)?;
                if duration.is_zero() {
                    return Ok(self.transfer(output, next));
                }
                let until = now
                    .monotonic
                    .checked_add(
                        duration.min(self.deadline.saturating_duration_since(now.monotonic)),
                    )
                    .ok_or(AslError::InvalidExecution)?;
                self.phase = Phase::Waiting {
                    until,
                    continuation: Continuation::Wait { output, next },
                };
                Ok(DriverStep::Park(until))
            }
            StateView::Task {
                flow,
                timeout,
                resource,
                ..
            } => self.task(flow, timeout, resource, now, source),
        }
    }
    fn task<S: ExecutionSource + ?Sized>(
        &mut self,
        flow: &DataFlow,
        timeout: &TaskTimeout,
        resource: &Integration,
        now: ClockSample,
        source: &mut S,
    ) -> Result<DriverStep, AslError> {
        let seconds = self.task_seconds(timeout)?;
        let deadline = self.task_deadline(seconds, now.monotonic)?;
        let heartbeat = if let Integration::LambdaCallback(heartbeat) = resource {
            let heartbeat = self.task_seconds(heartbeat)?;
            if heartbeat.is_some_and(|h| h >= seconds.unwrap_or(TASK_SECONDS_LIMIT)) {
                return Err(AslError::WaitValue);
            }
            heartbeat.map(|s| Duration::from_secs(u64::from(s)))
        } else {
            None
        };
        // Callback templates require a real registered token. Ordinary
        // Tasks retain their existing eager single-evaluation behavior.
        let parameters = if matches!(resource, Integration::LambdaCallback(_)) {
            None
        } else {
            Some(flow.prepare_with_source(&self.input, &self.context, source)?)
        };
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(AslError::InvalidExecution)?;
        let attempt = AttemptId(self.generation);
        self.phase = Phase::Calling { attempt, deadline };
        Ok(match parameters {
            Some(parameters) => DriverStep::Dispatch(TaskDispatch {
                attempt,
                state: self.state,
                parameters,
                deadline,
            }),
            None => DriverStep::Callback(CallbackDispatch {
                identity: CallbackIdentity {
                    attempt,
                    state: self.state,
                    deadline,
                    timeout: seconds,
                    heartbeat,
                },
                machine: self.machine.clone(),
                input: self.input.clone(),
                context: self.context.clone(),
            }),
        })
    }
    fn failure_field<S: ExecutionSource + ?Sized>(
        &self,
        field: FailureView<'_>,
        max: usize,
        source: &mut S,
    ) -> Result<Option<String>, AslError> {
        let value = match field {
            FailureView::Omitted => return Ok(None),
            FailureView::Literal(value) => return Ok(Some(value.into())),
            FailureView::Dynamic(expression) => {
                expression.evaluate_with_source(&self.input, &self.context, source)?
            }
        };
        let text = value.value().as_str().ok_or(AslError::InvalidFailure)?;
        if text.len() > max {
            return Err(AslError::InvalidFailure);
        }
        Ok(Some(text.into()))
    }
    fn task_seconds(&self, timeout: &TaskTimeout) -> Result<Option<u32>, AslError> {
        match timeout {
            TaskTimeout::Omitted => Ok(None),
            TaskTimeout::Seconds(seconds) => Ok(Some(seconds.get())),
            TaskTimeout::Path(path) => {
                let value = path
                    .lookup(&self.input, &self.context)
                    .and_then(Value::as_u64)
                    .filter(|s| (1..=u64::from(TASK_SECONDS_LIMIT)).contains(s))
                    .ok_or(AslError::WaitValue)?;
                Ok(Some(u32::try_from(value).map_err(|_| AslError::WaitValue)?))
            }
        }
    }
    fn task_deadline(&self, seconds: Option<u32>, now: Instant) -> Result<Instant, AslError> {
        let Some(seconds) = seconds else {
            return Ok(self.deadline);
        };
        let remaining = self.deadline.saturating_duration_since(now);
        now.checked_add(Duration::from_secs(u64::from(seconds)).min(remaining))
            .ok_or(AslError::InvalidExecution)
    }
}
