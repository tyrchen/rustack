//! Explicit-clock traversal and acknowledged resource-retirement regressions.

use std::{
    error::Error,
    sync::Arc,
    time::{Duration, Instant},
};

use chrono::Utc;
use proptest::prelude::*;
use rstest::rstest;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    FailureOrigin, FrameCapacity, FrameError, FrameLimits, FrameScheduler, FrameUpdate, ROOT,
};
use crate::{
    AslCompiler, AslError, ClockSample, DriverStep, EvaluationContext, ExecutionDriver,
    ExecutionLimits, ExecutionOutcome, ExecutionSource, FrameCallId, Payload, StateSupport,
    SystemUuidSource, TaskDispatch, TaskFailure, TaskSupport, TenantScope, UuidSource,
};

type TestResult = Result<(), Box<dyn Error>>;
type Call = (FrameCallId, TaskDispatch);
fn now() -> ClockSample {
    ClockSample::new(Instant::now(), Utc::now())
}
fn after(start: ClockSample, millis: u32) -> ClockSample {
    ClockSample::new(
        start.monotonic + Duration::from_millis(u64::from(millis)),
        start.wall + chrono::Duration::milliseconds(i64::from(millis)),
    )
}
fn definition(state: Value) -> Value {
    let mut result = json!({"StartAt":"Run","States":{}});
    result["States"]["Run"] = state;
    result
}
fn succeed() -> Value {
    definition(json!({"Type":"Succeed"}))
}
fn task() -> Value {
    definition(
        json!({"Type":"Task","Resource":"arn:aws:lambda:us-east-1:000000000000:function:worker","End":true}),
    )
}
fn parallel(branches: Vec<Value>) -> Value {
    let mut state = json!({"Type":"Parallel","End":true});
    state["Branches"] = Value::Array(branches);
    definition(state)
}
fn map(concurrency: u8) -> Value {
    definition(
        json!({"Type":"Map","ItemProcessor":task(),"MaxConcurrency":concurrency,"ItemSelector":{"index.$":"$$.Map.Item.Index","value.$":"$$.Map.Item.Value"},"End":true}),
    )
}
fn driver(
    value: &Value,
    input: &Value,
    start: ClockSample,
) -> Result<ExecutionDriver, Box<dyn Error>> {
    let compiled = AslCompiler::new(TenantScope::default(), TaskSupport::Lambda)
        .with_state_support(StateSupport::Structured)
        .compile(&serde_json::to_vec(value)?)?;
    Ok(ExecutionDriver::for_structured_owner(
        compiled.machine().clone(),
        Payload::from_value(input)?,
        EvaluationContext::from_value(&json!({"Execution":{"Id":"root"}}))?,
        start,
        ExecutionLimits::new(Duration::from_secs(60), 100_000)?,
    )?)
}
fn owner(
    value: &Value,
    input: &Value,
    start: ClockSample,
) -> Result<FrameScheduler, Box<dyn Error>> {
    Ok(FrameScheduler::new(
        driver(value, input, start)?,
        Arc::new(FrameCapacity::default()),
        FrameLimits::default(),
    )?)
}
fn pump(
    tree: &mut FrameScheduler,
    start: ClockSample,
    retire: bool,
) -> Result<Vec<Call>, Box<dyn Error>> {
    let mut calls = Vec::new();
    let mut source = SystemUuidSource::default();
    for _ in 0..50_000 {
        match tree.advance(start, &mut source) {
            FrameUpdate::Transition(t) => {
                if let DriverStep::Dispatch(dispatch) = t.step() {
                    calls.push((t.call().ok_or("missing call identity")?, dispatch.clone()));
                }
            }
            FrameUpdate::Cancel(call) => {
                if retire {
                    tree.retired(call)?;
                }
            }
            FrameUpdate::Park { .. } | FrameUpdate::Settled(_) => return Ok(calls),
            _ => {}
        }
    }
    Err("scheduler did not park/settle within bounded fixture steps".into())
}
fn complete(
    tree: &mut FrameScheduler,
    call: FrameCallId,
    value: &Value,
    start: ClockSample,
) -> TestResult {
    assert!(
        tree.complete(
            call,
            Ok(Payload::from_value(value)?),
            start,
            &mut SystemUuidSource::default()
        )?
        .is_some()
    );
    Ok(())
}
fn output(tree: &FrameScheduler) -> Result<&Value, Box<dyn Error>> {
    let Some(result) = tree.outcome() else {
        return Err("missing root outcome".into());
    };
    let ExecutionOutcome::Succeeded(value) = result.as_ref() else {
        return Err("expected success".into());
    };
    Ok(value.value())
}
fn error(tree: &FrameScheduler) -> Result<Option<&str>, Box<dyn Error>> {
    let Some(result) = tree.outcome() else {
        return Err("missing root outcome".into());
    };
    let ExecutionOutcome::Failed(failure) = result.as_ref() else {
        return Err("expected failure".into());
    };
    Ok(failure.error())
}

#[test]
fn test_should_run_nested_map_parallel_and_order_outputs_independently_of_completion() -> TestResult
{
    let start = now();
    let mut inner = map(2);
    inner["States"]["Run"]["ItemProcessor"] = parallel(vec![
        definition(json!({"Type":"Succeed","InputPath":"$.value"})),
        definition(json!({"Type":"Succeed","InputPath":"$.value"})),
    ]);
    let mut tree = owner(&inner, &json!([1, 2, 3]), start)?;
    assert!(pump(&mut tree, start, true)?.is_empty());
    assert_eq!(output(&tree)?, &json!([[1, 1], [2, 2], [3, 3]]));
    assert!(tree.cleanup_ready());
    assert_eq!(tree.frame_count(), 0);
    Ok(())
}

#[test]
fn test_should_keep_successful_calls_owned_while_admitting_later_iterations() -> TestResult {
    let start = now();
    let capacity = Arc::new(FrameCapacity::new(8, 1024 * 1024)?);
    let mut tree = FrameScheduler::new(
        driver(&map(2), &json!([5, 6, 7]), start)?,
        capacity.clone(),
        FrameLimits::default(),
    )?;
    let initial = pump(&mut tree, start, false)?;
    assert_eq!(initial.len(), 2);
    assert_eq!(
        initial
            .first()
            .ok_or("missing first")?
            .1
            .parameters()
            .value()
            .get("index"),
        Some(&json!(0))
    );
    let first = initial.first().ok_or("missing first")?.0;
    let second = initial.get(1).ok_or("missing second")?.0;
    assert_eq!(first.attempt(), second.attempt());
    assert_ne!(first.frame(), second.frame());
    complete(&mut tree, second, &json!(60), start)?;
    let later = pump(&mut tree, start, false)?;
    assert_eq!(later.len(), 1);
    assert_eq!(
        later
            .first()
            .ok_or("missing third")?
            .1
            .parameters()
            .value()
            .get("index"),
        Some(&json!(2))
    );
    let third = later.first().ok_or("missing third")?.0;
    complete(&mut tree, third, &json!(70), start)?;
    complete(&mut tree, first, &json!(50), start)?;
    pump(&mut tree, start, false)?;
    assert_eq!(output(&tree)?, &json!([50, 60, 70]));
    assert!(!tree.cleanup_ready());
    assert_eq!(tree.frame_count(), 4);
    for call in [first, second, third] {
        assert!(tree.retired(call)?);
        assert!(!tree.retired(call)?);
    }
    assert!(tree.cleanup_ready());
    assert_eq!(capacity.available_frames(), 8);
    assert!(
        tree.complete(
            second,
            Ok(Payload::parse(b"99")?),
            start,
            &mut SystemUuidSource::default()
        )?
        .is_none()
    );
    Ok(())
}

#[test]
fn test_should_wait_for_failed_round_sibling_retirement_before_retrying_fresh_frames() -> TestResult
{
    let start = now();
    let mut value = parallel(vec![task(), task()]);
    value["States"]["Run"]["Retry"] =
        json!([{"ErrorEquals":["Oops"],"IntervalSeconds":1,"MaxAttempts":1}]);
    let mut tree = owner(&value, &json!({}), start)?;
    let old = pump(&mut tree, start, false)?;
    assert_eq!(old.len(), 2);
    let first = old.first().ok_or("first")?.0;
    let second = old.get(1).ok_or("second")?.0;
    let _ = tree.complete(
        first,
        Err(TaskFailure::new("Oops", None)?),
        start,
        &mut SystemUuidSource::default(),
    )?;
    assert!(pump(&mut tree, start, false)?.is_empty());
    assert!(tree.retired(first)?);
    assert!(pump(&mut tree, after(start, 500), false)?.is_empty());
    assert!(tree.outcome().is_none());
    assert_eq!(tree.driver(ROOT).ok_or("root")?.retry_count(), 0);
    assert!(tree.retired(second)?);
    assert!(pump(&mut tree, after(start, 500), false)?.is_empty());
    assert_eq!(tree.driver(ROOT).ok_or("root")?.retry_count(), 1);
    let fresh = pump(&mut tree, after(start, 1500), false)?;
    assert_eq!(fresh.len(), 2);
    assert!(
        fresh
            .iter()
            .all(|(call, _)| call.frame() != first.frame() && call.frame() != second.frame())
    );
    assert!(
        tree.complete(
            second,
            Ok(Payload::parse(b"999")?),
            after(start, 1500),
            &mut SystemUuidSource::default()
        )?
        .is_none()
    );
    for (call, _) in &fresh {
        complete(&mut tree, *call, &json!(2), after(start, 1500))?;
    }
    pump(&mut tree, after(start, 1500), true)?;
    assert_eq!(output(&tree)?, &json!([2, 2]));
    assert!(tree.cleanup_ready());
    Ok(())
}

#[test]
fn test_should_reject_active_retirement_and_invalidate_every_nested_call_on_stop() -> TestResult {
    let start = now();
    let value = parallel(vec![map(2), map(2)]);
    let mut tree = owner(&value, &json!([1, 2]), start)?;
    let calls = pump(&mut tree, start, false)?;
    assert_eq!(calls.len(), 4);
    assert!(matches!(
        tree.retired(calls.first().ok_or("first")?.0),
        Err(FrameError::Event)
    ));
    let update = tree.stop(TaskFailure::new("Stopped", Some("requested"))?, start);
    let mut cancels = Vec::new();
    if let FrameUpdate::Cancel(call) = update {
        cancels.push(call);
    }
    for _ in 0..100 {
        match tree.advance(start, &mut SystemUuidSource::default()) {
            FrameUpdate::Cancel(call) => cancels.push(call),
            FrameUpdate::Settled(_) => break,
            _ => {}
        }
    }
    assert_eq!(cancels.len(), 4);
    assert!(!tree.cleanup_ready());
    for (call, _) in calls {
        assert!(
            tree.complete(
                call,
                Ok(Payload::parse(b"1")?),
                start,
                &mut SystemUuidSource::default()
            )?
            .is_none()
        );
        assert!(tree.retired(call)?);
    }
    assert!(tree.cleanup_ready());
    assert!(matches!(
        tree.outcome().map(AsRef::as_ref),
        Some(ExecutionOutcome::Aborted(_))
    ));
    Ok(())
}

#[test]
fn test_should_pin_first_observed_failure_and_emit_cancelled_child_notifications_before_recovery()
-> TestResult {
    let start = now();
    let mut value = parallel(vec![task(), task()]);
    value["States"]["Run"]["Catch"] = json!([{"ErrorEquals":["States.ALL"],"Next":"Handled"}]);
    value["States"]["Handled"] = json!({"Type":"Succeed"});
    let mut tree = owner(&value, &json!({}), start)?;
    let calls = pump(&mut tree, start, false)?;
    let first = calls.first().ok_or("first")?.0;
    let second = calls.get(1).ok_or("second")?.0;
    let _ = tree.complete(
        first,
        Err(TaskFailure::new("First", None)?),
        start,
        &mut SystemUuidSource::default(),
    )?;
    assert!(
        tree.complete(
            second,
            Err(TaskFailure::new("Second", None)?),
            start,
            &mut SystemUuidSource::default()
        )?
        .is_none()
    );
    let mut cancelled = false;
    for _ in 0..100 {
        match tree.advance(start, &mut SystemUuidSource::default()) {
            FrameUpdate::Cancel(call) => {
                tree.retired(call)?;
            }
            FrameUpdate::ChildFinished { child, outcome, .. } if child == second.frame() => {
                assert!(matches!(outcome.as_ref(), ExecutionOutcome::Aborted(_)));
                cancelled = true;
            }
            FrameUpdate::Settled(_) => break,
            _ => {}
        }
    }
    assert!(cancelled);
    assert_eq!(output(&tree)?.get("Error"), Some(&json!("First")));
    assert!(tree.cleanup_ready());
    Ok(())
}

#[test]
fn test_should_inherit_exact_subsecond_deadline_and_prioritize_root_expiry() -> TestResult {
    let start = now();
    let mut value = parallel(vec![task()]);
    value["TimeoutSeconds"] = json!(1);
    let mut tree = owner(&value, &json!({}), start)?;
    let calls = pump(&mut tree, after(start, 900), false)?;
    let (call, dispatch) = calls.first().ok_or("missing call")?;
    assert_eq!(dispatch.deadline(), after(start, 1000).monotonic);
    assert_eq!(
        tree.driver(call.frame()).ok_or("child")?.deadline(),
        dispatch.deadline()
    );
    assert!(
        tree.complete(
            *call,
            Ok(Payload::parse(b"1")?),
            after(start, 1000),
            &mut SystemUuidSource::default()
        )?
        .is_none()
    );
    assert!(matches!(
        tree.outcome().map(AsRef::as_ref),
        Some(ExecutionOutcome::TimedOut)
    ));
    pump(&mut tree, after(start, 1000), true)?;
    assert!(tree.cleanup_ready());
    Ok(())
}

#[test]
fn test_should_fail_join_only_frame_exhaustion_without_business_retry_or_deadlock() -> TestResult {
    let start = now();
    let value = parallel(vec![parallel(vec![succeed()])]);
    let capacity = Arc::new(FrameCapacity::default());
    let mut tree = FrameScheduler::new(
        driver(&value, &json!({}), start)?,
        capacity.clone(),
        FrameLimits::new(2, 4096)?,
    )?;
    pump(&mut tree, start, true)?;
    assert_eq!(error(&tree)?, Some("Rustack.FrameLimitExceeded"));
    assert!(tree.cleanup_ready());
    assert_eq!(capacity.available_frames(), 4096);
    Ok(())
}

#[test]
fn test_should_park_capacity_pressure_while_an_existing_task_can_free_slots() -> TestResult {
    let start = now();
    let capacity = Arc::new(FrameCapacity::new(3, 1024 * 1024)?);
    let mut tree = FrameScheduler::new(
        driver(&map(2), &json!([1, 2, 3]), start)?,
        capacity,
        FrameLimits::new(3, 1024 * 1024)?,
    )?;
    let calls = pump(&mut tree, start, false)?;
    assert_eq!(calls.len(), 2);
    let first = calls.first().ok_or("first")?.0;
    complete(&mut tree, first, &json!(10), start)?;
    assert!(pump(&mut tree, start, false)?.is_empty());
    assert!(tree.outcome().is_none());
    tree.retired(first)?;
    assert_eq!(pump(&mut tree, start, false)?.len(), 1);
    Ok(())
}

#[rstest]
#[case(true)]
#[case(false)]
fn test_should_charge_global_and_local_result_slots_before_retention(
    #[case] local: bool,
) -> TestResult {
    let start = now();
    let global = if local { 1024 * 1024 } else { 4096 };
    let local_bytes = if local { 4096 } else { 1024 * 1024 };
    let mut tree = FrameScheduler::new(
        driver(&parallel(vec![succeed()]), &json!("x".repeat(3000)), start)?,
        Arc::new(FrameCapacity::new(8, global)?),
        FrameLimits::new(8, local_bytes)?,
    )?;
    pump(&mut tree, start, true)?;
    assert_eq!(error(&tree)?, Some("Rustack.ResultLimitExceeded"));
    assert!(tree.cleanup_ready());
    Ok(())
}

#[test]
fn test_should_recover_aggregate_data_limit_only_after_successful_sibling_retirement() -> TestResult
{
    let start = now();
    let mut value = parallel(vec![task(), task()]);
    value["States"]["Run"]["Catch"] =
        json!([{"ErrorEquals":["States.DataLimitExceeded"],"Next":"Handled"}]);
    value["States"]["Handled"] = json!({"Type":"Pass","Result":"recovered","End":true});
    let mut tree = owner(&value, &json!({}), start)?;
    let calls = pump(&mut tree, start, false)?;
    for (call, _) in &calls {
        complete(&mut tree, *call, &json!("x".repeat(131_071)), start)?;
    }
    pump(&mut tree, start, false)?;
    assert!(tree.outcome().is_none());
    for (call, _) in calls {
        tree.retired(call)?;
    }
    pump(&mut tree, start, true)?;
    assert_eq!(output(&tree)?, &json!("recovered"));
    assert!(tree.cleanup_ready());
    Ok(())
}

#[test]
fn test_should_keep_terminal_business_origin_after_a_handled_evaluation_diagnostic() -> TestResult {
    let start = now();
    let branch = json!({"StartAt":"Call","States":{"Call":{"Type":"Task","Resource":"arn:aws:lambda:us-east-1:000000000000:function:worker","Parameters":{"x":"x".repeat(262_144)},"Catch":[{"ErrorEquals":["States.DataLimitExceeded"],"Next":"Fail"}],"End":true},"Fail":{"Type":"Fail","Error":"BusinessFailure"}}});
    let mut value = parallel(vec![branch]);
    value["States"]["Run"]["Catch"] = json!([{"ErrorEquals":["BusinessFailure"],"Next":"Handled"}]);
    value["States"]["Handled"] = json!({"Type":"Succeed"});
    let mut tree = owner(&value, &json!({}), start)?;
    assert!(pump(&mut tree, start, true)?.is_empty());
    assert_eq!(output(&tree)?.get("Error"), Some(&json!("BusinessFailure")));
    Ok(())
}

#[derive(Debug)]
struct BrokenSource;
impl UuidSource for BrokenSource {
    fn next_uuid(&mut self) -> Result<Uuid, AslError> {
        Err(AslError::InvalidStateId)
    }
}
impl ExecutionSource for BrokenSource {
    fn random_u64(&mut self) -> Result<u64, AslError> {
        Err(AslError::InvalidStateId)
    }
}
#[test]
fn test_should_preserve_typed_child_internal_failure_across_nested_rounds() -> TestResult {
    let start = now();
    let branch =
        definition(json!({"Type":"Pass","Parameters":{"uuid.$":"States.UUID()"},"End":true}));
    let mut value = parallel(vec![parallel(vec![branch])]);
    value["States"]["Run"]["Catch"] = json!([{"ErrorEquals":["States.ALL"],"Next":"Handled"}]);
    value["States"]["Handled"] = json!({"Type":"Succeed"});
    let mut tree = owner(&value, &json!({}), start)?;
    for _ in 0..100 {
        if matches!(
            tree.advance(start, &mut BrokenSource),
            FrameUpdate::Settled(_)
        ) {
            break;
        }
    }
    assert_eq!(error(&tree)?, Some("Rustack.InternalError"));
    assert!(tree.cleanup_ready());
    Ok(())
}

#[test]
fn test_should_share_root_transition_budget_across_all_frames() -> TestResult {
    let start = now();
    let mut root = driver(&parallel(vec![succeed(), succeed()]), &json!({}), start)?;
    root.remaining = 2;
    let mut tree = FrameScheduler::new(
        root,
        Arc::new(FrameCapacity::default()),
        FrameLimits::default(),
    )?;
    pump(&mut tree, start, true)?;
    assert_eq!(error(&tree)?, Some("Rustack.TransitionLimitExceeded"));
    assert!(tree.cleanup_ready());
    Ok(())
}

#[tokio::test]
async fn test_should_wake_capacity_observer_after_releasing_the_actual_slot() -> TestResult {
    let start = now();
    let capacity = Arc::new(FrameCapacity::new(1, 4096)?);
    let mut tree = FrameScheduler::new(
        driver(&succeed(), &json!({}), start)?,
        capacity.clone(),
        FrameLimits::default(),
    )?;
    assert_eq!(capacity.available_frames(), 0);
    let wait = capacity.wait_for_capacity(tokio::time::Instant::now() + Duration::from_secs(1));
    let release = async {
        tokio::task::yield_now().await;
        pump(&mut tree, start, true)
    };
    let (observed, released) = tokio::join!(wait, release);
    observed?;
    released?;
    assert_eq!(capacity.available_frames(), 1);
    assert!(matches!(
        capacity
            .wait_for_capacity(tokio::time::Instant::now())
            .await,
        Err(FrameError::Deadline)
    ));
    Ok(())
}

#[test]
fn test_should_reject_root_admission_before_ownership_and_restore_budget_on_clean_finish()
-> TestResult {
    let start = now();
    let capacity = Arc::new(FrameCapacity::new(1, 4096)?);
    let mut first = FrameScheduler::new(
        driver(&succeed(), &json!({}), start)?,
        capacity.clone(),
        FrameLimits::default(),
    )?;
    assert!(matches!(
        FrameScheduler::new(
            driver(&succeed(), &json!({}), start)?,
            capacity.clone(),
            FrameLimits::default()
        ),
        Err(FrameError::Capacity)
    ));
    pump(&mut first, start, true)?;
    let mut second = FrameScheduler::new(
        driver(&succeed(), &json!({}), start)?,
        capacity,
        FrameLimits::default(),
    )?;
    pump(&mut second, start, true)?;
    assert!(second.cleanup_ready());
    Ok(())
}

proptest! {
 #![proptest_config(ProptestConfig::with_cases(64))]
 #[test]
 fn test_should_preserve_results_for_bounded_nested_map_parallel_trees(width in 1_usize..4,depth in 1_usize..5) {
  let start=now();let mut machine=succeed();let mut input=json!(7);let mut expected=json!(7);
  for level in 0..depth {
   if level%2==0 {machine=definition(json!({"Type":"Map","ItemProcessor":machine,"End":true}));input=json!(vec![input;width]);}
   else {machine=parallel(vec![machine;width]);}
   expected=json!(vec![expected;width]);
  }
  let mut tree=owner(&machine,&input,start).map_err(|e|TestCaseError::fail(e.to_string()))?;
  pump(&mut tree,start,true).map_err(|e|TestCaseError::fail(e.to_string()))?;
  prop_assert_eq!(output(&tree).map_err(|e|TestCaseError::fail(e.to_string()))?,&expected);prop_assert!(tree.cleanup_ready());
 }
}

#[test]
fn test_should_hold_frame_charges_until_queued_success_notification_is_emitted() -> TestResult {
    let start = now();
    let capacity = Arc::new(FrameCapacity::new(3, 1024 * 1024)?);
    let mut tree = FrameScheduler::new(
        driver(&parallel(vec![task(), task()]), &json!({}), start)?,
        capacity.clone(),
        FrameLimits::default(),
    )?;
    let calls = pump(&mut tree, start, false)?;
    let success = calls.first().ok_or("success")?.0;
    let failed = calls.get(1).ok_or("failure")?.0;
    complete(&mut tree, success, &json!("held".repeat(1000)), start)?;
    let _ = tree.complete(
        failed,
        Err(TaskFailure::new("Oops", None)?),
        start,
        &mut SystemUuidSource::default(),
    )?;
    tree.retired(success)?;
    tree.retired(failed)?;
    assert!(!tree.pending.is_empty());
    assert_eq!(capacity.available_frames(), 0);
    assert_eq!(tree.frame_count(), 3);
    let update = tree.advance(start, &mut SystemUuidSource::default());
    let FrameUpdate::ChildFinished { child, outcome, .. } = update else {
        return Err("expected retained notification".into());
    };
    assert_eq!(child, success.frame());
    assert!(matches!(outcome.as_ref(), ExecutionOutcome::Succeeded(_)));
    assert_eq!(capacity.available_frames(), 1);
    pump(&mut tree, start, true)?;
    assert_eq!(error(&tree)?, Some("Oops"));
    assert!(tree.cleanup_ready());
    assert_eq!(capacity.available_frames(), 3);
    Ok(())
}

#[derive(Debug)]
struct WorkSource;
impl UuidSource for WorkSource {
    fn next_uuid(&mut self) -> Result<Uuid, AslError> {
        Err(AslError::WorkLimit)
    }
}
impl ExecutionSource for WorkSource {
    fn random_u64(&mut self) -> Result<u64, AslError> {
        Err(AslError::WorkLimit)
    }
}
#[test]
fn test_should_preserve_recovery_failure_classification_cause_and_source_across_nested_rounds()
-> TestResult {
    let start = now();
    let mut branch = task();
    branch["States"]["Run"]["Retry"] = json!([{"ErrorEquals":["Oops"],"JitterStrategy":"FULL"}]);
    let mut tree = owner(&parallel(vec![parallel(vec![branch])]), &json!({}), start)?;
    let calls = pump(&mut tree, start, false)?;
    let call = calls.first().ok_or("call")?.0;
    let _ = tree.complete(
        call,
        Err(TaskFailure::new("Oops", None)?),
        start,
        &mut WorkSource,
    )?;
    pump(&mut tree, start, true)?;
    assert_eq!(error(&tree)?, Some("Rustack.InternalError"));
    let Some(outcome) = tree.outcome() else {
        return Err("outcome".into());
    };
    let ExecutionOutcome::Failed(failure) = outcome.as_ref() else {
        return Err("failure".into());
    };
    assert_eq!(failure.cause(), Some("Task recovery failed"));
    assert!(
        matches!(&failure.origin,FailureOrigin::Evaluation(source) if matches!(source.as_ref(),AslError::WorkLimit))
    );
    assert!(tree.cleanup_ready());
    Ok(())
}

#[test]
fn test_should_wake_wait_frames_and_report_task_timeout_without_releasing_its_call() -> TestResult {
    let start = now();
    let mut branch = task();
    branch["States"]["Run"]["TimeoutSeconds"] = json!(1);
    branch["States"]["Run"]["Catch"] = json!([{"ErrorEquals":["States.Timeout"],"Next":"Done"}]);
    branch["States"]["Done"] = json!({"Type":"Pass","Result":"caught","End":true});
    let mut tree = owner(&parallel(vec![branch]), &json!({}), start)?;
    let calls = pump(&mut tree, start, false)?;
    let call = calls.first().ok_or("call")?.0;
    let later = after(start, 1000);
    let mut timed_out = false;
    for _ in 0..100 {
        if let FrameUpdate::Transition(t) = tree.advance(later, &mut SystemUuidSource::default()) {
            if t.timed_out_call() == Some(call) {
                timed_out = true;
                break;
            }
        }
    }
    assert!(timed_out);
    pump(&mut tree, later, false)?;
    assert_eq!(output(&tree)?, &json!(["caught"]));
    assert!(!tree.cleanup_ready());
    tree.retired(call)?;
    assert!(tree.cleanup_ready());
    let mut waiting = owner(
        &parallel(vec![definition(
            json!({"Type":"Wait","Seconds":1,"End":true}),
        )]),
        &json!(7),
        start,
    )?;
    assert!(pump(&mut waiting, start, true)?.is_empty());
    assert!(waiting.outcome().is_none());
    pump(&mut waiting, later, true)?;
    assert_eq!(output(&waiting)?, &json!([7]));
    Ok(())
}

#[test]
fn test_should_bound_unretired_task_attempts_and_retain_all_prior_calls_on_exhaustion() -> TestResult
{
    let start = now();
    let value = json!({"StartAt":"Run","States":{"Run":{"Type":"Task","Resource":"arn:aws:lambda:us-east-1:000000000000:function:worker","Next":"Choice"},"Choice":{"Type":"Choice","Choices":[{"Variable":"$.again","BooleanEquals":true,"Next":"Run"}],"Default":"Done"},"Done":{"Type":"Succeed"}}});
    let mut tree = owner(&value, &json!({}), start)?;
    let mut owned = Vec::new();
    for _ in 0..1024 {
        let calls = pump(&mut tree, start, false)?;
        assert_eq!(calls.len(), 1);
        let call = calls.first().ok_or("call")?.0;
        complete(&mut tree, call, &json!({"again":true}), start)?;
        owned.push(call);
    }
    assert!(pump(&mut tree, start, false)?.is_empty());
    assert_eq!(error(&tree)?, Some("Rustack.TaskOwnershipLimitExceeded"));
    assert!(!tree.cleanup_ready());
    for call in owned {
        assert!(tree.retired(call)?);
    }
    assert!(tree.cleanup_ready());
    Ok(())
}

#[test]
fn test_should_reject_invalid_frame_configuration_and_clock_regression() -> TestResult {
    for (frames, bytes) in [
        (0, 4096),
        (513, 4096),
        (1, 4095),
        (1, 128 * 1024 * 1024 + 1),
    ] {
        assert!(matches!(
            FrameLimits::new(frames, bytes),
            Err(FrameError::Invalid)
        ));
    }
    for (frames, bytes) in [
        (0, 4096),
        (4097, 4096),
        (1, 4095),
        (1, 1024 * 1024 * 1024 + 1),
    ] {
        assert!(matches!(
            FrameCapacity::new(frames, bytes),
            Err(FrameError::Invalid)
        ));
    }
    let start = now();
    let mut tree = owner(&parallel(vec![task()]), &json!({}), start)?;
    pump(&mut tree, after(start, 10), false)?;
    pump(&mut tree, start, true)?;
    assert_eq!(error(&tree)?, Some("Rustack.InternalError"));
    assert!(tree.cleanup_ready());
    Ok(())
}
