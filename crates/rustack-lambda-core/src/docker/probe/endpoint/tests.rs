use std::time::Duration;

use rstest::rstest;
use rustack_lambda_runtime::protocol::AttemptId;
use serde_json::{json, to_vec};
use tokio::time::Instant;
use uuid::Uuid;

use super::{EndpointProbe, EndpointProbeConfig, verify_report};
use crate::{
    artifact::Architecture,
    docker::{
        ContainerPolicy, DirectoryLease, DockerWire, EnvironmentConfig, EnvironmentManager,
        EnvironmentProbe, EnvironmentProbeConfig, EnvironmentProbeError, GatewayInstanceId,
        NetworkSelection, RelayBudgets, plan::tests::image_with, probe::tests::report,
        probe_endpoint, tests::Daemon,
    },
};

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
fn report_bytes(attempt: AttemptId, id: GatewayInstanceId) -> Vec<u8> {
    to_vec(&json!({"version":1,"attempt":attempt,"architecture":"arm64","instanceId":id})).unwrap()
}
#[rstest]
#[case::attempt(0)]
#[case::architecture(1)]
#[case::instance(2)]
#[case::version(3)]
#[case::unknown(4)]
#[test]
fn test_should_require_exact_endpoint_report_schema_attempt_architecture_and_instance(
    #[case] case: u8,
) {
    let attempt = AttemptId::default();
    let id = GatewayInstanceId::generate().unwrap();
    let mut value = json!({"version":1,"attempt":attempt,"architecture":"arm64","instanceId":id});
    match case {
        0 => value["attempt"] = json!(AttemptId::default()),
        1 => value["architecture"] = json!("x86_64"),
        2 => value["instanceId"] = json!(GatewayInstanceId::generate().unwrap()),
        3 => value["version"] = json!(2),
        _ => value["extra"] = json!("PRIVATE-PEER"),
    }
    let error =
        verify_report(&to_vec(&value).unwrap(), attempt, Architecture::Arm64, id).unwrap_err();
    assert!(!format!("{error:?}:{error}").contains("PRIVATE-PEER"));
}
#[test]
fn test_should_decode_bounded_endpoint_report_without_turning_data_into_a_receipt() {
    let attempt = AttemptId::default();
    let id = GatewayInstanceId::generate().unwrap();
    let bytes = report_bytes(attempt, id);
    let report = verify_report(&bytes, attempt, Architecture::Arm64, id).unwrap();
    assert_eq!(report.instance_id(), id);
    assert!(verify_report(&vec![b' '; 4097], attempt, Architecture::Arm64, id).is_err());
    let duplicate = format!(
        "{{\"version\":1,\"version\":1,\"attempt\":\"{attempt}\",\"architecture\":\"arm64\",\"\
         instanceId\":{}}}",
        serde_json::to_string(&id).unwrap()
    );
    assert!(verify_report(duplicate.as_bytes(), attempt, Architecture::Arm64, id).is_err());
}
async fn fixture() -> (Daemon, DockerWire, EnvironmentManager, EndpointProbeConfig) {
    let mut daemon = Daemon::new();
    let wire = daemon.wire().await;
    let directory = DirectoryLease::testing(Uuid::new_v4(), Uuid::new_v4());
    let owner =
        EnvironmentManager::start(wire.port(), directory.clone(), EnvironmentConfig::default())
            .unwrap();
    let selection: NetworkSelection = "endpoint-fixture".parse().unwrap();
    let port = wire.port();
    let (network, ()) = tokio::join!(selection.qualify(&port, deadline()), async {
        daemon.next().await.reply(200,&to_vec(&json!({"Name":"endpoint-fixture","Id":"a".repeat(64),"Driver":"bridge","Scope":"local","Ingress":false,"ConfigOnly":false})).unwrap()).await;
    });
    let network = network.unwrap();
    let policy = ContainerPolicy::new(
        selection,
        1000,
        1000,
        1_000_000_000,
        RelayBudgets::default(),
    )
    .unwrap();
    let relay = image_with("1000", vec![]);
    let base = EnvironmentProbeConfig::builder()
        .directory(directory.clone())
        .relay(relay.clone())
        .environments(owner.port())
        .network(network.clone())
        .policy(policy.clone())
        .build();
    // Synthetic private receipt only tests scope/cleanup logic; real proof is separately gated.
    let resource = EnvironmentProbe {
        report: report(AttemptId::default()),
        directory,
        relay,
        policy: network.apply(&policy).unwrap(),
        environments: owner.port(),
    };
    let config = EndpointProbeConfig::new(
        base,
        resource,
        "http://private-gateway.example:4566".parse().unwrap(),
        GatewayInstanceId::generate().unwrap(),
    )
    .unwrap();
    (daemon, wire, owner, config)
}
#[tokio::test]
async fn test_should_bind_endpoint_receipt_to_exact_origin_instance_scope_policy_and_live_owner() {
    let (daemon, mut wire, mut owner, config) = fixture().await;
    let attempt = AttemptId::default();
    let receipt = EndpointProbe {
        report: verify_report(
            &report_bytes(attempt, config.instance),
            attempt,
            Architecture::Arm64,
            config.instance,
        )
        .unwrap(),
        inputs: config.clone(),
    };
    assert!(receipt.matches(&config));
    let mut changed = config.clone();
    changed.endpoint = "http://other-gateway.example".parse().unwrap();
    assert!(!receipt.matches(&changed));
    let mut changed = config.clone();
    changed.instance = GatewayInstanceId::generate().unwrap();
    assert!(!receipt.matches(&changed));
    let mut changed = config.clone();
    changed.base.directory = DirectoryLease::testing(Uuid::new_v4(), Uuid::new_v4());
    assert!(!receipt.matches(&changed));
    let mut changed = config.clone();
    changed.base.policy = ContainerPolicy::new(
        "endpoint-fixture".parse().unwrap(),
        1000,
        1000,
        200_000_000,
        RelayBudgets::default(),
    )
    .unwrap();
    assert!(!receipt.matches(&changed));
    assert!(matches!(
        probe_endpoint(config.clone(), Instant::now()).await,
        Err(EnvironmentProbeError::Environment(_))
    ));
    let mut base = config.base.clone();
    base.network = "none"
        .parse::<NetworkSelection>()
        .unwrap()
        .qualify(&wire.port(), deadline())
        .await
        .unwrap();
    assert!(
        EndpointProbeConfig::new(
            base,
            config.resource.clone(),
            config.endpoint.clone(),
            config.instance
        )
        .is_err()
    );
    owner.shutdown(deadline()).await.unwrap();
    assert!(!receipt.matches(&config));
    assert!(probe_endpoint(config, deadline()).await.is_err());
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[rstest]
#[case::rejected(false)]
#[case::abandoned(true)]
#[tokio::test]
async fn test_should_retain_endpoint_environment_cleanup_without_replaying_admitted_probe(
    #[case] abandoned: bool,
) {
    let (mut daemon, mut wire, mut owner, config) = fixture().await;
    let task = tokio::spawn(probe_endpoint(config, deadline()));
    let held = daemon.next().await;
    if abandoned {
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(owner.port().status().allocated, 1);
        held.reply(400, b"{}").await;
    } else {
        held.reply(400, b"{}").await;
        assert!(matches!(
            task.await.unwrap(),
            Err(EnvironmentProbeError::Environment(_))
        ));
    }
    owner.shutdown(deadline()).await.unwrap();
    assert_eq!(owner.port().status().allocated, 0);
    assert!(daemon.requests.try_recv().is_err());
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
