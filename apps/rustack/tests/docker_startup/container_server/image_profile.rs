//! Exact image contracts; release health remains enabled and independently observed.

use anyhow::{Context, Result, ensure};
use rustack_lambda_core::docker::models::{
    ContainerInspectResponse, HealthConfig, HealthStatusEnum,
};
use serde::Deserialize;
use tokio::time::Instant;

use super::cli::{Cli, arguments};

const CONTRACT: &str = r#"{"user":{{json .Config.User}},"workingDir":{{json .Config.WorkingDir}},"entrypoint":{{json .Config.Entrypoint}},"cmd":{{json .Config.Cmd}},"health":{{json .Config.Healthcheck}}}"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImageProfile {
    Fixture,
    Release,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Contract {
    user: String,
    working_dir: String,
    entrypoint: Vec<String>,
    cmd: Option<Vec<String>>,
    health: Option<HealthConfig>,
}

impl ImageProfile {
    pub(super) fn variable(self) -> &'static str {
        match self {
            Self::Fixture => "RUSTACK_SERVER_IMAGE",
            Self::Release => "RUSTACK_RELEASE_IMAGE",
        }
    }
    pub(super) fn configuration(self) -> &'static str {
        match self {
            Self::Fixture => "/etc/rustack/config.yaml",
            Self::Release => "/data/qualification.yaml",
        }
    }
    pub(super) async fn source(self, cli: &Cli, id: &str, cutoff: Instant) -> Result<()> {
        let contract: Contract = cli
            .json(
                &arguments(&["image", "inspect", "--format", CONTRACT, id]),
                cutoff,
            )
            .await?;
        ensure!(
            contract.user == "65532:65532"
                && contract.working_dir == "/data"
                && contract
                    .entrypoint
                    .iter()
                    .map(String::as_str)
                    .eq(["/rustack"])
                && contract.cmd.as_ref().is_none_or(Vec::is_empty),
            "server image execution contract differs"
        );
        self.health(contract.health.as_ref())
    }
    pub(super) fn health(self, health: Option<&HealthConfig>) -> Result<()> {
        let health = health.context("server image healthcheck missing")?;
        let command = health
            .test
            .as_ref()
            .context("server image health command missing")?;
        match self {
            Self::Fixture => ensure!(
                command.iter().map(String::as_str).eq(["NONE"])
                    && [
                        health.interval,
                        health.timeout,
                        health.retries,
                        health.start_period,
                        health.start_interval
                    ]
                    .iter()
                    .all(|v| v.is_none_or(|v| v == 0)),
                "fixture image health differs"
            ),
            Self::Release => ensure!(
                command
                    .iter()
                    .map(String::as_str)
                    .eq(["CMD", "/rustack", "--health-check"])
                    && health.interval == Some(2_000_000_000)
                    && health.timeout == Some(4_000_000_000)
                    && health.start_period == Some(1_000_000_000)
                    && health.retries == Some(3)
                    && health.start_interval.is_none_or(|v| v == 0),
                "release image health differs"
            ),
        }
        Ok(())
    }
    pub(super) fn healthy(self, view: &ContainerInspectResponse) -> bool {
        self == Self::Fixture
            || view
                .state
                .as_ref()
                .and_then(|s| s.health.as_ref())
                .and_then(|h| h.status)
                == Some(HealthStatusEnum::HEALTHY)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{super::cli::decode, *};

    #[test]
    fn test_should_require_exact_release_exec_health_without_shell_or_disable() -> Result<()> {
        let value = json!({"Test":["CMD","/rustack","--health-check"],"Interval":2_000_000_000_i64,
            "Timeout":4_000_000_000_i64,"StartPeriod":1_000_000_000_i64,"Retries":3});
        ImageProfile::Release
            .health(Some(&decode::<HealthConfig>(&serde_json::to_vec(&value)?)?))?;
        for (field, replacement) in [
            ("Test", json!(["NONE"])),
            ("Test", json!(["CMD-SHELL", "/rustack --health-check"])),
            ("Interval", json!(0)),
            ("Timeout", json!(1)),
            ("StartPeriod", json!(0)),
            ("Retries", json!(1)),
            ("StartInterval", json!(1)),
        ] {
            let mut changed = value.clone();
            changed
                .as_object_mut()
                .context("health fixture")?
                .insert(field.into(), replacement);
            assert!(
                ImageProfile::Release
                    .health(Some(&decode::<HealthConfig>(&serde_json::to_vec(
                        &changed
                    )?)?))
                    .is_err()
            );
        }
        assert!(ImageProfile::Release.health(None).is_err());
        Ok(())
    }

    #[test]
    fn test_should_keep_fixture_none_and_require_actual_release_healthy_state() -> Result<()> {
        let none: HealthConfig = decode(br#"{"Test":["NONE"]}"#)?;
        ImageProfile::Fixture.health(Some(&none))?;
        assert!(ImageProfile::Release.health(Some(&none)).is_err());
        for state in ["starting", "unhealthy", "none", ""] {
            let view: ContainerInspectResponse = decode(&serde_json::to_vec(
                &json!({"State":{"Health":{"Status":state}}}),
            )?)?;
            assert!(ImageProfile::Fixture.healthy(&view));
            assert!(!ImageProfile::Release.healthy(&view));
        }
        let view: ContainerInspectResponse =
            decode(br#"{"State":{"Health":{"Status":"healthy"}}}"#)?;
        assert!(ImageProfile::Release.healthy(&view));
        Ok(())
    }
}
