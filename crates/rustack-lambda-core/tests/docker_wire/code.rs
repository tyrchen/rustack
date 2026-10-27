//! Actual typed ZIP transfer into a separately owned daemon-local code volume.

use std::{
    collections::{BTreeMap, HashMap},
    env,
    io::{Cursor, Write},
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use bytes::Bytes;
use http::StatusCode;
use rustack_lambda_core::{
    artifact::Architecture,
    docker::{
        CodeRetirement, CodeVolumeConfig, CodeVolumeIdentity, CodeVolumeManager, DockerCodeArchive,
        DockerError, DockerKey, FunctionLogs, OwnerFilter, PinnedCode,
        models::{
            ContainerCreateBody, ContainerInspectResponse, ContainerStateStatusEnum, HostConfig,
            Mount, MountType, MountVolumeOptions,
        },
    },
    storage::FunctionStore,
};
use serde_json::{Value, json};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
    task::spawn_blocking,
    time::{sleep, timeout_at},
};
use uuid::Uuid;
use zip::{ZipWriter, write::SimpleFileOptions};

use super::{Environment, IO, deadline};

#[derive(Clone, Copy, PartialEq, Eq)]
enum VolumePhase {
    Unissued,
    Uncertain,
    Known,
}
impl VolumePhase {
    fn name(self) -> &'static str {
        match self {
            Self::Unissued => "unissued",
            Self::Uncertain => "uncertain",
            Self::Known => "known",
        }
    }
}
#[derive(Clone)]
enum ContainerPhase {
    Unissued,
    Uncertain,
    Known(DockerKey),
}
impl ContainerPhase {
    fn key(&self) -> Option<&DockerKey> {
        if let Self::Known(key) = self {
            Some(key)
        } else {
            None
        }
    }
    fn record(&self) -> Value {
        match self {
            Self::Unissued => json!({"state":"unissued"}),
            Self::Uncertain => json!({"state":"uncertain"}),
            Self::Known(key) => json!({"state":"known","id":key.as_str()}),
        }
    }
}

struct Fixture {
    environment: Environment,
    volume: DockerKey,
    loader: DockerKey,
    function: DockerKey,
    labels: BTreeMap<String, String>,
    image: String,
    hash: String,
    volume_phase: VolumePhase,
    loader_phase: ContainerPhase,
    function_phase: ContainerPhase,
    manager: Option<CodeVolumeManager>,
    ready: Option<PinnedCode>,
    retirement: Option<CodeRetirement>,
}

impl Fixture {
    async fn new() -> Result<Self> {
        Self::new_with_manager(false).await
    }

    async fn new_with_manager(managed: bool) -> Result<Self> {
        let mut environment = Environment::new_with_code(managed).await?;
        let root = environment
            .directory
            .as_mut()
            .context("code fixture evidence")?;
        root.disable_cleanup(true);
        eprintln!("ZIP code evidence: {}", root.path().display());
        let id = Uuid::new_v4();
        let labels = BTreeMap::from([
            (
                "rustack.lambda.ownerId".into(),
                environment.names.owner.to_string(),
            ),
            (
                "rustack.lambda.runId".into(),
                environment.names.run.to_string(),
            ),
            ("rustack.lambda.environmentId".into(), id.to_string()),
            ("rustack.test.code".into(), id.to_string()),
        ]);
        Ok(Self {
            environment,
            volume: format!("rustack-code-{id}").parse()?,
            loader: format!("rustack-code-loader-{id}").parse()?,
            function: format!("rustack-code-consumer-{id}").parse()?,
            labels,
            image: String::new(),
            hash: String::new(),
            volume_phase: VolumePhase::Unissued,
            loader_phase: ContainerPhase::Unissued,
            function_phase: ContainerPhase::Unissued,
            manager: None,
            ready: None,
            retirement: None,
        })
    }

    async fn record(&self) -> Result<()> {
        let root = self
            .environment
            .directory
            .as_ref()
            .context("code evidence root")?
            .path();
        let record = serde_json::to_vec(&json!({"schemaVersion":1,"volume":self.volume.as_str(),
            "loader":self.loader.as_str(),"function":self.function.as_str(),"labels":self.labels,
            "image":self.image,"zipSha256":self.hash,"volumeState":self.volume_phase.name(),
            "loaderState":self.loader_phase.record(),"functionState":self.function_phase.record()}))?;
        timeout_at(deadline(), async {
            let pending = root.join("code-intent.pending");
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&pending)
                .await?;
            file.write_all(&record).await?;
            file.sync_all().await?;
            drop(file);
            fs::rename(&pending, root.join("code-intent.json")).await?;
            fs::File::open(root).await?.sync_all().await
        })
        .await??;
        Ok(())
    }

    fn body(&self, loader: bool) -> ContainerCreateBody {
        let mut labels: HashMap<String, String> = self.labels.clone().into_iter().collect();
        labels.insert(
            "rustack.lambda.role".into(),
            if loader {
                "code-loader"
            } else {
                "code-consumer"
            }
            .into(),
        );
        ContainerCreateBody {
            image: Some(self.image.clone()),
            user: Some(if loader { "65534:65534" } else { "1000:1000" }.into()),
            entrypoint: Some(vec![
                if loader {
                    "/never-start-this-loader"
                } else {
                    "/var/task/bootstrap"
                }
                .into(),
            ]),
            cmd: Some(Vec::new()),
            working_dir: Some("/var/task".into()),
            labels: Some(labels),
            tty: Some(false),
            host_config: Some(HostConfig {
                network_mode: Some("none".into()),
                readonly_rootfs: Some(true),
                memory: Some(128 * 1024 * 1024),
                nano_cpus: Some(1_000_000_000),
                pids_limit: Some(64),
                cap_drop: Some(vec!["ALL".into()]),
                security_opt: Some(vec!["no-new-privileges:true".into()]),
                tmpfs: Some(HashMap::from([(
                    "/tmp".into(),
                    "rw,nosuid,nodev,size=536870912".into(),
                )])),
                mounts: Some(vec![Mount {
                    typ: Some(MountType::VOLUME),
                    source: Some(self.volume.as_str().into()),
                    target: Some("/var/task".into()),
                    read_only: Some(!loader),
                    volume_options: Some(MountVolumeOptions {
                        no_copy: Some(true),
                        ..MountVolumeOptions::default()
                    }),
                    ..Mount::default()
                }]),
                ..HostConfig::default()
            }),
            ..ContainerCreateBody::default()
        }
    }

    fn owned(&self, inspected: &ContainerInspectResponse, loader: bool) -> Result<DockerKey> {
        let name = if loader { &self.loader } else { &self.function };
        let config = inspected
            .config
            .as_ref()
            .context("owned container config")?;
        let expected = self.body(loader);
        let id = inspected.id.as_deref().context("owned container ID")?;
        ensure!(
            id.len() == 64
                && id
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "noncanonical container ID"
        );
        ensure!(
            inspected.name.as_deref() == Some(format!("/{}", name.as_str()).as_str())
                && config.image.as_deref() == Some(self.image.as_str())
                && expected.labels.as_ref().is_some_and(|required| config
                    .labels
                    .as_ref()
                    .is_some_and(|actual| required
                        .iter()
                        .all(|(key, value)| actual.get(key) == Some(value)))),
            "foreign code fixture container"
        );
        let mounts = inspected.mounts.as_ref().context("actual code mount")?;
        ensure!(
            mounts
                .iter()
                .any(|mount| mount.name.as_deref() == Some(self.volume.as_str())
                    && mount.destination.as_deref() == Some("/var/task")
                    && mount.rw == Some(loader)),
            "actual volume mount mode/name mismatch"
        );
        Ok(id.parse()?)
    }

    async fn observe(&mut self, loader: bool) -> Result<DockerKey> {
        let name = if loader { &self.loader } else { &self.function };
        let inspected = self
            .environment
            .docker
            .inspect_container(name, deadline())
            .await?;
        let id = self.owned(&inspected, loader)?;
        if loader {
            self.loader_phase = ContainerPhase::Known(id.clone());
        } else {
            self.function_phase = ContainerPhase::Known(id.clone());
        }
        self.record().await?;
        Ok(id)
    }

    async fn prepare_code(&mut self) -> Result<(DockerCodeArchive, Architecture)> {
        let arch = env::var("RUSTACK_RUNTIME_ARCH")?;
        let image = self
            .environment
            .image(&format!("rustack-custom-ric:a-{arch}"), &arch)
            .await?;
        image
            .resolved()
            .metadata()
            .image_id
            .as_str()
            .clone_into(&mut self.image);
        let architecture = image.resolved().metadata().architecture;
        let path = env::var("RUSTACK_ZIP_CODE_BOOTSTRAP")
            .context("explicit cross-built ZIP probe required")?;
        let bytes = timeout_at(deadline(), async {
            let mut file = fs::File::open(path).await?;
            let mut bytes = Vec::new();
            (&mut file)
                .take(20 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .await?;
            ensure!(bytes.len() <= 20 * 1024 * 1024, "probe source cap");
            Ok::<_, anyhow::Error>(bytes)
        })
        .await??;
        let zip = timeout_at(deadline(), spawn_blocking(move || package(&bytes))).await???;
        let root = self
            .environment
            .directory
            .as_ref()
            .context("artifact root")?
            .path()
            .join("artifacts");
        let store = FunctionStore::new(&root);
        let (path, hash, _) = store
            .store_zip_code("code-probe", "$LATEST", &Bytes::from(zip))
            .await?;
        let artifact = store.capture_artifact(&path).await?;
        self.hash = hash;
        timeout_at(
            deadline(),
            fs::write(path.join("bootstrap"), b"mutated host extraction"),
        )
        .await??;
        let code = DockerCodeArchive::prepare(artifact, architecture, deadline()).await?;
        ensure!(
            code.len() > 1024 * 1024 && code.code_sha256() == self.hash,
            "large immutable transfer identity"
        );
        Ok((code, architecture))
    }

    async fn exercise(&mut self) -> Result<()> {
        let (code, architecture) = self.prepare_code().await?;
        self.volume_phase = VolumePhase::Uncertain;
        self.record().await?;
        self.environment
            .docker
            .create_volume(&self.volume, &self.labels, deadline())
            .await?;
        self.volume_phase = VolumePhase::Known;
        self.record().await?;
        self.loader_phase = ContainerPhase::Uncertain;
        self.record().await?;
        self.environment
            .docker
            .create_container_on_platform(&self.loader, &self.body(true), architecture, deadline())
            .await?;
        let loader = self.observe(true).await?;
        self.require_unstarted(&loader).await?;
        self.environment
            .docker
            .upload_code(&loader, &code, deadline())
            .await?;
        self.require_unstarted(&loader).await?;
        self.retire(true).await?;
        self.function_phase = ContainerPhase::Uncertain;
        self.record().await?;
        self.environment
            .docker
            .create_container_on_platform(
                &self.function,
                &self.body(false),
                architecture,
                deadline(),
            )
            .await?;
        let function = self.observe(false).await?;
        self.consume_probe(&function).await?;
        Ok(())
    }

    async fn exercise_managed(&mut self) -> Result<()> {
        let (code, architecture) = self.prepare_code().await?;
        let base = self
            .environment
            .artifacts
            .last()
            .context("source pin")?
            .clone();
        let manager = CodeVolumeManager::start(
            self.environment.docker.clone(),
            self.environment.lease.clone(),
            CodeVolumeConfig::default(),
        )?;
        let port = manager.port();
        self.manager = Some(manager);
        let preparation = port.start_preparation(
            CodeVolumeIdentity::new(Uuid::new_v4(), Uuid::new_v4())?,
            code,
            base,
            deadline() + Duration::from_secs(120),
        )?;
        self.retirement = Some(preparation.retirement());
        let ready = preparation.wait().await?;
        self.volume = ready.volume_name().clone();
        ensure!(
            ready.base_image_id().as_str() == self.image,
            "changed base image"
        );
        let mount = ready.mount()?;
        self.ready = Some(ready);
        ensure!(
            port.status().volumes == 1 && port.status().preparing == 0,
            "Ready admission"
        );
        self.function_phase = ContainerPhase::Uncertain;
        self.record().await?;
        let mut body = self.body(false);
        body.host_config.as_mut().context("consumer policy")?.mounts = Some(vec![mount]);
        self.environment
            .docker
            .create_container_on_platform(&self.function, &body, architecture, deadline())
            .await?;
        let function = self.observe(false).await?;
        self.consume_probe(&function).await?;
        ensure!(
            port.status().volumes == 1,
            "consumer must retain code ownership"
        );
        Ok(())
    }

    async fn consume_probe(&mut self, function: &DockerKey) -> Result<()> {
        self.environment.logs = Some(FunctionLogs::start(
            self.environment
                .docker
                .attach_logs(function, deadline())
                .await?,
            IO,
            65_536,
        )?);
        self.environment
            .docker
            .start_container(function, deadline())
            .await?;
        let exit = self
            .environment
            .docker
            .wait_container(function, deadline())
            .await?;
        let logs = self.environment.logs.as_mut().context("probe log owner")?;
        timeout_at(deadline(), async {
            loop {
                let snapshot = logs.snapshot(deadline()).await?;
                if snapshot.closed {
                    ensure!(snapshot.failure.is_none(), "probe log failure");
                    break;
                }
                sleep(Duration::from_millis(10)).await;
            }
            Ok::<_, anyhow::Error>(())
        })
        .await??;
        let tail = logs.shutdown().await?.tail;
        let evidence = self
            .environment
            .directory
            .as_ref()
            .context("probe evidence root")?
            .path()
            .join("code-output.log");
        timeout_at(deadline(), fs::write(evidence, &tail)).await??;
        ensure!(
            exit.status_code == 0 && exit.error.is_none(),
            "code probe exited unsuccessfully"
        );
        let output: Value = serde_json::from_slice(&tail)?;
        ensure!(
            output
                == json!({"computed":42,"readOnly":true,"tmpWritable":true,"metadata":[
            {"uid":0,"gid":0,"mode":0o555},{"uid":0,"gid":0,"mode":0o555},{"uid":0,"gid":0,"mode":0o444}]}),
            "actual ZIP code/ownership/mode computation mismatch"
        );
        Ok(())
    }

    async fn require_unstarted(&self, key: &DockerKey) -> Result<()> {
        let inspected = self
            .environment
            .docker
            .inspect_container(key, deadline())
            .await?;
        self.owned(&inspected, true)?;
        ensure!(
            inspected
                .state
                .is_some_and(|state| state.running == Some(false)
                    && state.status == Some(ContainerStateStatusEnum::CREATED)),
            "loader was started"
        );
        Ok(())
    }

    async fn retire(&mut self, loader: bool) -> Result<()> {
        let (phase, name) = if loader {
            (self.loader_phase.clone(), self.loader.clone())
        } else {
            (self.function_phase.clone(), self.function.clone())
        };
        if matches!(phase, ContainerPhase::Unissued) {
            return Ok(());
        }
        let known = phase.key().cloned();
        let lookup = known.as_ref().unwrap_or(&name);
        let inspected = match self
            .environment
            .docker
            .inspect_container(lookup, deadline())
            .await
        {
            Err(DockerError::Status(StatusCode::NOT_FOUND)) if known.is_some() => return Ok(()),
            result => result?,
        };
        let id = self.owned(&inspected, loader)?;
        ensure!(
            known.as_ref().is_none_or(|known| known == &id),
            "changed known container identity"
        );
        if loader {
            self.loader_phase = ContainerPhase::Known(id.clone());
        } else {
            self.function_phase = ContainerPhase::Known(id.clone());
        }
        self.record().await?;
        match inspected.state.and_then(|state| state.running) {
            Some(true) => {
                let _ = self
                    .environment
                    .docker
                    .kill_container(&id, deadline())
                    .await;
                self.environment
                    .docker
                    .wait_container(&id, deadline())
                    .await?;
            }
            Some(false) => {}
            None => bail!("missing code container state"),
        }
        let _ = self
            .environment
            .docker
            .remove_container(&id, deadline())
            .await;
        ensure!(
            matches!(
                self.environment
                    .docker
                    .inspect_container(&id, deadline())
                    .await,
                Err(DockerError::Status(StatusCode::NOT_FOUND))
            ),
            "code container retirement unproven"
        );
        Ok(())
    }

    async fn cleanup(&mut self) -> Result<()> {
        self.retire(false).await?;
        if let Some(manager) = &mut self.manager {
            self.ready.take();
            if let Some(retirement) = &mut self.retirement {
                retirement.wait(deadline()).await?;
            }
            manager.shutdown(deadline()).await?;
        }
        self.retire(true).await?;
        if self.volume_phase != VolumePhase::Unissued {
            match self
                .environment
                .docker
                .inspect_volume(&self.volume, deadline())
                .await
            {
                Err(DockerError::Status(StatusCode::NOT_FOUND))
                    if self.volume_phase == VolumePhase::Known => {}
                result => {
                    let volume = result?;
                    ensure!(
                        volume.name == self.volume.as_str()
                            && volume.driver == "local"
                            && volume.options.is_empty()
                            && volume.labels == self.labels.clone().into_iter().collect(),
                        "foreign code volume"
                    );
                    self.volume_phase = VolumePhase::Known;
                    self.record().await?;
                    let _ = self
                        .environment
                        .docker
                        .remove_volume(&self.volume, deadline())
                        .await;
                    ensure!(
                        matches!(
                            self.environment
                                .docker
                                .inspect_volume(&self.volume, deadline())
                                .await,
                            Err(DockerError::Status(StatusCode::NOT_FOUND))
                        ),
                        "code volume retirement unproven"
                    );
                }
            }
        }
        ensure!(
            self.environment
                .docker
                .list_volumes(
                    OwnerFilter::new(self.environment.names.owner, None, None)?,
                    deadline()
                )
                .await?
                .is_empty(),
            "owned code volumes remain"
        );
        self.environment.cleanup().await
    }
}

#[tokio::test]
#[ignore = "requires actual retained code owner; run make test-lambda-docker"]
async fn test_should_prepare_code_with_retained_owner_and_consume_it_read_only() -> Result<()> {
    let mut fixture = Fixture::new_with_manager(true).await?;
    let result = timeout_at(
        deadline() + Duration::from_secs(180),
        fixture.exercise_managed(),
    )
    .await
    .context("managed code fixture observer expired")
    .and_then(|result| result);
    let cleanup = fixture.cleanup().await;
    result?;
    cleanup?;
    fixture
        .environment
        .directory
        .as_mut()
        .context("code evidence root")?
        .disable_cleanup(false);
    Ok(())
}

fn package(bootstrap: &[u8]) -> Result<Vec<u8>> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer.start_file(
        "bootstrap",
        SimpleFileOptions::default().unix_permissions(0o755),
    )?;
    writer.write_all(bootstrap)?;
    writer.start_file(
        "data/input",
        SimpleFileOptions::default().unix_permissions(0o644),
    )?;
    writer.write_all(b"21")?;
    writer.start_file(
        "large-data",
        SimpleFileOptions::default().unix_permissions(0o644),
    )?;
    writer.write_all(&vec![b'x'; 2 * 1024 * 1024])?;
    Ok(writer.finish()?.into_inner())
}

#[tokio::test]
#[ignore = "requires actual code-volume transport; run make test-lambda-docker-wire"]
async fn test_should_upload_zip_code_to_a_private_volume_and_consume_it_read_only() -> Result<()> {
    let mut fixture = Fixture::new().await?;
    let result = timeout_at(deadline() + Duration::from_secs(180), fixture.exercise())
        .await
        .context("code fixture observer expired")
        .and_then(|result| result);
    let cleanup = fixture.cleanup().await;
    result?;
    cleanup?;
    fixture
        .environment
        .directory
        .as_mut()
        .context("code evidence root")?
        .disable_cleanup(false);
    Ok(())
}
