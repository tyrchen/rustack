//! Shared executor request/response/config types.

use std::{collections::HashMap, fmt, str::FromStr, sync::Arc, time::Duration};

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

/// Validated function configuration captured before invocation admission.
///
/// Requests share this value through Arc; updates create another captured revision.
#[derive(Clone, TypedBuilder)]
#[non_exhaustive]
pub struct ExecutionRevision {
    /// Stable function lifetime identity.
    pub resource_id: FunctionResourceId,
    /// Captured configuration revision identifier.
    pub revision_id: String,
    /// Backend selection policy captured at admission.
    pub backend: ExecutorBackend,
    /// Bare function name.
    pub function_name: String,
    /// Resolved numeric version or `$LATEST`, never an invoked alias.
    pub qualifier: String,
    /// Runtime identifier for ZIP code.
    pub runtime: Option<String>,
    /// Configured ZIP handler.
    pub handler: Option<String>,
    /// One validated architecture.
    pub architecture: Architecture,
    /// One immutable artifact, with shared lifetime ownership.
    pub artifact: DeploymentArtifact,
    /// Complete image entry configuration; missing and empty argv differ.
    pub image_config: Option<ImageConfig>,
    /// Captured user and provider-injected environment.
    pub environment: HashMap<String, String>,
    /// Function handler time limit.
    pub timeout: Duration,
    /// Function memory in MiB.
    pub memory_mb: u32,
    /// Ephemeral storage in MiB.
    pub ephemeral_storage_mb: u32,
}

impl fmt::Debug for ExecutionRevision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecutionRevision")
            .field("resource_id", &self.resource_id)
            .field("revision_id", &self.revision_id)
            .field("backend", &self.backend)
            .field("function_name", &self.function_name)
            .field("qualifier", &self.qualifier)
            .field("artifact", &self.artifact)
            .field("architecture", &self.architecture)
            .field("environment", &"[REDACTED]")
            .field("image_config", &"[REDACTED]")
            .field("timeout", &self.timeout)
            .field("memory_mb", &self.memory_mb)
            .field("ephemeral_storage_mb", &self.ephemeral_storage_mb)
            .finish_non_exhaustive()
    }
}

/// One invocation of an immutable captured revision.
#[derive(Clone)]
#[non_exhaustive]
pub struct InvokeRequest {
    /// Shared captured configuration and artifact ownership.
    pub revision: Arc<ExecutionRevision>,
    /// Canonical requested ARN, preserving the invoked qualifier.
    pub function_arn: String,
    /// Original event bytes.
    pub payload: Bytes,
    /// Whether to return the final runtime log bytes.
    pub capture_logs: bool,
}

impl InvokeRequest {
    /// Create an invocation without copying its captured configuration.
    ///
    /// ```
    /// # use std::sync::Arc;
    /// # use bytes::Bytes;
    /// # use rustack_lambda_core::executor::{ExecutionRevision, InvokeRequest};
    /// # fn capture(revision: Arc<ExecutionRevision>, arn: String) {
    /// let request = InvokeRequest::new(Arc::clone(&revision), arn, Bytes::new());
    /// assert!(Arc::ptr_eq(&request.revision, &revision));
    /// # }
    /// ```
    #[must_use]
    pub fn new(revision: Arc<ExecutionRevision>, function_arn: String, payload: Bytes) -> Self {
        Self {
            revision,
            function_arn,
            payload,
            capture_logs: false,
        }
    }
}

impl fmt::Debug for InvokeRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InvokeRequest")
            .field("revision", &self.revision)
            .field("function_arn", &self.function_arn)
            .field("payload", &"[REDACTED]")
            .field("capture_logs", &self.capture_logs)
            .finish()
    }
}

#[cfg(test)]
impl InvokeRequest {
    pub(crate) fn fixture() -> Self {
        let revision = ExecutionRevision::builder()
            .resource_id(FunctionResourceId::default())
            .revision_id("fixture-revision".into())
            .backend(ExecutorBackend::Native)
            .function_name("fixture".into())
            .qualifier("$LATEST".into())
            .runtime(Some("provided.al2023".into()))
            .handler(Some("bootstrap".into()))
            .architecture(Architecture::Arm64)
            .artifact(DeploymentArtifact::Zip(ValidatedArtifact::fixture()))
            .image_config(None)
            .environment(HashMap::new())
            .timeout(Duration::from_secs(3))
            .memory_mb(128)
            .ephemeral_storage_mb(512)
            .build();
        Self::new(
            Arc::new(revision),
            "arn:aws:lambda:us-east-1:000000000000:function:fixture".into(),
            Bytes::new(),
        )
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
    fn test_should_share_captured_revisions_and_isolate_separately_updated_configuration() {
        let original = InvokeRequest::fixture();
        let mut updated = original.clone();
        assert!(Arc::ptr_eq(&original.revision, &updated.revision));
        Arc::make_mut(&mut updated.revision).memory_mb = 512;
        assert_eq!(original.revision.memory_mb, 128);
        assert_eq!(updated.revision.memory_mb, 512);
        assert!(!Arc::ptr_eq(&original.revision, &updated.revision));
    }

    #[test]
    fn test_should_redact_invocation_request_and_response_debug() {
        let secret = "fixture-secret-should-never-be-rendered";
        let mut request = InvokeRequest::fixture();
        Arc::make_mut(&mut request.revision)
            .environment
            .insert("SECRET".into(), secret.into());
        request.payload = Bytes::from(secret);
        Arc::make_mut(&mut request.revision).image_config = Some(ImageConfig {
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
