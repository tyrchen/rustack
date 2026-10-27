//! Independent callback instruction/effect and generated-history expectations.

use std::{collections::BTreeSet, time::SystemTime};

use chrono::Utc;
use rstest::rstest;
use serde_json::{Value, json};
use uuid::Uuid;

use super::*;
use crate::{
    AslCompiler, ClockSample, DriverStep, ExecutionDriver, ExecutionLimits, ExecutionOutcome,
    ExecutionSource, FrameCapacity, FrameLimits, FrameScheduler, FrameUpdate, StateSupport,
    TaskFailure, TaskSupport, TenantScope,
};

#[derive(Default)]
struct Source {
    uuids: usize,
    fail: bool,
}
impl UuidSource for Source {
    fn next_uuid(&mut self) -> Result<Uuid, AslError> {
        self.uuids += 1;
        if self.fail {
            return Err(AslError::InvalidExecution);
        }
        Ok(Uuid::parse_str("00000000-0000-4000-8000-000000000001").unwrap())
    }
}
impl ExecutionSource for Source {
    fn random_u64(&mut self) -> Result<u64, AslError> {
        Ok(0)
    }
}
fn definition(extra: Value) -> Value {
    let mut state = json!({"Type":"Task","Resource":"arn:aws:states:::lambda:invoke.waitForTaskToken","Parameters":{"FunctionName":"Worker","Payload":{"n.$":"$.n","token.$":"$$.Task.Token","retry.$":"$$.State.RetryCount","uuid.$":"States.UUID()"}},"End":true});
    let Value::Object(extra) = extra else {
        panic!("object fixture");
    };
    state.as_object_mut().unwrap().extend(extra);
    json!({"StartAt":"Call","States":{"Call":state}})
}
fn compile(definition: &Value) -> Arc<CompiledMachine> {
    AslCompiler::new(TenantScope::default(), TaskSupport::Lambda)
        .with_state_support(StateSupport::Structured)
        .with_callback_support(true)
        .compile(&serde_json::to_vec(definition).unwrap())
        .unwrap()
        .machine()
        .clone()
}
fn clock(base: Instant, seconds: u64) -> ClockSample {
    ClockSample::new(base + Duration::from_secs(seconds), Utc::now())
}
fn driver(definition: &Value, input: &Value) -> (ExecutionDriver, Instant) {
    let base = Instant::now();
    (
        ExecutionDriver::for_structured_owner(
            compile(definition),
            Payload::from_value(input).unwrap(),
            EvaluationContext::from_value(
                &json!({"Execution":{"Id":"actual-parent-context","Input":input}}),
            )
            .unwrap(),
            clock(base, 0),
            ExecutionLimits::new(Duration::from_secs(100), 1000).unwrap(),
        )
        .unwrap(),
        base,
    )
}
fn effect(
    driver: &mut ExecutionDriver,
    base: Instant,
    seconds: u64,
    source: &mut Source,
) -> CallbackDispatch {
    for _ in 0..10 {
        if let DriverStep::Callback(effect) = driver.advance(clock(base, seconds), source).step() {
            return effect.clone();
        }
    }
    panic!("expected deferred callback")
}
#[test]
fn test_should_require_explicit_callback_and_lambda_compiler_capabilities() {
    let bytes = serde_json::to_vec(&definition(json!({}))).unwrap();
    for (support, callback, expected) in [
        (TaskSupport::Lambda, false, "RUSTACK_PROFILE_UNAVAILABLE"),
        (TaskSupport::Disabled, true, "RUSTACK_DEPENDENCY_DISABLED"),
    ] {
        let error = AslCompiler::new(TenantScope::default(), support)
            .with_callback_support(callback)
            .compile(&bytes)
            .unwrap_err();
        assert_eq!(error.diagnostic().code(), expected);
    }
    assert!(compile(&definition(json!({}))).uses_callbacks());
    for resource in [
        "arn:aws-cn:states:::lambda:invoke.waitForTaskToken",
        "arn:aws:states:::lambda:invoke.sync",
        "arn:aws:states:::sqs:sendMessage.waitForTaskToken",
    ] {
        let mut source = definition(json!({}));
        source["States"]["Call"]["Resource"] = json!(resource);
        assert!(
            AslCompiler::new(TenantScope::default(), TaskSupport::Lambda)
                .with_callback_support(true)
                .compile(&serde_json::to_vec(&source).unwrap())
                .is_err()
        );
    }
}
#[rstest]
#[case(json!({"HeartbeatSeconds":0}))]
#[case(json!({"HeartbeatSeconds":-1}))]
#[case(json!({"HeartbeatSeconds":1.5}))]
#[case(json!({"HeartbeatSeconds":100_000_000}))]
#[case(json!({"HeartbeatSeconds":99_999_999}))]
#[case(json!({"HeartbeatSeconds":true}))]
#[case(json!({"HeartbeatSeconds":"5"}))]
#[case(json!({"HeartbeatSeconds":5,"HeartbeatSecondsPath":"$.hb"}))]
#[case(json!({"HeartbeatSecondsPath":5}))]
#[case(json!({"HeartbeatSecondsPath":"$.hb[*]"}))]
#[case(json!({"HeartbeatSeconds":5,"TimeoutSeconds":5}))]
#[case(json!({"HeartbeatSeconds":6,"TimeoutSeconds":5}))]
#[case(json!({"Parameters":{"FunctionName":"Worker","InvocationType":"Event"}}))]
fn test_should_reject_invalid_callback_intervals_and_async_invocation(#[case] extra: Value) {
    let source = definition(extra);
    assert!(
        AslCompiler::new(TenantScope::default(), TaskSupport::Lambda)
            .with_callback_support(true)
            .compile(&serde_json::to_vec(&source).unwrap())
            .is_err()
    );
}
#[rstest]
#[case("arn:aws:states:::lambda:invoke")]
#[case("arn:aws:lambda:us-east-1:000000000000:function:Worker")]
#[case("arn:aws:states:::states:startExecution.sync")]
fn test_should_keep_heartbeat_unavailable_on_other_integrations(#[case] resource: &str) {
    let mut source = definition(json!({"HeartbeatSeconds":5}));
    source["States"]["Call"]["Resource"] = json!(resource);
    let error = AslCompiler::new(TenantScope::default(), TaskSupport::Lambda)
        .with_callback_support(true)
        .with_child_support(true)
        .compile(&serde_json::to_vec(&source).unwrap())
        .unwrap_err();
    assert_eq!(error.diagnostic().code(), "RUSTACK_PROFILE_UNAVAILABLE");
}
#[test]
fn test_should_assign_attempt_and_original_input_intervals_before_evaluating_parameters_once() {
    let source = definition(
        json!({"InputPath":"$.payload","HeartbeatSecondsPath":"$.hb","TimeoutSecondsPath":"$.total"}),
    );
    let input = json!({"payload":{"n":21},"hb":10,"total":30,"untouched":"marker"});
    let (mut driver, base) = driver(&source, &input);
    let mut entropy = Source::default();
    let deferred = effect(&mut driver, base, 0, &mut entropy);
    assert_eq!(entropy.uuids, 0);
    assert_eq!(deferred.deadline(), base + Duration::from_secs(30));
    assert_eq!(deferred.timeout_seconds(), Some(30));
    assert_eq!(deferred.heartbeat(), Some(Duration::from_secs(10)));
    let token: CallbackToken = "a".repeat(64).parse().unwrap();
    let (actual, history) = deferred.prepare_with_source(&token, &mut entropy).unwrap();
    assert_eq!(entropy.uuids, 1);
    assert_eq!(actual.attempt(), deferred.attempt());
    assert_eq!(actual.state(), deferred.state());
    assert_eq!(actual.deadline(), deferred.deadline());
    assert_eq!(actual.parameters().value()["Payload"]["n"], 21);
    assert_eq!(
        actual.parameters().value()["Payload"]["token"],
        token.expose()
    );
    assert_eq!(actual.parameters().value()["Payload"]["retry"], 0);
    assert!(history.payload().value()["Payload"]["token"].is_null());
    assert_eq!(
        actual.parameters().value()["Payload"]["uuid"],
        history.payload().value()["Payload"]["uuid"]
    );
    assert_eq!(driver.input().value(), &input);
    assert!(deferred.context.value().get("Task").is_none());
    assert!(!format!("{deferred:?}{history:?}{actual:?}").contains(token.expose()));
}
#[rstest]
#[case(json!({"HeartbeatSeconds":40,"TimeoutSeconds":60}),Some(40),Some(60))]
#[case(json!({"HeartbeatSeconds":99_999_998}),Some(99_999_998),None)]
#[case(json!({}),None,None)]
fn test_should_preserve_omission_and_compare_heartbeat_to_explicit_timeout_not_remaining_parent(
    #[case] extra: Value,
    #[case] heartbeat: Option<u64>,
    #[case] timeout: Option<u32>,
) {
    let (mut driver, base) = driver(&definition(extra), &json!({"n":21}));
    driver.deadline = base + Duration::from_secs(20);
    let effect = effect(&mut driver, base, 0, &mut Source::default());
    assert_eq!(effect.deadline(), base + Duration::from_secs(20));
    assert_eq!(effect.timeout_seconds(), timeout);
    assert_eq!(effect.heartbeat(), heartbeat.map(Duration::from_secs));
}
#[rstest]
#[case(json!({"hb":0,"total":30}))]
#[case(json!({"hb":-1,"total":30}))]
#[case(json!({"hb":1.5,"total":30}))]
#[case(json!({"hb":"5","total":30}))]
#[case(json!({"hb":100_000_000,"total":30}))]
#[case(json!({"hb":30,"total":30}))]
#[case(json!({"hb":31,"total":30}))]
#[case(json!({"total":30}))]
#[case(json!({"hb":5,"total":null}))]
fn test_should_fail_invalid_dynamic_intervals_before_allocating_an_attempt_or_template_work(
    #[case] input: Value,
) {
    let (mut driver, base) = driver(
        &definition(json!({"HeartbeatSecondsPath":"$.hb","TimeoutSecondsPath":"$.total"})),
        &input,
    );
    let mut entropy = Source::default();
    let result = driver.advance(clock(base, 0), &mut entropy);
    assert!(
        matches!(result.step(),DriverStep::Settled(terminal) if matches!(terminal.as_ref(),ExecutionOutcome::Failed(_)))
    );
    assert_eq!(driver.generation, 0);
    assert_eq!(entropy.uuids, 0);
}
#[test]
fn test_should_preserve_typed_preparation_errors_without_partial_values_or_mutating_driver() {
    let (mut driver, base) = driver(&definition(json!({})), &json!({"n":21}));
    let effect = effect(&mut driver, base, 0, &mut Source::default());
    let token: CallbackToken = "a".repeat(64).parse().unwrap();
    let mut entropy = Source {
        fail: true,
        ..Source::default()
    };
    assert!(matches!(
        effect.prepare_with_source(&token, &mut entropy),
        Err(AslError::InvalidExecution)
    ));
    assert_eq!(entropy.uuids, 1);
    assert!(effect.context.value().get("Task").is_none());
    assert!(driver.outcome().is_none());
    let (_, history) = effect
        .prepare_with_source(&token, &mut Source::default())
        .unwrap();
    assert!(history.redacted());
}
#[test]
fn test_should_bind_callback_history_to_official_scheduled_and_truncated_submitted_schemas() {
    let (mut driver, base) = driver(
        &definition(json!({"TimeoutSeconds":30,"HeartbeatSeconds":10})),
        &json!({"n":21}),
    );
    let effect = effect(&mut driver, base, 0, &mut Source::default());
    let token: CallbackToken = "a".repeat(64).parse().unwrap();
    let (actual, history) = effect
        .prepare_with_source(&token, &mut Source::default())
        .unwrap();
    assert!(
        events::scheduled(
            effect.machine(),
            effect.state(),
            actual.parameters(),
            Some(30),
            Utc::now()
        )
        .is_err()
    );
    let scheduled = effect.scheduled_event(&history, Utc::now()).unwrap();
    let wire = serde_json::to_value(&scheduled).unwrap();
    assert_eq!(wire["type"], "TaskScheduled");
    assert_eq!(
        wire["taskScheduledEventDetails"]["resource"],
        "invoke.waitForTaskToken"
    );
    assert_eq!(wire["taskScheduledEventDetails"]["timeoutInSeconds"], 30);
    assert_eq!(wire["taskScheduledEventDetails"]["heartbeatInSeconds"], 10);
    assert!(
        wire["taskScheduledEventDetails"]
            .get("parametersDetails")
            .is_none()
    );
    assert!(!wire.to_string().contains(token.expose()));
    let meta = CallbackInvocationMetadata::new(Uuid::new_v4(), "$LATEST").unwrap();
    let submitted =
        serde_json::to_value(effect.submitted_event(&meta, Utc::now()).unwrap()).unwrap();
    assert_eq!(submitted["type"], "TaskSubmitted");
    assert_eq!(
        submitted["taskSubmittedEventDetails"]["outputDetails"]["truncated"],
        true
    );
    let output: Value = serde_json::from_str(
        submitted["taskSubmittedEventDetails"]["output"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert!(output.get("Payload").is_none());
    assert_eq!(
        output["SdkResponseMetadata"]["RequestId"],
        meta.request_id().to_string()
    );
}
#[test]
fn test_should_reuse_recovery_and_result_flow_with_fresh_callback_attempts_and_stale_rejection() {
    let source = definition(
        json!({"Retry":[{"ErrorEquals":["Busy"],"IntervalSeconds":1,"MaxAttempts":1}],"ResultSelector":{"selected.$":"$.value"},"ResultPath":"$.result","OutputPath":"$.result"}),
    );
    let (mut driver, base) = driver(&source, &json!({"n":21}));
    let mut entropy = Source::default();
    let first = effect(&mut driver, base, 0, &mut entropy);
    let update = driver.complete(
        first.attempt(),
        Err(TaskFailure::new("Busy", None).unwrap()),
        clock(base, 0),
        &mut entropy,
    );
    assert!(matches!(update.step(), DriverStep::Park(_)));
    let second = effect(&mut driver, base, 1, &mut entropy);
    assert_ne!(first.attempt(), second.attempt());
    let token: CallbackToken = "b".repeat(64).parse().unwrap();
    let (actual, _) = second.prepare_with_source(&token, &mut entropy).unwrap();
    assert_eq!(actual.parameters().value()["Payload"]["retry"], 1);
    let _ = driver.complete(
        first.attempt(),
        Ok(Payload::parse(b"false").unwrap()),
        clock(base, 1),
        &mut entropy,
    );
    assert!(driver.outcome().is_none());
    let _ = driver.complete(
        second.attempt(),
        Ok(Payload::parse(br#"{"value":42}"#).unwrap()),
        clock(base, 1),
        &mut entropy,
    );
    assert!(
        matches!(driver.outcome().unwrap().as_ref(),ExecutionOutcome::Succeeded(value) if value.value()==&json!({"selected":42}))
    );
    let _ = driver.complete(
        second.attempt(),
        Ok(Payload::parse(b"null").unwrap()),
        clock(base, 1),
        &mut entropy,
    );
    assert_eq!(entropy.uuids, 1);
}
#[test]
fn test_should_charge_and_retire_distinct_callback_frames_with_equal_local_generations() {
    let branch = definition(json!({}));
    let source = json!({"StartAt":"Both","States":{"Both":{"Type":"Parallel","Branches":[branch.clone(),branch],"End":true}}});
    let (driver, base) = driver(&source, &json!({"n":21}));
    let capacity = Arc::new(FrameCapacity::default());
    let mut frames = FrameScheduler::new(driver, capacity.clone(), FrameLimits::default()).unwrap();
    let mut entropy = Source::default();
    let mut calls = Vec::new();
    for _ in 0..50 {
        if let FrameUpdate::Transition(t) = frames.advance(clock(base, 0), &mut entropy) {
            if matches!(t.step(), DriverStep::Callback(_)) {
                calls.push(t.call().unwrap());
            }
        }
        if calls.len() == 2 {
            break;
        }
    }
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].attempt(), calls[1].attempt());
    assert_ne!(calls[0].frame(), calls[1].frame());
    assert!(capacity.available_frames() < 4096);
    assert_eq!(entropy.uuids, 0);
    for (index, call) in calls.iter().enumerate().rev() {
        frames
            .complete(
                *call,
                Ok(Payload::from_value(&json!(index)).unwrap()),
                clock(base, 0),
                &mut entropy,
            )
            .unwrap();
        frames.retired(*call).unwrap();
    }
    for _ in 0..100 {
        let _ = frames.advance(clock(base, 0), &mut entropy);
        if frames.outcome().is_some() {
            break;
        }
    }
    assert!(
        matches!(frames.outcome().unwrap().as_ref(),ExecutionOutcome::Succeeded(output) if output.value()==&json!([0,1]))
    );
}
#[test]
fn test_should_cancel_every_deferred_call_on_stop_and_wait_for_explicit_retirement() {
    let (driver, base) = driver(&definition(json!({})), &json!({"n":21}));
    let mut frames = FrameScheduler::new(
        driver,
        Arc::new(FrameCapacity::default()),
        FrameLimits::default(),
    )
    .unwrap();
    let mut entropy = Source::default();
    let call = loop {
        if let FrameUpdate::Transition(t) = frames.advance(clock(base, 0), &mut entropy) {
            if let Some(call) = t.call() {
                break call;
            }
        }
    };
    let mut cancelled = BTreeSet::new();
    if let FrameUpdate::Cancel(call) =
        frames.stop(TaskFailure::new("Stopped", None).unwrap(), clock(base, 0))
    {
        cancelled.insert(call);
    }
    for _ in 0..20 {
        if let FrameUpdate::Cancel(call) = frames.advance(clock(base, 0), &mut entropy) {
            cancelled.insert(call);
        }
    }
    assert_eq!(cancelled, BTreeSet::from([call]));
    frames.retired(call).unwrap();
    assert!(matches!(
        frames.outcome().unwrap().as_ref(),
        ExecutionOutcome::Aborted(_)
    ));
}
#[test]
fn test_should_reject_sequential_driver_admission_even_for_nested_callback_dependence() {
    let branch = definition(json!({}));
    for source in [
        branch.clone(),
        json!({"StartAt":"Both","States":{"Both":{"Type":"Parallel","Branches":[branch],"End":true}}}),
    ] {
        let machine = compile(&source);
        assert!(machine.uses_callbacks());
        assert!(machine.uses_lambda());
        let payload = Payload::parse(b"{}").unwrap();
        assert!(matches!(
            ExecutionDriver::new(
                machine,
                payload.clone(),
                EvaluationContext::from(&payload),
                ClockSample::new(Instant::now(), SystemTime::now().into()),
                ExecutionLimits::default()
            ),
            Err(AslError::StructuredExecutionRequired)
        ));
    }
}

#[rstest]
#[case(json!({"HeartbeatSeconds":10,"TimeoutSeconds":30}),json!({"n":21}),Some(10))]
#[case(json!({"HeartbeatSecondsPath":"$.hb","TimeoutSecondsPath":"$.total"}),json!({"n":21,"hb":15,"total":30}),Some(15))]
#[case(json!({}),json!({"n":21}),None)]
fn test_should_project_literal_dynamic_and_omitted_heartbeat_metadata(
    #[case] extra: Value,
    #[case] input: Value,
    #[case] expected: Option<i64>,
) {
    let (mut driver, base) = driver(&definition(extra), &input);
    let effect = effect(&mut driver, base, 0, &mut Source::default());
    let token: CallbackToken = "a".repeat(64).parse().unwrap();
    let (_, history) = effect
        .prepare_with_source(&token, &mut Source::default())
        .unwrap();
    let wire = serde_json::to_value(effect.scheduled_event(&history, Utc::now()).unwrap()).unwrap();
    assert_eq!(
        wire["taskScheduledEventDetails"]
            .get("heartbeatInSeconds")
            .and_then(Value::as_i64),
        expected
    );
    if expected.is_none() {
        assert!(
            wire["taskScheduledEventDetails"]
                .get("heartbeatInSeconds")
                .is_none()
        );
    }
}
#[test]
fn test_should_recover_callback_failure_through_original_input_catch_and_ignore_late_success() {
    let mut source = definition(
        json!({"Catch":[{"ErrorEquals":["ExternalFailure"],"ResultPath":"$.failure","Next":"Done"}]}),
    );
    source["States"]["Done"] = json!({"Type":"Succeed"});
    let (mut driver, base) = driver(&source, &json!({"n":21,"keep":"original"}));
    let mut entropy = Source::default();
    let effect = effect(&mut driver, base, 0, &mut entropy);
    let _ = driver.complete(
        effect.attempt(),
        Err(TaskFailure::new("ExternalFailure", Some("cause")).unwrap()),
        clock(base, 0),
        &mut entropy,
    );
    let _ = driver.complete(
        effect.attempt(),
        Ok(Payload::parse(b"false").unwrap()),
        clock(base, 0),
        &mut entropy,
    );
    let _ = driver.advance(clock(base, 0), &mut entropy);
    assert!(
        matches!(driver.outcome().unwrap().as_ref(),ExecutionOutcome::Succeeded(p) if p.value()==&json!({"n":21,"keep":"original","failure":{"Error":"ExternalFailure","Cause":"cause"}}))
    );
}
#[test]
fn test_should_keep_absolute_task_timeout_and_discard_late_callback_result() {
    let (mut driver, base) = driver(&definition(json!({"TimeoutSeconds":5})), &json!({"n":21}));
    let mut entropy = Source::default();
    let effect = effect(&mut driver, base, 0, &mut entropy);
    let result = driver.complete(
        effect.attempt(),
        Ok(Payload::parse(b"true").unwrap()),
        clock(base, 5),
        &mut entropy,
    );
    assert_eq!(result.cancellation(), Some(effect.attempt()));
    assert!(
        matches!(driver.outcome().unwrap().as_ref(),ExecutionOutcome::Failed(f) if f.error()==Some("States.Timeout"))
    );
    assert_eq!(entropy.uuids, 0);
}

mod admission;

#[rstest]
#[case(json!({"HeartbeatSeconds":5,"TimeoutSecondsPath":"$.total"}),json!({"total":6}),true)]
#[case(json!({"HeartbeatSeconds":5,"TimeoutSecondsPath":"$.total"}),json!({"total":5}),false)]
#[case(json!({"HeartbeatSecondsPath":"$.hb","TimeoutSeconds":6}),json!({"hb":5}),true)]
#[case(json!({"HeartbeatSecondsPath":"$.hb","TimeoutSeconds":5}),json!({"hb":5}),false)]
#[case(json!({"HeartbeatSecondsPath":"$.hb"}),json!({"hb":99_999_999}),false)]
fn test_should_check_mixed_static_dynamic_interval_relation_and_omitted_validation_ceiling(
    #[case] extra: Value,
    #[case] input: Value,
    #[case] valid: bool,
) {
    let (mut driver, base) = driver(&definition(extra), &input);
    let mut entropy = Source::default();
    let step = driver.advance(clock(base, 0), &mut entropy);
    assert_eq!(matches!(step.step(), DriverStep::Callback(_)), valid);
    assert_eq!(entropy.uuids, 0);
    if !valid {
        assert!(matches!(
            driver.outcome().unwrap().as_ref(),
            ExecutionOutcome::Failed(_)
        ));
        assert_eq!(driver.generation, 0);
    }
}
#[test]
fn test_should_prepare_map_callback_items_lazily_and_preserve_item_result_order() {
    let source = json!({"StartAt":"Items","States":{"Items":{"Type":"Map","ItemSelector":{"n.$":"$$.Map.Item.Value"},"ItemProcessor":definition(json!({})),"End":true}}});
    let (driver, base) = driver(&source, &json!([21, 22]));
    let mut frames = FrameScheduler::new(
        driver,
        Arc::new(FrameCapacity::default()),
        FrameLimits::default(),
    )
    .unwrap();
    let mut entropy = Source::default();
    let mut effects = Vec::new();
    for _ in 0..50 {
        if let FrameUpdate::Transition(t) = frames.advance(clock(base, 0), &mut entropy) {
            if let DriverStep::Callback(effect) = t.step() {
                effects.push((t.call().unwrap(), effect.clone()));
            }
        }
        if effects.len() == 2 {
            break;
        }
    }
    assert_eq!(effects.len(), 2);
    assert_eq!(entropy.uuids, 0);
    for (call, effect) in effects.iter().rev() {
        let token: CallbackToken = "a".repeat(64).parse().unwrap();
        let (dispatch, history) = effect.prepare_with_source(&token, &mut entropy).unwrap();
        assert!(history.payload().value()["Payload"]["token"].is_null());
        let n = dispatch.parameters().value()["Payload"]["n"]
            .as_u64()
            .unwrap();
        frames
            .complete(
                *call,
                Ok(Payload::from_value(&json!(n * 2)).unwrap()),
                clock(base, 0),
                &mut entropy,
            )
            .unwrap();
        frames.retired(*call).unwrap();
    }
    for _ in 0..100 {
        let _ = frames.advance(clock(base, 0), &mut entropy);
        if frames.outcome().is_some() {
            break;
        }
    }
    assert_eq!(entropy.uuids, 2);
    assert!(
        matches!(frames.outcome().unwrap().as_ref(),ExecutionOutcome::Succeeded(p) if p.value()==&json!([42,44]))
    );
}
