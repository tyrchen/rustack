use std::{sync::Arc, time::Duration};

use http::StatusCode;
use rstest::rstest;
use serde_json::{Value, json, to_vec};
use tempfile::TempDir;
use tokio::{
    spawn,
    time::{Instant, timeout},
};

use super::{NamespaceRecovery, reconcile_namespace};
use crate::docker::{
    DirectoryLease, DockerError, DockerPort, DockerWire, EnvironmentError, EnvironmentPlan,
    ImagePinScope,
    ownership::{ContainerRecord, ImageRecord},
    plan::{
        ContainerRole,
        tests::{image_with, policy, revision},
    },
    tests::{Daemon, IncomingRequest},
};

mod boundaries;
mod real;

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}

#[derive(Debug, Clone, Copy)]
enum Entry {
    None,
    Unknown,
    Known,
}

async fn fixture(container: Entry, pin_entry: Entry) -> (TempDir, DirectoryLease) {
    let root = tempfile::tempdir().unwrap();
    let directory = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    Daemon::bind_fixture(&directory).await;
    if !matches!(container, Entry::None) {
        let plan = EnvironmentPlan::new(
            directory.clone(),
            image_with("1000", vec![]),
            Arc::new(revision()),
            policy(),
        )
        .unwrap();
        let record = ContainerRecord::from_plan(&plan, ContainerRole::Relay).unwrap();
        directory
            .prepare_container(record.clone(), deadline())
            .await
            .unwrap();
        if matches!(container, Entry::Known) {
            directory
                .know_container(record, "a".repeat(64).parse().unwrap(), deadline())
                .await
                .unwrap();
        }
    }
    if !matches!(pin_entry, Entry::None) {
        let pin = ImagePinScope::new(directory.clone()).allocate().unwrap();
        let record =
            ImageRecord::new(&pin, &format!("sha256:{}", "a".repeat(64)).parse().unwrap()).unwrap();
        directory
            .prepare_image(record.clone(), deadline())
            .await
            .unwrap();
        if matches!(pin_entry, Entry::Known) {
            directory.commit_image(record, deadline()).await.unwrap();
        }
    }
    drop(directory);
    let next = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    (root, next)
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
fn container_query(request: &IncomingRequest, directory: &DirectoryLease) {
    let target = request
        .head
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    let (path, query) = target.split_once('?').unwrap();
    assert_eq!(path, "/v1.53/containers/json");
    let pairs = form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect::<Vec<_>>();
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs[0], ("all".into(), "true".into()));
    assert_eq!(
        serde_json::from_str::<Value>(&pairs[1].1).unwrap(),
        json!({"label":[format!("rustack.lambda.ownerId={}",directory.owner_id())]})
    );
    assert_eq!(pairs[1].0, "filters");
}
async fn empty_container(daemon: &mut Daemon, directory: &DirectoryLease) {
    let request = daemon.next().await;
    container_query(&request, directory);
    request.reply(200, b"[]").await;
}
async fn image_inventory(daemon: &mut Daemon, body: &[u8]) {
    let request = daemon.next().await;
    assert!(request.head.starts_with("GET /v1.53/images/json?all=true "));
    request.reply(200, body).await;
}
async fn no_more(daemon: &mut Daemon) {
    assert!(
        timeout(Duration::from_millis(20), daemon.requests.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn test_should_audit_empty_namespace_repeatedly_without_truncated_queries() {
    let (_root, directory) = fixture(Entry::None, Entry::None).await;
    let (mut daemon, mut wire, port) = wire(&directory).await;
    for _ in 0..2 {
        let (report, ()) =
            tokio::join!(reconcile_namespace(&directory, &port, deadline()), async {
                empty_container(&mut daemon, &directory).await;
                image_inventory(
                    &mut daemon,
                    br#"[{"RepoTags":null},{"RepoTags":["operator/image:v1"]}]"#,
                )
                .await;
                empty_container(&mut daemon, &directory).await;
            });
        let report = report.unwrap();
        assert_eq!(report.containers.remaining, 0);
        assert_eq!(report.pins.remaining, 0);
    }
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_settle_recorded_containers_before_pins_and_audit_afterward() {
    let (_root, directory) = fixture(Entry::Known, Entry::Known).await;
    let (mut daemon, mut wire, port) = wire(&directory).await;
    let (report, ()) = tokio::join!(reconcile_namespace(&directory, &port, deadline()), async {
        let request = daemon.next().await;
        assert!(request.head.contains(&"a".repeat(64)));
        request.reply(404, b"{}").await;
        empty_container(&mut daemon, &directory).await;
        assert_eq!(directory.container_records(deadline()).await.unwrap(), []);
        assert_eq!(directory.image_records(deadline()).await.unwrap().len(), 1);
        let request = daemon.next().await;
        assert!(request.head.contains("/images/"));
        request.reply(404, b"{}").await;
        image_inventory(&mut daemon, b"[]").await;
        assert_eq!(directory.image_records(deadline()).await.unwrap(), []);
        empty_container(&mut daemon, &directory).await;
    });
    let report = report.unwrap();
    assert_eq!(report.containers.settled, 1);
    assert_eq!(report.pins.settled, 1);
    assert_eq!(report.containers.remaining, 0);
    assert_eq!(report.pins.remaining, 0);
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_leave_pins_untouched_while_unknown_containers_remain() {
    let (_root, directory) = fixture(Entry::Unknown, Entry::Known).await;
    let (mut daemon, mut wire, port) = wire(&directory).await;
    let before = directory.image_records(deadline()).await.unwrap();
    let (report, ()) = tokio::join!(reconcile_namespace(&directory, &port, deadline()), async {
        daemon.next().await.reply(404, b"{}").await;
    });
    let report = report.unwrap();
    assert_eq!(report.containers.remaining, 1);
    assert_eq!(report.pins.remaining, 1);
    assert_eq!(report.pins.settled, 0);
    assert_eq!(directory.image_records(deadline()).await.unwrap(), before);
    no_more(&mut daemon).await;
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_not_use_absent_inventory_to_settle_unknown_pin() {
    let (_root, directory) = fixture(Entry::None, Entry::Unknown).await;
    let (mut daemon, mut wire, port) = wire(&directory).await;
    let (report, ()) = tokio::join!(reconcile_namespace(&directory, &port, deadline()), async {
        empty_container(&mut daemon, &directory).await;
        daemon.next().await.reply(404, b"{}").await;
    });
    let report = report.unwrap();
    assert_eq!(report.pins.remaining, 1);
    assert_eq!(report.pins.settled, 0);
    no_more(&mut daemon).await;
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[rstest]
#[case(1, false)]
#[case(65, false)]
#[case(1, true)]
#[tokio::test]
async fn test_should_refuse_orphan_or_late_containers_without_adoption(
    #[case] count: usize,
    #[case] late: bool,
) {
    let (_root, directory) = fixture(Entry::None, Entry::None).await;
    let (mut daemon, mut wire, port) = wire(&directory).await;
    let (report, ()) = tokio::join!(reconcile_namespace(&directory, &port, deadline()), async {
        if late {
            empty_container(&mut daemon, &directory).await;
            image_inventory(&mut daemon, b"[]").await;
        }
        let request = daemon.next().await;
        container_query(&request, &directory);
        request
            .reply(
                200,
                &to_vec(&vec![json!({"Id":"a".repeat(64)}); count]).unwrap(),
            )
            .await;
    });
    assert!(matches!(
        report,
        Err(EnvironmentError::Docker(
            DockerError::Quarantined | DockerError::TooLarge
        ))
    ));
    assert_eq!(directory.container_records(deadline()).await.unwrap(), []);
    no_more(&mut daemon).await;
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_refuse_unrecorded_owned_alias_without_image_deletion() {
    let (_root, directory) = fixture(Entry::None, Entry::None).await;
    let (mut daemon, mut wire, port) = wire(&directory).await;
    let tag = format!(
        "rustack-internal/lambda-{}/old/deep:pin",
        directory.owner_id().simple()
    );
    let (report, ()) = tokio::join!(reconcile_namespace(&directory, &port, deadline()), async {
        empty_container(&mut daemon, &directory).await;
        image_inventory(&mut daemon, &to_vec(&json!([{"RepoTags":[tag]}])).unwrap()).await;
    });
    assert!(matches!(
        report,
        Err(EnvironmentError::Docker(DockerError::Quarantined))
    ));
    assert_eq!(directory.image_records(deadline()).await.unwrap(), []);
    no_more(&mut daemon).await;
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn test_should_reject_current_run_or_late_journal_entries(#[case] late: bool) {
    let (_root, directory) = fixture(Entry::None, Entry::None).await;
    let (mut daemon, mut wire, port) = wire(&directory).await;
    let pin = ImagePinScope::new(directory.clone()).allocate().unwrap();
    let record =
        ImageRecord::new(&pin, &format!("sha256:{}", "a".repeat(64)).parse().unwrap()).unwrap();
    if !late {
        directory
            .prepare_image(record.clone(), deadline())
            .await
            .unwrap();
    }
    let (report, ()) = tokio::join!(reconcile_namespace(&directory, &port, deadline()), async {
        if late {
            empty_container(&mut daemon, &directory).await;
            image_inventory(&mut daemon, b"[]").await;
            let request = daemon.next().await;
            directory.prepare_image(record, deadline()).await.unwrap();
            request.reply(200, b"[]").await;
        }
    });
    assert!(matches!(
        report,
        Err(EnvironmentError::Docker(DockerError::Quarantined))
    ));
    assert_eq!(directory.image_records(deadline()).await.unwrap().len(), 1);
    no_more(&mut daemon).await;
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[rstest]
#[case(false, 404)]
#[case(false, 500)]
#[case(true, 404)]
#[tokio::test]
async fn test_should_preserve_list_status_origin_instead_of_empty_success(
    #[case] images: bool,
    #[case] status: u16,
) {
    let (_root, directory) = fixture(Entry::None, Entry::None).await;
    let (mut daemon, mut wire, port) = wire(&directory).await;
    let (report, ()) = tokio::join!(reconcile_namespace(&directory, &port, deadline()), async {
        if images {
            empty_container(&mut daemon, &directory).await;
        }
        daemon.next().await.reply(status, b"{}").await;
    });
    assert!(
        matches!(report, Err(EnvironmentError::Docker(DockerError::Status(actual))) if actual.as_u16()==status)
    );
    no_more(&mut daemon).await;
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_resume_recorded_cleanup_after_abandoned_observer() {
    let (_root, directory) = fixture(Entry::Known, Entry::None).await;
    let (mut daemon, mut wire, port) = wire(&directory).await;
    let copy = directory.clone();
    let call = port.clone();
    let task = spawn(async move { reconcile_namespace(&copy, &call, deadline()).await });
    let held = daemon.next().await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    held.reply(404, b"{}").await;
    assert_eq!(
        directory.container_records(deadline()).await.unwrap().len(),
        1
    );
    let (report, ()) = tokio::join!(reconcile_namespace(&directory, &port, deadline()), async {
        daemon.next().await.reply(404, b"{}").await;
        empty_container(&mut daemon, &directory).await;
        image_inventory(&mut daemon, b"[]").await;
        empty_container(&mut daemon, &directory).await;
    });
    let NamespaceRecovery { containers, pins } = report.unwrap();
    assert_eq!(containers.settled, 1);
    assert_eq!(pins.remaining, 0);
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_reject_identity_404_without_auditing_or_claiming_absence() {
    let (_root, directory) = fixture(Entry::None, Entry::None).await;
    let (mut daemon, mut wire, port) = wire(&directory).await;
    daemon.identity_status.send_replace(404);
    assert!(
        matches!(reconcile_namespace(&directory, &port, deadline()).await,
        Err(EnvironmentError::Docker(DockerError::IdentityCheck(source))) if matches!(source.as_ref(),DockerError::Status(StatusCode::NOT_FOUND)))
    );
    no_more(&mut daemon).await;
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
