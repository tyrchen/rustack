use std::{sync::Arc, time::Duration};

use serde_json::{Value, json};
use tokio::time::timeout;

use super::*;
use crate::docker::{
    EnvironmentPlan,
    plan::{
        ContainerRole,
        tests::{image_with, policy, revision},
    },
    tests::Daemon,
};

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
async fn fixture() -> (
    tempfile::TempDir,
    DirectoryLease,
    Vec<ContainerRecord>,
    Vec<Value>,
) {
    let root = tempfile::tempdir().unwrap();
    let directory = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    Daemon::bind_fixture(&directory).await;
    let plan = EnvironmentPlan::new(
        directory.clone(),
        image_with("1000", vec![]),
        Arc::new(revision()),
        policy(),
    )
    .unwrap();
    let mut records = Vec::new();
    let mut inspected = Vec::new();
    for role in [ContainerRole::Function, ContainerRole::Relay] {
        let record = ContainerRecord::from_plan(&plan, role).unwrap();
        directory
            .prepare_container(record.clone(), deadline())
            .await
            .unwrap();
        let role = if record.function() {
            "function"
        } else {
            "relay"
        };
        let id = if record.function() { "a" } else { "b" }.repeat(64);
        inspected.push(json!({"Id": id,"Name":format!("/{}",record.key().unwrap().as_str()),"Config":{"Image":plan.image_id(if record.function() {ContainerRole::Function} else {ContainerRole::Relay}),"Labels":{
            "rustack.lambda.ownerId":directory.owner_id().to_string(),"rustack.lambda.runId":directory.run_id().to_string(),"rustack.lambda.environmentId":plan.environment_id().to_string(),"rustack.lambda.functionResourceId":plan.revision().resource_id.to_string(),"rustack.lambda.role":role}},"State":{"Running":false}}));
        records.push(record);
    }
    drop(plan);
    drop(directory);
    let next = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    (root, next, records, inspected)
}
async fn respond(daemon: &mut Daemon, value: &Value) {
    daemon
        .next()
        .await
        .reply(200, &serde_json::to_vec(value).unwrap())
        .await;
}
#[tokio::test]
async fn test_should_keep_unknown_absence_and_block_relay_then_recover_late_commit() {
    let (_root, directory, records, inspected) = fixture().await;
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let port = wire
        .port()
        .bind_directory(&directory, deadline())
        .await
        .unwrap();
    let recover = reconcile_containers(&directory, &port, deadline());
    let serve = async {
        let request = daemon.next().await;
        assert!(request.head.contains("function"));
        request.reply(404, b"{}").await;
    };
    let (result, ()) = tokio::join!(recover, serve);
    assert_eq!(
        result.unwrap(),
        ContainerRecovery {
            settled: 0,
            remaining: 2
        }
    );
    assert_eq!(
        directory.container_records(deadline()).await.unwrap(),
        records
    );
    let port = wire
        .port()
        .bind_directory(&directory, deadline())
        .await
        .unwrap();
    let recover = reconcile_containers(&directory, &port, deadline());
    let serve = async {
        for value in &inspected {
            respond(&mut daemon, value).await;
            // The full ID must already be durable before any destructive call.
            let request = daemon.next().await;
            let saved = directory.container_records(deadline()).await.unwrap();
            assert!(saved.iter().any(|record| record.known.is_some()));
            assert!(request.head.contains(value["Id"].as_str().unwrap()));
            request
                .reply(200, &serde_json::to_vec(value).unwrap())
                .await;
            let remove = daemon.next().await;
            assert!(remove.head.starts_with("DELETE "));
            remove.reply(204, b"").await;
            daemon.next().await.reply(404, b"{}").await;
        }
    };
    let (result, ()) = tokio::join!(recover, serve);
    assert_eq!(
        result.unwrap(),
        ContainerRecovery {
            settled: 2,
            remaining: 0
        }
    );
    assert_eq!(directory.container_records(deadline()).await.unwrap(), []);
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[tokio::test]
async fn test_should_settle_only_durable_known_absence() {
    let (_root, directory, records, _) = fixture().await;
    for record in &records {
        directory
            .know_container(record.clone(), "a".repeat(64).parse().unwrap(), deadline())
            .await
            .unwrap();
    }
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let port = wire
        .port()
        .bind_directory(&directory, deadline())
        .await
        .unwrap();
    let (result, ()) = tokio::join!(reconcile_containers(&directory, &port, deadline()), async {
        for _ in 0..2 {
            let request = daemon.next().await;
            assert!(request.head.contains(&"a".repeat(64)));
            request.reply(404, b"{}").await;
        }
    });
    assert_eq!(result.unwrap().remaining, 0);
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[rstest::rstest]
#[case("/Name")]
#[case("/Id")]
#[case("/Config/Image")]
#[case("/Config/Labels/rustack.lambda.ownerId")]
#[case("/Config/Labels/rustack.lambda.runId")]
#[case("/Config/Labels/rustack.lambda.environmentId")]
#[case("/Config/Labels/rustack.lambda.functionResourceId")]
#[case("/Config/Labels/rustack.lambda.role")]
#[tokio::test]
async fn test_should_never_mutate_foreign_or_malformed_resources(#[case] field: &str) {
    let (_root, directory, _, mut inspected) = fixture().await;
    *inspected[0].pointer_mut(field).unwrap() = json!("foreign");
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let port = wire
        .port()
        .bind_directory(&directory, deadline())
        .await
        .unwrap();
    let (result, ()) = tokio::join!(
        reconcile_containers(&directory, &port, deadline()),
        respond(&mut daemon, &inspected[0])
    );
    assert!(result.is_err());
    assert_eq!(
        directory.container_records(deadline()).await.unwrap().len(),
        2
    );
    assert!(
        timeout(Duration::from_millis(20), daemon.requests.recv())
            .await
            .is_err()
    );
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[tokio::test]
async fn test_should_never_reconcile_current_run_records() {
    let (_root, directory, records, _) = fixture().await;
    for record in records {
        directory
            .settle_container(record, deadline())
            .await
            .unwrap();
    }
    let plan = EnvironmentPlan::new(
        directory.clone(),
        image_with("1000", vec![]),
        Arc::new(revision()),
        policy(),
    )
    .unwrap();
    directory
        .prepare_container(
            ContainerRecord::from_plan(&plan, ContainerRole::Relay).unwrap(),
            deadline(),
        )
        .await
        .unwrap();
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    assert_eq!(
        reconcile_containers(
            &directory,
            &wire
                .port()
                .bind_directory(&directory, deadline())
                .await
                .unwrap(),
            deadline()
        )
        .await
        .unwrap()
        .remaining,
        1
    );
    assert!(
        timeout(Duration::from_millis(20), daemon.requests.recv())
            .await
            .is_err()
    );
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
#[ignore = "requires explicit Docker socket and matching runtime image"]
async fn test_should_reconcile_real_previous_run_containers_from_durable_intents()
-> anyhow::Result<()> {
    use anyhow::{Context, ensure};

    use crate::{
        artifact::{Architecture, DeploymentArtifact},
        docker::{DockerWire, ImageLeaseConfig, ImageLeaseManager, ImagePinScope},
    };
    let socket =
        std::env::var("RUSTACK_RUNTIME_DOCKER_SOCKET").context("explicit Docker socket")?;
    let architecture = match std::env::var("RUSTACK_RUNTIME_ARCH")?.as_str() {
        "arm64" => Architecture::Arm64,
        "amd64" => Architecture::X86_64,
        _ => anyhow::bail!("unknown fixture architecture"),
    };
    let image = std::env::var("RUSTACK_RUNTIME_IMAGE")?.parse()?;
    let limit = || Instant::now() + Duration::from_secs(60);
    let mut wire = DockerWire::connect(format!("unix://{socket}").parse()?, limit()).await?;
    let root = tempfile::tempdir()?;
    let directory = DirectoryLease::acquire(root.path().into(), limit()).await?;
    let docker = wire.port().bind_directory(&directory, limit()).await?;
    let previous_run = directory.run_id();
    let mut images = ImageLeaseManager::start(
        docker.clone(),
        ImagePinScope::new(directory.clone()),
        ImageLeaseConfig::default(),
    )?;
    let setup = async {
        let image = images
            .port()
            .prepare(&image, architecture, None, limit())
            .await?;
        let mut revision = revision();
        revision.architecture = architecture;
        revision.artifact = DeploymentArtifact::Image(image.clone());
        let plan = EnvironmentPlan::new(directory.clone(), image, Arc::new(revision), policy())?;
        for role in [ContainerRole::Relay, ContainerRole::Function] {
            directory
                .prepare_container(ContainerRecord::from_plan(&plan, role)?, limit())
                .await?;
            let created = match role {
                ContainerRole::Relay => plan.create_relay(&docker.clone(), limit()).await?,
                ContainerRole::Function => plan.create_function(&docker.clone(), limit()).await?,
            };
            if matches!(role, ContainerRole::Relay) {
                docker
                    .start_container(&created.id.parse()?, limit())
                    .await?;
            }
        }
        anyhow::Ok(())
    }
    .await;
    let image_cleanup = images.shutdown(limit()).await;
    drop(images);
    drop(docker);
    drop(directory);
    let recovery = async {
        let next = DirectoryLease::acquire(root.path().into(), limit()).await?;
        ensure!(next.run_id() != previous_run, "run identity did not change");
        let docker = wire.port().bind_directory(&next, limit()).await?;
        let report = reconcile_containers(&next, &docker, limit()).await?;
        ensure!(
            next.container_records(limit()).await?.is_empty(),
            "durable records remain"
        );
        anyhow::Ok(report)
    }
    .await;
    if !recovery.as_ref().is_ok_and(|report| report.remaining == 0)
        || image_cleanup.is_err()
        || setup.is_err()
    {
        // Any post-intent failure retains the evidence, including failed lease
        // reacquisition while an image recovery owner still holds exclusion.
        let _ = root.keep();
    }
    let wire_cleanup = wire.shutdown().await;
    setup?;
    image_cleanup?;
    let report = recovery?;
    ensure!(
        report.settled == 2 && report.remaining == 0,
        "historical containers were not both settled"
    );
    wire_cleanup?;
    Ok(())
}

#[tokio::test]
async fn test_should_resume_known_identity_after_recovery_waiter_is_aborted() {
    let (_root, directory, _, inspected) = fixture().await;
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let lease = directory.clone();
    let port = wire
        .port()
        .bind_directory(&directory, deadline())
        .await
        .unwrap();
    let task = tokio::spawn(async move { reconcile_containers(&lease, &port, deadline()).await });
    respond(&mut daemon, &inspected[0]).await;
    let held = daemon.next().await;
    assert!(held.head.contains(&"a".repeat(64)));
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    held.reply(200, &serde_json::to_vec(&inspected[0]).unwrap())
        .await;
    let saved = directory.container_records(deadline()).await.unwrap();
    assert!(
        saved
            .iter()
            .any(|record| record.function() && record.known.is_some())
    );
    let port = wire
        .port()
        .bind_directory(&directory, deadline())
        .await
        .unwrap();
    let (result, ()) = tokio::join!(reconcile_containers(&directory, &port, deadline()), async {
        let known = daemon.next().await;
        assert!(known.head.contains(&"a".repeat(64)));
        known.reply(404, b"{}").await;
        daemon.next().await.reply(404, b"{}").await;
    });
    assert_eq!(
        result.unwrap(),
        ContainerRecovery {
            settled: 1,
            remaining: 1
        }
    );
    assert!(!directory.container_records(deadline()).await.unwrap()[0].function());
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
