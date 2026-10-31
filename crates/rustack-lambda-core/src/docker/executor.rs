//! Trusted runtime composition; startup qualification remains coordinator-owned.
//!
//! The synchronous readiness trait is object-safe for trusted in-process startup
//! injection. Executor uses async-trait for the existing dynamic backend boundary.

use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};
use parking_lot::Mutex;
use rustack_lambda_runtime::{
    metadata::{MetadataError, local_trace_id},
    protocol::{FunctionArn, Payload, ProtocolError},
    session::{Invocation, RuntimeResult, SessionError},
};
use thiserror::Error;
use tokio::{
    runtime::Handle,
    time::{Instant, timeout_at},
};
use typed_builder::TypedBuilder;

use super::{
    ContainerPolicy, DirectoryLease, DockerError, DockerZipRuntime, EnvironmentError,
    EnvironmentPlan, EnvironmentPool, EnvironmentPort, EnvironmentResponse, PoolConfig, PoolPort,
};
use crate::{
    artifact::{Architecture, DeploymentArtifact, PinnedImage},
    executor::{Executor, ExecutorError, InvokeRequest, InvokeResponse},
    invocation::{CleanupTask, InvocationContext},
};

/// Redacted typed failures at the Docker Executor composition boundary.
#[derive(Clone, Error)]
#[non_exhaustive]
pub enum DockerExecutorError {
    /// Startup/shutdown owner preserves its first diagnostic after physical retirement.
    #[error("Docker runtime owner failed")]
    Owner(#[source] Arc<super::DockerStartupError>),
    /// Invalid trusted settings, runtime scope or captured invocation metadata.
    #[error("invalid Docker executor configuration")]
    Configuration,
    /// Startup qualification was not granted or admission has been revoked.
    #[error("Docker execution is not ready")]
    Unavailable,
    /// Invalid bounded invocation payload, ARN or attempt identity.
    #[error("invalid Docker invocation metadata")]
    Request(#[source] Arc<ProtocolError>),
    /// Wall-clock tracing metadata could not be generated.
    #[error("Docker invocation clock failed")]
    Clock(#[from] MetadataError),
    /// Runtime session rejected construction before pool admission.
    #[error("Docker invocation session failed")]
    Session(#[from] SessionError),
    /// Retained environment preserves initialization, exit/OOM and wire classes.
    #[error("Docker environment failed: {0}")]
    Environment(#[from] EnvironmentError),
}
impl fmt::Debug for DockerExecutorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl From<ProtocolError> for DockerExecutorError {
    fn from(error: ProtocolError) -> Self {
        Self::Request(Arc::new(error))
    }
}

/// Trusted startup evidence port for the exact runtime inputs injected below.
///
/// Implementations must make bounded, nonblocking checks without IO. The startup
/// coordinator owns actual reconciliation and probe evidence; this trait alone
/// cannot establish it. Revocation rejects new calls; shutdown retires active work.
pub trait DockerReadiness: fmt::Debug + Send + Sync + 'static {
    /// Check coordinator-owned qualification for this runtime.
    ///
    /// # Errors
    /// Returns Unavailable or a typed qualification failure after revocation.
    ///
    /// ```no_run
    /// # fn ready(evidence: &dyn rustack_lambda_core::executor::DockerReadiness) {
    /// let qualified = evidence.check().is_ok();
    /// # }
    /// ```
    fn check(&self) -> Result<(), DockerExecutorError>;
}

/// Validated pool and independent public cleanup-wait budgets.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct DockerExecutorSettings {
    pool: PoolConfig,
    cleanup: Duration,
}
impl Default for DockerExecutorSettings {
    fn default() -> Self {
        Self {
            pool: PoolConfig::default(),
            cleanup: Duration::from_secs(10),
        }
    }
}
impl DockerExecutorSettings {
    /// Validate a whole-second cleanup budget in1..60s.
    ///
    /// # Errors
    /// Rejects zero, fractional or excessive cleanup duration.
    ///
    /// ```
    /// let settings = rustack_lambda_core::executor::DockerExecutorSettings::new(
    ///     Default::default(), std::time::Duration::from_secs(10))?;
    /// # Ok::<(), rustack_lambda_core::executor::DockerExecutorError>(())
    /// ```
    pub fn new(pool: PoolConfig, cleanup: Duration) -> Result<Self, DockerExecutorError> {
        if cleanup.subsec_nanos() != 0 || !(1..=60).contains(&cleanup.as_secs()) {
            return Err(DockerExecutorError::Configuration);
        }
        Ok(Self { pool, cleanup })
    }
}

/// Trusted startup inputs; readiness must qualify this exact scope/relay/policy.
#[derive(Debug, Clone, TypedBuilder)]
#[non_exhaustive]
pub struct DockerExecutorConfig {
    /// Persistent namespace capability retained through cleanup.
    pub directory: DirectoryLease,
    /// Immutable live startup relay image.
    pub relay: PinnedImage,
    /// Validated operator allocation policy.
    pub policy: ContainerPolicy,
    /// Retained environment service for the same directory owner and run.
    pub environments: EnvironmentPort,
    /// Trusted startup evidence, never supplied by an AWS request.
    pub readiness: Arc<dyn DockerReadiness>,
    /// Validated dispatch/cleanup budgets.
    #[builder(default)]
    pub settings: DockerExecutorSettings,
    /// Optional affiliated provided.al2023 source, qualified by production startup.
    #[builder(default)]
    pub zip: Option<DockerZipRuntime>,
}

#[derive(Debug)]
struct PoolOwner {
    root: Mutex<Option<EnvironmentPool>>,
    port: PoolPort,
    cleanup: CleanupTask,
    runtime: Handle,
    stopping: AtomicBool,
}

/// Actual image Lambda Executor over retained pool/environment ownership.
#[derive(Debug)]
#[non_exhaustive]
pub struct DockerExecutor {
    config: DockerExecutorConfig,
    pool: PoolOwner,
}
impl DockerExecutor {
    pub(super) fn physically_settled(&self) -> bool {
        self.pool.stopping.load(Ordering::Acquire) && self.pool.cleanup.is_finished()
    }

    /// Construct dispatch over matching live runtime inputs without daemon IO.
    ///
    /// This does not qualify production startup. Retain the environment/image
    /// managers until shutdown has joined this executor's pool.
    ///
    /// # Errors
    /// Rejects scope mismatch, revoked relay or missing Tokio supervision.
    ///
    /// ```no_run
    /// # async fn compose(config: rustack_lambda_core::executor::DockerExecutorConfig) -> Result<(), rustack_lambda_core::executor::ExecutorError> {
    /// use rustack_lambda_core::executor::{DockerExecutor, Executor};
    /// let executor = DockerExecutor::new(config)?;
    /// executor.shutdown().await?;
    /// # Ok(()) }
    /// ```
    pub fn new(config: DockerExecutorConfig) -> Result<Self, DockerExecutorError> {
        if !config.environments.same_scope(&config.directory) || !config.relay.is_live() {
            return Err(DockerExecutorError::Configuration);
        }
        if let Some(zip) = &config.zip {
            if !zip.same_scope(&config.directory)
                || zip.base.resolved().metadata().architecture
                    != config.relay.resolved().metadata().architecture
            {
                return Err(DockerExecutorError::Configuration);
            }
            zip.available().map_err(EnvironmentError::from)?;
        }
        let runtime = Handle::try_current().map_err(|_| DockerExecutorError::Unavailable)?;
        let root = EnvironmentPool::start(config.environments.clone(), config.settings.pool)?;
        let port = root.port();
        Ok(Self::with_pool(config, root, port, runtime))
    }
    fn with_pool(
        config: DockerExecutorConfig,
        root: EnvironmentPool,
        port: PoolPort,
        runtime: Handle,
    ) -> Self {
        Self {
            config,
            pool: PoolOwner {
                root: Mutex::new(Some(root)),
                port,
                cleanup: CleanupTask::default(),
                runtime,
                stopping: AtomicBool::new(false),
            },
        }
    }
    fn plan(&self, request: &InvokeRequest) -> Result<EnvironmentPlan, DockerExecutorError> {
        let plan = match &request.revision.artifact {
            DeploymentArtifact::Image(_) => EnvironmentPlan::new(
                self.config.directory.clone(),
                self.config.relay.clone(),
                request.revision.clone(),
                self.config.policy.clone(),
            ),
            DeploymentArtifact::Zip(_) => EnvironmentPlan::new_zip(
                self.config.directory.clone(),
                self.config.relay.clone(),
                request.revision.clone(),
                self.config.policy.clone(),
                self.config
                    .zip
                    .clone()
                    .ok_or(DockerExecutorError::Unavailable)?,
            ),
        }
        .map_err(EnvironmentError::from)?;
        let _: FunctionArn = request.function_arn.parse()?;
        let (_, function) = request
            .function_arn
            .split_once(":function:")
            .ok_or(DockerExecutorError::Configuration)?;
        let (name, qualifier) = function
            .split_once(':')
            .map_or((function, None), |(name, q)| (name, Some(q)));
        let version = request.revision.qualifier.as_str();
        let matches = match qualifier {
            None | Some("$LATEST") => version == "$LATEST",
            Some(value) if value.bytes().all(|b| b.is_ascii_digit()) => value == version,
            Some(_) => version != "$LATEST",
        };
        let region = request.function_arn.split(':').nth(3);
        if name != request.revision.function_name
            || !matches
            || region
                != request
                    .revision
                    .environment
                    .get("AWS_REGION")
                    .map(String::as_str)
        {
            return Err(DockerExecutorError::Configuration);
        }
        Payload::try_from(request.payload.clone())?;
        self.config.environments.check_memory(plan.memory_bytes())?;
        Ok(plan)
    }
    fn start_cleanup(&self) {
        self.pool.stopping.store(true, Ordering::Release);
        self.pool.cleanup.start_on(&self.pool.runtime, || {
            let root = self.pool.root.lock().take();
            async move {
                let Some(mut root) = root else {
                    return Err(cleanup_error());
                };
                loop {
                    match root
                        .shutdown(Instant::now() + Duration::from_secs(60))
                        .await
                    {
                        Ok(()) => return Ok(()),
                        Err(
                            EnvironmentError::Deadline
                            | EnvironmentError::Docker(DockerError::Deadline),
                        ) => {}
                        Err(_) => return Err(cleanup_error()),
                    }
                }
            }
        });
    }
}
impl Drop for DockerExecutor {
    fn drop(&mut self) {
        self.start_cleanup();
    }
}

#[async_trait]
impl Executor for DockerExecutor {
    fn image_available(&self) -> Result<(), ExecutorError> {
        self.config.readiness.check()?;
        let environment = self.config.environments.status();
        let pool = self.pool.port.status();
        if environment.worker_failed || pool.worker_failed {
            return Err(DockerExecutorError::Environment(EnvironmentError::Worker).into());
        }
        if self.pool.stopping.load(Ordering::Acquire)
            || !self.config.relay.is_live()
            || !environment.accepting
            || !pool.accepting
        {
            return Err(DockerExecutorError::Unavailable.into());
        }
        Ok(())
    }
    fn docker_zip_architecture(&self) -> Option<Architecture> {
        self.config
            .zip
            .as_ref()
            .map(|source| source.base.resolved().metadata().architecture)
    }

    fn zip_available(&self, architecture: Architecture) -> Result<(), ExecutorError> {
        self.image_available()?;
        let source = self.config.zip.as_ref().ok_or_else(|| {
            ExecutorError::Unsupported("Docker ZIP runtime is not configured".into())
        })?;
        source
            .available()
            .map_err(EnvironmentError::from)
            .map_err(DockerExecutorError::from)?;
        if source.base.resolved().metadata().architecture != architecture {
            return Err(ExecutorError::Unsupported(
                "Docker ZIP architecture does not match the selected base".into(),
            ));
        }
        Ok(())
    }
    fn available(&self, request: &InvokeRequest) -> Result<(), ExecutorError> {
        self.image_available()?;
        self.plan(request)?;
        Ok(())
    }
    async fn invoke(
        &self,
        request: InvokeRequest,
        context: InvocationContext,
    ) -> Result<InvokeResponse, ExecutorError> {
        self.image_available()?;
        context.check()?;
        let plan = self.plan(&request)?;
        let invocation = Invocation::new(
            context
                .id()
                .to_string()
                .parse()
                .map_err(DockerExecutorError::from)?,
            context.deadline(),
            request
                .function_arn
                .parse()
                .map_err(DockerExecutorError::from)?,
            local_trace_id().map_err(DockerExecutorError::from)?,
            Payload::try_from(request.payload.clone()).map_err(DockerExecutorError::from)?,
        )
        .map_err(|error| match error {
            SessionError::Deadline => ExecutorError::DeadlineExceeded,
            error => DockerExecutorError::Session(error).into(),
        })?;
        context.check()?;
        let mut call = self
            .pool
            .port
            .start(plan, invocation, request.capture_logs)
            .map_err(environment_error)?;
        let result = tokio::select! { biased;
            reason = context.cancelled() => Err(reason.error()),
            result = call.wait() => result.map_err(environment_error),
        };
        call.cancel_and_wait(Instant::now() + self.config.settings.cleanup)
            .await
            .map_err(|_| cleanup_error())?;
        context.check()?;
        response(result?, &request)
    }
    async fn shutdown(&self) -> Result<(), ExecutorError> {
        self.start_cleanup();
        timeout_at(
            Instant::now() + self.config.settings.cleanup,
            self.pool.cleanup.wait(),
        )
        .await
        .map_err(|_| cleanup_error())?
    }
}

fn cleanup_error() -> ExecutorError {
    ExecutorError::CleanupFailed("Docker resource settlement is not confirmed".into())
}
fn environment_error(error: EnvironmentError) -> ExecutorError {
    match error {
        EnvironmentError::Cancelled => ExecutorError::Cancelled,
        EnvironmentError::Deadline => ExecutorError::DeadlineExceeded,
        EnvironmentError::Timeout(timeout) => ExecutorError::Timeout(timeout),
        error => DockerExecutorError::Environment(error).into(),
    }
}
fn response(
    result: EnvironmentResponse,
    request: &InvokeRequest,
) -> Result<InvokeResponse, ExecutorError> {
    if result.logs.len() > 4096 {
        return Err(DockerExecutorError::Environment(DockerError::Protocol.into()).into());
    }
    let (payload, function_error) = match result.result {
        RuntimeResult::Success(payload) => (payload, None),
        RuntimeResult::FunctionError(payload) => (payload, Some("Unhandled".into())),
        RuntimeResult::InitError(payload) if payload.as_ref().len() <= 32768 => {
            return Err(environment_error(EnvironmentError::Initialization(payload)));
        }
        _ => return Err(environment_error(DockerError::Protocol.into())),
    };
    Ok(InvokeResponse {
        status: 200,
        payload: payload.into(),
        function_error,
        log_tail: request.capture_logs.then(|| STANDARD.encode(result.logs)),
        executed_version: request.revision.qualifier.clone(),
    })
}

#[cfg(test)]
mod tests;
