//! Explicit-clock structured frame semantics; no asynchronous owner is faked.

use std::{
    error::Error,
    time::{Duration, Instant},
};

use chrono::Utc;
use proptest::prelude::*;
use rstest::rstest;
use serde_json::{Value, json};
use uuid::{Builder, Uuid};

use crate::{
    AslCompiler, AslError, ClockSample, DriverStep, EvaluationContext, ExecutionDriver,
    ExecutionError, ExecutionFailure, ExecutionLimits, ExecutionOutcome, ExecutionSource,
    ExecutionSupervisor, FanoutFailure, FanoutPlan, HistoryLimits, MAX_PAYLOAD_BYTES, Payload,
    RuntimeLimits, StateSupport, TaskFailure, TaskSupport, TenantScope, UuidSource,
};

type TestResult = Result<(), Box<dyn Error>>;
#[derive(Default)]
struct Source {
    uuids: u64,
    draws: usize,
}
impl UuidSource for Source {
    fn next_uuid(&mut self) -> Result<Uuid, AslError> {
        self.uuids += 1;
        let mut bytes = [0_u8; 16];
        bytes
            .get_mut(8..)
            .ok_or(AslError::InvalidExecution)?
            .copy_from_slice(&self.uuids.to_be_bytes());
        Ok(Builder::from_random_bytes(bytes).into_uuid())
    }
}
impl ExecutionSource for Source {
    fn random_u64(&mut self) -> Result<u64, AslError> {
        self.draws += 1;
        Ok(u64::MAX)
    }
}
fn now() -> ClockSample {
    ClockSample::new(Instant::now(), Utc::now())
}
fn after(start: ClockSample, seconds: u32) -> ClockSample {
    ClockSample::new(
        start.monotonic + Duration::from_secs(u64::from(seconds)),
        start.wall + chrono::Duration::seconds(i64::from(seconds)),
    )
}
fn definition(state: Value) -> Value {
    let mut definition = json!({"StartAt":"Run","States":{}});
    definition["States"]["Run"] = state;
    definition
}
fn leaf() -> Value {
    definition(json!({"Type":"Succeed"}))
}
fn map() -> Value {
    definition(json!({"Type":"Map","ItemProcessor":leaf(),"End":true}))
}
fn parallel(count: usize) -> Value {
    definition(json!({"Type":"Parallel","Branches":vec![leaf();count],"End":true}))
}
fn driver(
    definition: &Value,
    input: &Value,
    base: &Value,
    start: ClockSample,
) -> Result<ExecutionDriver, Box<dyn Error>> {
    let compiled = AslCompiler::new(TenantScope::default(), TaskSupport::Lambda)
        .with_state_support(StateSupport::Structured)
        .compile(&serde_json::to_vec(definition)?)?;
    Ok(ExecutionDriver::for_structured_owner(
        compiled.machine().clone(),
        Payload::from_value(input)?,
        EvaluationContext::from_value(base)?,
        start,
        ExecutionLimits::new(Duration::from_secs(60), 1000)?,
    )?)
}
fn plan(
    driver: &mut ExecutionDriver,
    now: ClockSample,
    source: &mut Source,
) -> Result<FanoutPlan, Box<dyn Error>> {
    let DriverStep::FanOut(plan) = driver.advance(now, source).step else {
        return Err("expected FanOut".into());
    };
    Ok(plan)
}
fn output(driver: &ExecutionDriver) -> Result<&Value, Box<dyn Error>> {
    let Some(outcome) = driver.outcome() else {
        return Err("missing outcome".into());
    };
    let ExecutionOutcome::Succeeded(payload) = outcome.as_ref() else {
        return Err("expected success".into());
    };
    Ok(payload.value())
}
fn failure(driver: &ExecutionDriver) -> Result<&ExecutionFailure, Box<dyn Error>> {
    let Some(outcome) = driver.outcome() else {
        return Err("missing outcome".into());
    };
    let ExecutionOutcome::Failed(failure) = outcome.as_ref() else {
        return Err("expected failure".into());
    };
    Ok(failure)
}

#[test]
fn test_should_prepare_parallel_inputs_and_merge_ordered_results_into_original_input() -> TestResult
{
    let start = now();
    let mut source = Source::default();
    let mut definition = parallel(3);
    definition["States"]["Run"]["InputPath"] = json!("$.selected");
    definition["States"]["Run"]["Parameters"] = json!({"n.$":"States.MathAdd($.n,1)"});
    definition["States"]["Run"]["ResultSelector"] = json!({"first.$":"$[0]","last.$":"$[2]"});
    definition["States"]["Run"]["ResultPath"] = json!("$.joined");
    let mut driver = driver(
        &definition,
        &json!({"selected":{"n":4},"keep":true}),
        &json!({"Execution":{"Id":"root"}}),
        start,
    )?;
    let plan = plan(&mut driver, start, &mut source)?;
    assert_eq!(plan.item_count(), 3);
    assert_eq!(plan.max_concurrency().get(), 3);
    assert_eq!(plan.deadline(), driver.deadline());
    for index in 0..3 {
        let child = plan.branch(index, &mut source)?;
        assert_eq!(child.input().value(), &json!({"n":5}));
        assert_eq!(child.context().value(), &json!({"Execution":{"Id":"root"}}));
        assert_eq!(child.machine().state_name(child.machine().start())?, "Run");
    }
    let joined = plan.collect(&[
        (2, Payload::parse(b"30")?),
        (0, Payload::parse(b"10")?),
        (1, Payload::parse(b"20")?),
    ])?;
    let _ = driver.complete_fanout(plan.attempt(), Ok(joined), start, &mut source);
    assert_eq!(
        output(&driver)?,
        &json!({"selected":{"n":4},"keep":true,"joined":{"first":10,"last":30}})
    );
    Ok(())
}

#[rstest]
#[case("ItemProcessor", "ItemSelector")]
#[case("Iterator", "Parameters")]
#[case("ItemProcessor", "Parameters")]
#[case("Iterator", "ItemSelector")]
fn test_should_select_items_lazily_with_enclosing_state_context_and_scoped_metadata(
    #[case] processor_key: &str,
    #[case] selector_key: &str,
) -> TestResult {
    let start = now();
    let mut source = Source::default();
    let mut state = json!({"Type":"Map","InputPath":"$.batch","ItemsPath":"$.items","MaxConcurrencyPath":"$.limit","End":true});
    state[processor_key] = leaf();
    state[selector_key] = json!({"item.$":"$$.Map.Item.Value","index.$":"$$.Map.Item.Index","source.$":"$$.Map.Item.Source","label.$":"$.label","state.$":"$$.State.Name","retry.$":"$$.State.RetryCount","uuid.$":"States.UUID()"});
    let mut driver = driver(
        &definition(state),
        &json!({"batch":{"items":[null,7],"limit":1,"label":"selected"},"limit":41}),
        &json!({"Execution":{"Id":"root"},"Map":{"Item":{"Index":999}}}),
        start,
    )?;
    let plan = plan(&mut driver, start, &mut source)?;
    assert_eq!(source.uuids, 0);
    assert_eq!(plan.item_count(), 2);
    assert_eq!(plan.max_concurrency().get(), 1);
    let first = plan.branch(0, &mut source)?;
    assert_eq!(source.uuids, 1);
    let second = plan.branch(1, &mut source)?;
    assert_eq!(source.uuids, 2);
    assert_eq!(first.input().value().get("item"), Some(&Value::Null));
    assert_eq!(second.input().value().get("item"), Some(&json!(7)));
    assert_eq!(second.input().value().get("index"), Some(&json!(1)));
    assert_eq!(
        second.input().value().get("source"),
        Some(&json!("STATE_DATA"))
    );
    assert_eq!(
        second.input().value().get("label"),
        Some(&json!("selected"))
    );
    assert_eq!(second.input().value().get("state"), Some(&json!("Run")));
    assert_eq!(second.input().value().get("retry"), Some(&json!(0)));
    assert_ne!(
        first.input().value().get("uuid"),
        second.input().value().get("uuid")
    );
    assert_eq!(first.context().value(), &json!({"Execution":{"Id":"root"}}));
    assert_eq!(second.context().value(), first.context().value());
    Ok(())
}

#[test]
fn test_should_preserve_default_null_item_and_hide_map_namespace_from_processor_states()
-> TestResult {
    let start = now();
    let mut source = Source::default();
    let mut definition = map();
    definition["States"]["Run"]["ItemProcessor"] = self::definition(
        json!({"Type":"Pass","Parameters":{"bad.$":"$$.Map.Item.Index"},"End":true}),
    );
    let mut driver = driver(&definition, &json!([null]), &json!({}), start)?;
    let plan = plan(&mut driver, start, &mut source)?;
    let branch = plan.branch(0, &mut source)?;
    assert!(branch.input().value().is_null());
    let mut child = ExecutionDriver::new(
        branch.machine().clone(),
        branch.input().clone(),
        branch.context().clone(),
        start,
        ExecutionLimits::default(),
    )?;
    child.deadline = plan.deadline();
    let _ = child.advance(start, &mut source);
    assert_eq!(failure(&child)?.error(), Some("States.Runtime"));
    Ok(())
}

#[test]
fn test_should_finish_empty_map_without_child_or_selector_evaluation_and_apply_output_pipeline()
-> TestResult {
    let start = now();
    let mut source = Source::default();
    let mut definition = map();
    definition["States"]["Run"]["ItemsPath"] = json!("$.items");
    definition["States"]["Run"]["ItemSelector"] =
        json!({"uuid.$":"States.UUID()","missing.$":"$.absent"});
    definition["States"]["Run"]["ResultSelector"] = json!({"count.$":"States.ArrayLength($)"});
    definition["States"]["Run"]["ResultPath"] = json!("$.result");
    definition["States"]["Run"]["OutputPath"] = json!("$.result");
    let mut driver = driver(
        &definition,
        &json!({"items":[],"keep":true}),
        &json!({}),
        start,
    )?;
    let step = driver.advance(start, &mut source);
    assert!(matches!(step.step, DriverStep::Settled(_)));
    assert_eq!(source.uuids, 0);
    assert_eq!(output(&driver)?, &json!({"count":0}));
    Ok(())
}

#[rstest]
#[case(json!(-1))]
#[case(json!(1.5))]
#[case(json!(1.0))]
#[case(json!(41))]
#[case(json!(null))]
#[case(json!("1"))]
#[case(json!(true))]
#[case(json!({}))]
fn test_should_reject_invalid_dynamic_concurrency_even_for_empty_map(
    #[case] limit: Value,
) -> TestResult {
    let start = now();
    let mut source = Source::default();
    let mut definition = map();
    definition["States"]["Run"]["ItemsPath"] = json!("$.items");
    definition["States"]["Run"]["MaxConcurrencyPath"] = json!("$.limit");
    let mut driver = driver(
        &definition,
        &json!({"items":[],"limit":limit}),
        &json!({}),
        start,
    )?;
    let _ = driver.advance(start, &mut source);
    assert_eq!(failure(&driver)?.error(), Some("States.Runtime"));
    assert!(matches!(driver.diagnostic(), Some(AslError::InvalidMap)));
    Ok(())
}

#[rstest]
#[case(0, 40)]
#[case(1, 1)]
#[case(40, 40)]
fn test_should_resolve_valid_dynamic_concurrency_from_effective_input(
    #[case] limit: u8,
    #[case] expected: usize,
) -> TestResult {
    let start = now();
    let mut source = Source::default();
    let mut definition = map();
    definition["States"]["Run"]["InputPath"] = json!("$.batch");
    definition["States"]["Run"]["ItemsPath"] = json!("$.items");
    definition["States"]["Run"]["MaxConcurrencyPath"] = json!("$.limit");
    let mut driver = driver(
        &definition,
        &json!({"batch":{"items":[1],"limit":limit},"limit":41}),
        &json!({}),
        start,
    )?;
    assert_eq!(
        plan(&mut driver, start, &mut source)?
            .max_concurrency()
            .get(),
        expected
    );
    Ok(())
}

#[rstest]
#[case(json!(null))]
#[case(json!({}))]
#[case(json!(1))]
fn test_should_reject_nonarray_map_input_without_spawning_descriptors(
    #[case] input: Value,
) -> TestResult {
    let start = now();
    let mut driver = driver(&map(), &input, &json!({}), start)?;
    let _ = driver.advance(start, &mut Source::default());
    assert_eq!(failure(&driver)?.error(), Some("States.Runtime"));
    Ok(())
}

#[test]
fn test_should_enforce_exact_item_limit_and_missing_reference_failure() -> TestResult {
    let start = now();
    let mut source = Source::default();
    let mut exact = driver(&map(), &json!(vec![0; 10_000]), &json!({}), start)?;
    let plan = plan(&mut exact, start, &mut source)?;
    assert_eq!(plan.item_count(), 10_000);
    assert!(matches!(
        plan.branch(10_000, &mut source),
        Err(AslError::InvalidFanoutResult)
    ));
    let mut over = driver(&map(), &json!(vec![0; 10_001]), &json!({}), start)?;
    let _ = over.advance(start, &mut source);
    assert_eq!(failure(&over)?.error(), Some("States.Runtime"));
    let mut definition = map();
    definition["States"]["Run"]["ItemsPath"] = json!("$.missing");
    let mut missing = driver(&definition, &json!({}), &json!({}), start)?;
    let _ = missing.advance(start, &mut source);
    assert!(matches!(missing.diagnostic(), Some(AslError::Missing)));
    Ok(())
}

#[test]
fn test_should_bound_aggregate_before_materializing_and_reject_invalid_index_sets() -> TestResult {
    let start = now();
    let mut source = Source::default();
    let mut driver = driver(&parallel(1), &json!({}), &json!({}), start)?;
    let plan = plan(&mut driver, start, &mut source)?;
    let exact = Payload::from_value(&json!("x".repeat(MAX_PAYLOAD_BYTES - 4)))?;
    assert_eq!(
        plan.collect(&[(0, exact)])?.original_bytes().len(),
        MAX_PAYLOAD_BYTES
    );
    let over = Payload::from_value(&json!("x".repeat(MAX_PAYLOAD_BYTES - 3)))?;
    assert!(matches!(
        plan.collect(&[(0, over)]),
        Err(AslError::DataLimit)
    ));
    assert!(matches!(
        plan.collect(&[]),
        Err(AslError::InvalidFanoutResult)
    ));
    assert!(matches!(
        plan.collect(&[(1, Payload::parse(b"null")?)]),
        Err(AslError::InvalidFanoutResult)
    ));
    let mut driver = self::driver(&parallel(2), &json!({}), &json!({}), start)?;
    let plan = self::plan(&mut driver, start, &mut source)?;
    assert!(matches!(
        plan.collect(&[(0, Payload::parse(b"1")?), (0, Payload::parse(b"2")?)]),
        Err(AslError::InvalidFanoutResult)
    ));
    Ok(())
}

#[test]
fn test_should_retry_whole_map_with_fresh_generation_context_and_unchanged_deadline() -> TestResult
{
    let start = now();
    let mut source = Source::default();
    let mut definition = map();
    definition["States"]["Run"]["ItemsPath"] = json!("$.items");
    definition["States"]["Run"]["ItemSelector"] =
        json!({"retry.$":"$$.State.RetryCount","entered.$":"$$.State.EnteredTime","n.$":"$.n"});
    definition["States"]["Run"]["Retry"] =
        json!([{"ErrorEquals":["Oops"],"IntervalSeconds":1,"MaxAttempts":1}]);
    let raw = json!({"items":[7],"n":9});
    let mut driver = driver(&definition, &raw, &json!({}), start)?;
    let old = plan(&mut driver, start, &mut source)?;
    let old_child = old.branch(0, &mut source)?;
    let deadline = driver.deadline();
    let update = driver.complete_fanout(
        old.attempt(),
        Err(TaskFailure::new("Oops", None)?.into()),
        start,
        &mut source,
    );
    assert!(matches!(update.step,DriverStep::Park(until) if until==after(start,1).monotonic));
    assert_eq!(driver.input().value(), &raw);
    assert_eq!(driver.retry_count(), 1);
    assert!(matches!(
        driver.advance(after(start, 1), &mut source).step,
        DriverStep::Continue
    ));
    let fresh = plan(&mut driver, after(start, 1), &mut source)?;
    let fresh_child = fresh.branch(0, &mut source)?;
    assert_ne!(old.attempt(), fresh.attempt());
    assert_eq!(fresh.deadline(), deadline);
    assert_eq!(fresh_child.input().value().get("retry"), Some(&json!(1)));
    assert_eq!(
        old_child.input().value().get("entered"),
        fresh_child.input().value().get("entered")
    );
    assert_eq!(fresh_child.input().value().get("n"), Some(&json!(9)));
    let _ = driver.complete_fanout(
        old.attempt(),
        Ok(Payload::parse(b"[0]")?),
        after(start, 1),
        &mut source,
    );
    assert!(driver.outcome().is_none());
    let _ = driver.complete(
        fresh.attempt(),
        Ok(Payload::parse(b"[99]")?),
        after(start, 1),
        &mut source,
    );
    assert!(driver.outcome().is_none());
    let _ = driver.complete_fanout(
        fresh.attempt(),
        Ok(Payload::parse(b"[2]")?),
        after(start, 1),
        &mut source,
    );
    assert_eq!(output(&driver)?, &json!([2]));
    let _ = driver.complete_fanout(
        fresh.attempt(),
        Ok(Payload::parse(b"[3]")?),
        after(start, 1),
        &mut source,
    );
    assert_eq!(output(&driver)?, &json!([2]));
    Ok(())
}

#[test]
fn test_should_not_apply_fanout_completion_to_lambda_attempt() -> TestResult {
    let start = now();
    let mut source = Source::default();
    let mut driver = driver(
        &definition(json!({"Type":"Task","Resource":"arn:aws:states:::lambda:invoke","End":true})),
        &json!({}),
        &json!({}),
        start,
    )?;
    let DriverStep::Dispatch(call) = driver.advance(start, &mut source).step else {
        return Err("missing Task effect".into());
    };
    let _ = driver.complete_fanout(
        call.attempt(),
        Ok(Payload::parse(b"[]")?),
        start,
        &mut source,
    );
    assert!(driver.outcome().is_none());
    let _ = driver.complete(
        call.attempt(),
        Ok(Payload::parse(b"7")?),
        start,
        &mut source,
    );
    assert_eq!(output(&driver)?, &json!(7));
    Ok(())
}

#[rstest]
#[case(json!(null))]
#[case(json!([]))]
#[case(json!([1,2]))]
fn test_should_fail_invalid_join_shape_as_internal_owner_error(#[case] value: Value) -> TestResult {
    let start = now();
    let mut source = Source::default();
    let mut driver = driver(&parallel(1), &json!({}), &json!({}), start)?;
    let plan = plan(&mut driver, start, &mut source)?;
    let _ = driver.complete_fanout(
        plan.attempt(),
        Ok(Payload::from_value(&value)?),
        start,
        &mut source,
    );
    assert_eq!(failure(&driver)?.error(), Some("Rustack.InternalError"));
    assert!(matches!(
        driver.diagnostic(),
        Some(AslError::InvalidFanoutResult)
    ));
    Ok(())
}

#[test]
fn test_should_timeout_or_stop_pending_round_once_and_retain_cancellation_obligation() -> TestResult
{
    let start = now();
    let mut source = Source::default();
    let mut definition = parallel(1);
    definition["TimeoutSeconds"] = json!(2);
    let mut expired = driver(&definition, &json!({}), &json!({}), start)?;
    let plan = plan(&mut expired, start, &mut source)?;
    assert_eq!(plan.deadline(), after(start, 2).monotonic);
    assert!(
        matches!(expired.advance(after(start,1),&mut source).step,DriverStep::Park(until) if until==plan.deadline())
    );
    let update = expired.complete_fanout(
        plan.attempt(),
        Ok(Payload::parse(b"[1]")?),
        after(start, 2),
        &mut source,
    );
    assert_eq!(update.cancellation(), Some(plan.attempt()));
    assert!(matches!(
        expired.outcome().map(AsRef::as_ref),
        Some(ExecutionOutcome::TimedOut)
    ));
    let mut stopped = driver(&definition, &json!({}), &json!({}), start)?;
    let plan = self::plan(&mut stopped, start, &mut source)?;
    let update = stopped.stop(TaskFailure::new("Stopped", None)?, start);
    assert_eq!(update.cancellation(), Some(plan.attempt()));
    let _ = stopped.complete_fanout(
        plan.attempt(),
        Ok(Payload::parse(b"[2]")?),
        start,
        &mut source,
    );
    assert!(matches!(
        stopped.outcome().map(AsRef::as_ref),
        Some(ExecutionOutcome::Aborted(_))
    ));
    assert!(
        stopped
            .stop(TaskFailure::new("Second", None)?, start)
            .cancellation()
            .is_none()
    );
    Ok(())
}

fn catching(name: &str) -> Value {
    json!({"StartAt":"Run","States":{"Run":{"Type":"Parallel","Branches":[leaf()],"Catch":[{"ErrorEquals":[name],"ResultPath":"$.caught","Next":"Recovered"}],"End":true},"Recovered":{"Type":"Succeed"}}})
}
#[test]
fn test_should_recover_typed_data_limit_merge_raw_input_and_keep_fatal_runtime_uncatchable()
-> TestResult {
    let start = now();
    let mut source = Source::default();
    let raw = json!({"keep":true});
    let mut recoverable = driver(
        &catching("States.DataLimitExceeded"),
        &raw,
        &json!({}),
        start,
    )?;
    let plan = plan(&mut recoverable, start, &mut source)?;
    let _ = recoverable.complete_fanout(
        plan.attempt(),
        Err(AslError::DataLimit.into()),
        start,
        &mut source,
    );
    let _ = recoverable.advance(start, &mut source);
    assert_eq!(output(&recoverable)?.get("keep"), Some(&json!(true)));
    assert_eq!(
        output(&recoverable)?
            .get("caught")
            .and_then(|v| v.get("Error")),
        Some(&json!("States.DataLimitExceeded"))
    );
    assert!(matches!(
        recoverable.diagnostic(),
        Some(AslError::DataLimit)
    ));
    let mut fatal = driver(&catching("States.ALL"), &raw, &json!({}), start)?;
    let plan = self::plan(&mut fatal, start, &mut source)?;
    let _ = fatal.complete_fanout(
        plan.attempt(),
        Err(AslError::Missing.into()),
        start,
        &mut source,
    );
    assert_eq!(failure(&fatal)?.error(), Some("States.Runtime"));
    assert!(matches!(fatal.diagnostic(), Some(AslError::Missing)));
    Ok(())
}

#[test]
fn test_should_match_omitted_branch_error_without_rewriting_unhandled_failure_fields() -> TestResult
{
    let start = now();
    let mut source = Source::default();
    let mut handled = driver(
        &catching("States.TaskFailed"),
        &json!({}),
        &json!({}),
        start,
    )?;
    let plan = plan(&mut handled, start, &mut source)?;
    let _ = handled.complete_fanout(
        plan.attempt(),
        Err(FanoutFailure::Branch(ExecutionFailure::fields(None, None))),
        start,
        &mut source,
    );
    let _ = handled.advance(start, &mut source);
    assert_eq!(output(&handled)?, &json!({"caught":{}}));
    let mut unhandled = driver(&parallel(1), &json!({}), &json!({}), start)?;
    let plan = self::plan(&mut unhandled, start, &mut source)?;
    let _ = unhandled.complete_fanout(
        plan.attempt(),
        Err(FanoutFailure::Branch(ExecutionFailure::fields(
            None,
            Some("secret-cause".into()),
        ))),
        start,
        &mut source,
    );
    assert_eq!(failure(&unhandled)?.error(), None);
    assert_eq!(failure(&unhandled)?.cause(), Some("secret-cause"));
    assert!(!format!("{unhandled:?}").contains("secret-cause"));
    Ok(())
}

#[tokio::test]
async fn test_should_reject_structured_effect_driver_before_supervisor_admission() -> TestResult {
    let start = now();
    let driver = driver(&parallel(1), &json!({}), &json!({}), start)?;
    let mut owner = ExecutionSupervisor::new(
        RuntimeLimits::default(),
        HistoryLimits::default(),
        256 * 1024 * 1024,
        None,
    )?;
    assert!(matches!(
        owner.start(driver),
        Err(ExecutionError::Unavailable)
    ));
    owner
        .drain(tokio::time::Instant::now() + Duration::from_secs(1))
        .await?;
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn test_should_collect_reversed_completion_order_by_item_index(count in 1_usize..80) {
        let start=now(); let mut source=Source::default();
        let mut driver=driver(&map(),&json!(vec![0;count]),&json!({}),start).map_err(|e|TestCaseError::fail(e.to_string()))?;
        let plan=plan(&mut driver,start,&mut source).map_err(|e|TestCaseError::fail(e.to_string()))?;
        let outputs=(0..count).rev().map(|index| Payload::from_value(&json!(index)).map(|value|(index,value))).collect::<Result<Vec<_>,_>>().map_err(|e|TestCaseError::fail(e.to_string()))?;
        let collected=plan.collect(&outputs).map_err(|e|TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(collected.value(),&json!((0..count).collect::<Vec<_>>()));
    }
}

#[test]
fn test_should_report_lazy_selector_errors_and_internal_failures_without_business_replay()
-> TestResult {
    let start = now();
    let mut source = Source::default();
    let mut definition = catching("States.ALL");
    definition["States"]["Run"] = json!({"Type":"Map","ItemProcessor":leaf(),"ItemSelector":{"bad.$":"$.absent"},"Catch":[{"ErrorEquals":["States.ALL"],"Next":"Recovered"}],"End":true});
    let mut driver = driver(&definition, &json!([1]), &json!({}), start)?;
    let plan = plan(&mut driver, start, &mut source)?;
    let error = plan
        .branch(0, &mut source)
        .err()
        .ok_or("missing selector failure")?;
    assert!(matches!(error, AslError::Missing));
    let _ = driver.complete_fanout(plan.attempt(), Err(error.into()), start, &mut source);
    assert_eq!(failure(&driver)?.error(), Some("States.Runtime"));
    assert!(matches!(driver.diagnostic(), Some(AslError::Missing)));
    let mut internal = self::driver(&catching("States.ALL"), &json!({}), &json!({}), start)?;
    let plan = self::plan(&mut internal, start, &mut source)?;
    let _ = internal.complete_fanout(
        plan.attempt(),
        Err(AslError::InvalidStateId.into()),
        start,
        &mut source,
    );
    assert_eq!(failure(&internal)?.error(), Some("Rustack.InternalError"));
    assert!(matches!(
        internal.diagnostic(),
        Some(AslError::InvalidStateId)
    ));
    Ok(())
}

#[test]
fn test_should_reject_round_retry_without_remaining_transitions_before_entropy_or_count_mutation()
-> TestResult {
    let start = now();
    let mut source = Source::default();
    let mut definition = parallel(1);
    definition["States"]["Run"]["Retry"] =
        json!([{"ErrorEquals":["Oops"],"JitterStrategy":"FULL"}]);
    let mut driver = driver(&definition, &json!({}), &json!({}), start)?;
    driver.remaining = 1;
    let plan = plan(&mut driver, start, &mut source)?;
    let _ = driver.complete_fanout(
        plan.attempt(),
        Err(TaskFailure::new("Oops", None)?.into()),
        start,
        &mut source,
    );
    assert_eq!(
        failure(&driver)?.error(),
        Some("Rustack.TransitionLimitExceeded")
    );
    assert_eq!(driver.retry_count(), 0);
    assert_eq!(source.draws, 0);
    assert_eq!(source.uuids, 0);
    Ok(())
}
