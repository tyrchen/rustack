use std::{
    future::Future,
    task::{Context, Poll, Waker},
};

use bytes::Bytes;

use super::*;

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

fn payload() -> Payload {
    Payload::try_from(Bytes::from_static(b"secret-payload")).unwrap()
}

fn invocation() -> Invocation {
    Invocation::new(
        AttemptId::default(),
        deadline(),
        "arn:aws:lambda:us-east-1:000000000000:function:echo:prod"
            .parse()
            .unwrap(),
        "Root=1-00000000-000000000000000000000000;Sampled=0"
            .parse()
            .unwrap(),
        payload(),
    )
    .unwrap()
}

fn poll_once<F: Future + Unpin>(future: &mut F) -> Poll<F::Output> {
    Future::poll(
        std::pin::Pin::new(future),
        &mut Context::from_waker(Waker::noop()),
    )
}

#[tokio::test]
async fn test_should_deliver_once_validate_completion_and_reuse_after_success_and_function_error() {
    let mut session = RuntimeSession::start(Duration::from_secs(60)).unwrap();
    let handle = session.handle().clone();
    let first = invocation();
    let ticket = handle.submit(first.clone(), deadline()).await.unwrap();
    assert_eq!(ticket.attempt(), first.parts().0);
    assert_eq!(
        handle.submit(invocation(), deadline()).await.unwrap_err(),
        SessionError::Busy
    );
    assert_eq!(handle.next(deadline()).await.unwrap(), first);
    assert_eq!(
        handle.next(deadline()).await.unwrap_err(),
        SessionError::Busy
    );
    assert_eq!(
        handle
            .complete(
                AttemptId::default(),
                None,
                Outcome::Success,
                payload(),
                deadline()
            )
            .await,
        Err(SessionError::UnknownAttempt)
    );
    assert_eq!(
        handle
            .complete(
                ticket.attempt(),
                Some(AttemptId::default()),
                Outcome::Success,
                payload(),
                deadline()
            )
            .await,
        Err(SessionError::UnknownAttempt)
    );
    handle
        .complete(
            ticket.attempt(),
            Some(ticket.attempt()),
            Outcome::Success,
            payload(),
            deadline(),
        )
        .await
        .unwrap();
    let old = ticket.attempt();
    assert_eq!(
        ticket.wait().await.unwrap(),
        RuntimeResult::Success(payload())
    );
    let second = handle.submit(invocation(), deadline()).await.unwrap();
    handle.next(deadline()).await.unwrap();
    assert_eq!(
        handle
            .complete(old, None, Outcome::Success, payload(), deadline())
            .await,
        Err(SessionError::UnknownAttempt)
    );
    handle
        .complete(
            second.attempt(),
            None,
            Outcome::FunctionError,
            payload(),
            deadline(),
        )
        .await
        .unwrap();
    assert_eq!(
        second.wait().await.unwrap(),
        RuntimeResult::FunctionError(payload())
    );
    session.shutdown().await.unwrap();
    session.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn test_should_distinguish_listener_readiness_first_poll_and_cancelled_poll_replacement() {
    let mut session = RuntimeSession::start(Duration::from_secs(60)).unwrap();
    let handle = session.handle().clone();
    let mut readiness = handle.bootstrap.clone();
    assert_eq!(
        handle
            .bootstrap_ready(Instant::now() + Duration::from_millis(1))
            .await,
        Err(SessionError::Deadline)
    );
    let mut first = Box::pin(handle.next(deadline()));
    assert!(poll_once(&mut first).is_pending());
    handle.bootstrap_ready(deadline()).await.unwrap();
    assert!(*readiness.borrow_and_update());
    assert_eq!(
        handle.next(deadline()).await.unwrap_err(),
        SessionError::Busy
    );
    drop(first);
    let call = invocation();
    let ticket = handle.submit(call.clone(), deadline()).await.unwrap();
    assert_eq!(handle.next(deadline()).await.unwrap(), call);
    assert!(
        !readiness.has_changed().unwrap(),
        "readiness must be emitted once"
    );
    handle
        .complete(
            ticket.attempt(),
            None,
            Outcome::Success,
            payload(),
            deadline(),
        )
        .await
        .unwrap();
    ticket.wait().await.unwrap();
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_should_report_initialization_failure_once_before_delivery_and_close_pending_work() {
    let mut session = RuntimeSession::start(Duration::from_secs(60)).unwrap();
    let notification = session.take_initialization_error().unwrap();
    assert!(session.take_initialization_error().is_none());
    let handle = session.handle().clone();
    let ticket = handle.submit(invocation(), deadline()).await.unwrap();
    handle
        .initialization_error(payload(), deadline())
        .await
        .unwrap();
    assert_eq!(notification.await.unwrap(), payload());
    assert_eq!(
        ticket.wait().await.unwrap(),
        RuntimeResult::InitError(payload())
    );
    assert_eq!(
        handle.next(deadline()).await.unwrap_err(),
        SessionError::Unavailable
    );
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_should_reject_initialization_error_after_any_delivery_without_consuming_active_result()
 {
    let mut session = RuntimeSession::start(Duration::from_secs(60)).unwrap();
    let handle = session.handle().clone();
    let ticket = handle.submit(invocation(), deadline()).await.unwrap();
    handle.next(deadline()).await.unwrap();
    assert_eq!(
        handle.initialization_error(payload(), deadline()).await,
        Err(SessionError::Busy)
    );
    handle
        .complete(
            ticket.attempt(),
            None,
            Outcome::Success,
            payload(),
            deadline(),
        )
        .await
        .unwrap();
    ticket.wait().await.unwrap();
    assert_eq!(
        handle.initialization_error(payload(), deadline()).await,
        Err(SessionError::Busy)
    );
    session.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn test_should_invalidate_dropped_tickets_and_expired_attempts_without_reuse() {
    for delivered in [false, true] {
        let mut session = RuntimeSession::start(Duration::from_secs(60)).unwrap();
        let handle = session.handle().clone();
        let ticket = handle.submit(invocation(), deadline()).await.unwrap();
        if delivered {
            handle.next(deadline()).await.unwrap();
        }
        drop(ticket);
        tokio::task::yield_now().await;
        assert_eq!(
            handle.submit(invocation(), deadline()).await.unwrap_err(),
            SessionError::Unavailable
        );
        session.shutdown().await.unwrap();
    }
    let mut session = RuntimeSession::start(Duration::from_secs(60)).unwrap();
    let handle = session.handle().clone();
    let ticket = handle.submit(invocation(), deadline()).await.unwrap();
    handle.next(deadline()).await.unwrap();
    tokio::time::advance(Duration::from_secs(5)).await;
    assert_eq!(ticket.wait().await.unwrap_err(), SessionError::Deadline);
    tokio::task::yield_now().await;
    assert_eq!(
        handle.next(deadline()).await.unwrap_err(),
        SessionError::Unavailable
    );
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_should_cancel_attached_waiters_and_never_cancel_new_work_when_an_old_ticket_drops() {
    let mut session = RuntimeSession::start(Duration::from_secs(60)).unwrap();
    let handle = session.handle().clone();
    let old = handle.submit(invocation(), deadline()).await.unwrap();
    handle.next(deadline()).await.unwrap();
    handle
        .complete(old.attempt(), None, Outcome::Success, payload(), deadline())
        .await
        .unwrap();
    let current = handle.submit(invocation(), deadline()).await.unwrap();
    drop(old);
    handle.next(deadline()).await.unwrap();
    handle
        .complete(
            current.attempt(),
            None,
            Outcome::Success,
            payload(),
            deadline(),
        )
        .await
        .unwrap();
    current.wait().await.unwrap();

    let cancelled = handle.submit(invocation(), deadline()).await.unwrap();
    let mut waiting = Box::pin(cancelled.wait());
    assert!(poll_once(&mut waiting).is_pending());
    drop(waiting);
    tokio::task::yield_now().await;
    assert_eq!(
        handle.next(deadline()).await.unwrap_err(),
        SessionError::Unavailable
    );
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_should_retain_actor_join_ownership_after_a_cancelled_shutdown_waiter() {
    let mut session = RuntimeSession::start(Duration::from_secs(60)).unwrap();
    let handle = session.handle().clone();
    let mut shutdown = Box::pin(session.shutdown());
    assert!(poll_once(&mut shutdown).is_pending());
    drop(shutdown);
    assert!(session.task.is_some());
    session.shutdown().await.unwrap();
    assert!(session.task.is_none());
    assert_eq!(
        handle.next(deadline()).await.unwrap_err(),
        SessionError::Unavailable
    );
}

#[tokio::test]
async fn test_should_cancel_admitted_work_after_the_admission_waiter_is_dropped() {
    let mut session = RuntimeSession::start(Duration::from_secs(60)).unwrap();
    let handle = session.handle().clone();
    let mut admission = Box::pin(handle.submit(invocation(), deadline()));
    assert!(poll_once(&mut admission).is_pending());
    tokio::task::yield_now().await;
    drop(admission);
    tokio::task::yield_now().await;
    assert_eq!(
        handle.next(deadline()).await.unwrap_err(),
        SessionError::Unavailable
    );
    session.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn test_should_bound_idle_and_command_deadlines_and_prioritize_stop_over_a_full_mailbox() {
    assert!(matches!(
        RuntimeSession::start(Duration::ZERO),
        Err(SessionError::InvalidMetadata)
    ));
    assert!(matches!(
        RuntimeSession::start(Duration::from_hours(25)),
        Err(SessionError::InvalidMetadata)
    ));
    let mut session = RuntimeSession::start(Duration::from_secs(1)).unwrap();
    let handle = session.handle().clone();
    assert_eq!(
        handle.next(Instant::now()).await.unwrap_err(),
        SessionError::Deadline
    );
    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    assert_eq!(
        handle.next(deadline()).await.unwrap_err(),
        SessionError::Unavailable
    );
    session.shutdown().await.unwrap();

    let mut session = RuntimeSession::start(Duration::from_secs(60)).unwrap();
    let handle = session.handle().clone();
    let mut receivers = Vec::new();
    for _ in 0..MAILBOX_CAPACITY {
        let (reply, receiver) = oneshot::channel();
        handle.send(Command::Next { reply }, deadline()).unwrap();
        receivers.push(receiver);
    }
    let (reply, _) = oneshot::channel();
    assert_eq!(
        handle.send(Command::Next { reply }, deadline()),
        Err(SessionError::Busy)
    );
    handle.close();
    session.shutdown().await.unwrap();
    assert!(
        !*handle.bootstrap.borrow(),
        "priority stop must beat queued polls"
    );
    for receiver in receivers {
        assert!(receiver.await.is_err());
    }
}

#[tokio::test]
async fn test_should_never_report_success_after_actor_supervision_failure() {
    let mut session = RuntimeSession::start(Duration::from_secs(60)).unwrap();
    session.task.as_ref().unwrap().abort();
    assert_eq!(
        session.shutdown().await,
        Err(SessionError::SupervisorFailed)
    );
    assert_eq!(
        session.shutdown().await,
        Err(SessionError::SupervisorFailed)
    );
}

#[test]
fn test_should_reject_start_without_a_tokio_runtime() {
    assert!(matches!(
        RuntimeSession::start(Duration::from_secs(60)),
        Err(SessionError::Unavailable)
    ));
}

#[tokio::test(start_paused = true)]
async fn test_should_reject_expired_mailbox_commands_without_delivering_or_settling_work() {
    let mut session = RuntimeSession::start(Duration::from_secs(60)).unwrap();
    let handle = session.handle().clone();
    let (reply, response) = oneshot::channel();
    handle
        .send(
            Command::Next { reply },
            Instant::now() + Duration::from_millis(1),
        )
        .unwrap();
    tokio::time::advance(Duration::from_millis(1)).await;
    assert!(matches!(
        response.await.unwrap(),
        Err(SessionError::Deadline)
    ));
    assert!(!*handle.bootstrap.borrow());

    let ticket = handle.submit(invocation(), deadline()).await.unwrap();
    handle.next(deadline()).await.unwrap();
    let (reply, response) = oneshot::channel();
    handle
        .send(
            Command::Complete {
                attempt: ticket.attempt(),
                invocation_id: None,
                outcome: Outcome::Success,
                payload: payload(),
                reply,
            },
            Instant::now() + Duration::from_millis(1),
        )
        .unwrap();
    tokio::time::advance(Duration::from_millis(1)).await;
    assert_eq!(response.await.unwrap(), Err(SessionError::Deadline));
    handle
        .complete(
            ticket.attempt(),
            None,
            Outcome::Success,
            payload(),
            deadline(),
        )
        .await
        .unwrap();
    assert_eq!(
        ticket.wait().await.unwrap(),
        RuntimeResult::Success(payload())
    );
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_should_signal_owned_actor_cleanup_when_the_session_owner_is_dropped() {
    let session = RuntimeSession::start(Duration::from_secs(60)).unwrap();
    let handle = session.handle().clone();
    let call = invocation();
    assert!(
        !format!("{call:?} {:?}", RuntimeResult::Success(payload())).contains("secret-payload")
    );
    let ticket = handle.submit(call, deadline()).await.unwrap();
    assert!(!format!("{ticket:?}").contains("secret-payload"));
    drop(session);
    assert_eq!(ticket.wait().await, Err(SessionError::Cancelled));
    assert_eq!(
        handle.next(deadline()).await,
        Err(SessionError::Unavailable)
    );
}

#[tokio::test]
async fn test_should_requeue_an_offered_event_if_the_poll_future_drops_before_receiving_it() {
    let mut session = RuntimeSession::start(Duration::from_secs(60)).unwrap();
    let handle = session.handle().clone();
    let call = invocation();
    let ticket = handle.submit(call.clone(), deadline()).await.unwrap();
    let mut poll = Box::pin(handle.next(deadline()));
    assert!(poll_once(&mut poll).is_pending());
    // The actor sends the event, but this poll future never consumes the reply.
    tokio::task::yield_now().await;
    drop(poll);
    assert_eq!(handle.next(deadline()).await.unwrap(), call);
    handle
        .complete(
            ticket.attempt(),
            None,
            Outcome::Success,
            payload(),
            deadline(),
        )
        .await
        .unwrap();
    ticket.wait().await.unwrap();
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_should_reconcile_ready_poll_receipts_before_completion_or_initialization_commands() {
    for initialization in [false, true] {
        let mut actor = transition_actor();
        let invocation = invocation();
        let attempt = invocation.attempt;
        let (_cancel, cancellation) = watch::channel(false);
        let (response, received) = oneshot::channel();
        let pending = Pending {
            attempt,
            deadline: invocation.deadline,
            timeout_error: SessionError::Deadline,
            cancellation,
            response,
        };
        let (receipt, acknowledged) = oneshot::channel();
        actor.phase = Phase::Offering {
            invocation,
            pending,
            receipt: acknowledged,
        };
        receipt.send(Instant::now()).unwrap();
        let (reply, result) = oneshot::channel();
        if initialization {
            assert!(!actor.command(Command::InitError {
                payload: payload(),
                reply
            }));
            assert_eq!(result.await.unwrap(), Err(SessionError::Busy));
            assert!(actor.ever_delivered);
            assert!(matches!(actor.phase, Phase::Delivered(_)));
            let (reply, _) = oneshot::channel();
            assert!(actor.complete(attempt, None, Outcome::Success, payload(), reply));
        } else {
            assert!(actor.command(Command::Complete {
                attempt,
                invocation_id: None,
                outcome: Outcome::Success,
                payload: payload(),
                reply
            }));
            assert_eq!(result.await.unwrap(), Ok(()));
        }
        assert_eq!(
            received.await.unwrap(),
            Ok(RuntimeResult::Success(payload()))
        );
    }
}

fn transition_actor() -> SessionActor {
    let (_, inbox) = mpsc::channel(MAILBOX_CAPACITY);
    let (_, stop) = watch::channel(false);
    let (bootstrap, _) = watch::channel(false);
    SessionActor::builder()
        .inbox(inbox)
        .stop(stop)
        .bootstrap(bootstrap)
        .initialization_error(None)
        .idle_lease(Duration::from_secs(60))
        .idle_deadline(Instant::now() + Duration::from_secs(60))
        .build()
}

#[tokio::test]
async fn test_should_resume_a_borrowed_ticket_result_after_select_cancellation_and_reject_repolling()
 {
    let mut session = RuntimeSession::start(Duration::from_secs(60)).unwrap();
    let handle = session.handle().clone();
    let mut ticket = handle.submit(invocation(), deadline()).await.unwrap();
    let mut result = Box::pin(ticket.result());
    assert!(poll_once(&mut result).is_pending());
    drop(result);
    assert!(!*ticket.cancellation.borrow());
    handle.next(deadline()).await.unwrap();
    handle
        .complete(
            ticket.attempt(),
            None,
            Outcome::Success,
            payload(),
            deadline(),
        )
        .await
        .unwrap();
    assert_eq!(
        ticket.result().await.unwrap(),
        RuntimeResult::Success(payload())
    );
    assert_eq!(ticket.result().await, Err(SessionError::Unavailable));
    session.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn test_should_revalidate_cancellation_closed_owners_and_expiry_at_every_delivery_transition()
{
    for transition in ["deliver", "complete", "receipt"] {
        for condition in ["cancelled", "closed", "expired"] {
            let mut actor = transition_actor();
            let invocation = invocation();
            let attempt = invocation.attempt;
            let (cancel, cancellation) = watch::channel(false);
            let mut cancel = Some(cancel);
            let (response, received) = oneshot::channel();
            let pending = Pending {
                attempt,
                deadline: invocation.deadline,
                timeout_error: SessionError::Deadline,
                cancellation,
                response,
            };
            match condition {
                "cancelled" => {
                    cancel.as_ref().unwrap().send_replace(true);
                }
                "closed" => {
                    cancel.take();
                }
                "expired" => tokio::time::advance(Duration::from_secs(5)).await,
                _ => panic!("unknown test case"),
            }
            let expected = if condition == "expired" {
                SessionError::Deadline
            } else {
                SessionError::Cancelled
            };
            match transition {
                "deliver" => {
                    actor.phase = Phase::Queued {
                        invocation,
                        pending,
                    };
                    let (poll, received) = oneshot::channel();
                    actor.poll = Some(poll);
                    actor.deliver();
                    assert!(matches!(received.await.unwrap(), Err(error) if error == expected));
                }
                "complete" => {
                    actor.phase = Phase::Delivered(pending);
                    let (reply, received) = oneshot::channel();
                    assert!(!actor.complete(attempt, None, Outcome::Success, payload(), reply));
                    assert_eq!(received.await.unwrap(), Err(expected));
                }
                "receipt" => {
                    let (_, receipt) = oneshot::channel();
                    actor.phase = Phase::Offering {
                        invocation,
                        pending,
                        receipt,
                    };
                    actor.received(Some(Instant::now()));
                }
                _ => panic!("unknown test case"),
            }
            assert_eq!(received.await.unwrap(), Err(expected));
            assert!(matches!(actor.phase, Phase::Closed));
            assert!(actor.inbox.is_closed());
        }
    }
}

#[tokio::test(start_paused = true)]
async fn test_should_arm_handler_timeout_only_when_the_runtime_consumes_the_offered_event() {
    let mut session = RuntimeSession::start(Duration::from_secs(60)).unwrap();
    let handle = session.handle().clone();
    let mut call = invocation();
    call.deadline = Instant::now() + Duration::from_secs(30);
    call = call.with_handler_timeout(HandlerTimeout::try_from(Duration::from_secs(1)).unwrap());
    let ticket = handle.submit(call.clone(), deadline()).await.unwrap();
    tokio::time::advance(Duration::from_secs(4)).await;
    assert!(!handle.is_closed());
    let mut abandoned = Box::pin(handle.next(deadline()));
    assert!(poll_once(&mut abandoned).is_pending());
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(4)).await;
    assert!(!handle.is_closed());
    drop(abandoned);
    let started = Instant::now();
    let delivered = handle.next(deadline()).await.unwrap();
    assert_eq!(delivered.parts().1, started + Duration::from_secs(1));
    assert_ne!(delivered.parts().1, call.parts().1);
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(ticket.wait().await, Err(SessionError::HandlerTimeout));
    assert!(handle.is_closed());
    session.shutdown().await.unwrap();
}
#[tokio::test(start_paused = true)]
async fn test_should_keep_outer_deadline_classification_when_shorter_or_tied_with_handler_timeout()
{
    for remaining in [Duration::from_millis(500), Duration::from_secs(1)] {
        let mut session = RuntimeSession::start(Duration::from_secs(60)).unwrap();
        let handle = session.handle().clone();
        let mut call = invocation();
        call.deadline = Instant::now() + remaining;
        call = call.with_handler_timeout(HandlerTimeout::try_from(Duration::from_secs(1)).unwrap());
        let expected = call.parts().1;
        let ticket = handle.submit(call, deadline()).await.unwrap();
        assert_eq!(handle.next(deadline()).await.unwrap().parts().1, expected);
        tokio::time::advance(remaining).await;
        assert_eq!(ticket.wait().await, Err(SessionError::Deadline));
        session.shutdown().await.unwrap();
    }
}
#[tokio::test(start_paused = true)]
async fn test_should_reject_late_handler_completion_and_delayed_consumption_receipt_without_replay()
{
    for transition in ["receipt", "complete", "receipt_after_outer"] {
        let mut actor = transition_actor();
        let call = invocation()
            .with_handler_timeout(HandlerTimeout::try_from(Duration::from_secs(1)).unwrap());
        let attempt = call.attempt;
        let (_cancel, cancellation) = watch::channel(false);
        let (response, received) = oneshot::channel();
        let pending = Pending {
            attempt,
            deadline: call.deadline,
            timeout_error: SessionError::Deadline,
            cancellation,
            response,
        };
        let (receipt, acknowledged) = oneshot::channel();
        actor.phase = Phase::Offering {
            invocation: call,
            pending,
            receipt: acknowledged,
        };
        receipt.send(Instant::now()).unwrap();
        if transition == "complete" {
            actor.reconcile_receipt();
        }
        tokio::time::advance(Duration::from_secs(
            if transition == "receipt_after_outer" {
                6
            } else {
                1
            },
        ))
        .await;
        let (reply, result) = oneshot::channel();
        assert!(!actor.command(Command::Complete {
            attempt,
            invocation_id: None,
            outcome: Outcome::Success,
            payload: payload(),
            reply
        }));
        assert_eq!(
            result.await.unwrap(),
            Err(if transition == "complete" {
                SessionError::HandlerTimeout
            } else {
                SessionError::Unavailable
            })
        );
        assert_eq!(received.await.unwrap(), Err(SessionError::HandlerTimeout));
        assert!(matches!(actor.phase, Phase::Closed));
    }
}
#[tokio::test(start_paused = true)]
async fn test_should_preserve_on_time_completion_when_the_ticket_waiter_resumes_after_handler_expiry()
 {
    let mut session = RuntimeSession::start(Duration::from_secs(60)).unwrap();
    let handle = session.handle().clone();
    let call = invocation()
        .with_handler_timeout(HandlerTimeout::try_from(Duration::from_secs(1)).unwrap());
    let ticket = handle.submit(call, deadline()).await.unwrap();
    handle.next(deadline()).await.unwrap();
    handle
        .complete(
            ticket.attempt(),
            None,
            Outcome::Success,
            payload(),
            deadline(),
        )
        .await
        .unwrap();
    tokio::time::advance(Duration::from_secs(2)).await;
    assert_eq!(ticket.wait().await, Ok(RuntimeResult::Success(payload())));
    assert!(!handle.is_closed());
    session.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn test_should_reject_an_offered_call_consumed_after_its_outer_deadline_before_receipting_delivery()
 {
    let mut session = RuntimeSession::start(Duration::from_secs(60)).unwrap();
    let handle = session.handle().clone();
    let mut call = invocation()
        .with_handler_timeout(HandlerTimeout::try_from(Duration::from_secs(1)).unwrap());
    call.deadline = Instant::now() + Duration::from_secs(2);
    let ticket = handle.submit(call, deadline()).await.unwrap();
    let mut next = Box::pin(handle.next(Instant::now() + Duration::from_secs(10)));
    assert!(poll_once(&mut next).is_pending());
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(2)).await;
    assert_eq!(next.await, Err(SessionError::Deadline));
    assert_eq!(ticket.wait().await, Err(SessionError::Deadline));
    tokio::time::timeout(Duration::from_secs(1), handle.closed())
        .await
        .unwrap();
    session.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn test_should_reconcile_consumed_delivery_when_receipt_and_old_outer_timer_become_ready_between_actor_polls()
 {
    let (commands, inbox) = mpsc::channel(MAILBOX_CAPACITY);
    let (stop, stopped) = watch::channel(false);
    let (bootstrap, _) = watch::channel(false);
    let mut actor = SessionActor::builder()
        .inbox(inbox)
        .stop(stopped)
        .bootstrap(bootstrap)
        .initialization_error(None)
        .idle_lease(Duration::from_secs(60))
        .idle_deadline(Instant::now() + Duration::from_secs(60))
        .build();
    let call = invocation()
        .with_handler_timeout(HandlerTimeout::try_from(Duration::from_secs(1)).unwrap());
    let attempt = call.attempt;
    let (_cancel, cancellation) = watch::channel(false);
    let (response, received) = oneshot::channel();
    let (receipt, acknowledged) = oneshot::channel();
    actor.phase = Phase::Offering {
        pending: Pending {
            attempt,
            deadline: call.deadline,
            timeout_error: SessionError::Deadline,
            cancellation,
            response,
        },
        invocation: call,
        receipt: acknowledged,
    };
    let mut running = Box::pin(actor.run());
    assert!(poll_once(&mut running).is_pending());
    receipt.send(Instant::now()).unwrap();
    tokio::time::advance(Duration::from_secs(6)).await;
    assert!(poll_once(&mut running).is_ready());
    assert_eq!(received.await.unwrap(), Err(SessionError::HandlerTimeout));
    drop((commands, stop));
}
