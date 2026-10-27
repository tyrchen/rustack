//! Explicit local Docker qualification of production HTTP/upgrade/control IO.

#![cfg(unix)]
#![forbid(unsafe_code)]

#[path = "docker_wire/code.rs"]
mod code;

use std::{collections::HashMap, env, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use bytes::Bytes;
use http::StatusCode;
use rustack_lambda_core::{
    artifact::{
        Architecture as ImageArchitecture, DeploymentArtifact, ImageReference, PinnedImage,
        ResolvedImage,
    },
    docker::{
        ContainerPolicy, DirectoryLease, DirectoryLeaseError, DockerError, DockerKey, DockerPort,
        DockerSocket, DockerWire, EnvironmentPlan, FunctionLogs, ImageLeaseConfig,
        ImageLeaseManager, ImagePinScope, ImageResolver, OutputChannel, OwnerFilter, RelayBudgets,
    },
    executor::{ExecutionRevision, ExecutorBackend},
    identity::FunctionResourceId,
};
use rustack_lambda_model::types::ImageConfig;
use rustack_lambda_runtime::{
    client::{ClientConfig, ClientEvent, ControlClient},
    metadata::local_trace_id,
    protocol::{Architecture, AttemptId, EnvironmentId, Payload},
    session::{Invocation, RuntimeResult},
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::{
    io::AsyncWriteExt,
    sync::watch,
    task::JoinHandle,
    time::{Instant, timeout_at},
};
use uuid::Uuid;

const IO: Duration = Duration::from_secs(60);
const LARGE: usize = 6 * 1024 * 1024 - 4096;
fn deadline() -> Instant {
    Instant::now() + IO
}

#[derive(Debug)]
struct Names {
    relay: DockerKey,
    function: DockerKey,
    owner: Uuid,
    run: Uuid,
    environment: EnvironmentId,
}

#[derive(Debug)]
struct Runtime {
    client: ControlClient,
    stop: watch::Sender<bool>,
    pump: Option<JoinHandle<Result<(), DockerError>>>,
    settlement: Option<Result<(), DockerError>>,
}
impl Runtime {
    async fn shutdown(&mut self) -> Result<()> {
        let protocol = self
            .client
            .shutdown(deadline())
            .await
            .map_err(anyhow::Error::from);
        self.stop.send_replace(true);
        let pump = if let Some(result) = &self.settlement {
            result.clone()
        } else {
            let result = if let Some(pump) = &mut self.pump {
                pump.await
                    .map_err(|_| DockerError::Worker)
                    .and_then(|result| result)
            } else {
                Ok(())
            };
            self.pump.take();
            self.settlement = Some(result.clone());
            result
        };
        protocol.and(pump.map_err(anyhow::Error::from))
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.stop.send_replace(true);
    }
}

#[derive(Debug)]
struct Environment {
    wire: DockerWire,
    docker: DockerPort,
    names: Names,
    runtime: Option<Runtime>,
    logs: Option<FunctionLogs>,
    images: ImageLeaseManager,
    artifacts: Vec<PinnedImage>,
    plan: Option<EnvironmentPlan>,
    lease: DirectoryLease,
    directory: Option<TempDir>,
}
impl Environment {
    async fn new() -> Result<Self> {
        let socket = env::var("RUSTACK_RUNTIME_DOCKER_SOCKET")
            .context("explicit test Docker socket required")?;
        let socket: DockerSocket = format!("unix://{socket}").parse()?;
        let wire = DockerWire::connect(socket, deadline()).await?;
        let directory = tempfile::tempdir()?;
        let lease = DirectoryLease::acquire(directory.path().to_path_buf(), deadline()).await?;
        let run = lease.run_id();
        let owner = lease.owner_id();
        let docker = wire.port().bind_directory(&lease, deadline()).await?;
        let images = ImageLeaseManager::start(
            docker.clone(),
            ImagePinScope::new(lease.clone()),
            ImageLeaseConfig::default(),
        )?;
        ensure!(
            matches!(
                DirectoryLease::acquire(directory.path().to_path_buf(), deadline()).await,
                Err(DirectoryLeaseError::InUse)
            ),
            "image owner did not retain directory exclusion"
        );
        Ok(Self {
            wire,
            docker,
            names: Names {
                relay: format!("rustack-wire-relay-{run}").parse()?,
                function: format!("rustack-wire-function-{run}").parse()?,
                owner,
                run,
                environment: EnvironmentId::default(),
            },
            runtime: None,
            logs: None,
            images,
            artifacts: Vec::new(),
            plan: None,
            lease,
            directory: Some(directory),
        })
    }

    async fn image(&mut self, reference: &str, architecture: &str) -> Result<PinnedImage> {
        let reference: ImageReference = reference.parse()?;
        let architecture = match architecture {
            "arm64" => ImageArchitecture::Arm64,
            "amd64" => ImageArchitecture::X86_64,
            _ => bail!("unknown fixture architecture"),
        };
        let resolved = ImageResolver::new(self.docker.clone())
            .resolve_local(&reference, architecture, None, deadline())
            .await?;
        let saved: ResolvedImage = serde_json::from_slice(&serde_json::to_vec(&resolved)?)?;
        let image = self.images.port().restore(&saved, None, deadline()).await?;
        ensure!(
            image.resolved().code_sha256()? == saved.code_sha256()?,
            "restored image hash changed"
        );
        self.artifacts.push(image.clone());
        Ok(image)
    }

    async fn start(&mut self) -> Result<()> {
        self.start_with(None, true).await
    }

    async fn start_with(&mut self, overrides: Option<ImageConfig>, bootstrap: bool) -> Result<()> {
        let arch = env::var("RUSTACK_RUNTIME_ARCH").context("explicit architecture required")?;
        let (architecture, daemon_arch) = match arch.as_str() {
            "arm64" => (Architecture::Arm64, "aarch64"),
            "amd64" => (Architecture::X86_64, "x86_64"),
            _ => bail!("architecture must be arm64 or amd64"),
        };
        let relay = env::var("RUSTACK_RUNTIME_IMAGE").context("relay fixture image required")?;
        let port = self.docker.clone();
        let info = port.info(deadline()).await?;
        ensure!(
            info.os_type.as_deref() == Some("linux")
                && info.architecture.as_deref() == Some(daemon_arch),
            "daemon platform mismatch"
        );
        let relay = self.image(&relay, &arch).await?;
        let function = self
            .image(&format!("rustack-p0-handler:{arch}"), &arch)
            .await?;
        let revision = ExecutionRevision::builder()
            .resource_id(FunctionResourceId::new())
            .revision_id(Uuid::new_v4().to_string())
            .backend(ExecutorBackend::Docker)
            .function_name("wire-fixture".into())
            .qualifier("$LATEST".into())
            .runtime(None)
            .handler(None)
            .architecture(function.resolved().metadata().architecture)
            .artifact(DeploymentArtifact::Image(function))
            .image_config(overrides)
            .environment(HashMap::from([
                ("AWS_REGION".into(), "us-east-1".into()),
                ("AWS_DEFAULT_REGION".into(), "us-east-1".into()),
                ("TEST_VALUE".into(), "from-captured-revision".into()),
            ]))
            .timeout(Duration::from_secs(120))
            .memory_mb(192)
            .ephemeral_storage_mb(512)
            .build();
        let plan = EnvironmentPlan::new(
            self.lease.clone(),
            relay,
            Arc::new(revision),
            ContainerPolicy::new(
                "none".parse()?,
                1000,
                1000,
                1_000_000_000,
                RelayBudgets::new(IO, Duration::from_secs(600), IO)?,
            )?,
        )?;
        self.names.relay = plan.relay_key().clone();
        self.names.function = plan.function_key().clone();
        self.names.environment = plan.environment_id();
        self.plan = Some(plan);
        self.plan
            .as_ref()
            .context("allocation plan")?
            .create_relay(&port, deadline())
            .await?;
        self.attach(&port, architecture).await?;
        port.start_container(&self.names.relay, deadline()).await?;
        let client = &mut self.runtime.as_mut().context("runtime owner")?.client;
        ensure!(
            client.next(deadline()).await? == ClientEvent::RelayReady,
            "relay handshake mismatch"
        );
        self.plan
            .as_ref()
            .context("allocation plan")?
            .create_function(&port, deadline())
            .await?;
        self.logs = Some(FunctionLogs::start(
            port.attach_logs(&self.names.function, deadline()).await?,
            IO,
            262_144,
        )?);
        port.start_container(&self.names.function, deadline())
            .await?;
        if bootstrap {
            ensure!(
                client.next(deadline()).await? == ClientEvent::BootstrapReady,
                "function readiness mismatch"
            );
        }
        Ok(())
    }

    async fn attach(&mut self, port: &DockerPort, architecture: Architecture) -> Result<()> {
        let attached = port.attach(&self.names.relay, deadline()).await?;
        let (mut output, input) = attached.split(IO)?;
        let (mut bridge, reader) = tokio::io::duplex(64 * 1024);
        let client = ControlClient::start(
            ClientConfig::new(self.names.environment, architecture, IO)?,
            reader,
            input,
        )?;
        let (stop, mut stopped) = watch::channel(false);
        let pump = tokio::spawn(async move {
            loop {
                let chunk = tokio::select! {
                    biased;
                    () = async { let _ = stopped.wait_for(|stop| *stop).await; } => return Ok(()),
                    chunk = output.next(Instant::now() + Duration::from_secs(600)) => chunk?,
                };
                match chunk {
                    Some(chunk) if chunk.channel == OutputChannel::Stdout => {
                        timeout_at(deadline(), bridge.write_all(&chunk.bytes))
                            .await
                            .map_err(|_| DockerError::Deadline)??;
                    }
                    Some(_) => {} // Relay diagnostics are deliberately not copied into logs.
                    None => return Ok(()),
                }
            }
        });
        self.runtime = Some(Runtime {
            client,
            stop,
            pump: Some(pump),
            settlement: None,
        });
        Ok(())
    }

    async fn call(&mut self, event: &Value, failed: bool) -> Result<Value> {
        let call = Invocation::new(
            AttemptId::default(),
            Instant::now() + Duration::from_secs(120),
            "arn:aws:lambda:us-east-1:000000000000:function:wire-fixture:prod".parse()?,
            local_trace_id()?,
            Payload::try_from(Bytes::from(serde_json::to_vec(event)?))?,
        )?;
        let id = call.parts().0;
        let client = &mut self.runtime.as_mut().context("runtime owner")?.client;
        client.invoke(call, deadline()).await?;
        match client
            .next(Instant::now() + Duration::from_secs(120))
            .await?
        {
            ClientEvent::InvocationResult { attempt, result } if attempt == id => match result {
                RuntimeResult::Success(payload) if !failed => {
                    Ok(serde_json::from_slice(payload.as_ref())?)
                }
                RuntimeResult::FunctionError(payload) if failed => {
                    Ok(serde_json::from_slice(payload.as_ref())?)
                }
                _ => bail!("unexpected function outcome"),
            },
            _ => bail!("unexpected control outcome"),
        }
    }

    async fn cleanup(&mut self) -> Result<()> {
        let port = self.docker.clone();
        let function = remove(&port, &self.names.function).await;
        let logs = if let Some(logs) = &mut self.logs {
            logs.shutdown().await.map(drop).map_err(anyhow::Error::from)
        } else {
            Ok(())
        };
        let runtime = if let Some(runtime) = &mut self.runtime {
            runtime.shutdown().await
        } else {
            Ok(())
        };
        let relay = remove(&port, &self.names.relay).await;
        let remaining = port
            .list_containers(
                OwnerFilter::new(self.names.owner, Some(self.names.run), None)?,
                deadline(),
            )
            .await;
        let listed = remaining.map_err(anyhow::Error::from).and_then(|items| {
            ensure!(
                items.is_empty(),
                "owned containers remain after wire cleanup"
            );
            Ok(())
        });
        self.plan.take();
        self.artifacts.clear();
        let images = self
            .images
            .shutdown(deadline())
            .await
            .map_err(anyhow::Error::from);
        let joined = if images.is_ok() {
            self.wire.shutdown().await.map_err(anyhow::Error::from)
        } else {
            Ok(())
        };
        let result = function
            .and(logs)
            .and(runtime)
            .and(relay)
            .and(listed)
            .and(images)
            .and(joined);
        if result.is_err() {
            // Uncertain cleanup still owns the namespace; retain its files for
            // recovery rather than unlinking the live lock inode on test Drop.
            if let Some(directory) = self.directory.take() {
                let _retained = directory.keep();
            }
        }
        result
    }
}

async fn remove(port: &DockerPort, key: &DockerKey) -> Result<()> {
    match port.remove_container(key, deadline()).await {
        Ok(()) | Err(DockerError::Status(StatusCode::NOT_FOUND)) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[tokio::test]
#[ignore = "requires explicit Docker wire qualification images/socket"]
async fn test_should_execute_real_ric_over_production_wire_with_large_binary_frames_and_cleanup()
-> Result<()> {
    let mut environment = Environment::new().await?;
    let result = async {
        environment.start().await?;
        let event =
            json!({"value":35,"padding":"p".repeat(LARGE),"response_bytes":LARGE,"logs":true,"policy":true});
        let computed = environment.call(&event, false).await?;
        ensure!(
            computed["computed"] == 42
                && computed["counter"] == 1
                && computed["inputBytes"] == LARGE,
            "computed/large input mismatch"
        );
        ensure!(
                computed["large"].as_str().is_some_and(
                    |value| value.len() == LARGE && value.bytes().all(|byte| byte == b'r')
                ),
                "large output mismatch"
            );
        ensure!(
            computed["uid"] == 1000
                && computed["rootReadOnly"] == true
                && computed["memoryMax"] == "201326592"
                && computed["pidsMax"] == "256",
            "function resource controls differ"
        );
        ensure!(
            computed["tmpBytes"] == 512 * 1024 * 1024_u64
                && computed["tmpMode"] == 0o1777
                && computed["swapMax"] == "0"
                && computed["capEff"] == "0000000000000000"
                && computed["noNewPrivs"] == "1"
                && computed["taskRoot"] == "/var/task"
                && computed["functionName"] == "wire-fixture"
                && computed["functionVersion"] == "$LATEST"
                && computed["region"] == "us-east-1"
                && computed["configuredValue"] == "from-captured-revision",
            "allocation policy runtime enforcement differs"
        );
        let cpu = computed["cpuMax"].as_str().context("CPU quota report")?;
        let cpu: Vec<_> = cpu.split_whitespace().collect();
        ensure!(matches!(cpu.as_slice(), [quota, period] if quota.parse::<u64>().is_ok_and(|quota| quota > 0 && period.parse::<u64>() == Ok(quota))), "CPU quota does not enforce one CPU");
        let warm = environment.call(&json!({"value":0}), false).await?;
        ensure!(
            warm["computed"] == 7 && warm["counter"] == 2,
            "warm reuse mismatch"
        );
        let failure = environment.call(&json!({"fail":true}), true).await?;
        ensure!(
            failure["errorType"] == "ValueError",
            "function error mismatch"
        );
        let logs = environment.logs.as_ref().context("function log capture")?;
        timeout_at(deadline(), async {
            loop {
                let snapshot = logs.snapshot(deadline()).await?;
                ensure!(snapshot.failure.is_none(), "log capture failed");
                if snapshot
                    .tail
                    .windows(19)
                    .any(|part| part == b"WIRE_STDOUT_CAPTURE")
                    && snapshot
                        .tail
                        .windows(19)
                        .any(|part| part == b"WIRE_STDERR_CAPTURE")
                {
                    break Ok::<(), anyhow::Error>(());
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .context("function log receipt deadline")??;
        Ok(())
    }
    .await;
    let cleanup = environment.cleanup().await;
    result.and(cleanup)
}

#[tokio::test]
#[ignore = "requires explicit Docker wire qualification images/socket"]
async fn test_should_preserve_empty_overrides_in_actual_docker_create_merging() -> Result<()> {
    for reset in [false, true] {
        let mut environment = Environment::new().await?;
        let result = async {
            let overrides = ImageConfig {
                entry_point: reset.then(Vec::new),
                command: Some(if reset {
                    vec!["/bin/true".into()]
                } else {
                    vec![]
                }),
                working_directory: Some("/tmp".into()),
            };
            environment
                .start_with(Some(overrides.clone()), false)
                .await?;
            let port = environment.docker.clone();
            let expected = environment
                .artifacts
                .last()
                .context("function artifact")?
                .resolved()
                .metadata()
                .execution
                .plan(Some(&overrides))?;
            let actual = port
                .inspect_container(&environment.names.function, deadline())
                .await?
                .config
                .context("actual create config")?;
            ensure!(
                actual.working_dir.as_deref() == Some("/tmp"),
                "working directory override differs"
            );
            if reset {
                ensure!(
                    actual.entrypoint.as_deref().unwrap_or_default().is_empty(),
                    "daemon restored a cleared entrypoint"
                );
                ensure!(
                    actual
                        .cmd
                        .as_deref()
                        .is_some_and(|command| command == ["/bin/true"]),
                    "reset command mismatch"
                );
            } else {
                ensure!(
                    actual
                        .entrypoint
                        .as_deref()
                        .is_some_and(|entrypoint| entrypoint == expected.entrypoint()),
                    "inherited entrypoint mismatch"
                );
                ensure!(
                    actual.cmd.as_deref().unwrap_or_default().is_empty(),
                    "daemon restored a cleared command"
                );
            }
            let exit = port
                .wait_container(&environment.names.function, deadline())
                .await?;
            ensure!(
                exit.status_code == if reset { 0 } else { 142 },
                "empty override execution exit differs"
            );
            Ok(())
        }
        .await;
        let cleanup = environment.cleanup().await;
        result.and(cleanup)?;
    }
    Ok(())
}
