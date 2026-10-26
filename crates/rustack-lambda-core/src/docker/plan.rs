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
use tokio::time::Instant;
use uuid::Uuid;

use super::{
    DirectoryLease, DockerError, DockerKey, DockerPort,
    models::{
        ContainerCreateBody, ContainerCreateResponse, ContainerInspectResponse, HealthConfig,
        HostConfig, HostConfigLogConfig, RestartPolicy, RestartPolicyNameEnum,
    },
    mutation::MutationGuard,
};
use crate::{
    artifact::{DeploymentArtifact, PinnedImage},
    executor::{ExecutionRevision, ExecutorBackend},
    function_config::validate_environment_entries,
};

const MIB: u64 = 1024 * 1024;
const RELAY_MEMORY: u64 = 64 * MIB;

#[derive(Debug, Clone, Copy)]
pub(super) enum ContainerRole {
    Relay,
    Function,
}

/// Operator network selection; a bridge still requires daemon startup validation.
#[derive(Clone)]
#[non_exhaustive]
pub struct NetworkSelection(String);
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
#[derive(Debug, Clone, Copy)]
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
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ContainerPolicy {
    network: NetworkSelection,
    uid: u32,
    gid: u32,
    nano_cpus: i64,
    relay: RelayBudgets,
}
impl ContainerPolicy {
    /// Validate nonzero signed32-bit UID/GID and0.1..16 CPU allocation.
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
            nano_cpus: i64::try_from(nano_cpus).map_err(|_| DockerError::Configuration)?,
            relay,
        })
    }
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
#[non_exhaustive]
pub struct EnvironmentPlan {
    pub(super) directory: DirectoryLease,
    relay: PinnedImage,
    revision: Arc<ExecutionRevision>,
    identity: Identity,
    bodies: Bodies,
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
    pub(super) fn same_scope(&self, directory: &DirectoryLease) -> bool {
        self.directory.owner_id() == directory.owner_id()
            && self.directory.run_id() == directory.run_id()
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
        id.parse()
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
        validate_revision(&revision)?;
        let function = image(&revision)?;
        if !relay.is_live() || relay.resolved().metadata().architecture != revision.architecture {
            return Err(DockerError::Configuration);
        }
        let environment = EnvironmentId::default();
        let identity = Identity {
            environment,
            relay: format!("rustack-relay-{environment}").parse()?,
            function: format!("rustack-function-{environment}").parse()?,
        };
        let labels = HashMap::from([
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
        ]);
        let user = format!("{}:{}", policy.uid, policy.gid);
        let mut relay_body = body(&relay, None, &user, &labels, "relay")?;
        relay_body.env = Some(vec![
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
        relay_body.attach_stdin = Some(true);
        relay_body.open_stdin = Some(true);
        relay_body.stdin_once = Some(true);
        relay_body.host_config = Some(limits(
            policy.network.0,
            RELAY_MEMORY,
            100_000_000,
            64,
            MIB,
        )?);
        let selected_user = image_user(
            &function.resolved().metadata().execution.metadata().user,
            &user,
        )?;
        let mut function_body = body(
            function,
            revision.image_config.as_ref(),
            &selected_user,
            &labels,
            "function",
        )?;
        function_body.env = Some(function_environment(&revision, environment)?);
        let memory = u64::from(revision.memory_mb)
            .checked_mul(MIB)
            .ok_or(DockerError::Configuration)?;
        function_body.host_config = Some(limits(
            format!("container:{}", identity.relay.as_str()),
            memory,
            policy.nano_cpus,
            256,
            u64::from(revision.ephemeral_storage_mb)
                .checked_mul(MIB)
                .ok_or(DockerError::Configuration)?,
        )?);
        let memory = memory
            .checked_add(RELAY_MEMORY)
            .ok_or(DockerError::Configuration)?;
        Ok(Self {
            directory,
            relay,
            revision,
            identity,
            bodies: Bodies {
                relay: relay_body,
                function: function_body,
                memory,
            },
        })
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
    fn live(&self) -> Result<(), DockerError> {
        match &self.revision.artifact {
            DeploymentArtifact::Image(function) if self.relay.is_live() && function.is_live() => {
                Ok(())
            }
            DeploymentArtifact::Image(_) => Err(DockerError::Stopped),
            DeploymentArtifact::Zip(_) => Err(DockerError::Configuration),
        }
    }
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
        || revision.runtime.is_some()
        || revision.handler.is_some()
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
    image(revision)?;
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
    let image = image(revision)?;
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
