use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

use rstest::rstest;
use serde_json::{Value, from_slice, json, to_vec};
use tempfile::TempDir;
use tokio::{
    sync::watch,
    time::{Instant, timeout},
};
use uuid::Uuid;

use super::*;
use crate::{
    artifact::{Architecture, ImageLeaseContext, PinnedImage, ResolvedImage},
    docker::{
        DirectoryLease, DockerError, DockerWire,
        code::tests::{elf, prepare, source, zip},
        tests::{Daemon, IncomingRequest},
    },
};

pub(in crate::docker) fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
fn config() -> CodeVolumeConfig {
    CodeVolumeConfig::new(2, 2, Duration::from_secs(1), Duration::from_secs(2)).unwrap()
}
pub(in crate::docker) async fn fixture() -> (
    TempDir,
    DirectoryLease,
    Daemon,
    DockerWire,
    crate::docker::DockerPort,
) {
    let root = tempfile::tempdir().unwrap();
    let directory = DirectoryLease::acquire_code(root.path().into(), deadline())
        .await
        .unwrap();
    let daemon = Daemon::new();
    let wire = daemon.wire().await;
    let port = wire
        .port()
        .bind_directory(&directory, deadline())
        .await
        .unwrap();
    (root, directory, daemon, wire, port)
}
pub(in crate::docker) fn base(
    directory: &DirectoryLease,
) -> (PinnedImage, watch::Sender<bool>, watch::Receiver<()>) {
    let (release, released) = watch::channel(());
    let (stop, stopping) = watch::channel(false);
    let base = PinnedImage::with_context(
        ResolvedImage::fixture(Architecture::Arm64),
        1024.try_into().unwrap(),
        release,
        Arc::new(AtomicBool::new(true)),
        stopping,
        ImageLeaseContext::Owned {
            directory: directory.clone(),
            preparation: None,
        },
    );
    (base, stop, released)
}
pub(in crate::docker) async fn code() -> (TempDir, crate::docker::DockerCodeArchive) {
    let (root, artifact) = source(zip(&[("bootstrap", &elf(Architecture::Arm64), 0o755)])).await;
    (root, prepare(artifact).await)
}
pub(in crate::docker) fn target(request: &IncomingRequest) -> String {
    request
        .head
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .into()
}
pub(in crate::docker) async fn reply(request: IncomingRequest, value: &Value) {
    request.reply(200, &to_vec(value).unwrap()).await;
}
fn image() -> Value {
    json!({"Id":format!("sha256:{}","a".repeat(64)),"Os":"linux","Architecture":"arm64","Size":1024,"Config":{"Labels":{"com.amazonaws.lambda.platform.kernel":"k510ga"}}})
}
pub(in crate::docker) async fn begin_volume(
    daemon: &mut Daemon,
    directory: &DirectoryLease,
) -> (IncomingRequest, Value) {
    let request = daemon.next().await;
    assert!(target(&request).contains("/images/sha256%3A"));
    reply(request, &image()).await;
    let request = daemon.next().await;
    assert!(target(&request).ends_with("/volumes/create"));
    let body: Value = from_slice(&request.body).unwrap();
    let saved = directory
        .code_records(deadline())
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.volume_key().unwrap().as_str() == body["Name"].as_str().unwrap())
        .unwrap();
    assert_eq!(
        saved.volume,
        crate::docker::ownership::codes::VolumeState::Uncertain
    );
    let volume = json!({"Name":body["Name"],"Labels":body["Labels"],"Driver":"local","Scope":"local","Options":{},"Mountpoint":"/not-a-host-path"});
    (request, volume)
}
async fn begin_loader(
    daemon: &mut Daemon,
    directory: &DirectoryLease,
    volume: &Value,
) -> (IncomingRequest, Value) {
    let request = daemon.next().await;
    assert!(target(&request).contains("/containers/create?"));
    let mut body: Value = from_slice(&request.body).unwrap();
    assert_eq!(body["HostConfig"]["NetworkMode"], "none");
    assert_eq!(body["HostConfig"]["ReadonlyRootfs"], true);
    assert_eq!(
        body["HostConfig"]["Mounts"][0]["VolumeOptions"]["NoCopy"],
        true
    );
    let name = form_urlencoded::parse(target(&request).split_once('?').unwrap().1.as_bytes())
        .find(|(k, _)| k == "name")
        .unwrap()
        .1
        .into_owned();
    let saved = directory
        .code_records(deadline())
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.volume_key().unwrap().as_str() == volume["Name"].as_str().unwrap())
        .unwrap();
    assert_eq!(
        saved.loader,
        crate::docker::ownership::codes::LoaderState::Uncertain
    );
    body["Labels"]["com.amazonaws.lambda.platform.kernel"] = json!("k510ga");
    let inspected = json!({"Id":"c".repeat(64),"Name":format!("/{name}"),"Image":body["Image"],"Config":{"Image":body["Image"],"Labels":body["Labels"]},"State":{"Status":"created","Running":false,"StartedAt":"0001-01-01T00:00:00Z"},"Mounts":[{"Type":"volume","Name":volume["Name"],"Driver":"local","Destination":"/var/task","RW":true}]});
    (request, inspected)
}
pub(in crate::docker) async fn allocated(
    daemon: &mut Daemon,
    directory: &DirectoryLease,
) -> (Value, Value) {
    let (request, volume) = begin_volume(daemon, directory).await;
    request.reply(201, &to_vec(&volume).unwrap()).await;
    let (request, inspected) = begin_loader(daemon, directory, &volume).await;
    request
        .reply(
            201,
            &to_vec(&json!({"Id":"c".repeat(64),"Warnings":[]})).unwrap(),
        )
        .await;
    reply(daemon.next().await, &inspected).await;
    (volume, inspected)
}
pub(in crate::docker) async fn finish_loader(daemon: &mut Daemon, inspected: &Value) {
    reply(daemon.next().await, inspected).await;
    reply(daemon.next().await, inspected).await;
    let request = daemon.next().await;
    assert!(request.head.starts_with("DELETE "));
    assert!(target(&request).ends_with("?force=false"));
    request.reply(204, b"").await;
    daemon.next().await.reply(404, b"{}").await;
}
pub(in crate::docker) async fn stage(daemon: &mut Daemon, directory: &DirectoryLease) -> Value {
    let (volume, inspected) = allocated(daemon, directory).await;
    let request = daemon.next().await;
    assert!(request.head.starts_with("PUT "));
    assert!(
        request
            .head
            .to_ascii_lowercase()
            .contains("content-type: application/x-tar")
    );
    assert!(target(&request).contains("copyUIDGID=false"));
    request.reply(200, b"").await;
    finish_loader(daemon, &inspected).await;
    volume
}
pub(in crate::docker) async fn retire(daemon: &mut Daemon, volume: &Value) {
    let request = daemon.next().await;
    assert!(target(&request).contains("/volumes/"));
    reply(request, volume).await;
    let request = daemon.next().await;
    assert!(request.head.starts_with("DELETE "));
    assert!(target(&request).ends_with("?force=false"));
    request.reply(204, b"").await;
    daemon.next().await.reply(404, b"{}").await;
}
pub(in crate::docker) async fn empty(port: &CodeVolumePort) {
    timeout(Duration::from_secs(5), async {
        while port.status().volumes != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[rstest]
#[case(0, 1, 1, 1)]
#[case(33, 1, 1, 1)]
#[case(1, 2, 1, 1)]
#[case(2, 0, 1, 1)]
#[case(3, 3, 1, 1)]
#[case(2, 2, 0, 1)]
#[case(2, 2, 61, 1)]
#[case(2, 2, 1, 0)]
#[case(2, 2, 1, 601)]
fn test_should_reject_invalid_admission_and_io_settings(
    #[case] volumes: u8,
    #[case] jobs: u8,
    #[case] io: u64,
    #[case] upload: u64,
) {
    assert!(
        CodeVolumeConfig::new(
            volumes,
            jobs,
            Duration::from_secs(io),
            Duration::from_secs(upload)
        )
        .is_err()
    );
}
#[test]
fn test_should_require_nonnil_resource_revision_and_generate_fresh_environments() {
    assert!(CodeVolumeIdentity::new(Uuid::nil(), Uuid::new_v4()).is_err());
    assert!(CodeVolumeIdentity::new(Uuid::new_v4(), Uuid::nil()).is_err());
    let a = CodeVolumeIdentity::new(Uuid::new_v4(), Uuid::new_v4()).unwrap();
    let b = CodeVolumeIdentity::new(a.resource, a.revision).unwrap();
    assert_ne!(a.environment_id(), b.environment_id());
}
#[tokio::test]
async fn test_should_exclude_replacement_producers_until_original_owner_joins() {
    let (_root, directory, daemon, mut wire, port) = fixture().await;
    let mut owner = CodeVolumeManager::start(port.clone(), directory.clone(), config()).unwrap();
    assert!(CodeVolumeManager::start(port.clone(), directory.clone(), config()).is_err());
    let stale = owner.port();
    owner.shutdown(deadline()).await.unwrap();
    assert!(!stale.status().accepting);
    let mut next = CodeVolumeManager::start(port, directory, config()).unwrap();
    next.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[tokio::test]
async fn test_should_prepare_clone_and_retire_code_only_after_last_ready_release() {
    let (_root, directory, mut daemon, mut wire, docker) = fixture().await;
    let mut owner = CodeVolumeManager::start(docker, directory.clone(), config()).unwrap();
    let port = owner.port();
    let (_source, archive) = code().await;
    let (base, _stop, mut released) = base(&directory);
    let id = CodeVolumeIdentity::new(Uuid::new_v4(), Uuid::new_v4()).unwrap();
    let env = id.environment_id();
    let (ready, volume) = tokio::join!(
        port.prepare(id, archive, base, deadline()),
        stage(&mut daemon, &directory)
    );
    let ready = ready.unwrap();
    assert_eq!(ready.identity().environment_id(), env);
    assert_eq!(ready.mount().unwrap().read_only, Some(true));
    assert!(ready.is_live());
    assert_eq!(port.status().volumes, 1);
    assert_eq!(port.status().preparing, 0);
    assert_eq!(port.status().quarantined, 0);
    let clone = ready.clone();
    drop(ready);
    assert!(!released.has_changed().unwrap());
    assert!(
        timeout(Duration::from_millis(20), daemon.requests.recv())
            .await
            .is_err()
    );
    drop(clone);
    retire(&mut daemon, &volume).await;
    empty(&port).await;
    let _ = released.changed().await;
    assert!(released.has_changed().is_err());
    assert_eq!(directory.code_records(deadline()).await.unwrap(), []);
    owner.shutdown(deadline()).await.unwrap();
    owner.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

async fn queued(port: &CodeVolumePort) {
    timeout(Duration::from_secs(3), async {
        while port.ready_handoffs() != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
fn poll_once<F: std::future::Future>(future: std::pin::Pin<&mut F>) -> std::task::Poll<F::Output> {
    let waker = std::task::Waker::noop();
    let mut context = std::task::Context::from_waker(waker);
    future.poll(&mut context)
}

#[rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn test_should_expire_or_abandon_ready_without_repolling_its_live_observer(
    #[case] abandon: bool,
) {
    let (_root, directory, mut daemon, mut wire, docker) = fixture().await;
    let mut owner = CodeVolumeManager::start(docker, directory.clone(), config()).unwrap();
    let port = owner.port();
    let (_source, archive) = code().await;
    let (base, _stop, mut released) = base(&directory);
    let at = Instant::now() + Duration::from_secs(if abandon { 30 } else { 1 });
    let mut waiting = Box::pin(port.prepare(
        CodeVolumeIdentity::new(Uuid::new_v4(), Uuid::new_v4()).unwrap(),
        archive,
        base,
        at,
    ));
    assert!(poll_once(waiting.as_mut()).is_pending());
    let volume = stage(&mut daemon, &directory).await;
    queued(&port).await;
    assert_eq!(port.status().preparing, 1);
    assert!(!released.has_changed().unwrap());
    if abandon {
        drop(waiting);
    } else {
        // This future stays alive, with a queued Ready value, throughout physical cleanup.
        let ((), ()) = tokio::join!(retire(&mut daemon, &volume), empty(&port));
        assert_eq!(port.status().preparing, 0);
        let _ = released.changed().await;
        assert!(released.has_changed().is_err());
        assert!(matches!(waiting.await, Err(DockerError::Deadline)));
        owner.shutdown(deadline()).await.unwrap();
        wire.shutdown().await.unwrap();
        daemon.close().await;
        return;
    }
    retire(&mut daemon, &volume).await;
    empty(&port).await;
    let _ = released.changed().await;
    assert!(released.has_changed().is_err());
    assert_eq!(port.status().preparing, 0);
    owner.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_honor_consumption_before_deadline_even_when_owner_processes_it_later() {
    let (_root, directory, mut daemon, mut wire, docker) = fixture().await;
    let mut owner = CodeVolumeManager::start(docker, directory.clone(), config()).unwrap();
    let port = owner.port();
    let (_source, archive) = code().await;
    let (base, _stop, _released) = base(&directory);
    let mut waiting = Box::pin(port.prepare(
        CodeVolumeIdentity::new(Uuid::new_v4(), Uuid::new_v4()).unwrap(),
        archive,
        base,
        deadline(),
    ));
    assert!(poll_once(waiting.as_mut()).is_pending());
    let volume = stage(&mut daemon, &directory).await;
    queued(&port).await;
    let std::task::Poll::Ready(Ok(ready)) = poll_once(waiting.as_mut()) else {
        panic!("queued on-time handoff not consumed");
    };
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(11)).await;
    tokio::time::resume();
    timeout(Duration::from_secs(3), async {
        while port.ready_handoffs() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(ready.is_live());
    assert_eq!(port.status().volumes, 1);
    assert_eq!(port.status().preparing, 0);
    drop(ready);
    retire(&mut daemon, &volume).await;
    empty(&port).await;
    owner.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_reject_equal_uuid_foreign_acquisition_and_bad_deadlines_before_io() {
    let (_root, directory, mut daemon, mut wire, docker) = fixture().await;
    let mut owner = CodeVolumeManager::start(docker, directory.clone(), config()).unwrap();
    let port = owner.port();
    let (_source, archive) = code().await;
    let foreign = DirectoryLease::testing(directory.owner_id(), directory.run_id());
    let (foreign_base, _foreign_stop, _) = base(&foreign);
    assert!(matches!(
        port.prepare(
            CodeVolumeIdentity::new(Uuid::new_v4(), Uuid::new_v4()).unwrap(),
            archive.clone(),
            foreign_base,
            deadline()
        )
        .await,
        Err(DockerError::Configuration)
    ));
    let (base, _stop, _) = base(&directory);
    for at in [Instant::now(), Instant::now() + Duration::from_secs(3601)] {
        assert!(matches!(
            port.prepare(
                CodeVolumeIdentity::new(Uuid::new_v4(), Uuid::new_v4()).unwrap(),
                archive.clone(),
                base.clone(),
                at
            )
            .await,
            Err(DockerError::Configuration | DockerError::Deadline)
        ));
    }
    assert_eq!(port.status().volumes, 0);
    assert_eq!(port.status().preparing, 0);
    assert!(
        timeout(Duration::from_millis(20), daemon.requests.recv())
            .await
            .is_err()
    );
    owner.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_revoke_ready_and_retire_when_the_source_owner_revokes_its_pin() {
    let (_root, directory, mut daemon, mut wire, docker) = fixture().await;
    let mut owner = CodeVolumeManager::start(docker, directory.clone(), config()).unwrap();
    let port = owner.port();
    let (_source, archive) = code().await;
    let (base, stop, mut released) = base(&directory);
    let (ready, volume) = tokio::join!(
        port.prepare(
            CodeVolumeIdentity::new(Uuid::new_v4(), Uuid::new_v4()).unwrap(),
            archive,
            base,
            deadline()
        ),
        stage(&mut daemon, &directory)
    );
    let ready = ready.unwrap();
    stop.send_replace(true);
    assert!(!ready.is_live());
    assert!(ready.mount().is_err());
    retire(&mut daemon, &volume).await;
    empty(&port).await;
    let _ = released.changed().await;
    assert!(released.has_changed().is_err());
    drop(ready);
    owner.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn test_should_join_original_accepted_upload_before_cleanup_after_expired_shutdown_or_root_loss(
    #[case] root_loss: bool,
) {
    let (_root, directory, mut daemon, mut wire, mut docker) = fixture().await;
    let (entered, mut observed) = watch::channel(false);
    let (release, released) = watch::channel(false);
    docker.raw.upload_completion = Some(crate::docker::CompletionGate {
        entered,
        release: released,
    });
    let mut owner = CodeVolumeManager::start(
        docker.clone(),
        directory.clone(),
        CodeVolumeConfig::new(2, 2, Duration::from_secs(1), Duration::from_secs(1)).unwrap(),
    )
    .unwrap();
    let port = owner.port();
    let reservation = Arc::new(tokio::sync::Semaphore::new(2));
    let (_source, artifact) = source(zip(&[("bootstrap", &elf(Architecture::Arm64), 0o755)])).await;
    let archive = crate::docker::code::tests::prepare_with(artifact, reservation.clone()).await;
    let (base, _stop, mut source_released) = base(&directory);
    let call = port.clone();
    let waiting = tokio::spawn(async move {
        call.prepare(
            CodeVolumeIdentity::new(Uuid::new_v4(), Uuid::new_v4()).unwrap(),
            archive,
            base,
            deadline(),
        )
        .await
    });
    let (volume, inspected) = allocated(&mut daemon, &directory).await;
    let upload = daemon.next().await;
    assert!(upload.head.starts_with("PUT "));
    timeout(Duration::from_secs(3), observed.wait_for(|v| *v))
        .await
        .unwrap()
        .unwrap();
    // The original envelope is retained after its IO deadline and driver join,
    // before response delivery; a timed observer must not replace that ownership.
    assert_eq!(reservation.available_permits(), 1);
    assert_eq!(port.status().volumes, 1);
    assert_eq!(port.status().preparing, 1);
    assert!(!source_released.has_changed().unwrap());
    if root_loss {
        owner.abort_root();
    } else {
        waiting.abort();
    }
    assert!(matches!(
        owner
            .shutdown(Instant::now() + Duration::from_millis(30))
            .await,
        Err(DockerError::Deadline)
    ));
    assert!(CodeVolumeManager::start(docker, directory.clone(), config()).is_err());
    assert!(
        timeout(Duration::from_millis(30), daemon.requests.recv())
            .await
            .is_err()
    );
    assert_eq!(reservation.available_permits(), 1);
    assert_eq!(port.status().preparing, 1);
    release.send_replace(true);
    drop(upload);
    finish_loader(&mut daemon, &inspected).await;
    retire(&mut daemon, &volume).await;
    empty(&port).await;
    assert_eq!(reservation.available_permits(), 2);
    assert_eq!(port.status().preparing, 0);
    let _ = source_released.changed().await;
    assert!(source_released.has_changed().is_err());
    if root_loss {
        assert!(matches!(
            waiting.await.unwrap(),
            Err(DockerError::Deadline | DockerError::Stopped | DockerError::Worker)
        ));
        assert!(matches!(
            owner.shutdown(deadline()).await,
            Err(DockerError::Worker)
        ));
        assert!(matches!(
            owner.shutdown(deadline()).await,
            Err(DockerError::Worker)
        ));
    } else {
        assert!(waiting.await.unwrap_err().is_cancelled());
        owner.shutdown(deadline()).await.unwrap();
    }
    assert!(owner.physically_settled());
    assert_eq!(directory.code_records(deadline()).await.unwrap(), []);
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn test_should_retain_issued_unknown_create_until_late_matching_observation(
    #[case] loader: bool,
) {
    let (_root, directory, mut daemon, mut wire, docker) = fixture().await;
    let mut owner = CodeVolumeManager::start(docker.clone(), directory.clone(), config()).unwrap();
    let port = owner.port();
    let (_source, archive) = code().await;
    let (base, _stop, mut source_released) = base(&directory);
    let call = port.clone();
    let waiting = tokio::spawn(async move {
        call.prepare(
            CodeVolumeIdentity::new(Uuid::new_v4(), Uuid::new_v4()).unwrap(),
            archive,
            base,
            deadline(),
        )
        .await
    });
    let (volume_request, volume) = begin_volume(&mut daemon, &directory).await;
    let (held, inspected) = if loader {
        volume_request.reply(201, &to_vec(&volume).unwrap()).await;
        let (held, inspected) = begin_loader(&mut daemon, &directory, &volume).await;
        (held, Some(inspected))
    } else {
        (volume_request, None)
    };
    assert!(matches!(waiting.await.unwrap(), Err(DockerError::Deadline)));
    drop(held);
    daemon.next().await.reply(404, b"{}").await;
    timeout(Duration::from_secs(3), async {
        while port.status().quarantined == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(port.status().volumes, 1);
    assert_eq!(port.status().preparing, 1);
    assert!(!source_released.has_changed().unwrap());
    assert!(CodeVolumeManager::start(docker, directory.clone(), config()).is_err());
    if let Some(inspected) = inspected {
        reply(daemon.next().await, &inspected).await;
        // Unknown name is now persisted as a known full ID before the recheck/delete.
        reply(daemon.next().await, &inspected).await;
        let request = daemon.next().await;
        assert!(request.head.starts_with("DELETE "));
        assert!(target(&request).contains(&"c".repeat(64)));
        request.reply(204, b"").await;
        daemon.next().await.reply(404, b"{}").await;
    } else {
        reply(daemon.next().await, &volume).await;
    }
    retire(&mut daemon, &volume).await;
    empty(&port).await;
    assert_eq!(port.status().preparing, 0);
    assert_eq!(port.status().quarantined, 0);
    assert!(!port.status().worker_failed);
    let _ = source_released.changed().await;
    assert!(source_released.has_changed().is_err());
    owner.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_keep_create_owned_after_cancel_then_retire_positive_ack_without_loader_creation()
 {
    let (_root, directory, mut daemon, mut wire, docker) = fixture().await;
    let mut owner = CodeVolumeManager::start(docker, directory.clone(), config()).unwrap();
    let port = owner.port();
    let (_source, archive) = code().await;
    let (base, _stop, mut source_released) = base(&directory);
    let call = port.clone();
    let waiting = tokio::spawn(async move {
        call.prepare(
            CodeVolumeIdentity::new(Uuid::new_v4(), Uuid::new_v4()).unwrap(),
            archive,
            base,
            deadline(),
        )
        .await
    });
    let (request, volume) = begin_volume(&mut daemon, &directory).await;
    waiting.abort();
    assert!(waiting.await.unwrap_err().is_cancelled());
    assert_eq!(port.status().preparing, 1);
    assert!(!source_released.has_changed().unwrap());
    request.reply(201, &to_vec(&volume).unwrap()).await;
    retire(&mut daemon, &volume).await;
    empty(&port).await;
    assert_eq!(directory.code_records(deadline()).await.unwrap(), []);
    let _ = source_released.changed().await;
    assert!(source_released.has_changed().is_err());
    owner.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[rstest]
#[case(1)]
#[case(5)]
#[case(8)]
#[tokio::test]
async fn test_should_reap_worker_root_and_unissued_stage_panics_without_creating_resources(
    #[case] point: u8,
) {
    let (_root, directory, mut daemon, mut wire, docker) = fixture().await;
    let mut owner = CodeVolumeManager::start(docker, directory.clone(), config()).unwrap();
    let port = owner.port();
    port.set_fault(point);
    let (_source, archive) = code().await;
    let (base, _stop, mut source_released) = base(&directory);
    let call = port.clone();
    let waiting = tokio::spawn(async move {
        call.prepare(
            CodeVolumeIdentity::new(Uuid::new_v4(), Uuid::new_v4()).unwrap(),
            archive,
            base,
            deadline(),
        )
        .await
    });
    if point == 8 {
        reply(daemon.next().await, &image()).await;
    }
    assert!(matches!(
        waiting.await.unwrap(),
        Err(DockerError::Worker | DockerError::Stopped)
    ));
    empty(&port).await;
    assert_eq!(directory.code_records(deadline()).await.unwrap(), []);
    assert_eq!(port.status().preparing, 0);
    assert!(port.status().worker_failed);
    let _ = source_released.changed().await;
    assert!(source_released.has_changed().is_err());
    assert!(matches!(
        owner.shutdown(deadline()).await,
        Err(DockerError::Worker)
    ));
    assert!(
        timeout(Duration::from_millis(20), daemon.requests.recv())
            .await
            .is_err()
    );
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[rstest]
#[case(3)]
#[case(9)]
#[tokio::test]
async fn test_should_reap_stage_panics_after_volume_commit_or_unissued_loader(#[case] point: u8) {
    let (_root, directory, mut daemon, mut wire, docker) = fixture().await;
    let mut owner = CodeVolumeManager::start(docker, directory.clone(), config()).unwrap();
    let port = owner.port();
    port.set_fault(point);
    let (_source, archive) = code().await;
    let (base, _stop, _released) = base(&directory);
    let call = port.clone();
    let waiting = tokio::spawn(async move {
        call.prepare(
            CodeVolumeIdentity::new(Uuid::new_v4(), Uuid::new_v4()).unwrap(),
            archive,
            base,
            deadline(),
        )
        .await
    });
    let (request, volume) = begin_volume(&mut daemon, &directory).await;
    request.reply(201, &to_vec(&volume).unwrap()).await;
    assert!(matches!(waiting.await.unwrap(), Err(DockerError::Worker)));
    retire(&mut daemon, &volume).await;
    empty(&port).await;
    assert_eq!(directory.code_records(deadline()).await.unwrap(), []);
    assert!(matches!(
        owner.shutdown(deadline()).await,
        Err(DockerError::Worker)
    ));
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[rstest]
#[case(6)]
#[case(7)]
#[tokio::test]
async fn test_should_retain_retirement_witness_across_cleanup_panic_or_lost_erase_ack(
    #[case] point: u8,
) {
    let (_root, directory, mut daemon, mut wire, docker) = fixture().await;
    let mut owner = CodeVolumeManager::start(docker, directory.clone(), config()).unwrap();
    let port = owner.port();
    let (_source, archive) = code().await;
    let (base, _stop, mut source_released) = base(&directory);
    let (ready, volume) = tokio::join!(
        port.prepare(
            CodeVolumeIdentity::new(Uuid::new_v4(), Uuid::new_v4()).unwrap(),
            archive,
            base,
            deadline()
        ),
        stage(&mut daemon, &directory)
    );
    let ready = ready.unwrap();
    port.set_fault(point);
    drop(ready);
    retire(&mut daemon, &volume).await;
    // The reaper must retain quota/source after physical absence until the
    // original erased/retired journal state is positively reconciled.
    timeout(Duration::from_secs(3), async {
        while if point == 6 {
            !port.status().worker_failed
        } else {
            port.status().quarantined == 0
        } {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(port.status().volumes, 1);
    assert!(!source_released.has_changed().unwrap());
    empty(&port).await;
    assert_eq!(directory.code_records(deadline()).await.unwrap(), []);
    let _ = source_released.changed().await;
    assert!(source_released.has_changed().is_err());
    let result = owner.shutdown(deadline()).await;
    if point == 6 {
        assert!(matches!(result, Err(DockerError::Worker)));
    } else {
        result.unwrap();
    }
    assert!(
        timeout(Duration::from_millis(20), daemon.requests.recv())
            .await
            .is_err()
    );
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[rstest]
#[case(true)]
#[case(false)]
#[tokio::test(start_paused = true)]
async fn test_should_reject_retirement_completion_observed_after_deadline_and_allow_reobserve(
    #[case] positive: bool,
) {
    let (settled, received) = watch::channel(false);
    let mut receipt = CodeRetirement::new(received);
    let mut waiting = Box::pin(receipt.wait(Instant::now() + Duration::from_secs(1)));
    assert!(poll_once(waiting.as_mut()).is_pending());
    if positive {
        settled.send_replace(true);
    }
    drop(settled);
    tokio::time::advance(Duration::from_secs(2)).await;
    assert!(matches!(
        poll_once(waiting.as_mut()),
        std::task::Poll::Ready(Err(DockerError::Deadline))
    ));
    drop(waiting);
    let result = receipt.wait(deadline()).await;
    if positive {
        result.unwrap();
    } else {
        assert!(matches!(result, Err(DockerError::Worker)));
    }
}

#[tokio::test]
async fn test_should_publish_no_effect_retirement_when_stopping_queued_preparation() {
    let (_root, directory, mut daemon, mut wire, docker) = fixture().await;
    let mut owner = CodeVolumeManager::start(docker, directory.clone(), config()).unwrap();
    let port = owner.port();
    let (_source, archive) = code().await;
    let (base, _stop, _released) = base(&directory);
    let preparation = port
        .start_preparation(
            CodeVolumeIdentity::new(Uuid::new_v4(), Uuid::new_v4()).unwrap(),
            archive,
            base,
            deadline(),
        )
        .unwrap();
    let mut retired = preparation.retirement();
    // No await between enqueue and stop: biased stop discards this unissued envelope.
    owner.shutdown(deadline()).await.unwrap();
    retired.wait(deadline()).await.unwrap();
    assert!(preparation.wait().await.is_err());
    assert_eq!(port.status().volumes, 0);
    assert_eq!(directory.code_records(deadline()).await.unwrap(), []);
    assert!(
        timeout(Duration::from_millis(20), daemon.next())
            .await
            .is_err()
    );
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_observe_same_physical_retirement_after_failed_prepare_and_expired_receipt() {
    let (_root, directory, mut daemon, mut wire, docker) = fixture().await;
    let mut owner = CodeVolumeManager::start(docker, directory.clone(), config()).unwrap();
    let port = owner.port();
    let (_source, archive) = code().await;
    let (base, _stop, _released) = base(&directory);
    let preparation = port
        .start_preparation(
            CodeVolumeIdentity::new(Uuid::new_v4(), Uuid::new_v4()).unwrap(),
            archive,
            base,
            deadline(),
        )
        .unwrap();
    let mut retired = preparation.retirement();
    let (request, volume) = begin_volume(&mut daemon, &directory).await;
    drop(preparation);
    request.reply(201, &to_vec(&volume).unwrap()).await;
    let inspect = daemon.next().await;
    assert!(matches!(
        retired
            .wait(Instant::now() + Duration::from_millis(10))
            .await,
        Err(DockerError::Deadline)
    ));
    assert_eq!(port.status().volumes, 1);
    reply(inspect, &volume).await;
    let delete = daemon.next().await;
    assert!(delete.head.starts_with("DELETE "));
    delete.reply(204, b"").await;
    daemon.next().await.reply(404, b"{}").await;
    retired.wait(deadline()).await.unwrap();
    empty(&port).await;
    owner.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[rstest]
#[case(2)]
#[case(4)]
#[tokio::test]
async fn test_should_reap_panics_with_known_loader_without_starting_it(#[case] point: u8) {
    let (_root, directory, mut daemon, mut wire, docker) = fixture().await;
    let mut owner = CodeVolumeManager::start(docker, directory.clone(), config()).unwrap();
    let port = owner.port();
    port.set_fault(point);
    let (_source, archive) = code().await;
    let (base, _stop, _released) = base(&directory);
    let preparation = port
        .start_preparation(
            CodeVolumeIdentity::new(Uuid::new_v4(), Uuid::new_v4()).unwrap(),
            archive,
            base,
            deadline(),
        )
        .unwrap();
    let mut retired = preparation.retirement();
    let waiting = tokio::spawn(preparation.wait());
    let volume = if point == 2 {
        stage(&mut daemon, &directory).await
    } else {
        let (volume, inspected) = allocated(&mut daemon, &directory).await;
        finish_loader(&mut daemon, &inspected).await;
        volume
    };
    assert!(matches!(waiting.await.unwrap(), Err(DockerError::Worker)));
    retire(&mut daemon, &volume).await;
    retired.wait(deadline()).await.unwrap();
    empty(&port).await;
    assert!(matches!(
        owner.shutdown(deadline()).await,
        Err(DockerError::Worker)
    ));
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[rstest]
#[case(0)]
#[case(1)]
#[case(2)]
#[case(3)]
#[case(4)]
#[case(5)]
#[case(6)]
#[case(7)]
#[case(8)]
#[tokio::test]
async fn test_should_reject_unsafe_or_changed_base_before_durable_or_daemon_effects(
    #[case] invalid: u8,
) {
    let (_root, directory, mut daemon, mut wire, docker) = fixture().await;
    let mut owner = CodeVolumeManager::start(docker, directory.clone(), config()).unwrap();
    let port = owner.port();
    let (_source, archive) = code().await;
    let (base, _stop, _released) = base(&directory);
    let preparation = port
        .start_preparation(
            CodeVolumeIdentity::new(Uuid::new_v4(), Uuid::new_v4()).unwrap(),
            archive,
            base,
            deadline(),
        )
        .unwrap();
    let mut retired = preparation.retirement();
    let waiting = tokio::spawn(preparation.wait());
    let mut value = image();
    match invalid {
        0 => value["Id"] = json!(format!("sha256:{}", "b".repeat(64))),
        1 => value["Os"] = json!("windows"),
        2 => value["Architecture"] = json!("amd64"),
        3 => value["Config"]["Volumes"] = json!({"/inherited":{}}),
        4 => value["Config"]["Labels"] = json!({"rustack.lambda.ownerId":"foreign"}),
        5 => value["Config"]["Labels"] = json!({"bad/key":"value"}),
        6 => value["Config"]["Labels"] = json!({"key":"x".repeat(257)}),
        7 => value["Config"]["Labels"] = json!({"key":"line\nbreak"}),
        _ => {
            value["Config"]["Labels"] =
                Value::Object((0..57).map(|i| (format!("label{i}"), json!("v"))).collect());
        }
    }
    reply(daemon.next().await, &value).await;
    assert!(matches!(
        waiting.await.unwrap(),
        Err(DockerError::Quarantined)
    ));
    retired.wait(deadline()).await.unwrap();
    empty(&port).await;
    assert_eq!(directory.code_records(deadline()).await.unwrap(), []);
    assert!(
        timeout(Duration::from_millis(20), daemon.next())
            .await
            .is_err()
    );
    owner.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_bound_queued_admission_and_reject_architecture_before_io() {
    let (_root, directory, mut daemon, mut wire, docker) = fixture().await;
    let mut owner = CodeVolumeManager::start(docker, directory.clone(), config()).unwrap();
    let port = owner.port();
    let (_source, archive) = code().await;
    let (base, _stop, _released) = base(&directory);
    let (_other, artifact) = source(zip(&[("bootstrap", &elf(Architecture::X86_64), 0o755)])).await;
    let foreign =
        crate::docker::DockerCodeArchive::prepare(artifact, Architecture::X86_64, deadline())
            .await
            .unwrap();
    let identity = || CodeVolumeIdentity::new(Uuid::new_v4(), Uuid::new_v4()).unwrap();
    assert!(matches!(
        port.start_preparation(identity(), foreign, base.clone(), deadline()),
        Err(DockerError::Configuration)
    ));
    let first = port
        .start_preparation(identity(), archive.clone(), base.clone(), deadline())
        .unwrap();
    let second = port
        .start_preparation(identity(), archive.clone(), base.clone(), deadline())
        .unwrap();
    assert!(matches!(
        port.start_preparation(identity(), archive, base, deadline()),
        Err(DockerError::Busy)
    ));
    assert_eq!(port.status().volumes, 2);
    assert_eq!(port.status().preparing, 2);
    let mut one = first.retirement();
    let mut two = second.retirement();
    owner.shutdown(deadline()).await.unwrap();
    one.wait(deadline()).await.unwrap();
    two.wait(deadline()).await.unwrap();
    assert_eq!(port.status().volumes, 0);
    assert_eq!(port.status().preparing, 0);
    assert!(
        timeout(Duration::from_millis(20), daemon.next())
            .await
            .is_err()
    );
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[test]
fn test_should_poison_same_acquisition_after_runtime_loss_with_an_issued_uncertain_create() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (
        root,
        directory,
        old_daemon,
        mut old_wire,
        mut owner,
        preparation,
        held,
        source,
        stop,
        saved,
    ) = runtime.block_on(async {
        let (root, directory, mut daemon, wire, docker) = fixture().await;
        let owner = CodeVolumeManager::start(docker, directory.clone(), config()).unwrap();
        let (source, archive) = code().await;
        let (base, stop, _released) = base(&directory);
        let preparation = owner
            .port()
            .start_preparation(
                CodeVolumeIdentity::new(Uuid::new_v4(), Uuid::new_v4()).unwrap(),
                archive,
                base,
                deadline(),
            )
            .unwrap();
        let (held, _) = begin_volume(&mut daemon, &directory).await;
        let saved = directory.code_records(deadline()).await.unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(
            saved.first().unwrap().volume,
            crate::docker::ownership::codes::VolumeState::Uncertain
        );
        (
            root,
            directory,
            daemon,
            wire,
            owner,
            preparation,
            held,
            source,
            stop,
            saved,
        )
    });
    let journal = runtime
        .block_on(tokio::fs::read(
            root.path().join(".rustack-lambda-docker").join("code.v1"),
        ))
        .unwrap();
    drop(runtime);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        assert!(matches!(
            directory.code_producer(),
            Err(crate::docker::DirectoryLeaseError::InvalidState)
        ));
        let mut daemon = Daemon::new();
        let mut wire = daemon.wire().await;
        let docker = wire
            .port()
            .bind_directory(&directory, deadline())
            .await
            .unwrap();
        assert!(CodeVolumeManager::start(docker, directory.clone(), config()).is_err());
        assert_eq!(directory.code_records(deadline()).await.unwrap(), saved);
        assert_eq!(
            tokio::fs::read(root.path().join(".rustack-lambda-docker").join("code.v1"))
                .await
                .unwrap(),
            journal
        );
        let mut receipt = preparation.retirement();
        assert!(matches!(preparation.wait().await, Err(DockerError::Worker)));
        assert!(matches!(
            receipt.wait(deadline()).await,
            Err(DockerError::Worker)
        ));
        assert!(matches!(
            owner.shutdown(deadline()).await,
            Err(DockerError::Worker)
        ));
        assert!(!owner.physically_settled());
        assert!(old_wire.shutdown().await.is_err());
        assert!(
            timeout(Duration::from_millis(20), daemon.next())
                .await
                .is_err()
        );
        wire.shutdown().await.unwrap();
        daemon.close().await;
    });
    drop((held, old_daemon, source, stop));
}
