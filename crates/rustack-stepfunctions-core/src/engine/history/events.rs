//! Official sequential history DTO construction; no source text in diagnostics.

use std::str;

use chrono::{DateTime, Utc};
use rustack_stepfunctions_model::types::{
    ExecutionAbortedEventDetails, ExecutionFailedEventDetails, ExecutionStartedEventDetails,
    ExecutionSucceededEventDetails, ExecutionTimedOutEventDetails, HistoryEvent,
    HistoryEventExecutionDataDetails, HistoryEventType, LambdaFunctionFailedEventDetails,
    LambdaFunctionScheduleFailedEventDetails, LambdaFunctionScheduledEventDetails,
    LambdaFunctionSucceededEventDetails, LambdaFunctionTimedOutEventDetails,
    StateEnteredEventDetails, StateExitedEventDetails, TaskFailedEventDetails,
    TaskScheduledEventDetails, TaskStartFailedEventDetails, TaskStartedEventDetails,
    TaskSubmittedEventDetails, TaskSucceededEventDetails, TaskTimedOutEventDetails,
};

use crate::{
    AslError, CallbackHistory, CompiledMachine, ExecutionOutcome, Integration, Payload, StateId,
    StateView, TaskFailure,
};

pub(crate) fn event(kind: HistoryEventType, wall: DateTime<Utc>) -> HistoryEvent {
    HistoryEvent::builder()
        .id(0)
        .timestamp(wall)
        .r#type(kind)
        .build()
}
fn text(payload: &Payload) -> Result<String, AslError> {
    str::from_utf8(payload.original_bytes())
        .map(str::to_owned)
        .map_err(|_| AslError::InvalidExecution)
}
fn details() -> HistoryEventExecutionDataDetails {
    let mut d = HistoryEventExecutionDataDetails::default();
    d.truncated = Some(false);
    d
}
pub(crate) fn started(input: &Payload, wall: DateTime<Utc>) -> Result<HistoryEvent, AslError> {
    let mut e = event(HistoryEventType::ExecutionStarted, wall);
    let mut d = ExecutionStartedEventDetails::default();
    d.input = Some(text(input)?);
    d.input_details = Some(details());
    e.execution_started_event_details = Some(d);
    Ok(e)
}
pub(crate) fn entered(
    machine: &CompiledMachine,
    state: StateId,
    input: &Payload,
    wall: DateTime<Utc>,
) -> Result<HistoryEvent, AslError> {
    let kind = match machine.state(state)? {
        StateView::Pass { .. } => HistoryEventType::PassStateEntered,
        StateView::Task { .. } => HistoryEventType::TaskStateEntered,
        StateView::Wait { .. } => HistoryEventType::WaitStateEntered,
        StateView::Choice { .. } => HistoryEventType::ChoiceStateEntered,
        StateView::Succeed { .. } => HistoryEventType::SucceedStateEntered,
        StateView::Fail { .. } => HistoryEventType::FailStateEntered,
        StateView::Parallel { .. } => HistoryEventType::ParallelStateEntered,
        StateView::Map { .. } => HistoryEventType::MapStateEntered,
    };
    let mut e = event(kind, wall);
    let mut d = StateEnteredEventDetails::default();
    d.name = machine.state_name(state)?.into();
    d.input = Some(text(input)?);
    d.input_details = Some(details());
    e.state_entered_event_details = Some(d);
    Ok(e)
}
pub(crate) fn exited(
    machine: &CompiledMachine,
    state: StateId,
    output: &Payload,
    wall: DateTime<Utc>,
) -> Result<HistoryEvent, AslError> {
    let kind = match machine.state(state)? {
        StateView::Pass { .. } => HistoryEventType::PassStateExited,
        StateView::Task { .. } => HistoryEventType::TaskStateExited,
        StateView::Wait { .. } => HistoryEventType::WaitStateExited,
        StateView::Choice { .. } => HistoryEventType::ChoiceStateExited,
        StateView::Succeed { .. } => HistoryEventType::SucceedStateExited,
        StateView::Fail { .. } => return Err(AslError::InvalidStateId),
        StateView::Parallel { .. } => HistoryEventType::ParallelStateExited,
        StateView::Map { .. } => HistoryEventType::MapStateExited,
    };
    let mut e = event(kind, wall);
    let mut d = StateExitedEventDetails::default();
    d.name = machine.state_name(state)?.into();
    d.output = Some(text(output)?);
    d.output_details = Some(details());
    e.state_exited_event_details = Some(d);
    Ok(e)
}
fn resource(machine: &CompiledMachine, state: StateId) -> Result<&Integration, AslError> {
    match machine.state(state)? {
        StateView::Task { resource, .. } => Ok(resource),
        _ => Err(AslError::InvalidStateId),
    }
}
fn metadata(resource: &Integration) -> (&'static str, &'static str) {
    match resource {
        Integration::StartExecution(pattern) => (pattern.action(), "states"),
        Integration::LambdaCallback(_) => ("invoke.waitForTaskToken", "lambda"),
        Integration::LambdaInvoke | Integration::DirectLambda(_) => ("invoke", "lambda"),
    }
}
pub(crate) fn task_submitted(
    machine: &CompiledMachine,
    state: StateId,
    output: &Payload,
    wall: DateTime<Utc>,
) -> Result<HistoryEvent, AslError> {
    let integration = resource(machine, state)?;
    if !matches!(integration, Integration::StartExecution(pattern) if pattern.owned())
        && !matches!(integration, Integration::LambdaCallback(_))
    {
        return Err(AslError::InvalidStateId);
    }
    let mut e = event(HistoryEventType::TaskSubmitted, wall);
    let mut d = TaskSubmittedEventDetails::default();
    d.resource = metadata(integration).0.into();
    d.resource_type = metadata(integration).1.into();
    d.output = Some(text(output)?);
    let mut output_details = details();
    output_details.truncated = Some(matches!(integration, Integration::LambdaCallback(_)));
    d.output_details = Some(output_details);
    e.task_submitted_event_details = Some(d);
    Ok(e)
}
pub(crate) fn scheduled(
    machine: &CompiledMachine,
    state: StateId,
    input: &Payload,
    timeout: Option<i64>,
    wall: DateTime<Utc>,
) -> Result<HistoryEvent, AslError> {
    match resource(machine, state)? {
        Integration::DirectLambda(arn) => {
            let mut e = event(HistoryEventType::LambdaFunctionScheduled, wall);
            let mut d = LambdaFunctionScheduledEventDetails::default();
            d.input = Some(text(input)?);
            d.input_details = Some(details());
            d.resource = arn.as_str().into();
            d.timeout_in_seconds = timeout;
            e.lambda_function_scheduled_event_details = Some(d);
            Ok(e)
        }
        integration @ (Integration::LambdaInvoke | Integration::StartExecution(_)) => {
            task_scheduled(machine, integration, input, timeout, None, wall)
        }
        Integration::LambdaCallback(_) => Err(AslError::InvalidExecution),
    }
}
pub(crate) fn scheduled_callback(
    machine: &CompiledMachine,
    state: StateId,
    history: &CallbackHistory,
    timeout: Option<i64>,
    heartbeat: Option<i64>,
    wall: DateTime<Utc>,
) -> Result<HistoryEvent, AslError> {
    let integration = resource(machine, state)?;
    if !matches!(integration, Integration::LambdaCallback(_)) {
        return Err(AslError::InvalidStateId);
    }
    task_scheduled(
        machine,
        integration,
        history.payload(),
        timeout,
        heartbeat,
        wall,
    )
}
fn task_scheduled(
    machine: &CompiledMachine,
    integration: &Integration,
    input: &Payload,
    timeout: Option<i64>,
    heartbeat: Option<i64>,
    wall: DateTime<Utc>,
) -> Result<HistoryEvent, AslError> {
    let mut e = event(HistoryEventType::TaskScheduled, wall);
    e.task_scheduled_event_details = Some(
        TaskScheduledEventDetails::builder()
            .parameters(text(input)?)
            .region(machine.scope().region().into())
            .resource(metadata(integration).0.into())
            .resource_type(metadata(integration).1.into())
            .timeout_in_seconds(timeout)
            .heartbeat_in_seconds(heartbeat)
            .build(),
    );
    Ok(e)
}
pub(crate) fn task_started(
    machine: &CompiledMachine,
    state: StateId,
    wall: DateTime<Utc>,
) -> Result<HistoryEvent, AslError> {
    match resource(machine, state)? {
        Integration::DirectLambda(_) => Ok(event(HistoryEventType::LambdaFunctionStarted, wall)),
        integration @ (Integration::LambdaInvoke
        | Integration::LambdaCallback(_)
        | Integration::StartExecution(_)) => {
            let mut e = event(HistoryEventType::TaskStarted, wall);
            let mut d = TaskStartedEventDetails::default();
            d.resource = metadata(integration).0.into();
            d.resource_type = metadata(integration).1.into();
            e.task_started_event_details = Some(d);
            Ok(e)
        }
    }
}
pub(crate) fn task_success(
    machine: &CompiledMachine,
    state: StateId,
    output: &Payload,
    wall: DateTime<Utc>,
) -> Result<HistoryEvent, AslError> {
    match resource(machine, state)? {
        Integration::DirectLambda(_) => {
            let mut e = event(HistoryEventType::LambdaFunctionSucceeded, wall);
            let mut d = LambdaFunctionSucceededEventDetails::default();
            d.output = Some(text(output)?);
            d.output_details = Some(details());
            e.lambda_function_succeeded_event_details = Some(d);
            Ok(e)
        }
        integration @ (Integration::LambdaInvoke
        | Integration::LambdaCallback(_)
        | Integration::StartExecution(_)) => {
            let mut e = event(HistoryEventType::TaskSucceeded, wall);
            let mut d = TaskSucceededEventDetails::default();
            d.output = Some(text(output)?);
            d.output_details = Some(details());
            d.resource = metadata(integration).0.into();
            d.resource_type = metadata(integration).1.into();
            e.task_succeeded_event_details = Some(d);
            Ok(e)
        }
    }
}
#[derive(Clone, Copy)]
pub(crate) enum TaskFailureKind {
    Failed,
    TimedOut,
    Admission,
}
pub(crate) fn task_failure(
    machine: &CompiledMachine,
    state: StateId,
    failure: &TaskFailure,
    kind: TaskFailureKind,
    wall: DateTime<Utc>,
) -> Result<HistoryEvent, AslError> {
    let integration = resource(machine, state)?;
    macro_rules! failure {
        ($event:ident,$field:ident,$details:ident) => {{
            let mut e = event(HistoryEventType::$event, wall);
            let mut d = $details::default();
            d.error.clone_from(&failure.0.error);
            d.cause.clone_from(&failure.0.cause);
            e.$field = Some(d);
            e
        }};
        (optimized $event:ident,$field:ident,$details:ident) => {{
            let mut e = failure!($event, $field, $details);
            if let Some(d) = &mut e.$field {
                d.resource = metadata(integration).0.into();
                d.resource_type = metadata(integration).1.into();
            }
            e
        }};
    }
    Ok(match (integration, kind) {
        (Integration::DirectLambda(_), TaskFailureKind::Failed) => failure!(
            LambdaFunctionFailed,
            lambda_function_failed_event_details,
            LambdaFunctionFailedEventDetails
        ),
        (Integration::DirectLambda(_), TaskFailureKind::TimedOut) => failure!(
            LambdaFunctionTimedOut,
            lambda_function_timed_out_event_details,
            LambdaFunctionTimedOutEventDetails
        ),
        (Integration::DirectLambda(_), TaskFailureKind::Admission) => failure!(
            LambdaFunctionScheduleFailed,
            lambda_function_schedule_failed_event_details,
            LambdaFunctionScheduleFailedEventDetails
        ),
        (
            Integration::LambdaInvoke
            | Integration::LambdaCallback(_)
            | Integration::StartExecution(_),
            TaskFailureKind::Failed,
        ) => {
            failure!(optimized TaskFailed,task_failed_event_details,TaskFailedEventDetails)
        }
        (
            Integration::LambdaInvoke
            | Integration::LambdaCallback(_)
            | Integration::StartExecution(_),
            TaskFailureKind::TimedOut,
        ) => {
            failure!(optimized TaskTimedOut,task_timed_out_event_details,TaskTimedOutEventDetails)
        }
        (
            Integration::LambdaInvoke
            | Integration::LambdaCallback(_)
            | Integration::StartExecution(_),
            TaskFailureKind::Admission,
        ) => {
            failure!(optimized TaskStartFailed,task_start_failed_event_details,TaskStartFailedEventDetails)
        }
    })
}
pub(crate) fn terminal(
    outcome: &ExecutionOutcome,
    wall: DateTime<Utc>,
) -> Result<HistoryEvent, AslError> {
    let e = match outcome {
        ExecutionOutcome::Succeeded(output) => {
            let mut e = event(HistoryEventType::ExecutionSucceeded, wall);
            let mut d = ExecutionSucceededEventDetails::default();
            d.output = Some(text(output)?);
            d.output_details = Some(details());
            e.execution_succeeded_event_details = Some(d);
            e
        }
        ExecutionOutcome::Failed(f) => {
            let mut e = event(HistoryEventType::ExecutionFailed, wall);
            let mut d = ExecutionFailedEventDetails::default();
            d.error.clone_from(&f.error);
            d.cause.clone_from(&f.cause);
            e.execution_failed_event_details = Some(d);
            e
        }
        ExecutionOutcome::Aborted(f) => {
            let mut e = event(HistoryEventType::ExecutionAborted, wall);
            let mut d = ExecutionAbortedEventDetails::default();
            d.error.clone_from(&f.0.error);
            d.cause.clone_from(&f.0.cause);
            e.execution_aborted_event_details = Some(d);
            e
        }
        ExecutionOutcome::TimedOut => {
            let mut e = event(HistoryEventType::ExecutionTimedOut, wall);
            let mut d = ExecutionTimedOutEventDetails::default();
            d.error = Some("States.Timeout".into());
            d.cause = Some("Execution deadline exceeded".into());
            e.execution_timed_out_event_details = Some(d);
            e
        }
    };
    Ok(e)
}
