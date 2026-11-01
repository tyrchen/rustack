//! Validated per-attempt plans and bounded direct/optimized result mapping.

use std::{
    fmt,
    io::{Error as IoError, Result as IoResult, Write},
    str,
    sync::Arc,
};

use rustack_core::json::JsonError;
use rustack_lambda_core::{
    error::LambdaServiceError,
    executor::{ExecutorError, InvokeResponse},
};
use rustack_stepfunctions_core::{
    AslError, CallbackError, CallbackInvocationMetadata, Integration, LambdaTaskCommand,
    MAX_PAYLOAD_BYTES, Payload, StateView, TaskFailure, TenantScope, UuidSource,
};
use serde_json::{Value, json};
use tokio::time::Instant;

use super::BridgeError;

const CAUSE_LIMIT: usize = 32_768;
const TRUNCATION_MARKER: &str = " [truncated]";

struct CauseWriter {
    prefix: Vec<u8>,
    truncated: bool,
}
impl fmt::Debug for CauseWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LambdaCauseWriter(<redacted>)")
    }
}
impl Write for CauseWriter {
    fn write(&mut self, bytes: &[u8]) -> IoResult<usize> {
        let count = CAUSE_LIMIT
            .saturating_sub(self.prefix.len())
            .min(bytes.len());
        self.prefix.extend_from_slice(
            bytes
                .get(..count)
                .ok_or_else(|| IoError::other("invalid Cause prefix"))?,
        );
        self.truncated |= count < bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

pub(super) struct CallPlan {
    pub(super) function: String,
    pub(super) qualifier: Option<String>,
    pub(super) payload: Payload,
    pub(super) mode: ResponseMode,
    pub(super) deadline: Instant,
}
#[derive(Debug, Clone, Copy)]
pub(super) enum ResponseMode {
    Direct,
    Optimized,
    ObservedCallback,
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
            Integration::LambdaInvoke | Integration::LambdaCallback(_) => {
                if matches!(resource, Integration::LambdaCallback(_))
                    && !command.is_observed_callback()
                {
                    return Err(BridgeError::Parameters);
                }
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
                validate_reference(&function, command.machine().scope())?;
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
                let payload = match object.get("Payload") {
                    None | Some(Value::Null) => Payload::parse(b"{}")?,
                    Some(Value::String(text)) => Payload::parse(text.as_bytes())?,
                    Some(value) => Payload::from_value(value)?,
                };
                (function, qualifier, payload, true)
            }
            _ => return Err(BridgeError::Parameters),
        };
        Ok(Self {
            function,
            qualifier,
            payload,
            mode: if command.is_observed_callback() {
                if !optimized {
                    return Err(BridgeError::Parameters);
                }
                ResponseMode::ObservedCallback
            } else if optimized {
                ResponseMode::Optimized
            } else {
                ResponseMode::Direct
            },
            deadline: Instant::from_std(call.deadline()),
        })
    }
}
pub(super) struct ServiceTarget {
    function: String,
    qualifier: Option<String>,
    scope: TenantScope,
}
impl fmt::Debug for ServiceTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LambdaServiceTarget(<redacted>)")
    }
}
impl ServiceTarget {
    pub(super) fn new(plan: &CallPlan, scope: &TenantScope) -> Self {
        Self {
            function: plan.function.clone(),
            qualifier: plan.qualifier.clone(),
            scope: scope.clone(),
        }
    }
    fn qualified_arn(&self) -> Result<String, BridgeError> {
        validate_reference(&self.function, &self.scope)?;
        let prefix = format!(
            "arn:{}:lambda:{}:{}:function:",
            self.scope.partition(),
            self.scope.region(),
            self.scope.account()
        );
        let reference = self
            .function
            .strip_prefix(&prefix)
            .unwrap_or(&self.function);
        let mut parts = reference.split(':');
        let name = parts.next().ok_or(BridgeError::Parameters)?;
        let embedded = parts.next();
        if parts.next().is_some()
            || embedded
                .zip(self.qualifier.as_deref())
                .is_some_and(|(left, right)| left != right)
        {
            return Err(BridgeError::Parameters);
        }
        let qualifier = embedded.or(self.qualifier.as_deref()).unwrap_or("$LATEST");
        if qualifier.is_empty()
            || qualifier.len() > 128
            || !qualifier
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'$'))
        {
            return Err(BridgeError::Parameters);
        }
        let arn = format!("{prefix}{name}:{qualifier}");
        if arn.len() > 512 {
            return Err(BridgeError::Parameters);
        }
        Ok(arn)
    }
}

fn validate_reference(reference: &str, scope: &TenantScope) -> Result<(), BridgeError> {
    let valid_name = |name: &str| {
        !name.is_empty()
            && name.len() <= 64
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    };
    if reference.starts_with("arn:") {
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
        return failure(error, &bounded_cause(payload.value())?);
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
pub(super) fn observed_response(
    invocation: &InvokeResponse,
    metadata: Result<CallbackInvocationMetadata, CallbackError>,
) -> Result<(Mapped, Option<CallbackInvocationMetadata>), BridgeError> {
    // FunctionError keeps the ordinary bounded failure path. Successful callback
    // invocations never parse or retain the synchronous function business body.
    if invocation.status != 200 || invocation.function_error.is_some() {
        return response(invocation, false, "").map(|mapped| (mapped, None));
    }
    let metadata = match metadata {
        Ok(metadata) => metadata,
        Err(error) => {
            let mut mapped = failure(
                "Lambda.ServiceException",
                "Invalid Lambda invocation metadata",
            )?;
            mapped.diagnostic = Some(BridgeError::Callback(Arc::new(error)));
            return Ok((mapped, None));
        }
    };
    let output = metadata.payload()?;
    Ok((
        Mapped {
            result: Ok(output),
            diagnostic: None,
        },
        Some(metadata),
    ))
}
fn bounded_cause(value: &Value) -> Result<String, BridgeError> {
    let mut writer = CauseWriter {
        prefix: Vec::with_capacity(CAUSE_LIMIT),
        truncated: false,
    };
    serde_json::to_writer(&mut writer, value).map_err(AslError::Encoding)?;
    if writer.truncated {
        writer
            .prefix
            .truncate(CAUSE_LIMIT.saturating_sub(TRUNCATION_MARKER.len()));
    }
    let prefix = match str::from_utf8(&writer.prefix) {
        Ok(prefix) => prefix,
        Err(error) if writer.truncated && error.error_len().is_none() => str::from_utf8(
            writer
                .prefix
                .get(..error.valid_up_to())
                .ok_or(BridgeError::Parameters)?,
        )
        .map_err(|_| BridgeError::Parameters)?,
        Err(_) => return Err(BridgeError::Parameters),
    };
    let mut cause = prefix.to_owned();
    if writer.truncated {
        cause.push_str(TRUNCATION_MARKER);
    }
    Ok(cause)
}
pub(super) fn service<S: UuidSource + ?Sized>(
    error: LambdaServiceError,
    target: &ServiceTarget,
    source: &mut S,
) -> Result<Mapped, BridgeError> {
    let name = match &error {
        #[cfg(unix)]
        LambdaServiceError::ImageRestore { .. } | LambdaServiceError::ImageRestoreWorker { .. } => {
            "Lambda.ServiceException"
        }
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
    let mut mapped = if matches!(
        &error,
        LambdaServiceError::FunctionNotFound { .. }
            | LambdaServiceError::VersionNotFound { .. }
            | LambdaServiceError::AliasNotFound { .. }
    ) {
        let arn = target.qualified_arn()?;
        let id = source.next_uuid()?;
        // The RFC 4122/9562 variant has leading bits 10 in the ninth byte.
        if id.get_version_num() != 4 || id.as_bytes().get(8).is_none_or(|byte| byte & 0xc0 != 0x80)
        {
            return Err(BridgeError::Parameters);
        }
        failure(
            name,
            &format!(
                "Function not found: {arn} (Service: AWSLambda; Status Code: 404; Error Code: \
                 ResourceNotFoundException; Request ID: {id}; Proxy: null)"
            ),
        )?
    } else {
        failure(name, "Lambda invocation admission failed")?
    };
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
