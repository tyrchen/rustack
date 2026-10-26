//! Explicit local Docker qualification of production HTTP/upgrade/control IO.

#![cfg(unix)]
#![forbid(unsafe_code)]

use std::{collections::HashMap, env, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use bytes::Bytes;
use http::StatusCode;
use rustack_lambda_core::{
    artifact::{Architecture as ImageArchitecture, ImageReference, PinnedImage},
    docker::{
        DirectoryLease, DirectoryLeaseError, DockerError, DockerKey, DockerPort, DockerSocket,
        DockerWire, ImageLeaseConfig, ImageLeaseManager, ImagePinScope, OutputChannel, OwnerFilter,
        models::{ContainerCreateBody, HostConfig},
    },
};
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
impl Names {
    fn labels(&self) -> HashMap<String, String> {
        HashMap::from([
            ("rustack.lambda.ownerId".into(), self.owner.to_string()),
            ("rustack.lambda.runId".into(), self.run.to_string()),
            (
                "rustack.lambda.environmentId".into(),
                self.environment.to_string(),
            ),
            (
                "rustack.lambda.functionResourceId".into(),
                self.environment.to_string(),
            ),
        ])
    }
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
    names: Names,
    runtime: Option<Runtime>,
    images: ImageLeaseManager,
    artifacts: Vec<PinnedImage>,
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
        let images = ImageLeaseManager::start(
            wire.port(),
            ImagePinScope::new(lease),
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
            names: Names {
                relay: format!("rustack-wire-relay-{run}").parse()?,
                function: format!("rustack-wire-function-{run}").parse()?,
                owner,
                run,
                environment: EnvironmentId::default(),
            },
            runtime: None,
            images,
            artifacts: Vec::new(),
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
        let image = self
            .images
            .port()
            .prepare(&reference, architecture, None, deadline())
            .await?;
        self.artifacts.push(image.clone());
        Ok(image)
    }

    async fn start(&mut self) -> Result<()> {
        let arch = env::var("RUSTACK_RUNTIME_ARCH").context("explicit architecture required")?;
        let (architecture, daemon_arch) = match arch.as_str() {
            "arm64" => (Architecture::Arm64, "aarch64"),
            "amd64" => (Architecture::X86_64, "x86_64"),
            _ => bail!("architecture must be arm64 or amd64"),
        };
        let relay = env::var("RUSTACK_RUNTIME_IMAGE").context("relay fixture image required")?;
        let port = self.wire.port();
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
        let relay_plan = relay.resolved().metadata().execution.plan(None)?;
        let function_plan = function.resolved().metadata().execution.plan(None)?;
        port.create_container_on_platform(
            &self.names.relay,
            &ContainerCreateBody {
                image: Some(relay.resolved().metadata().image_id.as_str().into()),
                entrypoint: Some(relay_plan.entrypoint().to_vec()),
                cmd: Some(relay_plan.command().to_vec()),
                working_dir: Some(relay_plan.working_directory().into()),
                user: Some("1000:1000".into()),
                env: Some(vec![
                    format!("RUSTACK_LAMBDA_ENVIRONMENT_ID={}", self.names.environment),
                    "RUSTACK_LAMBDA_RELAY_IO_SECONDS=60".into(),
                    "RUSTACK_LAMBDA_RELAY_RETIRE_SECONDS=60".into(),
                ]),
                labels: Some(self.names.labels()),
                attach_stdin: Some(true),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                open_stdin: Some(true),
                stdin_once: Some(true),
                tty: Some(false),
                host_config: Some(limits("none".into(), 64 * 1024 * 1024, 100_000_000, 64)),
                ..Default::default()
            },
            relay.resolved().metadata().architecture,
            deadline(),
        )
        .await?;
        self.attach(&port, architecture).await?;
        port.start_container(&self.names.relay, deadline()).await?;
        let client = &mut self.runtime.as_mut().context("runtime owner")?.client;
        ensure!(
            client.next(deadline()).await? == ClientEvent::RelayReady,
            "relay handshake mismatch"
        );
        port.create_container_on_platform(
            &self.names.function,
            &ContainerCreateBody {
                image: Some(function.resolved().metadata().image_id.as_str().into()),
                entrypoint: Some(function_plan.entrypoint().to_vec()),
                cmd: Some(function_plan.command().to_vec()),
                working_dir: Some(function_plan.working_directory().into()),
                user: Some("1000:1000".into()),
                labels: Some(self.names.labels()),
                env: Some(vec![
                    "AWS_LAMBDA_RUNTIME_API=127.0.0.1:9001".into(),
                    "AWS_LAMBDA_FUNCTION_NAME=wire-fixture".into(),
                    "AWS_LAMBDA_FUNCTION_VERSION=$LATEST".into(),
                    "AWS_LAMBDA_FUNCTION_MEMORY_SIZE=192".into(),
                    "AWS_REGION=us-east-1".into(),
                    "AWS_DEFAULT_REGION=us-east-1".into(),
                ]),
                host_config: Some(limits(
                    format!("container:{}", self.names.relay.as_str()),
                    192 * 1024 * 1024,
                    1_000_000_000,
                    256,
                )),
                ..Default::default()
            },
            function.resolved().metadata().architecture,
            deadline(),
        )
        .await?;
        port.start_container(&self.names.function, deadline())
            .await?;
        ensure!(
            client.next(deadline()).await? == ClientEvent::BootstrapReady,
            "function readiness mismatch"
        );
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
        let port = self.wire.port();
        let function = remove(&port, &self.names.function).await;
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

fn limits(network_mode: String, memory: i64, nano_cpus: i64, pids_limit: i64) -> HostConfig {
    HostConfig {
        network_mode: Some(network_mode),
        memory: Some(memory),
        nano_cpus: Some(nano_cpus),
        pids_limit: Some(pids_limit),
        readonly_rootfs: Some(true),
        cap_drop: Some(vec!["ALL".into()]),
        security_opt: Some(vec!["no-new-privileges:true".into()]),
        tmpfs: Some(HashMap::from([(
            "/tmp".into(),
            "rw,noexec,nosuid,size=536870912".into(),
        )])),
        ..Default::default()
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
        let event = json!({"value":35,"padding":"p".repeat(LARGE),"response_bytes":LARGE});
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
        Ok(())
    }
    .await;
    let cleanup = environment.cleanup().await;
    result.and(cleanup)
}

#[tokio::test]
#[ignore = "requires explicit Docker wire qualification images/socket"]
async fn test_should_preserve_empty_overrides_in_actual_docker_create_merging() -> Result<()> {
    let mut environment = Environment::new().await?;
    let result = async {
        let arch = env::var("RUSTACK_RUNTIME_ARCH").context("explicit architecture required")?;
        let port = environment.wire.port();
        let image = environment
            .image(&format!("rustack-p0-handler:{arch}"), &arch)
            .await?;
        for reset in [false, true] {
            let overrides = rustack_lambda_model::types::ImageConfig {
                entry_point: reset.then(Vec::new),
                command: Some(if reset {
                    vec!["/bin/true".into()]
                } else {
                    vec![]
                }),
                ..Default::default()
            };
            let plan = image
                .resolved()
                .metadata()
                .execution
                .plan(Some(&overrides))?;
            port.create_container_on_platform(
                &environment.names.function,
                &ContainerCreateBody {
                    image: Some(image.resolved().metadata().image_id.as_str().into()),
                    entrypoint: Some(plan.entrypoint().to_vec()),
                    cmd: Some(plan.command().to_vec()),
                    working_dir: Some(plan.working_directory().into()),
                    labels: Some(environment.names.labels()),
                    user: Some("1000:1000".into()),
                    host_config: Some(limits("none".into(), 192 * 1024 * 1024, 1_000_000_000, 256)),
                    ..Default::default()
                },
                image.resolved().metadata().architecture,
                deadline(),
            )
            .await?;
            let actual = port
                .inspect_container(&environment.names.function, deadline())
                .await?
                .config
                .context("actual create config")?;
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
                        .is_some_and(|entrypoint| entrypoint == plan.entrypoint()),
                    "inherited entrypoint mismatch"
                );
                ensure!(
                    actual.cmd.as_deref().unwrap_or_default().is_empty(),
                    "daemon restored a cleared command"
                );
            }
            remove(&port, &environment.names.function).await?;
        }
        Ok(())
    }
    .await;
    let cleanup = environment.cleanup().await;
    result.and(cleanup)
}
