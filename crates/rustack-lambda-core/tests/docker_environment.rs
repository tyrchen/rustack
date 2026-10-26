//! Real container lifecycle qualification of the retained environment owner.
#![cfg(unix)]
#![forbid(unsafe_code)]
use std::{collections::HashMap, env, sync::Arc, time::Duration};

use anyhow::{Context, Result, ensure};
use bytes::Bytes;
use rustack_lambda_core::{
    artifact::{Architecture, DeploymentArtifact, PinnedImage},
    docker::{
        ContainerPolicy, DirectoryLease, DockerWire, EnvironmentConfig, EnvironmentError,
        EnvironmentHandle, EnvironmentManager, EnvironmentPlan, EnvironmentSettings, FunctionLogs,
        ImageLeaseConfig, ImageLeaseManager, ImagePinScope, OwnerFilter, RelayBudgets,
    },
    executor::{ExecutionRevision, ExecutorBackend},
    identity::FunctionResourceId,
};
use rustack_lambda_runtime::{
    metadata::local_trace_id,
    protocol::{AttemptId, Payload},
    session::{Invocation, RuntimeResult},
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::time::{Instant, timeout};
use uuid::Uuid;
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(60)
}
struct Fixture {
    owner: EnvironmentManager,
    images: ImageLeaseManager,
    wire: DockerWire,
    lease: DirectoryLease,
    relay: Option<PinnedImage>,
    function: Option<PinnedImage>,
    directory: Option<TempDir>,
}
impl Fixture {
    async fn new() -> Result<Self> {
        let socket =
            env::var("RUSTACK_RUNTIME_DOCKER_SOCKET").context("explicit Docker socket required")?;
        let wire = DockerWire::connect(format!("unix://{socket}").parse()?, deadline()).await?;
        let directory = tempfile::tempdir()?;
        let lease = DirectoryLease::acquire(directory.path().into(), deadline()).await?;
        let images = ImageLeaseManager::start(
            wire.port(),
            ImagePinScope::new(lease.clone()),
            ImageLeaseConfig::default(),
        )?;
        let arch = match env::var("RUSTACK_RUNTIME_ARCH")?.as_str() {
            "arm64" => Architecture::Arm64,
            "amd64" => Architecture::X86_64,
            _ => anyhow::bail!("unknown architecture"),
        };
        let config = EnvironmentConfig::try_from(
            EnvironmentSettings::builder()
                .environments(1)
                .memory_bytes(256 * 1024 * 1024)
                .io(Duration::from_secs(60))
                .start(Duration::from_secs(60))
                .build(),
        )?;
        let owner = EnvironmentManager::start(wire.port(), lease.clone(), config)?;
        let mut fixture = Self {
            owner,
            images,
            wire,
            lease,
            relay: None,
            function: None,
            directory: Some(directory),
        };
        if let Err(error) = fixture.prepare(arch).await {
            let cleanup = fixture.cleanup().await;
            return Err(error.context(format!(
                "setup cleanup: {}",
                if cleanup.is_ok() {
                    "confirmed"
                } else {
                    "unresolved"
                }
            )));
        }
        Ok(fixture)
    }
    async fn prepare(&mut self, arch: Architecture) -> Result<()> {
        let relay_reference = env::var("RUSTACK_RUNTIME_IMAGE")?.parse()?;
        let relay = self
            .images
            .port()
            .prepare(&relay_reference, arch, None, deadline())
            .await?;
        self.relay = Some(relay);
        let suffix = if arch == Architecture::Arm64 {
            "arm64"
        } else {
            "amd64"
        };
        let function = self
            .images
            .port()
            .prepare(
                &format!("rustack-p0-handler:{suffix}").parse()?,
                arch,
                None,
                deadline(),
            )
            .await?;
        self.function = Some(function);
        Ok(())
    }

    fn plan(&self) -> Result<EnvironmentPlan> {
        let revision = ExecutionRevision::builder()
            .resource_id(FunctionResourceId::new())
            .revision_id(Uuid::new_v4().to_string())
            .backend(ExecutorBackend::Docker)
            .function_name("environment-fixture".into())
            .qualifier("$LATEST".into())
            .runtime(None)
            .handler(None)
            .architecture(
                self.function
                    .as_ref()
                    .context("function image")?
                    .resolved()
                    .metadata()
                    .architecture,
            )
            .artifact(DeploymentArtifact::Image(
                self.function.as_ref().context("function image")?.clone(),
            ))
            .image_config(None)
            .environment(HashMap::from([
                ("AWS_REGION".into(), "us-east-1".into()),
                ("AWS_DEFAULT_REGION".into(), "us-east-1".into()),
            ]))
            .timeout(Duration::from_secs(120))
            .memory_mb(192)
            .ephemeral_storage_mb(512)
            .build();
        Ok(EnvironmentPlan::new(
            self.lease.clone(),
            self.relay.as_ref().context("relay image")?.clone(),
            Arc::new(revision),
            ContainerPolicy::new(
                "none".parse()?,
                1000,
                1000,
                1_000_000_000,
                RelayBudgets::new(
                    Duration::from_secs(60),
                    Duration::from_secs(600),
                    Duration::from_secs(60),
                )?,
            )?,
        )?)
    }
    async fn cleanup(&mut self) -> Result<()> {
        let result = async {
            self.owner.shutdown(deadline()).await?;
            ensure!(
                self.owner.port().status().allocated == 0,
                "allocation reservation remains"
            );
            let containers = self
                .wire
                .port()
                .list_containers(
                    OwnerFilter::new(self.lease.owner_id(), Some(self.lease.run_id()), None)?,
                    deadline(),
                )
                .await?;
            ensure!(containers.is_empty(), "owned containers remain");
            self.relay.take();
            self.function.take();
            self.images.shutdown(deadline()).await?;
            self.wire.shutdown().await?;
            Ok(())
        }
        .await;
        if result.is_err() {
            if let Some(directory) = self.directory.take() {
                let _ = directory.keep();
            }
        }
        result
    }
}
fn invocation(event: &Value) -> Result<Invocation> {
    Ok(Invocation::new(
        AttemptId::default(),
        Instant::now() + Duration::from_secs(120),
        "arn:aws:lambda:us-east-1:000000000000:function:environment-fixture:prod".parse()?,
        local_trace_id()?,
        Payload::try_from(Bytes::from(serde_json::to_vec(event)?))?,
    )?)
}
async fn call(handle: &mut EnvironmentHandle, event: &Value, failed: bool) -> Result<Value> {
    let response = handle.invoke(invocation(event)?, true).await?;
    match response.result {
        RuntimeResult::Success(payload) if !failed => Ok(serde_json::from_slice(payload.as_ref())?),
        RuntimeResult::FunctionError(payload) if failed => {
            Ok(serde_json::from_slice(payload.as_ref())?)
        }
        _ => anyhow::bail!("unexpected handler result"),
    }
}

#[tokio::test]
#[ignore = "requires explicit Docker environment images/socket"]
async fn test_should_own_real_initialization_warm_calls_function_error_and_cleanup() -> Result<()> {
    let mut fixture = Fixture::new().await?;
    let result=async{
        let mut handle=fixture.owner.port().spawn(fixture.plan()?,deadline())?;handle.wait_ready(deadline()).await?;
        ensure!(matches!(fixture.owner.port().spawn(fixture.plan()?,deadline()),Err(EnvironmentError::Busy)),"global environment bound not enforced");
        let first=call(&mut handle,&json!({"value":35,"padding":"x".repeat(6*1024*1024-4096),"response_bytes":6*1024*1024-4096}),false).await?;
        ensure!(first["computed"]==42&&first["counter"]==1,"computed result differs");ensure!(first["large"].as_str().is_some_and(|value|value.len()==6*1024*1024-4096),"large result differs");
        let second=call(&mut handle,&json!({"value":0}),false).await?;ensure!(second["computed"]==7&&second["counter"]==2,"warm result differs");
        let failure=call(&mut handle,&json!({"fail":true}),true).await?;ensure!(failure["errorType"]=="ValueError","FunctionError differs");handle.shutdown(deadline()).await?;Ok(())
    }.await;
    let cleanup = fixture.cleanup().await;
    result.and(cleanup)
}
#[tokio::test]
#[ignore = "requires explicit Docker environment images/socket"]
async fn test_should_retire_running_code_after_abandoned_call_and_join_all_owned_io() -> Result<()>
{
    let mut fixture = Fixture::new().await?;
    let result = async {
        let plan = fixture.plan()?;
        let key = plan.function_key().clone();
        let mut handle = fixture.owner.port().spawn(plan, deadline())?;
        handle.wait_ready(deadline()).await?;
        let mut observed = FunctionLogs::start(
            fixture.wire.port().attach_logs(&key, deadline()).await?,
            Duration::from_secs(60),
            262_144,
        )?;
        let call = tokio::spawn(async move {
            handle
                .invoke(invocation(&json!({"value":1,"sleep":true})).unwrap(), false)
                .await
        });
        let observation = timeout(Duration::from_secs(30), async {
            loop {
                let snapshot = observed.snapshot(deadline()).await?;
                if snapshot
                    .tail
                    .windows(14)
                    .any(|part| part == b"P0_BUSY_START=")
                {
                    break Ok::<(), anyhow::Error>(());
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .context("handler did not start");
        call.abort();
        let joined = call.await;
        let retired = fixture.owner.shutdown(deadline()).await;
        let logs = observed.shutdown().await;
        observation??;
        ensure!(
            matches!(joined,Err(ref error) if error.is_cancelled()),
            "abandoned call was not cancelled"
        );
        retired?;
        logs?;
        Ok(())
    }
    .await;
    let cleanup = fixture.cleanup().await;
    result.and(cleanup)
}
