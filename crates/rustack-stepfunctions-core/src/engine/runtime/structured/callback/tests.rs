//! Barrier proofs for callback result charges above frame/event retirement.

use chrono::Utc;
use serde_json::json;
use tokio::{sync::watch, time::Instant};

use super::*;
use crate::{
    AslCompiler, ClockSample, DriverStep, EvaluationContext, ExecutionDriver, ExecutionLimits,
    ExecutionSupervisor, ExecutionView, FrameCapacity, FrameLimits, FrameScheduler, FrameUpdate,
    HistoryLimits, Payload, RuntimeLimits, StateSupport, SystemUuidSource, TaskSupport,
    TenantScope,
    engine::{
        history::{Journal, TerminalReserve},
        runtime::{
            CleanupState,
            structured::{Call, Completion},
        },
    },
};

fn graph(
    bytes: usize,
) -> (
    FrameScheduler,
    FrameCallId,
    CallbackDispatch,
    ClockSample,
    Arc<FrameCapacity>,
) {
    let source = json!({"StartAt":"Both","States":{"Both":{"Type":"Parallel","Branches":[{"StartAt":"Call","States":{"Call":{"Type":"Task","Resource":"arn:aws:states:::lambda:invoke.waitForTaskToken","End":true}}}],"End":true}}});
    let machine = AslCompiler::new(TenantScope::default(), TaskSupport::Lambda)
        .with_callback_support(true)
        .with_state_support(StateSupport::Structured)
        .compile(&serde_json::to_vec(&source).unwrap())
        .unwrap()
        .machine()
        .clone();
    let now = ClockSample::new(Instant::now().into_std(), Utc::now());
    let driver = ExecutionDriver::for_structured_owner(
        machine,
        Payload::parse(b"{}").unwrap(),
        EvaluationContext::from_value(&json!({})).unwrap(),
        now,
        ExecutionLimits::default(),
    )
    .unwrap();
    let capacity = Arc::new(FrameCapacity::new(8, bytes).unwrap());
    let mut graph = FrameScheduler::new(
        driver,
        capacity.clone(),
        FrameLimits::new(8, bytes).unwrap(),
    )
    .unwrap();
    for _ in 0..20 {
        if let FrameUpdate::Transition(t) = graph.advance(now, &mut SystemUuidSource::default()) {
            if let DriverStep::Callback(effect) = t.step() {
                return (graph, t.call().unwrap(), effect.clone(), now, capacity);
            }
        }
    }
    panic!("callback effect")
}
#[tokio::test]
async fn test_should_keep_callback_bytes_through_retired_child_until_result_notification_is_consumed()
 {
    let (mut graph, id, effect, now, capacity) = graph(600_000);
    let budget = graph.task_data_budget();
    let lease = Arc::new(DispatchLease {
        _permit: None,
        _frame: graph.lease(id.frame()),
        _data: Some(budget.reserve(RESULT_BYTES).unwrap()),
    });
    let _ = graph
        .complete(
            id,
            Ok(Payload::from_value(&json!("x".repeat(120_000))).unwrap()),
            now,
            &mut SystemUuidSource::default(),
        )
        .unwrap();
    let runtime = ExecutionSupervisor::new(
        RuntimeLimits::default(),
        HistoryLimits::default(),
        256 * 1024 * 1024,
        None,
    )
    .unwrap();
    let history = Journal::new(
        HistoryLimits::default(),
        runtime.shared.history.capacity.clone(),
        TerminalReserve::new(&runtime.shared.history.capacity).unwrap(),
    );
    let (view, _reader) = watch::channel(Arc::new(ExecutionView {
        history: history.view.clone(),
        outcome: None,
        stopped: None,
        cleanup: CleanupState::Pending,
        diagnostic: None,
    }));
    let (_stop, stopped) = watch::channel(None);
    let (ledger, _ledger) = watch::channel(Arc::new(Vec::<Arc<DispatchLease>>::new()));
    let mut actor = Actor::new(
        graph,
        Instant::from_std(effect.deadline()),
        history,
        view,
        stopped,
        ledger,
        runtime.shared.clone(),
    );
    let (call, responder) = TaskCall::channel();
    let cancel = call.cancellation_sender();
    responder.finish(Err(TaskFailure::service_exception()));
    drop(call);
    actor.leases.push(lease.clone());
    actor.sync_ledger();
    actor.calls.insert(
        id,
        Call {
            cancel,
            lease,
            machine: effect.machine().clone(),
            dispatch: Instruction::Callback(effect),
            progress: Progress::Submitted,
            result_seen: true,
            cancelled: false,
            settled: Some(true),
        },
    );
    actor.acks.insert(id);
    actor.reconcile();
    assert!(actor.calls.contains_key(&id));
    assert!(budget.reserve(RESULT_BYTES).is_err());
    let mut delivered = false;
    for _ in 0..20 {
        let update = actor
            .graph
            .as_mut()
            .unwrap()
            .advance(now, &mut SystemUuidSource::default());
        if matches!(update,FrameUpdate::ChildFinished{child,..} if child==id.frame()) {
            delivered = true;
            drop(update);
            break;
        }
    }
    assert!(delivered);
    actor.reconcile();
    assert!(!actor.calls.contains_key(&id));
    let admission = budget.reserve(RESULT_BYTES).unwrap();
    drop(admission);
    drop(actor);
    drop(runtime);
    assert_eq!(capacity.available_frames(), 8);
}
#[tokio::test]
async fn test_should_charge_queued_business_payload_even_after_scheduler_and_other_lease_owners_drop()
 {
    let (graph, id, _, _, _) = graph(RESULT_BYTES + 4096);
    let budget = graph.task_data_budget();
    let lease = Arc::new(DispatchLease {
        _permit: None,
        _frame: graph.lease(id.frame()),
        _data: Some(budget.reserve(RESULT_BYTES).unwrap()),
    });
    let (sender, mut receiver) = mpsc::channel(1);
    sender
        .send(Message::Completed(
            id,
            Completion {
                result: Ok(Payload::from_value(&json!("x".repeat(120_000))).unwrap()),
                _lease: Some(lease),
            },
        ))
        .await
        .unwrap();
    drop(graph);
    assert!(budget.reserve(RESULT_BYTES).is_err());
    drop(receiver.recv().await.unwrap());
    let _admitted = budget.reserve(RESULT_BYTES).unwrap();
}
