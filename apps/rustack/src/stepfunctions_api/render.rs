//! Exact modeled M1 replies from immutable core captures.

use std::{str, sync::Arc};

use rustack_stepfunctions_core::{
    ExecutionOutcome, ExecutionSnapshot, MachineSnapshot, MachineStatus, RegistryResult,
};
use rustack_stepfunctions_http::Response;
use rustack_stepfunctions_model::{
    StepFunctionsError, StepFunctionsErrorCode as Code, StepFunctionsOperation as Op, output,
    types::{
        CloudWatchEventsExecutionDataDetails, EncryptionConfiguration, EncryptionType,
        ExecutionListItem, ExecutionStatus, LogLevel, LoggingConfiguration, StateMachineListItem,
        StateMachineStatus, StateMachineType, Tag, TracingConfiguration,
    },
};

use super::error::{error, source};

pub(super) fn text(bytes: &[u8]) -> Result<String, StepFunctionsError> {
    str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|e| source(Code::InternalFailure, e))
}
pub(super) fn status(value: &ExecutionSnapshot) -> Result<ExecutionStatus, StepFunctionsError> {
    Ok(match value.view().outcome().map(Arc::as_ref) {
        None => ExecutionStatus::Running,
        Some(ExecutionOutcome::Succeeded(_)) => ExecutionStatus::Succeeded,
        Some(ExecutionOutcome::Failed(_)) => ExecutionStatus::Failed,
        Some(ExecutionOutcome::TimedOut) => ExecutionStatus::TimedOut,
        Some(ExecutionOutcome::Aborted(_)) => ExecutionStatus::Aborted,
        _ => return Err(error(Code::InternalFailure)),
    })
}
fn off() -> LoggingConfiguration {
    let mut value = LoggingConfiguration::default();
    value.level = Some(LogLevel::Off);
    value
}
fn tracing() -> TracingConfiguration {
    let mut value = TracingConfiguration::default();
    value.enabled = Some(false);
    value
}
fn encryption() -> EncryptionConfiguration {
    let mut value = EncryptionConfiguration::default();
    value.r#type = EncryptionType::AwsOwnedKey;
    value
}
pub(super) fn tag(key: &str, value: &str) -> Tag {
    let mut out = Tag::default();
    out.key = Some(key.into());
    out.value = Some(value.into());
    out
}
pub(super) fn machine_item(m: &MachineSnapshot) -> StateMachineListItem {
    let mut out = StateMachineListItem::default();
    out.creation_date = m.creation_date();
    out.name = m.arn().name().as_str().into();
    out.state_machine_arn = m.arn().as_str().into();
    out.r#type = StateMachineType::Standard;
    out
}
pub(super) fn execution_item(
    e: &ExecutionSnapshot,
) -> Result<ExecutionListItem, StepFunctionsError> {
    let mut out = ExecutionListItem::default();
    out.execution_arn = e.arn().as_str().into();
    out.name = e.arn().name().as_str().into();
    out.start_date = e.start_date();
    out.state_machine_arn = e.arn().machine().as_str().into();
    out.status = status(e)?;
    out.stop_date = e.view().stop_date();
    Ok(out)
}
pub(super) fn render(op: Op, result: RegistryResult) -> Result<Response, StepFunctionsError> {
    match (op, result) {
        (Op::CreateStateMachine, RegistryResult::Machine(m)) => {
            let mut out = output::CreateStateMachineOutput::default();
            out.creation_date = m.creation_date();
            out.state_machine_arn = m.arn().as_str().into();
            Ok(out.into())
        }
        (Op::UpdateStateMachine, RegistryResult::Machine(m)) => {
            let mut out = output::UpdateStateMachineOutput::default();
            out.update_date = m.update_date();
            out.revision_id = Some(m.revision().id().into());
            Ok(out.into())
        }
        (Op::DeleteStateMachine, RegistryResult::Deleted) => {
            Ok(output::DeleteStateMachineOutput::default().into())
        }
        (Op::TagResource, RegistryResult::Tags(_)) => {
            Ok(output::TagResourceOutput::default().into())
        }
        (Op::UntagResource, RegistryResult::Tags(_)) => {
            Ok(output::UntagResourceOutput::default().into())
        }
        (Op::ListTagsForResource, RegistryResult::Machine(m)) => {
            let mut out = output::ListTagsForResourceOutput::default();
            out.tags = m.tags().values().iter().map(|(k, v)| tag(k, v)).collect();
            Ok(out.into())
        }
        (Op::ListStateMachineVersions, RegistryResult::Machine(_)) => {
            Ok(output::ListStateMachineVersionsOutput::default().into())
        }
        (Op::DescribeStateMachine, RegistryResult::Machine(m)) => describe_machine(&m),
        (Op::StartExecution, RegistryResult::Execution(e)) => {
            let mut out = output::StartExecutionOutput::default();
            out.execution_arn = e.arn().as_str().into();
            out.start_date = e.start_date();
            Ok(out.into())
        }
        (Op::DescribeExecution, RegistryResult::Execution(e)) => describe_execution(&e),
        (Op::StopExecution, RegistryResult::Execution(e)) => {
            let mut out = output::StopExecutionOutput::default();
            out.stop_date = e
                .view()
                .stop_date()
                .ok_or_else(|| error(Code::InternalFailure))?;
            Ok(out.into())
        }
        (Op::DescribeStateMachineForExecution, RegistryResult::Execution(e)) => {
            let mut out = output::DescribeStateMachineForExecutionOutput::default();
            out.definition = text(e.revision().machine().original_bytes())?;
            out.name = e.arn().machine().name().as_str().into();
            out.role_arn = e.revision().role().as_str().into();
            out.state_machine_arn = e.arn().machine().as_str().into();
            out.revision_id = Some(e.revision().id().into());
            out.update_date = e.revision().publication_date();
            out.logging_configuration = Some(off());
            out.tracing_configuration = Some(tracing());
            out.encryption_configuration = Some(encryption());
            Ok(out.into())
        }
        _ => Err(error(Code::InternalFailure)),
    }
}
fn describe_machine(m: &MachineSnapshot) -> Result<Response, StepFunctionsError> {
    let mut out = output::DescribeStateMachineOutput::default();
    out.definition = text(m.revision().machine().original_bytes())?;
    out.creation_date = m.creation_date();
    out.name = m.arn().name().as_str().into();
    out.role_arn = m.revision().role().as_str().into();
    out.state_machine_arn = m.arn().as_str().into();
    out.revision_id = Some(m.revision().id().into());
    out.r#type = StateMachineType::Standard;
    out.status = Some(match m.status() {
        MachineStatus::Active => StateMachineStatus::Active,
        MachineStatus::Deleting => StateMachineStatus::Deleting,
        _ => return Err(error(Code::InternalFailure)),
    });
    out.logging_configuration = Some(off());
    out.tracing_configuration = Some(tracing());
    out.encryption_configuration = Some(encryption());
    Ok(out.into())
}
fn describe_execution(e: &ExecutionSnapshot) -> Result<Response, StepFunctionsError> {
    let mut out = output::DescribeExecutionOutput::default();
    out.execution_arn = e.arn().as_str().into();
    out.state_machine_arn = e.arn().machine().as_str().into();
    out.start_date = e.start_date();
    out.stop_date = e.view().stop_date();
    out.status = status(e)?;
    out.name = Some(e.arn().name().as_str().into());
    out.input = Some(text(e.input().original_bytes())?);
    out.trace_header = e.trace_header().map(str::to_owned);
    let mut details = CloudWatchEventsExecutionDataDetails::default();
    details.included = Some(true);
    out.input_details = Some(details.clone());
    match e.view().outcome().map(Arc::as_ref) {
        Some(ExecutionOutcome::Succeeded(payload)) => {
            out.output = Some(text(payload.original_bytes())?);
            out.output_details = Some(details);
        }
        Some(ExecutionOutcome::Failed(f)) => {
            out.error = f.error().map(str::to_owned);
            out.cause = f.cause().map(str::to_owned);
        }
        Some(ExecutionOutcome::Aborted(f)) => {
            out.error = f.details().error().map(str::to_owned);
            out.cause = f.details().cause().map(str::to_owned);
        }
        _ => {}
    }
    Ok(out.into())
}
