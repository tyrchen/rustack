//! Async ownership/history fixtures using explicit channel-backed task admission.

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use chrono::Utc;
use serde_json::{Value, json};
use tokio::{
    sync::mpsc,
    time::{Instant, timeout},
};

use super::{CleanupState, ExecutionError, ExecutionSupervisor};
use crate::{
    AslCompiler, ClockSample, EvaluationContext, ExecutionDriver, ExecutionLimits,
    ExecutionOutcome, HistoryLimits, LambdaTaskCommand, LambdaTaskPort, Payload, RuntimeLimits,
    TaskCall, TaskFailure, TaskResponder, TaskSupport, TenantScope,
};

#[derive(Debug)]
struct Port {
    sender: mpsc::Sender<(LambdaTaskCommand, TaskResponder)>,
    starts: AtomicUsize,
    panic: bool,
}
impl LambdaTaskPort for Port {
    fn start(&self, command: LambdaTaskCommand) -> Result<TaskCall, TaskFailure> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        assert!(!self.panic, "controlled actor failure");
        let (call, owner) = TaskCall::channel();
        self.sender
            .try_send((command, owner))
            .map_err(|_| TaskFailure::new("Lambda.TooManyRequestsException", None).unwrap())?;
        Ok(call)
    }
}
fn fixture(
    open: usize,
    dispatch: usize,
    history: HistoryLimits,
) -> (
    ExecutionSupervisor,
    mpsc::Receiver<(LambdaTaskCommand, TaskResponder)>,
    Arc<Port>,
) {
    let (sender, receiver) = mpsc::channel(32);
    let port = Arc::new(Port {
        sender,
        starts: AtomicUsize::new(0),
        panic: false,
    });
    let owner = ExecutionSupervisor::new(
        RuntimeLimits::new(open, dispatch, 2).unwrap(),
        history,
        256 * 1024 * 1024,
        Some(port.clone()),
    )
    .unwrap();
    (owner, receiver, port)
}
fn driver(definition: &Value, input: &Value) -> ExecutionDriver {
    let machine = AslCompiler::new(TenantScope::default(), TaskSupport::Lambda)
        .compile(&serde_json::to_vec(definition).unwrap())
        .unwrap()
        .machine()
        .clone();
    let input = Payload::from_value(input).unwrap();
    let context = EvaluationContext::from_value(
        &json!({"Execution":{"Input":input.value(),"Id":"execution-fixture"}}),
    )
    .unwrap();
    ExecutionDriver::new(
        machine,
        input,
        context,
        ClockSample::new(Instant::now().into_std(), Utc::now()),
        ExecutionLimits::new(Duration::from_secs(60), 100_000).unwrap(),
    )
    .unwrap()
}
fn task() -> Value {
    json!({"StartAt":"Work","States":{"Work":{"Type":"Task","Resource":"arn:aws:lambda:us-east-1:000000000000:function:Worker","End":true}}})
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}
async fn next(
    receiver: &mut mpsc::Receiver<(LambdaTaskCommand, TaskResponder)>,
) -> (LambdaTaskCommand, TaskResponder) {
    timeout(Duration::from_secs(5), receiver.recv())
        .await
        .unwrap()
        .unwrap()
}
fn kinds(view: &super::ExecutionView) -> Vec<String> {
    view.history()
        .page(1, 1000, true)
        .unwrap()
        .events()
        .iter()
        .map(|e| e.r#type.as_str().into())
        .collect()
}

#[tokio::test]
async fn test_should_execute_pure_states_and_publish_terminal_history_atomically() {
    let (mut owner, _, _) = fixture(2, 2, HistoryLimits::default());
    let definition = json!({"StartAt":"P","States":{"P":{"Type":"Pass","Result":{"n":7},"Next":"W"},"W":{"Type":"Wait","Seconds":0,"Next":"C"},"C":{"Type":"Choice","Choices":[{"Variable":"$.n","NumericEquals":7,"Next":"Done"}],"Default":"Failed"},"Done":{"Type":"Succeed"},"Failed":{"Type":"Fail"}}});
    let mut handle = owner
        .start(driver(&definition, &json!({"secret":"marker"})))
        .unwrap();
    let initial = handle.snapshot();
    assert!(initial.outcome().is_none());
    assert_eq!(initial.history().len(), 1);
    let view = handle.wait(deadline()).await.unwrap();
    assert!(
        matches!(view.outcome().unwrap().as_ref(),ExecutionOutcome::Succeeded(p) if p.value()==&json!({"n":7}))
    );
    assert_eq!(
        kinds(&view),
        [
            "ExecutionStarted",
            "PassStateEntered",
            "PassStateExited",
            "WaitStateEntered",
            "WaitStateExited",
            "ChoiceStateEntered",
            "ChoiceStateExited",
            "SucceedStateEntered",
            "SucceedStateExited",
            "ExecutionSucceeded"
        ]
    );
    let events = view.history().page(1, 100, true).unwrap();
    for (index, event) in events.events().iter().enumerate() {
        assert_eq!(event.id, i64::try_from(index + 1).unwrap());
        assert_eq!(event.previous_event_id, Some(i64::try_from(index).unwrap()));
    }
    assert_eq!(initial.history().len(), 1);
    owner.drain(deadline()).await.unwrap();
    assert_eq!(handle.snapshot().cleanup(), CleanupState::Clean);
}

#[tokio::test]
async fn test_should_execute_actual_channel_admission_and_fast_started_completion() {
    let (mut owner, mut calls, _) = fixture(2, 2, HistoryLimits::default());
    let mut handle = owner.start(driver(&task(), &json!({"n":3}))).unwrap();
    let (command, mut response) = next(&mut calls).await;
    assert_eq!(command.dispatch().parameters().value(), &json!({"n":3}));
    response.started().unwrap();
    response.finish(Ok(Payload::from_value(&json!({"computed":9})).unwrap()));
    let view = handle.wait(deadline()).await.unwrap();
    assert_eq!(
        kinds(&view),
        [
            "ExecutionStarted",
            "TaskStateEntered",
            "LambdaFunctionScheduled",
            "LambdaFunctionStarted",
            "LambdaFunctionSucceeded",
            "TaskStateExited",
            "ExecutionSucceeded"
        ]
    );
    owner.drain(deadline()).await.unwrap();
}

#[tokio::test]
async fn test_should_preserve_call_ownership_after_observer_drop() {
    let (mut owner, mut calls, port) = fixture(1, 1, HistoryLimits::default());
    let handle = owner.start(driver(&task(), &json!({}))).unwrap();
    let mut observer = handle.clone();
    drop(handle);
    let (_, mut response) = next(&mut calls).await;
    assert!(!*response.cancellation().borrow());
    response.started().unwrap();
    response.finish(Ok(Payload::parse(b"5").unwrap()));
    let view = observer.wait(deadline()).await.unwrap();
    assert!(matches!(
        view.outcome().unwrap().as_ref(),
        ExecutionOutcome::Succeeded(_)
    ));
    assert_eq!(port.starts.load(Ordering::SeqCst), 1);
    owner.drain(deadline()).await.unwrap();
}

#[tokio::test]
async fn test_should_publish_stop_then_retain_slot_and_cleanup_through_waiter_expiry() {
    let (mut owner, mut calls, _) = fixture(1, 1, HistoryLimits::default());
    let mut handle = owner.start(driver(&task(), &json!({}))).unwrap();
    let (_, mut response) = next(&mut calls).await;
    response.started().unwrap();
    let view = handle
        .stop(TaskFailure::new("Stopped", None).unwrap(), deadline())
        .await
        .unwrap();
    assert!(matches!(
        view.outcome().unwrap().as_ref(),
        ExecutionOutcome::Aborted(_)
    ));
    assert_eq!(
        kinds(&view).last().map(String::as_str),
        Some("ExecutionAborted")
    );
    assert!(*response.cancellation().borrow());
    assert_eq!(owner.owned_executions(), 1);
    assert!(matches!(
        owner.start(driver(&task(), &json!({}))),
        Err(ExecutionError::Capacity)
    ));
    assert!(matches!(
        owner
            .drain(Instant::now() + Duration::from_millis(20))
            .await,
        Err(ExecutionError::Deadline)
    ));
    assert_eq!(owner.owned_executions(), 1);
    response.finish(Err(TaskFailure::new("Cancelled", None).unwrap()));
    owner.drain(deadline()).await.unwrap();
    assert_eq!(owner.owned_executions(), 0);
    assert_eq!(handle.snapshot().cleanup(), CleanupState::Clean);
    assert_eq!(kinds(&handle.snapshot()), kinds(&view));
}

#[tokio::test]
async fn test_should_retry_uncertain_result_while_retaining_prior_cleanup_capacity() {
    let (mut owner, mut calls, port) = fixture(2, 2, HistoryLimits::default());
    let mut definition = task();
    definition["States"]["Work"]["Retry"] =
        json!([{"ErrorEquals":["Lambda.ServiceException"],"IntervalSeconds":1,"MaxAttempts":1}]);
    let mut handle = owner.start(driver(&definition, &json!({"n":2}))).unwrap();
    let (first, mut response) = next(&mut calls).await;
    response.started().unwrap();
    let retained = response
        .quarantine(TaskFailure::new("Lambda.ServiceException", Some("unknown result")).unwrap());
    let (second, mut response) = next(&mut calls).await;
    assert_ne!(first.dispatch().attempt(), second.dispatch().attempt());
    assert_eq!(second.dispatch().parameters().value(), &json!({"n":2}));
    response.started().unwrap();
    response.finish(Ok(Payload::parse(b"4").unwrap()));
    let view = handle.wait(deadline()).await.unwrap();
    assert!(matches!(
        view.outcome().unwrap().as_ref(),
        ExecutionOutcome::Succeeded(_)
    ));
    assert!(owner.shared.pools.dispatch.available_permits() < 2);
    assert_eq!(handle.snapshot().cleanup(), CleanupState::Quarantined);
    assert!(matches!(
        owner
            .drain(Instant::now() + Duration::from_millis(20))
            .await,
        Err(ExecutionError::Deadline)
    ));
    retained.settle();
    owner.drain(deadline()).await.unwrap();
    assert_eq!(owner.shared.pools.dispatch.available_permits(), 2);
    assert_eq!(port.starts.load(Ordering::SeqCst), 2);
    let names = kinds(&handle.snapshot());
    assert_eq!(names.iter().filter(|k| *k == "TaskStateEntered").count(), 1);
    assert_eq!(
        names
            .iter()
            .filter(|k| *k == "LambdaFunctionScheduled")
            .count(),
        2
    );
    assert!(!names.iter().any(|k| k.contains("Retry")));
}

#[tokio::test]
async fn test_should_retain_lost_cleanup_and_reject_clean_drain() {
    let (mut owner, mut calls, _) = fixture(1, 1, HistoryLimits::default());
    let mut handle = owner.start(driver(&task(), &json!({}))).unwrap();
    let (_, response) = next(&mut calls).await;
    drop(response);
    let view = handle.wait(deadline()).await.unwrap();
    assert!(matches!(
        view.outcome().unwrap().as_ref(),
        ExecutionOutcome::Failed(_)
    ));
    assert!(matches!(
        owner.drain(deadline()).await,
        Err(ExecutionError::Quarantined)
    ));
    assert_eq!(handle.snapshot().cleanup(), CleanupState::Quarantined);
    assert_eq!(owner.owned_executions(), 1);
    assert_eq!(owner.shared.pools.dispatch.available_permits(), 0);
    // The test explicitly drops the root only after proving retained ownership;
    // production's Drop fallback retains this bounded quarantine graph.
}

#[tokio::test]
async fn test_should_timeout_wait_and_call_without_accepting_late_success() {
    let (mut owner, mut calls, _) = fixture(2, 2, HistoryLimits::default());
    let mut definition = task();
    definition["TimeoutSeconds"] = json!(1);
    let mut handle = owner.start(driver(&definition, &json!({}))).unwrap();
    let (_, mut response) = next(&mut calls).await;
    response.started().unwrap();
    let view = handle.wait(deadline()).await.unwrap();
    assert!(matches!(
        view.outcome().unwrap().as_ref(),
        ExecutionOutcome::TimedOut
    ));
    assert!(*response.cancellation().borrow());
    let before = kinds(&view);
    assert_eq!(before.last().map(String::as_str), Some("ExecutionTimedOut"));
    response.finish(Ok(Payload::parse(b"99").unwrap()));
    owner.drain(deadline()).await.unwrap();
    assert_eq!(kinds(&handle.snapshot()), before);
    let (mut owner, _, _) = fixture(1, 1, HistoryLimits::default());
    let definition = json!({"TimeoutSeconds":1,"StartAt":"Wait","States":{"Wait":{"Type":"Wait","Seconds":50,"End":true}}});
    let mut handle = owner.start(driver(&definition, &json!({}))).unwrap();
    let view = handle.wait(deadline()).await.unwrap();
    assert!(matches!(
        view.outcome().unwrap().as_ref(),
        ExecutionOutcome::TimedOut
    ));
    owner.drain(deadline()).await.unwrap();
}

#[tokio::test]
async fn test_should_catch_task_timeout_and_keep_cancelled_call_until_clean() {
    let (mut owner, mut calls, _) = fixture(1, 1, HistoryLimits::default());
    let mut definition = task();
    definition["States"]["Work"]["TimeoutSeconds"] = json!(1);
    definition["States"]["Work"]["Catch"] =
        json!([{"ErrorEquals":["States.Timeout"],"Next":"Recovered"}]);
    definition["States"]["Recovered"] = json!({"Type":"Succeed"});
    let mut handle = owner.start(driver(&definition, &json!({}))).unwrap();
    let (_, mut response) = next(&mut calls).await;
    response.started().unwrap();
    let view = handle.wait(deadline()).await.unwrap();
    assert!(matches!(
        view.outcome().unwrap().as_ref(),
        ExecutionOutcome::Succeeded(_)
    ));
    assert!(kinds(&view).iter().any(|k| k == "LambdaFunctionTimedOut"));
    assert!(*response.cancellation().borrow());
    response.finish(Ok(Payload::parse(b"true").unwrap()));
    owner.drain(deadline()).await.unwrap();
}

#[tokio::test]
async fn test_should_timeout_while_waiting_for_dispatch_without_issuing_a_call() {
    let (mut owner, mut calls, port) = fixture(2, 1, HistoryLimits::default());
    let mut first = owner.start(driver(&task(), &json!({}))).unwrap();
    let (_, mut response) = next(&mut calls).await;
    response.started().unwrap();
    let mut definition = task();
    definition["States"]["Work"]["TimeoutSeconds"] = json!(1);
    let mut waiting = owner.start(driver(&definition, &json!({}))).unwrap();
    let view = waiting.wait(deadline()).await.unwrap();
    assert!(
        matches!(view.outcome().unwrap().as_ref(),ExecutionOutcome::Failed(f) if f.error()==Some("States.Timeout"))
    );
    assert_eq!(port.starts.load(Ordering::SeqCst), 1);
    assert!(calls.try_recv().is_err());
    let _ = first
        .stop(TaskFailure::new("Stopped", None).unwrap(), deadline())
        .await
        .unwrap();
    response.finish(Err(TaskFailure::new("Cancelled", None).unwrap()));
    owner.drain(deadline()).await.unwrap();
}

#[tokio::test]
async fn test_should_stop_while_waiting_for_evaluation_capacity() {
    let (mut owner, _, port) = fixture(1, 1, HistoryLimits::default());
    let held = owner
        .shared
        .pools
        .cpu
        .clone()
        .acquire_many_owned(2)
        .await
        .unwrap();
    let mut handle = owner.start(driver(&task(), &json!({}))).unwrap();
    let view = handle
        .stop(TaskFailure::new("Stopped", None).unwrap(), deadline())
        .await
        .unwrap();
    assert!(matches!(
        view.outcome().unwrap().as_ref(),
        ExecutionOutcome::Aborted(_)
    ));
    assert_eq!(view.history().len(), 2);
    assert_eq!(port.starts.load(Ordering::SeqCst), 0);
    drop(held);
    owner.drain(deadline()).await.unwrap();
}

#[tokio::test]
async fn test_should_supervise_actor_panic_without_releasing_dispatch_ownership() {
    let (sender, _receiver) = mpsc::channel(1);
    let port = Arc::new(Port {
        sender,
        starts: AtomicUsize::new(0),
        panic: true,
    });
    let mut owner = ExecutionSupervisor::new(
        RuntimeLimits::new(1, 1, 1).unwrap(),
        HistoryLimits::default(),
        256 * 1024 * 1024,
        Some(port),
    )
    .unwrap();
    let mut handle = owner.start(driver(&task(), &json!({}))).unwrap();
    timeout(Duration::from_secs(5), owner.reap_next())
        .await
        .unwrap();
    let view = handle.wait(deadline()).await.unwrap();
    assert!(
        matches!(view.outcome().unwrap().as_ref(),ExecutionOutcome::Failed(f) if f.error()==Some("Rustack.InternalError"))
    );
    assert_eq!(
        kinds(&view).last().map(String::as_str),
        Some("ExecutionFailed")
    );
    assert_eq!(owner.shared.pools.dispatch.available_permits(), 0);
    assert_eq!(owner.owned_executions(), 1);
    assert!(matches!(
        owner.drain(deadline()).await,
        Err(ExecutionError::Quarantined)
    ));
}

#[tokio::test]
async fn test_should_reserve_terminal_history_and_avoid_invocation_on_history_exhaustion() {
    let (mut owner, _, port) = fixture(1, 1, HistoryLimits::new(3, 1024 * 1024).unwrap());
    let mut handle = owner.start(driver(&task(), &json!({}))).unwrap();
    let view = handle.wait(deadline()).await.unwrap();
    assert!(
        matches!(view.outcome().unwrap().as_ref(),ExecutionOutcome::Failed(f) if f.error()==Some("States.HistoryLimitExceeded"))
    );
    assert_eq!(
        kinds(&view),
        ["ExecutionStarted", "TaskStateEntered", "ExecutionFailed"]
    );
    assert_eq!(port.starts.load(Ordering::SeqCst), 0);
    owner.drain(deadline()).await.unwrap();
}

#[tokio::test]
async fn test_should_replace_unretainable_success_with_reserved_failure_event() {
    let (mut owner, _, _) = fixture(1, 1, HistoryLimits::new(25_000, 1024 * 1024).unwrap());
    let definition = json!({"StartAt":"Done","States":{"Done":{"Type":"Succeed"}}});
    let mut handle = owner
        .start(driver(&definition, &json!("\0".repeat(40_000))))
        .unwrap();
    let view = handle.wait(deadline()).await.unwrap();
    assert!(
        matches!(view.outcome().unwrap().as_ref(),ExecutionOutcome::Failed(f) if f.error()==Some("States.HistoryLimitExceeded"))
    );
    let names = kinds(&view);
    assert_eq!(names.last().map(String::as_str), Some("ExecutionFailed"));
    assert!(!names.iter().any(|k| k == "ExecutionSucceeded"));
    owner.drain(deadline()).await.unwrap();
}

#[tokio::test]
async fn test_should_preserve_original_history_and_omit_input_output_parameters_in_projection() {
    let (mut owner, mut calls, _) = fixture(1, 1, HistoryLimits::default());
    let definition = json!({"StartAt":"Work","States":{"Work":{"Type":"Task","Resource":"arn:aws:states:::lambda:invoke","Parameters":{"FunctionName":"Worker","Payload.$":"$"},"End":true}}});
    let mut handle = owner
        .start(driver(&definition, &json!({"secret":"projection-marker"})))
        .unwrap();
    let (_, mut response) = next(&mut calls).await;
    response.started().unwrap();
    response.finish(Ok(Payload::from_value(
        &json!({"Payload":{"secret":"projection-marker"}}),
    )
    .unwrap()));
    let view = handle.wait(deadline()).await.unwrap();
    let full = view.history().page(1, 100, true).unwrap();
    let omitted = view.history().page(1, 100, false).unwrap();
    assert!(
        serde_json::to_string(full.events())
            .unwrap()
            .contains("projection-marker")
    );
    assert!(
        !serde_json::to_string(omitted.events())
            .unwrap()
            .contains("projection-marker")
    );
    assert_eq!(
        full.events().iter().map(|e| e.id).collect::<Vec<_>>(),
        omitted.events().iter().map(|e| e.id).collect::<Vec<_>>()
    );
    assert_eq!(
        full.events()
            .iter()
            .map(|e| e.previous_event_id)
            .collect::<Vec<_>>(),
        omitted
            .events()
            .iter()
            .map(|e| e.previous_event_id)
            .collect::<Vec<_>>()
    );
    owner.drain(deadline()).await.unwrap();
}

#[tokio::test]
async fn test_should_retain_history_budget_until_last_observer_view_is_dropped() {
    let (mut owner, _, _) = fixture(1, 1, HistoryLimits::default());
    let definition = json!({"StartAt":"Done","States":{"Done":{"Type":"Succeed"}}});
    let mut handle = owner.start(driver(&definition, &json!({"n":1}))).unwrap();
    let view = handle.wait(deadline()).await.unwrap();
    owner.reap_next().await;
    assert!(owner.history_bytes() > 0);
    drop(handle);
    assert!(owner.history_bytes() > 0);
    drop(view);
    assert_eq!(owner.history_bytes(), 0);
    owner.drain(deadline()).await.unwrap();
}

#[tokio::test]
async fn test_should_finish_pure_loop_stop_without_starving_control() {
    let (mut owner, _, _) = fixture(1, 1, HistoryLimits::default());
    let definition = json!({"StartAt":"Loop","States":{"Loop":{"Type":"Choice","Choices":[{"Variable":"$","BooleanEquals":true,"Next":"Loop"}],"Default":"Done"},"Done":{"Type":"Succeed"}}});
    let mut handle = owner.start(driver(&definition, &json!(true))).unwrap();
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    let view = handle
        .stop(TaskFailure::new("Stopped", None).unwrap(), deadline())
        .await
        .unwrap();
    assert!(matches!(
        view.outcome().unwrap().as_ref(),
        ExecutionOutcome::Aborted(_)
    ));
    owner.drain(deadline()).await.unwrap();
}

#[tokio::test]
async fn test_should_transfer_drop_to_fallback_cleanup_owner() {
    let (mut owner, mut calls, _) = fixture(1, 1, HistoryLimits::default());
    let mut handle = owner.start(driver(&task(), &json!({}))).unwrap();
    let (_, mut response) = next(&mut calls).await;
    response.started().unwrap();
    drop(owner);
    let view = handle.wait(deadline()).await.unwrap();
    assert!(matches!(
        view.outcome().unwrap().as_ref(),
        ExecutionOutcome::Aborted(_)
    ));
    assert!(*response.cancellation().borrow());
    response.finish(Err(TaskFailure::new("Cancelled", None).unwrap()));
    timeout(Duration::from_secs(5), async {
        while handle.snapshot().cleanup() != CleanupState::Clean {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[test]
fn test_should_keep_fallback_ownership_when_its_runtime_shuts_down() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (shared, response) = runtime.block_on(async {
        let (mut owner, mut calls, _) = fixture(1, 1, HistoryLimits::default());
        let shared = owner.shared.clone();
        let mut handle = owner.start(driver(&task(), &json!({}))).unwrap();
        let (_, mut response) = next(&mut calls).await;
        response.started().unwrap();
        drop(owner);
        let view = handle.wait(deadline()).await.unwrap();
        assert!(matches!(
            view.outcome().unwrap().as_ref(),
            ExecutionOutcome::Aborted(_)
        ));
        (shared, response)
    });
    runtime.shutdown_timeout(Duration::from_millis(100));
    assert_eq!(shared.pools.open.available_permits(), 0);
    assert_eq!(shared.pools.dispatch.available_permits(), 0);
    assert!(shared.history.capacity.used() > 0);
    // The adapter receipt outlives the stopped execution runtime. There is no
    // executor left to prove joined cleanup; process-lifetime quarantine is correct.
    response.finish(Err(TaskFailure::new("Cancelled", None).unwrap()));
    assert_eq!(shared.pools.dispatch.available_permits(), 0);
}

#[derive(Debug)]
struct RejectPort {
    stop: Option<tokio::sync::watch::Sender<Option<TaskFailure>>>,
    delay: Duration,
}
impl LambdaTaskPort for RejectPort {
    fn start(&self, _command: LambdaTaskCommand) -> Result<TaskCall, TaskFailure> {
        if let Some(stop) = &self.stop {
            stop.send_replace(Some(TaskFailure::new("Stopped", None).unwrap()));
        }
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
        Err(TaskFailure::new("Lambda.ServiceException", Some("admission rejected")).unwrap())
    }
}
async fn rejected_admission(stop_now: bool) -> Arc<super::ExecutionView> {
    use super::{
        super::{
            Phase,
            history::{Journal, TerminalReserve, events},
        },
        actor::Actor,
    };
    let (stop, cancelled) = tokio::sync::watch::channel(None);
    let port = Arc::new(RejectPort {
        stop: stop_now.then_some(stop.clone()),
        delay: if stop_now {
            Duration::ZERO
        } else {
            Duration::from_millis(10)
        },
    });
    let owner = ExecutionSupervisor::new(
        RuntimeLimits::new(1, 1, 1).unwrap(),
        HistoryLimits::default(),
        256 * 1024 * 1024,
        Some(port),
    )
    .unwrap();
    let mut driver = driver(&task(), &json!({}));
    let update = driver.advance(
        ClockSample::new(Instant::now().into_std(), Utc::now()),
        &mut crate::SystemUuidSource::default(),
    );
    let crate::DriverStep::Dispatch(mut dispatch) = update.step else {
        panic!("expected dispatch")
    };
    if !stop_now {
        dispatch.deadline = Instant::now().into_std() + Duration::from_millis(2);
        if let Phase::Calling { deadline, .. } = &mut driver.phase {
            *deadline = dispatch.deadline;
        }
    }
    let reserve = TerminalReserve::new(&owner.shared.history.capacity).unwrap();
    let mut journal = Journal::new(
        owner.shared.history.limits,
        owner.shared.history.capacity.clone(),
        reserve,
    );
    journal
        .append(events::started(driver.input(), Utc::now()).unwrap(), false)
        .unwrap();
    journal
        .append(
            events::entered(driver.machine(), driver.state(), driver.input(), Utc::now()).unwrap(),
            false,
        )
        .unwrap();
    let (view, reader) = tokio::sync::watch::channel(Arc::new(super::ExecutionView {
        history: journal.view.clone(),
        outcome: None,
        stopped: None,
        cleanup: CleanupState::Pending,
        diagnostic: None,
    }));
    let (ledger, _reader) = tokio::sync::watch::channel(Arc::new(Vec::new()));
    let mut actor = Actor::new(
        driver,
        journal,
        view,
        cancelled,
        ledger,
        owner.shared.clone(),
    );
    actor.dispatch(dispatch).await;
    actor.run().await;
    reader.borrow().clone()
}
#[tokio::test]
async fn test_should_recheck_stop_after_synchronous_admission_rejection() {
    let view = rejected_admission(true).await;
    assert!(matches!(
        view.outcome().unwrap().as_ref(),
        ExecutionOutcome::Aborted(_)
    ));
    let names = kinds(&view);
    assert!(!names.iter().any(|k| k == "LambdaFunctionScheduleFailed"));
    assert_eq!(names.last().map(String::as_str), Some("ExecutionAborted"));
}
#[tokio::test]
async fn test_should_recheck_deadline_after_synchronous_admission_rejection() {
    let view = rejected_admission(false).await;
    assert!(
        matches!(view.outcome().unwrap().as_ref(),ExecutionOutcome::Failed(f) if f.error()==Some("States.Timeout"))
    );
    let names = kinds(&view);
    assert!(!names.iter().any(|k| k == "LambdaFunctionScheduleFailed"));
    assert!(names.iter().any(|k| k == "LambdaFunctionTimedOut"));
}

#[derive(Debug)]
pub(super) struct WorkerHook {
    entered: tokio::sync::Notify,
    released: std::sync::atomic::AtomicBool,
    skip: AtomicUsize,
    panic: bool,
    before: bool,
}
impl WorkerHook {
    fn install(
        owner: &mut ExecutionSupervisor,
        skip: usize,
        panic: bool,
        before: bool,
    ) -> WorkerGate {
        let hook = Arc::new(Self {
            entered: tokio::sync::Notify::new(),
            released: std::sync::atomic::AtomicBool::new(false),
            skip: AtomicUsize::new(skip),
            panic,
            before,
        });
        Arc::get_mut(&mut owner.shared).unwrap().worker_hook = Some(hook.clone());
        WorkerGate { hook }
    }
    pub(super) fn run(&self, before: bool) {
        if self.before != before
            || self
                .skip
                .try_update(Ordering::AcqRel, Ordering::Acquire, |skip| {
                    skip.checked_sub(1)
                })
                .is_ok()
        {
            return;
        }
        self.entered.notify_one();
        while !self.released.load(Ordering::Acquire) {
            std::thread::park_timeout(Duration::from_millis(1));
        }
        assert!(!self.panic, "controlled CPU worker failure");
    }
    fn release(&self) {
        self.released.store(true, Ordering::Release);
    }
}
#[derive(Debug)]
struct WorkerGate {
    hook: Arc<WorkerHook>,
}
impl std::ops::Deref for WorkerGate {
    type Target = WorkerHook;
    fn deref(&self) -> &Self::Target {
        &self.hook
    }
}
impl Drop for WorkerGate {
    fn drop(&mut self) {
        self.hook.release();
    }
}
#[tokio::test]
async fn test_should_publish_stop_while_cpu_worker_remains_owned_and_unjoined() {
    let (mut owner, _, port) = fixture(1, 1, HistoryLimits::default());
    let gate = WorkerHook::install(&mut owner, 0, false, true);
    let mut handle = owner.start(driver(&task(), &json!({}))).unwrap();
    timeout(Duration::from_secs(5), gate.entered.notified())
        .await
        .unwrap();
    let view = handle
        .stop(TaskFailure::new("Stopped", None).unwrap(), deadline())
        .await
        .unwrap();
    assert!(matches!(
        view.outcome().unwrap().as_ref(),
        ExecutionOutcome::Aborted(_)
    ));
    assert_eq!(port.starts.load(Ordering::SeqCst), 0);
    assert_eq!(owner.shared.pools.cpu.available_permits(), 1);
    assert!(matches!(
        owner
            .drain(Instant::now() + Duration::from_millis(10))
            .await,
        Err(ExecutionError::Deadline)
    ));
    gate.release();
    owner.drain(deadline()).await.unwrap();
    assert_eq!(owner.shared.pools.cpu.available_permits(), 2);
}
#[tokio::test]
async fn test_should_reject_prepared_dispatch_after_overall_deadline_before_cpu_return() {
    let (mut owner, _, port) = fixture(1, 1, HistoryLimits::default());
    let gate = WorkerHook::install(&mut owner, 0, false, false);
    let mut definition = task();
    definition["TimeoutSeconds"] = json!(1);
    let mut handle = owner.start(driver(&definition, &json!({}))).unwrap();
    timeout(Duration::from_secs(5), gate.entered.notified())
        .await
        .unwrap();
    let view = handle.wait(deadline()).await.unwrap();
    assert!(matches!(
        view.outcome().unwrap().as_ref(),
        ExecutionOutcome::TimedOut
    ));
    assert_eq!(port.starts.load(Ordering::SeqCst), 0);
    assert_eq!(kinds(&view), ["ExecutionStarted", "ExecutionTimedOut"]);
    gate.release();
    owner.drain(deadline()).await.unwrap();
    assert_eq!(kinds(&handle.snapshot()), kinds(&view));
}
#[tokio::test]
async fn test_should_not_publish_success_when_execution_expires_during_result_processing() {
    let (mut owner, mut calls, _) = fixture(1, 1, HistoryLimits::default());
    let gate = WorkerHook::install(&mut owner, 1, false, false);
    let mut definition = task();
    definition["TimeoutSeconds"] = json!(1);
    let mut handle = owner.start(driver(&definition, &json!({}))).unwrap();
    let (_, mut response) = next(&mut calls).await;
    response.started().unwrap();
    response.finish(Ok(Payload::parse(b"true").unwrap()));
    timeout(Duration::from_secs(5), gate.entered.notified())
        .await
        .unwrap();
    let view = handle.wait(deadline()).await.unwrap();
    assert!(matches!(
        view.outcome().unwrap().as_ref(),
        ExecutionOutcome::TimedOut
    ));
    let names = kinds(&view);
    assert!(
        !names
            .iter()
            .any(|k| k == "ExecutionSucceeded" || k == "TaskStateExited")
    );
    assert!(names.iter().any(|k| k == "LambdaFunctionSucceeded"));
    gate.release();
    owner.drain(deadline()).await.unwrap();
}
#[tokio::test]
async fn test_should_keep_cpu_panic_source_and_fail_without_replaying_business_work() {
    use std::error::Error;
    let (mut owner, _, port) = fixture(1, 1, HistoryLimits::default());
    let gate = WorkerHook::install(&mut owner, 0, true, true);
    let mut handle = owner.start(driver(&task(), &json!({}))).unwrap();
    timeout(Duration::from_secs(5), gate.entered.notified())
        .await
        .unwrap();
    gate.release();
    let view = handle.wait(deadline()).await.unwrap();
    assert!(
        matches!(view.outcome().unwrap().as_ref(),ExecutionOutcome::Failed(f) if f.error()==Some("Rustack.InternalError"))
    );
    assert!(matches!(view.diagnostic(), Some(ExecutionError::Worker(_))));
    assert!(view.diagnostic().unwrap().source().is_some());
    assert_eq!(port.starts.load(Ordering::SeqCst), 0);
    assert!(!format!("{view:?}").contains("controlled CPU"));
    owner.drain(deadline()).await.unwrap();
}

#[test]
fn test_should_reject_invalid_runtime_limits_and_start_without_tokio() {
    for (open, dispatch, cpu) in [
        (0, 1, 1),
        (4097, 1, 1),
        (1, 0, 1),
        (1, 1025, 1),
        (1, 1, 0),
        (1, 1, 129),
    ] {
        assert!(RuntimeLimits::new(open, dispatch, cpu).is_err());
    }
    let mut owner = ExecutionSupervisor::new(
        RuntimeLimits::default(),
        HistoryLimits::default(),
        256 * 1024 * 1024,
        None,
    )
    .unwrap();
    let definition = json!({"StartAt":"Done","States":{"Done":{"Type":"Succeed"}}});
    assert!(matches!(
        owner.start(driver(&definition, &json!({}))),
        Err(ExecutionError::Unavailable)
    ));
    assert_eq!(owner.owned_executions(), 0);
    assert_eq!(owner.history_bytes(), 0);
}
#[tokio::test]
async fn test_should_support_pure_execution_without_lambda_and_reject_unsupported_start() {
    let mut owner = ExecutionSupervisor::new(
        RuntimeLimits::default(),
        HistoryLimits::default(),
        256 * 1024 * 1024,
        None,
    )
    .unwrap();
    let definition = json!({"StartAt":"Done","States":{"Done":{"Type":"Succeed"}}});
    assert!(matches!(
        owner.start(driver(&task(), &json!({}))),
        Err(ExecutionError::Unavailable)
    ));
    let mut used = driver(&definition, &json!({}));
    let _ = used.advance(
        ClockSample::new(Instant::now().into_std(), Utc::now()),
        &mut crate::SystemUuidSource::default(),
    );
    assert!(matches!(
        owner.start(used),
        Err(ExecutionError::Unavailable)
    ));
    let mut handle = owner.start(driver(&definition, &json!({}))).unwrap();
    handle.wait(deadline()).await.unwrap();
    owner.drain(deadline()).await.unwrap();
    assert!(matches!(
        owner.start(driver(&definition, &json!({}))),
        Err(ExecutionError::Unavailable)
    ));
}
#[tokio::test]
async fn test_should_not_cancel_work_when_observation_deadline_expires() {
    let (mut owner, mut calls, _) = fixture(1, 1, HistoryLimits::default());
    let mut handle = owner.start(driver(&task(), &json!({}))).unwrap();
    let (_, mut response) = next(&mut calls).await;
    assert!(matches!(
        handle.wait(Instant::now()).await,
        Err(ExecutionError::Deadline)
    ));
    assert!(!*response.cancellation().borrow());
    response.started().unwrap();
    response.finish(Ok(Payload::parse(b"1").unwrap()));
    let view = handle.wait(deadline()).await.unwrap();
    let repeated = handle
        .stop(TaskFailure::new("Stopped", None).unwrap(), deadline())
        .await
        .unwrap();
    assert!(Arc::ptr_eq(
        view.outcome().unwrap(),
        repeated.outcome().unwrap()
    ));
    assert_eq!(view.stop_date(), repeated.stop_date());
    owner.drain(deadline()).await.unwrap();
}
#[tokio::test]
async fn test_should_charge_retained_terminal_histories_against_global_admission() {
    let mut owner = ExecutionSupervisor::new(
        RuntimeLimits::new(1, 1, 1).unwrap(),
        HistoryLimits::new(25_000, 1024 * 1024).unwrap(),
        8 * 1024 * 1024,
        None,
    )
    .unwrap();
    let definition = json!({"StartAt":"Done","States":{"Done":{"Type":"Succeed"}}});
    let mut views = Vec::new();
    let mut exhausted = false;
    for _ in 0..32 {
        match owner.start(driver(&definition, &json!("\0".repeat(40_000)))) {
            Ok(mut handle) => {
                let view = handle.wait(deadline()).await.unwrap();
                owner.reap_next().await;
                views.push(view);
            }
            Err(ExecutionError::Capacity) => {
                exhausted = true;
                break;
            }
            Err(other) => panic!("unexpected {other:?}"),
        }
    }
    assert!(exhausted);
    assert!(!views.is_empty());
    assert_eq!(owner.owned_executions(), 0);
    assert!(owner.history_bytes() <= 8 * 1024 * 1024);
    drop(views);
    assert_eq!(owner.history_bytes(), 0);
    owner.drain(deadline()).await.unwrap();
}
#[tokio::test]
async fn test_should_reject_unstarted_success_without_fabricating_start_history() {
    let (mut owner, mut calls, _) = fixture(1, 1, HistoryLimits::default());
    let mut handle = owner.start(driver(&task(), &json!({}))).unwrap();
    let (_, response) = next(&mut calls).await;
    response.finish(Ok(Payload::parse(b"true").unwrap()));
    let view = handle.wait(deadline()).await.unwrap();
    assert!(
        matches!(view.outcome().unwrap().as_ref(),ExecutionOutcome::Failed(f) if f.error()==Some("Rustack.InternalError"))
    );
    let names = kinds(&view);
    assert!(
        !names
            .iter()
            .any(|k| k == "LambdaFunctionStarted" || k == "LambdaFunctionSucceeded")
    );
    owner.drain(deadline()).await.unwrap();
}

mod structured;
