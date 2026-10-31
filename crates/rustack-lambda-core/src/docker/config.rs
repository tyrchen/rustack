//! Pure operator settings; private credential IO has separate retained ownership.

mod auth;

use std::{
    fmt,
    path::{Component, Path, PathBuf},
    str::FromStr,
    time::Duration,
};

pub use auth::RegistryAuthError;
use rustack_core::settings::RuntimeBudgets;
use rustack_lambda_runtime::probe::GatewayEndpoint;
use tokio::time::Instant;
use typed_builder::TypedBuilder;

use super::{
    ContainerPolicy, DockerError, DockerSocket, ImagePullPolicy, NetworkSelection,
    RegistryCredentials, RegistryHost, RelayBudgets, sdk::LocalSdkCredentials,
};
use crate::artifact::ImageReference;

/// Operator-selected function endpoint, independent of request Host and readiness.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DockerGatewayEndpoint(GatewayEndpoint);
impl fmt::Debug for DockerGatewayEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DockerGatewayEndpoint(<redacted>)")
    }
}
impl FromStr for DockerGatewayEndpoint {
    type Err = DockerError;
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        raw.parse()
            .map(Self)
            .map_err(|_| DockerError::Configuration)
    }
}
impl DockerGatewayEndpoint {
    /// Canonical operator endpoint; this does not establish network reachability.
    /// ```
    /// let endpoint:rustack_lambda_core::docker::DockerGatewayEndpoint="http://localhost:4566".parse()?;
    /// assert_eq!(endpoint.as_str(),"http://localhost:4566/");
    /// # Ok::<(),rustack_lambda_core::docker::DockerError>(())
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl From<DockerGatewayEndpoint> for GatewayEndpoint {
    fn from(endpoint: DockerGatewayEndpoint) -> Self {
        endpoint.0
    }
}

/// Explicit absolute private-file path; never searched or expanded.
///
/// Requires a controlled local filesystem and ACLs granting no untrusted
/// credential read/write or ancestor replacement; POSIX checks do not prove ACL privacy.
#[derive(Clone)]
#[non_exhaustive]
pub struct RegistryAuthFile(PathBuf);
impl fmt::Debug for RegistryAuthFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RegistryAuthFile(<redacted>)")
    }
}
impl FromStr for RegistryAuthFile {
    type Err = DockerError;
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let path = Path::new(raw);
        if raw.is_empty()
            || raw.len() > 4096
            || raw.ends_with('/')
            || raw.chars().any(char::is_control)
            || !path.is_absolute()
            || path
                .components()
                .any(|part| matches!(part, Component::ParentDir))
            || path.file_name().is_none()
        {
            return Err(DockerError::Configuration);
        }
        Ok(Self(path.into()))
    }
}
impl RegistryAuthFile {
    /// Read an explicitly approved private credential document under a total budget.
    /// # Errors
    /// Rejects expiry, unsafe files, IO/worker failure or invalid bounded YAML/policy.
    /// ```no_run
    /// # async fn example()->Result<(),rustack_lambda_core::docker::RegistryAuthError>{
    /// let path:rustack_lambda_core::docker::RegistryAuthFile="/private/registry.yaml".parse().map_err(|_|rustack_lambda_core::docker::RegistryAuthError::Configuration)?;
    /// let credentials=path.load(vec!["registry.example".parse().map_err(|_|rustack_lambda_core::docker::RegistryAuthError::Configuration)?],tokio::time::Instant::now()+std::time::Duration::from_secs(5)).await?;
    /// # Ok(()) }
    /// ```
    pub async fn load(
        &self,
        approved: Vec<RegistryHost>,
        deadline: Instant,
    ) -> Result<RegistryCredentials, RegistryAuthError> {
        auth::load(self.0.clone(), approved, deadline).await
    }
}

#[derive(Clone)]
struct PullSettings {
    hosts: Vec<RegistryHost>,
    file: Option<RegistryAuthFile>,
}
impl fmt::Debug for PullSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PullSettings(<redacted>)")
    }
}
/// Immutable operator configuration; constructing it performs no IO or activation.
///
/// Only the validated constructor is public:
/// ```compile_fail
/// rustack_lambda_core::docker::DockerRuntimeSettings::builder();
/// ```
#[derive(Clone, TypedBuilder)]
#[builder(builder_method(vis = ""))]
#[non_exhaustive]
pub struct DockerRuntimeSettings {
    socket: DockerSocket,
    relay: ImageReference,
    zip: Option<ImageReference>,
    endpoint: Option<DockerGatewayEndpoint>,
    sdk_credentials: Option<LocalSdkCredentials>,
    policy: ContainerPolicy,
    budgets: RuntimeBudgets,
    log_ring_bytes: usize,
    cross_architecture: bool,
    pull: Option<PullSettings>,
}
impl fmt::Debug for DockerRuntimeSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DockerRuntimeSettings(<redacted>)")
    }
}
impl DockerRuntimeSettings {
    pub(super) fn sdk_credentials(&self) -> Option<&LocalSdkCredentials> {
        self.sdk_credentials.as_ref()
    }
    /// Validate explicit settings and shared budgets without reading files or Docker.
    /// # Errors
    /// Rejects missing required settings, malformed domain values or inconsistent budgets.
    /// ```
    /// let settings=rustack_lambda_core::docker::DockerRuntimeSettings::from_lookup(|key|match key{
    /// "LAMBDA_DOCKER_RELAY_IMAGE"=>Some("relay:v1".into()),"LAMBDA_DOCKER_NETWORK"=>Some("none".into()),_=>None},&Default::default(),std::time::Duration::from_secs(600))?;
    /// assert!(!settings.cross_architecture());
    /// # Ok::<(),rustack_lambda_core::docker::DockerError>(())
    /// ```
    pub fn from_lookup(
        lookup: impl Fn(&str) -> Option<String>,
        budgets: &RuntimeBudgets,
        idle: Duration,
    ) -> Result<Self, DockerError> {
        budgets.validate().map_err(|_| DockerError::Configuration)?;
        let socket = lookup("LAMBDA_DOCKER_SOCKET")
            .unwrap_or_else(|| "unix:///var/run/docker.sock".into())
            .parse()?;
        let relay = lookup("LAMBDA_DOCKER_RELAY_IMAGE")
            .ok_or(DockerError::Configuration)?
            .parse()?;
        let network: NetworkSelection = lookup("LAMBDA_DOCKER_NETWORK")
            .ok_or(DockerError::Configuration)?
            .parse()?;
        let io = Duration::from_secs(budgets.lambda_docker_io_seconds);
        let cleanup = Duration::from_secs(budgets.lambda_docker_cleanup_seconds);
        let policy = ContainerPolicy::new(
            network,
            number(&lookup, "UID", 1000, 1, 2_147_483_647)?
                .try_into()
                .map_err(|_| DockerError::Configuration)?,
            number(&lookup, "GID", 1000, 1, 2_147_483_647)?
                .try_into()
                .map_err(|_| DockerError::Configuration)?,
            cpu(&lookup("LAMBDA_DOCKER_CPU").unwrap_or_else(|| "1".into()))?,
            RelayBudgets::new(io, idle, cleanup)?,
        )?;
        let cross_architecture = match lookup("LAMBDA_DOCKER_CROSS_ARCHITECTURE").as_deref() {
            None | Some("false") => false,
            Some("true") => true,
            _ => return Err(DockerError::Configuration),
        };
        let endpoint = lookup("LAMBDA_DOCKER_GATEWAY_ENDPOINT")
            .map(|raw| raw.parse::<DockerGatewayEndpoint>())
            .transpose()?;
        let sdk_credentials = LocalSdkCredentials::parse(
            lookup("LAMBDA_DOCKER_SDK_ACCESS_KEY_ID"),
            lookup("LAMBDA_DOCKER_SDK_SECRET_ACCESS_KEY"),
        )?;
        if sdk_credentials.is_some() && (endpoint.is_none() || policy.network.value() == "none") {
            return Err(DockerError::Configuration);
        }
        Ok(Self::builder()
            .socket(socket)
            .relay(relay)
            .zip(
                lookup("LAMBDA_DOCKER_ZIP_IMAGE")
                    .map(|raw| {
                        let reference: ImageReference = raw.parse()?;
                        if reference.digest().is_none() {
                            return Err(DockerError::Configuration);
                        }
                        Ok(reference)
                    })
                    .transpose()?,
            )
            .endpoint(endpoint)
            .sdk_credentials(sdk_credentials)
            .policy(policy)
            .budgets(budgets.clone())
            .log_ring_bytes(
                number(&lookup, "LOG_RING_BYTES", 262_144, 4096, 1_048_576)?
                    .try_into()
                    .map_err(|_| DockerError::Configuration)?,
            )
            .cross_architecture(cross_architecture)
            .pull(parse_pull(&lookup)?)
            .build())
    }
    /// Load only explicit credentials and construct the existing pull policy.
    /// # Errors
    /// Returns bounded file/schema/worker/deadline or policy errors; never falls back.
    /// ```no_run
    /// # async fn load(settings:&rustack_lambda_core::docker::DockerRuntimeSettings)->Result<(),rustack_lambda_core::docker::RegistryAuthError>{
    /// let policy=settings.load_pull_policy(tokio::time::Instant::now()+std::time::Duration::from_secs(5)).await?;
    /// # Ok(()) }
    /// ```
    pub async fn load_pull_policy(
        &self,
        deadline: Instant,
    ) -> Result<ImagePullPolicy, RegistryAuthError> {
        auth::valid_deadline(deadline)?;
        let Some(pull) = &self.pull else {
            return Ok(ImagePullPolicy::default());
        };
        let credentials = match &pull.file {
            Some(file) => file.load(pull.hosts.clone(), deadline).await?,
            None => RegistryCredentials::default(),
        };
        let policy = ImagePullPolicy::if_missing(
            pull.hosts.clone(),
            credentials,
            Duration::from_secs(self.budgets.lambda_docker_pull_seconds),
        )
        .map_err(|_| RegistryAuthError::Configuration)?;
        auth::valid_deadline(deadline)?;
        Ok(policy)
    }
    /// Validated local socket.
    /// ```no_run
    /// # fn inspect(s:&rustack_lambda_core::docker::DockerRuntimeSettings){ let socket=s.socket(); }
    /// ```
    #[must_use]
    pub fn socket(&self) -> &DockerSocket {
        &self.socket
    }
    /// Relay source selected by the operator; not yet resolved or pinned.
    /// ```no_run
    /// # fn inspect(s:&rustack_lambda_core::docker::DockerRuntimeSettings){ let source=s.relay_image(); }
    /// ```
    #[must_use]
    pub fn relay_image(&self) -> &ImageReference {
        &self.relay
    }
    /// Optional ZIP runtime source; no implicit catalog.
    /// ```no_run
    /// # fn inspect(s:&rustack_lambda_core::docker::DockerRuntimeSettings){ let source=s.zip_image(); }
    /// ```
    #[must_use]
    pub fn zip_image(&self) -> Option<&ImageReference> {
        self.zip.as_ref()
    }
    /// Optional endpoint; parsing is not reachability evidence.
    /// ```no_run
    /// # fn inspect(s:&rustack_lambda_core::docker::DockerRuntimeSettings){ let endpoint=s.gateway_endpoint(); }
    /// ```
    #[must_use]
    pub fn gateway_endpoint(&self) -> Option<&DockerGatewayEndpoint> {
        self.endpoint.as_ref()
    }
    /// Validated immutable container policy.
    /// ```no_run
    /// # fn inspect(s:&rustack_lambda_core::docker::DockerRuntimeSettings){ let policy=s.container_policy(); }
    /// ```
    #[must_use]
    pub fn container_policy(&self) -> &ContainerPolicy {
        &self.policy
    }
    /// Validated shared runtime budgets captured for startup.
    /// ```no_run
    /// # fn inspect(s:&rustack_lambda_core::docker::DockerRuntimeSettings){ let limits=s.budgets(); }
    /// ```
    #[must_use]
    pub fn budgets(&self) -> &RuntimeBudgets {
        &self.budgets
    }
    /// Per-channel bounded function log storage.
    /// ```no_run
    /// # fn inspect(s:&rustack_lambda_core::docker::DockerRuntimeSettings){ let bytes=s.log_ring_bytes(); }
    /// ```
    #[must_use]
    pub fn log_ring_bytes(&self) -> usize {
        self.log_ring_bytes
    }
    /// Explicit permission to qualify an emulated architecture path.
    /// ```no_run
    /// # fn inspect(s:&rustack_lambda_core::docker::DockerRuntimeSettings){ let emulation=s.cross_architecture(); }
    /// ```
    #[must_use]
    pub fn cross_architecture(&self) -> bool {
        self.cross_architecture
    }
}
fn parse_pull(
    lookup: &impl Fn(&str) -> Option<String>,
) -> Result<Option<PullSettings>, DockerError> {
    let hosts = lookup("LAMBDA_DOCKER_REGISTRIES");
    let file = lookup("LAMBDA_DOCKER_REGISTRY_AUTH_FILE");
    match lookup("LAMBDA_DOCKER_PULL_POLICY").as_deref() {
        None | Some("never") => {
            if hosts.is_some() || file.is_some() {
                return Err(DockerError::Configuration);
            }
            Ok(None)
        }
        Some("ifMissing") => {
            let raw = hosts.ok_or(DockerError::Configuration)?;
            if raw.len() > 8127 || raw.split(',').count() > 32 {
                return Err(DockerError::Configuration);
            }
            let hosts: Vec<RegistryHost> =
                raw.split(',').map(str::parse).collect::<Result<_, _>>()?;
            ImagePullPolicy::if_missing(
                hosts.clone(),
                RegistryCredentials::default(),
                Duration::from_secs(300),
            )?;
            Ok(Some(PullSettings {
                hosts,
                file: file.map(|raw| raw.parse()).transpose()?,
            }))
        }
        _ => Err(DockerError::Configuration),
    }
}
fn number(
    lookup: &impl Fn(&str) -> Option<String>,
    suffix: &str,
    default: u64,
    min: u64,
    max: u64,
) -> Result<u64, DockerError> {
    let value = match lookup(&format!("LAMBDA_DOCKER_{suffix}")) {
        None => default,
        Some(raw) => {
            if raw.len() > 20 || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(DockerError::Configuration);
            }
            raw.parse().map_err(|_| DockerError::Configuration)?
        }
    };
    if !(min..=max).contains(&value) {
        return Err(DockerError::Configuration);
    }
    Ok(value)
}
fn cpu(raw: &str) -> Result<u64, DockerError> {
    if raw.is_empty() || raw.len() > 12 {
        return Err(DockerError::Configuration);
    }
    let (whole, fraction) = raw
        .split_once('.')
        .map_or((raw, ""), |(whole, fraction)| (whole, fraction));
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > 9
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || (raw.contains('.') && fraction.is_empty())
    {
        return Err(DockerError::Configuration);
    }
    let whole: u64 = whole.parse().map_err(|_| DockerError::Configuration)?;
    let fraction_value = if fraction.is_empty() {
        0
    } else {
        fraction
            .parse::<u64>()
            .map_err(|_| DockerError::Configuration)?
    };
    let scale = 10_u64
        .checked_pow(
            u32::try_from(9_usize.saturating_sub(fraction.len()))
                .map_err(|_| DockerError::Configuration)?,
        )
        .ok_or(DockerError::Configuration)?;
    let nano = whole
        .checked_mul(1_000_000_000)
        .and_then(|value| {
            fraction_value
                .checked_mul(scale)
                .and_then(|fraction| value.checked_add(fraction))
        })
        .ok_or(DockerError::Configuration)?;
    if !(100_000_000..=16_000_000_000).contains(&nano) {
        return Err(DockerError::Configuration);
    }
    Ok(nano)
}

#[cfg(test)]
mod tests;
