//! Native (process) execution backend.
//!
//! Spawns the function's `bootstrap` binary directly on the host. Suitable
//! only for `provided.al2` / `provided.al2023` runtimes whose architectures
//! match the host. The auto executor keeps macOS Zip functions on Squib and
//! leaves unsupported image packages for the future Docker backend.

use std::{net::SocketAddr, path::Path, process::Stdio, sync::Arc, time::Duration};

use async_trait::async_trait;
use parking_lot::Mutex as PMutex;
use tokio::{
    io::AsyncReadExt,
    process::Command,
    sync::{Semaphore, oneshot, watch},
    task::JoinSet,
};
use tracing::{debug, warn};

use super::{
    Executor, ExecutorError, InvokeRequest, InvokeResponse, PackageType,
    instance::{BackendHandle, BackendHandleObj, InstanceBackend, InstancePool, spawn_reaper},
};

/// Native process executor.
#[derive(Debug)]
pub struct NativeExecutor {
    pool: Arc<InstancePool>,
    cancel_tx: watch::Sender<bool>,
    reaper: PMutex<Option<tokio::task::JoinHandle<()>>>,
    processes: Arc<PMutex<JoinSet<()>>>,
}

impl Drop for NativeExecutor {
    fn drop(&mut self) {
        self.pool.shutdown();
        if let Some(reaper) = self.reaper.lock().take() {
            reaper.abort();
        }
        self.processes.lock().abort_all();
    }
}

impl NativeExecutor {
    /// Build a new native executor.
    #[must_use]
    pub fn new(max_warm: usize, idle_timeout: Duration, init_timeout: Duration) -> Self {
        let processes = Arc::new(PMutex::new(JoinSet::new()));
        let backend = Arc::new(NativeBackend {
            processes: Arc::clone(&processes),
            capacity: Arc::new(Semaphore::new(32)),
        });
        let pool = Arc::new(InstancePool::new(
            backend,
            max_warm,
            idle_timeout,
            init_timeout,
        ));
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let reaper = spawn_reaper(Arc::clone(&pool), Duration::from_secs(30), cancel_rx);
        Self {
            pool,
            cancel_tx,
            reaper: PMutex::new(Some(reaper)),
            processes,
        }
    }
}

#[async_trait]
impl Executor for NativeExecutor {
    async fn invoke(&self, req: InvokeRequest) -> Result<InvokeResponse, ExecutorError> {
        if req.package_type != PackageType::Zip {
            return Err(ExecutorError::Unsupported(
                "native backend only supports Zip packages".to_owned(),
            ));
        }
        let code_root = req
            .code_root
            .as_ref()
            .ok_or_else(|| ExecutorError::InvalidCode("missing code root".to_owned()))?;
        let bootstrap = code_root.join("bootstrap");
        ensure_executable(&bootstrap).await?;
        if !bootstrap_runs_on_host(&bootstrap, &req.architectures).await? {
            return Err(ExecutorError::Unsupported(format!(
                "bootstrap {} cannot run on host {}/{}; use docker backend",
                bootstrap.display(),
                std::env::consts::OS,
                std::env::consts::ARCH,
            )));
        }
        self.pool.invoke(req).await
    }

    async fn shutdown(&self) {
        let _ = self.cancel_tx.send(true);
        let reaper = self.reaper.lock().take();
        if let Some(r) = reaper {
            if let Err(error) = r.await {
                warn!(%error, "Lambda idle reaper failed");
            }
        }
        self.pool.shutdown();
        let mut processes = std::mem::take(&mut *self.processes.lock());
        while let Some(result) = processes.join_next().await {
            if let Err(error) = result {
                warn!(%error, "Lambda process supervisor failed");
            }
        }
    }
}

#[derive(Debug)]
struct NativeBackend {
    processes: Arc<PMutex<JoinSet<()>>>,
    capacity: Arc<Semaphore>,
}

#[async_trait]
impl InstanceBackend for NativeBackend {
    fn has_capacity(&self) -> bool {
        self.capacity.available_permits() != 0
    }
    async fn spawn(
        &self,
        req: &InvokeRequest,
        runtime_api_addr: SocketAddr,
    ) -> Result<BackendHandle, ExecutorError> {
        let code_root = req
            .code_root
            .clone()
            .ok_or_else(|| ExecutorError::InvalidCode("missing code root".to_owned()))?;
        let bootstrap = code_root.join("bootstrap");

        let mut cmd = Command::new(&bootstrap);
        cmd.current_dir(&code_root)
            .env_clear()
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .kill_on_drop(true);
        for (k, v) in lambda_env_vars(req, runtime_api_addr) {
            cmd.env(k, v);
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
        let mut processes = self.processes.lock();
        while let Some(result) = processes.try_join_next() {
            if let Err(error) = result {
                warn!(%error, "Lambda process supervisor failed");
            }
        }
        processes.spawn(async move {
            tokio::select! {
                _ = cancelled => { if let Err(error) = child.kill().await { warn!(%error, "Lambda process cancellation failed"); } },
                result = child.wait() => { if let Err(error) = result { warn!(%error, "Lambda process wait failed"); } },
            }
            logs.abort_all();
            while let Some(result) = logs.join_next().await { if let Err(error) = result { if !error.is_cancelled() { warn!(%error, "Lambda log task failed"); } } }
        });
        Ok(BackendHandle::new(NativeHandle {
            cancel: Some(cancel),
        }))
    }
}

#[derive(Debug)]
struct NativeHandle {
    cancel: Option<oneshot::Sender<()>>,
}

impl BackendHandleObj for NativeHandle {
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
    declared_archs: &[String],
) -> Result<bool, ExecutorError> {
    // Architecture check first — declared `architectures` must include the
    // host arch.
    let host_arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "arm64",
        other => other,
    };
    if !declared_archs.iter().any(|a| a == host_arch) {
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
    let mut env = Vec::with_capacity(16 + req.environment.len());
    let task_root = req
        .code_root
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    env.push(("AWS_LAMBDA_RUNTIME_API".into(), runtime_api.to_string()));
    env.push(("AWS_LAMBDA_FUNCTION_NAME".into(), req.function_name.clone()));
    env.push(("AWS_LAMBDA_FUNCTION_VERSION".into(), req.qualifier.clone()));
    env.push((
        "AWS_LAMBDA_FUNCTION_MEMORY_SIZE".into(),
        req.memory_mb.to_string(),
    ));
    env.push((
        "AWS_LAMBDA_FUNCTION_TIMEOUT".into(),
        req.timeout.as_secs().to_string(),
    ));
    env.push((
        "AWS_LAMBDA_LOG_GROUP_NAME".into(),
        format!("/aws/lambda/{}", req.function_name),
    ));
    env.push((
        "AWS_LAMBDA_LOG_STREAM_NAME".into(),
        uuid::Uuid::new_v4().to_string(),
    ));
    env.push(("_HANDLER".into(), req.handler.clone().unwrap_or_default()));
    env.push(("LAMBDA_TASK_ROOT".into(), task_root.clone()));
    env.push(("LAMBDA_RUNTIME_DIR".into(), task_root));
    env.push(("TZ".into(), ":UTC".into()));
    env.push(("PATH".into(), default_path()));
    // Sensible defaults so SDK calls back to the same rustack instance work.
    env.push(("AWS_ACCESS_KEY_ID".into(), "test".into()));
    env.push(("AWS_SECRET_ACCESS_KEY".into(), "test".into()));
    env.push(("AWS_SESSION_TOKEN".into(), "test".into()));
    // User env overrides last, but do not let it shadow the runtime API var.
    for (k, v) in &req.environment {
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
                buf.lock().push(&chunk[..n]);
                debug!(stream = label, bytes = n, "lambda runtime output");
            }
            Err(e) => {
                debug!(stream = label, error = %e, "lambda runtime stream closed");
                break;
            }
        }
    }
}
