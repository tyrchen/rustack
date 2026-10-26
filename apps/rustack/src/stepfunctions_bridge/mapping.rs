//! Validated per-attempt plans and bounded direct/optimized result mapping.

use std::{fmt, str, sync::Arc};

use rustack_core::json::JsonError;
use rustack_lambda_core::{
    error::LambdaServiceError,
    executor::{ExecutorError, InvokeResponse},
};
use rustack_stepfunctions_core::{
    AslError, Integration, LambdaTaskCommand, MAX_PAYLOAD_BYTES, Payload, StateView, TaskFailure,
};
use serde_json::{Value, json};
use tokio::time::Instant;

use super::BridgeError;

pub(super) struct CallPlan {
    pub(super) function: String,
    pub(super) qualifier: Option<String>,
    pub(super) payload: Payload,
    pub(super) optimized: bool,
    pub(super) deadline: Instant,
}
impl fmt::Debug for CallPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LambdaCallPlan(<redacted>)")
    }
}
#[derive(Debug)]
pub(super) struct Mapped {
    pub(super) result: Result<Payload, TaskFailure>,
    pub(super) diagnostic: Option<BridgeError>,
}
impl CallPlan {
    pub(super) fn compile(command: &LambdaTaskCommand) -> Result<Self, BridgeError> {
        let call = command.dispatch();
        let StateView::Task { resource, .. } = command.machine().state(call.state())? else {
            return Err(BridgeError::Parameters);
        };
        let (function, qualifier, payload, optimized) = match resource {
            Integration::DirectLambda(arn) => {
                (arn.as_str().into(), None, call.parameters().clone(), false)
            }
            Integration::LambdaInvoke => {
                let object = call
                    .parameters()
                    .value()
                    .as_object()
                    .ok_or(BridgeError::Parameters)?;
                if object.keys().any(|key| {
                    !matches!(
                        key.as_str(),
                        "FunctionName" | "Payload" | "Qualifier" | "InvocationType"
                    )
                }) {
                    return Err(BridgeError::Parameters);
                }
                let function = object
                    .get("FunctionName")
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty() && v.len() <= 256)
                    .ok_or(BridgeError::Parameters)?
                    .to_owned();
                validate_reference(&function, command)?;
                let qualifier = object
                    .get("Qualifier")
                    .map(|v| {
                        v.as_str()
                            .filter(|q| !q.is_empty() && q.len() <= 128)
                            .map(str::to_owned)
                            .ok_or(BridgeError::Parameters)
                    })
                    .transpose()?;
                if object
                    .get("InvocationType")
                    .is_some_and(|v| v.as_str() != Some("RequestResponse"))
                {
                    return Err(BridgeError::Parameters);
                }
                let payload = object
                    .get("Payload")
                    .map(Payload::from_value)
                    .transpose()?
                    .unwrap_or(Payload::parse(b"{}")?);
                (function, qualifier, payload, true)
            }
            _ => return Err(BridgeError::Parameters),
        };
        Ok(Self {
            function,
            qualifier,
            payload,
            optimized,
            deadline: Instant::from_std(call.deadline()),
        })
    }
}
fn validate_reference(reference: &str, command: &LambdaTaskCommand) -> Result<(), BridgeError> {
    let valid_name = |name: &str| {
        !name.is_empty()
            && name.len() <= 64
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    };
    if reference.starts_with("arn:") {
        let scope = command.machine().scope();
        let prefix = format!(
            "arn:{}:lambda:{}:{}:function:",
            scope.partition(),
            scope.region(),
            scope.account()
        );
        let suffix = reference
            .strip_prefix(&prefix)
            .ok_or(BridgeError::Parameters)?;
        let mut parts = suffix.split(':');
        let name = parts.next().ok_or(BridgeError::Parameters)?;
        if !valid_name(name)
            || parts.next().is_some_and(|q| {
                q.is_empty()
                    || q.len() > 128
                    || !q
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'$'))
            })
            || parts.next().is_some()
        {
            return Err(BridgeError::Parameters);
        }
    } else if !valid_name(reference) {
        return Err(BridgeError::Parameters);
    }
    Ok(())
}
fn failure(name: &str, cause: &str) -> Result<Mapped, BridgeError> {
    Ok(Mapped {
        result: Err(TaskFailure::new(name, Some(cause))?),
        diagnostic: None,
    })
}
pub(super) fn parameters(error: BridgeError) -> Result<Mapped, BridgeError> {
    let mut mapped = failure("States.Runtime", "Invalid evaluated Lambda task parameters")?;
    mapped.diagnostic = Some(error);
    Ok(mapped)
}
pub(super) fn response(
    response: &InvokeResponse,
    optimized: bool,
    request_id: &str,
) -> Result<Mapped, BridgeError> {
    if response.status != 200 {
        return failure(
            "Lambda.ServiceException",
            "Unexpected Lambda response status",
        );
    }
    if response.payload.len() > MAX_PAYLOAD_BYTES {
        return failure(
            "States.DataLimitExceeded",
            "Lambda result exceeds the ASL payload limit",
        );
    }
    let payload = match Payload::parse(&response.payload) {
        Ok(payload) => payload,
        Err(error) => {
            let name = if response.function_error.is_some() {
                "Lambda.Unknown"
            } else {
                "States.Runtime"
            };
            let mut mapped = failure(name, "Lambda returned invalid JSON")?;
            mapped.diagnostic = Some(error.into());
            return Ok(mapped);
        }
    };
    if response.function_error.is_some() {
        let error = payload
            .value()
            .get("errorType")
            .and_then(Value::as_str)
            .filter(|name| {
                !name.is_empty()
                    && name.len() <= 256
                    && !name.starts_with("States.")
                    && !name.chars().any(char::is_control)
            })
            .unwrap_or("Lambda.Unknown");
        let cause = str::from_utf8(&response.payload).map_err(|_| BridgeError::Parameters)?;
        return failure(error, &bounded_cause(cause)?);
    }
    if !optimized {
        return Ok(Mapped {
            result: Ok(payload),
            diagnostic: None,
        });
    }
    if response.executed_version.is_empty()
        || response.executed_version.len() > 128
        || !response
            .executed_version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'$'))
    {
        return failure("Lambda.ServiceException", "Invalid Lambda version metadata");
    }
    let envelope = json!({"Payload":payload.value(),"StatusCode":200,"ExecutedVersion":response.executed_version,
        "SdkHttpMetadata":{"HttpStatusCode":200},"SdkResponseMetadata":{"RequestId":request_id}});
    match Payload::from_value(&envelope) {
        Ok(payload) => Ok(Mapped {
            result: Ok(payload),
            diagnostic: None,
        }),
        Err(error) => {
            let name = match &error {
                AslError::DataLimit
                | AslError::Json(JsonError::Bytes | JsonError::String | JsonError::Key) => {
                    "States.DataLimitExceeded"
                }
                AslError::Encoding(_) | AslError::Allocation(_) | AslError::Entropy(_) => {
                    "Lambda.ServiceException"
                }
                _ => "States.Runtime",
            };
            let mut mapped = failure(
                name,
                "Lambda result envelope exceeds the ASL payload profile",
            )?;
            mapped.diagnostic = Some(error.into());
            Ok(mapped)
        }
    }
}
fn bounded_cause(cause: &str) -> Result<String, BridgeError> {
    const MARKER: &str = " [truncated]";
    const MAX: usize = 32_768;
    if cause.len() <= MAX {
        return Ok(cause.into());
    }
    let mut end = MAX.saturating_sub(MARKER.len());
    while !cause.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    let mut output = cause.get(..end).ok_or(BridgeError::Parameters)?.to_owned();
    output.push_str(MARKER);
    Ok(output)
}
pub(super) fn service(error: LambdaServiceError) -> Result<Mapped, BridgeError> {
    let name = match &error {
        LambdaServiceError::TooManyRequests => "Lambda.TooManyRequestsException",
        LambdaServiceError::FunctionNotFound { .. }
        | LambdaServiceError::VersionNotFound { .. }
        | LambdaServiceError::AliasNotFound { .. }
        | LambdaServiceError::EventSourceMappingNotFound { .. }
        | LambdaServiceError::PolicyNotFound { .. }
        | LambdaServiceError::EventInvokeConfigNotFound { .. } => {
            "Lambda.ResourceNotFoundException"
        }
        LambdaServiceError::ResourceConflict { .. } => "Lambda.ResourceConflictException",
        LambdaServiceError::PreconditionFailed => "Lambda.PreconditionFailedException",
        LambdaServiceError::InvalidParameter { .. }
        | LambdaServiceError::InvalidZipFile { .. }
        | LambdaServiceError::InvalidArn { .. } => "Lambda.InvalidParameterValueException",
        LambdaServiceError::ResourceNotReady { .. } => "Lambda.ResourceNotReadyException",
        LambdaServiceError::RequestTooLarge { .. } => "Lambda.RequestTooLargeException",
        LambdaServiceError::ArtifactIo { .. }
        | LambdaServiceError::DockerNotAvailable
        | LambdaServiceError::Internal { .. } => "Lambda.ServiceException",
    };
    let mut mapped = failure(name, "Lambda invocation admission failed")?;
    mapped.diagnostic = Some(BridgeError::Service(Arc::new(error)));
    Ok(mapped)
}
pub(super) fn executor(error: ExecutorError) -> Result<Mapped, BridgeError> {
    let name = match &error {
        ExecutorError::Timeout(_) => "Sandbox.Timedout",
        ExecutorError::Unsupported(_)
        | ExecutorError::InvalidCode(_)
        | ExecutorError::InitFailed(_) => "Runtime.InvalidEntrypoint",
        ExecutorError::RuntimeExited(_) => "Runtime.ExitError",
        _ => "Lambda.ServiceException",
    };
    let mut mapped = failure(name, "Lambda execution failed")?;
    mapped.diagnostic = Some(BridgeError::Executor(Arc::new(error)));
    Ok(mapped)
}
