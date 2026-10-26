//! Six compiled state instructions; no resource lifetime is hidden in evaluation.

use std::time::{Duration, Instant};

use super::{
    AttemptId, ClockSample, Continuation, DriverStep, ExecutionDriver, ExecutionFailure,
    ExecutionOutcome, ExecutionSource, Phase, TaskDispatch,
};
use crate::{AslError, FailureView, Payload, StateView, TaskTimeout, Transition};

impl ExecutionDriver {
    pub(super) fn execute<S: ExecutionSource + ?Sized>(
        &mut self,
        now: ClockSample,
        source: &mut S,
    ) -> Result<DriverStep, AslError> {
        let machine = self.machine.clone();
        match machine.state(self.state)? {
            StateView::Parallel { .. } | StateView::Map { .. } => {
                Err(AslError::StructuredExecutionRequired)
            }
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
                Ok(self.settle(ExecutionOutcome::Failed(ExecutionFailure { error, cause })))
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
            StateView::Task { flow, timeout, .. } => {
                // Timeout path is state input metadata, not the invocation event.
                let deadline = self.task_deadline(timeout, now.monotonic)?;
                let parameters = flow.prepare_with_source(&self.input, &self.context, source)?;
                self.generation = self
                    .generation
                    .checked_add(1)
                    .ok_or(AslError::InvalidExecution)?;
                let attempt = AttemptId(self.generation);
                self.phase = Phase::Calling { attempt, deadline };
                Ok(DriverStep::Dispatch(TaskDispatch {
                    attempt,
                    state: self.state,
                    parameters,
                    deadline,
                }))
            }
        }
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
    fn task_deadline(&self, timeout: &TaskTimeout, now: Instant) -> Result<Instant, AslError> {
        let seconds = match timeout {
            TaskTimeout::Omitted => return Ok(self.deadline),
            TaskTimeout::Seconds(seconds) => u64::from(seconds.get()),
            TaskTimeout::Path(path) => path
                .lookup(&self.input, &self.context)
                .ok_or(AslError::Missing)?
                .as_u64()
                .filter(|s| (1..=99_999_999).contains(s))
                .ok_or(AslError::WaitValue)?,
        };
        let remaining = self.deadline.saturating_duration_since(now);
        now.checked_add(Duration::from_secs(seconds).min(remaining))
            .ok_or(AslError::InvalidExecution)
    }
}
