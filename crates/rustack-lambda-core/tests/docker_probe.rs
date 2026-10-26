//! Actual pinned Rust resource probe in disconnected and operator bridge networks.
#![cfg(unix)]
#![forbid(unsafe_code)]

use std::{env, time::Duration};

use anyhow::{Context, Result, ensure};
use rustack_lambda_core::docker::{
    ContainerPolicy, EnvironmentProbeConfig, NetworkSelection, ProbeSettings, RelayBudgets,
    probe_environment,
};
use tokio::time::Instant;

mod common;
use common::docker::Fixture;

#[rstest::rstest]
#[case::disconnected(false)]
#[case::operator_bridge(true)]
#[tokio::test]
#[ignore = "requires explicit Docker runtime image/socket and operator probe bridge"]
async fn test_should_qualify_actual_kernel_limits_and_join_probe_retirement(
    #[case] bridge: bool,
) -> Result<()> {
    let mut fixture = Fixture::new().await?;
    let result = async {
        let selection: NetworkSelection = if bridge {
            env::var("RUSTACK_RUNTIME_NETWORK")
                .ok()
                .filter(|value| !value.is_empty())
                .context("explicit operator probe bridge required")?
                .parse()?
        } else {
            "none".parse()?
        };
        let deadline = Instant::now() + Duration::from_secs(60);
        let network = selection.qualify(&fixture.wire.port(), deadline).await?;
        ensure!(network.disconnected() != bridge, "network mode differs");
        let policy = ContainerPolicy::new(
            selection,
            1000,
            1000,
            333_333_333,
            RelayBudgets::new(
                Duration::from_secs(60),
                Duration::from_secs(600),
                Duration::from_secs(60),
            )?,
        )?;
        let config = EnvironmentProbeConfig::builder()
            .directory(fixture.lease.clone())
            .relay(fixture.relay.as_ref().context("relay pin")?.clone())
            .environments(fixture.owner.port())
            .policy(policy)
            .network(network)
            .settings(ProbeSettings::new(
                Duration::from_secs(60),
                Duration::from_secs(60),
            )?)
            .build();
        let evidence = probe_environment(config.clone(), deadline).await?;
        ensure!(
            evidence.matches(&config),
            "fresh probe does not match retained inputs"
        );
        ensure!(
            fixture.owner.port().status().allocated == 0,
            "proof issued before physical retirement"
        );
        let report = evidence.report();
        ensure!(
            report.memory_max == Some(128 * 1024 * 1024)
                && report.swap_max == Some(0)
                && report.pids_max == Some(256),
            "kernel limits differ"
        );
        ensure!(
            report.cpu_quota == Some(33_333) && report.cpu_period == 100_000,
            "CPU rounding differs"
        );
        ensure!(
            report.euid != 0
                && report.egid != 0
                && report.cap_effective == 0
                && report.no_new_privileges
                && report.seccomp_mode == 2,
            "privilege isolation differs"
        );
        ensure!(
            report.root_readonly
                && report.temporary.bytes == 512 * 1024 * 1024
                && report.temporary.mode == 0o1777,
            "filesystem isolation differs"
        );
        let mut changed = config.clone();
        changed.policy = ContainerPolicy::new(
            "none".parse()?,
            1000,
            1000,
            1_000_000_000,
            RelayBudgets::default(),
        )?;
        ensure!(
            !evidence.matches(&changed),
            "proof matched a different policy"
        );
        Ok(())
    }
    .await;
    let cleanup = fixture.cleanup().await;
    result.and(cleanup)
}
