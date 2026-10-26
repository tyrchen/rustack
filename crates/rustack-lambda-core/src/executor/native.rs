//! Native (process) execution backend.
//!
//! Spawns the function's `bootstrap` binary directly on the host. Suitable
//! only for `provided.al2` / `provided.al2023` runtimes whose architectures
//! match the host. The auto executor keeps macOS Zip functions on Squib and
//! leaves unsupported image packages for the future Docker backend.

use std::{
    future::poll_fn,
    net::SocketAddr,
    path::Path,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use parking_lot::Mutex as PMutex;
use tokio::{
    io::AsyncReadExt,
    process::Command,
    runtime::Handle,
    sync::{Semaphore, oneshot, watch},
    task::JoinSet,
};
use tracing::{debug, warn};

use super::{
    Executor, ExecutorError, InvokeRequest, InvokeResponse,
    instance::{BackendHandle, BackendHandleObj, InstanceBackend, InstancePool, spawn_reaper},
};
use crate::{
    artifact::{Architecture, DeploymentArtifact},
    invocation::{CleanupTask, InvocationContext, Settlement},
};

/// Native process executor.
#[derive(Debug)]
pub struct NativeExecutor {
    pool: Arc<InstancePool>,
    cancel_tx: watch::Sender<bool>,
    reaper: PMutex<Option<tokio::task::JoinHandle<Result<(), ExecutorError>>>>,
    processes: Arc<NativeProcesses>,
    cleanup: CleanupTask,
}

#[derive(Debug)]
struct NativeProcesses {
    tasks: PMutex<JoinSet<()>>,
    cleanup: CleanupTask,
    accepting: AtomicBool,
    capacity: Arc<Semaphore>,
    runtime: Handle,
}

impl NativeProcesses {
    fn start_cleanup(self: &Arc<Self>) {
        let processes = Arc::clone(self);
        self.cleanup.start_on(&self.runtime, move || async move {
            let mut failed = false;
            while let Some(result) =
                poll_fn(|context| processes.tasks.lock().poll_join_next(context)).await
            {
                if result.is_err() {
                    failed = true;
                }
            }
            if failed {
                Err(ExecutorError::CleanupFailed(
                    "Native process supervisor failed".into(),
                ))
            } else {
                Ok(())
            }
        });
    }
}

impl Drop for NativeExecutor {
    fn drop(&mut self) {
        self.start_cleanup();
    }
}

impl NativeExecutor {
    /// Build a new native executor.
    #[must_use]
    pub fn new(max_warm: usize, idle_timeout: Duration, init_timeout: Duration) -> Self {
        let processes = Arc::new(NativeProcesses {
            tasks: PMutex::new(JoinSet::new()),
            cleanup: CleanupTask::default(),
            accepting: AtomicBool::new(true),
            capacity: Arc::new(Semaphore::new(32)),
            runtime: Handle::current(),
        });
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let backend = Arc::new(NativeBackend {
            processes: Arc::clone(&processes),
            shutdown: cancel_rx.clone(),
        });
        let pool = Arc::new(InstancePool::new(
            backend,
            max_warm,
            idle_timeout,
            init_timeout,
        ));
        let reaper = spawn_reaper(Arc::clone(&pool), Duration::from_secs(30), cancel_rx);
        Self {
            pool,
            cancel_tx,
            reaper: PMutex::new(Some(reaper)),
            processes,
            cleanup: CleanupTask::default(),
        }
    }

    fn start_cleanup(&self) {
        self.processes.accepting.store(false, Ordering::Release);
        self.cancel_tx.send_replace(true);
        self.pool.stop_retention();
        self.cleanup.start_on(&self.processes.runtime, || {
            let pool = Arc::clone(&self.pool);
            let processes = Arc::clone(&self.processes);
            let reaper = self.reaper.lock().take();
            async move {
                processes.start_cleanup();
                let reaper = match reaper {
                    Some(reaper) => reaper
                        .await
                        .map_err(|_| ExecutorError::CleanupFailed("Native reaper failed".into()))
                        .and_then(|result| result),
                    None => Ok(()),
                };
                let pool = pool.shutdown().await;
                let children = processes.cleanup.wait().await;
                reaper.and(pool).and(children)
            }
        });
    }
}

#[async_trait]
impl Executor for NativeExecutor {
    fn available(&self, request: &InvokeRequest) -> Result<(), ExecutorError> {
        if !self.processes.accepting.load(Ordering::Acquire) {
            return Err(ExecutorError::Cancelled);
        }
        if !matches!(request.revision.artifact, DeploymentArtifact::Zip(_)) {
            return Err(ExecutorError::Unsupported(
                "native backend only supports Zip packages".into(),
            ));
        }
        if request.revision.architecture.as_str() != host_architecture() {
            return Err(ExecutorError::Unsupported(
                "native backend requires the host architecture".into(),
            ));
        }
        Ok(())
    }

    async fn invoke(
        &self,
        req: InvokeRequest,
        context: InvocationContext,
    ) -> Result<InvokeResponse, ExecutorError> {
        self.available(&req)?;
        context.check()?;
        let DeploymentArtifact::Zip(artifact) = &req.revision.artifact else {
            return Err(ExecutorError::Unsupported(
                "native backend only supports Zip packages".into(),
            ));
        };
        let code_root = artifact.code_root();
        let bootstrap = code_root.join("bootstrap");
        let supported = tokio::select! {
            biased;
            reason = context.cancelled() => return Err(reason.error()),
            result = async { ensure_executable(&bootstrap).await?; bootstrap_runs_on_host(&bootstrap, req.revision.architecture).await } => result?,
        };
        if !supported {
            return Err(ExecutorError::Unsupported(format!(
                "bootstrap {} cannot run on host {}/{}; use docker backend",
                bootstrap.display(),
                std::env::consts::OS,
                std::env::consts::ARCH,
            )));
        }
        self.pool.invoke(req, context).await
    }

    async fn shutdown(&self) -> Result<(), ExecutorError> {
        self.start_cleanup();
        self.cleanup.wait().await
    }
}

#[derive(Debug)]
struct NativeBackend {
    processes: Arc<NativeProcesses>,
    shutdown: watch::Receiver<bool>,
}

#[async_trait]
impl InstanceBackend for NativeBackend {
    fn has_capacity(&self) -> bool {
        self.processes.capacity.available_permits() != 0
    }
    async fn spawn(
        &self,
        req: &InvokeRequest,
        runtime_api_addr: SocketAddr,
    ) -> Result<BackendHandle, ExecutorError> {
        let DeploymentArtifact::Zip(artifact) = &req.revision.artifact else {
            return Err(ExecutorError::Unsupported(
                "native backend only supports Zip packages".into(),
            ));
        };
        let code_root = artifact.code_root();
        let bootstrap = code_root.join("bootstrap");

        let mut cmd = Command::new(&bootstrap);
        cmd.current_dir(code_root)
            .env_clear()
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .kill_on_drop(true);
        for (k, v) in lambda_env_vars(req, runtime_api_addr) {
            cmd.env(k, v);
        }

        let permit = Arc::clone(&self.processes.capacity)
            .acquire_owned()
            .await
            .map_err(|_| ExecutorError::Cancelled)?;
        let mut processes = self.processes.tasks.lock();
        if !self.processes.accepting.load(Ordering::Acquire) {
            return Err(ExecutorError::Cancelled);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| ExecutorError::Io(format!("spawn bootstrap: {e}")))?;

        // Drain stdout/stderr into a small ring so we don't fill pipe buffers.
        let log_buf = Arc::new(PMutex::new(LogTail::with_capacity(4 * 1024)));
        let mut logs = JoinSet::new();
        if let Some(stdout) = child.stdout.take() {
            logs.spawn(drain_to_buf(stdout, Arc::clone(&log_buf), "stdout"));
        }
        if let Some(stderr) = child.stderr.take() {
            logs.spawn(drain_to_buf(stderr, log_buf, "stderr"));
        }
        let (cancel, cancelled) = oneshot::channel();
        let artifact_lease = artifact.clone();
        while let Some(result) = processes.try_join_next() {
            if let Err(error) = result {
                warn!(%error, "Lambda process supervisor failed");
            }
        }
        let (completion, completed) = watch::channel(Settlement::Pending);
        let mut shutdown = self.shutdown.clone();
        processes.spawn(async move {
            {
                let _artifact_lease = artifact_lease;
                let _permit = permit;
                let result = tokio::select! {
                    biased;
                    () = async { let _ = shutdown.wait_for(|stopped| *stopped).await; } => child.kill().await,
                    _ = cancelled => child.kill().await,
                    result = child.wait() => result.map(|_| ()),
                };
                if result.is_err() {
                    completion.send_replace(Settlement::Uncertain);
                    // Preserve child/lease/allocation ownership until reaping is confirmed.
                    loop {
                        let _ = child.start_kill();
                        match child.wait().await {
                            Ok(_) => break,
                            Err(_) => tokio::time::sleep(Duration::from_secs(1)).await,
                        }
                    }
                }
                logs.abort_all();
                while let Some(result) = logs.join_next().await {
                    if result.is_err_and(|error| !error.is_cancelled()) {
                        warn!("Native log task failed");
                    }
                }
            }
            completion.send_replace(Settlement::Clean);
        });
        Ok(BackendHandle::new(NativeHandle {
            cancel: Some(cancel),
            completion: completed,
        }))
    }
}

#[derive(Debug)]
struct NativeHandle {
    cancel: Option<oneshot::Sender<()>>,
    completion: watch::Receiver<Settlement>,
}

#[async_trait]
impl BackendHandleObj for NativeHandle {
    fn running(&self) -> bool {
        *self.completion.borrow() == Settlement::Pending
    }

    async fn wait(&mut self) -> Result<(), ExecutorError> {
        match self
            .completion
            .wait_for(|state| *state != Settlement::Pending)
            .await
        {
            Ok(state) if *state == Settlement::Clean => Ok(()),
            _ => Err(ExecutorError::CleanupFailed(
                "Native child exit is not confirmed".into(),
            )),
        }
    }

    fn kill(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            if cancel.send(()).is_err() {
                debug!("Lambda process already exited");
            }
        }
    }
}

impl Drop for NativeHandle {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Check declared architecture and executable format with fallible asynchronous IO.
async fn bootstrap_runs_on_host(
    path: &Path,
    architecture: Architecture,
) -> Result<bool, ExecutorError> {
    // Architecture check first — declared `architectures` must include the
    // host arch.
    if architecture.as_str() != host_architecture() {
        return Ok(false);
    }
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| ExecutorError::Io(format!("Open bootstrap: {error}")))?;
    let mut hdr = [0u8; 4];
    file.read_exact(&mut hdr)
        .await
        .map_err(|error| ExecutorError::InvalidCode(format!("Read bootstrap header: {error}")))?;
    let elf = hdr == [0x7f, b'E', b'L', b'F'];
    let macho = hdr == [0xCF, 0xFA, 0xED, 0xFE]
        || hdr == [0xFE, 0xED, 0xFA, 0xCE]
        || hdr == [0xCE, 0xFA, 0xED, 0xFE]
        || hdr == [0xCA, 0xFE, 0xBA, 0xBE];
    let host_is_macos = std::env::consts::OS == "macos";
    let host_is_linux = std::env::consts::OS == "linux";
    Ok((elf && host_is_linux) || (macho && host_is_macos))
}

fn host_architecture() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        other => other,
    }
}

/// Require a regular executable bootstrap without modifying immutable artifacts.
#[cfg(unix)]
async fn ensure_executable(path: &Path) -> Result<(), ExecutorError> {
    use std::os::unix::fs::PermissionsExt as _;
    let meta = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|error| ExecutorError::Io(format!("Inspect bootstrap: {error}")))?;
    if meta.file_type().is_symlink() || !meta.is_file() || meta.permissions().mode() & 0o111 == 0 {
        return Err(ExecutorError::InvalidCode(
            "bootstrap must be a regular executable file".into(),
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
async fn ensure_executable(_path: &Path) -> Result<(), ExecutorError> {
    Ok(())
}

fn lambda_env_vars(req: &InvokeRequest, runtime_api: SocketAddr) -> Vec<(String, String)> {
    let mut env = Vec::with_capacity(16 + req.revision.environment.len());
    let task_root = match &req.revision.artifact {
        DeploymentArtifact::Zip(artifact) => artifact.code_root().display().to_string(),
        DeploymentArtifact::Image(_) => String::new(),
    };
    env.push(("AWS_LAMBDA_RUNTIME_API".into(), runtime_api.to_string()));
    env.push((
        "AWS_LAMBDA_FUNCTION_NAME".into(),
        req.revision.function_name.clone(),
    ));
    env.push((
        "AWS_LAMBDA_FUNCTION_VERSION".into(),
        req.revision.qualifier.clone(),
    ));
    env.push((
        "AWS_LAMBDA_FUNCTION_MEMORY_SIZE".into(),
        req.revision.memory_mb.to_string(),
    ));
    env.push((
        "AWS_LAMBDA_FUNCTION_TIMEOUT".into(),
        req.revision.timeout.as_secs().to_string(),
    ));
    env.push((
        "AWS_LAMBDA_LOG_GROUP_NAME".into(),
        format!("/aws/lambda/{}", req.revision.function_name),
    ));
    env.push((
        "AWS_LAMBDA_LOG_STREAM_NAME".into(),
        uuid::Uuid::new_v4().to_string(),
    ));
    env.push((
        "_HANDLER".into(),
        req.revision.handler.clone().unwrap_or_default(),
    ));
    env.push(("LAMBDA_TASK_ROOT".into(), task_root.clone()));
    env.push(("LAMBDA_RUNTIME_DIR".into(), task_root));
    env.push(("TZ".into(), ":UTC".into()));
    env.push(("PATH".into(), default_path()));
    // Sensible defaults so SDK calls back to the same rustack instance work.
    env.push(("AWS_ACCESS_KEY_ID".into(), "test".into()));
    env.push(("AWS_SECRET_ACCESS_KEY".into(), "test".into()));
    env.push(("AWS_SESSION_TOKEN".into(), "test".into()));
    // User env overrides last, but do not let it shadow the runtime API var.
    for (k, v) in &req.revision.environment {
        if k == "AWS_LAMBDA_RUNTIME_API" || k == "_HANDLER" || k == "LAMBDA_TASK_ROOT" {
            warn!(key = %k, "ignoring user override for reserved Lambda env var");
            continue;
        }
        env.push((k.clone(), v.clone()));
    }
    env
}

fn default_path() -> String {
    "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_owned()
}

#[derive(Debug)]
struct LogTail {
    cap: usize,
    buf: Vec<u8>,
}

impl LogTail {
    fn with_capacity(cap: usize) -> Self {
        Self {
            cap,
            buf: Vec::new(),
        }
    }
    fn push(&mut self, chunk: &[u8]) {
        self.buf.extend_from_slice(chunk);
        if self.buf.len() > self.cap {
            let drop = self.buf.len() - self.cap;
            self.buf.drain(..drop);
        }
    }
}

async fn drain_to_buf<R>(mut reader: R, buf: Arc<PMutex<LogTail>>, label: &'static str)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let mut chunk = [0u8; 4096];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => {
                let Some(bytes) = chunk.get(..n) else {
                    warn!(
                        stream = label,
                        "lambda runtime reader returned an invalid byte count"
                    );
                    break;
                };
                buf.lock().push(bytes);
                debug!(stream = label, bytes = n, "lambda runtime output");
            }
            Err(e) => {
                debug!(stream = label, error = %e, "lambda runtime stream closed");
                break;
            }
        }
    }
}
