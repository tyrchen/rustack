//! Actual composite-Action YAML contract and bounded control metadata export.
#![cfg(unix)]
#![forbid(unsafe_code)]
#![warn(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::Permissions,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
    time::{Duration, Instant, timeout},
};
use yaml_rust2::{Yaml, YamlLoader};

#[path = "workflows_sdk/runner.rs"]
mod runner;
use runner::{Runner, interpreter};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ActionContract {
    schema_version: u8,
    defaults: BTreeMap<String, String>,
    environment: BTreeMap<String, String>,
    run: String,
}

fn field<'a>(value: &'a Yaml, name: &str) -> Result<&'a Yaml> {
    value
        .as_hash()
        .context("Action mapping required")?
        .get(&Yaml::String(name.to_owned()))
        .context("Action field missing")
}

fn string(value: &Yaml, maximum: usize) -> Result<String> {
    let value = value.as_str().context("Action string required")?;
    ensure!(value.len() <= maximum, "Action string budget");
    Ok(value.to_owned())
}

fn contract(source: &str) -> Result<ActionContract> {
    ensure!(source.len() <= 131_072, "Action source byte budget");
    let documents = YamlLoader::load_from_str(source).context("Action YAML")?;
    let [root]: [Yaml; 1] = documents
        .try_into()
        .map_err(|_| anyhow::anyhow!("one Action document required"))?;
    let inputs = field(&root, "inputs")?
        .as_hash()
        .context("Action inputs mapping")?;
    ensure!(inputs.len() <= 32, "Action input count budget");
    let mut defaults = BTreeMap::new();
    for (name, definition) in inputs {
        defaults.insert(
            string(name, 64)?,
            string(field(definition, "default")?, 512)?,
        );
    }
    let runs = field(&root, "runs")?;
    ensure!(
        string(field(runs, "using")?, 32)? == "composite",
        "composite Action required"
    );
    let steps = field(runs, "steps")?.as_vec().context("Action steps")?;
    ensure!(steps.len() == 1, "one retained Action entrypoint required");
    let step = steps.first().context("Action step")?;
    ensure!(
        string(field(step, "id")?, 64)? == "start",
        "public output step changed"
    );
    ensure!(
        string(field(step, "shell")?, 32)? == "bash",
        "Action shell changed"
    );
    let run = string(field(step, "run")?, 16_384)?;
    ensure!(
        run.trim() == "exec python3 -B \"$RUSTACK_ACTION_ROOT/tools/github-action/runtime.py\""
            && !run.contains("${{"),
        "Action source must not interpolate context"
    );
    let env = field(step, "env")?
        .as_hash()
        .context("Action environment")?;
    ensure!(env.len() <= 128, "Action environment budget");
    let mut environment = BTreeMap::new();
    for (name, value) in env {
        environment.insert(string(name, 96)?, string(value, 512)?);
    }
    ensure!(
        environment.get("RUSTACK_ACTION_ROOT").map(String::as_str)
            == Some("${{ github.action_path }}"),
        "trusted Action root binding missing"
    );
    ensure!(
        environment.len() == defaults.len() + 1,
        "Action input/environment drift"
    );
    for key in defaults.keys() {
        let name = format!("RUSTACK_ACTION_{}", key.to_uppercase().replace('-', "_"));
        let expression = format!("${{{{ inputs.{key} }}}}");
        ensure!(
            environment.get(&name) == Some(&expression),
            "Action input env binding changed"
        );
    }
    let outputs = field(&root, "outputs")?;
    for (name, value) in [
        ("endpoint", "${{ steps.start.outputs.endpoint }}"),
        ("container-id", "${{ steps.start.outputs.container_id }}"),
    ] {
        ensure!(
            string(field(field(outputs, name)?, "value")?, 256)? == value,
            "Action output contract changed"
        );
    }
    Ok(ActionContract {
        schema_version: 1,
        defaults,
        environment,
        run,
    })
}

async fn read_contract() -> Result<ActionContract> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = timeout(Duration::from_secs(5), async {
        let file = fs::File::open(root.join("action.yml")).await?;
        ensure!(
            file.metadata().await?.len() <= 131_072,
            "Action source byte budget"
        );
        let mut bytes = Vec::new();
        file.take(131_073).read_to_end(&mut bytes).await?;
        ensure!(bytes.len() <= 131_072, "Action source grew beyond budget");
        Ok::<_, anyhow::Error>(String::from_utf8(bytes)?)
    })
    .await
    .context("Action source IO deadline")??;
    contract(&source)
}

#[tokio::test]
async fn test_should_validate_actual_action_entrypoint_contract() -> Result<()> {
    let value = read_contract().await?;
    ensure!(
        value.defaults.contains_key("stepfunctions-skip-auth"),
        "Step Functions Action setting missing"
    );
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ControlReport {
    schema_version: u8,
    cases: u32,
    successful: bool,
}

async fn private_write(path: &Path, body: &[u8]) -> Result<()> {
    ensure!(body.len() <= 131_072, "Action control artifact byte budget");
    timeout(Duration::from_secs(5), async {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true).mode(0o600);
        let mut file = options.open(path).await?;
        file.write_all(body).await?;
        file.sync_all().await?;
        Ok::<_, anyhow::Error>(())
    })
    .await
    .context("Action private artifact deadline")??;
    Ok(())
}

#[tokio::test]
#[ignore = "requires provisioned pinned Python; run make test-workflows-action-fixture"]
async fn test_should_execute_actual_action_process_controls() -> Result<()> {
    let mut root: TempDir = tempfile::tempdir()?;
    root.disable_cleanup(true);
    timeout(
        Duration::from_secs(5),
        fs::set_permissions(root.path(), Permissions::from_mode(0o700)),
    )
    .await
    .context("Action control root permissions deadline")??;
    ensure!(
        fs::metadata(root.path()).await?.permissions().mode() & 0o777 == 0o700,
        "Action controls require a private directory"
    );
    let value = read_contract().await?;
    let metadata = root.path().join("action.json");
    private_write(&metadata, &serde_json::to_vec(&value)?).await?;
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/workflows/test_github_action.py");
    let arguments: Vec<OsString> = vec![script.into_os_string(), metadata.into_os_string()];
    let cutoff = Instant::now() + Duration::from_secs(180);
    let mut child = Runner::spawn(&interpreter()?, &arguments)?;
    let entered = child.await_pid().await;
    let report = child.wait(cutoff).await?;
    private_write(&root.path().join("stdout"), &report.stdout).await?;
    private_write(&root.path().join("stderr"), &report.stderr).await?;
    ensure!(
        entered.is_ok_and(|pid| pid == report.pid) && !report.forced && report.status.success(),
        "Action controls failed; private evidence {} (stdout SHA256-base64 {}, stderr \
         SHA256-base64 {})",
        root.path().display(),
        STANDARD.encode(Sha256::digest(&report.stdout)),
        STANDARD.encode(Sha256::digest(&report.stderr))
    );
    let observed: ControlReport =
        serde_json::from_slice(&report.stdout).context("Action control result schema")?;
    ensure!(
        observed.schema_version == 1 && observed.successful && (24..=64).contains(&observed.cases),
        "Action control catalog incomplete"
    );
    println!(
        "Actual composite Action controls: {} cases; independent Rust PID/capture/timeout owner \
         joined",
        observed.cases
    );
    root.disable_cleanup(false);
    Ok(())
}
