//! Explicit container-origin endpoint qualification against a controlled gateway peer.
#![cfg(unix)]
#![forbid(unsafe_code)]

use std::{
    env,
    net::{IpAddr, SocketAddr},
    sync::atomic::Ordering,
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use rstest::rstest;
use rustack_lambda_core::{
    artifact::ImageReference,
    docker::{
        ContainerPolicy, DockerWire, EndpointProbeConfig, EnvironmentProbeConfig,
        EnvironmentProbeError, GatewayInstanceId, NetworkSelection, ProbeFailure, ProbeSettings,
        RelayBudgets, probe_endpoint, probe_environment,
    },
};
use tokio::{
    net::TcpListener,
    time::{Instant, sleep, timeout},
};

mod common;
#[path = "docker_endpoint/gateway.rs"]
mod gateway;
use common::docker::Fixture;
use gateway::Gateway;

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(60)
}
#[rstest]
#[case::matching(0)]
#[case::wrong_instance(1)]
#[case::unavailable_listener(2)]
#[case::abandoned(3)]
#[tokio::test]
#[ignore = "requires explicit Docker socket, pinned runtime, operator bridge and function-visible \
            host"]
async fn test_should_qualify_actual_container_endpoint_and_retain_physical_retirement(
    #[case] scenario: u8,
) -> Result<()> {
    let host: IpAddr = env::var("RUSTACK_RUNTIME_GATEWAY_HOST")
        .context("explicit function-visible host IP required")?
        .parse()?;
    let network: NetworkSelection = env::var("RUSTACK_RUNTIME_NETWORK")
        .context("explicit operator bridge required")?
        .parse()?;
    let source_reference: ImageReference = env::var("RUSTACK_RUNTIME_IMAGE")?.parse()?;
    let mut fixture = Fixture::new().await?;
    let mut source_id = None;
    let evidence = fixture
        .directory
        .as_ref()
        .map(|root| root.path().to_owned());
    let mut gateway = None;
    let result: Result<()> = async {
        source_id = Some(
            fixture
                .docker
                .inspect_image(&source_reference, deadline())
                .await?
                .id
                .context("source identity")?,
        );
        let network_proof = network.qualify(&fixture.docker, deadline()).await?;
        let policy = ContainerPolicy::new(
            network,
            1000,
            1000,
            333_333_333,
            RelayBudgets::new(
                Duration::from_secs(60),
                Duration::from_secs(600),
                Duration::from_secs(60),
            )?,
        )?;
        let base = EnvironmentProbeConfig::builder()
            .directory(fixture.lease.clone())
            .relay(fixture.relay.as_ref().context("pinned runtime")?.clone())
            .environments(fixture.owner.port())
            .policy(policy)
            .network(network_proof)
            .settings(ProbeSettings::new(
                Duration::from_secs(60),
                Duration::from_secs(60),
            )?)
            .build();
        let resources = probe_environment(base.clone(), deadline()).await?;
        ensure!(resources.matches(&base), "resource evidence stale");
        ensure!(
            fixture.owner.port().status().allocated == 0,
            "resource proof before retirement"
        );
        let (port, expected) = if scenario == 2 {
            let reserved = TcpListener::bind("0.0.0.0:0").await?;
            let port = reserved.local_addr()?.port();
            drop(reserved);
            (port, GatewayInstanceId::generate_owned(deadline()).await?)
        } else {
            gateway = Some(Gateway::start(scenario == 3).await?);
            let gateway = gateway.as_ref().context("gateway peer")?;
            (
                gateway.address.port(),
                if scenario == 1 {
                    GatewayInstanceId::generate_owned(deadline()).await?
                } else {
                    gateway.instance
                },
            )
        };
        let endpoint = format!("http://{}", SocketAddr::new(host, port)).parse()?;
        let config = EndpointProbeConfig::new(base.clone(), resources.clone(), endpoint, expected)?;
        if scenario == 3 {
            let job = tokio::spawn(probe_endpoint(config, deadline()));
            let observation: Result<()> = async {
                gateway
                    .as_mut()
                    .context("held gateway")?
                    .observed(deadline())
                    .await?;
                ensure!(
                    fixture.owner.port().status().allocated == 1,
                    "no running endpoint environment"
                );
                Ok(())
            }
            .await;
            job.abort();
            let cancelled = job.await.is_err_and(|error| error.is_cancelled());
            observation?;
            ensure!(cancelled, "probe caller was not cancelled");
            fixture.owner.shutdown(deadline()).await?;
            timeout(Duration::from_secs(10), async {
                while !gateway
                    .as_ref()
                    .is_some_and(|gateway| gateway.closed.load(Ordering::Acquire))
                {
                    sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .context("abandoned HTTP IO not closed")?;
        } else {
            let observed = probe_endpoint(config.clone(), deadline()).await;
            if scenario == 0 {
                let proof = observed?;
                ensure!(
                    proof.matches(&config) && proof.report().instance_id() == expected,
                    "endpoint receipt differs"
                );
                let changed = EndpointProbeConfig::new(
                    base,
                    resources,
                    format!("http://{}", SocketAddr::new(host, port)).parse()?,
                    GatewayInstanceId::generate_owned(deadline()).await?,
                )?;
                ensure!(
                    !proof.matches(&changed),
                    "proof matched another gateway instance"
                );
            } else {
                ensure!(
                    matches!(
                        observed,
                        Err(EnvironmentProbeError::Rejected(ProbeFailure::Outcome))
                    ),
                    "unavailable/wrong gateway did not produce a consumed probe failure"
                );
            }
        }
        ensure!(
            fixture.owner.port().status().allocated == 0,
            "endpoint proof/failure before physical retirement"
        );
        if let Some(gateway) = &gateway {
            ensure!(
                gateway.requests.load(Ordering::Acquire) == 1,
                "endpoint probe replayed"
            );
        }
        Ok(())
    }
    .await;
    let cleanup = fixture.cleanup().await;
    let source_audit: Result<()> = if cleanup.is_ok() && source_id.is_some() {
        async {
            let socket = env::var("RUSTACK_RUNTIME_DOCKER_SOCKET")?;
            let mut wire =
                DockerWire::connect(format!("unix://{socket}").parse()?, deadline()).await?;
            let result: Result<()> = async {
                let port = wire
                    .port()
                    .bind_directory(&fixture.lease, deadline())
                    .await?;
                ensure!(
                    port.inspect_image(&source_reference, deadline())
                        .await?
                        .id
                        .as_deref()
                        == source_id.as_deref(),
                    "source image changed during endpoint cleanup"
                );
                Ok(())
            }
            .await;
            let closed = wire.shutdown().await;
            result?;
            closed?;
            Ok(())
        }
        .await
    } else {
        Ok(())
    };
    let peer_cleanup = match &mut gateway {
        Some(gateway) => gateway.shutdown().await,
        None => Ok(()),
    };
    if result.is_err() || cleanup.is_err() || peer_cleanup.is_err() || source_audit.is_err() {
        if let Some(root) = fixture.directory.take() {
            eprintln!("retained endpoint evidence: {}", root.keep().display());
        } else if let Some(root) = evidence {
            eprintln!("retained endpoint evidence: {}", root.display());
        }
    }
    cleanup?;
    peer_cleanup?;
    source_audit?;
    result
}
