//! Matching real resource evidence and retained container-origin endpoint proof.

use bytes::Bytes;
use rustack_core::json::{JsonLimits, from_slice};
use rustack_lambda_runtime::{
    probe::{EndpointChallenge, EndpointReport, GatewayEndpoint, GatewayInstanceId, REPORT_BYTES},
    protocol::{Architecture as RuntimeArchitecture, AttemptId, Payload},
};
use serde_json::to_vec;
use tokio::time::Instant;

use super::{
    EnvironmentProbe, EnvironmentProbeConfig, EnvironmentProbeError, ProbeCommand, ProbeFailure,
    check_live, invoke_probe,
};
use crate::{artifact::Architecture, docker::DockerGatewayEndpoint};

/// Immutable endpoint qualification inputs, requiring actual matching kernel proof.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct EndpointProbeConfig {
    base: EnvironmentProbeConfig,
    resource: EnvironmentProbe,
    endpoint: DockerGatewayEndpoint,
    instance: GatewayInstanceId,
}
impl EndpointProbeConfig {
    pub(in crate::docker) fn gateway_endpoint(&self) -> &DockerGatewayEndpoint {
        &self.endpoint
    }
    /// Capture the exact qualified resource inputs and configured endpoint instance.
    /// # Errors
    /// Rejects disconnected networks and revoked/mismatching retained resource evidence.
    /// ```no_run
    /// # fn config(base:rustack_lambda_core::docker::EnvironmentProbeConfig,resource:rustack_lambda_core::docker::EnvironmentProbe,endpoint:rustack_lambda_core::docker::DockerGatewayEndpoint,instance:rustack_lambda_core::docker::GatewayInstanceId)->Result<(),rustack_lambda_core::docker::EnvironmentProbeError>{
    /// let config=rustack_lambda_core::docker::EndpointProbeConfig::new(base,resource,endpoint,instance)?;
    /// # Ok(()) }
    /// ```
    pub fn new(
        base: EnvironmentProbeConfig,
        resource: EnvironmentProbe,
        endpoint: DockerGatewayEndpoint,
        instance: GatewayInstanceId,
    ) -> Result<Self, EnvironmentProbeError> {
        if base.network.disconnected() || !resource.matches(&base) {
            return Err(EnvironmentProbeError::Configuration);
        }
        Ok(Self {
            base,
            resource,
            endpoint,
            instance,
        })
    }
    fn check(&self, deadline: Instant) -> Result<(), EnvironmentProbeError> {
        check_live(&self.base, deadline)?;
        if self.base.network.disconnected() || !self.resource.matches(&self.base) {
            return Err(EnvironmentProbeError::Configuration);
        }
        Ok(())
    }
}
/// Private receipt issued only after actual validated IO and physical retirement.
/// ```compile_fail
/// let forged=serde_json::from_str::<rustack_lambda_core::docker::EndpointProbe>("{}");
/// ```
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct EndpointProbe {
    report: EndpointReport,
    inputs: EndpointProbeConfig,
}
impl EndpointProbe {
    /// Recheck exact endpoint/identity and all retained live resource inputs.
    /// ```no_run
    /// # fn matches(proof:&rustack_lambda_core::docker::EndpointProbe,inputs:&rustack_lambda_core::docker::EndpointProbeConfig){
    /// let current=proof.matches(inputs);
    /// # }
    /// ```
    #[must_use]
    pub fn matches(&self, inputs: &EndpointProbeConfig) -> bool {
        self.inputs.endpoint == inputs.endpoint
            && self.inputs.instance == inputs.instance
            && self.inputs.resource.matches(&inputs.base)
            && inputs.resource.matches(&inputs.base)
            && self.inputs.resource.matches(&self.inputs.base)
            && !inputs.base.network.disconnected()
    }
    /// Bounded observed report data; cloning/serializing data cannot issue proof.
    /// ```no_run
    /// # fn report(proof:&rustack_lambda_core::docker::EndpointProbe){let data=proof.report();}
    /// ```
    #[must_use]
    pub fn report(&self) -> &EndpointReport {
        &self.report
    }
}
/// Execute one endpoint probe; a failed accepted invocation is never replayed.
/// # Errors
/// Rejects stale/invalid inputs, network/protocol/report mismatch, timeout or
/// uncertain cleanup. Dropped callers leave retirement with the environment owner.
/// ```no_run
/// # async fn probe(config:rustack_lambda_core::docker::EndpointProbeConfig){
/// let proof=rustack_lambda_core::docker::probe_endpoint(config,tokio::time::Instant::now()+std::time::Duration::from_secs(60)).await;
/// # }
/// ```
pub async fn probe_endpoint(
    config: EndpointProbeConfig,
    deadline: Instant,
) -> Result<EndpointProbe, EnvironmentProbeError> {
    let deadline = deadline.min(Instant::now() + config.base.settings.start);
    config.check(deadline)?;
    let challenge = EndpointChallenge::new(
        GatewayEndpoint::from(config.endpoint.clone()),
        config.instance,
    );
    let bytes = to_vec(&challenge).map_err(|_| EnvironmentProbeError::Configuration)?;
    if bytes.len() > REPORT_BYTES {
        return Err(EnvironmentProbeError::Configuration);
    }
    let payload =
        Payload::try_from(Bytes::from(bytes)).map_err(|_| EnvironmentProbeError::Configuration)?;
    let (report, _policy) = invoke_probe(
        &config.base,
        ProbeCommand::Endpoint,
        payload,
        deadline,
        |bytes, attempt, architecture, _policy| {
            verify_report(bytes, attempt, architecture, config.instance)
        },
    )
    .await?;
    config.check(deadline)?;
    Ok(EndpointProbe {
        report,
        inputs: config,
    })
}

fn verify_report(
    bytes: &[u8],
    attempt: AttemptId,
    architecture: Architecture,
    instance: GatewayInstanceId,
) -> Result<EndpointReport, EnvironmentProbeError> {
    let report: EndpointReport = from_slice(bytes, JsonLimits::new(REPORT_BYTES, 8, 16, 256, 32)?)?;
    let expected = match architecture {
        Architecture::Arm64 => RuntimeArchitecture::Arm64,
        Architecture::X86_64 => RuntimeArchitecture::X86_64,
    };
    if report.attempt() != attempt
        || report.architecture() != expected
        || report.instance_id() != instance
    {
        return Err(ProbeFailure::Endpoint.into());
    }
    Ok(report)
}
#[cfg(test)]
mod tests;
