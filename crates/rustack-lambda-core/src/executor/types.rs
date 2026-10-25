//! Shared executor request/response/config types.

use std::{collections::HashMap, fmt, str::FromStr, time::Duration};

use bytes::Bytes;
use rustack_lambda_model::types::ImageConfig;
use typed_builder::TypedBuilder;

#[cfg(test)]
use crate::artifact::ValidatedArtifact;
use crate::{
    artifact::{Architecture, DeploymentArtifact},
    identity::FunctionResourceId,
};

/// Selects which execution backend the provider should construct at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExecutorBackend {
    /// Metadata-only operation: execution fails explicitly and starts no process.
    #[default]
    Disabled,
    /// Pick the most appropriate backend per invocation: Squib for macOS Zip
    /// functions and native execution otherwise.
    Auto,
    /// Always native; reject invocations that can't run on the host.
    Native,
    /// Always Docker.
    Docker,
    /// Always Squib microVM execution.
    Squib,
}

impl FromStr for ExecutorBackend {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "disabled" | "off" | "noop" => Ok(Self::Disabled),
            "auto" => Ok(Self::Auto),
            "native" | "process" => Ok(Self::Native),
            "docker" | "container" => Ok(Self::Docker),
            "squib" | "microvm" => Ok(Self::Squib),
            other => Err(format!("unknown LAMBDA_EXECUTOR value: {other}")),
        }
    }
}

/// Everything an executor needs to run one invocation.
#[derive(Clone, TypedBuilder)]
#[non_exhaustive]
pub struct InvokeRequest {
    /// Function lifetime identity; deletion/recreation never reuses it.
    pub resource_id: FunctionResourceId,
    /// Captured configuration revision, independent of the invoked alias.
    pub revision_id: String,
    /// Complete Docker entry configuration; omitted and empty argv differ.
    pub image_config: Option<ImageConfig>,
    /// Ephemeral storage size in MiB, captured with this invocation's revision.
    pub ephemeral_storage_mb: u32,
    /// Qualified ARN — used for `Lambda-Runtime-Invoked-Function-Arn`.
    pub function_arn: String,
    /// Bare function name.
    pub function_name: String,
    /// Resolved version string, e.g. `"$LATEST"` or `"3"`.
    pub qualifier: String,
    /// Lambda runtime identifier, e.g. `"provided.al2023"`.
    pub runtime: Option<String>,
    /// Handler string from the function config.
    pub handler: Option<String>,
    /// Validated single Lambda architecture.
    pub architecture: Architecture,
    /// Exactly one immutable deployment artifact, with lifetime ownership.
    pub artifact: DeploymentArtifact,
    /// User-supplied environment variables.
    pub environment: HashMap<String, String>,
    /// Function timeout. Excess => `Sandbox.Timedout`.
    pub timeout: Duration,
    /// Memory size in MB (used as the container memory cap).
    pub memory_mb: u32,
    /// Raw invocation payload (typically JSON, but not required).
    pub payload: Bytes,
    /// When set, the executor captures the last 4KB of stderr as `log_tail`.
    pub capture_logs: bool,
}

impl fmt::Debug for InvokeRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InvokeRequest")
            .field("resource_id", &self.resource_id)
            .field("revision_id", &self.revision_id)
            .field("function_arn", &self.function_arn)
            .field("qualifier", &self.qualifier)
            .field("artifact", &self.artifact)
            .field("architecture", &self.architecture)
            .field("environment", &"[REDACTED]")
            .field("image_config", &"[REDACTED]")
            .field("payload", &"[REDACTED]")
            .field("timeout", &self.timeout)
            .field("memory_mb", &self.memory_mb)
            .field("ephemeral_storage_mb", &self.ephemeral_storage_mb)
            .field("capture_logs", &self.capture_logs)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
impl InvokeRequest {
    pub(crate) fn fixture() -> Self {
        Self::builder()
            .resource_id(FunctionResourceId::default())
            .revision_id("fixture-revision".into())
            .image_config(None)
            .ephemeral_storage_mb(512)
            .function_arn("arn:aws:lambda:us-east-1:000000000000:function:fixture".into())
            .function_name("fixture".into())
            .qualifier("$LATEST".into())
            .runtime(Some("provided.al2023".into()))
            .handler(Some("bootstrap".into()))
            .architecture(Architecture::Arm64)
            .artifact(DeploymentArtifact::Zip(ValidatedArtifact::fixture()))
            .environment(HashMap::new())
            .timeout(Duration::from_secs(3))
            .memory_mb(128)
            .payload(Bytes::new())
            .capture_logs(false)
            .build()
    }
}

/// Outcome of a successful round-trip with the user code (which may itself
/// have reported a function error).
#[derive(Clone)]
pub struct InvokeResponse {
    /// HTTP status to mirror back to the caller (typically 200).
    pub status: u16,
    /// Response body produced by the bootstrap.
    pub payload: Bytes,
    /// `Some("Unhandled")` when the bootstrap posted to `/error`.
    pub function_error: Option<String>,
    /// Base64-encoded last 4KB of stderr, if `capture_logs` was set.
    pub log_tail: Option<String>,
    /// What the executor actually invoked (echoes back the qualifier).
    pub executed_version: String,
}

impl fmt::Debug for InvokeResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InvokeResponse")
            .field("status", &self.status)
            .field("function_error", &self.function_error.is_some())
            .field("executed_version", &self.executed_version)
            .field("payload", &"[REDACTED]")
            .field("log_tail", &"[REDACTED]")
            .finish()
    }
}

impl InvokeResponse {
    /// Convenience constructor for a plain JSON success response.
    #[must_use]
    pub fn success(payload: Bytes, executed_version: String) -> Self {
        Self {
            status: 200,
            payload,
            function_error: None,
            log_tail: None,
            executed_version,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_redact_invocation_request_and_response_debug() {
        let secret = "fixture-secret-should-never-be-rendered";
        let mut request = InvokeRequest::fixture();
        request.environment.insert("SECRET".into(), secret.into());
        request.payload = Bytes::from(secret);
        request.image_config = Some(ImageConfig {
            entry_point: Some(vec![secret.into()]),
            command: Some(vec![secret.into()]),
            working_directory: Some(format!("/{secret}")),
        });
        let mut response = InvokeResponse::success(Bytes::from(secret), "1".into());
        response.function_error = Some(secret.into());
        response.log_tail = Some(secret.into());
        for rendered in [format!("{request:?}"), format!("{response:?}")] {
            assert!(!rendered.contains(secret));
            assert!(rendered.contains("[REDACTED]"));
        }
    }

    #[test]
    fn test_should_parse_executor_backend_strings() {
        assert_eq!(
            "disabled".parse::<ExecutorBackend>().unwrap(),
            ExecutorBackend::Disabled
        );
        assert_eq!(
            "Auto".parse::<ExecutorBackend>().unwrap(),
            ExecutorBackend::Auto
        );
        assert_eq!(
            "native".parse::<ExecutorBackend>().unwrap(),
            ExecutorBackend::Native
        );
        assert_eq!(
            "DOCKER".parse::<ExecutorBackend>().unwrap(),
            ExecutorBackend::Docker
        );
        assert_eq!(
            "squib".parse::<ExecutorBackend>().unwrap(),
            ExecutorBackend::Squib
        );
        assert_eq!(
            "microvm".parse::<ExecutorBackend>().unwrap(),
            ExecutorBackend::Squib
        );
        assert!("nope".parse::<ExecutorBackend>().is_err());
    }
}
