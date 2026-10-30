//! Dependency-ordered qualification and physical reverse-order retirement.

use std::{env::consts::ARCH, sync::Arc, time::Duration};

use tokio::time::{Instant, sleep, timeout_at};

use super::{DockerStartupError, Evidence, Qualification, State};
use crate::{
    artifact::{Architecture, PinnedImage},
    docker::{
        CodeVolumeConfig, CodeVolumeManager, DirectoryLease, DockerError, DockerExecutor,
        DockerExecutorConfig, DockerExecutorError, DockerExecutorSettings, DockerPort,
        DockerReadiness, DockerWire, DockerZipRuntime, EndpointProbeConfig, EnvironmentManager,
        EnvironmentProbeConfig, EnvironmentSettings, ImageLeaseConfig, ImageLeaseManager,
        ImageLeasePort, ImagePinScope, PoolSettings, ProbeSettings,
        code_volume::validate_base,
        ownership::DirectoryAcquisition,
        probe_endpoint, probe_environment, reconcile_namespace,
        recovery::inventory::{containers, images, volumes},
    },
    executor::{Executor, ExecutorError},
};

pub(super) fn deadline(at: Instant) -> Result<(), DockerStartupError> {
    let remaining = at
        .checked_duration_since(Instant::now())
        .ok_or(DockerStartupError::Deadline)?;
    if remaining.is_zero() {
        return Err(DockerStartupError::Deadline);
    }
    if remaining > Duration::from_secs(3600) {
        return Err(DockerStartupError::Configuration);
    }
    Ok(())
}
fn step(at: Instant, seconds: u64) -> Instant {
    at.min(Instant::now() + Duration::from_secs(seconds))
}

#[derive(Debug)]
struct PreparedImages {
    directory: DirectoryLease,
    docker: DockerPort,
    architecture: Architecture,
    images: ImageLeasePort,
    relay: PinnedImage,
}

impl State {
    async fn acquire_directory(
        &mut self,
        at: Instant,
    ) -> Result<DirectoryLease, DockerStartupError> {
        self.acquiring = Some(if self.config.settings.zip_image().is_some() {
            DirectoryAcquisition::begin_code(self.config.directory.clone(), step(at, 60))?
        } else {
            DirectoryAcquisition::begin(self.config.directory.clone(), step(at, 60))?
        });
        let acquired = self
            .acquiring
            .as_mut()
            .ok_or(DockerStartupError::Unavailable)?
            .wait(at)
            .await?;
        self.acquiring.take();
        self.directory = Some(acquired?);
        let directory = self
            .directory
            .as_ref()
            .ok_or(DockerStartupError::Unavailable)?
            .clone();
        directory.require_journaled()?;
        Ok(directory)
    }
    async fn prepare_images(&mut self, at: Instant) -> Result<PreparedImages, DockerStartupError> {
        let settings = self.config.settings.clone();
        let budgets = settings.budgets();
        let directory = self.acquire_directory(at).await?;
        self.connecting = Some(tokio::spawn(DockerWire::connect(
            settings.socket().clone(),
            step(at, budgets.lambda_docker_io_seconds),
        )));
        let connected = timeout_at(
            at,
            self.connecting
                .as_mut()
                .ok_or(DockerStartupError::Unavailable)?,
        )
        .await
        .map_err(|_| DockerStartupError::Deadline)?;
        self.connecting.take();
        self.wire = Some(connected.map_err(|_| DockerStartupError::Worker)??);
        let docker = self
            .wire
            .as_ref()
            .ok_or(DockerStartupError::Unavailable)?
            .port()
            .bind_directory(&directory, step(at, budgets.lambda_docker_io_seconds))
            .await?;
        self.bound = Some(docker.clone());
        let recovery = reconcile_namespace(&directory, &docker, step(at, 60)).await?;
        if recovery.containers.remaining != 0
            || recovery.pins.remaining != 0
            || recovery.code.remaining != 0
        {
            return Err(DockerError::Quarantined.into());
        }
        let info = docker
            .info(step(at, budgets.lambda_docker_io_seconds))
            .await?;
        let architecture = platform(
            info.os_type.as_deref(),
            info.architecture.as_deref(),
            settings.cross_architecture(),
        )?;
        let pull = settings.load_pull_policy(step(at, 60)).await?;
        self.producing = true;
        self.images = Some(ImageLeaseManager::start_qualified(
            docker.clone(),
            ImagePinScope::new(directory.clone()),
            ImageLeaseConfig::new(
                budgets
                    .lambda_docker_image_pins
                    .try_into()
                    .map_err(|_| DockerStartupError::Configuration)?,
                budgets
                    .lambda_docker_deployment_jobs
                    .try_into()
                    .map_err(|_| DockerStartupError::Configuration)?,
                Duration::from_secs(budgets.lambda_docker_io_seconds),
            )?,
            pull,
            architecture,
        )?);
        let images = self
            .images
            .as_ref()
            .ok_or(DockerStartupError::Unavailable)?
            .port();
        self.relay = Some(
            images
                .prepare(
                    settings.relay_image(),
                    architecture,
                    None,
                    step(at, budgets.lambda_docker_deployment_seconds),
                )
                .await
                .map_err(DockerStartupError::Relay)?,
        );
        let relay = self
            .relay
            .as_ref()
            .ok_or(DockerStartupError::Unavailable)?
            .clone();
        Ok(PreparedImages {
            directory,
            docker,
            architecture,
            images,
            relay,
        })
    }
    async fn prepare_zip(
        &mut self,
        prepared: &PreparedImages,
        at: Instant,
    ) -> Result<(), DockerStartupError> {
        let settings = self.config.settings.clone();
        let budgets = settings.budgets();
        if let Some(reference) = settings.zip_image() {
            let base = prepared
                .images
                .prepare(
                    reference,
                    prepared.architecture,
                    None,
                    step(at, budgets.lambda_docker_deployment_seconds),
                )
                .await?;
            validate_base(
                &base,
                &prepared.docker,
                step(at, budgets.lambda_docker_io_seconds),
            )
            .await?;
            let volumes: u8 = budgets
                .lambda_docker_environments
                .try_into()
                .map_err(|_| DockerStartupError::Configuration)?;
            self.code = Some(CodeVolumeManager::start(
                prepared.docker.clone(),
                prepared.directory.clone(),
                CodeVolumeConfig::new(
                    volumes,
                    volumes.min(2),
                    Duration::from_secs(budgets.lambda_docker_io_seconds),
                    Duration::from_secs(budgets.lambda_docker_code_upload_seconds),
                )?,
            )?);
            self.zip = Some(DockerZipRuntime::new(
                &prepared.directory,
                base,
                self.code
                    .as_ref()
                    .ok_or(DockerStartupError::Unavailable)?
                    .port(),
            )?);
        }
        Ok(())
    }
    async fn prepare_probe(
        &mut self,
        prepared: &PreparedImages,
        at: Instant,
    ) -> Result<EnvironmentProbeConfig, DockerStartupError> {
        let settings = self.config.settings.clone();
        let budgets = settings.budgets();
        let directory = prepared.directory.clone();
        let docker = prepared.docker.clone();
        let relay = prepared.relay.clone();
        let network = settings
            .container_policy()
            .network
            .qualify(&docker, step(at, budgets.lambda_docker_io_seconds))
            .await?;
        let policy = network.apply(settings.container_policy())?;
        self.environments = Some(EnvironmentManager::start(
            docker.clone(),
            directory.clone(),
            EnvironmentSettings::builder()
                .environments(
                    budgets
                        .lambda_docker_environments
                        .try_into()
                        .map_err(|_| DockerStartupError::Configuration)?,
                )
                .memory_bytes(budgets.lambda_docker_memory_bytes)
                .io(Duration::from_secs(budgets.lambda_docker_io_seconds))
                .start(Duration::from_secs(budgets.lambda_docker_start_seconds))
                .init(self.config.lambda.init_timeout)
                .idle(self.config.lambda.idle_timeout)
                .log_ring_bytes(settings.log_ring_bytes())
                .build()
                .try_into()?,
        )?);
        let environments = self
            .environments
            .as_ref()
            .ok_or(DockerStartupError::Unavailable)?
            .port();
        let base = EnvironmentProbeConfig::builder()
            .directory(directory.clone())
            .relay(relay.clone())
            .environments(environments.clone())
            .policy(policy.clone())
            .network(network)
            .settings(ProbeSettings::new(
                Duration::from_secs(budgets.lambda_docker_start_seconds),
                Duration::from_secs(budgets.lambda_docker_cleanup_seconds),
            )?)
            .build();
        Ok(base)
    }
    pub(super) async fn qualify(
        &mut self,
        qualification: &Arc<Qualification>,
        at: Instant,
    ) -> Result<(), DockerStartupError> {
        let prepared = self.prepare_images(at).await?;
        self.prepare_zip(&prepared, at).await?;
        let base = self.prepare_probe(&prepared, at).await?;
        let settings = self.config.settings.clone();
        let budgets = settings.budgets();
        let resource = probe_environment(base.clone(), at).await?;
        let endpoint = if let Some(endpoint) = settings.gateway_endpoint() {
            let config = EndpointProbeConfig::new(
                base.clone(),
                resource.clone(),
                endpoint.clone(),
                self.config.instance,
            )
            .map_err(DockerStartupError::Endpoint)?;
            let proof = probe_endpoint(config.clone(), at)
                .await
                .map_err(DockerStartupError::Endpoint)?;
            Some((config, proof))
        } else {
            None
        };
        deadline(at)?;
        let evidence = Evidence::builder()
            .base(base.clone())
            .resource(resource)
            .endpoint(endpoint)
            .images(prepared.images)
            .docker(prepared.docker)
            .architecture(prepared.architecture)
            .zip(self.zip.clone())
            .build();
        qualification
            .evidence
            .set(evidence)
            .map_err(|_| DockerStartupError::Unavailable)?;
        // Installing the exact proof is separate from constructing dispatch. The
        // final live/clock fence still precedes any installation publication.
        qualification
            .check()
            .map_err(|_| DockerStartupError::Unavailable)?;
        self.executor = Some(Arc::new(DockerExecutor::new(
            DockerExecutorConfig::builder()
                .directory(base.directory)
                .relay(base.relay)
                .policy(base.policy)
                .environments(base.environments)
                .readiness(qualification.clone())
                .zip(self.zip.clone())
                .settings(DockerExecutorSettings::new(
                    PoolSettings::builder()
                        .warm(
                            budgets
                                .lambda_docker_environments
                                .try_into()
                                .map_err(|_| DockerStartupError::Configuration)?,
                        )
                        .warm_per_revision(
                            self.config
                                .lambda
                                .max_warm_instances
                                .try_into()
                                .map_err(|_| DockerStartupError::Configuration)?,
                        )
                        .build()
                        .try_into()?,
                    Duration::from_secs(budgets.lambda_docker_cleanup_seconds),
                )?)
                .build(),
        )?));
        deadline(at)?;
        qualification
            .check()
            .map_err(|_| DockerStartupError::Unavailable)
    }
    pub(super) async fn cleanup(mut self) -> Result<(), ExecutorError> {
        let mut delay = Duration::from_secs(1);
        loop {
            if self.cleanup_attempt().await.is_ok() {
                return match self.fault.take() {
                    Some(error) => Err(error),
                    None => Ok(()),
                };
            }
            tracing::warn!("Docker cleanup remains owned; retrying without releasing namespace");
            sleep(delay).await;
            delay = delay.saturating_mul(2).min(Duration::from_secs(30));
        }
    }
    fn retain_fault(&mut self, error: DockerStartupError) {
        self.fault
            .get_or_insert_with(|| DockerExecutorError::Owner(Arc::new(error)).into());
    }
    async fn pending(&mut self) -> Result<(), DockerStartupError> {
        if let Some(acquiring) = &mut self.acquiring {
            acquiring.cancel();
            let result = acquiring
                .wait(Instant::now() + Duration::from_secs(60))
                .await?;
            self.acquiring.take();
            if let Ok(directory) = result {
                self.directory = Some(directory);
            }
        }
        if let Some(connecting) = &mut self.connecting {
            let result = timeout_at(Instant::now() + Duration::from_secs(60), connecting)
                .await
                .map_err(|_| DockerStartupError::Deadline)?;
            self.connecting.take();
            match result {
                Ok(Ok(wire)) => self.wire = Some(wire),
                Ok(Err(_)) => {}
                Err(_) => self.retain_fault(DockerStartupError::Worker),
            }
        }
        Ok(())
    }
    async fn cleanup_attempt(&mut self) -> Result<(), DockerStartupError> {
        self.pending().await?;
        if let Some(executor) = &self.executor {
            let result = executor.shutdown().await;
            let settled = executor.physically_settled();
            if let Err(error) = result {
                self.retain_fault(DockerStartupError::Cleanup(error));
            }
            if !settled {
                return Err(DockerStartupError::Unavailable);
            }
        }
        if let Some(environments) = &mut self.environments {
            let result = environments
                .shutdown(Instant::now() + Duration::from_secs(60))
                .await;
            let settled = environments.physically_settled();
            if let Err(error) = result {
                self.retain_fault(error.into());
            }
            if !settled {
                return Err(DockerStartupError::Unavailable);
            }
        }
        if let Some(code) = &mut self.code {
            let result = code
                .shutdown(Instant::now() + Duration::from_secs(60))
                .await;
            let settled = code.physically_settled();
            if let Err(error) = result {
                self.retain_fault(error.into());
            }
            if !settled {
                return Err(DockerStartupError::Unavailable);
            }
        }
        if let Some(images) = &mut self.images {
            let result = images
                .shutdown(Instant::now() + Duration::from_secs(60))
                .await;
            let settled = images.physically_settled();
            if let Err(error) = result {
                self.retain_fault(error.into());
            }
            if !settled {
                return Err(DockerStartupError::Unavailable);
            }
        }
        if self.producing {
            let directory = self
                .directory
                .as_ref()
                .ok_or(DockerStartupError::Unavailable)?;
            let docker = self.bound.as_ref().ok_or(DockerStartupError::Unavailable)?;
            let at = Instant::now() + Duration::from_secs(60);
            containers(directory, docker, at).await?;
            images(directory, docker, at).await?;
            volumes(directory, docker, at).await?;
            if !directory.container_records(at).await?.is_empty()
                || !directory.image_records(at).await?.is_empty()
                || !directory.code_records(at).await?.is_empty()
            {
                return Err(DockerError::Quarantined.into());
            }
            self.producing = false;
        }
        if let Some(wire) = &mut self.wire {
            let result = timeout_at(Instant::now() + Duration::from_secs(60), wire.shutdown())
                .await
                .map_err(|_| DockerStartupError::Deadline)?;
            let settled = wire.physically_settled();
            if let Err(error) = result {
                self.retain_fault(error.into());
            }
            if !settled {
                return Err(DockerStartupError::Unavailable);
            }
        }
        Ok(())
    }
}
fn platform(
    os: Option<&str>,
    architecture: Option<&str>,
    cross: bool,
) -> Result<Architecture, DockerStartupError> {
    if os != Some("linux") {
        return Err(DockerStartupError::Configuration);
    }
    let selected = match architecture {
        Some("aarch64" | "arm64") => Architecture::Arm64,
        Some("x86_64" | "amd64") => Architecture::X86_64,
        _ => return Err(DockerStartupError::Configuration),
    };
    if !cross
        && !matches!(
            (ARCH, selected),
            ("aarch64", Architecture::Arm64) | ("x86_64", Architecture::X86_64)
        )
    {
        return Err(DockerStartupError::Configuration);
    }
    Ok(selected)
}
