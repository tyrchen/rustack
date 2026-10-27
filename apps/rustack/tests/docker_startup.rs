//! Ordinary binary qualification; no injected readiness or replacement executor.
#![cfg(all(unix, feature = "lambda", feature = "stepfunctions"))]
#![forbid(unsafe_code)]

#[path = "docker_startup/custom.rs"]
mod custom;
#[path = "docker_startup/process.rs"]
mod process;
#[path = "workflows_sdk/runner.rs"]
mod runner;
#[path = "docker_startup/sdk.rs"]
mod sdk;

use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use process::{Profile, Server};
use rstest::rstest;
use sha2::{Digest, Sha256};
use tokio::{
    fs,
    io::AsyncReadExt,
    time::{Instant, timeout},
};

#[rstest]
#[case::disconnected(Profile::Disconnected)]
#[case::connected(Profile::Connected)]
#[case::missing_relay(Profile::MissingRelay)]
#[case::legacy(Profile::Legacy)]
#[case::wrong_endpoint(Profile::WrongEndpoint)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires explicit two-daemon image prerequisites; run make test-lambda-startup-docker"]
async fn test_should_qualify_ordinary_binary_and_retain_negative_startup_cleanup(
    #[case] profile: Profile,
) -> Result<()> {
    let mut server = Server::spawn(profile).await?;
    let result = timeout(Duration::from_secs(360), async {
        if profile.rejects() {
            server.failure().await?;
            return Ok(());
        }
        server.ready().await?;
        sdk::exercise(
            &server.endpoint,
            &process::handler()?,
            &process::architecture()?,
            profile == Profile::Connected,
            false,
        )
        .await?;
        ensure!(
            process::get(&server.endpoint, "/_rustack/capabilities")
                .await?
                .1["ready"]
                == true,
            "ordinary FunctionError poisoned backend readiness"
        );
        if profile == Profile::Disconnected {
            boto(&server).await?;
            let before = process::get(&server.endpoint, "/_rustack/runtime-probe")
                .await?
                .1;
            let owner =
                fs::read(server.path()?.join("data/.rustack-lambda-docker/owner.v1")).await?;
            server.stop(true).await?;
            server.audit().await?;
            server.restart().await?;
            server.ready().await?;
            let after = process::get(&server.endpoint, "/_rustack/runtime-probe")
                .await?
                .1;
            ensure!(
                before["instanceId"] != after["instanceId"],
                "restart retained stale gateway identity"
            );
            ensure!(
                fs::read(server.path()?.join("data/.rustack-lambda-docker/owner.v1")).await?
                    == owner,
                "restart replaced immutable namespace identity"
            );
        }
        Ok::<(), anyhow::Error>(())
    })
    .await
    .context("ordinary startup SDK observer expired")
    .and_then(|result| result);
    let cleanup = server.stop(!profile.rejects()).await;
    let audit = server.audit().await;
    if result.is_err() || cleanup.is_err() || audit.is_err() {
        server.retain_on_error().await;
    }
    cleanup?;
    audit?;
    result
}
async fn boto(server: &Server) -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let arguments = [
        root.join("tests/workflows/docker_startup_sdk.py")
            .into_os_string(),
        OsString::from(&server.endpoint),
        OsString::from(process::handler()?),
        OsString::from(if process::architecture()? == "arm64" {
            "arm64"
        } else {
            "x86_64"
        }),
    ];
    let mut job = runner::Runner::spawn(&runner::interpreter()?, &arguments)?;
    let barrier = job.await_pid().await;
    let deadline = if barrier.is_ok() {
        Instant::now() + Duration::from_secs(180)
    } else {
        Instant::now()
    };
    let report = job.wait(deadline).await?;
    let pid = barrier?;
    ensure!(
        pid == report.pid && !report.forced && report.status.success(),
        "ordinary boto3 image exercise failed: {}",
        String::from_utf8_lossy(&report.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&report.stdout)?;
    ensure!(
        value["computed"] == 42 && value["warmCounter"] == 2 && value["functionError"] == true,
        "missing boto3 evidence"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires actual production Docker startup; run make test-lambda-startup-docker"]
async fn test_should_reject_metadata_only_image_archive_before_retiring_previous_generation()
-> Result<()> {
    let mut server = Server::spawn(Profile::Archive).await?;
    let result = timeout(Duration::from_secs(360), async {
        server.ready().await?;
        sdk::exercise(
            &server.endpoint,
            &process::handler()?,
            &process::architecture()?,
            false,
            true,
        )
        .await?;
        server.stop(true).await?;
        server.audit().await?;
        let snapshots = server.path()?.join("snapshots");
        let current = snapshots.join("images");
        let previous = snapshots.join(".images.previous");
        copy_tree(&current, &previous).await?;
        let before = fingerprint(&snapshots).await?;
        server.restart().await?;
        server.failure().await?;
        ensure!(
            fingerprint(&snapshots).await? == before,
            "image rejection modified current/previous archive generations"
        );
        Ok::<(), anyhow::Error>(())
    })
    .await
    .context("image archive observer expired")
    .and_then(|result| result);
    let cleanup = server.stop(false).await;
    let audit = server.audit().await;
    if result.is_err() || cleanup.is_err() || audit.is_err() {
        server.retain_on_error().await;
    }
    cleanup?;
    audit?;
    result
}
async fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    ensure!(
        !fs::try_exists(target).await?,
        "previous generation already exists"
    );
    let mut queue = vec![(source.to_owned(), target.to_owned())];
    let mut visited = 0;
    while let Some((source, target)) = queue.pop() {
        visited += 1;
        ensure!(visited <= 128, "fixture directory cap");
        fs::create_dir_all(&target).await?;
        let mut entries = fs::read_dir(&source).await?;
        while let Some(entry) = entries.next_entry().await? {
            let kind = entry.file_type().await?;
            if kind.is_dir() {
                queue.push((entry.path(), target.join(entry.file_name())));
            } else {
                ensure!(
                    kind.is_file(),
                    "fixture refuses symlink/special archive entry"
                );
                fs::copy(entry.path(), target.join(entry.file_name())).await?;
            }
        }
    }
    Ok(())
}
async fn fingerprint(root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>> {
    let mut queue = vec![root.to_owned()];
    let mut result = BTreeMap::new();
    let mut total = 0_usize;
    let mut visited = 0;
    while let Some(directory) = queue.pop() {
        visited += 1;
        ensure!(visited <= 128, "fixture directory cap");
        let mut entries = fs::read_dir(directory).await?;
        while let Some(entry) = entries.next_entry().await? {
            let kind = entry.file_type().await?;
            if kind.is_dir() {
                queue.push(entry.path());
                continue;
            }
            ensure!(
                kind.is_file() && result.len() < 512,
                "fixture archive file cap/kind"
            );
            let mut bytes = Vec::new();
            fs::File::open(entry.path())
                .await?
                .take(16 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .await?;
            total = total
                .checked_add(bytes.len())
                .context("fixture size overflow")?;
            ensure!(
                bytes.len() <= 16 * 1024 * 1024 && total <= 64 * 1024 * 1024,
                "fixture archive byte cap"
            );
            result.insert(
                entry.path().strip_prefix(root)?.to_owned(),
                Sha256::digest(&bytes).to_vec(),
            );
        }
    }
    Ok(result)
}

#[test]
fn test_should_reject_earlier_startup_failure_as_negative_stage_evidence() {
    assert!(
        !Profile::WrongEndpoint.matches_failure("Docker startup probe failed: resource mismatch")
    );
    assert!(
        !Profile::MissingRelay
            .matches_failure("Docker startup daemon failed: Docker response status 404 Not Found")
    );
    assert!(!Profile::MissingRelay.matches_failure(
        "Docker startup relay preparation failed: Docker response status 500 Internal Server Error"
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires explicit custom RIC images and bridge; run make test-lambda-custom-ric-docker"]
async fn test_should_execute_custom_ric_imageconfig_and_immutable_revisions_through_ordinary_service()
-> Result<()> {
    let mut server = Server::spawn(Profile::Connected).await?;
    // Keep alias intent even if the outer test panics or its future is dropped.
    server
        .root
        .as_mut()
        .context("custom fixture evidence")?
        .disable_cleanup(true);
    eprintln!("custom RIC evidence: {}", server.path()?.display());
    let result = custom::exercise(&mut server).await;
    let cleanup = server.stop(true).await;
    let audit = server.audit().await;
    if result.is_err() || cleanup.is_err() || audit.is_err() {
        server.retain_on_error().await;
    }
    result?;
    cleanup?;
    audit?;
    server
        .root
        .as_mut()
        .context("custom fixture evidence")?
        .disable_cleanup(false);
    Ok(())
}
