//! Shared explicit-daemon fixture. Readiness/probes are not implied by setup.
use std::{env, time::Duration};

use anyhow::{Context, Result, ensure};
use rustack_lambda_core::{
    artifact::{Architecture, PinnedImage},
    docker::{
        DirectoryLease, DockerPort, DockerWire, EnvironmentConfig, EnvironmentManager,
        EnvironmentSettings, ImageLeaseConfig, ImageLeaseManager, ImagePinScope, OwnerFilter,
    },
};
use tempfile::TempDir;
use tokio::time::Instant;
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(60)
}
pub(crate) struct Fixture {
    pub(crate) owner: EnvironmentManager,
    pub(crate) images: ImageLeaseManager,
    pub(crate) wire: DockerWire,
    pub(crate) docker: DockerPort,
    pub(crate) lease: DirectoryLease,
    pub(crate) relay: Option<PinnedImage>,
    pub(crate) function: Option<PinnedImage>,
    pub(crate) directory: Option<TempDir>,
}
impl Fixture {
    pub(crate) async fn new() -> Result<Self> {
        let socket =
            env::var("RUSTACK_RUNTIME_DOCKER_SOCKET").context("explicit Docker socket required")?;
        let wire = DockerWire::connect(format!("unix://{socket}").parse()?, deadline()).await?;
        let directory = tempfile::tempdir()?;
        let lease = DirectoryLease::acquire(directory.path().into(), deadline()).await?;
        let docker = wire.port().bind_directory(&lease, deadline()).await?;
        let images = ImageLeaseManager::start(
            docker.clone(),
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
        let owner = EnvironmentManager::start(docker.clone(), lease.clone(), config)?;
        let mut fixture = Self {
            owner,
            images,
            wire,
            docker,
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

    pub(crate) async fn cleanup(&mut self) -> Result<()> {
        let result = async {
            self.owner.shutdown(deadline()).await?;
            ensure!(
                self.owner.port().status().allocated == 0,
                "allocation reservation remains"
            );
            let containers = self
                .docker
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
