use serde_json::{Value, json};
use tokio::time::{Duration, Instant};

use super::*;
use crate::docker::{plan::tests::policy, tests::Daemon};

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(3)
}
fn fixture() -> Value {
    json!({"Name":"probe-network","Id":"a".repeat(64),"Driver":"bridge","Scope":"local","Ingress":false,"ConfigOnly":false,"Options":{}})
}
#[rstest::rstest]
#[case::driver("Driver",json!("host"))]
#[case::scope("Scope",json!("swarm"))]
#[case::ingress("Ingress",json!(true))]
#[case::config_only("ConfigOnly",json!(true))]
#[case::missing_ingress("Ingress", Value::Null)]
#[case::missing_scope("Scope", Value::Null)]
#[case::wrong_name("Name",json!("another-network"))]
#[case::builtin("Name",json!("bridge"))]
#[case::bad_id("Id",json!("a".repeat(63)))]
#[case::uppercase_id("Id",json!("A".repeat(64)))]
#[case::default_bridge("Options",json!({"com.docker.network.bridge.default_bridge":"true"}))]
#[case::unknown_default("Options",json!({"com.docker.network.bridge.default_bridge":"unknown"}))]
#[test]
fn test_should_reject_unqualified_network_metadata(#[case] field: &str, #[case] value: Value) {
    let mut network = fixture();
    network[field] = value;
    let network = serde_json::from_value(network).unwrap();
    assert!(validate(&"probe-network".parse().unwrap(), &network).is_err());
}
#[test]
fn test_should_pin_only_exact_names_or_full_ids_and_bind_matching_policy() {
    let network = serde_json::from_value(fixture()).unwrap();
    let requested: NetworkSelection = "probe-network".parse().unwrap();
    let selected = validate(&requested, &network).unwrap();
    assert_eq!(selected.value(), "a".repeat(64));
    assert!(validate(&"aaaa".parse().unwrap(), &network).is_err());
    assert!(validate(&"a".repeat(64).parse().unwrap(), &network).is_ok());
    let qualified = QualifiedNetwork {
        requested,
        selected,
    };
    let mut configured = policy();
    assert!(qualified.apply(&configured).is_err());
    configured.network = "probe-network".parse().unwrap();
    let fixed = qualified.apply(&configured).unwrap();
    assert_eq!(fixed.network.value(), "a".repeat(64));
    assert!(qualified.apply(&fixed).is_ok());
    assert!(!qualified.disconnected());
}
#[tokio::test]
async fn test_should_issue_only_bounded_network_inspect_and_no_request_for_none() {
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let port = wire.port();
    let task = tokio::spawn(async move {
        "probe-network"
            .parse::<NetworkSelection>()
            .unwrap()
            .qualify(&port, deadline())
            .await
    });
    let request = daemon.next().await;
    assert!(
        request
            .head
            .starts_with("GET /v1.53/networks/probe%2Dnetwork ")
    );
    request
        .reply(200, &serde_json::to_vec(&fixture()).unwrap())
        .await;
    assert!(!task.await.unwrap().unwrap().disconnected());
    let none = "none"
        .parse::<NetworkSelection>()
        .unwrap()
        .qualify(&wire.port(), deadline())
        .await
        .unwrap();
    assert!(none.disconnected());
    assert!(none.apply(&policy()).is_ok());
    assert!(daemon.requests.try_recv().is_err());
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
