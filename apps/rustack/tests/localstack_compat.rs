//! Original external pytest cases against a directly owned actual Rustack process.
#![cfg(all(unix, feature = "lambda", feature = "stepfunctions"))]
#![forbid(unsafe_code)]

#[path = "workflows_sdk/runner.rs"]
mod runner;

use std::{
    env,
    ffi::OsString,
    fmt::Write as FmtWrite,
    fs::Permissions,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use runner::{Runner, interpreter};
use rustack_lambda_core::docker::{DirectoryLease, DockerWire, reconcile_namespace};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::{
    fs,
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    process::{Child, Command},
    task::JoinHandle,
    time::{Instant, sleep, timeout, timeout_at},
};
use uuid::Uuid;

#[derive(Debug)]
struct Server {
    child: Child,
    output: Vec<JoinHandle<Result<Vec<u8>>>>,
    endpoint: String,
    root: PathBuf,
    docker: bool,
}

impl Server {
    async fn spawn(root: &Path, docker: bool) -> Result<Self> {
        let reservation = TcpListener::bind("127.0.0.1:0").await?;
        let address = reservation.local_addr()?;
        drop(reservation);
        let config = root.join("config.yaml");
        let access = format!("compat-{}", Uuid::new_v4().simple());
        let secret = Uuid::new_v4().simple().to_string();
        let auth = root.join("auth.json");
        save(
            &auth,
            &serde_json::to_vec(&json!({"accessKey":access, "secretKey":secret}))?,
        )
        .await?;
        save(
            &config,
            b"budgets:\n  startupSeconds: 180\n  shutdownSeconds: 60\n  workflowDrainSeconds: 0\n  \
             workflowTransitions: 100000\n  workflowHistoryBytes: 33554432\n  \
             workflowTotalHistoryBytes: 268435456\n  lambdaDockerIoSeconds: 60\n  \
             lambdaDockerStartSeconds: 60\n  lambdaDockerCleanupSeconds: 60\n  \
             lambdaDockerEnvironments: 2\n",
        )
        .await?;
        let mut command = Command::new(env!("CARGO_BIN_EXE_rustack"));
        command
            .env_clear()
            .env("GATEWAY_LISTEN", address.to_string())
            .env(
                "SERVICES",
                if docker {
                    "stepfunctions,lambda"
                } else {
                    "stepfunctions"
                },
            )
            .env("RUSTACK_CONFIG", &config)
            .env("DATA_DIR", root.join("data"))
            .env(
                "LAMBDA_EXECUTOR",
                if docker { "docker" } else { "disabled" },
            )
            .env("STEPFUNCTIONS_SKIP_SIGNATURE_VALIDATION", "false")
            .env("LAMBDA_SKIP_SIGNATURE_VALIDATION", "false")
            .env("ACCESS_KEY", access)
            .env("SECRET_KEY", secret)
            .env("LOG_LEVEL", "error")
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if docker {
            command
                .env(
                    "LAMBDA_DOCKER_SOCKET",
                    format!("unix://{}", env::var("RUSTACK_RUNTIME_DOCKER_SOCKET")?),
                )
                .env(
                    "LAMBDA_DOCKER_RELAY_IMAGE",
                    env::var("RUSTACK_RUNTIME_IMAGE")?,
                )
                .env("LAMBDA_DOCKER_NETWORK", "none")
                .env("LAMBDA_DOCKER_CROSS_ARCHITECTURE", "false")
                .env("LAMBDA_INIT_TIMEOUT_SECS", "20");
        }
        let child = command
            .spawn()
            .context("start actual compatibility server")?;
        let mut owned = Self {
            child,
            output: Vec::new(),
            endpoint: format!("http://{address}"),
            root: root.to_owned(),
            docker,
        };
        if let Some(stream) = owned.child.stdout.take() {
            owned.output.push(tokio::spawn(capture(stream)));
        }
        if let Some(stream) = owned.child.stderr.take() {
            owned.output.push(tokio::spawn(capture(stream)));
        }
        Ok(owned)
    }

    async fn ready(&mut self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(180);
        while Instant::now() < deadline {
            ensure!(
                self.child.try_wait()?.is_none(),
                "compatibility server exited during startup"
            );
            if health(&self.endpoint).await.is_ok() {
                return Ok(());
            }
            sleep(Duration::from_millis(50)).await;
        }
        bail!("compatibility readiness deadline")
    }

    async fn stop(&mut self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(90);
        let status = self.retire(deadline).await;
        let mut logs = Vec::new();
        let mut errors = Vec::new();
        for handle in self.output.drain(..) {
            match join_capture(handle, deadline).await {
                Ok(bytes) => logs.extend(bytes),
                Err(error) => errors.push(error.to_string()),
            }
        }
        let saved = save(&self.root.join("server.log"), &logs).await;
        let server_joined = self.child.id().is_none();
        let audit = if self.docker && server_joined {
            self.audit().await
        } else {
            Ok(())
        };
        let original_bytes = timeout(
            Duration::from_secs(5),
            fs::read(self.root.join("server-intent.json")),
        )
        .await
        .context("server intent read timeout")??;
        let original: Value = serde_json::from_slice(&original_bytes)?;
        let expected = original
            .get("sha256")
            .and_then(Value::as_str)
            .context("server intent fingerprint")?;
        let same_binary = binary_hash().await? == expected;
        let cleanup = json!({"serverJoined":server_joined, "captureTasksJoined":true,
            "ownedResourcesAbsent":server_joined && audit.is_ok(), "serverBinaryUnchanged":same_binary,
            "dockerAuditPerformed":self.docker && server_joined,
            "exitSuccess":status.as_ref().is_ok_and(ExitStatus::success),
            "serverIntentSha256":hexadecimal(&Sha256::digest(&original_bytes))?});
        save(
            &self.root.join("server-cleanup.json"),
            &serde_json::to_vec(&cleanup)?,
        )
        .await?;
        let status = status?;
        audit?;
        saved?;
        ensure!(
            same_binary,
            "compatibility server binary changed during execution"
        );
        ensure!(
            errors.is_empty(),
            "compatibility capture failure: {errors:?}"
        );
        ensure!(status.success(), "compatibility server exit: {status}");
        Ok(())
    }

    async fn retire(&mut self, deadline: Instant) -> Result<ExitStatus> {
        if let Some(status) = self.child.try_wait()? {
            return Ok(status);
        }
        let pid = i32::try_from(self.child.id().context("owned compatibility server PID")?)?;
        let signalled = kill(Pid::from_raw(pid), Signal::SIGTERM);
        let status = if let Ok(status) =
            timeout_at(deadline - Duration::from_secs(10), self.child.wait()).await
        {
            status?
        } else {
            let killed = self.child.start_kill();
            let status = timeout_at(deadline, self.child.wait())
                .await
                .context("forced server reap")??;
            killed?;
            status
        };
        signalled.context("stop exact compatibility server")?;
        Ok(status)
    }

    async fn audit(&self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(60);
        let lease = DirectoryLease::acquire(self.root.join("data"), deadline).await?;
        let socket = format!("unix://{}", env::var("RUSTACK_RUNTIME_DOCKER_SOCKET")?);
        let mut wire = DockerWire::connect(socket.parse()?, deadline).await?;
        let checked = async {
            let port = wire.port().bind_directory(&lease, deadline).await?;
            let report = reconcile_namespace(&lease, &port, deadline).await?;
            ensure!(
                report.containers.settled == 0
                    && report.containers.remaining == 0
                    && report.pins.settled == 0
                    && report.pins.remaining == 0
                    && report.code.settled == 0
                    && report.code.remaining == 0,
                "compatibility shutdown left owned Docker resources"
            );
            Ok::<(), anyhow::Error>(())
        }
        .await;
        let stopped = wire.shutdown().await;
        checked?;
        stopped?;
        Ok(())
    }
}

async fn capture<R: AsyncRead + Unpin>(source: R) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    source.take(1_048_577).read_to_end(&mut bytes).await?;
    ensure!(
        bytes.len() <= 1_048_576,
        "compatibility server diagnostic bound"
    );
    Ok(bytes)
}

async fn join_capture(
    mut handle: JoinHandle<Result<Vec<u8>>>,
    deadline: Instant,
) -> Result<Vec<u8>> {
    if let Ok(result) = timeout_at(deadline, &mut handle).await {
        result.context("compatibility capture task panicked")?
    } else {
        handle.abort();
        let joined = handle.await;
        if let Err(error) = joined {
            ensure!(
                error.is_cancelled(),
                "capture task panicked during retirement: {error}"
            );
        }
        bail!("compatibility capture deadline; task aborted and joined")
    }
}

async fn save(path: &Path, bytes: &[u8]) -> Result<()> {
    timeout(Duration::from_secs(5), async {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .await?;
        file.write_all(bytes).await?;
        file.sync_all().await?;
        Ok::<(), anyhow::Error>(())
    })
    .await
    .context("compatibility evidence IO timeout")?
}

async fn binary_hash() -> Result<String> {
    timeout(Duration::from_secs(10), async {
        let mut source = fs::File::open(env!("CARGO_BIN_EXE_rustack")).await?;
        let mut hash = Sha256::new();
        let mut bytes = vec![0_u8; 65536].into_boxed_slice();
        let mut total = 0_u64;
        loop {
            let count = source.read(&mut bytes).await?;
            if count == 0 {
                break;
            }
            total = total.saturating_add(u64::try_from(count)?);
            ensure!(total <= 268_435_456, "compatibility executable size bound");
            hash.update(bytes.get(..count).context("executable read slice")?);
        }
        hexadecimal(&hash.finalize())
    })
    .await
    .context("compatibility executable fingerprint timeout")?
}

fn hexadecimal(bytes: &[u8]) -> Result<String> {
    let mut text = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        write!(&mut text, "{byte:02x}")?;
    }
    Ok(text)
}

async fn health(endpoint: &str) -> Result<()> {
    timeout(Duration::from_secs(2), async {
        let address = endpoint
            .strip_prefix("http://")
            .context("compatibility endpoint")?;
        let mut stream = TcpStream::connect(address).await?;
        stream
            .write_all(
                b"GET /_localstack/health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            )
            .await?;
        let mut bytes = Vec::new();
        stream.take(16385).read_to_end(&mut bytes).await?;
        ensure!(
            bytes.len() <= 16384 && bytes.starts_with(b"HTTP/1.1 200"),
            "compatibility readiness response"
        );
        Ok::<(), anyhow::Error>(())
    })
    .await
    .context("compatibility readiness timeout")?
}

async fn python(arguments: &[OsString]) -> Result<()> {
    let mut child = Runner::spawn(&interpreter()?, arguments)?;
    let entered = child.await_pid().await;
    let report = child.wait(Instant::now() + Duration::from_mins(30)).await?;
    ensure!(
        entered? == report.pid,
        "compatibility child identity changed"
    );
    if !report.status.success() {
        eprintln!(
            "compatibility child: {} {}",
            String::from_utf8_lossy(&report.stdout),
            String::from_utf8_lossy(&report.stderr)
        );
    }
    ensure!(
        !report.forced && report.status.success(),
        "external compatibility failed; retained evidence"
    );
    Ok(())
}

async fn exercise(docker: bool) -> Result<()> {
    let root = PathBuf::from(
        env::var_os("RUSTACK_LOCALSTACK_EVIDENCE")
            .context("explicit compatibility evidence directory")?,
    );
    ensure!(
        root.is_absolute(),
        "compatibility evidence must be absolute"
    );
    let parent = root.parent().context("compatibility evidence parent")?;
    let canonical = timeout(Duration::from_secs(5), fs::canonicalize(parent))
        .await
        .context("evidence parent IO timeout")??;
    ensure!(
        Some(canonical.as_path()) == root.parent(),
        "compatibility evidence parent must be canonical"
    );
    timeout(Duration::from_secs(5), fs::create_dir(&root))
        .await
        .context("create evidence timeout")??;
    timeout(
        Duration::from_secs(5),
        fs::set_permissions(&root, Permissions::from_mode(0o700)),
    )
    .await
    .context("private evidence timeout")??;
    save(
        &root.join("server-intent.json"),
        &serde_json::to_vec(&json!({
        "path":env!("CARGO_BIN_EXE_rustack"), "sha256":binary_hash().await?, "docker":docker}))?,
    )
    .await?;
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/workflows/localstack_compat.py");
    let mode = if docker { "docker" } else { "pure" };
    let images = if docker {
        env::var_os("RUSTACK_LOCALSTACK_IMAGES")
            .context("explicit immutable handler image receipt")?
    } else {
        OsString::from("-")
    };
    let socket = if docker {
        OsString::from(format!(
            "unix://{}",
            env::var("RUSTACK_RUNTIME_DOCKER_SOCKET")?
        ))
    } else {
        OsString::from("-")
    };
    python(&[
        script.clone().into_os_string(),
        "preflight".into(),
        root.clone().into_os_string(),
        mode.into(),
        images.clone(),
        socket,
    ])
    .await?;
    let mut server = Server::spawn(&root, docker).await?;
    let result = async {
        server.ready().await?;
        python(&[
            script.into_os_string(),
            "run".into(),
            root.into_os_string(),
            mode.into(),
            OsString::from(&server.endpoint),
            images,
        ])
        .await
    }
    .await;
    let cleanup = server.stop().await;
    if let Err(error) = &cleanup {
        eprintln!("compatibility cleanup failed: {error:#}");
    }
    result?;
    cleanup?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires pinned external Python environment; make test-workflows-localstack-pure"]
async fn test_should_execute_original_localstack_pure_cases() -> Result<()> {
    exercise(false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires explicit Docker and upstream handler images; make \
            test-workflows-localstack-docker"]
async fn test_should_execute_original_localstack_image_cases() -> Result<()> {
    exercise(true).await
}

#[tokio::test]
async fn test_should_join_capture_when_a_writer_outlives_server_retirement() -> Result<()> {
    let (reader, writer) = tokio::io::duplex(16);
    let task = tokio::spawn(capture(reader));
    let result = join_capture(task, Instant::now() + Duration::from_millis(10)).await;
    ensure!(result.is_err(), "held writer escaped capture deadline");
    drop(writer);
    Ok(())
}
