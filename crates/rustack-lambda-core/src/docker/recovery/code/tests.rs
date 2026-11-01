use std::time::Duration;

use rstest::rstest;
use serde_json::{Value, json, to_vec};
use tempfile::TempDir;
use tokio::{
    spawn,
    time::{Instant, timeout},
};

use super::*;
use crate::docker::{
    DockerWire, EnvironmentError, ImagePinScope,
    ownership::{
        ImageRecord,
        codes::tests::{publish, record},
    },
    reconcile_namespace,
    tests::{Daemon, IncomingRequest},
};

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
async fn fixture(
    volume: VolumeState,
    loader: LoaderState,
) -> (TempDir, DirectoryLease, CodeRecord) {
    let root = tempfile::tempdir().unwrap();
    let directory = DirectoryLease::acquire_code(root.path().into(), deadline())
        .await
        .unwrap();
    Daemon::bind_fixture(&directory).await;
    let row = record(directory.run_id(), volume, loader);
    publish(&directory, vec![row.clone()]).await;
    drop(directory);
    let directory = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    (root, directory, row)
}
async fn wire(directory: &DirectoryLease) -> (Daemon, DockerWire, DockerPort) {
    let daemon = Daemon::new();
    let wire = daemon.wire().await;
    let port = wire
        .port()
        .bind_directory(directory, deadline())
        .await
        .unwrap();
    (daemon, wire, port)
}
fn volume_json(directory: &DirectoryLease, row: &CodeRecord) -> Value {
    json!({"Name":row.volume_key().unwrap().as_str(),"Driver":"local","Scope":"local","Mountpoint":"/unused","Labels":row.labels(directory.owner_id(),false),"Options":{}})
}
fn loader_json(directory: &DirectoryLease, row: &CodeRecord) -> Value {
    json!({"Id":"cc".repeat(32),"Name":format!("/rustack-code-loader-{}",row.environment),"Image":format!("sha256:{}","aa".repeat(32)),"Config":{"Image":format!("sha256:{}","aa".repeat(32)),"Labels":row.labels(directory.owner_id(),true)},"State":{"Status":"created","Running":false,"StartedAt":"0001-01-01T00:00:00Z"},"Mounts":[{"Type":"volume","Name":row.volume_key().unwrap().as_str(),"Destination":"/var/task","RW":true,"Driver":"local"}]})
}
async fn reply_json(request: IncomingRequest, value: &Value) {
    request.reply(200, &to_vec(value).unwrap()).await;
}
async fn no_more(daemon: &mut Daemon) {
    assert!(
        timeout(Duration::from_millis(20), daemon.requests.recv())
            .await
            .is_err()
    );
}
async fn audit(daemon: &mut Daemon, directory: &DirectoryLease) {
    let request = daemon.next().await;
    assert!(request.head.contains("/containers/json?all=true"));
    request.reply(200, b"[]").await;
    let request = daemon.next().await;
    volume_query(&request, directory);
    request.reply(200, br#"{"Volumes":[],"Warnings":[]}"#).await;
}
fn volume_query(request: &IncomingRequest, directory: &DirectoryLease) {
    let target = request
        .head
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    let (path, query) = target.split_once('?').unwrap();
    assert_eq!(path, "/v1.53/volumes");
    let pairs = form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect::<Vec<_>>();
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0].0, "filters");
    assert_eq!(
        serde_json::from_str::<Value>(&pairs[0].1).unwrap(),
        json!({"label":[format!("rustack.lambda.ownerId={}",directory.owner_id())]})
    );
}

#[rstest]
#[case(VolumeState::Planned, LoaderState::Planned)]
#[case(VolumeState::Retired, LoaderState::Retired)]
#[tokio::test]
async fn test_should_erase_no_effect_or_retired_records_without_docker_requests(
    #[case] volume: VolumeState,
    #[case] loader: LoaderState,
) {
    let (_root, directory, _) = fixture(volume, loader).await;
    let (mut daemon, mut wire, port) = wire(&directory).await;
    let report = reconcile_code(&directory, &port, deadline()).await.unwrap();
    assert_eq!(
        report,
        CodeRecovery {
            settled: 1,
            remaining: 0
        }
    );
    no_more(&mut daemon).await;
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[rstest]
#[case(VolumeState::Uncertain, LoaderState::Planned)]
#[case(VolumeState::Known, LoaderState::Uncertain)]
#[tokio::test]
async fn test_should_retain_unknown_absence_without_volume_or_pin_deletion(
    #[case] volume: VolumeState,
    #[case] loader: LoaderState,
) {
    let (_root, directory, row) = fixture(volume, loader).await;
    let (mut daemon, mut wire, port) = wire(&directory).await;
    let (report, ()) = tokio::join!(reconcile_code(&directory, &port, deadline()), async {
        daemon.next().await.reply(404, b"{}").await;
    });
    assert_eq!(
        report.unwrap(),
        CodeRecovery {
            settled: 0,
            remaining: 1
        }
    );
    assert_eq!(directory.code_records(deadline()).await.unwrap(), [row]);
    assert!(matches!(
        super::super::images::reconcile_image_pins(&directory, &port, deadline()).await,
        Err(DockerError::Quarantined)
    ));
    no_more(&mut daemon).await;
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[tokio::test]
async fn test_should_erase_known_absent_loader_before_known_absent_volume() {
    let (_root, directory, _) = fixture(VolumeState::Known, LoaderState::Known([0xcc; 32])).await;
    let (mut daemon, mut wire, port) = wire(&directory).await;
    let (report, ()) = tokio::join!(reconcile_code(&directory, &port, deadline()), async {
        let request = daemon.next().await;
        assert!(request.head.contains(&"cc".repeat(32)));
        request.reply(404, b"{}").await;
        let request = daemon.next().await;
        assert!(request.head.contains("/volumes/rustack%2Dcode%2D"));
        assert_eq!(
            directory.code_records(deadline()).await.unwrap()[0].loader,
            LoaderState::Retired
        );
        request.reply(404, b"{}").await;
    });
    assert_eq!(
        report.unwrap(),
        CodeRecovery {
            settled: 1,
            remaining: 0
        }
    );
    no_more(&mut daemon).await;
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[tokio::test]
async fn test_should_publish_observed_id_before_delete_and_retire_loader_before_volume() {
    let (_root, directory, row) = fixture(VolumeState::Known, LoaderState::Uncertain).await;
    let (mut daemon, mut wire, port) = wire(&directory).await;
    let loader = loader_json(&directory, &row);
    let volume = volume_json(&directory, &row);
    let (report, ()) = tokio::join!(reconcile_code(&directory, &port, deadline()), async {
        let request = daemon.next().await;
        assert!(
            request
                .head
                .contains("/containers/rustack%2Dcode%2Dloader%2D")
        );
        reply_json(request, &loader).await;
        let request = daemon.next().await;
        assert!(request.head.contains(&"cc".repeat(32)));
        assert_eq!(
            directory.code_records(deadline()).await.unwrap()[0].loader,
            LoaderState::Known([0xcc; 32])
        );
        reply_json(request, &loader).await;
        let request = daemon.next().await;
        assert!(request.head.starts_with("DELETE "));
        assert!(request.head.contains("force=false"));
        assert!(request.head.contains(&"cc".repeat(32)));
        request.reply(204, b"").await;
        daemon.next().await.reply(404, b"{}").await;
        let request = daemon.next().await;
        assert!(request.head.contains("/volumes/rustack%2Dcode%2D"));
        assert_eq!(
            directory.code_records(deadline()).await.unwrap()[0].loader,
            LoaderState::Retired
        );
        reply_json(request, &volume).await;
        let request = daemon.next().await;
        assert!(request.head.starts_with("DELETE "));
        assert!(request.head.contains("force=false"));
        assert!(request.head.contains("force=false"));
        request.reply(204, b"").await;
        daemon.next().await.reply(404, b"{}").await;
    });
    assert_eq!(
        report.unwrap(),
        CodeRecovery {
            settled: 1,
            remaining: 0
        }
    );
    no_more(&mut daemon).await;
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[tokio::test]
async fn test_should_commit_uncertain_volume_before_retiring_planned_loader_and_deleting_volume() {
    let (_root, directory, row) = fixture(VolumeState::Uncertain, LoaderState::Planned).await;
    let (mut daemon, mut wire, port) = wire(&directory).await;
    let volume = volume_json(&directory, &row);
    let (report, ()) = tokio::join!(reconcile_code(&directory, &port, deadline()), async {
        reply_json(daemon.next().await, &volume).await;
        let request = daemon.next().await;
        let saved = directory.code_records(deadline()).await.unwrap();
        assert_eq!(saved[0].volume, VolumeState::Known);
        assert_eq!(saved[0].loader, LoaderState::Retired);
        reply_json(request, &volume).await;
        daemon.next().await.reply(204, b"").await;
        daemon.next().await.reply(404, b"{}").await;
    });
    assert_eq!(report.unwrap().remaining, 0);
    no_more(&mut daemon).await;
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[rstest]
#[case("running")]
#[case("started")]
#[case("image")]
#[case("configImage")]
#[case("name")]
#[case("id")]
#[case("knownId")]
#[case("labels")]
#[case("mountName")]
#[case("mountMode")]
#[case("mountType")]
#[case("mountDriver")]
#[case("extraMount")]
#[case("destination")]
#[tokio::test]
async fn test_should_quarantine_conflicting_or_executed_loader_without_delete(
    #[case] conflict: &str,
) {
    let (_root, directory, row) = fixture(VolumeState::Known, LoaderState::Known([0xcc; 32])).await;
    let (mut daemon, mut wire, port) = wire(&directory).await;
    let mut loader = loader_json(&directory, &row);
    match conflict {
        "running" => {
            loader["State"]["Status"] = json!("running");
            loader["State"]["Running"] = json!(true);
        }
        "started" => loader["State"]["StartedAt"] = json!("2026-01-01T00:00:00Z"),
        "image" => loader["Image"] = json!(format!("sha256:{}", "dd".repeat(32))),
        "configImage" => loader["Config"]["Image"] = json!("operator:mutable"),
        "name" => loader["Name"] = json!("/foreign"),
        "id" => loader["Id"] = json!("CC".repeat(32)),
        "knownId" => loader["Id"] = json!("dd".repeat(32)),
        "labels" => loader["Config"]["Labels"]["rustack.lambda.unexpected"] = json!("foreign"),
        "mountName" => loader["Mounts"][0]["Name"] = json!("foreign"),
        "mountMode" => loader["Mounts"][0]["RW"] = json!(false),
        "mountType" => loader["Mounts"][0]["Type"] = json!("bind"),
        "mountDriver" => loader["Mounts"][0]["Driver"] = json!("foreign"),
        "extraMount" => loader["Mounts"]
            .as_array_mut()
            .unwrap()
            .push(json!({"Type":"bind"})),
        _ => loader["Mounts"][0]["Destination"] = json!("/wrong"),
    }
    let (report, ()) = tokio::join!(reconcile_code(&directory, &port, deadline()), async {
        reply_json(daemon.next().await, &loader).await;
    });
    assert!(matches!(report, Err(DockerError::Quarantined)));
    assert_eq!(directory.code_records(deadline()).await.unwrap(), [row]);
    no_more(&mut daemon).await;
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[rstest]
#[case("driver")]
#[case("scope")]
#[case("options")]
#[case("labels")]
#[tokio::test]
async fn test_should_quarantine_foreign_volume_without_delete(#[case] conflict: &str) {
    let (_root, directory, row) = fixture(VolumeState::Known, LoaderState::Retired).await;
    let (mut daemon, mut wire, port) = wire(&directory).await;
    let mut volume = volume_json(&directory, &row);
    match conflict {
        "driver" => volume["Driver"] = json!("foreign"),
        "scope" => volume["Scope"] = json!("global"),
        "options" => volume["Options"] = json!({"device":"/host"}),
        _ => volume["Labels"]["unexpected"] = json!("foreign"),
    }
    let (report, ()) = tokio::join!(reconcile_code(&directory, &port, deadline()), async {
        reply_json(daemon.next().await, &volume).await;
    });
    assert!(matches!(report, Err(DockerError::Quarantined)));
    assert_eq!(directory.code_records(deadline()).await.unwrap(), [row]);
    no_more(&mut daemon).await;
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[tokio::test]
async fn test_should_retain_attached_volume_after_conflicting_delete_until_positive_absence() {
    let (_root, directory, row) = fixture(VolumeState::Known, LoaderState::Retired).await;
    let (mut daemon, mut wire, port) = wire(&directory).await;
    let volume = volume_json(&directory, &row);
    let (report, ()) = tokio::join!(reconcile_code(&directory, &port, deadline()), async {
        reply_json(daemon.next().await, &volume).await;
        daemon.next().await.reply(409, b"{}").await;
        reply_json(daemon.next().await, &volume).await;
    });
    assert_eq!(report.unwrap().remaining, 1);
    assert_eq!(directory.code_records(deadline()).await.unwrap(), [row]);
    let (report, ()) = tokio::join!(reconcile_code(&directory, &port, deadline()), async {
        daemon.next().await.reply(404, b"{}").await;
    });
    assert_eq!(report.unwrap().remaining, 0);
    no_more(&mut daemon).await;
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[tokio::test]
async fn test_should_keep_durable_known_id_after_observer_loss_and_resume_exact_cleanup() {
    let (_root, directory, row) = fixture(VolumeState::Known, LoaderState::Uncertain).await;
    let (mut daemon, mut wire, port) = wire(&directory).await;
    let loader = loader_json(&directory, &row);
    let copy = directory.clone();
    let call = port.clone();
    let task = spawn(async move { reconcile_code(&copy, &call, deadline()).await });
    reply_json(daemon.next().await, &loader).await;
    let held = daemon.next().await;
    assert_eq!(
        directory.code_records(deadline()).await.unwrap()[0].loader,
        LoaderState::Known([0xcc; 32])
    );
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    held.reply(404, b"{}").await;
    let (report, ()) = tokio::join!(reconcile_code(&directory, &port, deadline()), async {
        let request = daemon.next().await;
        assert!(request.head.contains(&"cc".repeat(32)));
        request.reply(404, b"{}").await;
        daemon.next().await.reply(404, b"{}").await;
    });
    assert_eq!(report.unwrap().remaining, 0);
    no_more(&mut daemon).await;
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[tokio::test]
async fn test_should_refuse_identity_fence_404_without_settling_resource() {
    let (_root, directory, row) = fixture(VolumeState::Known, LoaderState::Retired).await;
    let (mut daemon, mut wire, port) = wire(&directory).await;
    daemon.identity_status.send_replace(404);
    assert!(matches!(
        reconcile_code(&directory, &port, deadline()).await,
        Err(DockerError::IdentityCheck(_))
    ));
    assert_eq!(directory.code_records(deadline()).await.unwrap(), [row]);
    no_more(&mut daemon).await;
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[tokio::test]
async fn test_should_block_namespace_and_pin_retirement_while_code_is_uncertain() {
    let (_root, directory, _) = fixture(VolumeState::Known, LoaderState::Uncertain).await;
    let (mut daemon, mut wire, port) = wire(&directory).await;
    let (report, ()) = tokio::join!(reconcile_namespace(&directory, &port, deadline()), async {
        daemon.next().await.reply(404, b"{}").await;
    });
    let report = report.unwrap();
    assert_eq!(report.code.remaining, 1);
    assert_eq!(report.pins.settled, 0);
    no_more(&mut daemon).await;
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[tokio::test]
async fn test_should_recover_code_before_pin_and_complete_untruncated_volume_audits() {
    let root = tempfile::tempdir().unwrap();
    let previous = DirectoryLease::acquire_code(root.path().into(), deadline())
        .await
        .unwrap();
    Daemon::bind_fixture(&previous).await;
    publish(
        &previous,
        vec![record(
            previous.run_id(),
            VolumeState::Known,
            LoaderState::Retired,
        )],
    )
    .await;
    let pin = ImagePinScope::new(previous.clone()).allocate().unwrap();
    let entry = ImageRecord::new(
        &pin,
        &format!("sha256:{}", "aa".repeat(32)).parse().unwrap(),
    )
    .unwrap();
    previous
        .prepare_image(entry.clone(), deadline())
        .await
        .unwrap();
    previous.commit_image(entry, deadline()).await.unwrap();
    drop(pin);
    drop(previous);
    let directory = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    let (mut daemon, mut wire, port) = wire(&directory).await;
    let (report, ()) = tokio::join!(reconcile_namespace(&directory, &port, deadline()), async {
        let request = daemon.next().await;
        assert!(request.head.contains("/volumes/rustack%2Dcode%2D"));
        request.reply(404, b"{}").await;
        audit(&mut daemon, &directory).await;
        assert_eq!(directory.code_records(deadline()).await.unwrap(), []);
        let request = daemon.next().await;
        assert!(request.head.contains("/images/"));
        request.reply(404, b"{}").await;
        let request = daemon.next().await;
        assert!(request.head.contains("/images/json?all=true"));
        request.reply(200, b"[]").await;
        audit(&mut daemon, &directory).await;
    });
    let report = report.unwrap();
    assert_eq!(report.code.settled, 1);
    assert_eq!(report.pins.settled, 1);
    no_more(&mut daemon).await;
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn test_should_refuse_orphan_volume_without_adoption_or_pin_deletion(#[case] late: bool) {
    let (_root, directory, row) = fixture(VolumeState::Retired, LoaderState::Retired).await;
    let (mut daemon, mut wire, port) = wire(&directory).await;
    let volume = volume_json(&directory, &row);
    let (report, ()) = tokio::join!(reconcile_namespace(&directory, &port, deadline()), async {
        if late {
            audit(&mut daemon, &directory).await;
            daemon.next().await.reply(200, b"[]").await;
        }
        daemon.next().await.reply(200, b"[]").await;
        let request = daemon.next().await;
        volume_query(&request, &directory);
        reply_json(request, &json!({"Volumes":[volume],"Warnings":[]})).await;
    });
    assert!(matches!(
        report,
        Err(EnvironmentError::Docker(DockerError::Quarantined))
    ));
    no_more(&mut daemon).await;
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[tokio::test]
async fn test_should_reject_current_run_code_before_any_namespace_request() {
    let (_root, directory, _) = fixture(VolumeState::Retired, LoaderState::Retired).await;
    publish(
        &directory,
        vec![record(
            directory.run_id(),
            VolumeState::Known,
            LoaderState::Retired,
        )],
    )
    .await;
    let (mut daemon, mut wire, port) = wire(&directory).await;
    assert!(matches!(
        reconcile_namespace(&directory, &port, deadline()).await,
        Err(EnvironmentError::Docker(DockerError::Quarantined))
    ));
    no_more(&mut daemon).await;
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

mod real;

#[tokio::test]
async fn test_should_use_nonforce_loader_delete_and_retain_started_race_without_kill() {
    let (_root, directory, row) = fixture(VolumeState::Known, LoaderState::Known([0xcc; 32])).await;
    let (mut daemon, mut wire, port) = wire(&directory).await;
    let loader = loader_json(&directory, &row);
    let mut running = loader.clone();
    running["State"]["Status"] = json!("running");
    running["State"]["Running"] = json!(true);
    let (report, ()) = tokio::join!(reconcile_code(&directory, &port, deadline()), async {
        reply_json(daemon.next().await, &loader).await;
        reply_json(daemon.next().await, &loader).await;
        let request = daemon.next().await;
        assert!(request.head.starts_with(&format!(
            "DELETE /v1.53/containers/{}?force=false HTTP/",
            "cc".repeat(32)
        )));
        request.reply(409, b"{}").await;
        reply_json(daemon.next().await, &running).await;
    });
    assert_eq!(report.unwrap().remaining, 1);
    assert_eq!(directory.code_records(deadline()).await.unwrap(), [row]);
    let (report, ()) = tokio::join!(reconcile_code(&directory, &port, deadline()), async {
        reply_json(daemon.next().await, &running).await;
    });
    assert!(matches!(report, Err(DockerError::Quarantined)));
    no_more(&mut daemon).await;
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_recheck_loader_identity_after_durable_publication_before_delete() {
    let (_root, directory, row) = fixture(VolumeState::Known, LoaderState::Uncertain).await;
    let (mut daemon, mut wire, port) = wire(&directory).await;
    let loader = loader_json(&directory, &row);
    let mut foreign = loader.clone();
    foreign["Id"] = json!("dd".repeat(32));
    let (report, ()) = tokio::join!(reconcile_code(&directory, &port, deadline()), async {
        reply_json(daemon.next().await, &loader).await;
        let request = daemon.next().await;
        assert_eq!(
            directory.code_records(deadline()).await.unwrap()[0].loader,
            LoaderState::Known([0xcc; 32])
        );
        reply_json(request, &foreign).await;
    });
    assert!(matches!(report, Err(DockerError::Quarantined)));
    assert_eq!(
        directory.code_records(deadline()).await.unwrap()[0].loader,
        LoaderState::Known([0xcc; 32])
    );
    no_more(&mut daemon).await;
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[test]
fn test_should_allow_bounded_inherited_labels_but_reject_unknown_reserved_or_oversized_labels() {
    let row = record(
        uuid::Uuid::new_v4(),
        VolumeState::Known,
        LoaderState::Known([0xcc; 32]),
    );
    let directory = DirectoryLease::testing(uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
    let mut loader = loader_json(&directory, &row);
    loader["Config"]["Labels"]["com.amazonaws.lambda.platform.kernel"] = json!("k510ga");
    let parsed = serde_json::from_value(loader.clone()).unwrap();
    assert!(row.owned_loader(directory.owner_id(), &parsed).is_ok());
    loader["Config"]["Labels"]["com.amazonaws.lambda.platform.kernel"] = json!("x".repeat(257));
    let parsed = serde_json::from_value(loader).unwrap();
    assert!(matches!(
        row.owned_loader(directory.owner_id(), &parsed),
        Err(DockerError::Quarantined)
    ));
}
