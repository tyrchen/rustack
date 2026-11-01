//! Real archive restore with moved references and valid but foreign affiliations.

use std::{env, path::Path, time::Duration};

use anyhow::{Context, Result, ensure};
use rustack_lambda_core::{
    artifact::ImageReference,
    docker::{DockerError, DockerWire},
};
use sha2::{Digest, Sha256};
use tokio::{
    fs,
    time::{Instant, timeout},
};
use uuid::Uuid;

use super::{
    archive, copy_tree,
    custom::Alias,
    fingerprint,
    process::{self, Profile, Server},
    sdk,
};

pub(super) async fn exercise() -> Result<()> {
    let mut server = Server::spawn(Profile::Archive).await?;
    let mut alias = None;
    let result = timeout(Duration::from_secs(600), async {
        server.ready().await?;
        let a = env::var("RUSTACK_CUSTOM_IMAGE_A")?;
        let b = env::var("RUSTACK_CUSTOM_IMAGE_B")?;
        let ids = [server.track_image(&a).await?, server.track_image(&b).await?];
        ensure!(
            ids.first() != ids.get(1),
            "moving-tag sources are identical"
        );
        alias = Some(Alias::new(&server, ids).await?);
        let moving = alias.as_mut().context("retained source alias")?;
        moving.select(0).await?;
        let mut witness = sdk::Witness::start_image(&server, moving.name()).await?;
        server.stop(true).await?;
        server.audit().await?;
        moving.select(1).await?;
        server.restart().await?;
        server.ready().await?;
        witness.cold_revision("A").await?;
        server.stop(true).await?;
        server.audit().await?;
        let snapshots = server.path()?.join("snapshots");
        let current = snapshots.join("images");
        let previous = snapshots.join(".images.previous");
        copy_tree(&current, &previous).await?;
        let meta = current.join("services/lambda/meta.ss.zst");
        let original_meta = fs::read(&meta).await?;
        let absent = archive::missing_image(server.path()?).await?;
        let at = Instant::now() + Duration::from_secs(10);
        let mut wire = DockerWire::connect(process::socket()?.parse()?, at).await?;
        let missing: ImageReference = absent.parse()?;
        let observed = wire.port().inspect_image(&missing, at).await;
        let joined = wire.shutdown().await;
        joined?;
        ensure!(
            matches!(observed, Err(DockerError::Status(status)) if status.as_u16() == 404),
            "absent-image fixture unexpectedly exists"
        );
        let before = fingerprint(&snapshots).await?;
        server.restart().await?;
        server
            .failure_with(Some("Lambda immutable image restore failed"))
            .await?;
        ensure!(
            fingerprint(&snapshots).await? == before,
            "missing image changed snapshot generations"
        );
        server.stop(false).await?;
        fs::write(&meta, original_meta).await?;
        reject_affiliations(&mut server, &snapshots).await?;
        server.audit().await?;
        Ok::<(), anyhow::Error>(())
    })
    .await
    .context("archive rejection observer expired")
    .and_then(|value| value);
    let cleanup = server.reap().await;
    let alias_cleanup = if let Some(moving) = &mut alias {
        moving.cleanup().await
    } else {
        Ok(())
    };
    let audit = server.audit().await;
    if result.is_err() || cleanup.is_err() || alias_cleanup.is_err() || audit.is_err() {
        server.retain_on_error().await;
    }
    result?;
    cleanup?;
    alias_cleanup?;
    audit?;
    Ok(())
}

async fn reject_affiliations(server: &mut Server, snapshots: &Path) -> Result<()> {
    let affiliation = server.path()?.join("data/.rustack-lambda-docker/daemon.v1");
    let original = fs::read(&affiliation).await?;
    ensure!(original.len() == 72, "daemon record fixture length");
    fs::write(server.path()?.join("affiliation-original.v1"), &original).await?;
    for (range, role) in [(8..24, "owner"), (24..40, "engine")] {
        let mut changed = original.clone();
        changed
            .get_mut(range)
            .context("affiliation UUID")?
            .copy_from_slice(Uuid::new_v4().as_bytes());
        let hash = Sha256::digest(changed.get(..40).context("affiliation payload")?);
        changed
            .get_mut(40..)
            .context("affiliation checksum")?
            .copy_from_slice(&hash);
        fs::write(
            server.path()?.join(format!("affiliation-{role}-intent.v1")),
            &changed,
        )
        .await?;
        fs::write(&affiliation, &changed).await?;
        let before = fingerprint(snapshots).await?;
        server.restart().await?;
        server
            .failure_with(Some("invalid Docker ownership state"))
            .await?;
        server.stop(false).await?;
        ensure!(
            fingerprint(snapshots).await? == before && fs::read(&affiliation).await? == changed,
            "foreign affiliation was rewritten or snapshot changed"
        );
        fs::write(&affiliation, &original).await?;
    }
    Ok(())
}
