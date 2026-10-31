//! Actual image callback consumers and confirmed-save credential observations.

#[path = "callback/audit.rs"]
mod audit;

use std::{collections::HashSet, env, ffi::OsString, path::PathBuf, time::Duration};

use anyhow::{Context, Result, anyhow, ensure};
use serde::Deserialize;
use serde_json::Value;
use tokio::time::Instant;
use uuid::Uuid;

use super::{
    image_sdk,
    process::{self, Profile, Server},
};

#[derive(Debug, Clone, Copy)]
pub(super) enum Group {
    Causal,
    Timing,
}
impl Group {
    fn name(self) -> &'static str {
        match self {
            Self::Causal => "causal",
            Self::Timing => "timing",
        }
    }
    fn cases(self) -> &'static [&'static str] {
        match self {
            Self::Causal => &["early", "late", "externalFailure", "invocationFailure"],
            Self::Timing => &[
                "retry",
                "heartbeat",
                "absoluteTimeout",
                "heartbeatTimeout",
                "stop",
            ],
        }
    }
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Versions {
    lambda: String,
    sfn: String,
    dynamodb: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Evidence {
    schema_version: u8,
    group: String,
    cases: Vec<String>,
    versions: Versions,
    functions: [String; 2],
    executions: Vec<String>,
    worker_requests: Vec<String>,
    hashes: Vec<String>,
    history_candidates: usize,
    history_bytes: usize,
}
fn evidence(group: Group, observed: &Value) -> Result<Evidence> {
    ensure!(
        serde_json::to_vec(observed)?.len() <= 16 * 1024,
        "callback report byte cap"
    );
    let value: Evidence = serde_json::from_value(observed.clone())
        .map_err(|_| anyhow!("invalid callback report shape"))?;
    let (hashes, workers) = match group {
        Group::Causal => (4, 1),
        Group::Timing => (6, 4),
    };
    ensure!(
        value.schema_version == 1
            && value.group == group.name()
            && value
                .cases
                .iter()
                .map(String::as_str)
                .eq(group.cases().iter().copied())
            && [
                &value.versions.lambda,
                &value.versions.sfn,
                &value.versions.dynamodb
            ]
            .into_iter()
            .all(|version| version == "3.1105.0"),
        "callback semantic/version inventory differs"
    );
    ensure!(
        value
            .functions
            .iter()
            .all(|name| (1..=64).contains(&name.len())
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)))
            && value.functions.iter().collect::<HashSet<_>>().len() == 2,
        "invalid callback function collection"
    );
    ensure!(
        value.executions.len() == 5
            && value.executions.iter().all(|arn| arn.len() <= 256
                && arn.starts_with("arn:aws:states:us-east-1:000000000000:execution:")
                && arn
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_:-".contains(&b)))
            && value.executions.iter().collect::<HashSet<_>>().len() == 5,
        "invalid retained callback execution collection"
    );
    ensure!(
        value.worker_requests.len() == workers
            && value.worker_requests.iter().all(|id| id.len() == 36
                && Uuid::parse_str(id)
                    .is_ok_and(|uuid| uuid.get_version_num() == 4 && uuid.to_string() == *id))
            && value.worker_requests.iter().collect::<HashSet<_>>().len() == workers,
        "invalid completed callback Event collection"
    );
    ensure!(
        value.hashes.len() == hashes
            && value.hashes.iter().collect::<HashSet<_>>().len() == hashes
            && value.hashes.iter().all(|hash| audit::valid_hash(hash))
            && value.history_candidates <= 65_536
            && value.history_bytes <= 16 * 1024 * 1024,
        "invalid callback privacy evidence"
    );
    Ok(value)
}

pub(super) async fn exercise(group: Group) -> Result<()> {
    image_sdk::exercise_with_audit(
        Profile::Callback,
        async |server: &mut Server| {
            let image = env::var("RUSTACK_CALLBACK_IMAGE").context("explicit callback image")?;
            server.track_image(&image).await?;
            let capabilities = server.ready().await?;
            ensure!(
                capabilities["workflows"]["callbacks"] == true
                    && capabilities["workflows"]["lambdaRequestResponse"] == true
                    && capabilities["workflows"]["limits"]["dispatches"] == 1,
                "image callback capability/profile mismatch"
            );
            let (owner, engine) = server.observation_identity().await?;
            Ok(vec![
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../tests/workflows/callback_image_sdk.py")
                    .into_os_string(),
                OsString::from(&server.endpoint),
                OsString::from(image),
                OsString::from(if process::architecture()? == "arm64" {
                    "arm64"
                } else {
                    "x86_64"
                }),
                env::var_os("RUSTACK_RUNTIME_DOCKER_SOCKET").context("callback metadata socket")?,
                OsString::from(owner),
                OsString::from(engine),
                OsString::from(group.name()),
            ])
        },
        |observed| evidence(group, observed).map(|_| ()),
        async |server: &Server, observed: &Value| {
            let input = evidence(group, observed)?;
            let root = server.path()?.to_owned();
            let deadline = Instant::now() + Duration::from_secs(30);
            audit::run(&root, &input, deadline).await?;
            eprintln!(
                "actual image callback SDK versions: 3.1105.0; {}",
                group.name()
            );
            Ok(())
        },
    )
    .await
}
