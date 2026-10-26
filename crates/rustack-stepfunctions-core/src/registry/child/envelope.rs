//! Bounded optimized envelopes, distinct from HTTP epoch-seconds DTOs.

use serde_json::{Value, json};

use super::{ChildError, ChildExecution, ChildPattern};
use crate::{ExecutionOutcome, ExecutionSnapshot, Payload, TaskFailure};

impl ChildExecution {
    /// Map the observed child to a bounded optimized Task result without IO.
    ///
    /// Embedding async owners run this potentially payload-bearing work under
    /// retained CPU admission. This does not confirm physical cleanup.
    ///
    /// # Errors
    /// Returns Pending, quota/JSON failure, or an unsuccessful child Task result.
    ///
    /// ```no_run
    /// # fn result(c:&rustack_stepfunctions_core::ChildExecution){let result=c.result();}
    /// ```
    pub fn result(&self) -> Result<Result<Payload, TaskFailure>, ChildError> {
        map(&self.snapshot, self.pattern)
    }
    pub(crate) fn mapper(
        &self,
        start: bool,
    ) -> impl FnOnce() -> Result<Result<Payload, TaskFailure>, ChildError> + Send + 'static {
        let snapshot = self.snapshot.clone();
        let pattern = if start {
            ChildPattern::RequestResponse
        } else {
            self.pattern
        };
        move || map(&snapshot, pattern)
    }
}
fn map(
    snapshot: &ExecutionSnapshot,
    pattern: ChildPattern,
) -> Result<Result<Payload, TaskFailure>, ChildError> {
    let strings = match pattern {
        ChildPattern::RequestResponse => {
            return Ok(Ok(Payload::from_value(&json!({
                "ExecutionArn":snapshot.arn().as_str(),
                "StartDate":snapshot.start_date().timestamp_millis(),
            }))?));
        }
        ChildPattern::Sync => true,
        ChildPattern::Sync2 => false,
    };
    let outcome = snapshot.view().outcome().ok_or(ChildError::Pending)?;
    let output = match outcome.as_ref() {
        ExecutionOutcome::Succeeded(output) => output,
        other => return Ok(Err(failed(snapshot.arn().as_str(), other)?)),
    };
    let (input, output) = if strings {
        (
            Value::String(text(snapshot.input())?),
            Value::String(text(output)?),
        )
    } else {
        (snapshot.input().value().clone(), output.value().clone())
    };
    let stopped = snapshot.view().stop_date().ok_or(ChildError::Invalid)?;
    Ok(Ok(Payload::from_value(&json!({
        "ExecutionArn":snapshot.arn().as_str(),
        "StateMachineArn":snapshot.arn().machine().as_str(),
        "Name":snapshot.arn().name().as_str(),
        "Status":"SUCCEEDED", "Input":input, "Output":output,
        "InputDetails":{"Included":true}, "OutputDetails":{"Included":true},
        "StartDate":snapshot.start_date().timestamp_millis(),
        "StopDate":stopped.timestamp_millis(),
    }))?))
}
fn text(payload: &Payload) -> Result<String, ChildError> {
    String::from_utf8(payload.original_bytes().to_vec()).map_err(|_| ChildError::Invalid)
}
fn failed(arn: &str, outcome: &ExecutionOutcome) -> Result<TaskFailure, ChildError> {
    let (status, error, cause) = match outcome {
        ExecutionOutcome::Failed(failure) => ("FAILED", failure.error(), failure.cause()),
        ExecutionOutcome::TimedOut => ("TIMED_OUT", None, None),
        ExecutionOutcome::Aborted(failure) => (
            "ABORTED",
            failure.details().error(),
            failure.details().cause(),
        ),
        ExecutionOutcome::Succeeded(_) => return Err(ChildError::Invalid),
    };
    let truncated = cause.is_some_and(|cause| cause.len() > 4096);
    let cause = cause
        .map(|cause| {
            let mut end = cause.len().min(4096);
            while !cause.is_char_boundary(end) {
                end = end.saturating_sub(1);
            }
            cause.get(..end).ok_or(ChildError::Invalid)
        })
        .transpose()?;
    let details =
        json!({"ExecutionArn":arn,"Status":status,"Error":error,"Cause":cause,"CauseTruncated":truncated}).to_string();
    Ok(TaskFailure::new("States.TaskFailed", Some(&details))?)
}
