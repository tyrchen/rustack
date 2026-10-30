//! Ordinary signed provided runtime operations; original ZIP files are shared with Native.

use std::{env, ffi::OsString, path::PathBuf, time::Duration};

use anyhow::{Context, Result, ensure};
use rustack_lambda_core::storage::compute_sha256;
use serde_json::{Value, json};
use tokio::{
    fs,
    io::AsyncReadExt,
    time::{Instant, timeout},
};

use super::{
    faults::check_logs,
    process::{self, Profile, Server},
    runner::{Runner, interpreter},
};

pub(super) async fn exercise(phase: &str) -> Result<()> {
    let mut server = Server::spawn(if matches!(phase, "initializing" | "busy") {
        Profile::ZipControlled
    } else {
        Profile::Zip
    })
    .await?;
    let result = timeout(Duration::from_secs(360), run(&mut server, phase))
        .await
        .context("ordinary provided runtime observer expired")
        .and_then(|result| result);
    let stopped = server.stop(true).await;
    let audited = server.audit().await;
    let logs = check_logs(&server).await;
    if result.is_err() || stopped.is_err() || audited.is_err() || logs.is_err() {
        server.retain_on_error().await;
    }
    result?;
    stopped?;
    audited?;
    logs
}

async fn run(server: &mut Server, phase: &str) -> Result<()> {
    server.ready().await?;
    server
        .track_image(&env::var("RUSTACK_ZIP_BASE_IMAGE")?)
        .await?;
    let (owner, engine) = server.observation_identity().await?;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let arguments = [
        root.join("tests/workflows/provided_zip_sdk.py")
            .into_os_string(),
        OsString::from(&server.endpoint),
        env::var_os("RUSTACK_PROVIDED_ZIP_A").context("original provided ZIP A")?,
        env::var_os("RUSTACK_PROVIDED_ZIP_B").context("original provided ZIP B")?,
        OsString::from(if process::architecture()? == "arm64" {
            "arm64"
        } else {
            "x86_64"
        }),
        OsString::from(phase),
        env::var_os("RUSTACK_RUNTIME_GATEWAY_HOST").context("explicit hold collector host")?,
        env::var_os("RUSTACK_RUNTIME_DOCKER_SOCKET").context("explicit actual metadata socket")?,
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
        "ordinary provided SDK failed: {}",
        String::from_utf8_lossy(&report.stderr)
    );
    let observed: Value = serde_json::from_slice(&report.stdout)?;
    let expected = match phase {
        "deployments" => json!([
            "cold-warm-readonly",
            "transactional-rejection",
            "immutable-A-B-alias",
            "ASL-Lambda-computation"
        ]),
        "faults" => json!([
            "init-error",
            "handler-exit",
            "handler-timeout",
            "FunctionError-recovery"
        ]),
        "initializing" | "busy" => json!([
            format!("{phase}-causal-disconnect"),
            "physical-retirement",
            "cold-recovery"
        ]),
        _ => anyhow::bail!("unknown provided resource case"),
    };
    ensure!(
        observed["schemaVersion"] == 1 && observed["cases"] == expected,
        "ZIP semantic inventory differs"
    );
    verify_digests(&observed).await
}

async fn verify_digests(observed: &Value) -> Result<()> {
    let digests = observed["zipSha256"]
        .as_array()
        .context("ZIP identity report")?;
    ensure!(
        digests.len() == 2 && digests.first() != digests.get(1),
        "ZIP revision identity differs"
    );
    for (index, key) in ["RUSTACK_PROVIDED_ZIP_A", "RUSTACK_PROVIDED_ZIP_B"]
        .into_iter()
        .enumerate()
    {
        let path = env::var(key)?;
        let metadata = fs::symlink_metadata(&path).await?;
        ensure!(
            metadata.is_file() && metadata.len() <= 50 * 1024 * 1024,
            "bounded original ZIP file"
        );
        let file = fs::File::open(path).await?;
        let mut bytes = Vec::new();
        file.take(50 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .await?;
        ensure!(bytes.len() <= 50 * 1024 * 1024, "original ZIP byte cap");
        let hash = compute_sha256(&bytes);
        ensure!(
            digests.get(index).and_then(Value::as_str) == Some(hash.as_str()),
            "SDK original ZIP digest changed"
        );
        eprintln!("provided original ZIP {index} digest: {hash}");
    }
    Ok(())
}
