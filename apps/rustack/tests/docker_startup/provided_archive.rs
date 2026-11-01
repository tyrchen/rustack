//! Ordinary mixed archives; both server handles outlive every observer outcome.

use std::{env, ffi::OsString, path::PathBuf, time::Duration};

use anyhow::{Context, Result, ensure};
use serde_json::Value;
use tokio::{
    fs,
    time::{Instant, timeout},
};

use super::{
    archive, copy_tree, fingerprint,
    process::{self, Profile, Server},
    provided::verify_digests,
    runner::{Runner, interpreter},
};

fn guard(server: &mut Server, retained: bool) -> Result<()> {
    server
        .root
        .as_mut()
        .context("provided archive evidence guard")?
        .disable_cleanup(retained);
    Ok(())
}
pub(super) async fn exercise(no_source: bool) -> Result<()> {
    let mut source = Server::spawn(Profile::ZipArchive).await?;
    guard(&mut source, true)?;
    eprintln!("provided archive evidence: {}", source.path()?.display());
    let mut consumer = None;
    let observed = timeout(
        Duration::from_mins(12),
        run(&mut source, &mut consumer, no_source),
    )
    .await
    .context("provided archive observer expired")
    .and_then(|result| result);
    let source_stop = source.stop(no_source).await;
    let source_audit = source.audit().await;
    let consumer_cleanup = if let Some(server) = &mut consumer {
        let stopped = server.stop(false).await;
        let audit = server.audit().await;
        stopped.and(audit)
    } else {
        Ok(())
    };
    if observed.is_err()
        || source_stop.is_err()
        || source_audit.is_err()
        || consumer_cleanup.is_err()
    {
        source.retain_on_error().await;
        if let Some(server) = &mut consumer {
            server.retain_on_error().await;
        }
    }
    observed?;
    source_stop?;
    source_audit?;
    consumer_cleanup?;
    guard(&mut source, false)?;
    if let Some(server) = &mut consumer {
        guard(server, false)?;
    }
    Ok(())
}
async fn run(source: &mut Server, consumer: &mut Option<Server>, no_source: bool) -> Result<()> {
    source.ready().await?;
    source
        .track_image(&env::var("RUSTACK_ZIP_BASE_IMAGE")?)
        .await?;
    source.track_image(&process::handler()?).await?;
    let saved = sdk(source, "prepare").await?;
    let checkpoint = source.path()?.join("zip-checkpoint.json");
    let bytes = serde_json::to_vec(&saved)?;
    ensure!(bytes.len() <= 65536, "bounded archive checkpoint");
    fs::write(&checkpoint, bytes).await?;
    source.stop(true).await?;
    source.audit().await?;
    let snapshots = source.path()?.join("snapshots");
    let current = snapshots.join("images");
    if no_source {
        *consumer = Some(Server::spawn(Profile::ZipArchiveNoSource).await?);
        let target = consumer.as_mut().context("image-only archive consumer")?;
        guard(target, true)?;
        eprintln!(
            "provided archive image-only evidence: {}",
            target.path()?.display()
        );
        target.ready().await?;
        ensure!(
            process::get(&target.endpoint, "/_rustack/capabilities")
                .await?
                .1["workers"]["docker"]["zip"]
                == false,
            "negative archive source was installed"
        );
        target.stop(true).await?;
        target.audit().await?;
        let target_snapshots = target.path()?.join("snapshots");
        let target_current = target_snapshots.join("images");
        fs::rename(&target_current, target_snapshots.join(".baseline-empty")).await?;
        copy_tree(&current, &target_current).await?;
        copy_tree(&current, &target_snapshots.join(".images.previous")).await?;
        let before = fingerprint(&target_snapshots).await?;
        target.restart().await?;
        target.failure().await?;
        ensure!(
            fingerprint(&target_snapshots).await? == before,
            "unconfigured ZIP import changed complete archive generations"
        );
    } else {
        let original = serde_json::to_value(archive::read(source.path()?).await?)?;
        let previous = snapshots.join(".images.previous");
        copy_tree(&current, &previous).await?;
        source.restart().await?;
        source.ready().await?;
        ensure!(
            !fs::try_exists(&previous).await?,
            "successful ZIP import kept previous generation"
        );
        let checked = sdk(source, "verify").await?;
        ensure!(
            checked["checkpoint"] == saved["checkpoint"],
            "SDK restored checkpoint changed"
        );
        source.stop(true).await?;
        source.audit().await?;
        ensure!(
            serde_json::to_value(archive::read(source.path()?).await?)? == original,
            "restored ZIP/Image metadata/resource/revision identity changed"
        );
        copy_tree(&current, &previous).await?;
        archive::corrupt_zip(source.path()?).await?;
        let before = fingerprint(&snapshots).await?;
        source.restart().await?;
        source.failure().await?;
        ensure!(
            fingerprint(&snapshots).await? == before,
            "invalid ZIP import changed complete archive generations"
        );
    }
    Ok(())
}
async fn sdk(server: &Server, phase: &str) -> Result<Value> {
    let (owner, engine) = server.observation_identity().await?;
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let arguments = [
        repository
            .join("tests/workflows/provided_zip_archive_sdk.py")
            .into_os_string(),
        OsString::from(&server.endpoint),
        env::var_os("RUSTACK_PROVIDED_ZIP_A").context("original archive ZIP A")?,
        env::var_os("RUSTACK_PROVIDED_ZIP_B").context("original archive ZIP B")?,
        OsString::from(if process::architecture()? == "arm64" {
            "arm64"
        } else {
            "x86_64"
        }),
        OsString::from(process::handler()?),
        OsString::from(phase),
        server.path()?.join("zip-checkpoint.json").into_os_string(),
        env::var_os("RUSTACK_RUNTIME_DOCKER_SOCKET").context("actual archive daemon socket")?,
        OsString::from(owner),
        OsString::from(engine),
    ];
    let mut child = Runner::spawn(&interpreter()?, &arguments)?;
    let entered = child.await_pid().await;
    let report = child
        .wait(if entered.is_ok() {
            Instant::now() + Duration::from_secs(300)
        } else {
            Instant::now()
        })
        .await?;
    let pid = entered?;
    ensure!(
        report.pid == pid && !report.forced && report.status.success(),
        "provided archive SDK failed: {}",
        String::from_utf8_lossy(&report.stderr)
    );
    ensure!(
        report.stdout.len() <= 65536,
        "archive checkpoint output limit"
    );
    let observed: Value = serde_json::from_slice(&report.stdout)?;
    let expected = if phase == "prepare" {
        serde_json::json!([
            "original-ZIP-revisions",
            "mixed-Image",
            "terminal-history-checkpoint"
        ])
    } else {
        ensure!(phase == "verify", "unknown archive SDK phase");
        serde_json::json!([
            "original-ZIP-revisions",
            "mixed-Image",
            "terminal-history-no-replay",
            "cold-warm-after-restart"
        ])
    };
    ensure!(
        observed["schemaVersion"] == 1
            && observed["phase"] == phase
            && observed["cases"] == expected,
        "provided archive semantic inventory differs"
    );
    verify_digests(&observed).await?;
    Ok(observed)
}
