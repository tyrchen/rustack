//! Ordinary binary qualification; no injected readiness or replacement executor.
#![cfg(all(unix, feature = "lambda", feature = "stepfunctions"))]
#![forbid(unsafe_code)]

#[path = "docker_startup/archive.rs"]
mod archive;
#[path = "docker_startup/callback.rs"]
mod callback;
#[path = "docker_startup/custom.rs"]
mod custom;
#[path = "docker_startup/faults.rs"]
mod faults;
#[path = "docker_startup/handler_sdk.rs"]
mod handler_sdk;
#[path = "docker_startup/image_sdk.rs"]
mod image_sdk;
#[path = "docker_startup/io_faults.rs"]
mod io_faults;
#[path = "docker_startup/node.rs"]
mod node;
#[path = "docker_startup/process.rs"]
mod process;
#[path = "docker_startup/provided.rs"]
mod provided;
#[path = "docker_startup/provided_archive.rs"]
mod provided_archive;
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires explicit official Node image prerequisites; run make test-lambda-node-docker"]
async fn test_should_execute_official_node_image_with_signed_sdks_and_asl() -> Result<()> {
    node::exercise().await
}

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
async fn test_should_restore_real_image_archive_and_preserve_generations_after_rejection()
-> Result<()> {
    let mut server = Server::spawn(Profile::Archive).await?;
    server
        .root
        .as_mut()
        .context("image archive evidence guard")?
        .disable_cleanup(true);
    eprintln!("image archive evidence: {}", server.path()?.display());
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
        let checkpoint = sdk::archive_checkpoint(&server.endpoint).await?;
        server.stop(true).await?;
        server.audit().await?;
        let original = serde_json::to_value(archive::read(server.path()?).await?)?;
        let snapshots = server.path()?.join("snapshots");
        let current = snapshots.join("images");
        let previous = snapshots.join(".images.previous");
        copy_tree(&current, &previous).await?;
        server.restart().await?;
        server.ready().await?;
        ensure!(
            !fs::try_exists(&previous).await?,
            "successful domain import did not retire previous generation"
        );
        sdk::verify_archive(&server.endpoint, &checkpoint).await?;
        server.stop(true).await?;
        server.audit().await?;
        ensure!(
            serde_json::to_value(archive::read(server.path()?).await?)? == original,
            "restored Lambda metadata/resource/revision identity changed"
        );
        copy_tree(&current, &previous).await?;
        archive::corrupt_size(server.path()?).await?;
        let before = fingerprint(&snapshots).await?;
        server.restart().await?;
        server.failure().await?;
        ensure!(
            fingerprint(&snapshots).await? == before,
            "rejected immutable image proof modified current/previous archive generations"
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
    result?;
    server
        .root
        .as_mut()
        .context("image archive evidence guard")?
        .disable_cleanup(false);
    Ok(())
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
    assert!(!Profile::ZipArchiveNoSource.matches_failure("Docker ownership profile mismatch"));
    assert!(!Profile::ZipArchive.matches_failure("Docker startup resource probe failed"));
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

#[rstest]
#[case::typed_faults(faults::Case::Faults)]
#[case::initializing_disconnect(faults::Case::Initializing)]
#[case::busy_disconnect(faults::Case::Busy)]
#[case::accepted_work_shutdown(faults::Case::Saturation)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires real Docker fault fixtures; run make test-lambda-faults-docker"]
async fn test_should_settle_real_image_faults_and_public_cancellation_admission(
    #[case] case: faults::Case,
) -> Result<()> {
    let mut server = Server::spawn(case.profile()).await?;
    server
        .root
        .as_mut()
        .context("fault evidence guard")?
        .disable_cleanup(true);
    eprintln!("fault evidence: {}", server.path()?.display());
    let result = faults::exercise(&mut server, case).await;
    let cleanup = server.stop(case != faults::Case::Saturation).await;
    let audit = server.audit().await;
    let logs = faults::check_logs(&server).await;
    if result.is_err() || cleanup.is_err() || audit.is_err() || logs.is_err() {
        server.retain_on_error().await;
    }
    result?;
    cleanup?;
    audit?;
    logs?;
    server
        .root
        .as_mut()
        .context("fault evidence guard")?
        .disable_cleanup(false);
    Ok(())
}

#[rstest]
#[case::allocating(io_faults::Case::Create)]
#[case::cleanup(io_faults::Case::Delete)]
#[case::daemon_loss(io_faults::Case::Loss)]
#[case::deployment_saturation(io_faults::Case::Saturation)]
#[case::create_io_timeout(io_faults::Case::CreateTimeout)]
#[case::delete_ack_timeout(io_faults::Case::DeleteTimeout)]
#[case::interrupted_pull(io_faults::Case::Pull)]
#[case::negative_images(io_faults::Case::Negative)]
#[case::runtime_http_bounds(io_faults::Case::RuntimeBounds)]
#[case::global_execution_limit(io_faults::Case::GlobalLimit)]
#[case::default_function_limit(io_faults::Case::DefaultLimit)]
#[case::independent_memory_limit(io_faults::Case::MemoryLimit)]
#[case::independent_environment_limit(io_faults::Case::EnvironmentLimit)]
#[case::zip_allocating(io_faults::Case::ZipCreate)]
#[case::zip_cleanup(io_faults::Case::ZipDelete)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires explicit real-daemon IO proxy fixtures; run make test-lambda-io-faults-docker"]
async fn test_should_retain_actual_management_io_through_cancellation_and_deployment_faults(
    #[case] case: io_faults::Case,
) -> Result<()> {
    io_faults::exercise(case).await
}

#[rstest]
#[case::deployments("deployments")]
#[case::faults("faults")]
#[case::initializing("initializing")]
#[case::busy("busy")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires actual provided runtime and original ZIP files; run make test-lambda-docker"]
async fn test_should_execute_ordinary_provided_zip_and_join_owned_retirement(
    #[case] phase: &str,
) -> Result<()> {
    provided::exercise(phase).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires actual ZIP/Image archive startup; run make test-lambda-provided-archive-docker"]
async fn test_should_restore_provided_zip_archives_without_replay_and_preserve_invalid_generations()
-> Result<()> {
    provided_archive::exercise(false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires configured and image-only real startup; run make \
            test-lambda-provided-archive-docker"]
async fn test_should_refuse_provided_zip_archives_without_configured_runtime() -> Result<()> {
    provided_archive::exercise(true).await
}

#[tokio::test]
#[ignore = "requires explicit Docker socket, fixtures and qualified gateway"]
async fn test_should_execute_local_s3_and_dynamodb_from_an_explicit_image_sdk_context() -> Result<()>
{
    handler_sdk::exercise(false).await
}
#[tokio::test]
#[ignore = "requires explicit Docker socket, fixtures and qualified gateway"]
async fn test_should_reject_wrong_image_sdk_credentials_without_ambient_fallback() -> Result<()> {
    handler_sdk::exercise(true).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires explicit callback image, Docker socket and qualified gateway"]
async fn test_should_execute_real_image_callbacks_before_and_after_lambda_response() -> Result<()> {
    callback::exercise(callback::Group::Causal).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires explicit callback image, Docker socket and qualified gateway"]
async fn test_should_enforce_image_callback_retry_heartbeat_timeouts_and_stop() -> Result<()> {
    callback::exercise(callback::Group::Timing).await
}
