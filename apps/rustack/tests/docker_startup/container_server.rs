//! Actual Linux server container; normal production readiness and retirement.
#![warn(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

#[path = "container_server/cli.rs"]
mod cli;
#[path = "container_server/image_profile.rs"]
mod image_profile;
#[path = "container_server/logs.rs"]
mod logs;
#[path = "container_server/owner.rs"]
mod owner;
#[path = "container_server/policy.rs"]
mod policy;
#[path = "container_server/records.rs"]
mod records;

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    path::PathBuf,
    time::Duration,
};

use anyhow::{Context, Result, ensure};
pub(super) use image_profile::ImageProfile;
use owner::Owner;
use serde::Deserialize;
use tokio::time::Instant;

use super::{
    image_sdk::join_with_pid_before,
    runner::{Runner, interpreter},
};

#[derive(Debug, Clone, Copy)]
pub(super) enum Mode {
    Sdk,
    Unreachable,
}
impl Mode {
    fn gateway_port(self) -> u16 {
        match self {
            Self::Sdk => 4566,
            Self::Unreachable => 9,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Sdk => "sdk-nested",
            Self::Unreachable => "gateway-rejection",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Versions {
    s3: String,
    dynamodb: String,
    node: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Evidence {
    schema_version: u8,
    cases: Vec<String>,
    versions: Versions,
    computed: Vec<u16>,
    counters: Vec<u8>,
    warm: BTreeMap<String, String>,
    parent_execution: String,
    child_execution: String,
}

pub(super) async fn exercise(mode: Mode, profile: ImageProfile) -> Result<()> {
    let cutoff = Instant::now() + Duration::from_secs(600);
    let mut owner = Owner::new(mode, profile, cutoff).await?;
    let observed = async {
        owner.prepare(cutoff).await?;
        match mode {
            Mode::Sdk => {
                owner.ready(cutoff).await?;
                sdk(&owner, cutoff).await?;
            }
            Mode::Unreachable => owner.rejection(cutoff).await?,
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;
    // No outer timeout drops a live CLI wait or the root's cleanup authority.
    let cleaned = owner.cleanup().await;
    observed?;
    cleaned?;
    if profile == ImageProfile::Release {
        let receipt = owner.publication_receipt(mode.name()).await?;
        println!(
            "WORKFLOWS_RELEASE_RESOURCE {}",
            serde_json::to_string(&receipt)?
        );
    }
    owner.release();
    let topology = if profile == ImageProfile::Release {
        "release-image server-container"
    } else {
        "server-container"
    };
    println!(
        "actual {topology} linux/{}: {}",
        owner.inputs.architecture,
        mode.name()
    );
    Ok(())
}

async fn sdk(owner: &Owner, cutoff: Instant) -> Result<()> {
    let identity = owner.identity()?;
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/workflows/server_container_sdk.py");
    let architecture = if owner.inputs.architecture == "arm64" {
        "arm64"
    } else {
        "x86_64"
    };
    let socket = owner
        .inputs
        .client
        .strip_prefix("unix://")
        .context("client socket prefix")?;
    let args: Vec<OsString> = [
        script.into_os_string(),
        owner.endpoint()?.into(),
        owner.inputs.sdk_image.clone().into(),
        architecture.into(),
        socket.into(),
        identity.owner.to_string().into(),
        identity.engine.to_string().into(),
        format!("http://{}:4566/", owner.expected.name).into(),
        owner.source(&owner.inputs.sdk_image)?.into(),
        owner.source(&owner.inputs.relay)?.into(),
        owner.expected.network.clone().into(),
    ]
    .into();
    let child = Runner::spawn(&interpreter()?, &args)?;
    let (entered, report) = join_with_pid_before(child, cutoff).await?;
    ensure!(
        report.pid == entered.context("server SDK child did not publish its execution PID")?
            && !report.forced,
        "server SDK process failed its execution barrier"
    );
    if !report.status.success() {
        anyhow::bail!(
            "server SDK consumer failed: {} ({})",
            failure_stage(&report.stderr),
            failure_origin(&report.stderr)
        );
    }
    let observed: Evidence = cli::decode(&report.stdout)?;
    validate(&observed)?;
    let actual: BTreeSet<_> = owner.runtime_ids(cutoff).await?.into_iter().collect();
    let reported: BTreeSet<_> = observed.warm.values().cloned().collect();
    ensure!(
        actual == reported && actual.len() == 2,
        "warm server runtime pair did not survive until shutdown"
    );
    Ok(())
}

fn failure_stage(stderr: &[u8]) -> &'static str {
    let Ok(text) = std::str::from_utf8(stderr) else {
        return "invalid diagnostic";
    };
    let Some(stage) = text
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("server-container SDK oracle failed at "))
    else {
        return "unclassified diagnostic";
    };
    match stage {
        "input" => "input",
        "create-data" => "create-data",
        "create-function" => "create-function",
        "direct-first" => "direct-first",
        "direct-second" => "direct-second",
        "nested" => "nested",
        "delete-data" => "delete-data",
        _ => "invalid diagnostic stage",
    }
}

fn failure_origin(stderr: &[u8]) -> String {
    let origin = std::str::from_utf8(stderr)
        .ok()
        .filter(|text| text.len() <= 512 && text.lines().count() == 2)
        .and_then(|text| text.lines().nth(1))
        .and_then(|line| line.strip_prefix("origin "))
        .and_then(|value| value.split_once(':'));
    if let Some((name, line)) = origin {
        let allowed = [
            "server_container_sdk.py",
            "server_container_oracle.py",
            "handler_sdk_image.py",
            "lambda_fault_support.py",
            "stepfunctions_history.py",
            "stepfunctions_nested.py",
        ];
        if allowed.contains(&name)
            && line
                .parse::<u16>()
                .is_ok_and(|number| (1..=10000).contains(&number) && number.to_string() == line)
        {
            return format!("{name}:{line}");
        }
    }
    "unknown source coordinate".to_owned()
}

fn validate(evidence: &Evidence) -> Result<()> {
    ensure!(
        evidence.schema_version == 1
            && evidence.cases.iter().map(String::as_str).eq([
                "direct-sdk-cold",
                "direct-sdk-warm",
                "nested-sync2-sdk",
                "internal-dns-origin",
                "no-server-mounts",
                "warm-for-shutdown"
            ])
            && evidence.versions.s3 == "3.1105.0"
            && evidence.versions.dynamodb == "3.1105.0"
            && evidence.versions.node == "24.22.1"
            && evidence.computed == [42, 46, 50]
            && evidence.counters == [1, 2, 3]
            && evidence.warm.len() == 2
            && evidence.warm.contains_key("function")
            && evidence.warm.contains_key("relay"),
        "server SDK catalog or literal outputs differ"
    );
    let ids = evidence
        .warm
        .values()
        .map(|id| cli::container_id(id))
        .collect::<Result<BTreeSet<_>>>()?;
    ensure!(
        ids.len() == 2,
        "server SDK reported duplicate environment IDs"
    );
    execution(&evidence.parent_execution, "ServerParent")?;
    execution(&evidence.child_execution, "ServerChild")?;
    ensure!(
        evidence.parent_execution != evidence.child_execution,
        "nested execution identity reused"
    );
    Ok(())
}

fn execution(value: &str, expected: &str) -> Result<()> {
    ensure!(value.len() <= 256, "execution ARN byte bound");
    let tail = value
        .strip_prefix("arn:aws:states:us-east-1:000000000000:execution:")
        .context("execution ARN scope")?;
    let (name, run) = tail.split_once(':').context("execution ARN resource")?;
    ensure!(
        name.starts_with(expected)
            && [name, run].iter().all(|part| (1..=80).contains(&part.len())
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))),
        "execution ARN name grammar"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_reject_invalid_daemon_socket_and_gid_before_cli_dispatch() -> Result<()> {
        for source in [
            "",
            "/",
            "relative/docker.sock",
            "/run//docker.sock",
            "/run/../docker.sock",
            "/run/./docker.sock",
            "/run/docker.sock/",
            "/run/a,b",
            "/run/a\n",
        ] {
            assert!(policy::socket_source(source).is_err());
        }
        policy::socket_source("/run/user/1000/docker.sock")?;
        for gid in ["", "-1", "01", "2147483648", "4294967296", "991\n"] {
            assert!(policy::socket_gid(gid).is_err());
        }
        assert_eq!(policy::socket_gid("0")?, 0);
        assert_eq!(policy::socket_gid("991")?, 991);
        Ok(())
    }

    #[test]
    fn test_should_never_project_untrusted_sdk_diagnostics() {
        assert_eq!(failure_origin(b"server-container SDK oracle failed at direct-first\norigin server_container_oracle.py:50\n"), "server_container_oracle.py:50");
        for origin in [
            "local-secret:50",
            "server_container_oracle.py:local-secret",
            "server_container_oracle.py:050",
            "server_container_oracle.py:0",
            "server_container_oracle.py:10001",
            "server_container_oracle.py:50\nlocal-secret",
        ] {
            let diagnostic =
                format!("server-container SDK oracle failed at nested\norigin {origin}\n");
            assert_eq!(
                failure_origin(diagnostic.as_bytes()),
                "unknown source coordinate"
            );
        }
        assert_eq!(
            failure_stage(b"server-container SDK oracle failed at nested\n"),
            "nested"
        );
        for bytes in [
            b"raw-private-diagnostic".as_slice(),
            b"server-container SDK oracle failed at arbitrary\n",
            b"\xff",
        ] {
            assert!(!["raw-private-diagnostic", "arbitrary"].contains(&failure_stage(bytes)));
        }
    }
}
