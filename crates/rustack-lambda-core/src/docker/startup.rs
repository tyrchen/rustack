//! Actual startup receipts and one ordered cleanup owner, never request-supplied readiness.

mod qualify;
use std::{
    fmt,
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use serde::Serialize;
use thiserror::Error;
use tokio::{
    runtime::Handle,
    sync::{Semaphore, SemaphorePermit},
    task::JoinHandle,
    time::{Instant, timeout_at},
};
use typed_builder::TypedBuilder;

use super::{
    CodeVolumeManager, DirectoryLease, DirectoryLeaseError, DockerCpuAllocation, DockerError,
    DockerExecutor, DockerExecutorError, DockerPort, DockerReadiness, DockerRuntimeSettings,
    DockerSdkEnvironment, DockerWire, DockerZipRuntime, EndpointProbe, EndpointProbeConfig,
    EnvironmentCapacity, EnvironmentError, EnvironmentManager, EnvironmentProbe,
    EnvironmentProbeConfig, EnvironmentProbeError, GatewayInstanceId, ImageLeaseManager,
    ImageLeasePort, RegistryAuthError,
    ownership::{DataDirectory, DirectoryAcquisition},
    sdk::{LocalSdkCredentials, SdkAffiliation},
};
use crate::{
    artifact::{Architecture, PinnedImage},
    config::LambdaConfig,
    deployment::ImageDeploymentConfig,
    error::LambdaServiceError,
    executor::{Executor, ExecutorBackend, ExecutorError},
    invocation::CleanupTask,
    provider::RustackLambda,
    storage::FunctionStore,
};

static STARTUPS: Semaphore = Semaphore::const_new(4);

/// Static startup errors retain typed sources without displaying operator values.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DockerStartupError {
    /// Invalid or mismatching operator inputs.
    #[error("invalid Docker startup configuration")]
    Configuration,
    /// This attempt was already begun, failed, abandoned or stopped.
    #[error("Docker startup attempt is unavailable")]
    Unavailable,
    /// Global startup or cleanup observation expired.
    #[error("Docker startup observation deadline exceeded")]
    Deadline,
    /// Current runtime or bounded startup admission is unavailable.
    #[error("Docker startup supervision unavailable")]
    Worker,
    /// Unsafe, legacy, busy or damaged namespace.
    #[error("Docker startup ownership failed")]
    Directory(#[source] Arc<DirectoryLeaseError>),
    /// Bounded daemon or image failure.
    #[error("Docker startup daemon failed: {0}")]
    Docker(#[from] DockerError),
    /// The selected relay could not be prepared.
    #[error("Docker startup relay preparation failed: {0}")]
    Relay(#[source] DockerError),
    /// The configured endpoint failed its actual container-origin proof.
    #[error("Docker startup endpoint qualification failed: {0}")]
    Endpoint(#[source] EnvironmentProbeError),
    /// Actual resource qualification failed.
    #[error("Docker startup probe failed: {0}")]
    Probe(#[from] EnvironmentProbeError),
    /// Retained runtime allocation failed.
    #[error("Docker startup environment failed: {0}")]
    Environment(#[from] EnvironmentError),
    /// Private credential loading failed.
    #[error("Docker startup credentials failed: {0}")]
    Credentials(#[from] RegistryAuthError),
    /// Executor/provider construction failed.
    #[error("Docker startup installation failed")]
    Installation(#[source] Arc<LambdaServiceError>),
    /// Runtime graph has not confirmed physical cleanup.
    #[error("Docker runtime cleanup is not confirmed")]
    Cleanup(#[source] ExecutorError),
}
impl From<DirectoryLeaseError> for DockerStartupError {
    fn from(error: DirectoryLeaseError) -> Self {
        Self::Directory(Arc::new(error))
    }
}
impl From<DockerExecutorError> for DockerStartupError {
    fn from(error: DockerExecutorError) -> Self {
        Self::Cleanup(error.into())
    }
}

/// Validated immutable production settings, including the exact provider configuration.
#[derive(Clone)]
#[non_exhaustive]
pub struct DockerStartupConfig {
    settings: DockerRuntimeSettings,
    directory: PathBuf,
    instance: GatewayInstanceId,
    lambda: LambdaConfig,
}
impl fmt::Debug for DockerStartupConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DockerStartupConfig(<redacted>)")
    }
}
impl DockerStartupConfig {
    /// Validate one selected image runtime before any listener or filesystem IO.
    /// # Errors
    /// Rejects non-Docker settings, invalid directory, init/idle/warm limits or a
    /// configured endpoint paired with disconnected networking.
    /// ```no_run
    /// # fn config(settings:rustack_lambda_core::docker::DockerRuntimeSettings,lambda:rustack_lambda_core::config::LambdaConfig,id:rustack_lambda_core::docker::GatewayInstanceId)->Result<(),rustack_lambda_core::docker::DockerStartupError>{
    /// let config=rustack_lambda_core::docker::DockerStartupConfig::new(settings,"/private/data".into(),id,lambda)?;
    /// # Ok(()) }
    /// ```
    pub fn new(
        settings: DockerRuntimeSettings,
        directory: PathBuf,
        instance: GatewayInstanceId,
        lambda: LambdaConfig,
    ) -> Result<Self, DockerStartupError> {
        DataDirectory::try_from(directory.clone())?;
        if settings.zip_image().is_some() && settings.budgets().lambda_docker_image_pins < 2 {
            return Err(DockerStartupError::Configuration);
        }

        if lambda.executor != ExecutorBackend::Docker
            || lambda.max_warm_instances != 1
            || lambda.init_timeout.subsec_nanos() != 0
            || !(1..=900).contains(&lambda.init_timeout.as_secs())
            || lambda.idle_timeout != settings.container_policy().idle()
            || (settings.gateway_endpoint().is_some()
                && settings.container_policy().network.value() == "none")
        {
            return Err(DockerStartupError::Configuration);
        }
        Ok(Self {
            settings,
            directory,
            instance,
            lambda,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    New,
    Qualifying,
    Ready,
    Failed,
    Stopping,
}

#[derive(Debug, TypedBuilder)]
struct Evidence {
    base: EnvironmentProbeConfig,
    resource: EnvironmentProbe,
    endpoint: Option<(EndpointProbeConfig, EndpointProbe)>,
    images: ImageLeasePort,
    docker: DockerPort,
    architecture: Architecture,
    #[builder(default)]
    zip: Option<DockerZipRuntime>,
}
#[derive(Debug, Default)]
struct Qualification {
    installed: AtomicBool,
    revoked: AtomicBool,
    evidence: OnceLock<Evidence>,
}
impl Qualification {
    fn sdk_environment(
        self: &Arc<Self>,
        credentials: LocalSdkCredentials,
    ) -> Result<DockerSdkEnvironment, DockerStartupError> {
        self.check().map_err(|_| DockerStartupError::Unavailable)?;
        let evidence = self.evidence.get().ok_or(DockerStartupError::Unavailable)?;
        let (endpoint, _) = evidence
            .endpoint
            .as_ref()
            .ok_or(DockerStartupError::Configuration)?;
        let readiness: Arc<dyn DockerReadiness> = self.clone();
        Ok(DockerSdkEnvironment::new(
            credentials,
            endpoint.gateway_endpoint().clone(),
            SdkAffiliation::new(
                evidence.base.directory.clone(),
                evidence.base.relay.clone(),
                evidence.base.policy.clone(),
                evidence.base.environments.clone(),
                readiness,
            ),
        )?)
    }
}
impl DockerReadiness for Qualification {
    fn check(&self) -> Result<(), DockerExecutorError> {
        if self.revoked.load(Ordering::Acquire) {
            return Err(DockerExecutorError::Unavailable);
        }
        let evidence = self
            .evidence
            .get()
            .ok_or(DockerExecutorError::Unavailable)?;
        let environment = evidence.base.environments.status();
        if evidence.base.directory.require_journaled().is_err()
            || evidence
                .docker
                .require_directory(&evidence.base.directory)
                .is_err()
            || *evidence.docker.raw.stop.borrow()
            || evidence.images.available().is_err()
            || evidence
                .zip
                .as_ref()
                .is_some_and(|zip| zip.available().is_err())
            || !environment.accepting
            || environment.worker_failed
            || !evidence.resource.matches(&evidence.base)
            || evidence
                .endpoint
                .as_ref()
                .is_some_and(|(config, proof)| !proof.matches(config))
        {
            return Err(DockerExecutorError::Unavailable);
        }
        Ok(())
    }
}

/// Read-only live backend diagnostics; cloning never transfers shutdown authority.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DockerRuntimeStatus(Arc<Qualification>);
/// Bounded capability data, separate from private startup receipts.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct DockerCapabilities {
    /// Requested/effective CPU pair from retained kernel-qualified policy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu: Option<DockerCpuAllocation>,
    /// Actual retained allocation/memory charges, absent before qualification.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capacity: Option<EnvironmentCapacity>,
    /// Actual installed owner health and receipt validity.
    pub ready: bool,
    /// Qualified Linux platform, absent before successful qualification.
    pub platform: Option<&'static str>,
    /// Whether a configured gateway passed the actual endpoint proof.
    pub sdk_gateway: bool,
    /// Qualified provided.al2023 execution through the installed code owner.
    pub zip: bool,
}
impl DockerRuntimeStatus {
    /// Observe current receipt/owner health without IO or peer data.
    /// ```no_run
    /// # fn status(status:&rustack_lambda_core::docker::DockerRuntimeStatus){let capabilities=status.capabilities();}
    /// ```
    #[must_use]
    pub fn capabilities(&self) -> DockerCapabilities {
        let evidence = self.0.evidence.get();
        DockerCapabilities {
            cpu: evidence.map(|evidence| evidence.base.policy.cpu_allocation()),
            capacity: evidence.map(|evidence| evidence.base.environments.status()),
            ready: self.0.installed.load(Ordering::Acquire) && self.0.check().is_ok(),
            platform: evidence.map(|e| match e.architecture {
                Architecture::Arm64 => "linux/arm64",
                Architecture::X86_64 => "linux/amd64",
            }),
            sdk_gateway: evidence.is_some_and(|e| e.endpoint.is_some()),
            zip: self.0.installed.load(Ordering::Acquire)
                && self.0.check().is_ok()
                && evidence.is_some_and(|e| e.zip.is_some()),
        }
    }
}

/// Qualified provider installation, issued only by the private startup owner.
#[derive(Clone)]
#[non_exhaustive]
pub struct DockerInstallation {
    lambda: LambdaConfig,
    executor: Arc<DockerExecutor>,
    images: ImageLeasePort,
    deployment: ImageDeploymentConfig,
}
impl fmt::Debug for DockerInstallation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DockerInstallation(<retained>)")
    }
}
impl DockerInstallation {
    /// Install the exact validated provider over its real executor and image owner.
    /// # Errors
    /// Rejects revoked startup evidence or failed deployment construction.
    /// ```no_run
    /// # fn install(installation:&rustack_lambda_core::docker::DockerInstallation,store:rustack_lambda_core::storage::FunctionStore)->Result<(),rustack_lambda_core::docker::DockerStartupError>{
    /// let provider=installation.provider(store)?;
    /// # Ok(()) }
    /// ```
    pub fn provider(&self, store: FunctionStore) -> Result<RustackLambda, DockerStartupError> {
        self.executor
            .image_available()
            .map_err(DockerStartupError::Cleanup)?;
        RustackLambda::with_executor(store, self.lambda.clone(), self.executor.clone())
            .with_image_preparation(self.images.clone(), self.deployment)
            .map_err(|error| DockerStartupError::Installation(Arc::new(error)))
    }
}

#[derive(Debug, TypedBuilder)]
struct State {
    config: DockerStartupConfig,
    _admission: SemaphorePermit<'static>,
    #[builder(default)]
    directory: Option<DirectoryLease>,
    #[builder(default)]
    acquiring: Option<DirectoryAcquisition>,
    #[builder(default)]
    connecting: Option<JoinHandle<Result<DockerWire, DockerError>>>,
    #[builder(default)]
    fault: Option<ExecutorError>,
    #[builder(default)]
    wire: Option<DockerWire>,
    #[builder(default)]
    bound: Option<DockerPort>,
    #[builder(default)]
    producing: bool,
    #[builder(default)]
    images: Option<ImageLeaseManager>,
    #[builder(default)]
    code: Option<CodeVolumeManager>,
    #[builder(default)]
    zip: Option<DockerZipRuntime>,
    #[builder(default)]
    environments: Option<EnvironmentManager>,
    #[builder(default)]
    relay: Option<PinnedImage>,
    #[builder(default)]
    executor: Option<Arc<DockerExecutor>>,
}
/// Unique startup/shutdown authority for the exact production runtime graph.
#[derive(Debug)]
#[non_exhaustive]
pub struct DockerRuntime {
    state: Option<State>,
    phase: Phase,
    qualification: Arc<Qualification>,
    runtime: Handle,
    cleanup: CleanupTask,
}
impl DockerRuntime {
    /// Reserve a bounded startup owner without IO; qualification is one attempt.
    /// # Errors
    /// Rejects absent runtime or four already-retained runtime owners.
    /// ```no_run
    /// # fn begin(config:rustack_lambda_core::docker::DockerStartupConfig)->Result<(),rustack_lambda_core::docker::DockerStartupError>{
    /// let owner=rustack_lambda_core::docker::DockerRuntime::new(config)?;
    /// # Ok(()) }
    /// ```
    pub fn new(config: DockerStartupConfig) -> Result<Self, DockerStartupError> {
        let runtime = Handle::try_current().map_err(|_| DockerStartupError::Worker)?;
        let admission = STARTUPS
            .try_acquire()
            .map_err(|_| DockerStartupError::Worker)?;
        Ok(Self {
            state: Some(
                State::builder()
                    .config(config)
                    ._admission(admission)
                    .build(),
            ),
            phase: Phase::New,
            qualification: Arc::new(Qualification::default()),
            runtime,
            cleanup: CleanupTask::default(),
        })
    }
    /// Consume actual receipts once under the supplied absolute process deadline.
    /// # Errors
    /// Returns typed qualification failure. Dropping this observer cannot replay
    /// the attempt; retain this owner for shutdown, or Drop starts that same cleanup.
    /// ```no_run
    /// # async fn qualify(owner:&mut rustack_lambda_core::docker::DockerRuntime){
    /// let result=owner.qualify(tokio::time::Instant::now()+std::time::Duration::from_secs(600)).await;
    /// # }
    /// ```
    pub async fn qualify(&mut self, deadline: Instant) -> Result<(), DockerStartupError> {
        if self.phase != Phase::New {
            return Err(DockerStartupError::Unavailable);
        }
        qualify::deadline(deadline)?;
        self.phase = Phase::Qualifying;
        let state = self.state.as_mut().ok_or(DockerStartupError::Unavailable)?;
        let result = Box::pin(state.qualify(&self.qualification, deadline)).await;
        self.phase = if result.is_ok() {
            self.qualification.installed.store(true, Ordering::Release);
            Phase::Ready
        } else {
            self.qualification.revoked.store(true, Ordering::Release);
            Phase::Failed
        };
        result
    }
    /// Obtain a qualified installation without granting alternate configuration.
    /// # Errors
    /// Rejects incomplete, failed, abandoned, stopped or revoked qualification.
    /// ```no_run
    /// # fn installation(owner:&rustack_lambda_core::docker::DockerRuntime){let installation=owner.installation();}
    /// ```
    pub fn installation(&self) -> Result<DockerInstallation, DockerStartupError> {
        if self.phase != Phase::Ready {
            return Err(DockerStartupError::Unavailable);
        }
        self.qualification
            .check()
            .map_err(|_| DockerStartupError::Unavailable)?;
        let state = self.state.as_ref().ok_or(DockerStartupError::Unavailable)?;
        let budgets = state.config.settings.budgets();
        Ok(DockerInstallation {
            lambda: state.config.lambda.clone(),
            executor: state
                .executor
                .as_ref()
                .ok_or(DockerStartupError::Unavailable)?
                .clone(),
            images: state
                .images
                .as_ref()
                .ok_or(DockerStartupError::Unavailable)?
                .port(),
            deployment: ImageDeploymentConfig::new(
                budgets
                    .lambda_docker_deployment_jobs
                    .try_into()
                    .map_err(|_| DockerStartupError::Configuration)?,
                Duration::from_secs(budgets.lambda_docker_deployment_seconds),
            )
            .map_err(|error| DockerStartupError::Installation(Arc::new(error)))?,
        })
    }
    /// Retain live read-only capability observations, without startup authority.
    /// ```no_run
    /// # fn status(owner:&rustack_lambda_core::docker::DockerRuntime){let status=owner.status();}
    /// ```
    #[must_use]
    pub fn status(&self) -> DockerRuntimeStatus {
        DockerRuntimeStatus(self.qualification.clone())
    }
    fn start_cleanup(&mut self) {
        self.phase = Phase::Stopping;
        self.qualification.revoked.store(true, Ordering::Release);
        if let Some(state) = self.state.take() {
            self.cleanup
                .start_on(&self.runtime, move || state.cleanup());
        }
    }
    /// Revoke admission and observe the same retained ordered cleanup operation.
    /// # Errors
    /// Returns expiry or failed cleanup observation without releasing unresolved
    /// ownership. A later call resumes observation; Drop keeps that operation alive.
    /// ```no_run
    /// # async fn stop(owner:&mut rustack_lambda_core::docker::DockerRuntime){let result=owner.shutdown(tokio::time::Instant::now()+std::time::Duration::from_secs(30)).await;}
    /// ```
    pub async fn shutdown(&mut self, deadline: Instant) -> Result<(), DockerStartupError> {
        self.start_cleanup();
        if Instant::now() >= deadline {
            return Err(DockerStartupError::Deadline);
        }
        timeout_at(deadline, self.cleanup.wait())
            .await
            .map_err(|_| DockerStartupError::Deadline)?
            .map_err(DockerStartupError::Cleanup)?;
        if Instant::now() >= deadline {
            return Err(DockerStartupError::Deadline);
        }
        Ok(())
    }
}
impl Drop for DockerRuntime {
    fn drop(&mut self) {
        self.start_cleanup();
    }
}

#[cfg(test)]
mod tests;
