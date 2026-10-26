use std::{
    future::Future,
    sync::atomic::AtomicUsize,
    task::{Context, Waker},
};

use super::*;
use crate::docker::environment::test_peer;

#[rstest::rstest]
#[case::cancel(false, false)]
#[case::expired(true, false)]
#[case::root_abort(false, true)]
#[tokio::test]
async fn test_should_retain_per_call_settlement_across_expiry_waiter_drop_and_root_recovery(
    #[case] expired: bool,
    #[case] abort: bool,
) {
    let daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let lease = directory();
    let mut environments =
        EnvironmentManager::start(wire.port(), lease.clone(), EnvironmentConfig::default())
            .unwrap();
    let plan = plan(&lease);
    let (handle, peer) = test_peer::pair();
    let physical = handle.retirement();
    let count = Arc::new(AtomicUsize::new(0));
    let (release, gate) = oneshot::channel();
    let (stopping, stopped) = oneshot::channel();
    let serving = tokio::spawn(peer.serve_gated(count.clone(), gate, stopping));
    let mut pool = actor::testing_start(
        environments.port(),
        PoolConfig::default(),
        vec![(plan.warm_key().into(), handle)],
    );
    let until = if expired {
        Instant::now() + Duration::from_millis(250)
    } else {
        deadline()
    };
    let mut call = pool.port().start(plan, invocation(until), false).unwrap();
    timeout_at(deadline(), async {
        while count.load(Ordering::Acquire) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    if abort {
        pool.task.as_ref().unwrap().abort();
    } else if expired {
        tokio::time::sleep_until(until).await;
        assert!(matches!(call.wait().await, Err(EnvironmentError::Deadline)));
    }
    let mut waiting = Box::pin(call.cancel_and_wait(deadline()));
    assert!(
        waiting
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    timeout_at(deadline(), stopped).await.unwrap().unwrap();
    assert!(
        waiting
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    drop(waiting);
    assert!(!physical.removed());
    assert!(matches!(
        call.cancel_and_wait(Instant::now() + Duration::from_millis(20))
            .await,
        Err(EnvironmentError::Deadline)
    ));
    assert!(!physical.removed());
    release.send(()).unwrap();
    let result = call.cancel_and_wait(deadline()).await;
    assert!(physical.removed());
    if abort {
        assert!(matches!(result, Err(EnvironmentError::Worker)));
        assert!(matches!(
            call.cancel_and_wait(deadline()).await,
            Err(EnvironmentError::Worker)
        ));
        assert!(matches!(
            pool.shutdown(deadline()).await,
            Err(EnvironmentError::Worker)
        ));
    } else {
        result.unwrap();
        call.cancel_and_wait(deadline()).await.unwrap();
        pool.shutdown(deadline()).await.unwrap();
    }
    serving.await.unwrap();
    assert_eq!(count.load(Ordering::Acquire), 1);
    environments.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_resume_pending_response_wait_and_leave_consumed_success_warm() {
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
    let (handle, peer) = test_peer::pair();
    let physical = handle.retirement();
    let count = Arc::new(AtomicUsize::new(0));
    let serving = tokio::spawn(peer.serve(count.clone()));
    let mut pool = actor::testing_start(
        environments.port(),
        PoolConfig::default(),
        vec![(make().warm_key().into(), handle)],
    );
    let port = pool.port();
    for expected in 1..=2 {
        let mut call = port.start(make(), invocation(deadline()), false).unwrap();
        let mut waiting = Box::pin(call.wait());
        assert!(
            waiting
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        drop(waiting);
        call.wait().await.unwrap();
        assert!(matches!(
            call.wait().await,
            Err(EnvironmentError::Configuration)
        ));
        call.cancel_and_wait(deadline()).await.unwrap();
        drop(call);
        idle(&port).await;
        assert!(!physical.removed());
        assert_eq!(count.load(Ordering::Acquire), expected);
    }
    pool.shutdown(deadline()).await.unwrap();
    serving.await.unwrap();
    environments.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_settle_cancelled_queued_call_without_waiting_for_an_unrelated_allocation() {
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let lease = directory();
    let mut environments = EnvironmentManager::start(
        wire.port(),
        lease.clone(),
        crate::docker::EnvironmentSettings::builder()
            .environments(1)
            .build()
            .try_into()
            .unwrap(),
    )
    .unwrap();
    let mut occupied = environments.port().spawn(plan(&lease), deadline()).unwrap();
    let held = daemon.next().await;
    let mut pool = EnvironmentPool::start(environments.port(), PoolConfig::default()).unwrap();
    let mut call = pool
        .port()
        .start(plan(&lease), invocation(deadline()), false)
        .unwrap();
    call.cancel_and_wait(deadline()).await.unwrap();
    assert_eq!(environments.port().status().allocated, 1);
    assert!(daemon.requests.try_recv().is_err());
    pool.shutdown(deadline()).await.unwrap();
    held.reply(400, b"{}").await;
    occupied.shutdown(deadline()).await.unwrap();
    environments.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_retain_call_receipt_above_a_failed_dispatch_worker() {
    let daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let lease = directory();
    let mut environments =
        EnvironmentManager::start(wire.port(), lease.clone(), EnvironmentConfig::default())
            .unwrap();
    let (environment, peer) = test_peer::pair();
    let physical = environment.retirement();
    let (release, gate) = oneshot::channel();
    let (stopping, stopped) = oneshot::channel();
    let serving = tokio::spawn(peer.serve_gated(Arc::new(AtomicUsize::new(0)), gate, stopping));
    let (mut pool, mut call) = actor::testing_failed_call_start(
        environments.port(),
        plan(&lease),
        invocation(deadline()),
        environment,
    );
    timeout_at(deadline(), stopped).await.unwrap().unwrap();
    assert!(matches!(call.wait().await, Err(EnvironmentError::Worker)));
    let mut waiting = Box::pin(call.cancel_and_wait(deadline()));
    assert!(
        waiting
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    drop(waiting);
    assert!(!physical.removed());
    release.send(()).unwrap();
    assert!(matches!(
        call.cancel_and_wait(deadline()).await,
        Err(EnvironmentError::Worker)
    ));
    assert!(physical.removed());
    assert!(matches!(
        pool.shutdown(deadline()).await,
        Err(EnvironmentError::Worker)
    ));
    serving.await.unwrap();
    environments.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_treat_a_closed_response_and_missing_settlement_as_uncertain() {
    let (reply, response) = oneshot::channel();
    let (settlement, observed) = watch::channel(None);
    drop(reply);
    drop(settlement);
    let mut call = PoolInvocation {
        deadline: deadline(),
        stop: watch::channel(false).0,
        response: Some(response),
        settlement: observed,
        failed: Arc::new(AtomicBool::new(false)),
        consumed: false,
    };
    assert!(matches!(call.wait().await, Err(EnvironmentError::Stopped)));
    assert!(matches!(
        call.wait().await,
        Err(EnvironmentError::Configuration)
    ));
    assert!(matches!(
        call.cancel_and_wait(deadline()).await,
        Err(EnvironmentError::Worker)
    ));
    assert!(matches!(
        call.cancel_and_wait(deadline()).await,
        Err(EnvironmentError::Worker)
    ));
}

#[rstest::rstest]
#[case::expired(true)]
#[case::excessive(false)]
#[tokio::test]
async fn test_should_signal_cancellation_before_rejecting_invalid_cleanup_deadline(
    #[case] expired: bool,
) {
    let (reply, response) = oneshot::channel();
    let (_settled, observed) = watch::channel(Some(Ok(())));
    let mut call = PoolInvocation {
        deadline: deadline(),
        stop: watch::channel(false).0,
        response: Some(response),
        settlement: observed,
        failed: Arc::new(AtomicBool::new(false)),
        consumed: false,
    };
    let until = if expired {
        Instant::now()
    } else {
        Instant::now() + Duration::from_secs(601)
    };
    let error = call.cancel_and_wait(until).await.unwrap_err();
    if expired {
        assert!(matches!(error, EnvironmentError::Deadline));
    } else {
        assert!(matches!(error, EnvironmentError::Configuration));
    }
    assert!(*call.stop.borrow());
    assert!(reply.is_closed());
    call.cancel_and_wait(deadline()).await.unwrap();
}
