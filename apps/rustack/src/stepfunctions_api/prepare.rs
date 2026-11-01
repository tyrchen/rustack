//! Retained blocking preparation under the installed STANDARD/JSONPath profile.

use rustack_stepfunctions_core::{
    AslCompiler, CallbackCommand, CallbackToken, DiagnosticSeverity, ExecutionArn, Payload,
    PreparedDefinition, RegistryCommand, RoleArn, StateMachineArn, StopReason, Tags, TenantScope,
};
use rustack_stepfunctions_http::{HttpError, Request, Response};
use rustack_stepfunctions_model::{
    StepFunctionsError, StepFunctionsErrorCode as Code, StepFunctionsOperation as Op, input,
    output,
    types::{
        EncryptionConfiguration, EncryptionType, ExecutionStatus, IncludedData, LogLevel,
        LoggingConfiguration, StateMachineType, Tag, TracingConfiguration,
        ValidateStateMachineDefinitionDiagnostic, ValidateStateMachineDefinitionResultCode,
        ValidateStateMachineDefinitionSeverity,
    },
};

use super::{
    error::{callback, error, source},
    pages::{Binding, Flags},
};

pub(super) enum Prepared {
    Command(RegistryCommand, Op),
    Page(Binding, Option<String>, RegistryCommand),
    Callback(CallbackCommand, Op),
    Ready(Response),
}
pub(super) fn prepare(
    request: Request,
    compiler: &AslCompiler,
    scope: &TenantScope,
    callbacks: bool,
) -> Result<Prepared, StepFunctionsError> {
    request.validate().map_err(|e| {
        let code = match &e {
            HttpError::Field(code) => *code,
            _ => Code::ValidationException,
        };
        source(code, e)
    })?;
    let op = request.operation();
    let command = match request {
        Request::CreateStateMachine(v) => create(*v, compiler, scope)?,
        Request::UpdateStateMachine(v) => update(&v, compiler, scope)?,
        Request::DeleteStateMachine(v) => {
            RegistryCommand::Delete(machine(&v.state_machine_arn, scope)?)
        }
        Request::DescribeStateMachine(v) => {
            included(v.included_data.as_ref())?;
            RegistryCommand::Machine(machine(&v.state_machine_arn, scope)?)
        }
        Request::ListStateMachineVersions(v) => {
            let arn = machine(&v.state_machine_arn, scope)?;
            if v.next_token.is_some() {
                return Err(error(Code::InvalidToken));
            }
            RegistryCommand::Machine(arn)
        }
        Request::StartExecution(v) => RegistryCommand::Start(
            machine(&v.state_machine_arn, scope)?,
            v.name
                .as_deref()
                .map(str::parse)
                .transpose()
                .map_err(|e| source(Code::InvalidName, e))?,
            Payload::parse(v.input.as_deref().unwrap_or("{}").as_bytes())
                .map_err(|e| source(Code::InvalidExecutionInput, e))?,
            v.trace_header,
        ),
        Request::DescribeExecution(v) => {
            included(v.included_data.as_ref())?;
            RegistryCommand::Execution(execution(&v.execution_arn, scope)?)
        }
        Request::StopExecution(v) => RegistryCommand::Stop(
            execution(&v.execution_arn, scope)?,
            StopReason::new(v.error, v.cause).map_err(|e| source(Code::ValidationException, e))?,
        ),
        Request::DescribeStateMachineForExecution(v) => {
            included(v.included_data.as_ref())?;
            RegistryCommand::Execution(execution(&v.execution_arn, scope)?)
        }
        Request::TagResource(v) => {
            RegistryCommand::Tag(machine(&v.resource_arn, scope)?, tags(v.tags)?)
        }
        Request::UntagResource(v) => RegistryCommand::Untag(
            machine(&v.resource_arn, scope)?,
            Tags::new(v.tag_keys.into_iter().map(|k| (k, String::new())).collect())
                .map_err(|e| source(Code::ValidationException, e))?,
        ),
        Request::ListTagsForResource(v) => {
            RegistryCommand::Machine(machine(&v.resource_arn, scope)?)
        }
        Request::ValidateStateMachineDefinition(v) => {
            return Ok(Prepared::Ready(validate(&v, compiler)?.into()));
        }
        Request::ListStateMachines(v) => {
            return Ok(Prepared::Page(
                Binding {
                    op,
                    resource: String::new(),
                    flags: Flags::new(v.max_results, None, false, true)?,
                },
                token(v.next_token)?,
                RegistryCommand::Machines,
            ));
        }
        Request::ListExecutions(v) => {
            if v.map_run_arn.is_some()
                || v.redrive_filter.is_some()
                || matches!(v.status_filter, Some(ExecutionStatus::PendingRedrive))
            {
                return Err(error(Code::UnsupportedOperation));
            }
            let arn = machine(
                v.state_machine_arn
                    .as_deref()
                    .ok_or_else(|| error(Code::ValidationException))?,
                scope,
            )?;
            return Ok(Prepared::Page(
                Binding {
                    op,
                    resource: arn.as_str().into(),
                    flags: Flags::new(v.max_results, v.status_filter, false, true)?,
                },
                token(v.next_token)?,
                RegistryCommand::Executions(arn),
            ));
        }
        Request::GetExecutionHistory(v) => return history(*v, scope),
        request => return callback_request(request, callbacks),
    };
    Ok(Prepared::Command(command, op))
}
fn callback_request(request: Request, enabled: bool) -> Result<Prepared, StepFunctionsError> {
    if !enabled {
        return Err(error(Code::UnsupportedOperation));
    }
    let op = request.operation();
    let command = match request {
        Request::SendTaskSuccess(v) => CallbackCommand::success(
            CallbackToken::try_from(v.task_token).map_err(callback)?,
            v.output.as_bytes(),
        )
        .map_err(callback)?,
        Request::SendTaskFailure(v) => CallbackCommand::failure(
            CallbackToken::try_from(v.task_token).map_err(callback)?,
            v.error,
            v.cause,
        )
        .map_err(callback)?,
        Request::SendTaskHeartbeat(v) => {
            CallbackCommand::heartbeat(CallbackToken::try_from(v.task_token).map_err(callback)?)
        }
        _ => return Err(error(Code::UnsupportedOperation)),
    };
    Ok(Prepared::Callback(command, op))
}

fn history(
    v: input::GetExecutionHistoryInput,
    scope: &TenantScope,
) -> Result<Prepared, StepFunctionsError> {
    let arn = execution(&v.execution_arn, scope)?;
    Ok(Prepared::Page(
        Binding {
            op: Op::GetExecutionHistory,
            resource: arn.as_str().into(),
            flags: Flags::new(
                v.max_results,
                None,
                v.reverse_order.unwrap_or(false),
                v.include_execution_data.unwrap_or(true),
            )?,
        },
        token(v.next_token)?,
        RegistryCommand::Execution(arn),
    ))
}
fn create(
    v: input::CreateStateMachineInput,
    compiler: &AslCompiler,
    scope: &TenantScope,
) -> Result<RegistryCommand, StepFunctionsError> {
    profile(
        v.r#type.as_ref(),
        v.logging_configuration.as_ref(),
        v.tracing_configuration.as_ref(),
        v.encryption_configuration.as_ref(),
        v.publish,
        v.version_description.as_deref(),
    )?;
    Ok(RegistryCommand::Create(
        v.name.parse().map_err(|e| source(Code::InvalidName, e))?,
        definition(compiler, &v.definition)?,
        role(&v.role_arn, scope)?,
        tags(v.tags)?,
    ))
}
fn update(
    v: &input::UpdateStateMachineInput,
    compiler: &AslCompiler,
    scope: &TenantScope,
) -> Result<RegistryCommand, StepFunctionsError> {
    profile(
        None,
        v.logging_configuration.as_ref(),
        v.tracing_configuration.as_ref(),
        v.encryption_configuration.as_ref(),
        v.publish,
        v.version_description.as_deref(),
    )?;
    if v.definition.is_none() && v.role_arn.is_none() {
        return Err(error(Code::MissingRequiredParameter));
    }
    Ok(RegistryCommand::Update(
        machine(&v.state_machine_arn, scope)?,
        v.definition
            .as_deref()
            .map(|v| definition(compiler, v))
            .transpose()?,
        v.role_arn.as_deref().map(|v| role(v, scope)).transpose()?,
    ))
}
fn definition(
    compiler: &AslCompiler,
    text: &str,
) -> Result<PreparedDefinition, StepFunctionsError> {
    Ok(PreparedDefinition::new(
        compiler
            .compile(text.as_bytes())
            .map_err(|e| source(Code::InvalidDefinition, e))?
            .machine()
            .clone(),
    ))
}
fn role(text: &str, scope: &TenantScope) -> Result<RoleArn, StepFunctionsError> {
    let role: RoleArn = text.parse().map_err(|e| source(Code::InvalidArn, e))?;
    if !role.as_str().starts_with(&format!(
        "arn:{}:iam::{}:role/",
        scope.partition(),
        scope.account()
    )) {
        return Err(error(Code::InvalidArn));
    }
    Ok(role)
}
fn tags(values: Vec<Tag>) -> Result<Tags, StepFunctionsError> {
    Tags::new(
        values
            .into_iter()
            .map(|t| {
                Ok((
                    t.key.ok_or_else(|| error(Code::ValidationException))?,
                    t.value.unwrap_or_default(),
                ))
            })
            .collect::<Result<Vec<_>, StepFunctionsError>>()?,
    )
    .map_err(|e| source(Code::ValidationException, e))
}
fn scoped(text: &str, scope: &TenantScope) -> Result<(), StepFunctionsError> {
    if text.starts_with(&format!(
        "arn:{}:states:{}:{}:",
        scope.partition(),
        scope.region(),
        scope.account()
    )) {
        Ok(())
    } else {
        Err(error(Code::InvalidArn))
    }
}
fn machine(text: &str, scope: &TenantScope) -> Result<StateMachineArn, StepFunctionsError> {
    let value: StateMachineArn = text.parse().map_err(|e| source(Code::InvalidArn, e))?;
    scoped(value.as_str(), scope)?;
    Ok(value)
}
fn execution(text: &str, scope: &TenantScope) -> Result<ExecutionArn, StepFunctionsError> {
    let value: ExecutionArn = text.parse().map_err(|e| source(Code::InvalidArn, e))?;
    scoped(value.as_str(), scope)?;
    Ok(value)
}
fn token(value: Option<String>) -> Result<Option<String>, StepFunctionsError> {
    if value.as_ref().is_some_and(|t| {
        t.len() != 64
            || !t
                .bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
    }) {
        return Err(error(Code::InvalidToken));
    }
    Ok(value)
}
fn included(value: Option<&IncludedData>) -> Result<(), StepFunctionsError> {
    if value.is_some_and(|v| v != &IncludedData::AllData) {
        Err(error(Code::UnsupportedOperation))
    } else {
        Ok(())
    }
}
fn profile(
    kind: Option<&StateMachineType>,
    logging: Option<&LoggingConfiguration>,
    tracing: Option<&TracingConfiguration>,
    encryption: Option<&EncryptionConfiguration>,
    publish: Option<bool>,
    version: Option<&str>,
) -> Result<(), StepFunctionsError> {
    if kind.is_some_and(|v| v != &StateMachineType::Standard) {
        return Err(error(Code::StateMachineTypeNotSupported));
    }
    if publish == Some(true)
        || version.is_some()
        || logging.is_some_and(|v| {
            v.level.as_ref().is_some_and(|l| l != &LogLevel::Off) || !v.destinations.is_empty()
        })
        || tracing.is_some_and(|v| v.enabled == Some(true))
        || encryption.is_some_and(|v| {
            v.r#type != EncryptionType::AwsOwnedKey
                || v.kms_key_id.is_some()
                || v.kms_data_key_reuse_period_seconds.is_some()
        })
    {
        return Err(error(Code::ValidationException));
    }
    Ok(())
}
fn validate(
    v: &input::ValidateStateMachineDefinitionInput,
    compiler: &AslCompiler,
) -> Result<output::ValidateStateMachineDefinitionOutput, StepFunctionsError> {
    profile(v.r#type.as_ref(), None, None, None, None, None)?;
    let limit = match v.max_results {
        None | Some(0) => 100,
        Some(n) => usize::try_from(n)
            .ok()
            .filter(|n| *n <= 100)
            .ok_or_else(|| error(Code::ValidationException))?,
    };
    let mut response = output::ValidateStateMachineDefinitionOutput::default();
    let diagnostics = match compiler.compile(v.definition.as_bytes()) {
        Ok(artifact) => {
            response.result = ValidateStateMachineDefinitionResultCode::Ok;
            artifact.warnings().to_vec()
        }
        Err(error) => {
            response.result = ValidateStateMachineDefinitionResultCode::Fail;
            vec![error.diagnostic().clone()]
        }
    };
    let warnings = v.severity == Some(ValidateStateMachineDefinitionSeverity::Warning);
    let mut diagnostics: Vec<_> = diagnostics
        .iter()
        .filter(|d| warnings || d.severity() == DiagnosticSeverity::Error)
        .map(|d| {
            let mut out = ValidateStateMachineDefinitionDiagnostic::default();
            out.code = d.code().into();
            out.message = d.message().into();
            out.location = Some(d.location().into());
            out.severity = if d.severity() == DiagnosticSeverity::Error {
                ValidateStateMachineDefinitionSeverity::Error
            } else {
                ValidateStateMachineDefinitionSeverity::Warning
            };
            out
        })
        .collect();
    response.truncated = Some(diagnostics.len() > limit);
    diagnostics.truncate(limit);
    response.diagnostics = diagnostics;
    Ok(response)
}
