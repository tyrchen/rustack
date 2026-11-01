use std::{env, path::Path, time::Duration};

use anyhow::{Context, Result, ensure};
use tokio::time::Instant;
use uuid::Uuid;

use crate::{
    artifact::{Architecture, ImageReference, ResolvedImage, Sha256Digest},
    docker::{
        DirectoryLease, DockerWire, ImagePinScope, ImageResolver,
        models::{ContainerCreateBody, HostConfig, Mount, MountType, MountVolumeOptions},
        ownership::{
            ImageRecord,
            codes::{CodeRecord, LoaderState, VolumeState, tests::publish},
        },
        reconcile_namespace,
    },
};

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(60)
}
fn code_record(run: Uuid, image: &ResolvedImage) -> Result<CodeRecord> {
    let mut base = [0; 32];
    let mut fingerprint = [0; 32];
    hex::decode_to_slice(
        image
            .metadata()
            .image_id
            .as_str()
            .strip_prefix("sha256:")
            .context("base digest")?,
        &mut base,
    )?;
    hex::decode_to_slice(
        image
            .metadata()
            .inspection_hash
            .as_str()
            .strip_prefix("sha256:")
            .context("inspection digest")?,
        &mut fingerprint,
    )?;
    Ok(CodeRecord::builder()
        .run(run)
        .environment(Uuid::new_v4())
        .resource(Uuid::new_v4())
        .revision(Uuid::new_v4())
        .zip([0x11; 32])
        .base(base)
        .fingerprint(fingerprint)
        .volume(VolumeState::Uncertain)
        .loader(LoaderState::Planned)
        .build())
}
fn loader_body(
    row: &CodeRecord,
    directory: &DirectoryLease,
    parent: &Sha256Digest,
) -> Result<ContainerCreateBody> {
    Ok(ContainerCreateBody {
        image: Some(parent.as_str().into()),
        user: Some("65534:65534".into()),
        entrypoint: Some(vec!["/never-start-this-loader".into()]),
        cmd: Some(Vec::new()),
        working_dir: Some("/var/task".into()),
        labels: Some(row.labels(directory.owner_id(), true).into_iter().collect()),
        host_config: Some(HostConfig {
            network_mode: Some("none".into()),
            readonly_rootfs: Some(true),
            memory: Some(128 * 1024 * 1024),
            pids_limit: Some(64),
            cap_drop: Some(vec!["ALL".into()]),
            security_opt: Some(vec!["no-new-privileges:true".into()]),
            mounts: Some(vec![Mount {
                typ: Some(MountType::VOLUME),
                source: Some(row.volume_key()?.as_str().into()),
                target: Some("/var/task".into()),
                read_only: Some(false),
                volume_options: Some(MountVolumeOptions {
                    no_copy: Some(true),
                    ..MountVolumeOptions::default()
                }),
                ..Mount::default()
            }]),
            ..HostConfig::default()
        }),
        ..ContainerCreateBody::default()
    })
}
async fn exercise(
    wire: &DockerWire,
    root: &Path,
    architecture: Architecture,
    source: &ImageReference,
) -> Result<()> {
    let directory = DirectoryLease::acquire_code(root.into(), deadline()).await?;
    let port = wire.port().bind_directory(&directory, deadline()).await?;
    let resolved = ImageResolver::new(port.clone())
        .resolve_local(source, architecture, None, deadline())
        .await?;
    let parent = resolved.metadata().image_id.clone();
    let pin = ImagePinScope::new(directory.clone()).allocate()?;
    let image_record = ImageRecord::new(&pin, &parent)?;
    directory
        .prepare_image(image_record.clone(), deadline())
        .await?;
    port.tag_image_pin(&parent, &pin, deadline()).await?;
    directory.commit_image(image_record, deadline()).await?;
    let mut row = code_record(directory.run_id(), &resolved)?;
    publish(&directory, vec![row.clone()]).await;
    let volume_key = row.volume_key()?;
    let volume = port
        .create_volume(
            &volume_key,
            &row.labels(directory.owner_id(), false),
            deadline(),
        )
        .await?;
    row.owned_volume(directory.owner_id(), &volume)?;
    row.volume = VolumeState::Known;
    row.loader = LoaderState::Uncertain;
    publish(&directory, vec![row.clone()]).await;
    let loader_key = row.loader_key()?;
    let body = loader_body(&row, &directory, &parent)?;
    let created = port
        .create_container(&loader_key, &body, deadline())
        .await?;
    let inspected = port
        .inspect_container(&created.id.parse()?, deadline())
        .await?;
    let id = row.owned_loader(directory.owner_id(), &inspected)?;
    row.loader = LoaderState::Known(id);
    publish(&directory, vec![row.clone()]).await;
    let old_run = directory.run_id();
    let owner = directory.owner_id();
    drop(port);
    drop(pin);
    drop(directory);
    let directory = DirectoryLease::acquire(root.into(), deadline()).await?;
    ensure!(
        directory.owner_id() == owner && directory.run_id() != old_run,
        "immutable owner/fresh run"
    );
    let port = wire.port().bind_directory(&directory, deadline()).await?;
    let report = reconcile_namespace(&directory, &port, deadline()).await?;
    ensure!(
        report.code.settled == 1
            && report.code.remaining == 0
            && report.pins.settled == 1
            && report.pins.remaining == 0
            && report.containers.remaining == 0,
        "recorded code/pin retirement"
    );
    ensure!(
        directory.code_records(deadline()).await?.is_empty(),
        "code journal remains"
    );
    ensure!(
        port.inspect_image(source, deadline()).await?.id.as_deref() == Some(parent.as_str()),
        "source image changed"
    );
    let repeated = reconcile_namespace(&directory, &port, deadline()).await?;
    ensure!(
        repeated.code.settled == 0 && repeated.code.remaining == 0,
        "recovery not idempotent"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires explicit Docker socket and selected local runtime image"]
async fn test_should_recover_real_code_volume_and_unstarted_loader_across_reacquisition()
-> Result<()> {
    let socket = env::var("RUSTACK_RUNTIME_DOCKER_SOCKET").context("explicit Docker socket")?;
    let architecture = match env::var("RUSTACK_RUNTIME_ARCH")?.as_str() {
        "arm64" => Architecture::Arm64,
        "amd64" => Architecture::X86_64,
        _ => anyhow::bail!("unknown architecture"),
    };
    let source = env::var("RUSTACK_RUNTIME_IMAGE")?.parse()?;
    let mut root = tempfile::tempdir()?;
    root.disable_cleanup(true);
    eprintln!(
        "retained code namespace recovery evidence: {}",
        root.path().display()
    );
    let mut wire = DockerWire::connect(format!("unix://{socket}").parse()?, deadline()).await?;
    let result = async {
        if let Some(previous) = env::var_os("RUSTACK_CODE_RECOVERY_RESUME") {
            let directory = DirectoryLease::acquire(previous.into(), deadline()).await?;
            let port = wire.port().bind_directory(&directory, deadline()).await?;
            let report = reconcile_namespace(&directory, &port, deadline()).await?;
            ensure!(
                report.code.remaining == 0
                    && report.containers.remaining == 0
                    && report.pins.remaining == 0,
                "retained evidence still unresolved"
            );
        }
        exercise(&wire, root.path(), architecture, &source).await
    }
    .await;
    let joined = wire.shutdown().await;
    result?;
    joined?;
    root.disable_cleanup(false);
    Ok(())
}
