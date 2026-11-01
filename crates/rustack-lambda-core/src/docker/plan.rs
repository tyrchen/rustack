//! Immutable, bounded container creation policy; lifetime ownership remains above IO.

use std::{
    collections::{BTreeMap, HashMap},
    fmt,
    str::FromStr,
    sync::Arc,
    time::Duration,
};

use rustack_lambda_model::types::ImageConfig;
use rustack_lambda_runtime::protocol::EnvironmentId;
use serde::Serialize;
use tokio::time::Instant;
use typed_builder::TypedBuilder;
use uuid::Uuid;

use super::{
    CodeVolumeIdentity, DirectoryLease, DockerError, DockerKey, DockerPort, DockerSdkEnvironment,
    DockerZipRuntime,
    models::{
        ContainerCreateBody, ContainerCreateResponse, ContainerInspectResponse, HealthConfig,
        HostConfig, HostConfigCgroupnsModeEnum, HostConfigLogConfig, Mount, MountType,
        MountVolumeOptions, RestartPolicy, RestartPolicyNameEnum,
    },
    mutation::MutationGuard,
    sdk::SdkAffiliation,
    zip::{AllocationClaim, ZipPlan},
};
use crate::{
    artifact::{DeploymentArtifact, PinnedImage},
    executor::{ExecutionRevision, ExecutorBackend},
    function_config::{validate_environment_entries, validate_provided_settings},
    storage::compute_sha256,
};

const MIB: u64 = 1024 * 1024;
const RELAY_MEMORY: u64 = 64 * MIB;
const CPU_QUANTUM: u64 = 10_000_000;

#[derive(Debug, Clone, Copy)]
pub(super) enum ContainerRole {
    Relay,
    Function,
}

/// Operator network selection; a bridge still requires daemon startup validation.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct NetworkSelection(String);
impl NetworkSelection {
    pub(super) fn value(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for NetworkSelection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NetworkSelection(<operator>)")
    }
}
impl FromStr for NetworkSelection {
    type Err = DockerError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value == "none" {
            return Ok(Self(value.into()));
        }
        if matches!(value, "host" | "bridge" | "default") {
            return Err(DockerError::Configuration);
        }
        value.parse::<DockerKey>()?;
        Ok(Self(value.into()))
    }
}

/// Integral relay session budgets independent of handler and deployment budgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct RelayBudgets {
    io: Duration,
    idle: Duration,
    retirement: Duration,
}
impl Default for RelayBudgets {
    fn default() -> Self {
        Self {
            io: Duration::from_secs(5),
            idle: Duration::from_secs(600),
            retirement: Duration::from_secs(10),
        }
    }
}
impl RelayBudgets {
    /// Validate whole-second IO/retirement1..60s and idle1..86400s budgets.
    ///
    /// # Errors
    /// Rejects zero, fractional or excessive durations.
    ///
    /// ```
    /// let budgets = rustack_lambda_core::docker::RelayBudgets::new(
    ///     std::time::Duration::from_secs(5), std::time::Duration::from_secs(600),
    ///     std::time::Duration::from_secs(10))?;
    /// # Ok::<(), rustack_lambda_core::docker::DockerError>(())
    /// ```
    pub fn new(io: Duration, idle: Duration, retirement: Duration) -> Result<Self, DockerError> {
        for (duration, maximum) in [(io, 60), (idle, 86400), (retirement, 60)] {
            if duration.subsec_nanos() != 0 || !(1..=maximum).contains(&duration.as_secs()) {
                return Err(DockerError::Configuration);
            }
        }
        Ok(Self {
            io,
            idle,
            retirement,
        })
    }
}

/// Validated operator limits, independent of user function configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct ContainerPolicy {
    pub(super) network: NetworkSelection,
    uid: u32,
    gid: u32,
    requested_nano_cpus: u64,
    pub(super) nano_cpus: i64,
    relay: RelayBudgets,
}
impl ContainerPolicy {
    pub(super) fn idle(&self) -> Duration {
        self.relay.idle
    }
    /// Return the requested maximum and its downward-normalized CPU allocation.
    ///
    /// ```
    /// use rustack_lambda_core::docker::{ContainerPolicy, RelayBudgets};
    /// let policy = ContainerPolicy::new("none".parse()?, 1000, 1000,
    ///     333_333_333, RelayBudgets::default())?;
    /// assert_eq!(policy.cpu_allocation().effective_nano_cpus, 330_000_000);
    /// # Ok::<(), rustack_lambda_core::docker::DockerError>(())
    /// ```
    #[must_use]
    pub fn cpu_allocation(&self) -> DockerCpuAllocation {
        DockerCpuAllocation {
            requested_nano_cpus: self.requested_nano_cpus,
            effective_nano_cpus: self.nano_cpus.unsigned_abs(),
        }
    }
    /// Validate UID/GID and0.1..16 CPUs, rounding CPU down to0.01 increments.
    ///
    /// # Errors
    /// Rejects invalid IDs or CPU values, preserving bounded operator policy.
    ///
    /// ```
    /// let policy = rustack_lambda_core::docker::ContainerPolicy::new(
    ///     "none".parse()?, 1000, 1000, 1_000_000_000,
    ///     rustack_lambda_core::docker::RelayBudgets::default())?;
    /// # Ok::<(), rustack_lambda_core::docker::DockerError>(())
    /// ```
    pub fn new(
        network: NetworkSelection,
        uid: u32,
        gid: u32,
        nano_cpus: u64,
        relay: RelayBudgets,
    ) -> Result<Self, DockerError> {
        if !(1..=2_147_483_647).contains(&uid)
            || !(1..=2_147_483_647).contains(&gid)
            || !(100_000_000..=16_000_000_000).contains(&nano_cpus)
        {
            return Err(DockerError::Configuration);
        }
        Ok(Self {
            network,
            uid,
            gid,
            requested_nano_cpus: nano_cpus,
            nano_cpus: i64::try_from(nano_cpus - nano_cpus % CPU_QUANTUM)
                .map_err(|_| DockerError::Configuration)?,
            relay,
        })
    }
}

/// Captured CPU maximum and the portable allocation actually sent to Docker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct DockerCpuAllocation {
    /// Original validated operator maximum, in billionths of one CPU.
    pub requested_nano_cpus: u64,
    /// Maximum rounded down to one-percent CPU increments, never upward.
    pub effective_nano_cpus: u64,
}

struct Identity {
    environment: EnvironmentId,
    relay: DockerKey,
    function: DockerKey,
}
struct Bodies {
    relay: ContainerCreateBody,
    function: ContainerCreateBody,
    memory: u64,
}

/// Fixed create requests retaining revision, image availability and namespace leases.
#[derive(TypedBuilder)]
#[builder(builder_method(vis = ""))]
#[non_exhaustive]
pub struct EnvironmentPlan {
    pub(super) directory: DirectoryLease,
    relay: PinnedImage,
    revision: Arc<ExecutionRevision>,
    identity: Identity,
    bodies: Bodies,
    warm_key: String,
    #[builder(default)]
    pub(super) zip: Option<Arc<ZipPlan>>,
    #[builder(default)]
    sdk: Option<SdkAffiliation>,
}
impl fmt::Debug for EnvironmentPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnvironmentPlan")
            .field("environment", &self.identity.environment)
            .field("resource", &self.revision.resource_id)
            .field("memory_bytes", &self.bodies.memory)
            .finish_non_exhaustive()
    }
}
impl EnvironmentPlan {
    pub(super) fn with_sdk_environment(
        mut self,
        sdk: &DockerSdkEnvironment,
    ) -> Result<Self, DockerError> {
        let guard = sdk.liveness();
        guard.live()?;
        let entries = self
            .bodies
            .function
            .env
            .take()
            .ok_or(DockerError::Configuration)?;
        self.bodies.function.env = Some(sdk.environment(entries)?);
        self.warm_key =
            compute_sha256(format!("sdk-v1:{}:{}", self.warm_key, sdk.identity()).as_bytes());
        self.sdk = Some(guard);
        Ok(self)
    }
    pub(super) fn image_id(&self, role: ContainerRole) -> &str {
        match role {
            ContainerRole::Relay => self.relay.resolved().metadata().image_id.as_str(),
            ContainerRole::Function => self.bodies.function.image.as_deref().unwrap_or_default(),
        }
    }
    pub(super) fn warm_key(&self) -> &str {
        &self.warm_key
    }
    pub(super) fn same_scope(&self, directory: &DirectoryLease) -> bool {
        if self.zip.is_some() {
            self.directory.same_instance(directory)
        } else {
            self.directory.owner_id() == directory.owner_id()
                && self.directory.run_id() == directory.run_id()
        }
    }
    pub(super) fn revision(&self) -> &ExecutionRevision {
        &self.revision
    }
    pub(super) fn key(&self, role: ContainerRole) -> &DockerKey {
        match role {
            ContainerRole::Relay => self.relay_key(),
            ContainerRole::Function => self.function_key(),
        }
    }
    pub(super) fn owned(
        &self,
        role: ContainerRole,
        inspected: &ContainerInspectResponse,
    ) -> Result<DockerKey, DockerError> {
        let body = match role {
            ContainerRole::Relay => &self.bodies.relay,
            ContainerRole::Function => &self.bodies.function,
        };
        let config = inspected.config.as_ref().ok_or(DockerError::Protocol)?;
        let expected = body.labels.as_ref().ok_or(DockerError::Configuration)?;
        let actual = config.labels.as_ref().ok_or(DockerError::Quarantined)?;
        let id = inspected.id.as_deref().ok_or(DockerError::Protocol)?;
        if id.len() != 64
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || inspected.name.as_deref() != Some(format!("/{}", self.key(role).as_str()).as_str())
            || config.image != body.image
            || !expected
                .iter()
                .all(|(key, value)| actual.get(key) == Some(value))
        {
            return Err(DockerError::Quarantined);
        }
        if matches!(role, ContainerRole::Function) {
            if let Some(zip) = &self.zip {
                let mounts = inspected.mounts.as_ref().ok_or(DockerError::Quarantined)?;
                if mounts.len() != 1
                    || mounts.first().is_none_or(|mount| {
                        mount.typ.as_deref() != Some("volume")
                            || mount.name.as_deref() != Some(zip.volume.as_str())
                            || mount.destination.as_deref() != Some("/var/task")
                            || mount.rw != Some(false)
                            || mount.driver.as_deref() != Some("local")
                    })
                {
                    return Err(DockerError::Quarantined);
                }
            }
        }
        id.parse()
    }
    pub(super) fn claim_code(&self) -> Result<Option<AllocationClaim>, DockerError> {
        self.zip.as_ref().map(ZipPlan::claim).transpose()
    }
    pub(super) fn commit_code_claim(
        &self,
        claim: &mut Option<AllocationClaim>,
    ) -> Result<(), DockerError> {
        match (&self.zip, claim) {
            (Some(plan), Some(claim)) if claim.belongs_to(plan) => {
                claim.commit();
                Ok(())
            }
            (None, None) => Ok(()),
            _ => Err(DockerError::Configuration),
        }
    }
    pub(super) fn code_clean(&self) -> bool {
        self.zip.as_ref().is_none_or(|zip| zip.clean())
    }
    fn require_code(&self, role: ContainerRole) -> Result<(), DockerError> {
        if matches!(role, ContainerRole::Function) {
            if let Some(zip) = &self.zip {
                zip.ready_mount()?;
            }
        }
        Ok(())
    }
    /// Allocate identities and validate both immutable create bodies before IO.
    ///
    /// # Errors
    /// Rejects unavailable/mismatched artifacts, invalid captured settings or
    /// effective image configuration. This does not reserve environment capacity.
    ///
    /// ```no_run
    /// # fn plan(directory: rustack_lambda_core::docker::DirectoryLease, relay: rustack_lambda_core::artifact::PinnedImage, revision: std::sync::Arc<rustack_lambda_core::executor::ExecutionRevision>, policy: rustack_lambda_core::docker::ContainerPolicy) {
    /// let plan = rustack_lambda_core::docker::EnvironmentPlan::new(directory, relay, revision, policy);
    /// # }
    /// ```
    pub fn new(
        directory: DirectoryLease,
        relay: PinnedImage,
        revision: Arc<ExecutionRevision>,
        policy: ContainerPolicy,
    ) -> Result<Self, DockerError> {
        Self::construct(directory, relay, revision, policy, None)
    }

    /// Plan provided.al2023 execution over an affiliated immutable base and producer.
    ///
    /// Conversion and allocation begin only after environment admission.
    ///
    /// # Errors
    /// Rejects incompatible runtime/revision, source scope, platform or policy.
    ///
    /// ```no_run
    /// # fn plan(directory:rustack_lambda_core::docker::DirectoryLease,relay:rustack_lambda_core::artifact::PinnedImage,revision:std::sync::Arc<rustack_lambda_core::executor::ExecutionRevision>,policy:rustack_lambda_core::docker::ContainerPolicy,source:rustack_lambda_core::docker::DockerZipRuntime) {
    /// let plan=rustack_lambda_core::docker::EnvironmentPlan::new_zip(directory,relay,revision,policy,source);
    /// # }
    /// ```
    pub fn new_zip(
        directory: DirectoryLease,
        relay: PinnedImage,
        revision: Arc<ExecutionRevision>,
        policy: ContainerPolicy,
        source: DockerZipRuntime,
    ) -> Result<Self, DockerError> {
        Self::construct(directory, relay, revision, policy, Some(source))
    }

    fn construct(
        directory: DirectoryLease,
        relay: PinnedImage,
        revision: Arc<ExecutionRevision>,
        policy: ContainerPolicy,
        zip: Option<DockerZipRuntime>,
    ) -> Result<Self, DockerError> {
        validate_revision(&revision)?;
        let function = match (&revision.artifact, &zip) {
            (DeploymentArtifact::Image(_), None) => image(&revision)?,
            (DeploymentArtifact::Zip(_), Some(source)) => {
                if !source.same_scope(&directory)
                    || source.base.resolved().metadata().architecture != revision.architecture
                {
                    return Err(DockerError::Configuration);
                }
                source.available()?;
                &source.base
            }
            _ => return Err(DockerError::Configuration),
        };
        if !relay.is_live() || relay.resolved().metadata().architecture != revision.architecture {
            return Err(DockerError::Configuration);
        }
        let code = if zip.is_some() {
            Some(CodeVolumeIdentity::new(
                revision
                    .resource_id
                    .to_string()
                    .parse()
                    .map_err(|_| DockerError::Configuration)?,
                revision
                    .revision_id
                    .parse()
                    .map_err(|_| DockerError::Configuration)?,
            )?)
        } else {
            None
        };
        let environment = match &code {
            Some(code) => code
                .environment_id()
                .to_string()
                .parse()
                .map_err(|_| DockerError::Configuration)?,
            None => EnvironmentId::default(),
        };
        let identity = Identity {
            environment,
            relay: format!("rustack-relay-{environment}").parse()?,
            function: format!("rustack-function-{environment}").parse()?,
        };
        let labels = plan_labels(&directory, environment, &revision);
        let (mut function_body, memory) =
            function_create_body(function, &revision, &identity, &policy, &labels)?;
        let warm_key = warm_identity(&directory, &relay, &revision, &policy, function)?;
        let relay_body = relay_create_body(&relay, policy, environment, &labels)?;
        let zip = match (zip, code) {
            (Some(source), Some(code)) => Some(ZipPlan::new(source, code)?),
            (None, None) => None,
            _ => return Err(DockerError::Configuration),
        };
        if let Some(zip) = &zip {
            function_body
                .host_config
                .as_mut()
                .ok_or(DockerError::Configuration)?
                .mounts = Some(vec![Mount {
                typ: Some(MountType::VOLUME),
                source: Some(zip.volume.as_str().into()),
                target: Some("/var/task".into()),
                read_only: Some(true),
                volume_options: Some(MountVolumeOptions {
                    no_copy: Some(true),
                    ..Default::default()
                }),
                ..Default::default()
            }]);
        }
        let memory = memory
            .checked_add(RELAY_MEMORY)
            .ok_or(DockerError::Configuration)?;
        Ok(Self::builder()
            .directory(directory)
            .relay(relay)
            .revision(revision)
            .identity(identity)
            .bodies(Bodies {
                relay: relay_body,
                function: function_body,
                memory,
            })
            .warm_key(warm_key)
            .zip(zip)
            .build())
    }

    /// Fresh environment UUID used by both labels and control protocol.
    ///
    /// ```no_run
    /// # fn id(plan: &rustack_lambda_core::docker::EnvironmentPlan) {
    /// let id = plan.environment_id();
    /// # }
    /// ```
    #[must_use]
    pub fn environment_id(&self) -> EnvironmentId {
        self.identity.environment
    }

    /// Preallocated relay identity retained through uncertain allocation cleanup.
    ///
    /// ```no_run
    /// # fn key(plan: &rustack_lambda_core::docker::EnvironmentPlan) {
    /// let name = plan.relay_key();
    /// # }
    /// ```
    #[must_use]
    pub fn relay_key(&self) -> &DockerKey {
        &self.identity.relay
    }

    /// Preallocated function identity; never populated from invocation input.
    ///
    /// ```no_run
    /// # fn key(plan: &rustack_lambda_core::docker::EnvironmentPlan) {
    /// let name = plan.function_key();
    /// # }
    /// ```
    #[must_use]
    pub fn function_key(&self) -> &DockerKey {
        &self.identity.function
    }

    /// Checked function plus relay memory required before allocation admission.
    ///
    /// ```no_run
    /// # fn memory(plan: &rustack_lambda_core::docker::EnvironmentPlan) {
    /// let bytes = plan.memory_bytes();
    /// # }
    /// ```
    #[must_use]
    pub fn memory_bytes(&self) -> u64 {
        self.bodies.memory
    }

    /// Create the relay after its owner has recorded uncertain allocation.
    ///
    /// # Errors
    /// Returns revoked availability, deadline, wire/status or schema failure.
    /// Every uncertain outcome still requires owner reconciliation.
    ///
    /// ```no_run
    /// # async fn create(plan: &rustack_lambda_core::docker::EnvironmentPlan, port: &rustack_lambda_core::docker::DockerPort) {
    /// let created = plan.create_relay(port, tokio::time::Instant::now() + std::time::Duration::from_secs(5)).await;
    /// # }
    /// ```
    pub async fn create_relay(
        &self,
        port: &DockerPort,
        deadline: Instant,
    ) -> Result<ContainerCreateResponse, DockerError> {
        self.live()?;
        port.create_container_on_platform(
            &self.identity.relay,
            &self.bodies.relay,
            self.revision.architecture,
            deadline,
        )
        .await
    }

    /// Create the function only after its own relay is verified ready.
    ///
    /// # Errors
    /// Returns revoked availability, deadline, wire/status or schema failure.
    /// Caller owns startup ordering and every uncertain allocation outcome.
    ///
    /// ```no_run
    /// # async fn create(plan: &rustack_lambda_core::docker::EnvironmentPlan, port: &rustack_lambda_core::docker::DockerPort) {
    /// let created = plan.create_function(port, tokio::time::Instant::now() + std::time::Duration::from_secs(5)).await;
    /// # }
    /// ```
    pub async fn create_function(
        &self,
        port: &DockerPort,
        deadline: Instant,
    ) -> Result<ContainerCreateResponse, DockerError> {
        self.live()?;
        self.require_code(ContainerRole::Function)?;
        port.create_container_on_platform(
            &self.identity.function,
            &self.bodies.function,
            self.revision.architecture,
            deadline,
        )
        .await
    }
    pub(super) async fn create_tracked(
        &self,
        role: ContainerRole,
        port: &DockerPort,
        deadline: Instant,
        mutation: MutationGuard,
    ) -> Result<ContainerCreateResponse, DockerError> {
        self.live()?;
        self.require_code(role)?;
        let body = match role {
            ContainerRole::Relay => &self.bodies.relay,
            ContainerRole::Function => &self.bodies.function,
        };
        port.create_container_inner(
            self.key(role),
            body,
            Some(self.revision.architecture),
            deadline,
            Some(mutation),
        )
        .await
    }
    pub(super) fn live(&self) -> Result<(), DockerError> {
        if let Some(sdk) = &self.sdk {
            sdk.live()?;
        }
        match &self.revision.artifact {
            DeploymentArtifact::Image(function) if self.relay.is_live() && function.is_live() => {
                Ok(())
            }
            DeploymentArtifact::Zip(_) if self.relay.is_live() => {
                self.zip.as_ref().ok_or(DockerError::Configuration)?.live()
            }
            DeploymentArtifact::Image(_) | DeploymentArtifact::Zip(_) => Err(DockerError::Stopped),
        }
    }
}

fn plan_labels(
    directory: &DirectoryLease,
    environment: EnvironmentId,
    revision: &ExecutionRevision,
) -> HashMap<String, String> {
    HashMap::from([
        (
            "rustack.lambda.ownerId".into(),
            directory.owner_id().to_string(),
        ),
        (
            "rustack.lambda.runId".into(),
            directory.run_id().to_string(),
        ),
        (
            "rustack.lambda.environmentId".into(),
            environment.to_string(),
        ),
        (
            "rustack.lambda.functionResourceId".into(),
            revision.resource_id.to_string(),
        ),
    ])
}
fn relay_create_body(
    relay: &PinnedImage,
    policy: ContainerPolicy,
    environment: EnvironmentId,
    labels: &HashMap<String, String>,
) -> Result<ContainerCreateBody, DockerError> {
    let user = format!("{}:{}", policy.uid, policy.gid);
    let mut relay = body(relay, None, &user, labels, "relay")?;
    relay.env = Some(vec![
        format!("RUSTACK_LAMBDA_ENVIRONMENT_ID={environment}"),
        format!(
            "RUSTACK_LAMBDA_RELAY_IO_SECONDS={}",
            policy.relay.io.as_secs()
        ),
        format!(
            "RUSTACK_LAMBDA_RELAY_IDLE_SECONDS={}",
            policy.relay.idle.as_secs()
        ),
        format!(
            "RUSTACK_LAMBDA_RELAY_RETIRE_SECONDS={}",
            policy.relay.retirement.as_secs()
        ),
    ]);
    relay.attach_stdin = Some(true);
    relay.open_stdin = Some(true);
    relay.stdin_once = Some(true);
    relay.host_config = Some(limits(
        policy.network.0,
        RELAY_MEMORY,
        100_000_000,
        64,
        MIB,
    )?);
    Ok(relay)
}
fn function_create_body(
    function: &PinnedImage,
    revision: &ExecutionRevision,
    identity: &Identity,
    policy: &ContainerPolicy,
    labels: &HashMap<String, String>,
) -> Result<(ContainerCreateBody, u64), DockerError> {
    let user = format!("{}:{}", policy.uid, policy.gid);
    let zip = matches!(revision.artifact, DeploymentArtifact::Zip(_));
    let selected_user = if zip {
        user.clone()
    } else {
        image_user(
            &function.resolved().metadata().execution.metadata().user,
            &user,
        )?
    };
    let provided = ImageConfig {
        entry_point: Some(vec!["/var/task/bootstrap".into()]),
        command: Some(Vec::new()),
        working_directory: Some("/var/task".into()),
    };
    let overrides = if zip {
        Some(&provided)
    } else {
        revision.image_config.as_ref()
    };
    let mut body = body(function, overrides, &selected_user, labels, "function")?;
    body.env = Some(function_environment(
        revision,
        identity.environment,
        function,
    )?);
    let memory = u64::from(revision.memory_mb)
        .checked_mul(MIB)
        .ok_or(DockerError::Configuration)?;
    body.host_config = Some(limits(
        format!("container:{}", identity.relay.as_str()),
        memory,
        policy.nano_cpus,
        256,
        u64::from(revision.ephemeral_storage_mb)
            .checked_mul(MIB)
            .ok_or(DockerError::Configuration)?,
    )?);
    Ok((body, memory))
}

fn warm_identity(
    directory: &DirectoryLease,
    relay: &PinnedImage,
    revision: &ExecutionRevision,
    policy: &ContainerPolicy,
    function: &PinnedImage,
) -> Result<String, DockerError> {
    let environment: BTreeMap<_, _> = revision.environment.iter().collect();
    let encoded = serde_json::to_vec(&(
        (
            directory.owner_id().to_string(),
            directory.run_id().to_string(),
        ),
        (
            revision.resource_id.to_string(),
            &revision.revision_id,
            &revision.function_name,
            &revision.qualifier,
        ),
        (
            revision.architecture,
            function.resolved().metadata(),
            relay.resolved().metadata(),
        ),
        (
            &revision.image_config,
            environment,
            revision.timeout,
            revision.memory_mb,
            revision.ephemeral_storage_mb,
        ),
        (
            &policy,
            &revision.runtime,
            &revision.handler,
            match &revision.artifact {
                DeploymentArtifact::Zip(code) => Some(code.sha256()),
                DeploymentArtifact::Image(_) => None,
            },
        ),
    ))
    .map_err(|_| DockerError::Configuration)?;
    Ok(compute_sha256(&encoded))
}

fn image(revision: &ExecutionRevision) -> Result<&PinnedImage, DockerError> {
    match &revision.artifact {
        DeploymentArtifact::Image(image)
            if image.is_live()
                && image.resolved().metadata().architecture == revision.architecture =>
        {
            Ok(image)
        }
        _ => Err(DockerError::Configuration),
    }
}
fn validate_revision(revision: &ExecutionRevision) -> Result<(), DockerError> {
    if revision.revision_id.len() != 36 {
        return Err(DockerError::Configuration);
    }
    let id = Uuid::parse_str(&revision.revision_id).map_err(|_| DockerError::Configuration)?;
    let version = &revision.qualifier;
    let numeric_version = version.len() <= 20
        && version
            .parse::<u64>()
            .is_ok_and(|number| number > 0 && number.to_string() == *version);
    if revision.backend != ExecutorBackend::Docker
        || id.is_nil()
        || id.to_string() != revision.revision_id
        || !(1..=64).contains(&revision.function_name.len())
        || !revision
            .function_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
        || (version != "$LATEST" && !numeric_version)
        || !(1..=900).contains(&revision.timeout.as_secs())
        || revision.timeout.subsec_nanos() != 0
        || !(128..=10240).contains(&revision.memory_mb)
        || !(512..=10240).contains(&revision.ephemeral_storage_mb)
    {
        return Err(DockerError::Configuration);
    }
    match &revision.artifact {
        DeploymentArtifact::Image(_)
            if revision.runtime.is_none() && revision.handler.is_none() =>
        {
            image(revision)?;
        }
        DeploymentArtifact::Zip(_) if revision.image_config.is_none() => {
            validate_provided_settings(revision.runtime.as_deref(), revision.handler.as_deref())
                .map_err(|_| DockerError::Configuration)?;
        }
        _ => return Err(DockerError::Configuration),
    }
    Ok(())
}

fn body(
    image: &PinnedImage,
    overrides: Option<&ImageConfig>,
    user: &str,
    labels: &HashMap<String, String>,
    role: &str,
) -> Result<ContainerCreateBody, DockerError> {
    let plan = image.resolved().metadata().execution.plan(overrides)?;
    let mut labels = labels.clone();
    labels.insert("rustack.lambda.role".into(), role.into());
    Ok(ContainerCreateBody {
        image: Some(image.resolved().metadata().image_id.as_str().into()),
        entrypoint: Some(plan.entrypoint().to_vec()),
        cmd: Some(plan.command().to_vec()),
        working_dir: Some(plan.working_directory().into()),
        user: Some(user.into()),
        labels: Some(labels),
        tty: Some(false),
        attach_stdin: Some(false),
        attach_stdout: Some(true),
        attach_stderr: Some(true),
        open_stdin: Some(false),
        stdin_once: Some(false),
        healthcheck: Some(HealthConfig {
            test: Some(vec!["NONE".into()]),
            ..Default::default()
        }),
        ..Default::default()
    })
}

fn limits(
    network: String,
    memory: u64,
    cpu: i64,
    pids: i64,
    temporary: u64,
) -> Result<HostConfig, DockerError> {
    let memory = i64::try_from(memory).map_err(|_| DockerError::Configuration)?;
    Ok(HostConfig {
        cgroupns_mode: Some(HostConfigCgroupnsModeEnum::PRIVATE),
        network_mode: Some(network),
        memory: Some(memory),
        memory_swap: Some(memory),
        oom_kill_disable: Some(false),
        nano_cpus: Some(cpu),
        pids_limit: Some(pids),
        readonly_rootfs: Some(true),
        cap_drop: Some(vec!["ALL".into()]),
        security_opt: Some(vec!["no-new-privileges:true".into()]),
        privileged: Some(false),
        publish_all_ports: Some(false),
        auto_remove: Some(false),
        restart_policy: Some(RestartPolicy {
            name: Some(RestartPolicyNameEnum::NO),
            maximum_retry_count: Some(0),
        }),
        log_config: Some(HostConfigLogConfig {
            typ: Some("none".into()),
            config: None,
        }),
        tmpfs: Some(HashMap::from([(
            "/tmp".into(),
            format!("rw,nosuid,nodev,size={temporary},mode=1777"),
        )])),
        ..Default::default()
    })
}

fn image_user(value: &str, fallback: &str) -> Result<String, DockerError> {
    if value.is_empty() {
        return Ok(fallback.into());
    }
    let mut parts = value.split(':');
    let user = parts.next().ok_or(DockerError::Configuration)?;
    let group = parts.next();
    if parts.next().is_some() || !account(user) || group.is_some_and(|group| !account(group)) {
        return Err(DockerError::Configuration);
    }
    if user == "root" || user.parse::<u32>().is_ok_and(|uid| uid == 0) {
        Ok(fallback.into())
    } else {
        Ok(value.into())
    }
}
fn account(value: &str) -> bool {
    if value.is_empty() || value.len() > 64 {
        return false;
    }
    if value.bytes().all(|byte| byte.is_ascii_digit()) {
        return value.parse::<u32>().is_ok_and(|id| id <= 2_147_483_647);
    }
    value
        .as_bytes()
        .first()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
}

fn function_environment(
    revision: &ExecutionRevision,
    environment: EnvironmentId,
    image: &PinnedImage,
) -> Result<Vec<String>, DockerError> {
    if revision.environment.len() > 130 {
        return Err(DockerError::Configuration);
    }
    validate_environment_entries(
        revision
            .environment
            .iter()
            .filter(|(key, _)| !matches!(key.as_str(), "AWS_REGION" | "AWS_DEFAULT_REGION")),
    )
    .map_err(|_| DockerError::Configuration)?;
    let region = revision
        .environment
        .get("AWS_REGION")
        .ok_or(DockerError::Configuration)?;
    if !(1..=64).contains(&region.len())
        || !region
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || revision.environment.get("AWS_DEFAULT_REGION") != Some(region)
    {
        return Err(DockerError::Configuration);
    }
    let mut output: BTreeMap<String, Option<String>> = BTreeMap::new();
    for entry in &image.resolved().metadata().execution.metadata().environment {
        let (key, value) = entry
            .split_once('=')
            .map_or((entry.as_str(), None), |(key, value)| {
                (key, Some(value.to_owned()))
            });
        output.insert(key.to_owned(), value);
    }
    for (key, value) in &revision.environment {
        output.insert(key.clone(), Some(value.clone()));
    }
    let authoritative = [
        "AWS_LAMBDA_RUNTIME_API=127.0.0.1:9001".into(),
        format!("AWS_LAMBDA_FUNCTION_NAME={}", revision.function_name),
        format!("AWS_LAMBDA_FUNCTION_VERSION={}", revision.qualifier),
        format!("AWS_LAMBDA_FUNCTION_MEMORY_SIZE={}", revision.memory_mb),
        format!("AWS_LAMBDA_FUNCTION_TIMEOUT={}", revision.timeout.as_secs()),
        "AWS_LAMBDA_INITIALIZATION_TYPE=on-demand".into(),
        format!(
            "AWS_LAMBDA_LOG_GROUP_NAME=/aws/lambda/{}",
            revision.function_name
        ),
        format!("AWS_LAMBDA_LOG_STREAM_NAME=rustack/{environment}"),
        "LAMBDA_TASK_ROOT=/var/task".into(),
    ];
    if let DeploymentArtifact::Zip(_) = &revision.artifact {
        output.insert(
            "_HANDLER".into(),
            Some(revision.handler.clone().ok_or(DockerError::Configuration)?),
        );
        output.insert(
            "AWS_EXECUTION_ENV".into(),
            Some("AWS_Lambda_provided.al2023".into()),
        );
    }
    for entry in authoritative {
        let (key, value) = entry.split_once('=').ok_or(DockerError::Configuration)?;
        output.insert(key.to_owned(), Some(value.to_owned()));
    }
    Ok(output
        .into_iter()
        .map(|(key, value)| match value {
            Some(value) => format!("{key}={value}"),
            None => key,
        })
        .collect())
}

#[cfg(test)]
#[path = "plan_tests.rs"]
pub(super) mod tests;
