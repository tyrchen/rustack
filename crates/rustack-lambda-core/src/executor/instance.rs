//! A single warm execution instance and its pool.
//!
//! Used by both the native and Docker backends. Each instance owns its own
//! [`runtime_api::RuntimeApiHandle`] and a way to start / kill the
//! corresponding bootstrap (process or container). Pool entries are scoped
//! per `(function_name, qualifier)` because code can diverge between
//! versions.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use dashmap::DashMap;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, oneshot};
use tracing::debug;

use super::{
    error::ExecutorError,
    runtime_api::{self, Job, RuntimeApiHandle, RuntimeResult},
    types::{InvokeRequest, InvokeResponse},
};

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
    /// Returns a handle the pool will keep alive until the instance is reaped.
    async fn spawn(
        &self,
        req: &InvokeRequest,
        runtime_api_addr: std::net::SocketAddr,
    ) -> Result<BackendHandle, ExecutorError>;
}

/// Opaque handle to a backend-specific running thing (process or container).
/// Drop must clean it up; the pool also calls `kill` on graceful shutdown.
pub(crate) trait BackendHandleObj: Send + Sync + std::fmt::Debug {
    fn kill(&mut self);
}

/// Wrapper for object-safety + Drop.
pub(crate) struct BackendHandle(Box<dyn BackendHandleObj>);

impl BackendHandle {
    pub(crate) fn new<H: BackendHandleObj + 'static>(handle: H) -> Self {
        Self(Box::new(handle))
    }

    pub(crate) fn kill(mut self) {
        self.0.kill();
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
    init_error: Option<oneshot::Receiver<bytes::Bytes>>,
}

/// Pool of warm instances per `(function, qualifier)` key.
///
/// `acquire` fast-paths a warm instance and otherwise asks the backend to spawn
/// a new one. `release` returns the instance to the pool subject to
/// `max_warm`. `reap_idle` evicts instances older than `idle_timeout`.
#[derive(Debug)]
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
        Self {
            backend,
            max_warm,
            idle_timeout,
            init_timeout,
            pools: DashMap::new(),
            idle_capacity: Arc::new(Semaphore::new(32)),
        }
    }

    pub(crate) fn key(req: &InvokeRequest) -> PoolKey {
        let mut environment: Vec<_> = req.environment.iter().collect();
        environment.sort();
        let identity = format!(
            "{:?}",
            (
                &req.qualifier,
                &req.code_root,
                &req.image_uri,
                &req.runtime,
                &req.handler,
                &req.architectures,
                environment,
                req.timeout,
                req.memory_mb
            )
        );
        (
            req.function_name.clone(),
            crate::storage::compute_sha256(identity.as_bytes()),
        )
    }

    /// Run a single invocation against an acquired (or freshly spawned) instance.
    pub(crate) async fn invoke(&self, req: InvokeRequest) -> Result<InvokeResponse, ExecutorError> {
        let key = Self::key(&req);
        let mut instance = match self.try_acquire(&key) {
            Some(inst) => inst,
            None => self.spawn_new(&req).await?,
        };

        let request_id = uuid::Uuid::new_v4().to_string();
        let deadline = Instant::now() + req.timeout;
        let (resp_tx, resp_rx) = oneshot::channel();
        let job = Job {
            request_id: request_id.clone(),
            function_arn: req.function_arn.clone(),
            deadline,
            payload: req.payload.clone(),
            response_tx: resp_tx,
        };
        instance
            .api
            .submit(job)
            .await
            .map_err(|e| ExecutorError::Io(e.to_string()))?;

        let response = async {
            match instance.init_error.take() {
                Some(mut init) => tokio::select! {
                    response = resp_rx => response,
                    error = &mut init => error.map(RuntimeResult::InitError),
                },
                None => resp_rx.await,
            }
        };
        let result = match tokio::time::timeout(req.timeout, response).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => {
                // Bootstrap died before responding.
                instance.backend.kill();
                return Err(ExecutorError::RuntimeExited(
                    "bootstrap exited mid-invocation".to_owned(),
                ));
            }
            Err(_) => {
                instance.backend.kill();
                return Err(ExecutorError::Timeout(req.timeout));
            }
        };

        if !matches!(result, RuntimeResult::InitError(_)) {
            instance.last_used = Instant::now();
            self.release(key, instance);
        }

        match result {
            RuntimeResult::Success(payload) => Ok(InvokeResponse {
                status: 200,
                payload,
                function_error: None,
                log_tail: None,
                executed_version: req.qualifier,
            }),
            RuntimeResult::Error(payload) => Ok(InvokeResponse {
                status: 200,
                payload,
                function_error: Some("Unhandled".to_owned()),
                log_tail: None,
                executed_version: req.qualifier,
            }),
            RuntimeResult::InitError(payload) => {
                let msg = String::from_utf8_lossy(&payload).into_owned();
                Err(ExecutorError::InitFailed(msg))
            }
        }
    }

    fn try_acquire(&self, key: &PoolKey) -> Option<Instance> {
        let mut bucket = self.pools.get_mut(&key.0)?;
        let Some(index) = bucket.iter().position(|(revision, _)| revision == &key.1) else {
            bucket.clear();
            return None;
        };
        let (_, mut instance) = bucket.swap_remove(index);
        instance.idle_permit.take();
        Some(instance)
    }

    fn release(&self, key: PoolKey, mut instance: Instance) {
        let Ok(permit) = Arc::clone(&self.idle_capacity).try_acquire_owned() else {
            return;
        };
        let mut bucket = self.pools.entry(key.0).or_default();
        if bucket.len() < self.max_warm.min(1) {
            instance.idle_permit = Some(permit);
            bucket.push((key.1, instance));
        }
    }

    async fn spawn_new(&self, req: &InvokeRequest) -> Result<Instance, ExecutorError> {
        if !self.backend.has_capacity() {
            for mut bucket in self.pools.iter_mut() {
                if let Some((_, instance)) = bucket.pop() {
                    drop(instance);
                    break;
                }
            }
        }
        let api = runtime_api::start()
            .await
            .map_err(|e| ExecutorError::Io(format!("bind runtime api: {e}")))?;
        let addr = api.addr();
        let init_error = api.take_init_error_rx().await;

        // Race: backend spawn + first /next poll.  We don't observe /next here
        // directly — we rely on either submit landing on a polling bootstrap
        // OR an `/init/error` arriving.  To keep liveness, spawn the backend
        // within the init window and watch the init-error channel for a
        // fast-fail signal.
        let backend = tokio::time::timeout(self.init_timeout, self.backend.spawn(req, addr))
            .await
            .map_err(|_| ExecutorError::Timeout(self.init_timeout))??;
        let inst = Instance {
            api,
            backend,
            last_used: Instant::now(),
            idle_permit: None,
            init_error,
        };
        debug!(function = %req.function_name, addr = %addr, "spawned new lambda instance");
        Ok(inst)
    }

    /// Reap instances idle for longer than `idle_timeout`. Returns count.
    pub(crate) fn reap_idle(&self) -> usize {
        let now = Instant::now();
        let idle = self.idle_timeout;
        let mut killed = 0usize;
        for mut bucket in self.pools.iter_mut() {
            let before = bucket.len();
            bucket.retain(|(_, instance)| now.duration_since(instance.last_used) <= idle);
            killed += before.saturating_sub(bucket.len());
        }
        self.pools.retain(|_, bucket| !bucket.is_empty());
        killed
    }

    /// Drain and kill every instance in every pool.
    pub(crate) fn shutdown(&self) {
        self.pools.clear();
    }
}

/// Spawn a periodic idle-reaper background task. Stops when `cancel` flips.
pub(crate) fn spawn_reaper(
    pool: Arc<InstancePool>,
    interval: Duration,
    mut cancel: tokio::sync::watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(interval);
        tick.tick().await; // skip the immediate tick
        loop {
            tokio::select! {
                _ = cancel.changed() => break,
                _ = tick.tick() => {
                    let n = pool.reap_idle();
                    if n > 0 {
                        debug!(reaped = n, "lambda idle reaper");
                    }
                }
            }
        }
    })
}
