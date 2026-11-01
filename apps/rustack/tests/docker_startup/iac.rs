//! Actual Pulumi provider lifecycle and whole-function plugin ownership.

use std::{collections::HashSet, env, ffi::OsString, path::PathBuf};

use anyhow::{Context, Result, anyhow, ensure};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use super::{
    image_sdk,
    process::{self, Profile, Server},
};

const COMMANDS: [&str; 18] = [
    "version",
    "login",
    "init",
    "create",
    "refresh-created",
    "update",
    "refresh-updated",
    "reject-publish",
    "recover-publish",
    "settle-publish",
    "reject-logging",
    "recover-logging",
    "settle-logging",
    "reject-encryption",
    "recover-encryption",
    "settle-encryption",
    "destroy",
    "remove",
];
const STAGES: [(&str, u8); 13] = [
    ("created", 4),
    ("refreshed-created", 5),
    ("updated", 6),
    ("refreshed-updated", 7),
    ("rejected-publish", 8),
    ("recovered-publish", 10),
    ("rejected-logging", 11),
    ("recovered-logging", 13),
    ("rejected-encryption", 14),
    ("recovered-encryption", 16),
    ("pre-destroy", 16),
    ("post-destroy", 17),
    ("post-removal", 18),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Lifecycle,
    Stop,
}
impl Mode {
    fn name(self) -> &'static str {
        match self {
            Self::Lifecycle => "lifecycle",
            Self::Stop => "stop",
        }
    }
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Versions {
    cli: String,
    provider: String,
    sdk: String,
    aws: String,
    node: String,
    dynamodb: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Stage {
    stage: String,
    ordinal: u8,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Evidence {
    schema_version: u8,
    mode: String,
    versions: Versions,
    stages: Vec<Stage>,
    audits: Vec<String>,
    commands: Vec<String>,
    executions: Vec<String>,
    business_requests: Vec<String>,
    request_id: String,
    physical_stop: bool,
    actual_destroy: bool,
}

fn validate(mode: Mode, observed: &Value) -> Result<()> {
    ensure!(
        serde_json::to_vec(observed)?.len() <= 16 * 1024,
        "IaC report cap"
    );
    let value: Evidence = serde_json::from_value(observed.clone())
        .map_err(|_| anyhow!("invalid managed IaC report"))?;
    let positive = mode == Mode::Lifecycle;
    ensure!(
        value.schema_version == 1
            && value.mode == mode.name()
            && value.physical_stop != positive
            && value.actual_destroy == positive
            && value.versions.cli == "3.273.0"
            && value.versions.sdk == "3.273.0"
            && value.versions.provider == "7.52.1"
            && value.versions.aws == "7.52.1"
            && value.versions.node == "24.22.1"
            && value.versions.dynamodb == "3.1105.0",
        "IaC semantic or source versions differ"
    );
    let stages = STAGES
        .get(..if positive { STAGES.len() } else { 1 })
        .context("literal IaC stage catalog")?;
    let commands = COMMANDS
        .get(..if positive { COMMANDS.len() } else { 4 })
        .context("literal IaC command catalog")?;
    let audits: &[&str] = if positive {
        &["pre-destroy", "post-destroy", "post-removal"]
    } else {
        &[]
    };
    ensure!(
        value.stages.len() == stages.len()
            && value
                .stages
                .iter()
                .map(|row| (row.stage.as_str(), row.ordinal))
                .eq(stages.iter().copied())
            && value
                .commands
                .iter()
                .map(String::as_str)
                .eq(commands.iter().copied())
            && value
                .audits
                .iter()
                .map(String::as_str)
                .eq(audits.iter().copied()),
        "actual IaC lifecycle catalog differs"
    );
    ensure!(
        value.request_id.len() == 36
            && Uuid::parse_str(&value.request_id)
                .is_ok_and(|id| id.get_version_num() == 4 && id.to_string() == value.request_id)
            && value.business_requests.len() == if positive { 10 } else { 3 }
            && value.business_requests.iter().collect::<HashSet<_>>().len()
                == value.business_requests.len()
            && value.business_requests.iter().all(|id| id.len() == 36
                && Uuid::parse_str(id)
                    .is_ok_and(|uuid| uuid.get_version_num() == 4 && uuid.to_string() == *id))
            && value.executions.len() == if positive { 8 } else { 1 }
            && value.executions.iter().collect::<HashSet<_>>().len() == value.executions.len()
            && value
                .executions
                .iter()
                .all(|arn| (1..=256).contains(&arn.len())
                    && arn
                        .starts_with("arn:aws:states:us-east-1:000000000000:execution:IacTarget")
                    && arn
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"_:-".contains(&byte))),
        "invalid IaC invocation or independent execution identities"
    );
    eprintln!(
        "actual managed IaC CLI3.273.0/provider7.52.1: {}",
        mode.name()
    );
    Ok(())
}

pub(super) async fn exercise(mode: Mode) -> Result<()> {
    image_sdk::exercise(
        Profile::Iac,
        async |server: &mut Server| {
            let tool = env::var("RUSTACK_PULUMI_IMAGE").context("explicit Pulumi image")?;
            let business = env::var("RUSTACK_NODE_IMAGE").context("explicit business image")?;
            server.track_image(&tool).await?;
            server.track_image(&business).await?;
            let capabilities = server.ready().await?;
            ensure!(
                capabilities["workflows"]["lambdaRequestResponse"] == true
                    && capabilities["workflows"]["limits"]["dispatches"] == 2,
                "IaC capability/profile mismatch"
            );
            let (owner, engine) = server.observation_identity().await?;
            Ok(vec![
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../tests/workflows/pulumi_image_sdk.py")
                    .into_os_string(),
                OsString::from(&server.endpoint),
                OsString::from(tool),
                OsString::from(business),
                OsString::from(if process::architecture()? == "arm64" {
                    "arm64"
                } else {
                    "x86_64"
                }),
                env::var_os("RUSTACK_RUNTIME_DOCKER_SOCKET").context("IaC metadata socket")?,
                OsString::from(owner),
                OsString::from(engine),
                OsString::from(mode.name()),
                server.path()?.as_os_str().to_owned(),
            ])
        },
        |observed| validate(mode, observed),
    )
    .await
}
