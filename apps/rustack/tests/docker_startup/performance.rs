//! Real signed image measurements, published only after physical server cleanup.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use std::{env, ffi::OsString, path::PathBuf, time::Duration};

use anyhow::{Context, Result, ensure};
use serde_json::Value;
use tokio::{
    fs,
    io::AsyncReadExt,
    time::{Instant, timeout},
};

use super::{
    image_sdk,
    process::{Profile, Server, architecture},
    runner::{Runner, interpreter},
};

pub(super) async fn exercise(phase: &'static str) -> Result<()> {
    image_sdk::exercise_with_audit(
        Profile::Performance,
        async move |server| prepare(server, phase).await,
        move |observed| validate(observed, phase),
        async move |server, observed| audit(server, observed, phase).await,
    )
    .await
}

async fn prepare(server: &mut Server, phase: &str) -> Result<Vec<OsString>> {
    server.ready().await?;
    let baseline = read_log(server.path()?.join("server.log")).await?;
    ensure!(
        baseline.len() <= 32768 && baseline.ends_with(b"\n"),
        "initialization baseline bound or framing"
    );
    timeout(
        Duration::from_secs(5),
        fs::write(server.path()?.join("initialization-baseline.log"), baseline),
    )
    .await
    .context("initialization baseline write expired")??;
    let source = env::var("RUSTACK_CUSTOM_IMAGE_A")?;
    server.track_image(&source).await?;
    let (owner, engine) = server.observation_identity().await?;
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/workflows/image_performance_sdk.py");
    Ok(vec![
        script.into_os_string(),
        OsString::from(&server.endpoint),
        OsString::from(source),
        OsString::from(if architecture()? == "arm64" {
            "arm64"
        } else {
            "x86_64"
        }),
        OsString::from(phase),
        env::var_os("RUSTACK_RUNTIME_DOCKER_SOCKET").context("explicit metadata socket")?,
        OsString::from(owner),
        OsString::from(engine),
        env::var_os("RUSTACK_RUNTIME_GATEWAY_HOST").context("explicit collector gateway")?,
        OsString::from(server.process_id()?.to_string()),
        OsString::from("v1"),
        env::var_os("RUSTACK_WORKFLOWS_SDK_SITE_PACKAGES").unwrap_or_else(|| OsString::from("-")),
    ])
}

fn validate(observed: &Value, phase: &str) -> Result<()> {
    ensure!(
        observed.get("schemaVersion") == Some(&Value::from(1))
            && observed.get("phase").and_then(Value::as_str) == Some(phase)
            && observed.get("emptyAfterWorkload") == Some(&Value::Bool(true))
            && observed
                .get("referenceQualification")
                .and_then(Value::as_str)
                == Some("pending"),
        "image performance report identity or cleanup differs"
    );
    Ok(())
}

async fn audit(server: &Server, observed: &Value, phase: &str) -> Result<()> {
    let log = read_log(server.path()?.join("server.log")).await?;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/workflows/image_performance_report.py");
    let directory = server.path()?;
    let input = directory.join("image-performance.json");
    let mut observed = observed.clone();
    observed
        .as_object_mut()
        .context("measurement report object")?
        .insert(
            "debugAssertions".to_owned(),
            Value::Bool(cfg!(debug_assertions)),
        );
    timeout(
        Duration::from_secs(5),
        fs::write(&input, serde_json::to_vec(&observed)?),
    )
    .await
    .context("measurement evidence write expired")??;
    let child = Runner::spawn(
        &interpreter()?,
        &[
            root.into_os_string(),
            input.into_os_string(),
            directory.join("server.log").into_os_string(),
            OsString::from(phase),
            directory
                .join("initialization-baseline.log")
                .into_os_string(),
        ],
    )?;
    let (entered, report) =
        image_sdk::join_with_pid_before(child, Instant::now() + Duration::from_secs(10)).await?;
    ensure!(
        report.pid == entered? && !report.forced && report.status.success(),
        "independent image measurement validation failed: {}",
        String::from_utf8_lossy(&report.stderr)
    );
    let qualified: Value = serde_json::from_slice(&report.stdout)?;
    ensure!(!log.is_empty(), "missing actual initialization log");
    println!(
        "WORKFLOWS_IMAGE_PERFORMANCE {}",
        serde_json::to_string(&qualified)?
    );
    Ok(())
}

async fn read_log(path: PathBuf) -> Result<Vec<u8>> {
    timeout(Duration::from_secs(5), async {
        let file = fs::File::open(path).await?;
        let mut bytes = Vec::new();
        file.take(32769).read_to_end(&mut bytes).await?;
        ensure!(bytes.len() <= 32768, "initialization log evidence bound");
        Ok(bytes)
    })
    .await
    .context("initialization log read expired")?
}
