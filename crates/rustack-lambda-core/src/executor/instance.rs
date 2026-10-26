//! A single warm execution instance and its pool.
//!
//! Used by host bootstrap backends. Each instance owns its own
//! [`runtime_api::RuntimeApiHandle`] and a way to start / kill the
//! corresponding bootstrap (process or container). Pool entries are scoped
//! per `(function_name, qualifier)` because code can diverge between
//! versions.

use std::{
    mem::take,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use bytes::Bytes;
use dashmap::DashMap;
use rustack_lambda_runtime::{
    metadata::local_trace_id,
    protocol::{AttemptId, Payload},
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, oneshot},
    task::{JoinHandle, JoinSet},
    time::{Instant as TokioInstant, timeout_at},
};
use tracing::debug;
use typed_builder::TypedBuilder;

use super::{
    error::ExecutorError,
    runtime_api::{self, Invocation, RuntimeApiHandle, RuntimeResult, SessionError},
    types::{InvokeRequest, InvokeResponse},
};
use crate::{artifact::DeploymentArtifact, invocation::InvocationContext};

/// Identifier for the pool slot — a function/version pair.
pub(crate) type PoolKey = (String, String);

/// Trait that backends implement to create + destroy bootstraps.
///
/// Object-safe so the pool can hold `Arc<dyn InstanceBackend>`.
#[async_trait]
pub(crate) trait InstanceBackend: Send + Sync + std::fmt::Debug {
    /// Whether another OS execution resource can start without retiring an idle instance.
    fn has_capacity(&self) -> bool {
        true
    }

    /// Spawn a bootstrap pointing at `runtime_api_addr` for the given function.
    /// Register external ownership before an await after creation. Cancellation
    /// before returning a handle must leave no unowned process.
    async fn spawn(
        &self,
        req: &InvokeRequest,
        runtime_api_addr: std::net::SocketAddr,
    ) -> Result<BackendHandle, ExecutorError>;
}

/// Opaque handle to a backend-specific running thing (process or container).
/// Drop must clean it up; the pool also calls `kill` on graceful shutdown.
#[async_trait]
pub(crate) trait BackendHandleObj: Send + Sync + std::fmt::Debug {
    fn kill(&mut self);
    fn running(&self) -> bool;
    async fn wait(&mut self) -> Result<(), ExecutorError>;
}

/// Wrapper for object-safety + Drop.
pub(crate) struct BackendHandle(Box<dyn BackendHandleObj>);

impl BackendHandle {
    pub(crate) fn new<H: BackendHandleObj + 'static>(handle: H) -> Self {
        Self(Box::new(handle))
    }

    fn running(&self) -> bool {
        self.0.running()
    }
    async fn wait(&mut self) -> Result<(), ExecutorError> {
        self.0.wait().await
    }
    async fn terminate(mut self) -> Result<(), ExecutorError> {
        self.0.kill();
        self.0.wait().await
    }
}

impl std::fmt::Debug for BackendHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("BackendHandle").field(&self.0).finish()
    }
}

impl Drop for BackendHandle {
    fn drop(&mut self) {
        self.0.kill();
    }
}

/// One warm runtime instance. Owns the runtime API socket and the bootstrap.
#[derive(Debug)]
struct Instance {
    api: RuntimeApiHandle,
    backend: BackendHandle,
    last_used: Instant,
    idle_permit: Option<OwnedSemaphorePermit>,
    init_error: Option<oneshot::Receiver<Payload>>,
}

impl Instance {
    async fn run(
        &mut self,
        invocation: Invocation,
        context: &InvocationContext,
        timeout: Duration,
    ) -> Result<RuntimeResult, ExecutorError> {
        let response = async {
            let ticket = self
                .api
                .session()
                .submit(invocation, context.deadline())
                .await
                .map_err(|error| invocation_error(error, timeout))?;
            match self.init_error.take() {
                Some(mut init) => tokio::select! {
                    biased;
                    result = ticket.wait() => result.map_err(|error| invocation_error(error, timeout)),
                    error = &mut init => error.map(RuntimeResult::InitError).map_err(|_| ExecutorError::RuntimeExited("Runtime initialization channel closed".into())),
                },
                None => ticket
                    .wait()
                    .await
                    .map_err(|error| invocation_error(error, timeout)),
            }
        };
        tokio::select! {
            biased;
            reason = context.cancelled() => Err(reason.error()),
            result = response => result,
            exit = self.backend.wait() => exit.and(Err(ExecutorError::RuntimeExited("Bootstrap exited before completing invocation".into()))),
        }
    }

    async fn retire(mut self) -> Result<(), ExecutorError> {
        let api = self
            .api
            .shutdown()
            .await
            .map_err(|_| ExecutorError::CleanupFailed("Runtime API shutdown failed".into()));
        let backend = self.backend.terminate().await;
        backend.and(api)
    }
}

/// Pool of warm instances per `(function, qualifier)` key.
///
/// `acquire` fast-paths a warm instance and otherwise asks the backend to spawn
/// a new one. `release` returns the instance to the pool subject to
/// `max_warm`. `reap_idle` evicts instances older than `idle_timeout`.
#[derive(Debug, TypedBuilder)]
pub(crate) struct InstancePool {
    backend: Arc<dyn InstanceBackend>,
    max_warm: usize,
    idle_timeout: Duration,
    init_timeout: Duration,
    pools: DashMap<String, Vec<(String, Instance)>>,
    idle_capacity: Arc<Semaphore>,
}

impl InstancePool {
    pub(crate) fn new(
        backend: Arc<dyn InstanceBackend>,
        max_warm: usize,
        idle_timeout: Duration,
        init_timeout: Duration,
    ) -> Self {
        Self::builder()
            .backend(backend)
            .max_warm(max_warm)
            .idle_timeout(idle_timeout)
            .init_timeout(init_timeout)
            .pools(DashMap::new())
            .idle_capacity(Arc::new(Semaphore::new(32)))
            .build()
    }

    pub(crate) fn key(req: &InvokeRequest) -> PoolKey {
        let mut environment: Vec<_> = req.revision.environment.iter().collect();
        environment.sort();
        let code_root = match &req.revision.artifact {
            DeploymentArtifact::Zip(artifact) => Some(artifact.code_root()),
            DeploymentArtifact::Image(_) => None,
        };
        let identity = format!(
            "{:?}",
            (
                (
                    req.revision.resource_id,
                    &req.revision.revision_id,
                    &req.revision.qualifier,
                    req.revision.backend,
                ),
                (&req.revision.artifact, code_root),
                (
                    &req.revision.runtime,
                    &req.revision.handler,
                    req.revision.architecture
                ),
                environment,
                (
                    req.revision.timeout,
                    req.revision.memory_mb,
                    req.revision.ephemeral_storage_mb
                ),
                &req.revision.image_config,
            )
        );
        (
            req.revision.function_name.clone(),
            crate::storage::compute_sha256(identity.as_bytes()),
        )
    }

    /// Run a single invocation against an acquired (or freshly spawned) instance.
    pub(crate) async fn invoke(
        &self,
        req: InvokeRequest,
        context: InvocationContext,
    ) -> Result<InvokeResponse, ExecutorError> {
        context.check()?;
        let attempt: AttemptId = context
            .id()
            .to_string()
            .parse()
            .map_err(|_| ExecutorError::InvalidCode("Invalid invocation identity".into()))?;
        let arn = req
            .function_arn
            .parse()
            .map_err(|_| ExecutorError::InvalidCode("Invalid invocation ARN".into()))?;
        let trace = local_trace_id()
            .map_err(|_| ExecutorError::InvalidCode("Invalid invocation clock metadata".into()))?;
        let payload = Payload::try_from(req.payload.clone())
            .map_err(|_| ExecutorError::InvalidCode("Invocation payload too large".into()))?;
        let key = Self::key(&req);
        let (instance, retired) = self.try_acquire(&key);
        retire_all(retired).await?;
        let instance = match instance {
            Some(instance) if instance.backend.running() && !instance.api.session().is_closed() => {
                Some(instance)
            }
            Some(instance) => {
                instance.retire().await?;
                None
            }
            None => None,
        };
        let mut instance = match instance {
            Some(instance) => instance,
            None => self.spawn_new(&req, &context).await?,
        };
        let Some(handler_deadline) = TokioInstant::now().checked_add(req.revision.timeout) else {
            instance.retire().await?;
            return Err(ExecutorError::InvalidCode("Invalid handler timeout".into()));
        };
        let deadline = handler_deadline.min(context.deadline());
        let Ok(invocation) = Invocation::new(attempt, deadline, arn, trace, payload) else {
            instance.retire().await?;
            context.check()?;
            return Err(ExecutorError::Timeout(req.revision.timeout));
        };
        let result = instance
            .run(invocation, &context, req.revision.timeout)
            .await;
        let result = context.check().and(result);
        let reusable = matches!(&result, Ok(RuntimeResult::Success(_)))
            && instance.backend.running()
            && !instance.api.session().is_closed();
        if reusable {
            instance.last_used = Instant::now();
            if let Some(instance) = self.release(key, instance) {
                instance.retire().await?;
            }
        } else {
            instance.retire().await?;
        }
        match result? {
            RuntimeResult::Success(payload) => Ok(InvokeResponse::success(
                Bytes::from(payload),
                req.revision.qualifier.clone(),
            )),
            RuntimeResult::FunctionError(payload) => Ok(InvokeResponse {
                status: 200,
                payload: Bytes::from(payload),
                function_error: Some("Unhandled".into()),
                log_tail: None,
                executed_version: req.revision.qualifier.clone(),
            }),
            RuntimeResult::InitError(_) => Err(ExecutorError::InitFailed(
                "Bootstrap reported initialization failure".into(),
            )),
            _ => Err(ExecutorError::RuntimeExited(
                "Unsupported runtime result".into(),
            )),
        }
    }

    fn try_acquire(&self, key: &PoolKey) -> (Option<Instance>, Vec<Instance>) {
        let Some(mut bucket) = self.pools.get_mut(&key.0) else {
            return (None, Vec::new());
        };
        let result = if let Some(index) = bucket.iter().position(|(revision, _)| revision == &key.1)
        {
            let (_, mut instance) = bucket.swap_remove(index);
            instance.idle_permit.take();
            (Some(instance), Vec::new())
        } else {
            (
                None,
                take(&mut *bucket)
                    .into_iter()
                    .map(|(_, instance)| instance)
                    .collect(),
            )
        };
        drop(bucket);
        self.pools.remove_if(&key.0, |_, bucket| bucket.is_empty());
        result
    }

    fn release(&self, key: PoolKey, mut instance: Instance) -> Option<Instance> {
        let Ok(permit) = Arc::clone(&self.idle_capacity).try_acquire_owned() else {
            return Some(instance);
        };
        let mut bucket = self.pools.entry(key.0.clone()).or_default();
        if !self.idle_capacity.is_closed() && bucket.len() < self.max_warm.min(1) {
            instance.idle_permit = Some(permit);
            bucket.push((key.1, instance));
            None
        } else {
            drop(bucket);
            self.pools.remove_if(&key.0, |_, bucket| bucket.is_empty());
            Some(instance)
        }
    }

    async fn spawn_new(
        &self,
        req: &InvokeRequest,
        context: &InvocationContext,
    ) -> Result<Instance, ExecutorError> {
        context.check()?;
        if !self.backend.has_capacity() {
            let mut evicted = None;
            for mut bucket in self.pools.iter_mut() {
                if let Some((_, instance)) = bucket.pop() {
                    evicted = Some(instance);
                    break;
                }
            }
            if let Some(instance) = evicted {
                instance.retire().await?;
            }
        }
        let deadline = TokioInstant::now()
            .checked_add(self.init_timeout)
            .ok_or_else(|| ExecutorError::InvalidCode("Invalid initialization budget".into()))?
            .min(context.deadline());
        let mut api = tokio::select! {
            biased;
            reason = context.cancelled() => return Err(reason.error()),
            result = timeout_at(deadline, runtime_api::start_at(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0), self.idle_timeout)) => {
                result.map_err(|_| ExecutorError::Timeout(self.init_timeout))?.map_err(|_| ExecutorError::InitFailed("Runtime API startup failed".into()))?
            }
        };
        let addr = api.addr();
        let init_error = api.take_initialization_error();
        let backend = tokio::select! {
            biased;
            reason = context.cancelled() => Err(reason.error()),
            result = timeout_at(deadline, self.backend.spawn(req, addr)) => result.map_err(|_| ExecutorError::Timeout(self.init_timeout)).and_then(|result| result),
        };
        let backend = match backend {
            Ok(backend) => backend,
            Err(error) => {
                api.shutdown().await.map_err(|_| {
                    ExecutorError::CleanupFailed("Runtime API startup cleanup failed".into())
                })?;
                return Err(error);
            }
        };
        let mut instance = Instance {
            api,
            backend,
            last_used: Instant::now(),
            idle_permit: None,
            init_error,
        };
        let readiness = {
            let ready = async {
                match instance.init_error.as_mut() {
                    Some(init) => tokio::select! {
                        biased;
                        error = init => match error {
                            Ok(_) => Err(ExecutorError::InitFailed("Bootstrap reported initialization failure".into())),
                            Err(_) => Err(ExecutorError::RuntimeExited("Runtime initialization channel closed".into())),
                        },
                        ready = instance.api.session().bootstrap_ready(deadline) => ready.map_err(|error| invocation_error(error, self.init_timeout)),
                    },
                    None => instance
                        .api
                        .session()
                        .bootstrap_ready(deadline)
                        .await
                        .map_err(|error| invocation_error(error, self.init_timeout)),
                }
            };
            tokio::select! {
                biased;
                reason = context.cancelled() => Err(reason.error()),
                result = ready => result,
                exit = instance.backend.wait() => exit.and(Err(ExecutorError::RuntimeExited("Bootstrap exited before first Runtime API poll".into()))),
            }
        };
        if let Err(error) = readiness {
            instance.retire().await?;
            return Err(error);
        }
        debug!(function = %req.revision.function_name, "Lambda bootstrap completed first Runtime API poll");
        Ok(instance)
    }

    /// Reap expired idle instances and join their API/process cleanup.
    pub(crate) async fn reap_idle(&self) -> Result<usize, ExecutorError> {
        let now = Instant::now();
        let mut retired = Vec::new();
        for mut bucket in self.pools.iter_mut() {
            let mut index = 0;
            while index < bucket.len() {
                let expired = bucket.get(index).is_some_and(|(_, instance)| {
                    now.duration_since(instance.last_used) > self.idle_timeout
                });
                if expired {
                    let (_, instance) = bucket.swap_remove(index);
                    retired.push(instance);
                } else {
                    index += 1;
                }
            }
        }
        self.pools.retain(|_, bucket| !bucket.is_empty());
        let count = retired.len();
        retire_all(retired).await?;
        Ok(count)
    }

    /// Close warm retention before starting owned asynchronous cleanup.
    pub(crate) fn stop_retention(&self) {
        self.idle_capacity.close();
    }

    /// Stop warm retention and join every idle API/process owner.
    pub(crate) async fn shutdown(&self) -> Result<(), ExecutorError> {
        self.idle_capacity.close();
        let mut retired = Vec::new();
        for mut bucket in self.pools.iter_mut() {
            retired.extend(take(&mut *bucket).into_iter().map(|(_, instance)| instance));
        }
        self.pools.clear();
        retire_all(retired).await
    }
}

fn invocation_error(error: SessionError, timeout: Duration) -> ExecutorError {
    match error {
        SessionError::Deadline => ExecutorError::Timeout(timeout),
        SessionError::Cancelled => ExecutorError::Cancelled,
        SessionError::SupervisorFailed => {
            ExecutorError::CleanupFailed("Runtime session supervisor failed".into())
        }
        _ => ExecutorError::RuntimeExited("Runtime session is unavailable".into()),
    }
}

async fn retire_all(instances: Vec<Instance>) -> Result<(), ExecutorError> {
    // Idle retention is bounded to32; retire independently so one stalled
    // HTTP connection cannot multiply the shutdown budget by the pool size.
    let mut tasks = JoinSet::new();
    for instance in instances {
        tasks.spawn(instance.retire());
    }
    let mut failure = None;
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                failure.get_or_insert(error);
            }
            Err(_) => {
                failure.get_or_insert(ExecutorError::CleanupFailed(
                    "Idle instance cleanup task failed".into(),
                ));
            }
        }
    }
    failure.map_or(Ok(()), Err)
}

/// Spawn a periodic idle-reaper background task. Stops when `cancel` flips.
pub(crate) fn spawn_reaper(
    pool: Arc<InstancePool>,
    interval: Duration,
    mut cancel: tokio::sync::watch::Receiver<bool>,
) -> JoinHandle<Result<(), ExecutorError>> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(interval);
        tick.tick().await; // skip the immediate tick
        loop {
            tokio::select! {
                biased;
                () = async { let _ = cancel.wait_for(|stopped| *stopped).await; } => break,
                _ = tick.tick() => {
                    match pool.reap_idle().await {
                        Ok(n) if n > 0 => debug!(reaped = n, "lambda idle reaper"),
                        Ok(_) => {},
                        Err(error) => { pool.idle_capacity.close(); return Err(error); }
                    }
                }
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use rustack_lambda_model::types::ImageConfig;

    use super::*;

    #[test]
    fn test_should_key_warm_environment_by_resource_and_captured_configuration() {
        let request = InvokeRequest::fixture();
        let original = InstancePool::key(&request);
        let mut alias = request.clone();
        alias.function_arn.push_str(":prod");
        assert_eq!(InstancePool::key(&alias), original);

        let mut recreated = request.clone();
        Arc::make_mut(&mut recreated.revision).resource_id =
            crate::identity::FunctionResourceId::default();
        assert_ne!(InstancePool::key(&recreated), original);
        let mut updated = request.clone();
        Arc::make_mut(&mut updated.revision).revision_id = "new-revision".into();
        assert_ne!(InstancePool::key(&updated), original);
        let mut rehydrated = request.clone();
        Arc::make_mut(&mut rehydrated.revision).artifact =
            DeploymentArtifact::Zip(crate::artifact::ValidatedArtifact::fixture());
        assert_ne!(InstancePool::key(&rehydrated), original);
        let mut storage = request.clone();
        Arc::make_mut(&mut storage.revision).ephemeral_storage_mb = 1024;
        assert_ne!(InstancePool::key(&storage), original);
        let mut image = request.clone();
        Arc::make_mut(&mut image.revision).image_config = Some(ImageConfig {
            command: Some(vec![]),
            ..ImageConfig::default()
        });
        assert_ne!(InstancePool::key(&image), original);
        let explicitly_empty = InstancePool::key(&image);
        Arc::make_mut(&mut image.revision).image_config = Some(ImageConfig::default());
        assert_ne!(InstancePool::key(&image), explicitly_empty);
    }

    #[test]
    fn test_should_fingerprint_environment_independently_of_hashmap_iteration_order() {
        let mut first = InvokeRequest::fixture();
        Arc::make_mut(&mut first.revision)
            .environment
            .insert("FIRST".into(), "one".into());
        Arc::make_mut(&mut first.revision)
            .environment
            .insert("SECOND".into(), "two".into());
        let mut second = first.clone();
        Arc::make_mut(&mut second.revision).environment.clear();
        Arc::make_mut(&mut second.revision)
            .environment
            .insert("SECOND".into(), "two".into());
        Arc::make_mut(&mut second.revision)
            .environment
            .insert("FIRST".into(), "one".into());
        assert_eq!(InstancePool::key(&first), InstancePool::key(&second));
        Arc::make_mut(&mut second.revision)
            .environment
            .insert("SECOND".into(), "changed".into());
        assert_ne!(InstancePool::key(&first), InstancePool::key(&second));
    }
}
