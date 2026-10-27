//! Exact owned executable, data directory and post-exit daemon audit.

use std::{env, path::Path, process::Stdio, time::Duration};

use anyhow::{Context, Result, ensure};
use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use rustack_lambda_core::{
    artifact::ImageReference,
    docker::{DirectoryLease, DockerWire, reconcile_namespace},
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    process::{Child, Command},
    time::{Instant, sleep, timeout},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Profile {
    Disconnected,
    Connected,
    MissingRelay,
    Legacy,
    WrongEndpoint,
    Archive,
}
impl Profile {
    pub fn rejects(self) -> bool {
        matches!(
            self,
            Self::MissingRelay | Self::Legacy | Self::WrongEndpoint
        )
    }
    pub(super) fn matches_failure(self, text: &str) -> bool {
        let expected = match self {
            Self::MissingRelay => {
                "Docker startup relay preparation failed: Docker response status 404 Not Found"
            }
            Self::Legacy => "Docker ownership predates journal provenance",
            Self::WrongEndpoint => "Docker startup endpoint qualification failed",
            Self::Archive => "Docker archive requires immutable image lease restoration",
            _ => return false,
        };
        text.contains(expected)
    }
    fn network(self) -> bool {
        matches!(self, Self::Connected | Self::WrongEndpoint)
    }
}
#[derive(Debug)]
pub(super) struct Server {
    child: Child,
    pub root: Option<TempDir>,
    pub endpoint: String,
    profile: Profile,
    sources: Vec<(ImageReference, String)>,
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(60)
}
pub(super) fn image() -> Result<String> {
    Ok(env::var("RUSTACK_RUNTIME_IMAGE")?)
}
pub(super) fn architecture() -> Result<String> {
    let value = env::var("RUSTACK_RUNTIME_ARCH")?;
    ensure!(
        matches!(value.as_str(), "arm64" | "amd64"),
        "explicit runtime architecture"
    );
    Ok(value)
}
pub(super) fn handler() -> Result<String> {
    Ok(format!("rustack-p0-handler:{}", architecture()?))
}
pub(super) fn socket() -> Result<String> {
    Ok(format!(
        "unix://{}",
        env::var("RUSTACK_RUNTIME_DOCKER_SOCKET")?
    ))
}
impl Server {
    pub async fn spawn(profile: Profile) -> Result<Self> {
        let root = tempfile::tempdir()?;
        let reserved = TcpListener::bind("0.0.0.0:0").await?;
        let port = reserved.local_addr()?.port();
        drop(reserved);
        let config = root.path().join("config.yaml");
        fs::write(
            &config,
            "budgets:\n  startupSeconds: 180\n  shutdownSeconds: 60\n  lambdaDockerIoSeconds: \
             60\n  lambdaDockerStartSeconds: 60\n  lambdaDockerCleanupSeconds: 60\n  \
             lambdaDockerEnvironments: 2\n  lambdaDockerImagePins: 16\n  \
             lambdaDockerDeploymentJobs: 2\n",
        )
        .await?;
        if profile == Profile::Legacy {
            let lease = DirectoryLease::acquire(root.path().join("data"), deadline()).await?;
            drop(lease);
            let file = root.path().join("data/.rustack-lambda-docker/owner.v1");
            let mut bytes = fs::read(&file).await?;
            bytes
                .get_mut(..8)
                .context("owner magic")?
                .copy_from_slice(b"RSDKOWN1");
            let digest = Sha256::digest(bytes.get(..24).context("owner payload")?);
            bytes
                .get_mut(24..)
                .context("owner checksum")?
                .copy_from_slice(&digest);
            fs::write(&file, &bytes).await?;
            fs::write(root.path().join("legacy.expected"), bytes).await?;
        }
        let mut wire = DockerWire::connect(socket()?.parse()?, deadline()).await?;
        let result = async {
            let mut sources = Vec::new();
            for reference in [image()?, handler()?] {
                let reference: ImageReference = reference.parse()?;
                let id = wire
                    .port()
                    .inspect_image(&reference, deadline())
                    .await?
                    .id
                    .context("source image identity")?;
                sources.push((reference, id));
            }
            Ok::<_, anyhow::Error>(sources)
        }
        .await;
        let stopped = wire.shutdown().await;
        stopped?;
        let sources = result?;
        fs::copy(env!("CARGO_BIN_EXE_rustack"), root.path().join("rustack")).await?;
        let child = launch(root.path(), profile, port).await?;
        Ok(Self {
            child,
            root: Some(root),
            endpoint: format!("http://127.0.0.1:{port}"),
            profile,
            sources,
        })
    }
    pub async fn track_image(&mut self, source: &str) -> Result<String> {
        let reference: ImageReference = source.parse()?;
        let mut wire = DockerWire::connect(socket()?.parse()?, deadline()).await?;
        let result = wire.port().inspect_image(&reference, deadline()).await;
        let stopped = wire.shutdown().await;
        stopped?;
        let id = result?.id.context("fixture source identity")?;
        ensure!(
            id.len() == 71
                && id.starts_with("sha256:")
                && id
                    .get(7..)
                    .is_some_and(|value| value.bytes().all(|byte| byte.is_ascii_hexdigit())),
            "invalid source content identity"
        );
        self.sources.push((reference, id.clone()));
        Ok(id)
    }
    pub fn path(&self) -> Result<&Path> {
        Ok(self.root.as_ref().context("owned evidence root")?.path())
    }
    pub async fn ready(&mut self) -> Result<Value> {
        let at = Instant::now() + Duration::from_secs(180);
        loop {
            ensure!(
                self.child.try_wait()?.is_none(),
                "ordinary binary exited before readiness"
            );
            if let Ok((status, value)) = get(&self.endpoint, "/_rustack/capabilities").await {
                if value["ready"] == true {
                    ensure!(
                        status == 200 && value["workers"]["docker"]["ready"] == true,
                        "false backend readiness"
                    );
                    ensure!(
                        value["workers"]["docker"]["platform"]
                            == format!("linux/{}", architecture()?),
                        "wrong qualified platform"
                    );
                    ensure!(
                        value["workers"]["docker"]["sdkGateway"] == self.profile.network(),
                        "wrong endpoint capability"
                    );
                    ensure!(
                        value["workers"]["docker"]["zip"] == false,
                        "false ZIP capability"
                    );
                    return Ok(value);
                }
            }
            ensure!(Instant::now() < at, "ordinary startup observer expired");
            sleep(Duration::from_millis(25)).await;
        }
    }
    pub async fn failure(&mut self) -> Result<()> {
        let at = Instant::now() + Duration::from_secs(180);
        loop {
            if let Some(status) = self.child.try_wait()? {
                ensure!(!status.success(), "negative startup exited successfully");
                let file = fs::File::open(self.path()?.join("server.log")).await?;
                let mut bytes = Vec::new();
                file.take(32769).read_to_end(&mut bytes).await?;
                ensure!(bytes.len() <= 32768, "negative startup log cap");
                let text = std::str::from_utf8(&bytes)?;
                ensure!(
                    self.profile.matches_failure(text),
                    "startup failed at an unrelated prerequisite: {text}"
                );
                return Ok(());
            }
            if let Ok((_, value)) = get(&self.endpoint, "/_rustack/capabilities").await {
                ensure!(
                    value["ready"] != true,
                    "negative startup opened ordinary ingress"
                );
            }
            ensure!(Instant::now() < at, "negative startup observer expired");
            sleep(Duration::from_millis(25)).await;
        }
    }
    pub async fn stop(&mut self, success: bool) -> Result<()> {
        if self.child.try_wait()?.is_none() {
            let id = self.child.id().context("owned server PID")?;
            kill(Pid::from_raw(id.try_into()?), Signal::SIGTERM)?;
        }
        let result = timeout(Duration::from_secs(70), self.child.wait()).await;
        let status = if let Ok(result) = result {
            result?
        } else {
            self.child.start_kill()?;
            timeout(Duration::from_secs(5), self.child.wait()).await??;
            anyhow::bail!("forced server reap; graceful cleanup unproven");
        };
        ensure!(
            status.success() == success,
            "unexpected ordinary server exit status"
        );
        Ok(())
    }
    pub async fn restart(&mut self) -> Result<()> {
        let port = self
            .endpoint
            .rsplit(':')
            .next()
            .context("owned endpoint port")?
            .parse()?;
        self.child = launch(self.path()?, self.profile, port).await?;
        Ok(())
    }
    pub async fn audit(&self) -> Result<()> {
        ensure!(self.child.id().is_none(), "audit before child reap");
        let path = self.path()?;
        let lease = DirectoryLease::acquire(path.join("data"), deadline()).await?;
        let mut wire = DockerWire::connect(socket()?.parse()?, deadline()).await?;
        let result = async {
            if self.profile == Profile::Legacy {
                ensure!(
                    fs::read(path.join("data/.rustack-lambda-docker/owner.v1")).await?
                        == fs::read(path.join("legacy.expected")).await?,
                    "legacy identity was rewritten"
                );
                ensure!(
                    !fs::try_exists(path.join("data/.rustack-lambda-docker/daemon.v1")).await?,
                    "legacy startup reached daemon binding"
                );
            } else {
                ensure!(
                    fs::try_exists(path.join("data/.rustack-lambda-docker/daemon.v1")).await?,
                    "fixture never reached production daemon binding"
                );
                let port = wire.port().bind_directory(&lease, deadline()).await?;
                let recovery = reconcile_namespace(&lease, &port, deadline()).await?;
                ensure!(
                    recovery.containers.settled == 0
                        && recovery.containers.remaining == 0
                        && recovery.pins.settled == 0
                        && recovery.pins.remaining == 0,
                    "ordinary shutdown required post-exit recovery"
                );
            }
            for (reference, id) in &self.sources {
                ensure!(
                    wire.port()
                        .inspect_image(reference, deadline())
                        .await?
                        .id
                        .as_ref()
                        == Some(id),
                    "operator source changed"
                );
            }
            Ok::<(), anyhow::Error>(())
        }
        .await;
        let stopped = wire.shutdown().await;
        result?;
        stopped?;
        Ok(())
    }
    pub async fn retain_on_error(&mut self) {
        if let Ok(path) = self.path() {
            if let Ok(file) = fs::File::open(path.join("server.log")).await {
                let mut bytes = Vec::new();
                let _ = file.take(32768).read_to_end(&mut bytes).await;
                eprintln!(
                    "ordinary server diagnostics: {}",
                    String::from_utf8_lossy(&bytes)
                );
            }
        }
        if let Some(root) = self.root.take() {
            eprintln!("retained startup evidence: {}", root.keep().display());
        }
    }
}
async fn launch(root: &Path, profile: Profile, port: u16) -> Result<Child> {
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("server.log"))
        .await?
        .into_std()
        .await;
    let mut command = Command::new(root.join("rustack"));
    command
        .env_clear()
        .env("SERVICES", "lambda,stepfunctions")
        .env("GATEWAY_LISTEN", format!("0.0.0.0:{port}"))
        .env("RUSTACK_CONFIG", root.join("config.yaml"))
        .env("DATA_DIR", root.join("data"))
        .env("LAMBDA_EXECUTOR", "docker")
        .env("LAMBDA_DOCKER_SOCKET", socket()?)
        .env(
            "LAMBDA_DOCKER_RELAY_IMAGE",
            if profile == Profile::MissingRelay {
                format!("missing-{}:never", uuid::Uuid::new_v4().simple())
            } else {
                image()?
            },
        )
        .env(
            "LAMBDA_DOCKER_CROSS_ARCHITECTURE",
            if architecture()? == "amd64" {
                "true"
            } else {
                "false"
            },
        )
        .env("LAMBDA_INIT_TIMEOUT_SECS", "5")
        .env("LAMBDA_SKIP_SIGNATURE_VALIDATION", "false")
        .env("STEPFUNCTIONS_SKIP_SIGNATURE_VALIDATION", "false")
        .env("ACCESS_KEY", "local-key")
        .env("SECRET_KEY", "local-secret")
        .env("LOG_LEVEL", "error")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .kill_on_drop(true);
    if profile.network() {
        command.env(
            "LAMBDA_DOCKER_NETWORK",
            env::var("RUSTACK_RUNTIME_NETWORK")?,
        );
        let host = env::var("RUSTACK_RUNTIME_GATEWAY_HOST")?.parse::<std::net::IpAddr>()?;
        let target = if profile == Profile::WrongEndpoint {
            let closed = TcpListener::bind("0.0.0.0:0").await?;
            closed.local_addr()?.port()
        } else {
            port
        };
        command.env(
            "LAMBDA_DOCKER_GATEWAY_ENDPOINT",
            format!("http://{}", std::net::SocketAddr::new(host, target)),
        );
    } else {
        command.env("LAMBDA_DOCKER_NETWORK", "none");
    }
    if profile == Profile::Archive {
        command
            .arg("--snapshot")
            .arg("images")
            .env("RUSTACK_SNAPSHOT_DIR", root.join("snapshots"));
    }
    Ok(command.spawn()?)
}
pub(super) async fn get(endpoint: &str, path: &str) -> Result<(u16, Value)> {
    timeout(Duration::from_secs(2), async {
        let mut stream =
            TcpStream::connect(endpoint.strip_prefix("http://").context("local endpoint")?).await?;
        stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await?;
        let mut bytes = Vec::new();
        stream.take(65537).read_to_end(&mut bytes).await?;
        ensure!(bytes.len() <= 65536, "diagnostic response cap");
        let text = std::str::from_utf8(&bytes)?;
        let (head, body) = text.split_once("\r\n\r\n").context("HTTP framing")?;
        let status = head
            .split_whitespace()
            .nth(1)
            .context("HTTP status")?
            .parse()?;
        Ok((status, serde_json::from_str(body)?))
    })
    .await
    .context("diagnostic observer expired")?
}
