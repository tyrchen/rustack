use std::{sync::atomic::AtomicUsize, time::Duration};

use super::{actor::testing_start, *};
use crate::docker::{
    DirectoryLease, EnvironmentConfig, EnvironmentManager, EnvironmentSettings, RelayBudgets,
    environment::test_peer::pair,
    plan::tests::{image_with, policy, revision},
    tests::Daemon,
};

mod settlement;

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
fn invocation(until: Instant) -> Invocation {
    Invocation::new(
        rustack_lambda_runtime::protocol::AttemptId::default(),
        until,
        "arn:aws:lambda:us-east-1:000000000000:function:policy-fixture"
            .parse()
            .unwrap(),
        rustack_lambda_runtime::metadata::local_trace_id().unwrap(),
        rustack_lambda_runtime::protocol::Payload::try_from(bytes::Bytes::from_static(b"{}"))
            .unwrap(),
    )
    .unwrap()
}
#[test]
fn test_should_reject_invalid_pool_limits_and_allow_disabled_warm_retention() {
    for settings in [
        PoolSettings::builder().warm(33).build(),
        PoolSettings::builder().warm_per_revision(33).build(),
        PoolSettings::builder().queue(0).build(),
        PoolSettings::builder().queue(257).build(),
    ] {
        assert!(PoolConfig::try_from(settings).is_err());
    }
    assert!(
        PoolConfig::try_from(PoolSettings::builder().warm(0).warm_per_revision(0).build()).is_ok()
    );
}
#[tokio::test]
async fn test_should_reject_queued_call_after_pool_stop_without_allocation() {
    let daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let lease = directory();
    let mut environments =
        EnvironmentManager::start(wire.port(), lease.clone(), EnvironmentConfig::default())
            .unwrap();
    let mut pool = EnvironmentPool::start(environments.port(), PoolConfig::default()).unwrap();
    let port = pool.port();
    pool.shutdown(deadline()).await.unwrap();
    assert!(matches!(
        port.invoke(plan(&lease), invocation(deadline()), false)
            .await,
        Err(EnvironmentError::Stopped)
    ));
    assert_eq!(environments.port().status().allocated, 0);
    environments.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[tokio::test]
async fn test_should_cache_root_failure_and_release_queued_permits_without_mutation() {
    let daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let lease = directory();
    let mut environments =
        EnvironmentManager::start(wire.port(), lease.clone(), EnvironmentConfig::default())
            .unwrap();
    let mut pool = EnvironmentPool::start(environments.port(), PoolConfig::default()).unwrap();
    pool.task.as_ref().unwrap().abort();
    assert!(matches!(
        pool.shutdown(deadline()).await,
        Err(EnvironmentError::Worker)
    ));
    assert!(matches!(
        pool.shutdown(deadline()).await,
        Err(EnvironmentError::Worker)
    ));
    assert!(pool.port().status().worker_failed);
    assert_eq!(environments.port().status().allocated, 0);
    environments.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[test]
fn test_should_key_allocation_values_without_fresh_identities_or_hashmap_order() {
    let lease = directory();
    let original = revision();
    let make = |revision, policy| {
        EnvironmentPlan::new(
            lease.clone(),
            image_with("1000", vec![]),
            Arc::new(revision),
            policy,
        )
        .unwrap()
    };
    let first = make(original.clone(), policy());
    let mut reordered = original.clone();
    let mut entries: Vec<_> = original.environment.iter().collect();
    entries.sort();
    reordered.environment = entries
        .into_iter()
        .rev()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let second = make(reordered, policy());
    assert_ne!(first.environment_id(), second.environment_id());
    assert_eq!(first.warm_key(), second.warm_key());
    let mut changed = original.clone();
    changed
        .environment
        .insert("TEST_VALUE".into(), "another-secret".into());
    assert_ne!(first.warm_key(), make(changed, policy()).warm_key());
    let mut changed = original.clone();
    changed.memory_mb = 256;
    assert_ne!(first.warm_key(), make(changed, policy()).warm_key());
    let mut changed = original.clone();
    changed.revision_id = uuid::Uuid::new_v4().to_string();
    assert_ne!(first.warm_key(), make(changed, policy()).warm_key());
    let another_policy = crate::docker::ContainerPolicy::new(
        "none".parse().unwrap(),
        1001,
        1000,
        1_000_000_000,
        RelayBudgets::default(),
    )
    .unwrap();
    assert_ne!(
        first.warm_key(),
        make(original.clone(), another_policy).warm_key()
    );
    assert_ne!(
        first.warm_key(),
        EnvironmentPlan::new(
            directory(),
            image_with("1000", vec![]),
            Arc::new(original),
            policy()
        )
        .unwrap()
        .warm_key()
    );
    assert!(!format!("{first:?}").contains("secret-value"));
}
async fn idle(port: &PoolPort) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while port.status().active != 0 || port.status().warm != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn test_should_reuse_only_consumed_warm_results_and_join_idle_shutdown() {
    use std::sync::atomic::AtomicUsize;
    let daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let lease = directory();
    let mut environments =
        EnvironmentManager::start(wire.port(), lease.clone(), EnvironmentConfig::default())
            .unwrap();
    let revision = Arc::new(revision());
    let make = || {
        EnvironmentPlan::new(
            lease.clone(),
            image_with("1000", vec![]),
            revision.clone(),
            policy(),
        )
        .unwrap()
    };
    let (handle, peer) = crate::docker::environment::test_peer::pair();
    let count = Arc::new(AtomicUsize::new(0));
    let served = tokio::spawn(peer.serve(count.clone()));
    let mut pool = actor::testing_start(
        environments.port(),
        PoolConfig::default(),
        vec![(make().warm_key().into(), handle)],
    );
    let port = pool.port();
    for expected in 1..=3 {
        port.invoke(make(), invocation(deadline()), false)
            .await
            .unwrap();
        idle(&port).await;
        assert_eq!(count.load(Ordering::Acquire), expected);
    }
    pool.shutdown(deadline()).await.unwrap();
    served.await.unwrap();
    assert_eq!(port.status().warm, 0);
    environments.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
#[rstest::rstest]
#[case::abandoned(true)]
#[case::live_but_unpolled(false)]
#[tokio::test]
async fn test_should_retire_unconsumed_ready_reply_even_without_public_future_polling(
    #[case] abandoned: bool,
) {
    use std::{future::Future, sync::atomic::AtomicUsize, task::Poll};
    let daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let lease = directory();
    let mut environments =
        EnvironmentManager::start(wire.port(), lease.clone(), EnvironmentConfig::default())
            .unwrap();
    let plan = plan(&lease);
    let key = plan.warm_key().to_owned();
    let (handle, peer) = crate::docker::environment::test_peer::pair();
    let receipt = handle.retirement();
    let served = tokio::spawn(peer.serve(Arc::new(AtomicUsize::new(0))));
    let mut pool = actor::testing_start(
        environments.port(),
        PoolConfig::default(),
        vec![(key, handle)],
    );
    let port = pool.port();
    let mut call = Box::pin(port.invoke(
        plan,
        invocation(Instant::now() + Duration::from_millis(250)),
        false,
    ));
    assert!(matches!(
        call.as_mut()
            .poll(&mut std::task::Context::from_waker(std::task::Waker::noop())),
        Poll::Pending
    ));
    tokio::time::sleep(Duration::from_millis(20)).await;
    if abandoned {
        drop(call);
    } else {
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            receipt.removed(),
            "unpolled waiter retained its environment past deadline"
        );
        assert!(matches!(
            call.as_mut()
                .poll(&mut std::task::Context::from_waker(std::task::Waker::noop())),
            Poll::Ready(Err(EnvironmentError::Deadline))
        ));
    }
    tokio::time::timeout(Duration::from_secs(2), served)
        .await
        .unwrap()
        .unwrap();
    assert!(receipt.removed());
    pool.shutdown(deadline()).await.unwrap();
    assert_eq!(port.status().warm, 0);
    environments.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_bound_one_queue_across_mailbox_and_waiting_dispatch_then_cancel_without_replay()
 {
    use std::{
        future::Future,
        task::{Context, Poll, Waker},
    };
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let lease = directory();
    let config = crate::docker::EnvironmentSettings::builder()
        .environments(1)
        .build()
        .try_into()
        .unwrap();
    let mut environments = EnvironmentManager::start(wire.port(), lease.clone(), config).unwrap();
    let mut occupied = environments.port().spawn(plan(&lease), deadline()).unwrap();
    let held = daemon.next().await;
    let mut pool = EnvironmentPool::start(
        environments.port(),
        PoolSettings::builder().queue(1).build().try_into().unwrap(),
    )
    .unwrap();
    let port = pool.port();
    let mut waiting = Box::pin(port.invoke(plan(&lease), invocation(deadline()), false));
    assert!(matches!(
        waiting
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ));
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert_eq!(port.status().queued, 1);
    assert_eq!(port.status().active, 0);
    assert!(matches!(
        port.invoke(plan(&lease), invocation(deadline()), false)
            .await,
        Err(EnvironmentError::Busy)
    ));
    drop(waiting);
    tokio::time::timeout(Duration::from_secs(2), async {
        while port.status().queued != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(daemon.requests.try_recv().is_err());
    pool.shutdown(deadline()).await.unwrap();
    held.reply(400, b"{}").await;
    occupied.shutdown(deadline()).await.unwrap();
    environments.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[rstest::rstest]
#[case::idle(false)]
#[case::offered_result(true)]
#[tokio::test]
async fn test_should_recover_idle_or_dispatched_environment_after_root_abort(
    #[case] offered: bool,
) {
    use std::{
        future::Future,
        sync::atomic::AtomicUsize,
        task::{Context, Poll, Waker},
    };
    let daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let lease = directory();
    let mut environments =
        EnvironmentManager::start(wire.port(), lease.clone(), EnvironmentConfig::default())
            .unwrap();
    let plan = plan(&lease);
    let key = plan.warm_key().into();
    let (handle, peer) = crate::docker::environment::test_peer::pair();
    let receipt = handle.retirement();
    let count = Arc::new(AtomicUsize::new(0));
    let served = tokio::spawn(peer.serve(count.clone()));
    let mut pool = actor::testing_start(
        environments.port(),
        PoolConfig::default(),
        vec![(key, handle)],
    );
    let port = pool.port();
    let mut call = Box::pin(port.invoke(plan, invocation(deadline()), false));
    if offered {
        assert!(matches!(
            call.as_mut().poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
        tokio::time::timeout(Duration::from_secs(2), async {
            while count.load(Ordering::Acquire) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
    pool.task.as_ref().unwrap().abort();
    assert!(matches!(
        pool.shutdown(deadline()).await,
        Err(EnvironmentError::Worker)
    ));
    assert!(matches!(
        pool.shutdown(deadline()).await,
        Err(EnvironmentError::Worker)
    ));
    assert!(receipt.removed());
    served.await.unwrap();
    drop(call);
    environments.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_retain_one_lru_eviction_barrier_while_cleanup_is_delayed() {
    use std::{
        future::Future,
        sync::atomic::AtomicUsize,
        task::{Context, Poll, Waker},
    };
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let lease = directory();
    let config = crate::docker::EnvironmentSettings::builder()
        .environments(1)
        .build()
        .try_into()
        .unwrap();
    let mut environments = EnvironmentManager::start(wire.port(), lease.clone(), config).unwrap();
    let mut occupied = environments.port().spawn(plan(&lease), deadline()).unwrap();
    let held = daemon.next().await;
    let (oldest, first) = crate::docker::environment::test_peer::pair();
    let first_receipt = oldest.retirement();
    let (release, gate) = oneshot::channel();
    let (stopping, stopped) = oneshot::channel();
    let first_task = tokio::spawn(first.serve_gated(Arc::new(AtomicUsize::new(0)), gate, stopping));
    let (next, second) = crate::docker::environment::test_peer::pair();
    let second_receipt = next.retirement();
    let second_task = tokio::spawn(second.serve(Arc::new(AtomicUsize::new(0))));
    let mut pool = actor::testing_start(
        environments.port(),
        PoolConfig::default(),
        vec![("oldest".into(), oldest), ("next".into(), next)],
    );
    let port = pool.port();
    let mut waiting = Box::pin(port.invoke(plan(&lease), invocation(deadline()), false));
    assert!(matches!(
        waiting
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ));
    tokio::time::timeout(Duration::from_secs(2), stopped)
        .await
        .unwrap()
        .unwrap();
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert!(!first_receipt.removed());
    assert!(!second_receipt.removed());
    assert_eq!(port.status().warm, 1);
    assert_eq!(port.status().active, 1);
    drop(waiting);
    release.send(()).unwrap();
    pool.shutdown(deadline()).await.unwrap();
    first_task.await.unwrap();
    second_task.await.unwrap();
    held.reply(400, b"{}").await;
    occupied.shutdown(deadline()).await.unwrap();
    environments.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_keep_retirement_owned_when_shutdown_waiter_is_cancelled() {
    use std::sync::atomic::AtomicUsize;
    let daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let lease = directory();
    let mut environments =
        EnvironmentManager::start(wire.port(), lease.clone(), EnvironmentConfig::default())
            .unwrap();
    let (handle, peer) = crate::docker::environment::test_peer::pair();
    let receipt = handle.retirement();
    let (release, gate) = oneshot::channel();
    let (stopping, stopped) = oneshot::channel();
    let served = tokio::spawn(peer.serve_gated(Arc::new(AtomicUsize::new(0)), gate, stopping));
    let mut pool = actor::testing_start(
        environments.port(),
        PoolConfig::default(),
        vec![("idle".into(), handle)],
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(30), pool.shutdown(deadline()))
            .await
            .is_err()
    );
    stopped.await.unwrap();
    assert!(!receipt.removed());
    release.send(()).unwrap();
    pool.shutdown(deadline()).await.unwrap();
    pool.shutdown(deadline()).await.unwrap();
    served.await.unwrap();
    assert!(receipt.removed());
    environments.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[test]
fn test_should_reject_oversized_captured_inputs_before_warm_fingerprinting() {
    let lease = directory();
    let mut invalid = revision();
    invalid
        .environment
        .insert("OVERSIZED".into(), "s".repeat(8192));
    assert!(
        EnvironmentPlan::new(
            lease.clone(),
            image_with("1000", vec![]),
            Arc::new(invalid),
            policy()
        )
        .is_err()
    );
    let mut invalid = revision();
    invalid.image_config = Some(rustack_lambda_model::types::ImageConfig {
        command: Some(vec!["x".repeat(32768)]),
        ..Default::default()
    });
    assert!(
        EnvironmentPlan::new(
            lease,
            image_with("1000", vec![]),
            Arc::new(invalid),
            policy()
        )
        .is_err()
    );
}

#[tokio::test]
async fn test_should_preserve_retirement_above_an_aborted_dispatch_worker() {
    use std::sync::atomic::AtomicUsize;
    let daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let lease = directory();
    let mut environments =
        EnvironmentManager::start(wire.port(), lease, EnvironmentConfig::default()).unwrap();
    let (handle, peer) = crate::docker::environment::test_peer::pair();
    let receipt = handle.retirement();
    let served = tokio::spawn(peer.serve(Arc::new(AtomicUsize::new(0))));
    let mut pool =
        actor::testing_failed_worker_start(environments.port(), vec![("owned".into(), handle)]);
    assert!(matches!(
        pool.shutdown(deadline()).await,
        Err(EnvironmentError::Worker)
    ));
    assert!(matches!(
        pool.shutdown(deadline()).await,
        Err(EnvironmentError::Worker)
    ));
    assert!(pool.port().status().worker_failed);
    assert!(receipt.removed());
    served.await.unwrap();
    environments.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_wait_for_capacity_in_fifo_order_and_never_replay_rejected_startup() {
    use std::{
        future::Future,
        task::{Context, Poll, Waker},
    };
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let lease = directory();
    let config = crate::docker::EnvironmentSettings::builder()
        .environments(1)
        .build()
        .try_into()
        .unwrap();
    let mut environments = EnvironmentManager::start(wire.port(), lease.clone(), config).unwrap();
    let mut occupied = environments.port().spawn(plan(&lease), deadline()).unwrap();
    let held = daemon.next().await;
    let mut pool = EnvironmentPool::start(environments.port(), PoolConfig::default()).unwrap();
    let port = pool.port();
    let first = plan(&lease);
    let first_key = first.relay_key().as_str().to_owned();
    let second = plan(&lease);
    let second_key = second.relay_key().as_str().to_owned();
    let mut first = Box::pin(port.invoke(first, invocation(deadline()), false));
    let mut second = Box::pin(port.invoke(second, invocation(deadline()), false));
    for call in [&mut first, &mut second] {
        assert!(matches!(
            call.as_mut().poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
    }
    held.reply(400, b"{}").await;
    occupied.shutdown(deadline()).await.unwrap();
    for key in [first_key, second_key] {
        let request = daemon.next().await;
        assert!(
            percent_encoding::percent_decode_str(&request.head)
                .decode_utf8()
                .unwrap()
                .contains(&key)
        );
        request.reply(400, b"{}").await;
    }
    assert!(first.await.is_err());
    assert!(second.await.is_err());
    pool.shutdown(deadline()).await.unwrap();
    assert!(daemon.requests.try_recv().is_err());
    environments.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[rstest::rstest]
#[case::no_global_retention(0, 1, 0)]
#[case::no_revision_retention(32, 0, 0)]
#[case::one_per_revision(32, 1, 1)]
#[tokio::test]
async fn test_should_retire_successful_excess_concurrent_returns(
    #[case] warm: u8,
    #[case] per_revision: u8,
    #[case] retained: usize,
) {
    use std::{
        future::Future,
        sync::atomic::AtomicUsize,
        task::{Context, Poll, Waker},
    };
    let daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let lease = directory();
    let mut environments =
        EnvironmentManager::start(wire.port(), lease.clone(), EnvironmentConfig::default())
            .unwrap();
    let revision = Arc::new(revision());
    let make = || {
        EnvironmentPlan::new(
            lease.clone(),
            image_with("1000", vec![]),
            revision.clone(),
            policy(),
        )
        .unwrap()
    };
    let key = make().warm_key().to_owned();
    let count = Arc::new(AtomicUsize::new(0));
    let mut seeded = Vec::new();
    let mut peers = Vec::new();
    for _ in 0..2 {
        let (handle, peer) = crate::docker::environment::test_peer::pair();
        seeded.push((key.clone(), handle));
        peers.push(tokio::spawn(peer.serve(count.clone())));
    }
    let config = PoolSettings::builder()
        .warm(warm)
        .warm_per_revision(per_revision)
        .build()
        .try_into()
        .unwrap();
    let mut pool = actor::testing_start(environments.port(), config, seeded);
    let port = pool.port();
    let mut first = Box::pin(port.invoke(make(), invocation(deadline()), false));
    let mut second = Box::pin(port.invoke(make(), invocation(deadline()), false));
    for call in [&mut first, &mut second] {
        assert!(matches!(
            call.as_mut().poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        while count.load(Ordering::Acquire) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    first.await.unwrap();
    second.await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while port.status().active != 0 || port.status().warm != retained {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    pool.shutdown(deadline()).await.unwrap();
    for peer in peers {
        peer.await.unwrap();
    }
    environments.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_reject_impossible_memory_without_evicting_warm_work_or_blocking_fitting_calls()
{
    let daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let lease = directory();
    let config = EnvironmentSettings::builder()
        .memory_bytes(256 * 1024 * 1024)
        .build()
        .try_into()
        .unwrap();
    let mut environments = EnvironmentManager::start(wire.port(), lease.clone(), config).unwrap();
    let fitting = plan(&lease);
    let (handle, peer) = pair();
    let count = Arc::new(AtomicUsize::new(0));
    let served = tokio::spawn(peer.serve(count.clone()));
    let mut pool = testing_start(
        environments.port(),
        PoolConfig::default(),
        vec![(fitting.warm_key().into(), handle)],
    );
    let mut oversized = revision();
    oversized.memory_mb = 256;
    let oversized = EnvironmentPlan::new(
        lease,
        image_with("1000", vec![]),
        Arc::new(oversized),
        policy(),
    )
    .unwrap();
    assert!(matches!(
        pool.port()
            .invoke(oversized, invocation(deadline()), false)
            .await,
        Err(EnvironmentError::MemoryLimit)
    ));
    assert_eq!(pool.port().status().warm, 1);
    pool.port()
        .invoke(fitting, invocation(deadline()), false)
        .await
        .unwrap();
    idle(&pool.port()).await;
    assert_eq!(count.load(Ordering::Acquire), 1);
    assert_eq!(environments.port().status().allocated, 0);
    pool.shutdown(deadline()).await.unwrap();
    served.await.unwrap();
    environments.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
