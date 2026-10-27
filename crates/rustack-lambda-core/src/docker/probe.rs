//! Actual resource qualification through the retained environment owner.

mod endpoint;
use std::{collections::HashMap, fmt, sync::Arc, time::Duration};

use bytes::Bytes;
pub use endpoint::{EndpointProbe, EndpointProbeConfig, probe_endpoint};
use rustack_core::json::{JsonError, JsonLimits, from_slice};
use rustack_lambda_model::types::ImageConfig;
use rustack_lambda_runtime::{
    metadata::local_trace_id,
    probe::{REPORT_BYTES, ResourceReport},
    protocol::{Architecture as RuntimeArchitecture, AttemptId, Payload},
    session::{Invocation, RuntimeResult},
};
use thiserror::Error;
use tokio::time::Instant;
use typed_builder::TypedBuilder;
use uuid::Uuid;

use super::{
    ContainerPolicy, DirectoryLease, EnvironmentError, EnvironmentPlan, EnvironmentPort,
    QualifiedNetwork, check_deadline,
};
use crate::{
    artifact::{Architecture, DeploymentArtifact, PinnedImage},
    executor::{ExecutionRevision, ExecutorBackend},
    identity::FunctionResourceId,
};

/// Specific bounded resource qualification mismatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ProbeFailure {
    /// Wrong report schema, attempt or compiled architecture.
    #[error("resource probe identity mismatch")]
    Identity,
    /// Controller files are not rooted at the current process membership.
    #[error("resource probe cgroup namespace mismatch")]
    Namespace,
    /// Memory hard cap was not enforced.
    #[error("resource probe memory mismatch")]
    Memory,
    /// Extra swap was permitted or unavailable.
    #[error("resource probe swap mismatch")]
    Swap,
    /// PID cap was not enforced.
    #[error("resource probe PID mismatch")]
    Pids,
    /// CPU quota/period was not enforced.
    #[error("resource probe CPU mismatch")]
    Cpu,
    /// The actual process has a root UID/GID.
    #[error("resource probe user mismatch")]
    User,
    /// Effective capabilities are not empty.
    #[error("resource probe capability mismatch")]
    Capabilities,
    /// `no_new_privs` was not enforced.
    #[error("resource probe privilege mismatch")]
    Privilege,
    /// No active seccomp filter.
    #[error("resource probe seccomp mismatch")]
    Seccomp,
    /// The effective root mount is writable.
    #[error("resource probe root filesystem mismatch")]
    Root,
    /// The effective tmp mount differs from the captured policy.
    #[error("resource probe temporary filesystem mismatch")]
    Temporary,
    /// The observed gateway was not the exact expected instance/attempt/platform.
    #[error("endpoint probe identity mismatch")]
    Endpoint,
    /// The trusted probe did not return a report.
    #[error("resource probe execution failed")]
    Outcome,
}
/// Probe failures preserve bounded classes and redacted typed sources.
#[derive(Clone, Error)]
#[non_exhaustive]
pub enum EnvironmentProbeError {
    /// Invalid settings or mismatched retained runtime scope.
    #[error("invalid environment probe configuration")]
    Configuration,
    /// Bounded strict report decoding failed.
    #[error("invalid environment probe report")]
    Report(#[from] JsonError),
    /// Actual observations did not satisfy policy.
    #[error("{0}")]
    Rejected(#[from] ProbeFailure),
    /// Retained allocation/invocation failed.
    #[error("environment probe failed: {0}")]
    Environment(#[from] EnvironmentError),
    /// Cleanup did not confirm physical/IO settlement; lower owner retains it.
    #[error("environment probe cleanup not confirmed")]
    Cleanup(#[source] EnvironmentError),
}
impl fmt::Debug for EnvironmentProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Validated positive whole-second startup and independent cleanup budgets.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct ProbeSettings {
    start: Duration,
    cleanup: Duration,
}
impl Default for ProbeSettings {
    fn default() -> Self {
        Self {
            start: Duration::from_secs(60),
            cleanup: Duration::from_secs(10),
        }
    }
}
impl ProbeSettings {
    /// Validate both budgets in1..60 whole seconds.
    ///
    /// # Errors
    /// Rejects zero, fractional or excessive durations.
    ///
    /// ```
    /// let settings=rustack_lambda_core::docker::ProbeSettings::new(
    ///     std::time::Duration::from_secs(30),std::time::Duration::from_secs(10))?;
    /// # Ok::<(),rustack_lambda_core::docker::EnvironmentProbeError>(())
    /// ```
    pub fn new(start: Duration, cleanup: Duration) -> Result<Self, EnvironmentProbeError> {
        for duration in [start, cleanup] {
            if duration.subsec_nanos() != 0 || !(1..=60).contains(&duration.as_secs()) {
                return Err(EnvironmentProbeError::Configuration);
            }
        }
        Ok(Self { start, cleanup })
    }
}
/// Operator-owned private probe inputs; no user image or endpoint is accepted.
#[derive(Debug, Clone, TypedBuilder)]
#[non_exhaustive]
pub struct EnvironmentProbeConfig {
    /// Persistent namespace capability.
    pub directory: DirectoryLease,
    /// Pinned runtime image used for both relay and trusted probe function.
    pub relay: PinnedImage,
    /// Matching retained environment service.
    pub environments: EnvironmentPort,
    /// Operator policy, bound to the qualified network before allocation.
    pub policy: ContainerPolicy,
    /// Independent read-only network qualification.
    pub network: QualifiedNetwork,
    /// Validated startup/cleanup budgets.
    #[builder(default)]
    pub settings: ProbeSettings,
}
/// Private proof of actual matching kernel observations after joined retirement.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct EnvironmentProbe {
    report: ResourceReport,
    directory: DirectoryLease,
    relay: PinnedImage,
    policy: ContainerPolicy,
    environments: EnvironmentPort,
}
impl EnvironmentProbe {
    /// Inspect bounded observation data; it cannot reconstruct this proof.
    ///
    /// ```no_run
    /// # fn inspect(probe:&rustack_lambda_core::docker::EnvironmentProbe){
    /// let memory=probe.report().memory_max;
    /// # }
    /// ```
    #[must_use]
    pub fn report(&self) -> &ResourceReport {
        &self.report
    }
    /// Check the exact retained inputs and accepting environment service.
    ///
    /// ```no_run
    /// # fn matches(probe:&rustack_lambda_core::docker::EnvironmentProbe, inputs:&rustack_lambda_core::docker::EnvironmentProbeConfig){
    /// let current=probe.matches(inputs);
    /// # }
    /// ```
    #[must_use]
    pub fn matches(&self, inputs: &EnvironmentProbeConfig) -> bool {
        let metadata = self.relay.resolved().metadata();
        let other = inputs.relay.resolved().metadata();
        self.directory.owner_id() == inputs.directory.owner_id()
            && self.directory.run_id() == inputs.directory.run_id()
            && self.relay.is_live()
            && inputs.relay.is_live()
            && metadata.image_id == other.image_id
            && metadata.inspection_hash == other.inspection_hash
            && inputs
                .network
                .apply(&inputs.policy)
                .is_ok_and(|policy| policy == self.policy)
            && self.environments.same_service(&inputs.environments)
            && self.environments.status().accepting
    }
}

/// Execute the pinned Rust probe once and issue proof only after actual retirement.
///
/// This does not grant production readiness. The environment manager must outlive
/// a dropped/expired waiter, retaining uncertain container/IO cleanup.
///
/// # Errors
/// Returns invalid inputs, runtime/wire failure, strict report mismatch or cleanup
/// uncertainty. A failed accepted probe is never replayed.
///
/// ```no_run
/// # async fn probe(config:rustack_lambda_core::docker::EnvironmentProbeConfig){
/// let evidence=rustack_lambda_core::docker::probe_environment(config,
///     tokio::time::Instant::now()+std::time::Duration::from_secs(60)).await;
/// # }
/// ```
pub async fn probe_environment(
    config: EnvironmentProbeConfig,
    deadline: Instant,
) -> Result<EnvironmentProbe, EnvironmentProbeError> {
    let deadline = deadline.min(Instant::now() + config.settings.start);
    let payload = Payload::try_from(Bytes::from_static(b"{}"))
        .map_err(|_| EnvironmentProbeError::Configuration)?;
    let (report, policy) = invoke_probe(
        &config,
        ProbeCommand::Resource,
        payload,
        deadline,
        |bytes, attempt, architecture, policy| {
            let report = decode_report(bytes)?;
            verify(&report, attempt, architecture, policy)?;
            Ok(report)
        },
    )
    .await?;
    check_live(&config, deadline)?;
    Ok(EnvironmentProbe {
        report,
        directory: config.directory,
        relay: config.relay,
        policy,
        environments: config.environments,
    })
}
#[derive(Debug, Clone, Copy)]
enum ProbeCommand {
    Resource,
    Endpoint,
}
impl ProbeCommand {
    fn entry(self) -> &'static str {
        match self {
            Self::Resource => "probe",
            Self::Endpoint => "endpoint-probe",
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Resource => "rustack-runtime-probe",
            Self::Endpoint => "rustack-endpoint-probe",
        }
    }
}
fn check_live(
    config: &EnvironmentProbeConfig,
    deadline: Instant,
) -> Result<(), EnvironmentProbeError> {
    if Instant::now() >= deadline {
        return Err(EnvironmentError::Deadline.into());
    }
    if !config.environments.same_scope(&config.directory)
        || !config.relay.is_live()
        || !config.environments.status().accepting
    {
        return Err(EnvironmentProbeError::Configuration);
    }
    Ok(())
}
async fn invoke_probe<T>(
    config: &EnvironmentProbeConfig,
    command: ProbeCommand,
    payload: Payload,
    deadline: Instant,
    validate: impl FnOnce(
        &[u8],
        AttemptId,
        Architecture,
        &ContainerPolicy,
    ) -> Result<T, EnvironmentProbeError>,
) -> Result<(T, ContainerPolicy), EnvironmentProbeError> {
    check_deadline(deadline).map_err(EnvironmentError::from)?;
    check_live(config, deadline)?;
    let policy = config
        .network
        .apply(&config.policy)
        .map_err(EnvironmentError::from)?;
    let architecture = config.relay.resolved().metadata().architecture;
    let revision = ExecutionRevision::builder()
        .resource_id(FunctionResourceId::new())
        .revision_id(Uuid::new_v4().to_string())
        .backend(ExecutorBackend::Docker)
        .function_name(command.name().into())
        .qualifier("$LATEST".into())
        .runtime(None)
        .handler(None)
        .architecture(architecture)
        .artifact(DeploymentArtifact::Image(config.relay.clone()))
        .image_config(Some(ImageConfig {
            entry_point: Some(vec![
                "/rustack-lambda-runtime".into(),
                command.entry().into(),
            ]),
            command: Some(vec![]),
            working_directory: None,
        }))
        .environment(HashMap::from([
            ("AWS_REGION".into(), "us-east-1".into()),
            ("AWS_DEFAULT_REGION".into(), "us-east-1".into()),
        ]))
        .timeout(Duration::from_secs(10))
        .memory_mb(128)
        .ephemeral_storage_mb(512)
        .build();
    let plan = EnvironmentPlan::new(
        config.directory.clone(),
        config.relay.clone(),
        Arc::new(revision),
        policy.clone(),
    )
    .map_err(EnvironmentError::from)?;
    let attempt = AttemptId::default();
    let call = Invocation::new(
        attempt,
        deadline,
        format!(
            "arn:aws:lambda:us-east-1:000000000000:function:{}",
            command.name()
        )
        .parse()
        .map_err(|_| EnvironmentProbeError::Configuration)?,
        local_trace_id().map_err(|_| EnvironmentProbeError::Configuration)?,
        payload,
    )
    .map_err(|_| EnvironmentProbeError::Configuration)?;
    let mut environment = config.environments.spawn(plan, deadline)?;
    let result = async {
        environment.wait_ready(deadline).await?;
        let response = environment.invoke(call, false).await?;
        let RuntimeResult::Success(payload) = response.result else {
            return Err(ProbeFailure::Outcome.into());
        };
        validate(payload.as_ref(), attempt, architecture, &policy)
    }
    .await;
    environment
        .shutdown(Instant::now() + config.settings.cleanup)
        .await
        .map_err(EnvironmentProbeError::Cleanup)?;
    Ok((result?, policy))
}

fn decode_report(bytes: &[u8]) -> Result<ResourceReport, EnvironmentProbeError> {
    Ok(from_slice(
        bytes,
        JsonLimits::new(REPORT_BYTES, 8, 128, 256, 32)?,
    )?)
}
fn verify(
    report: &ResourceReport,
    attempt: AttemptId,
    architecture: Architecture,
    policy: &ContainerPolicy,
) -> Result<(), ProbeFailure> {
    if report.version != 1
        || report.attempt != attempt
        || report.architecture
            != match architecture {
                Architecture::Arm64 => RuntimeArchitecture::Arm64,
                Architecture::X86_64 => RuntimeArchitecture::X86_64,
            }
    {
        return Err(ProbeFailure::Identity);
    }
    if report.memory_max != Some(128 * 1024 * 1024) {
        return Err(ProbeFailure::Memory);
    }
    if !report.cgroup_root {
        return Err(ProbeFailure::Namespace);
    }
    if report.swap_max != Some(0) {
        return Err(ProbeFailure::Swap);
    }
    if report.pids_max != Some(256) {
        return Err(ProbeFailure::Pids);
    }
    let expected = u64::try_from(policy.nano_cpus)
        .ok()
        .and_then(|cpu| cpu.checked_mul(report.cpu_period))
        .map(|total| total / 1_000_000_000);
    if !(1000..=1_000_000).contains(&report.cpu_period)
        || !report
            .cpu_quota
            .is_some_and(|quota| (1000..=16_000_000).contains(&quota))
        || expected == Some(0)
        || report.cpu_quota != expected
        || expected.is_none()
    {
        return Err(ProbeFailure::Cpu);
    }
    if report.euid == 0 || report.egid == 0 {
        return Err(ProbeFailure::User);
    }
    if report.cap_effective != 0 {
        return Err(ProbeFailure::Capabilities);
    }
    if !report.no_new_privileges {
        return Err(ProbeFailure::Privilege);
    }
    if report.seccomp_mode != 2 {
        return Err(ProbeFailure::Seccomp);
    }
    if !report.root_readonly {
        return Err(ProbeFailure::Root);
    }
    let temporary = &report.temporary;
    if temporary.bytes != 512 * 1024 * 1024
        || temporary.mode != 0o1777
        || !temporary.tmpfs
        || !temporary.nosuid
        || !temporary.nodev
    {
        return Err(ProbeFailure::Temporary);
    }
    Ok(())
}

#[cfg(test)]
pub(super) mod tests;
