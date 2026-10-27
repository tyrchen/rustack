use std::{collections::BTreeSet, env, fmt, path::Path, sync::Arc, time::Duration};

use anyhow::{Context, Ok as anyhow_ok, Result, ensure};
use http::StatusCode;
use percent_encoding::percent_decode_str;
use rstest::rstest;
use tempfile::{TempDir, tempdir};
use tokio::time::Instant;
use typed_builder::TypedBuilder;
use uuid::Uuid;

use crate::{
    artifact::{Architecture, DeploymentArtifact, ImageReference, Sha256Digest},
    docker::{
        DirectoryLease, DockerError, DockerPort, DockerWire, EnvironmentError, EnvironmentPlan,
        ImageLeaseConfig, ImageLeaseManager, ImagePin, ImagePinScope,
        ownership::{ContainerRecord, ImageRecord},
        plan::{
            ContainerRole,
            tests::{image_with, policy, revision},
        },
        reconcile_namespace,
        tests::Daemon,
    },
};

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(60)
}

struct Evidence(Option<TempDir>);
impl fmt::Debug for Evidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Evidence(<retained>)")
    }
}
impl Evidence {
    fn path(&self) -> Result<&Path> {
        Ok(self.0.as_ref().context("retained evidence")?.path())
    }
    fn complete(&mut self) {
        self.0.take();
    }
}
impl Drop for Evidence {
    fn drop(&mut self) {
        if let Some(directory) = self.0.take() {
            eprintln!(
                "retained namespace qualification evidence: {}",
                directory.keep().display()
            );
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Mode {
    Recorded,
    OrphanContainer,
    OrphanAlias,
}

#[derive(TypedBuilder)]
struct Fixture {
    wire: DockerWire,
    evidence: Evidence,
    #[builder(default)]
    images: Option<ImageLeaseManager>,
    #[builder(default)]
    planned: Vec<ContainerRecord>,
    #[builder(default)]
    alias: Option<(Uuid, Uuid)>,
    #[builder(default)]
    previous_run: Option<Uuid>,
    #[builder(default)]
    source: Option<ImageReference>,
    #[builder(default)]
    parent: Option<Sha256Digest>,
}
impl fmt::Debug for Fixture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Fixture(<owned>)")
    }
}
impl Fixture {
    async fn setup(
        &mut self,
        architecture: Architecture,
        reference: ImageReference,
        mode: Mode,
    ) -> Result<()> {
        let directory = DirectoryLease::acquire(self.evidence.path()?.into(), deadline()).await?;
        let docker = self
            .wire
            .port()
            .bind_directory(&directory, deadline())
            .await?;
        self.previous_run = Some(directory.run_id());
        self.source = Some(reference.clone());
        self.images = Some(ImageLeaseManager::start(
            docker.clone(),
            ImagePinScope::new(directory.clone()),
            ImageLeaseConfig::default(),
        )?);
        let image = self
            .images
            .as_ref()
            .context("image owner")?
            .port()
            .prepare(&reference, architecture, None, deadline())
            .await?;
        self.parent = Some(image.resolved().metadata().image_id.clone());
        let mut revision = revision();
        revision.architecture = architecture;
        revision.artifact = DeploymentArtifact::Image(image.clone());
        let plan = EnvironmentPlan::new(directory.clone(), image, Arc::new(revision), policy())?;
        if !matches!(mode, Mode::OrphanAlias) {
            for role in [ContainerRole::Relay, ContainerRole::Function] {
                let record = ContainerRecord::from_plan(&plan, role)?;
                self.planned.push(record.clone());
                if matches!(mode, Mode::Recorded) {
                    directory.prepare_container(record, deadline()).await?;
                }
                let created = match role {
                    ContainerRole::Relay => plan.create_relay(&docker, deadline()).await?,
                    ContainerRole::Function => plan.create_function(&docker, deadline()).await?,
                };
                if matches!(role, ContainerRole::Relay) {
                    docker
                        .start_container(&created.id.parse()?, deadline())
                        .await?;
                }
            }
        }
        if !matches!(mode, Mode::OrphanContainer) {
            let pin = ImagePinScope::new(directory.clone()).allocate()?;
            self.alias = Some((pin.run, pin.id));
            let expected = self.parent.as_ref().context("parent image")?;
            if matches!(mode, Mode::Recorded) {
                directory
                    .prepare_image(ImageRecord::new(&pin, expected)?, deadline())
                    .await?;
            }
            docker.tag_image_pin(expected, &pin, deadline()).await?;
        }
        Ok(())
    }
    async fn close_images(&mut self) -> Result<()> {
        if let Some(images) = &mut self.images {
            images.shutdown(deadline()).await?;
        }
        self.images.take();
        Ok(())
    }
    async fn exercise(&self, mode: Mode) -> Result<()> {
        let directory = DirectoryLease::acquire(self.evidence.path()?.into(), deadline()).await?;
        ensure!(
            Some(directory.run_id()) != self.previous_run,
            "run identity did not change"
        );
        let docker = self
            .wire
            .port()
            .bind_directory(&directory, deadline())
            .await?;
        let result = reconcile_namespace(&directory, &docker, deadline()).await;
        match mode {
            Mode::Recorded => {
                let report = result?;
                ensure!(
                    report.containers.settled == 2
                        && report.containers.remaining == 0
                        && report.pins.settled == 1
                        && report.pins.remaining == 0,
                    "recorded namespace not fully settled"
                );
            }
            Mode::OrphanContainer => {
                ensure!(
                    matches!(
                        result,
                        Err(EnvironmentError::Docker(DockerError::Quarantined))
                    ),
                    "unrecorded container accepted"
                );
                for record in &self.planned {
                    let inspected = docker.inspect_container(&record.key()?, deadline()).await?;
                    record.owned(directory.owner_id(), &inspected)?;
                }
            }
            Mode::OrphanAlias => {
                ensure!(
                    matches!(
                        result,
                        Err(EnvironmentError::Docker(DockerError::Quarantined))
                    ),
                    "unrecorded alias accepted"
                );
                let (run, id) = self.alias.context("captured private alias")?;
                let pin = ImagePin::historical(directory.clone(), run, id)?;
                ensure!(
                    docker
                        .inspect_image_pin(&pin, deadline())
                        .await?
                        .id
                        .as_deref()
                        == Some(self.parent.as_ref().context("parent")?.as_str()),
                    "orphan alias was changed"
                );
            }
        }
        Ok(())
    }
    async fn cleanup(&self) -> Result<()> {
        let directory = DirectoryLease::acquire(self.evidence.path()?.into(), deadline()).await?;
        let docker = self
            .wire
            .port()
            .bind_directory(&directory, deadline())
            .await?;
        // Fixture-only cleanup uses identities captured before its own creates;
        // production namespace recovery never adopts unrecorded effects.
        let mut result = Ok(());
        let mut blocked = BTreeSet::new();
        for record in self.planned.iter().rev() {
            if !record.function() && blocked.contains(&record.environment) {
                continue;
            }
            let retired = cleanup_container(&directory, &docker, record).await;
            if retired.is_err() && record.function() {
                blocked.insert(record.environment);
            }
            preserve_error(&mut result, retired);
        }
        if let Some((run, id)) = self.alias.filter(|_| result.is_ok()) {
            let retired = async {
                let pin = ImagePin::historical(directory.clone(), run, id)?;
                cleanup_alias(
                    &directory,
                    &docker,
                    &pin,
                    self.parent.as_ref().context("parent")?,
                )
                .await
            }
            .await;
            preserve_error(&mut result, retired);
        }
        let audit = async {
            let report = reconcile_namespace(&directory, &docker, deadline()).await?;
            ensure!(
                report.containers.remaining == 0 && report.pins.remaining == 0,
                "cleanup retains unresolved ownership"
            );
            anyhow_ok(())
        }
        .await;
        preserve_error(&mut result, audit);
        let source = async {
            let source = docker
                .inspect_image(self.source.as_ref().context("source tag")?, deadline())
                .await?;
            ensure!(
                source.id.as_deref() == Some(self.parent.as_ref().context("parent")?.as_str()),
                "operator source tag changed"
            );
            anyhow_ok(())
        }
        .await;
        preserve_error(&mut result, source);
        result
    }
}
fn preserve_error(result: &mut Result<()>, next: Result<()>) {
    if result.is_ok() {
        *result = next;
    }
}
async fn cleanup_container(
    directory: &DirectoryLease,
    docker: &DockerPort,
    record: &ContainerRecord,
) -> Result<()> {
    let inspected = match docker.inspect_container(&record.key()?, deadline()).await {
        Err(DockerError::Status(StatusCode::NOT_FOUND)) => return Ok(()),
        result => result?,
    };
    let id = record.owned(directory.owner_id(), &inspected)?;
    if directory
        .container_records(deadline())
        .await?
        .iter()
        .any(|saved| {
            saved.environment == record.environment && saved.function() == record.function()
        })
    {
        directory
            .know_container(record.clone(), id.clone(), deadline())
            .await?;
    }
    if inspected.state.and_then(|state| state.running) == Some(true) {
        docker.kill_container(&id, deadline()).await?;
        ensure!(
            docker
                .wait_container(&id, deadline())
                .await?
                .error
                .is_none(),
            "container wait failed"
        );
    }
    docker.remove_container(&id, deadline()).await?;
    ensure!(
        matches!(
            docker.inspect_container(&id, deadline()).await,
            Err(DockerError::Status(StatusCode::NOT_FOUND))
        ),
        "fixture container remains"
    );
    Ok(())
}
async fn cleanup_alias(
    directory: &DirectoryLease,
    docker: &DockerPort,
    pin: &ImagePin,
    parent: &Sha256Digest,
) -> Result<()> {
    match docker.inspect_image_pin(pin, deadline()).await {
        Err(DockerError::Status(StatusCode::NOT_FOUND)) => return Ok(()),
        Ok(image) => ensure!(
            image.id.as_deref() == Some(parent.as_str()),
            "foreign private alias"
        ),
        Err(error) => return Err(error.into()),
    }
    let expected = ImageRecord::new(pin, parent)?;
    if directory
        .image_records(deadline())
        .await?
        .iter()
        .any(|saved| {
            saved
                .pin(directory)
                .is_ok_and(|saved| saved.reference() == pin.reference())
        })
    {
        directory.commit_image(expected, deadline()).await?;
    }
    ensure!(
        docker
            .inspect_image_pin(pin, deadline())
            .await?
            .id
            .as_deref()
            == Some(parent.as_str()),
        "alias identity changed"
    );
    docker.remove_image_pin(pin, deadline()).await?;
    ensure!(
        matches!(
            docker.inspect_image_pin(pin, deadline()).await,
            Err(DockerError::Status(StatusCode::NOT_FOUND))
        ),
        "fixture alias remains"
    );
    Ok(())
}

#[rstest]
#[case::recorded(Mode::Recorded)]
#[case::orphan_container(Mode::OrphanContainer)]
#[case::orphan_alias(Mode::OrphanAlias)]
#[tokio::test]
#[ignore = "requires explicit Docker socket and matching runtime image"]
async fn test_should_reconcile_real_namespace_and_preserve_unrecorded_observations(
    #[case] mode: Mode,
) -> Result<()> {
    let socket = env::var("RUSTACK_RUNTIME_DOCKER_SOCKET").context("explicit Docker socket")?;
    let architecture = match env::var("RUSTACK_RUNTIME_ARCH")?.as_str() {
        "arm64" => Architecture::Arm64,
        "amd64" => Architecture::X86_64,
        _ => anyhow::bail!("unknown architecture"),
    };
    let source = env::var("RUSTACK_RUNTIME_IMAGE")?.parse()?;
    let evidence = Evidence(Some(tempdir()?));
    let wire = DockerWire::connect(format!("unix://{socket}").parse()?, deadline()).await?;
    let mut fixture = Fixture::builder().wire(wire).evidence(evidence).build();
    let setup = fixture.setup(architecture, source, mode).await;
    let images = fixture.close_images().await;
    let exercise = if setup.is_ok() && images.is_ok() {
        fixture.exercise(mode).await
    } else {
        Ok(())
    };
    let cleanup = fixture.cleanup().await;
    let wire = fixture.wire.shutdown().await;
    setup?;
    images?;
    exercise?;
    cleanup?;
    wire?;
    fixture.evidence.complete();
    Ok(())
}

#[tokio::test]
async fn test_should_attempt_independent_cleanup_and_audit_after_first_failure() {
    let root = tempdir().unwrap();
    let directory = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    Daemon::bind_fixture(&directory).await;
    let mut planned = Vec::new();
    for _ in 0..2 {
        let plan = EnvironmentPlan::new(
            directory.clone(),
            image_with("1000", vec![]),
            Arc::new(revision()),
            policy(),
        )
        .unwrap();
        for role in [ContainerRole::Relay, ContainerRole::Function] {
            planned.push(ContainerRecord::from_plan(&plan, role).unwrap());
        }
    }
    let failure = planned[3].key().unwrap().as_str().to_owned();
    let independent = [planned[1].key().unwrap(), planned[0].key().unwrap()];
    drop(directory);
    let mut daemon = Daemon::new();
    let wire = daemon.wire().await;
    let parent: Sha256Digest = format!("sha256:{}", "a".repeat(64)).parse().unwrap();
    let mut fixture = Fixture::builder()
        .wire(wire)
        .evidence(Evidence(Some(root)))
        .planned(planned)
        .source(Some("operator/image:v1".parse().unwrap()))
        .parent(Some(parent.clone()))
        .build();
    let (result, ()) = tokio::join!(fixture.cleanup(), async {
        let request = daemon.next().await;
        ensure_fixture_target(&request.head, &failure);
        request.reply(500, b"{}").await;
        for key in independent {
            let request = daemon.next().await;
            ensure_fixture_target(&request.head, key.as_str());
            request.reply(404, b"{}").await;
        }
        for target in [
            "/containers/json?",
            "/images/json?all=true",
            "/containers/json?",
        ] {
            let request = daemon.next().await;
            assert!(request.head.contains(target));
            request.reply(200, b"[]").await;
        }
        let request = daemon.next().await;
        assert!(request.head.contains("/images/operator%2Fimage%3Av1/json"));
        request
            .reply(200, format!(r#"{{"Id":"{}"}}"#, parent.as_str()).as_bytes())
            .await;
    });
    let cleanup = fixture.wire.shutdown().await;
    daemon.close().await;
    assert!(matches!(
        result.unwrap_err().downcast_ref::<DockerError>(),
        Some(DockerError::Status(StatusCode::INTERNAL_SERVER_ERROR))
    ));
    cleanup.unwrap();
    fixture.evidence.complete();
}
fn ensure_fixture_target(head: &str, key: &str) {
    assert!(
        percent_decode_str(head)
            .decode_utf8()
            .unwrap()
            .contains(&format!("/containers/{key}/json"))
    );
}
