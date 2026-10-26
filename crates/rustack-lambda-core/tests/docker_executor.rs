//! Real SDK/HTTP/provider execution; controlled fixture readiness is not startup evidence.
#![cfg(unix)]
#![forbid(unsafe_code)]

use std::{path::Path, sync::Arc, time::Duration};

use anyhow::{Context, Result, ensure};
use hyper::server::conn::http1;
use hyper_util::rt::TokioIo;
use rustack_lambda_core::{
    config::LambdaConfig,
    deployment::ImageDeploymentConfig,
    docker::{ContainerPolicy, PoolConfig, RelayBudgets},
    executor::{
        DockerExecutor, DockerExecutorConfig, DockerExecutorError, DockerExecutorSettings,
        DockerReadiness, Executor, ExecutorBackend,
    },
    handler::RustackLambdaHandler,
    provider::RustackLambda,
    storage::FunctionStore,
};
use rustack_lambda_http::{LambdaHttpConfig, LambdaHttpService};
use tokio::{net::TcpListener, process::Command, sync::watch, task::JoinSet, time::timeout};

mod common;
use common::docker::Fixture;

#[derive(Debug)]
struct CompositionOnlyReadiness;
impl DockerReadiness for CompositionOnlyReadiness {
    fn check(&self) -> Result<(), DockerExecutorError> {
        Ok(())
    }
}

#[tokio::test]
#[ignore = "requires explicit Docker images/socket and pinned boto3 fixture"]
async fn test_should_deploy_publish_alias_and_invoke_real_images_through_boto3_http() -> Result<()>
{
    let mut fixture = Fixture::new().await?;
    let result = run(&fixture).await;
    let cleanup = fixture.cleanup().await;
    result.and(cleanup)
}
async fn run(fixture: &Fixture) -> Result<()> {
    let relay = fixture.relay.as_ref().context("relay image")?.clone();
    let executor = Arc::new(DockerExecutor::new(
        DockerExecutorConfig::builder()
            .directory(fixture.lease.clone())
            .relay(relay)
            .policy(ContainerPolicy::new(
                "none".parse()?,
                1000,
                1000,
                1_000_000_000,
                RelayBudgets::new(
                    Duration::from_secs(60),
                    Duration::from_secs(600),
                    Duration::from_secs(60),
                )?,
            )?)
            .environments(fixture.owner.port())
            .readiness(Arc::new(CompositionOnlyReadiness))
            .settings(DockerExecutorSettings::new(
                PoolConfig::default(),
                Duration::from_secs(60),
            )?)
            .build(),
    )?);
    let result = serve(fixture, executor.clone()).await;
    let retired = executor.shutdown().await;
    result.and(retired.map_err(Into::into))
}
async fn serve(fixture: &Fixture, executor: Arc<DockerExecutor>) -> Result<()> {
    let store = FunctionStore::new(
        fixture
            .directory
            .as_ref()
            .context("data directory")?
            .path()
            .join("code"),
    );
    let config = LambdaConfig {
        executor: ExecutorBackend::Docker,
        ..Default::default()
    };
    let provider = Arc::new(
        RustackLambda::with_executor(store, config, executor.clone())
            .with_image_preparation(fixture.images.port(), ImageDeploymentConfig::default())?,
    );
    let result = run_server(fixture, provider.clone()).await;
    let quiesced = provider.quiesce(Duration::from_secs(60)).await;
    let retired = executor.shutdown().await;
    provider.shutdown().await;
    result
        .and(quiesced.map_err(Into::into))
        .and(retired.map_err(Into::into))
}
async fn run_server(fixture: &Fixture, provider: Arc<RustackLambda>) -> Result<()> {
    let image = fixture
        .function
        .as_ref()
        .context("function image")?
        .resolved()
        .metadata();
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!("http://{}", listener.local_addr()?);
    let service = LambdaHttpService::new(
        Arc::new(RustackLambdaHandler::new(provider)),
        LambdaHttpConfig::default(),
    );
    let (stop, mut stopped) = watch::channel(false);
    let mut server = tokio::spawn(async move {
        let mut jobs = JoinSet::new();
        let result = async { loop {
            tokio::select! {biased;
                _=stopped.wait_for(|stop|*stop)=>return Ok::<(),anyhow::Error>(()),
                joined=jobs.join_next(),if !jobs.is_empty()=> {joined.unwrap()??;},
                stream=listener.accept(),if jobs.len()<8=> {
                    let (stream,_)=stream?;let service=service.clone();
                    jobs.spawn(async move {http1::Builder::new().serve_connection(TokioIo::new(stream),service).await});
                }
            }
        } }.await;
        jobs.abort_all();
        let mut workers_ok = true;
        while let Some(joined) = jobs.join_next().await {
            if let Err(error) = joined {
                workers_ok &= error.is_cancelled();
            }
        }
        result?;
        ensure!(workers_ok, "HTTP worker failed");
        Ok::<(), anyhow::Error>(())
    });
    let result = run_sdk(
        &endpoint,
        image.requested_reference.as_str(),
        image.architecture.as_str(),
    )
    .await;
    stop.send_replace(true);
    let joined = timeout(Duration::from_secs(60), &mut server).await;
    let joined = match joined {
        Ok(result) => result.context("HTTP supervisor join")?,
        Err(error) => {
            server.abort();
            let _ = server.await;
            Err(error.into())
        }
    };
    result.and(joined)
}
async fn run_sdk(endpoint: &str, image: &str, architecture: &str) -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut child = Command::new("uv");
    child
        .args([
            "run",
            "--no-project",
            "--python",
            "3.13",
            "--with-requirements",
        ])
        .arg(root.join("tests/workflows/requirements.txt"))
        .arg("python")
        .arg(root.join("tests/workflows/docker_lambda_sdk.py"))
        .args([endpoint, image, architecture])
        .kill_on_drop(true);
    let mut process = child.spawn().context("start pinned SDK fixture")?;
    let result = timeout(Duration::from_secs(300), process.wait()).await;
    let status = match result {
        Ok(result) => result?,
        Err(error) => {
            let _ = process.kill().await;
            let _ = process.wait().await;
            return Err(error.into());
        }
    };
    ensure!(status.success(), "SDK fixture failed: {status}");
    Ok(())
}
