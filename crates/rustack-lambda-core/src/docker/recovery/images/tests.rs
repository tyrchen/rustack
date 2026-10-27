use std::{path::Path, sync::Arc, time::Duration};

use anyhow::{Context, Result as TestResult, ensure};
use http::StatusCode;
use serde_json::json;
use tokio::time::{Instant, timeout};

use super::{ImagePinRecovery, reconcile_image_pins};
use crate::{
    artifact::Architecture,
    docker::{
        DirectoryLease, DockerError, DockerWire, EnvironmentPlan, ImagePin, ImagePinScope,
        ImageResolver,
        ownership::{ContainerRecord, ImageRecord},
        plan::{
            ContainerRole,
            tests::{image_with, policy, revision},
        },
        tests::Daemon,
    },
};
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
async fn fixture(
    committed: bool,
    containers: bool,
) -> (tempfile::TempDir, DirectoryLease, ImageRecord, ImagePin) {
    let root = tempfile::tempdir().unwrap();
    let directory = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    Daemon::bind_fixture(&directory).await;
    let pin = ImagePinScope::new(directory.clone()).allocate().unwrap();
    let record =
        ImageRecord::new(&pin, &format!("sha256:{}", "a".repeat(64)).parse().unwrap()).unwrap();
    directory
        .prepare_image(record.clone(), deadline())
        .await
        .unwrap();
    if committed {
        directory
            .commit_image(record.clone(), deadline())
            .await
            .unwrap();
    }
    if containers {
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
    }
    drop(pin);
    drop(directory);
    let next = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    let saved = next.image_records(deadline()).await.unwrap().remove(0);
    let pin = saved.pin(&next).unwrap();
    (root, next, saved, pin)
}
async fn matching(daemon: &mut Daemon) {
    daemon
        .next()
        .await
        .reply(
            200,
            &serde_json::to_vec(&json!({"Id":format!("sha256:{}","a".repeat(64))})).unwrap(),
        )
        .await;
}
#[tokio::test]
async fn test_should_retain_unknown_absence_then_commit_and_remove_exact_historical_alias() {
    let (_root, directory, record, pin) = fixture(false, false).await;
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let port = wire
        .port()
        .bind_directory(&directory, deadline())
        .await
        .unwrap();
    let (report, ()) = tokio::join!(reconcile_image_pins(&directory, &port, deadline()), async {
        daemon.next().await.reply(404, b"{}").await;
    });
    assert_eq!(
        report.unwrap(),
        ImagePinRecovery {
            settled: 0,
            remaining: 1
        }
    );
    assert_eq!(
        directory.image_records(deadline()).await.unwrap(),
        vec![record]
    );
    let (report, ()) = tokio::join!(reconcile_image_pins(&directory, &port, deadline()), async {
        matching(&mut daemon).await;
        let checked = daemon.next().await;
        assert!(directory.image_records(deadline()).await.unwrap()[0].committed);
        checked
            .reply(
                200,
                &serde_json::to_vec(&json!({"Id":format!("sha256:{}","a".repeat(64))})).unwrap(),
            )
            .await;
        let remove = daemon.next().await;
        assert!(remove.head.starts_with("DELETE "));
        assert!(
            remove
                .head
                .contains(&crate::docker::escaped(pin.reference().as_str()))
        );
        assert!(remove.head.contains("force=false&noprune=true"));
        remove.reply(200, b"[]").await;
        daemon.next().await.reply(404, b"{}").await;
    });
    assert_eq!(
        report.unwrap(),
        ImagePinRecovery {
            settled: 1,
            remaining: 0
        }
    );
    assert_eq!(directory.image_records(deadline()).await.unwrap(), []);
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[tokio::test]
async fn test_should_settle_absence_only_after_durable_commitment() {
    let (_root, directory, _, _pin) = fixture(true, false).await;
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let port = wire
        .port()
        .bind_directory(&directory, deadline())
        .await
        .unwrap();
    let (report, ()) = tokio::join!(reconcile_image_pins(&directory, &port, deadline()), async {
        daemon.next().await.reply(404, b"{}").await;
    });
    assert_eq!(
        report.unwrap(),
        ImagePinRecovery {
            settled: 1,
            remaining: 0
        }
    );
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[rstest::rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn test_should_never_delete_a_mismatching_alias(#[case] after_commit: bool) {
    let (_root, directory, _, _pin) = fixture(false, false).await;
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let port = wire
        .port()
        .bind_directory(&directory, deadline())
        .await
        .unwrap();
    let (result, ()) = tokio::join!(reconcile_image_pins(&directory, &port, deadline()), async {
        if after_commit {
            matching(&mut daemon).await;
        }
        daemon
            .next()
            .await
            .reply(
                200,
                &serde_json::to_vec(&json!({"Id":format!("sha256:{}","b".repeat(64))})).unwrap(),
            )
            .await;
    });
    assert!(matches!(result, Err(DockerError::Quarantined)));
    assert_eq!(
        directory.image_records(deadline()).await.unwrap()[0].committed,
        after_commit
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
async fn test_should_block_alias_mutations_while_historical_container_intents_remain() {
    let (_root, directory, _, _pin) = fixture(false, true).await;
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    assert!(matches!(
        reconcile_image_pins(
            &directory,
            &wire
                .port()
                .bind_directory(&directory, deadline())
                .await
                .unwrap(),
            deadline()
        )
        .await,
        Err(DockerError::Quarantined)
    ));
    assert_eq!(directory.image_records(deadline()).await.unwrap().len(), 1);
    assert!(
        timeout(Duration::from_millis(20), daemon.requests.recv())
            .await
            .is_err()
    );
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[tokio::test]
async fn test_should_exclude_current_run_aliases() {
    let (_root, directory, old, _pin) = fixture(false, false).await;
    directory.settle_image(old, deadline()).await.unwrap();
    let pin = ImagePinScope::new(directory.clone()).allocate().unwrap();
    directory
        .prepare_image(
            ImageRecord::new(&pin, &format!("sha256:{}", "a".repeat(64)).parse().unwrap()).unwrap(),
            deadline(),
        )
        .await
        .unwrap();
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    assert_eq!(
        reconcile_image_pins(
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
async fn test_should_resume_after_waiter_abandonment_with_persisted_commitment() {
    let (_root, directory, _, _pin) = fixture(false, false).await;
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let retained = directory.clone();
    let port = wire
        .port()
        .bind_directory(&directory, deadline())
        .await
        .unwrap();
    let task =
        tokio::spawn(async move { reconcile_image_pins(&retained, &port, deadline()).await });
    matching(&mut daemon).await;
    let held = daemon.next().await;
    assert!(directory.image_records(deadline()).await.unwrap()[0].committed);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    held.reply(200, b"{}").await;
    let port = wire
        .port()
        .bind_directory(&directory, deadline())
        .await
        .unwrap();
    let (report, ()) = tokio::join!(reconcile_image_pins(&directory, &port, deadline()), async {
        daemon.next().await.reply(404, b"{}").await;
    });
    assert_eq!(report.unwrap().remaining, 0);
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[derive(Debug)]
struct RecoveryEvidence(Option<tempfile::TempDir>);
impl RecoveryEvidence {
    fn path(&self) -> TestResult<&Path> {
        self.0
            .as_ref()
            .map(tempfile::TempDir::path)
            .context("missing retained image recovery evidence")
    }
    fn complete(&mut self) {
        drop(self.0.take());
    }
}
impl Drop for RecoveryEvidence {
    fn drop(&mut self) {
        if let Some(root) = self.0.take() {
            eprintln!(
                "unsettled image recovery evidence retained at {}",
                root.keep().display()
            );
        }
    }
}

#[rstest::rstest]
#[case::unknown_visible(false)]
#[case::committed_absent(true)]
#[tokio::test]
#[ignore = "requires explicit Docker socket and matching runtime image"]
async fn test_should_recover_real_private_alias_across_directory_reacquisition(
    #[case] committed_absent: bool,
) -> TestResult<()> {
    let socket =
        std::env::var("RUSTACK_RUNTIME_DOCKER_SOCKET").context("explicit Docker socket")?;
    let architecture = match std::env::var("RUSTACK_RUNTIME_ARCH")?.as_str() {
        "arm64" => Architecture::Arm64,
        "amd64" => Architecture::X86_64,
        _ => anyhow::bail!("unknown fixture architecture"),
    };
    let reference = std::env::var("RUSTACK_RUNTIME_IMAGE")?.parse()?;
    let limit = || Instant::now() + Duration::from_secs(60);
    let mut evidence = RecoveryEvidence(Some(tempfile::tempdir()?));
    let mut wire = DockerWire::connect(format!("unix://{socket}").parse()?, limit()).await?;
    let mut previous = None;
    let mut original = None;
    let mut alias = None;
    let setup = async {
        let directory = DirectoryLease::acquire(evidence.path()?.into(), limit()).await?;
        let docker = wire.port().bind_directory(&directory, limit()).await?;
        previous = Some(directory.run_id());
        let resolved = ImageResolver::new(docker.clone())
            .resolve_local(&reference, architecture, None, limit())
            .await?;
        let expected = resolved.metadata().image_id.clone();
        original = Some(expected.clone());
        let pin = ImagePinScope::new(directory.clone()).allocate()?;
        alias = Some(pin.reference().clone());
        let record = ImageRecord::new(&pin, &expected)?;
        directory.prepare_image(record.clone(), limit()).await?;
        docker.tag_image_pin(&expected, &pin, limit()).await?;
        if committed_absent {
            directory.commit_image(record, limit()).await?;
            docker.remove_image_pin(&pin, limit()).await?;
        }
        anyhow::Ok(())
    }
    .await;
    let recovery = async {
        let next = DirectoryLease::acquire(evidence.path()?.into(), limit()).await?;
        ensure!(
            Some(next.run_id()) != previous,
            "run identity did not change"
        );
        let docker = wire.port().bind_directory(&next, limit()).await?;
        let report = reconcile_image_pins(&next, &docker, limit()).await?;
        ensure!(
            report.settled == 1 && report.remaining == 0,
            "historical pin not settled"
        );
        ensure!(
            next.image_records(limit()).await?.is_empty(),
            "durable pin records remain"
        );
        let source = docker.inspect_image(&reference, limit()).await?;
        ensure!(
            source.id.as_deref()
                == Some(original.as_ref().context("captured source image")?.as_str()),
            "operator source tag changed or disappeared"
        );
        ensure!(
            matches!(
                docker
                    .inspect_image(alias.as_ref().context("captured private alias")?, limit())
                    .await,
                Err(DockerError::Status(StatusCode::NOT_FOUND))
            ),
            "private alias remains visible"
        );
        anyhow::Ok(())
    }
    .await;
    let wire_cleanup = wire.shutdown().await;
    setup?;
    recovery?;
    wire_cleanup?;
    evidence.complete();
    Ok(())
}
