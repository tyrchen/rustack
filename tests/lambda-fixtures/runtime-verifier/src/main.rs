//! Actual production Native provider inside an explicitly owned Linux verifier.
#![forbid(unsafe_code)]

use std::{
    env::consts::{ARCH, OS},
    io::ErrorKind,
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use rustack_lambda_core::{
    config::LambdaConfig,
    executor::ExecutorBackend,
    provider::{InvokeKind, InvokeOutcome, RustackLambda},
    storage::{FunctionStore, compute_sha256},
};
use rustack_lambda_model::{
    input::CreateFunctionInput,
    types::{Environment, FunctionCode},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{
    fs,
    io::AsyncReadExt,
    time::{Instant, timeout, timeout_at},
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    architecture: String,
    zip_sha256: String,
    code_marker: String,
}
impl Manifest {
    fn validate(&self) -> Result<()> {
        let architecture = match ARCH {
            "aarch64" => "arm64",
            "x86_64" => "x86_64",
            _ => bail!("unsupported Native verifier architecture"),
        };
        ensure!(
            OS == "linux" && self.architecture == architecture,
            "Native verifier platform mismatch"
        );
        ensure!(self.zip_sha256.len() == 44, "ZIP digest length");
        let digest = STANDARD
            .decode(&self.zip_sha256)
            .context("decode ZIP digest")?;
        ensure!(
            digest.len() == 32 && STANDARD.encode(digest) == self.zip_sha256,
            "noncanonical ZIP digest"
        );
        ensure!(
            (1..=16).contains(&self.code_marker.len())
                && self.code_marker.bytes().all(|b| b.is_ascii_alphanumeric()),
            "revision marker bound"
        );
        Ok(())
    }
}
async fn read(path: &str, maximum: u64) -> Result<Vec<u8>> {
    timeout(Duration::from_secs(30), async {
        let metadata = fs::symlink_metadata(path)
            .await
            .context("inspect fixed input")?;
        ensure!(
            metadata.is_file() && metadata.len() <= maximum,
            "input must be a bounded regular file"
        );
        let file = fs::File::open(path).await.context("open fixed input")?;
        let mut bytes = Vec::new();
        file.take(maximum.checked_add(1).context("input bound overflow")?)
            .read_to_end(&mut bytes)
            .await
            .context("read fixed input")?;
        ensure!(u64::try_from(bytes.len())? <= maximum, "input byte limit");
        Ok(bytes)
    })
    .await
    .context("input observation expired")?
}
async fn call(provider: &RustackLambda, n: i64) -> Result<Value> {
    let event = serde_json::to_vec(&json!({"n":n}))?;
    let result = provider
        .invoke("native-zip", None, &event, InvokeKind::RequestResponse)
        .await
        .context("invoke actual Native code")?;
    let InvokeOutcome::Sync(result) = result else {
        bail!("expected synchronous Native result");
    };
    ensure!(
        result.function_error.is_none(),
        "Native handler reported failure"
    );
    serde_json::from_slice(&result.payload).context("decode computed Native response")
}
async fn exercise(
    provider: &RustackLambda,
    manifest: &Manifest,
    zip: Vec<u8>,
) -> Result<Vec<Value>> {
    let code = FunctionCode {
        zip_file: Some(STANDARD.encode(zip)),
        ..Default::default()
    };
    let created = provider
        .create_function(CreateFunctionInput {
            function_name: "native-zip".into(),
            runtime: Some("provided.al2023".into()),
            handler: Some("handler.entry".into()),
            role: "arn:aws:iam::000000000000:role/test-role".into(),
            code,
            architectures: Some(vec![manifest.architecture.clone()]),
            timeout: Some(20),
            environment: Some(Environment {
                variables: Some(
                    [
                        ("WORKFLOW_MODE".into(), "1".into()),
                        ("PROVIDED_QUALIFICATION".into(), "1".into()),
                    ]
                    .into_iter()
                    .collect(),
                ),
            }),
            ..Default::default()
        })
        .await
        .context("deploy exact ZIP through Native provider")?;
    ensure!(
        created.code_sha256.as_deref() == Some(manifest.zip_sha256.as_str()),
        "Native code identity changed"
    );
    let cold = call(provider, 21).await?;
    let warm = call(provider, 23).await?;
    ensure!(
        cold.get("computed") == Some(&json!(42)) && warm.get("computed") == Some(&json!(46)),
        "Native computation mismatch"
    );
    ensure!(
        cold.get("providedCounter") == Some(&json!(1))
            && warm.get("providedCounter") == Some(&json!(2)),
        "Native cold/warm reuse mismatch"
    );
    ensure!(
        cold.get("codeMarker").and_then(Value::as_str) == Some(manifest.code_marker.as_str())
            && warm.get("codeMarker") == cold.get("codeMarker"),
        "Native revision mismatch"
    );
    let pid = cold
        .get("pid")
        .and_then(Value::as_u64)
        .context("Native child PID")?;
    ensure!(
        (2..(1_u64 << 31)).contains(&pid) && warm.get("pid").and_then(Value::as_u64) == Some(pid),
        "Native warm child identity mismatch"
    );
    Ok(vec![cold, warm])
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let manifest: Manifest = serde_json::from_slice(&read("/var/task/manifest.json", 4096).await?)
        .context("decode bounded manifest")?;
    manifest.validate()?;
    let zip = read("/var/task/input.zip", 50 * 1024 * 1024).await?;
    ensure!(
        compute_sha256(&zip) == manifest.zip_sha256,
        "original ZIP digest mismatch"
    );
    let provider = RustackLambda::with_store(
        FunctionStore::new("/tmp/native-store"),
        LambdaConfig {
            executor: ExecutorBackend::Native,
            max_warm_instances: 1,
            ..Default::default()
        },
    );
    let observed = timeout_at(
        Instant::now() + Duration::from_secs(90),
        exercise(&provider, &manifest, zip),
    )
    .await
    .context("Native operation observation expired")
    .and_then(|result| result);
    let cleanup = provider
        .shutdown_before(Instant::now() + Duration::from_secs(30))
        .await
        .context("join actual Native children and IO");
    cleanup?;
    let observed = observed?;
    let pid = observed
        .first()
        .and_then(|v| v.get("pid"))
        .and_then(Value::as_u64)
        .context("observed Native child")?;
    let absence = timeout(Duration::from_secs(5), fs::metadata(format!("/proc/{pid}")))
        .await
        .context("Native child absence observation expired")?;
    ensure!(
        matches!(absence, Err(error) if error.kind() == ErrorKind::NotFound),
        "Native child still exists after joined cleanup"
    );
    println!(
        "{}",
        json!({"schemaVersion":1,"kind":"native-same-zip","zipSha256":manifest.zip_sha256,"joined":true,"observed":observed})
    );
    Ok(())
}
