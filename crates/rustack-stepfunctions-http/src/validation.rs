//! Wire byte/count bounds. Core retains ARN, role, ASL and capability semantics.

use rustack_core::json::{JsonLimits, validate as json};
use rustack_stepfunctions_model::{
    StepFunctionsErrorCode as Code,
    input::{CreateStateMachineInput, UpdateStateMachineInput},
    types::{EncryptionConfiguration, LoggingConfiguration, Tag},
};

use crate::{HttpError, Request};

pub(crate) fn validate(request: &Request) -> Result<(), HttpError> {
    resource(request)?;
    details(request)
}
fn resource(request: &Request) -> Result<(), HttpError> {
    let arn = match request {
        Request::CreateStateMachine(value) => Some(value.role_arn.as_str()),
        Request::UpdateStateMachine(value) => Some(value.state_machine_arn.as_str()),
        Request::DeleteStateMachine(value) => Some(value.state_machine_arn.as_str()),
        Request::DescribeStateMachine(value) => Some(value.state_machine_arn.as_str()),
        Request::StartExecution(value) => Some(value.state_machine_arn.as_str()),
        Request::DescribeExecution(value) => Some(value.execution_arn.as_str()),
        Request::StopExecution(value) => Some(value.execution_arn.as_str()),
        Request::GetExecutionHistory(value) => Some(value.execution_arn.as_str()),
        Request::DescribeStateMachineForExecution(value) => Some(value.execution_arn.as_str()),
        Request::TagResource(value) => Some(value.resource_arn.as_str()),
        Request::UntagResource(value) => Some(value.resource_arn.as_str()),
        Request::ListTagsForResource(value) => Some(value.resource_arn.as_str()),
        _ => None,
    };
    if let Some(arn) = arn {
        identifier(arn, 256, Code::InvalidArn)?;
    }
    Ok(())
}
fn details(request: &Request) -> Result<(), HttpError> {
    match request {
        Request::CreateStateMachine(value) => create(value),
        Request::UpdateStateMachine(value) => update(value),
        Request::ValidateStateMachineDefinition(value) => {
            text(&value.definition, 1024 * 1024, Code::ValidationException)?;
            page(value.max_results, None, 100)
        }
        Request::ListStateMachines(value) => {
            page(value.max_results, value.next_token.as_deref(), 1000)
        }
        Request::ListExecutions(value) => {
            optional_id(value.state_machine_arn.as_deref(), 256, Code::InvalidArn)?;
            optional_id(value.map_run_arn.as_deref(), 256, Code::InvalidArn)?;
            page(value.max_results, value.next_token.as_deref(), 1000)
        }
        Request::GetExecutionHistory(value) => {
            page(value.max_results, value.next_token.as_deref(), 1000)
        }
        Request::StartExecution(value) => {
            if let Some(value) = &value.name {
                name(value)?;
            }
            if let Some(value) = &value.input {
                payload(value, Code::InvalidExecutionInput)?;
            }
            ascii_text(value.trace_header.as_deref(), 256)
        }
        Request::StopExecution(value) => failure(value.error.as_deref(), value.cause.as_deref()),
        Request::TagResource(value) => tags(&value.tags),
        Request::UntagResource(value) => {
            if value.tag_keys.len() > 50 {
                return Err(HttpError::Field(Code::TooManyTags));
            }
            for key in &value.tag_keys {
                nonempty(key, 128, Code::ValidationException)?;
            }
            Ok(())
        }
        Request::SendTaskSuccess(value) => {
            identifier(&value.task_token, 2048, Code::InvalidToken)?;
            payload(&value.output, Code::InvalidOutput)
        }
        Request::SendTaskFailure(value) => {
            identifier(&value.task_token, 2048, Code::InvalidToken)?;
            failure(value.error.as_deref(), value.cause.as_deref())
        }
        Request::SendTaskHeartbeat(value) => {
            identifier(&value.task_token, 2048, Code::InvalidToken)
        }
        _ => Ok(()),
    }
}
fn create(value: &CreateStateMachineInput) -> Result<(), HttpError> {
    text(&value.definition, 1024 * 1024, Code::ValidationException)?;
    name(&value.name)?;
    tags(&value.tags)?;
    metadata(
        value.logging_configuration.as_ref(),
        value.encryption_configuration.as_ref(),
        value.version_description.as_deref(),
    )
}
fn update(value: &UpdateStateMachineInput) -> Result<(), HttpError> {
    if let Some(value) = &value.definition {
        text(value, 1024 * 1024, Code::ValidationException)?;
    }
    optional_id(value.role_arn.as_deref(), 256, Code::InvalidArn)?;
    metadata(
        value.logging_configuration.as_ref(),
        value.encryption_configuration.as_ref(),
        value.version_description.as_deref(),
    )
}
fn metadata(
    logging: Option<&LoggingConfiguration>,
    encryption: Option<&EncryptionConfiguration>,
    version: Option<&str>,
) -> Result<(), HttpError> {
    if let Some(logging) = logging {
        if logging.destinations.len() > 1 {
            return Err(HttpError::Field(Code::ValidationException));
        }
        for destination in &logging.destinations {
            if let Some(group) = &destination.cloud_watch_logs_log_group {
                optional_id(
                    group.log_group_arn.as_deref(),
                    256,
                    Code::ValidationException,
                )?;
            }
        }
    }
    if let Some(encryption) = encryption {
        optional_id(
            encryption.kms_key_id.as_deref(),
            256,
            Code::ValidationException,
        )?;
        if encryption
            .kms_data_key_reuse_period_seconds
            .is_some_and(|value| !(60..=900).contains(&value))
        {
            return Err(HttpError::Field(Code::ValidationException));
        }
    }
    if let Some(version) = version {
        text(version, 256, Code::ValidationException)?;
    }
    Ok(())
}
fn page(maximum: Option<i32>, token: Option<&str>, cap: i32) -> Result<(), HttpError> {
    if maximum.is_some_and(|value| !(0..=cap).contains(&value)) {
        return Err(HttpError::Field(Code::ValidationException));
    }
    optional_id(token, 2048, Code::InvalidToken)
}
fn tags(values: &[Tag]) -> Result<(), HttpError> {
    if values.len() > 50 {
        return Err(HttpError::Field(Code::TooManyTags));
    }
    for value in values {
        let key = value
            .key
            .as_deref()
            .ok_or(HttpError::Field(Code::ValidationException))?;
        nonempty(key, 128, Code::ValidationException)?;
        if let Some(value) = &value.value {
            text(value, 256, Code::ValidationException)?;
        }
    }
    Ok(())
}
fn name(value: &str) -> Result<(), HttpError> {
    nonempty(value, 80, Code::InvalidName)?;
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
    {
        return Err(HttpError::Field(Code::InvalidName));
    }
    Ok(())
}
fn failure(error: Option<&str>, cause: Option<&str>) -> Result<(), HttpError> {
    if let Some(value) = error {
        text(value, 256, Code::ValidationException)?;
    }
    if let Some(value) = cause {
        text(value, 32768, Code::ValidationException)?;
    }
    Ok(())
}
fn optional_id(value: Option<&str>, max: usize, code: Code) -> Result<(), HttpError> {
    if let Some(value) = value {
        identifier(value, max, code)?;
    }
    Ok(())
}
fn identifier(value: &str, max: usize, code: Code) -> Result<(), HttpError> {
    nonempty(value, max, code)?;
    if !value.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err(HttpError::Field(code));
    }
    Ok(())
}
fn nonempty(value: &str, max: usize, code: Code) -> Result<(), HttpError> {
    if value.is_empty() {
        Err(HttpError::Field(code))
    } else {
        text(value, max, code)
    }
}
fn text(value: &str, max: usize, code: Code) -> Result<(), HttpError> {
    if value.len() > max {
        Err(HttpError::Field(code))
    } else {
        Ok(())
    }
}
fn payload(value: &str, code: Code) -> Result<(), HttpError> {
    nonempty(value, 262_144, code)?;
    let limits = JsonLimits::new(262_144, 64, 65_536, 262_144, 262_144).map_err(HttpError::Json)?;
    json(value.as_bytes(), limits).map_err(|_| HttpError::Field(code))
}

fn ascii_text(value: Option<&str>, maximum: usize) -> Result<(), HttpError> {
    if value.is_some_and(|value| value.len() > maximum || !value.is_ascii()) {
        Err(HttpError::Field(Code::ValidationException))
    } else {
        Ok(())
    }
}
