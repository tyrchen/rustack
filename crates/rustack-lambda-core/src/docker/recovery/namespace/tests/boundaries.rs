use std::{sync::Arc, time::Duration};

use rstest::rstest;
use serde_json::{json, to_vec};
use tokio::{io::AsyncWriteExt, time::Instant};

use super::{Entry, deadline, empty_container, fixture, no_more, wire};
use crate::docker::{
    DirectoryLease, DockerError, EnvironmentError, EnvironmentPlan, MAX_BODY_BYTES,
    ownership::ContainerRecord,
    plan::{
        ContainerRole,
        tests::{image_with, policy, revision},
    },
    reconcile_namespace,
};

#[rstest]
#[case::root_shape(0)]
#[case::tag_type(1)]
#[case::duplicate_fields(2)]
#[case::tag_bytes(3)]
#[case::row_members(4)]
#[case::tag_members(5)]
#[case::declared_body(6)]
#[case::exact_body(7)]
#[case::chunked_body(8)]
#[case::missing_tags(9)]
#[tokio::test]
async fn test_should_bound_actual_image_inventory_before_clean_result(#[case] mode: u8) {
    let (_root, directory) = fixture(Entry::None, Entry::None).await;
    let (mut daemon, mut wire, port) = wire(&directory).await;
    let (result, ()) = tokio::join!(reconcile_namespace(&directory, &port, deadline()), async {
        empty_container(&mut daemon, &directory).await;
        let mut request = daemon.next().await;
        assert!(request.head.starts_with("GET /v1.53/images/json?all=true "));
        match mode {
            6 => {
                request
                    .raw(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            MAX_BODY_BYTES + 1
                        )
                        .as_bytes(),
                    )
                    .await;
            }
            8 => {
                let mut response = format!(
                    "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: \
                     close\r\n\r\n{:x}\r\n",
                    MAX_BODY_BYTES + 1
                )
                .into_bytes();
                response.extend(vec![b' '; MAX_BODY_BYTES + 1]);
                response.extend_from_slice(b"\r\n0\r\n\r\n");
                let _ = request.stream.write_all(&response).await;
            }
            _ => {
                let body = match mode {
                    0 => b"{}".to_vec(),
                    1 => br#"[{"RepoTags":[42]}]"#.to_vec(),
                    2 => br#"[{"RepoTags":[],"RepoTags":[]}]"#.to_vec(),
                    3 => to_vec(&json!([{"RepoTags":["a".repeat(1025)]}])).unwrap(),
                    4 => to_vec(&vec![json!({}); 4097]).unwrap(),
                    5 => to_vec(&json!([{"RepoTags":vec!["worker:v1";4097]}])).unwrap(),
                    9 => b"[{}]".to_vec(),
                    _ => {
                        let mut bytes = b"[]".to_vec();
                        bytes.resize(MAX_BODY_BYTES, b' ');
                        bytes
                    }
                };
                request.reply(200, &body).await;
                if mode == 7 {
                    empty_container(&mut daemon, &directory).await;
                }
            }
        }
    });
    if mode == 7 {
        let report = result.unwrap();
        assert_eq!(report.containers.remaining, 0);
        assert_eq!(report.pins.remaining, 0);
    } else {
        assert!(result.is_err());
    }
    no_more(&mut daemon).await;
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn test_should_reject_invalid_budget_without_daemon_work(#[case] overlong: bool) {
    let (_root, directory) = fixture(Entry::None, Entry::None).await;
    let (mut daemon, mut wire, port) = wire(&directory).await;
    let limit = if overlong {
        Instant::now() + Duration::from_secs(61)
    } else {
        Instant::now()
    };
    assert!(matches!(
        reconcile_namespace(&directory, &port, limit).await,
        Err(EnvironmentError::Docker(
            DockerError::Deadline | DockerError::Configuration
        ))
    ));
    no_more(&mut daemon).await;
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_require_exact_directory_binding_before_reading_or_mutation() {
    let (_root, directory) = fixture(Entry::None, Entry::None).await;
    let (mut daemon, mut wire, port) = wire(&directory).await;
    let other_root = tempfile::tempdir().unwrap();
    let other = DirectoryLease::acquire(other_root.path().into(), deadline())
        .await
        .unwrap();
    assert!(matches!(
        reconcile_namespace(&other, &port, deadline()).await,
        Err(EnvironmentError::Docker(DockerError::DaemonBinding))
    ));
    assert!(matches!(
        reconcile_namespace(&directory, &wire.port(), deadline()).await,
        Err(EnvironmentError::Docker(DockerError::DaemonBinding))
    ));
    no_more(&mut daemon).await;
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_refuse_current_run_container_before_any_recovery_io() {
    let (_root, directory) = fixture(Entry::None, Entry::None).await;
    let (mut daemon, mut wire, port) = wire(&directory).await;
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
    assert!(matches!(
        reconcile_namespace(&directory, &port, deadline()).await,
        Err(EnvironmentError::Docker(DockerError::Quarantined))
    ));
    assert_eq!(
        directory.container_records(deadline()).await.unwrap(),
        [record]
    );
    no_more(&mut daemon).await;
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
