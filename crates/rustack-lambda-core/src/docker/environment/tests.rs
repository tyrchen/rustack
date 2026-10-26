use serde_json::{Value, json};
use tokio::time::timeout;

use super::*;
use crate::docker::{
    plan::tests::{image_with, policy, revision},
    tests::Daemon,
};

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
fn directory() -> DirectoryLease {
    DirectoryLease::testing(uuid::Uuid::new_v4(), uuid::Uuid::new_v4())
}
fn plan(directory: &DirectoryLease) -> EnvironmentPlan {
    EnvironmentPlan::new(
        directory.clone(),
        image_with("1000", vec![]),
        Arc::new(revision()),
        policy(),
    )
    .unwrap()
}
fn inspected(plan: &EnvironmentPlan) -> Value {
    let revision = plan.revision();
    let image = match &revision.artifact {
        crate::artifact::DeploymentArtifact::Image(image) => {
            image.resolved().metadata().image_id.as_str()
        }
        _ => panic!("image fixture"),
    };
    json!({"Id":"a".repeat(64),"Name":format!("/{}",plan.relay_key().as_str()),"Config":{"Image":image,"Labels":{
        "rustack.lambda.ownerId":plan.directory.owner_id().to_string(),"rustack.lambda.runId":plan.directory.run_id().to_string(),"rustack.lambda.environmentId":plan.environment_id().to_string(),"rustack.lambda.functionResourceId":revision.resource_id.to_string(),"rustack.lambda.role":"relay"}},"State":{"Running":false,"OOMKilled":false,"ExitCode":0}})
}
async fn quarantined(handle: &EnvironmentHandle) {
    timeout(Duration::from_secs(3), async {
        while handle.status().phase != EnvironmentPhase::Quarantined {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
async fn finish_relay(daemon: &mut Daemon, owned: &Value) {
    let observed = daemon.next().await;
    assert!(observed.head.starts_with("GET "));
    observed
        .reply(200, &serde_json::to_vec(owned).unwrap())
        .await;
    let checked = daemon.next().await;
    assert!(checked.head.contains(&"a".repeat(64)));
    checked
        .reply(200, &serde_json::to_vec(owned).unwrap())
        .await;
    let removed = daemon.next().await;
    assert!(removed.head.starts_with("DELETE "));
    removed.reply(204, b"").await;
    daemon.next().await.reply(404, b"{}").await;
}

#[test]
fn test_should_validate_environment_budgets_and_reserve_count_and_memory_transactionally() {
    for settings in [
        EnvironmentSettings::builder().environments(0).build(),
        EnvironmentSettings::builder().environments(33).build(),
        EnvironmentSettings::builder().memory_bytes(1).build(),
        EnvironmentSettings::builder().io(Duration::ZERO).build(),
        EnvironmentSettings::builder()
            .start(Duration::from_secs(61))
            .build(),
        EnvironmentSettings::builder()
            .idle(Duration::from_millis(1500))
            .build(),
        EnvironmentSettings::builder().log_ring_bytes(4095).build(),
    ] {
        assert!(EnvironmentConfig::try_from(settings).is_err());
    }
    let config = EnvironmentConfig::try_from(
        EnvironmentSettings::builder()
            .environments(2)
            .memory_bytes(256 * 1024 * 1024)
            .build(),
    )
    .unwrap();
    let budget = Arc::new(Budget::new(config));
    let first = budget.reserve(256 * 1024 * 1024).unwrap();
    assert!(budget.reserve(1).is_err());
    assert_eq!(budget.count(), 1);
    drop(first);
    assert_eq!(budget.count(), 0);
    assert_eq!(budget.memory.load(Ordering::Acquire), 0);
}
#[tokio::test]
async fn test_should_reject_foreign_scope_without_admitting_or_mutating() {
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let lease = directory();
    let mut manager =
        EnvironmentManager::start(wire.port(), lease, EnvironmentConfig::default()).unwrap();
    assert!(matches!(
        manager.port().spawn(plan(&directory()), deadline()),
        Err(EnvironmentError::Configuration)
    ));
    assert_eq!(manager.port().status().allocated, 0);
    assert!(
        timeout(Duration::from_millis(20), daemon.requests.recv())
            .await
            .is_err()
    );
    manager.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[tokio::test]
async fn test_should_settle_definitive_create_rejection_without_containers() {
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let lease = directory();
    let mut manager =
        EnvironmentManager::start(wire.port(), lease.clone(), EnvironmentConfig::default())
            .unwrap();
    let mut handle = manager.port().spawn(plan(&lease), deadline()).unwrap();
    daemon.next().await.reply(400, b"{}").await;
    assert!(handle.wait_ready(deadline()).await.is_err());
    handle.shutdown(deadline()).await.unwrap();
    manager.shutdown(deadline()).await.unwrap();
    assert_eq!(manager.port().status().allocated, 0);
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[tokio::test]
async fn test_should_retain_quota_after_500_and_initial_absence_until_late_create_is_removed() {
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let lease = directory();
    let planned = plan(&lease);
    let owned = inspected(&planned);
    let mut manager =
        EnvironmentManager::start(wire.port(), lease, EnvironmentConfig::default()).unwrap();
    let mut handle = manager.port().spawn(planned, deadline()).unwrap();
    daemon.next().await.reply(500, b"{}").await;
    daemon.next().await.reply(404, b"{}").await;
    quarantined(&handle).await;
    assert_eq!(manager.port().status().allocated, 1);
    assert_eq!(manager.port().status().memory_bytes, 256 * 1024 * 1024);
    assert!(matches!(
        handle
            .shutdown(Instant::now() + Duration::from_millis(20))
            .await,
        Err(EnvironmentError::Deadline)
    ));
    finish_relay(&mut daemon, &owned).await;
    handle.shutdown(deadline()).await.unwrap();
    manager.shutdown(deadline()).await.unwrap();
    assert_eq!(manager.port().status().allocated, 0);
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[rstest::rstest]
#[case("labels")]
#[case("image")]
#[case("name")]
#[case("id")]
#[tokio::test]
async fn test_should_quarantine_foreign_or_malformed_objects_without_deleting_them(
    #[case] conflict: &str,
) {
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let lease = directory();
    let planned = plan(&lease);
    let owned = inspected(&planned);
    let mut foreign = owned.clone();
    match conflict {
        "labels" => {
            foreign["Config"]["Labels"]["rustack.lambda.ownerId"] =
                json!(uuid::Uuid::new_v4().to_string());
        }
        "image" => foreign["Config"]["Image"] = json!("sha256:foreign"),
        "name" => foreign["Name"] = json!("/foreign"),
        "id" => foreign["Id"] = json!("not-an-id"),
        _ => panic!("case"),
    }
    let mut manager =
        EnvironmentManager::start(wire.port(), lease, EnvironmentConfig::default()).unwrap();
    let mut handle = manager.port().spawn(planned, deadline()).unwrap();
    daemon.next().await.reply(500, b"{}").await;
    daemon
        .next()
        .await
        .reply(200, &serde_json::to_vec(&foreign).unwrap())
        .await;
    quarantined(&handle).await;
    assert!(
        timeout(Duration::from_millis(20), daemon.requests.recv())
            .await
            .is_err()
    );
    assert_eq!(manager.port().status().allocated, 1);
    finish_relay(&mut daemon, &owned).await;
    handle.shutdown(deadline()).await.unwrap();
    manager.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[tokio::test]
async fn test_should_finish_critical_create_after_caller_cancel_before_cleanup() {
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let lease = directory();
    let planned = plan(&lease);
    let owned = inspected(&planned);
    let mut manager =
        EnvironmentManager::start(wire.port(), lease, EnvironmentConfig::default()).unwrap();
    let mut handle = manager.port().spawn(planned, deadline()).unwrap();
    let create = daemon.next().await;
    handle.cancel();
    assert!(
        timeout(Duration::from_millis(20), daemon.requests.recv())
            .await
            .is_err()
    );
    assert_eq!(manager.port().status().allocated, 1);
    create
        .reply(
            201,
            format!("{{\"Id\":\"{}\",\"Warnings\":[]}}", "a".repeat(64)).as_bytes(),
        )
        .await;
    daemon
        .next()
        .await
        .reply(200, &serde_json::to_vec(&owned).unwrap())
        .await;
    daemon
        .next()
        .await
        .reply(200, &serde_json::to_vec(&owned).unwrap())
        .await;
    daemon.next().await.reply(204, b"").await;
    daemon.next().await.reply(404, b"{}").await;
    handle.shutdown(deadline()).await.unwrap();
    manager.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[tokio::test]
async fn test_should_recover_retained_record_after_root_abort_with_create_in_flight() {
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let lease = directory();
    let planned = plan(&lease);
    let owned = inspected(&planned);
    let mut manager =
        EnvironmentManager::start(wire.port(), lease, EnvironmentConfig::default()).unwrap();
    let mut handle = manager.port().spawn(planned, deadline()).unwrap();
    let create = daemon.next().await;
    manager.task.as_ref().unwrap().abort();
    daemon.next().await.reply(404, b"{}").await;
    assert_eq!(manager.port().status().allocated, 1);
    create
        .reply(
            201,
            format!("{{\"Id\":\"{}\",\"Warnings\":[]}}", "a".repeat(64)).as_bytes(),
        )
        .await;
    finish_relay(&mut daemon, &owned).await;
    assert!(matches!(
        handle.shutdown(deadline()).await,
        Err(EnvironmentError::Worker)
    ));
    assert!(matches!(
        manager.shutdown(deadline()).await,
        Err(EnvironmentError::Worker)
    ));
    assert!(matches!(
        manager.shutdown(deadline()).await,
        Err(EnvironmentError::Worker)
    ));
    assert_eq!(manager.port().status().allocated, 0);
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_settle_revoked_before_issue_without_any_daemon_mutation() {
    use std::sync::atomic::AtomicBool;
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let lease = directory();
    let (release, _) = watch::channel(());
    let (stop, stopped) = watch::channel(false);
    let relay = crate::artifact::PinnedImage::new(
        image_with("1000", vec![]).resolved().clone(),
        4096.try_into().unwrap(),
        release,
        Arc::new(AtomicBool::new(true)),
        stopped,
    );
    let planned =
        EnvironmentPlan::new(lease.clone(), relay, Arc::new(revision()), policy()).unwrap();
    stop.send_replace(true);
    let mut manager =
        EnvironmentManager::start(wire.port(), lease, EnvironmentConfig::default()).unwrap();
    let mut handle = manager.port().spawn(planned, deadline()).unwrap();
    assert!(handle.wait_ready(deadline()).await.is_err());
    handle.shutdown(deadline()).await.unwrap();
    manager.shutdown(deadline()).await.unwrap();
    assert_eq!(manager.port().status().allocated, 0);
    assert!(
        timeout(Duration::from_millis(20), daemon.requests.recv())
            .await
            .is_err()
    );
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[tokio::test]
async fn test_should_release_unissued_allocation_when_socket_connection_fails() {
    let daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    daemon.close().await;
    let lease = directory();
    let mut manager =
        EnvironmentManager::start(wire.port(), lease.clone(), EnvironmentConfig::default())
            .unwrap();
    let mut handle = manager.port().spawn(plan(&lease), deadline()).unwrap();
    assert!(handle.wait_ready(deadline()).await.is_err());
    handle.shutdown(deadline()).await.unwrap();
    manager.shutdown(deadline()).await.unwrap();
    assert_eq!(manager.port().status().allocated, 0);
    wire.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_should_admit_a_queued_call_after_health_io_takes_longer_than_its_cadence() {
    use tokio::io::duplex;

    use crate::docker::{DockerOutputReader, FunctionLogs, plan::ContainerRole};
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let lease = directory();
    let planned = Arc::new(plan(&lease));
    let environment: EnvironmentId = planned.environment_id();
    let mut owned = inspected(&planned);
    owned["Name"] = json!(format!("/{}", planned.function_key().as_str()));
    owned["Config"]["Labels"]["rustack.lambda.role"] = json!("function");
    owned["State"]["Running"] = json!(true);
    let (client, peer) = ready_client(environment).await;
    let (_quiet, reader) = duplex(8);
    let logs = FunctionLogs::start_reader(
        DockerOutputReader::new(reader, Duration::from_secs(1)).unwrap(),
        4096,
    )
    .unwrap();
    let io_task = Arc::new(CleanupTask::default());
    let signals = Signals::new();
    let health = Arc::new(Health::default());
    let config = EnvironmentConfig::default();
    let progress = Arc::new(cleanup::Progress::new());
    progress
        .phase(ContainerRole::Function)
        .send_replace(cleanup::Phase::Known("a".repeat(64).parse().unwrap()));
    let cleanup = Arc::new(tokio::sync::Semaphore::new(2));
    let (global, _) = watch::channel(false);
    let cleaner = cleanup::Cleaner::builder()
        .docker(wire.port())
        .config(config)
        .plan(planned.clone())
        .progress(progress.clone())
        .health(health.clone())
        .cleanup(cleanup.clone())
        .signals(signals.clone())
        .build();
    let worker = worker::Worker::builder()
        .docker(wire.port())
        .config(config)
        .plan(planned)
        .progress(progress)
        .cleanup(cleanup)
        .health(health)
        .signals(signals.clone())
        .global(global)
        .io(io_task.clone())
        .runtime(Handle::current())
        .build();
    let mut io = io::IoGuard::new(Handle::current(), io_task.clone(), config, signals);
    io.test_streams(client, logs);
    let (calls, mut mailbox) = mpsc::channel(4);
    let task = tokio::spawn(async move {
        if let Some((reply, result)) = worker.serve_test(&mut io, &cleaner, &mut mailbox).await {
            let _ = reply.send(result);
        }
        io.begin_cleanup();
    });
    let inspection = daemon.next().await;
    let (stop, cancellation) = watch::channel(false);
    let (reply, response) = oneshot::channel();
    calls
        .send(Call {
            invocation: foreign_invocation(),
            capture_logs: false,
            cancellation,
            reply,
        })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(1100)).await;
    inspection
        .reply(200, &serde_json::to_vec(&owned).unwrap())
        .await;
    assert!(matches!(
        timeout(Duration::from_millis(500), response)
            .await
            .unwrap()
            .unwrap(),
        Err(EnvironmentError::Configuration)
    ));
    task.await.unwrap();
    io_task.wait().await.unwrap();
    peer.await.unwrap();
    drop(stop);
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[rstest::rstest]
#[case::normal(false, false)]
#[case::root_abort(true, false)]
#[case::wire_failure(true, true)]
#[tokio::test]
async fn test_should_keep_queued_handle_failure_distinct_from_clean_shutdown(
    #[case] failed: bool,
    #[case] wire_failure: bool,
) {
    let daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let lease = directory();
    let mut manager =
        EnvironmentManager::start(wire.port(), lease.clone(), EnvironmentConfig::default())
            .unwrap();
    let mut handle = manager.port().spawn(plan(&lease), deadline()).unwrap();
    if wire_failure {
        wire.port().raw.stop.send_replace(true);
    } else if failed {
        manager.task.as_ref().unwrap().abort();
    } else {
        manager.port.control.send_replace(true);
    }
    let result = handle.shutdown(deadline()).await;
    let joined = manager.shutdown(deadline()).await;
    if failed {
        assert!(matches!(result, Err(EnvironmentError::Worker)));
        assert!(matches!(joined, Err(EnvironmentError::Worker)));
    } else {
        result.unwrap();
        joined.unwrap();
    }
    assert_eq!(manager.port().status().allocated, 0);
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[tokio::test]
async fn test_should_allow_final_log_tail_at_eof_but_never_reuse_the_closed_environment() {
    use rustack_lambda_runtime::{
        client::{ClientConfig, ControlClient},
        protocol::{Architecture, EnvironmentId},
    };

    use crate::docker::{DockerOutputReader, FunctionLogs};
    let (reader, peer) = tokio::io::duplex(8);
    drop(peer);
    let client = ControlClient::start(
        ClientConfig::new(
            EnvironmentId::default(),
            Architecture::Arm64,
            Duration::from_secs(1),
        )
        .unwrap(),
        reader,
        tokio::io::sink(),
    )
    .unwrap();
    let logs = FunctionLogs::start_reader(
        DockerOutputReader::new(b"".as_slice(), Duration::from_secs(1)).unwrap(),
        4096,
    )
    .unwrap();
    let cleanup = Arc::new(CleanupTask::default());
    let mut io = io::IoGuard::new(
        Handle::current(),
        cleanup.clone(),
        EnvironmentConfig::default(),
        Signals::new(),
    );
    io.test_streams(client, logs);
    timeout(Duration::from_secs(1), async {
        loop {
            assert!(io.tail().await.unwrap().is_empty());
            if !io.reusable() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(io.healthy().await.is_err());
    io.begin_cleanup();
    cleanup.wait().await.unwrap();
}

async fn ready_client(
    environment: EnvironmentId,
) -> (
    rustack_lambda_runtime::client::ControlClient,
    tokio::task::JoinHandle<()>,
) {
    use rustack_lambda_runtime::{
        client::{ClientConfig, ClientEvent, ControlClient},
        protocol::{Acknowledgement, Architecture, Frame, Message, read_frame, write_frame},
    };
    use tokio::io::{duplex, split};
    let (stream, mut peer) = duplex(65536);
    let (reader, writer) = split(stream);
    let peer = tokio::spawn(async move {
        for message in [
            Message::Hello {
                architecture: Architecture::Arm64,
            },
            Message::Ready,
            Message::BootstrapReady,
        ] {
            write_frame(&mut peer, &Frame::new(environment, message), deadline())
                .await
                .unwrap();
        }
        let close = read_frame(&mut peer, deadline()).await.unwrap().unwrap();
        assert!(matches!(close.message(), Message::Close));
        write_frame(
            &mut peer,
            &Frame::new(environment, Message::Ack(Acknowledgement::Close)),
            deadline(),
        )
        .await
        .unwrap();
    });
    let mut client = ControlClient::start(
        ClientConfig::new(environment, Architecture::Arm64, Duration::from_secs(1)).unwrap(),
        reader,
        writer,
    )
    .unwrap();
    assert_eq!(
        client.next(deadline()).await.unwrap(),
        ClientEvent::RelayReady
    );
    assert_eq!(
        client.next(deadline()).await.unwrap(),
        ClientEvent::BootstrapReady
    );
    (client, peer)
}

fn foreign_invocation() -> rustack_lambda_runtime::session::Invocation {
    use rustack_lambda_runtime::{
        metadata::local_trace_id,
        protocol::{AttemptId, Payload},
        session::Invocation,
    };
    Invocation::new(
        AttemptId::default(),
        deadline(),
        "arn:aws:lambda:us-east-1:000000000000:function:other"
            .parse()
            .unwrap(),
        local_trace_id().unwrap(),
        Payload::try_from(Bytes::from_static(b"{}")).unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn test_should_bound_long_caller_deadline_by_start_budget_before_admission_validation() {
    let daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let lease = directory();
    let mut manager =
        EnvironmentManager::start(wire.port(), lease.clone(), EnvironmentConfig::default())
            .unwrap();
    let mut handle = manager
        .port()
        .spawn(plan(&lease), Instant::now() + Duration::from_mins(15))
        .unwrap();
    handle.cancel();
    handle.shutdown(deadline()).await.unwrap();
    manager.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[rstest::rstest]
#[case::abandoned(false)]
#[case::consumed(true)]
#[tokio::test]
async fn test_should_retire_ready_reply_only_when_its_waiter_is_abandoned(#[case] consumed: bool) {
    use std::{
        future::Future,
        task::{Context, Waker},
    };

    use rustack_lambda_runtime::protocol::Payload;
    let signals = Signals::new();
    signals.phase(EnvironmentPhase::Idle);
    let (calls, mut mailbox) = mpsc::channel(4);
    let (_done, settlement) = watch::channel(None);
    let mut handle = EnvironmentHandle {
        id: EnvironmentId::default(),
        calls,
        signals: signals.clone(),
        settlement,
    };
    {
        let invoke = handle.invoke(foreign_invocation(), false);
        tokio::pin!(invoke);
        assert!(
            invoke
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        let call = mailbox.recv().await.unwrap();
        call.reply
            .send(Ok(EnvironmentResponse {
                result: rustack_lambda_runtime::session::RuntimeResult::Success(
                    Payload::try_from(Bytes::from_static(b"{}")).unwrap(),
                ),
                logs: Bytes::new(),
            }))
            .unwrap();
        if consumed {
            assert!(matches!(
                invoke
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop())),
                std::task::Poll::Ready(Ok(_))
            ));
        }
    }
    assert_eq!(
        *signals.stop.borrow(),
        !consumed,
        "only abandoned replies retire the environment"
    );
}
