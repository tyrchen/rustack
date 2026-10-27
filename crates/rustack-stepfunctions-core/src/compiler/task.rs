//! Scoped static Lambda resource validation; no invocation or alias resolution.

use std::fmt;

use serde_json::{Map, Value};

use super::{AslCompiler, Code, CompileError, Location, TaskTimeout};
use crate::{AslError, ExecutionName, Payload, StateMachineArn};

/// Adapter support supplied by trusted composition, never a request field.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum TaskSupport {
    /// Compile pure workflows only.
    #[default]
    Disabled,
    /// A real Lambda adapter is installed by the owner.
    Lambda,
}

/// Validated partition/region/account used for static resource scope checks.
#[derive(Clone)]
#[non_exhaustive]
pub struct TenantScope {
    partition: String,
    region: String,
    account: String,
}
impl fmt::Debug for TenantScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TenantScope(<redacted>)")
    }
}
impl Default for TenantScope {
    fn default() -> Self {
        Self {
            partition: "aws".into(),
            region: "us-east-1".into(),
            account: "000000000000".into(),
        }
    }
}
impl TenantScope {
    /// Validate operator identity before storing or using it in resource checks.
    ///
    /// # Errors
    /// Rejects unsupported partitions, malformed regions/accounts and byte limits.
    ///
    /// ```
    /// let scope=rustack_stepfunctions_core::TenantScope::new("aws","us-east-1","000000000000")?;
    /// # Ok::<(),rustack_stepfunctions_core::AslError>(())
    /// ```
    pub fn new(partition: &str, region: &str, account: &str) -> Result<Self, AslError> {
        let region_valid = !region.is_empty()
            && region.len() <= 64
            && region
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && region
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_lowercase)
            && region
                .as_bytes()
                .last()
                .is_some_and(u8::is_ascii_alphanumeric);
        if !matches!(partition, "aws" | "aws-cn" | "aws-us-gov")
            || !region_valid
            || account.len() != 12
            || !account.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(AslError::InvalidScope);
        }
        Ok(Self {
            partition: partition.into(),
            region: region.into(),
            account: account.into(),
        })
    }
    read!(TenantScope,partition,&str,s=>s.partition.as_str(),"Read the validated partition.");
    read!(TenantScope,region,&str,s=>s.region.as_str(),"Read the validated region.");
    read!(TenantScope,account,&str,s=>s.account.as_str(),"Read the validated account.");
}

/// Static same-scope function ARN; resolution remains per invocation attempt.
#[derive(Clone)]
#[non_exhaustive]
pub struct FunctionArn {
    value: String,
    qualifier: Option<String>,
}
impl fmt::Debug for FunctionArn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FunctionArn(<redacted>)")
    }
}
impl FunctionArn {
    read!(FunctionArn,as_str,&str,s=>s.value.as_str(),"Read the validated function ARN.");
    read!(FunctionArn,qualifier,Option<&str>,s=>s.qualifier.as_deref(),"Read an optional unresolved qualifier.");
    fn parse(value: &str, scope: &TenantScope, location: &Location) -> Result<Self, CompileError> {
        if value.len() > 256 {
            return Err(invalid(location));
        }
        let mut parts = value.split(':');
        if parts.next() != Some("arn")
            || parts.next() != Some(scope.partition())
            || parts.next() != Some("lambda")
            || parts.next() != Some(scope.region())
            || parts.next() != Some(scope.account())
            || parts.next() != Some("function")
        {
            return Err(invalid(location));
        }
        let name = parts
            .next()
            .filter(|n| name(n))
            .ok_or_else(|| invalid(location))?;
        let qualifier = parts.next();
        if parts.next().is_some()
            || qualifier.is_some_and(|q| !valid_qualifier(q))
            || name.is_empty()
        {
            return Err(invalid(location));
        }
        Ok(Self {
            value: value.into(),
            qualifier: qualifier.map(str::to_owned),
        })
    }
}

/// Optimized nested integration behavior selected by the trusted caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ChildPattern {
    /// Detach at successful start acceptance.
    RequestResponse,
    /// Await termination; return Input/Output as serialized JSON strings.
    Sync,
    /// Await termination; return Input/Output as JSON values.
    Sync2,
}
impl ChildPattern {
    pub(crate) fn owned(self) -> bool {
        match self {
            Self::RequestResponse => false,
            Self::Sync | Self::Sync2 => true,
        }
    }
}
/// Fully allowlisted M1 Lambda task pattern.
#[derive(Clone)]
#[non_exhaustive]
pub enum Integration {
    /// Direct request-response function result.
    DirectLambda(FunctionArn),
    /// Optimized request-response result envelope.
    LambdaInvoke,
    /// Deferred optimized callback invocation with an optional heartbeat source.
    LambdaCallback(TaskTimeout),
    /// Start or await an execution through the installed registry.
    StartExecution(ChildPattern),
}
impl fmt::Debug for Integration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Integration(<redacted>)")
    }
}

fn invalid(location: &Location) -> CompileError {
    location.error(
        Code::InvalidResource,
        "invalid or unsupported scoped Lambda resource",
    )
}
fn name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}
fn valid_qualifier(value: &str) -> bool {
    value == "$LATEST"
        || (!value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-')))
}

pub(super) fn compile(
    value: &str,
    parameters: Option<&Value>,
    compiler: &AslCompiler,
    heartbeat: TaskTimeout,
    location: &Location,
) -> Result<Integration, CompileError> {
    let scope = &compiler.scope;
    let prefix = format!("arn:{}:states:::", scope.partition());
    let integration = if value == format!("{prefix}lambda:invoke") {
        optimized(parameters, scope, &location.field("Parameters")?)?;
        Integration::LambdaInvoke
    } else if let Some(pattern) = [
        ("states:startExecution", ChildPattern::RequestResponse),
        ("states:startExecution.sync", ChildPattern::Sync),
        ("states:startExecution.sync:2", ChildPattern::Sync2),
    ]
    .into_iter()
    .find_map(|(suffix, pattern)| (value == format!("{prefix}{suffix}")).then_some(pattern))
    {
        if !compiler.children {
            return Err(location.field("Resource")?.error(
                Code::ProfileUnavailable,
                "child execution adapter is unavailable",
            ));
        }
        child_parameters(parameters, scope, &location.field("Parameters")?)?;
        return Ok(Integration::StartExecution(pattern));
    } else if value == format!("{prefix}lambda:invoke.waitForTaskToken") {
        if !compiler.callbacks {
            return Err(location.field("Resource")?.error(
                Code::ProfileUnavailable,
                "callback effect support is unavailable",
            ));
        }
        optimized(parameters, scope, &location.field("Parameters")?)?;
        Integration::LambdaCallback(heartbeat)
    } else {
        Integration::DirectLambda(FunctionArn::parse(
            value,
            scope,
            &location.field("Resource")?,
        )?)
    };
    if compiler.tasks == TaskSupport::Disabled {
        return Err(location.field("Resource")?.error(
            Code::DependencyDisabled,
            "Lambda task adapter is unavailable",
        ));
    }
    Ok(integration)
}

fn optimized(
    parameters: Option<&Value>,
    scope: &TenantScope,
    location: &Location,
) -> Result<(), CompileError> {
    let Some(parameters) = parameters else {
        return Ok(());
    };
    let fields = parameters.as_object().ok_or_else(|| {
        location.error(
            Code::InvalidSchema,
            "optimized parameters must be an object",
        )
    })?;
    for key in fields.keys() {
        let key = key.strip_suffix(".$").unwrap_or(key);
        if !matches!(
            key,
            "FunctionName" | "Payload" | "Qualifier" | "InvocationType"
        ) {
            return Err(location.error(
                Code::UnsupportedField,
                "unsupported optimized Lambda parameter",
            ));
        }
    }
    let embedded = if let Some(value) = fields.get("FunctionName") {
        let value = value.as_str().ok_or_else(|| invalid(location))?;
        if value.starts_with("arn:") {
            FunctionArn::parse(value, scope, location)?.qualifier
        } else {
            if !name(value) {
                return Err(invalid(location));
            }
            None
        }
    } else {
        None
    };
    let qualifier = static_string(fields, "Qualifier", location)?;
    if qualifier.is_some_and(|value| !valid_qualifier(value))
        || embedded
            .as_deref()
            .zip(qualifier)
            .is_some_and(|(a, b)| a != b)
    {
        return Err(invalid(location));
    }
    if static_string(fields, "InvocationType", location)?
        .is_some_and(|value| value != "RequestResponse")
    {
        return Err(location.error(
            Code::ProfileUnavailable,
            "only request-response Lambda invocation is supported",
        ));
    }
    Ok(())
}
fn static_string<'a>(
    fields: &'a Map<String, Value>,
    key: &str,
    location: &Location,
) -> Result<Option<&'a str>, CompileError> {
    fields
        .get(key)
        .map(|v| v.as_str().ok_or_else(|| invalid(location)))
        .transpose()
}

fn child_parameters(
    parameters: Option<&Value>,
    scope: &TenantScope,
    location: &Location,
) -> Result<(), CompileError> {
    let Some(parameters) = parameters else {
        return Ok(());
    };
    let fields = parameters
        .as_object()
        .ok_or_else(|| location.error(Code::InvalidSchema, "child parameters must be an object"))?;
    for key in fields.keys() {
        if !matches!(
            key.strip_suffix(".$").unwrap_or(key),
            "StateMachineArn" | "Input" | "Name"
        ) {
            return Err(location.error(
                Code::UnsupportedField,
                "unsupported child execution parameter",
            ));
        }
    }
    if let Some(value) = fields.get("StateMachineArn") {
        let arn: StateMachineArn = value
            .as_str()
            .ok_or_else(|| invalid(location))?
            .parse()
            .map_err(|_| invalid(location))?;
        arn.check(scope).map_err(|_| invalid(location))?;
    }
    if let Some(value) = fields.get("Name") {
        let _: ExecutionName = value
            .as_str()
            .ok_or_else(|| invalid(location))?
            .parse()
            .map_err(|_| invalid(location))?;
    }
    if let Some(Value::String(input)) = fields.get("Input") {
        Payload::parse(input.as_bytes()).map_err(|e| CompileError::asl(location, e))?;
    }
    Ok(())
}
impl ChildPattern {
    pub(crate) fn action(self) -> &'static str {
        match self {
            Self::RequestResponse => "startExecution",
            Self::Sync => "startExecution.sync",
            Self::Sync2 => "startExecution.sync:2",
        }
    }
}
