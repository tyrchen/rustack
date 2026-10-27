use std::os::unix::fs::symlink;

use super::*;

async fn resolve(daemon: &mut Daemon) {
    daemon
        .next()
        .await
        .reply(
            200,
            &serde_json::to_vec(&metadata(&digest('a'), false)).unwrap(),
        )
        .await;
    daemon
        .next()
        .await
        .reply(
            200,
            &serde_json::to_vec(&metadata(&digest('a'), true)).unwrap(),
        )
        .await;
}
async fn matching(daemon: &mut Daemon) {
    daemon
        .next()
        .await
        .reply(
            200,
            &serde_json::to_vec(&metadata(&digest('a'), false)).unwrap(),
        )
        .await;
}

#[tokio::test]
async fn test_should_publish_intent_before_tag_and_commit_before_live_lease_or_delete() {
    let root = tempfile::tempdir().unwrap();
    let directory = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    Daemon::bind_fixture(&directory).await;
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let mut owner = ImageLeaseManager::start(
        wire.port()
            .bind_directory(&directory, deadline())
            .await
            .unwrap(),
        ImagePinScope::new(directory.clone()),
        config(1, 1),
    )
    .unwrap();
    let port = owner.port();
    let task = tokio::spawn(async move {
        port.prepare(
            &"worker:v1".parse().unwrap(),
            Architecture::Arm64,
            None,
            deadline(),
        )
        .await
    });
    resolve(&mut daemon).await;
    let tag = daemon.next().await;
    assert!(tag.head.starts_with("POST "));
    let records = directory.image_records(deadline()).await.unwrap();
    assert_eq!(records.len(), 1);
    assert!(!records[0].committed);
    tag.reply(201, b"").await;
    let inspected = daemon.next().await;
    assert!(directory.image_records(deadline()).await.unwrap()[0].committed);
    inspected
        .reply(
            200,
            &serde_json::to_vec(&metadata(&digest('a'), false)).unwrap(),
        )
        .await;
    daemon
        .next()
        .await
        .reply(
            200,
            &serde_json::to_vec(&manifests(&digest('a'), false)).unwrap(),
        )
        .await;
    let image = task.await.unwrap().unwrap();
    assert!(image.is_live());
    drop(image);
    matching(&mut daemon).await;
    matching(&mut daemon).await;
    let remove = daemon.next().await;
    assert!(remove.head.starts_with("DELETE "));
    assert!(directory.image_records(deadline()).await.unwrap()[0].committed);
    remove.reply(200, b"[]").await;
    daemon.next().await.reply(404, b"{}").await;
    owner.shutdown(deadline()).await.unwrap();
    assert_eq!(owner.port().status().pins, 0);
    assert_eq!(directory.image_records(deadline()).await.unwrap(), []);
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[tokio::test]
async fn test_should_retain_pin_capacity_when_no_effect_record_cannot_be_erased() {
    let root = tempfile::tempdir().unwrap();
    let directory = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    Daemon::bind_fixture(&directory).await;
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let mut owner = ImageLeaseManager::start(
        wire.port()
            .bind_directory(&directory, deadline())
            .await
            .unwrap(),
        ImagePinScope::new(directory.clone()),
        config(1, 1),
    )
    .unwrap();
    let port = owner.port();
    let task = tokio::spawn(async move {
        port.prepare(
            &"worker:v1".parse().unwrap(),
            Architecture::Arm64,
            None,
            deadline(),
        )
        .await
    });
    resolve(&mut daemon).await;
    let tag = daemon.next().await;
    assert_eq!(directory.image_records(deadline()).await.unwrap().len(), 1);
    let staging = root.path().join(".rustack-lambda-docker/pins.pending");
    symlink(root.path().join("outside"), &staging).unwrap();
    tag.reply(400, b"{}").await;
    assert!(matches!(
        task.await.unwrap(),
        Err(DockerError::Status(StatusCode::BAD_REQUEST))
    ));
    timeout(Duration::from_secs(2), async {
        while owner.port().status().quarantined == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(owner.port().status().pins, 1);
    assert!(
        timeout(Duration::from_millis(20), daemon.requests.recv())
            .await
            .is_err()
    );
    tokio::fs::remove_file(staging).await.unwrap();
    assert_eq!(directory.image_records(deadline()).await.unwrap().len(), 1);
    owner.shutdown(deadline()).await.unwrap();
    assert_eq!(owner.port().status().pins, 0);
    assert_eq!(directory.image_records(deadline()).await.unwrap(), []);
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_retain_acknowledgement_through_failed_commit_then_settle_absent_alias() {
    let root = tempfile::tempdir().unwrap();
    let directory = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    Daemon::bind_fixture(&directory).await;
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let mut owner = ImageLeaseManager::start(
        wire.port()
            .bind_directory(&directory, deadline())
            .await
            .unwrap(),
        ImagePinScope::new(directory.clone()),
        config(1, 1),
    )
    .unwrap();
    let port = owner.port();
    let task = tokio::spawn(async move {
        port.prepare(
            &"worker:v1".parse().unwrap(),
            Architecture::Arm64,
            None,
            deadline(),
        )
        .await
    });
    resolve(&mut daemon).await;
    let tag = daemon.next().await;
    let staging = root.path().join(".rustack-lambda-docker/pins.pending");
    symlink(root.path().join("outside"), &staging).unwrap();
    tag.reply(201, b"").await;
    assert!(matches!(
        task.await.unwrap(),
        Err(DockerError::Ownership(_) | DockerError::Quarantined)
    ));
    timeout(Duration::from_secs(2), async {
        while owner.port().status().quarantined == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(owner.port().status().pins, 1);
    assert!(
        timeout(Duration::from_millis(20), daemon.requests.recv())
            .await
            .is_err()
    );
    tokio::fs::remove_file(&staging).await.unwrap();
    let absent = timeout(Duration::from_secs(3), daemon.next())
        .await
        .unwrap();
    assert!(
        absent.head.starts_with("GET "),
        "acknowledged tag was replayed"
    );
    assert!(
        directory
            .image_records(deadline())
            .await
            .unwrap()
            .first()
            .unwrap()
            .committed
    );
    absent.reply(404, b"{}").await;
    owner.shutdown(deadline()).await.unwrap();
    assert_eq!(owner.port().status().pins, 0);
    assert_eq!(directory.image_records(deadline()).await.unwrap(), []);
    assert!(
        timeout(Duration::from_millis(20), daemon.requests.recv())
            .await
            .is_err()
    );
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_retain_positive_inspection_through_failed_commit_and_retire_without_retagging()
{
    let root = tempfile::tempdir().unwrap();
    let directory = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    Daemon::bind_fixture(&directory).await;
    let pin = ImagePinScope::new(directory.clone()).allocate().unwrap();
    let expected: Sha256Digest = digest('a').parse().unwrap();
    directory
        .prepare_image(ImageRecord::new(&pin, &expected).unwrap(), deadline())
        .await
        .unwrap();
    let mut mutation = MutationGuard::new();
    let receipt = mutation.receipt();
    mutation.issued();
    drop(mutation);
    let (_release, released) = watch::channel(());
    let progress = Arc::new(Progress {
        phase: watch::channel(Phase::Uncertain(expected, receipt)).0,
        released,
        active: Arc::new(AtomicBool::new(false)),
    });
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let worker = Worker::builder()
        .docker(
            wire.port()
                .bind_directory(&directory, deadline())
                .await
                .unwrap(),
        )
        .config(config(1, 1))
        .pin(pin)
        .progress(progress.clone())
        .control(watch::channel(false).0)
        .health(Arc::new(Health::default()))
        .cleanup(Arc::new(Semaphore::new(2)))
        .build();
    let staging = root.path().join(".rustack-lambda-docker/pins.pending");
    symlink(root.path().join("outside"), &staging).unwrap();
    let (clean, ()) = tokio::join!(worker.cleanup_step(), matching(&mut daemon));
    assert!(!clean, "unsafe storage did not block durable commitment");
    assert!(matches!(*progress.phase.borrow(), Phase::Committing(_)));
    assert!(
        timeout(Duration::from_millis(20), daemon.requests.recv())
            .await
            .is_err()
    );
    tokio::fs::remove_file(staging).await.unwrap();
    tokio::join!(worker.retire(), async {
        let absent = daemon.next().await;
        assert!(
            absent.head.starts_with("GET "),
            "matching proof was replaced with a repeated mutation"
        );
        assert!(
            directory
                .image_records(deadline())
                .await
                .unwrap()
                .first()
                .unwrap()
                .committed
        );
        absent.reply(404, b"{}").await;
    });
    assert!(matches!(*progress.phase.borrow(), Phase::Clean));
    assert_eq!(directory.image_records(deadline()).await.unwrap(), []);
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[rstest::rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn test_should_erase_preparing_or_definitely_unissued_intent_without_daemon_mutation(
    #[case] receipt: bool,
) {
    let root = tempfile::tempdir().unwrap();
    let directory = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    Daemon::bind_fixture(&directory).await;
    let pin = ImagePinScope::new(directory.clone()).allocate().unwrap();
    let expected: Sha256Digest = digest('a').parse().unwrap();
    directory
        .prepare_image(ImageRecord::new(&pin, &expected).unwrap(), deadline())
        .await
        .unwrap();
    let mutation = MutationGuard::new();
    let unissued = mutation.receipt();
    drop(mutation);
    let phase = if receipt {
        Phase::Uncertain(expected, unissued)
    } else {
        Phase::Preparing(expected)
    };
    let (_release, released) = watch::channel(());
    let progress = Arc::new(Progress {
        phase: watch::channel(phase).0,
        released,
        active: Arc::new(AtomicBool::new(false)),
    });
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    Worker::builder()
        .docker(
            wire.port()
                .bind_directory(&directory, deadline())
                .await
                .unwrap(),
        )
        .config(config(1, 1))
        .pin(pin)
        .progress(progress.clone())
        .control(watch::channel(false).0)
        .health(Arc::new(Health::default()))
        .cleanup(Arc::new(Semaphore::new(2)))
        .build()
        .retire()
        .await;
    assert!(matches!(*progress.phase.borrow(), Phase::Clean));
    assert_eq!(directory.image_records(deadline()).await.unwrap(), []);
    assert!(
        timeout(Duration::from_millis(20), daemon.requests.recv())
            .await
            .is_err()
    );
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
