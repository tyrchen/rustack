use std::{collections::BTreeMap, env, fmt, path::Path, time::Duration};

use anyhow::{Context, Error, Result, ensure};
use http::StatusCode;
use rustack_core::settings::ValidatedSettings;
use tempfile::{TempDir, tempdir};
use tokio::{
    fs,
    io::AsyncWriteExt,
    sync::watch,
    time::{Instant, timeout_at},
};

use crate::{
    artifact::{Architecture, ImageReference},
    docker::{
        DirectoryLease, DockerError, DockerRuntimeSettings, DockerWire, ImageLeaseConfig,
        ImageLeaseManager, ImagePinScope, ImagePullPolicy, reconcile_namespace,
    },
};

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(600)
}
fn io_deadline() -> Instant {
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
        Ok(self.0.as_ref().context("evidence directory")?.path())
    }
}
impl Drop for Evidence {
    fn drop(&mut self) {
        if let Some(root) = self.0.take() {
            eprintln!("retained pull evidence: {}", root.keep().display());
        }
    }
}

#[tokio::test]
#[ignore = "requires explicit Docker socket, architecture and immutable approved image"]
async fn test_should_pull_approved_immutable_image_prepare_live_pin_and_join_cleanup() -> Result<()>
{
    let socket = env::var("RUSTACK_RUNTIME_DOCKER_SOCKET").context("explicit Docker socket")?;
    let architecture = match env::var("RUSTACK_RUNTIME_ARCH")?.as_str() {
        "arm64" => Architecture::Arm64,
        "amd64" => Architecture::X86_64,
        _ => anyhow::bail!("unknown architecture"),
    };
    let reference: ImageReference = env::var("RUSTACK_PULL_IMAGE")
        .context("explicit immutable approved pull image")?
        .parse()?;
    let mut evidence = Evidence(Some(tempdir()?));
    let mut wire = DockerWire::connect(format!("unix://{socket}").parse()?, io_deadline()).await?;
    let mut images = None;
    let result = async {
        let directory = DirectoryLease::acquire(evidence.path()?.into(), io_deadline()).await?;
        let docker = wire
            .port()
            .bind_directory(&directory, io_deadline())
            .await?;
        ensure!(
            reference.digest().is_some() && reference.registry() == Some("public.ecr.aws"),
            "fixture requires immutable public.ecr.aws reference"
        );
        let cache_miss = match docker.inspect_image(&reference, io_deadline()).await {
            Ok(_) => false,
            Err(DockerError::Status(StatusCode::NOT_FOUND)) => true,
            Err(error) => return Err(error.into()),
        };
        let policy = configured_policy(evidence.path()?, &reference).await?;
        images = Some(ImageLeaseManager::start_with_pull(
            docker.clone(),
            ImagePinScope::new(directory.clone()),
            ImageLeaseConfig::default(),
            policy.clone(),
        )?);
        let image = images
            .as_ref()
            .context("image owner")?
            .port()
            .prepare(&reference, architecture, None, deadline())
            .await?;
        ensure!(
            image.is_live() && image.unpacked_size().get() > 0,
            "prepared image is not live"
        );
        let records = directory.image_records(io_deadline()).await?;
        ensure!(
            records.len() == 1 && records.iter().all(|record| record.committed),
            "pin not committed"
        );
        let expected = image.resolved().metadata().image_id.clone();
        // Always execute the actual endpoint too, even on a repeated warm run.
        let (_cancel, cancel) = watch::channel(());
        let (_stop, stop) = watch::channel(false);
        policy
            .pull(&docker, &reference, architecture, cancel, stop)
            .await?;
        ensure!(
            docker
                .inspect_image(&reference, io_deadline())
                .await?
                .id
                .as_deref()
                == Some(expected.as_str()),
            "immutable source changed"
        );
        drop(image);
        eprintln!(
            "approved real pull/preparation cacheMiss={cache_miss}; source cache intentionally \
             retained"
        );
        Ok::<(), Error>(())
    }
    .await;
    let cleanup = match images.as_mut() {
        Some(owner) => owner.shutdown(deadline()).await,
        None => Ok(()),
    };
    if cleanup.is_ok() {
        images.take();
    }
    let audit = audit_scope(&wire, evidence.path()?).await;
    let wire_cleanup = wire.shutdown().await;
    result?;
    cleanup?;
    audit?;
    wire_cleanup?;
    evidence.0.take();
    Ok(())
}

async fn audit_scope(wire: &DockerWire, root: &Path) -> Result<()> {
    let directory = DirectoryLease::acquire(root.into(), io_deadline()).await?;
    let docker = wire
        .port()
        .bind_directory(&directory, io_deadline())
        .await?;
    let report = reconcile_namespace(&directory, &docker, io_deadline()).await?;
    ensure!(
        report.containers.remaining == 0 && report.pins.remaining == 0,
        "owned resources remain"
    );
    Ok(())
}

async fn configured_policy(root: &Path, reference: &ImageReference) -> Result<ImagePullPolicy> {
    timeout_at(io_deadline(), async {
        let path = fs::canonicalize(root)
            .await?
            .join("registry-credentials.yaml");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .await?;
        file.write_all(b"{}\n").await?;
        drop(file);
        let shared = ValidatedSettings::parse(
            "budgets:\n  lambdaDockerPullSeconds: 600",
            &BTreeMap::from([
                (
                    "LAMBDA_DOCKER_RELAY_IMAGE".into(),
                    reference.as_str().to_owned(),
                ),
                ("LAMBDA_DOCKER_NETWORK".into(), "none".into()),
                ("LAMBDA_DOCKER_PULL_POLICY".into(), "ifMissing".into()),
                ("LAMBDA_DOCKER_REGISTRIES".into(), "public.ecr.aws".into()),
                (
                    "LAMBDA_DOCKER_REGISTRY_AUTH_FILE".into(),
                    path.to_str().context("UTF-8 fixture path")?.to_owned(),
                ),
            ]),
        )?;
        let settings = DockerRuntimeSettings::from_lookup(
            |key| shared.get(key).map(str::to_owned),
            shared.budgets(),
            Duration::from_secs(600),
        )?;
        Ok::<_, Error>(settings.load_pull_policy(io_deadline()).await?)
    })
    .await?
}
