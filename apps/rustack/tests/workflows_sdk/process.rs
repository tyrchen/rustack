//! Retained real server ownership; all SDK observers are separate from process life.

use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use tempfile::TempDir;
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    process::{Child, Command},
    sync::watch,
    task::JoinHandle,
    time::{Instant, sleep, timeout, timeout_at},
};

use super::runner::Runner;

pub(super) const ACCESS: &str = "local-key";
pub(super) const SECRET: &str = "local-secret";

#[derive(Debug)]
struct Resources {
    child: Child,
    directory: Option<TempDir>,
    markers: PathBuf,
    cleanup_proven: bool,
}
#[derive(Debug)]
struct Owner {
    state: Option<Resources>,
    stop: watch::Receiver<bool>,
}
#[derive(Debug)]
pub(super) struct Server {
    pub pid: u32,
    pub endpoint: String,
    pub markers: PathBuf,
    pub bootstrap: PathBuf,
    stop: watch::Sender<bool>,
    root: Option<JoinHandle<Result<usize>>>,
    joined: Option<Result<usize, Arc<anyhow::Error>>>,
}
impl Server {
    pub async fn capabilities(&self) -> Result<serde_json::Value> {
        let address = self
            .endpoint
            .strip_prefix("http://")
            .context("owned server endpoint")?;
        timeout(Duration::from_secs(2),async {
            let mut stream=TcpStream::connect(address).await?;
            stream.write_all(b"GET /_rustack/capabilities HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").await?;
            let mut response=Vec::new();stream.take(16_385).read_to_end(&mut response).await?;
            ensure!(response.len()<=16_384,"oversized capability response");
            let text=std::str::from_utf8(&response)?;
            ensure!(text.starts_with("HTTP/1.1 200"),"capability response failed");
            let (_,body)=text.split_once("\r\n\r\n").context("capability envelope")?;
            Ok(serde_json::from_str(body)?)
        }).await.context("capability observer expired")?
    }
    pub async fn start() -> Result<Self> {
        Self::start_with_dispatch(32).await
    }
    pub async fn start_with_dispatch(dispatch: usize) -> Result<Self> {
        ensure!(
            (1..=32).contains(&dispatch),
            "invalid fixture dispatch capacity"
        );
        let bootstrap = PathBuf::from(
            env::var_os("RUSTACK_WORKFLOWS_BOOTSTRAP")
                .context("run make test-stepfunctions-sdk")?,
        );
        ensure!(
            (1..=32 * 1024 * 1024).contains(&fs::metadata(&bootstrap).await?.len()),
            "invalid bootstrap size"
        );
        let directory = tempfile::tempdir()?;
        let markers = directory.path().join("processes");
        fs::create_dir(&markers).await?;
        fs::create_dir(directory.path().join("gates")).await?;
        fs::create_dir(directory.path().join("callbacks")).await?;
        fs::create_dir(directory.path().join("callback-gates")).await?;
        let config = directory.path().join("rustack.yaml");
        fs::write(
            &config,
            format!(
                "budgets:\n  workflowTransitions: 80\n  workflowHistoryBytes: 1048576\n  \
                 workflowTotalHistoryBytes: 8388608\n  workflowDrainSeconds: 0\n  \
                 workflowDispatches: {dispatch}\n  workflowCallbackTokens: 2\n  \
                 workflowCallbackTombstones: 64\n  workflowCallbackCommands: 8\n  \
                 workflowCallbackWorkers: 1\n"
            ),
        )
        .await?;
        let reservation = TcpListener::bind("127.0.0.1:0").await?;
        let address = reservation.local_addr()?;
        drop(reservation);
        let child = Command::new(env!("CARGO_BIN_EXE_rustack"))
            .env_clear()
            .env("TMPDIR", directory.path())
            .env("RUSTACK_CONFIG", &config)
            .env("GATEWAY_LISTEN", address.to_string())
            .env("SERVICES", "lambda,stepfunctions")
            .env("LAMBDA_EXECUTOR", "native")
            .env("LAMBDA_SKIP_SIGNATURE_VALIDATION", "false")
            .env("STEPFUNCTIONS_SKIP_SIGNATURE_VALIDATION", "false")
            .env("ACCESS_KEY", ACCESS)
            .env("SECRET_KEY", SECRET)
            .env("LOG_LEVEL", "error")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("spawn actual Rustack binary")?;
        let pid = child.id().context("owned server PID")?;
        let (stop, receiver) = watch::channel(false);
        let owner = Owner {
            state: Some(Resources {
                child,
                directory: Some(directory),
                markers: markers.clone(),
                cleanup_proven: false,
            }),
            stop: receiver,
        };
        let mut server = Self {
            pid,
            endpoint: format!("http://{address}"),
            markers,
            bootstrap,
            stop,
            root: Some(tokio::spawn(owner.run())),
            joined: None,
        };
        let ready = server.await_ready(&address.to_string()).await;
        if let Err(error) = ready {
            let cleanup = server
                .shutdown(Instant::now() + Duration::from_secs(40))
                .await;
            cleanup?;
            return Err(error);
        }
        Ok(server)
    }
    async fn await_ready(&self, address: &str) -> Result<()> {
        timeout(Duration::from_secs(20), async {
            loop {
                ensure!(
                    !self.root.as_ref().is_none_or(JoinHandle::is_finished),
                    "server exited before readiness"
                );
                if let Ok(Ok(())) = timeout(Duration::from_secs(1), health(address)).await {
                    return Ok(());
                }
                sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .context("server startup deadline")?
    }
    pub async fn shutdown(&mut self, deadline: Instant) -> Result<usize> {
        self.stop.send_replace(true);
        if self.joined.is_none() {
            let root = self.root.as_mut().context("missing retained server root")?;
            let result = timeout_at(deadline, root)
                .await
                .context("server shutdown observer expired; root retained")?;
            self.root = None;
            self.joined = Some(match result {
                Ok(result) => result.map_err(Arc::new),
                Err(error) => Err(Arc::new(error.into())),
            });
        }
        match &self.joined {
            Some(Ok(observed)) => Ok(*observed),
            Some(Err(error)) => bail!("server cleanup failed: {error:#}"),
            None => bail!("missing server outcome"),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.send_replace(true);
        // Intentional independent root: an SDK/cleanup observer does not own it.
        // Its Drop guard preserves files if the Tokio runtime itself is lost.
        if let Some(root) = self.root.take() {
            drop(root);
        }
    }
}
impl Owner {
    async fn run(mut self) -> Result<usize> {
        let state = self.state.as_mut().context("missing owned server state")?;
        let mut gate_failure = None;
        let status = tokio::select! {
            result = state.child.wait() => {
                let status = result?;
                ensure!(*self.stop.borrow(), "server exited without requested shutdown");
                status
            },
            _ = self.stop.changed() => {
                if let Err(error)=release_fixture_gates(&state.markers).await {gate_failure=Some(error);}
                let pid = state.child.id().context("missing owned server PID")?;
                let signal=Runner::spawn(Path::new("/bin/kill"), &[OsString::from("-TERM"), OsString::from(pid.to_string())])?
                    .wait(Instant::now()+Duration::from_secs(5)).await?;
                ensure!(!signal.forced && signal.status.success(), "cannot signal owned server");
                if let Ok(status) = timeout(Duration::from_secs(35), state.child.wait()).await { status? } else {
                        state.child.start_kill()?;
                        let _reaped = timeout(Duration::from_secs(5), state.child.wait()).await;
                        bail!("server required forced termination; ownership evidence retained");
                }
            }
        };
        ensure!(status.success(), "server coordinated shutdown failed");
        let observed = timeout(Duration::from_secs(10), verify_absent(&state.markers))
            .await
            .context("PID evidence deadline")??;
        state.cleanup_proven = true;
        if let Some(error) = gate_failure {
            return Err(error.context("fixture gates failed after coordinated process cleanup"));
        }
        self.state = None;
        Ok(observed)
    }
}
async fn release_fixture_gates(markers: &Path) -> Result<()> {
    let root = markers.parent().context("owned fixture directory")?;
    timeout(
        Duration::from_secs(2),
        fs::write(root.join("callback-gates/all.release"), b"cleanup"),
    )
    .await??;
    Ok(())
}
impl Drop for Owner {
    fn drop(&mut self) {
        if let Some(state) = &mut self.state {
            if let Some(directory) = state.directory.take() {
                let path = directory.keep();
                let reason = if state.cleanup_proven {
                    "SDK qualification failed after proven process cleanup"
                } else {
                    "SDK cleanup was not proven"
                };
                eprintln!("{reason}; retained evidence at {}", path.display());
            }
        }
    }
}
async fn verify_absent(markers: &Path) -> Result<usize> {
    let mut entries = fs::read_dir(markers).await?;
    let mut count = 0;
    while let Some(entry) = entries.next_entry().await? {
        count += 1;
        ensure!(
            count <= 64 && entry.file_type().await?.is_file(),
            "invalid PID evidence collection"
        );
        ensure!(
            entry.metadata().await?.len() <= 11,
            "oversized PID evidence"
        );
        let raw = fs::read_to_string(entry.path()).await?;
        let pid: u32 = raw.parse()?;
        ensure!(
            pid > 1 && entry.file_name() == format!("pid-{pid}").as_str(),
            "invalid PID identity"
        );
        assert_absent(pid).await?;
    }
    Ok(count)
}
async fn health(address: &str) -> Result<()> {
    let mut stream = TcpStream::connect(address).await?;
    stream
        .write_all(b"GET /_health/ready HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await?;
    let mut response = Vec::new();
    stream.take(16_385).read_to_end(&mut response).await?;
    ensure!(response.len() <= 16_384, "oversized health response");
    let text = std::str::from_utf8(&response)?;
    let (_, body) = text.split_once("\r\n\r\n").context("health envelope")?;
    let value: serde_json::Value = serde_json::from_str(body)?;
    ensure!(
        text.starts_with("HTTP/1.1 200") && value["ready"] == true,
        "server not ready"
    );
    Ok(())
}

pub(super) async fn assert_absent(pid: u32) -> Result<()> {
    let probe = Runner::spawn(
        Path::new("/bin/kill"),
        &[OsString::from("-0"), OsString::from(pid.to_string())],
    )?
    .wait(Instant::now() + Duration::from_secs(3))
    .await?;
    ensure!(
        !probe.forced
            && !probe.status.success()
            && String::from_utf8_lossy(&probe.stderr).contains("No such process"),
        "fixture PID absence was not proven"
    );
    Ok(())
}
