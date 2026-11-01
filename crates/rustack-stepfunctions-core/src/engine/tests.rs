//! Explicit-clock execution fixtures; no fake service is installed in production.

use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use rstest::rstest;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    AslCompiler, AslError, ClockSample, DriverStep, EvaluationContext, ExecutionDriver,
    ExecutionLimits, ExecutionOutcome, ExecutionSource, Payload, SystemUuidSource, TaskDispatch,
    TaskFailure, TaskSupport, TenantScope, UuidSource,
};

#[derive(Default)]
struct Source {
    draws: VecDeque<u64>,
    count: usize,
}
impl UuidSource for Source {
    fn next_uuid(&mut self) -> Result<Uuid, AslError> {
        Ok(Uuid::from_bytes([
            0, 0, 0, 0, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 0, 0, 1,
        ]))
    }
}
impl ExecutionSource for Source {
    fn random_u64(&mut self) -> Result<u64, AslError> {
        self.count += 1;
        Ok(self.draws.pop_front().unwrap_or(u64::MAX))
    }
}
fn now(base: Instant, seconds: u64) -> ClockSample {
    ClockSample::new(
        base + Duration::from_secs(seconds),
        DateTime::parse_from_rfc3339("2026-10-26T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
            + chrono::Duration::seconds(seconds.try_into().unwrap()),
    )
}
fn driver(definition: &Value, input: &Value) -> (ExecutionDriver, Instant) {
    let machine = AslCompiler::new(TenantScope::default(), TaskSupport::Lambda)
        .compile(&serde_json::to_vec(definition).unwrap())
        .unwrap()
        .machine()
        .clone();
    let context=EvaluationContext::from_value(&json!({"Execution":{"Input":input,"Id":"execution-test"},"StateMachine":{"Id":"machine-test"}})).unwrap();
    let base = Instant::now();
    (
        ExecutionDriver::new(
            machine,
            Payload::from_value(input).unwrap(),
            context,
            now(base, 0),
            ExecutionLimits::new(Duration::from_secs(100), 100).unwrap(),
        )
        .unwrap(),
        base,
    )
}
fn task(extra: &Value) -> Value {
    let mut state = json!({"Type":"Task","Resource":"arn:aws:lambda:us-east-1:000000000000:function:Worker","End":true});
    state
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    json!({"StartAt":"Work","States":{"Work":state,"Caught":{"Type":"Succeed"}}})
}
fn dispatch(d: &mut ExecutionDriver, base: Instant, seconds: u64) -> TaskDispatch {
    match d.advance(now(base, seconds), &mut Source::default()).step {
        DriverStep::Dispatch(call) => call,
        step => panic!("expected dispatch, got {step:?}"),
    }
}
fn failure(d: &ExecutionDriver) -> &str {
    match d.outcome().unwrap().as_ref() {
        ExecutionOutcome::Failed(failure) => failure.error().unwrap_or("<omitted>"),
        outcome => panic!("expected failure, got {outcome:?}"),
    }
}
fn output(d: &ExecutionDriver) -> &Value {
    match d.outcome().unwrap().as_ref() {
        ExecutionOutcome::Succeeded(output) => output.value(),
        outcome => panic!("expected success, got {outcome:?}"),
    }
}
fn fail_task(
    d: &mut ExecutionDriver,
    call: &TaskDispatch,
    base: Instant,
    seconds: u64,
    error: &str,
) -> crate::DriverUpdate {
    d.complete(
        call.attempt(),
        Err(TaskFailure::new(error, Some("reason")).unwrap()),
        now(base, seconds),
        &mut Source::default(),
    )
}
fn wake(d: &mut ExecutionDriver, base: Instant, seconds: u64) {
    assert!(matches!(
        d.advance(now(base, seconds), &mut Source::default()).step,
        DriverStep::Continue
    ));
}

#[test]
fn test_should_reject_invalid_driver_settings_and_context() {
    for (duration, transitions) in [(0, 1), (31_536_001, 1), (1, 0), (1, 1_000_001)] {
        assert!(matches!(
            ExecutionLimits::new(Duration::from_secs(duration), transitions),
            Err(AslError::InvalidExecution)
        ));
    }
    let (d, base) = driver(
        &json!({"StartAt":"Done","States":{"Done":{"Type":"Succeed"}}}),
        &json!({}),
    );
    let scalar = Payload::parse(b"null").unwrap();
    assert!(matches!(
        ExecutionDriver::new(
            d.machine().clone(),
            scalar.clone(),
            EvaluationContext::from(&scalar),
            now(base, 0),
            ExecutionLimits::default()
        ),
        Err(AslError::InvalidExecution)
    ));
    assert!(ExecutionLimits::new(Duration::from_hours(8760), 1_000_000).is_ok());
}

#[test]
fn test_should_execute_pass_against_original_input_and_succeed_paths() {
    let (mut d, base) = driver(
        &json!({"StartAt":"Prepare","States":{
            "Prepare":{"Type":"Pass","InputPath":"$.data","Parameters":{"value.$":"$.n","name.$":"$$.State.Name"},"ResultPath":"$.answer","Next":"Done"},
            "Done":{"Type":"Succeed","InputPath":"$.answer","OutputPath":"$.value"}
        }}),
        &json!({"data":{"n":7},"keep":true}),
    );
    wake(&mut d, base, 0);
    assert_eq!(
        d.input().value(),
        &json!({"data":{"n":7},"keep":true,"answer":{"value":7,"name":"Prepare"}})
    );
    let _ = d.advance(now(base, 0), &mut Source::default());
    assert_eq!(output(&d), &json!(7));
}

#[test]
fn test_should_use_static_pass_result_and_null_paths() {
    let (mut d, base) = driver(
        &json!({"StartAt":"P","States":{"P":{"Type":"Pass","InputPath":null,"Result":{"answer":42},"OutputPath":"$.answer","End":true}}}),
        &json!([1, 2]),
    );
    let _ = d.advance(now(base, 0), &mut Source::default());
    assert_eq!(output(&d), &json!(42));
}

#[test]
fn test_should_evaluate_choice_on_effective_input_and_preserve_branch_order() {
    let (mut d, base) = driver(
        &json!({"StartAt":"Choose","States":{
            "Choose":{"Type":"Choice","InputPath":"$.data","OutputPath":"$.n","Choices":[{"Variable":"$.n","NumericEquals":2,"Next":"Done"},{"Variable":"$.missing","NumericEquals":1,"Next":"Fail"}],"Default":"Fail"},
            "Done":{"Type":"Succeed"},"Fail":{"Type":"Fail"}
        }}),
        &json!({"data":{"n":2},"keep":true}),
    );
    wake(&mut d, base, 0);
    assert_eq!(d.input().value(), &json!(2));
    let _ = d.advance(now(base, 0), &mut Source::default());
    assert_eq!(output(&d), &json!(2));
}

#[rstest]
#[case(false, "States.NoChoiceMatched")]
#[case(true, "States.Runtime")]
fn test_should_fail_unmatched_or_unevaluable_choice(#[case] missing: bool, #[case] error: &str) {
    let variable = if missing { "$.missing" } else { "$.n" };
    let (mut d, base) = driver(
        &json!({"StartAt":"C","States":{"C":{"Type":"Choice","Choices":[{"Variable":variable,"NumericEquals":2,"Next":"Done"}]},"Done":{"Type":"Succeed"}}}),
        &json!({"n":1}),
    );
    let _ = d.advance(now(base, 0), &mut Source::default());
    assert_eq!(failure(&d), error);
}

#[rstest]
#[case(json!({}),None,None)]
#[case(json!({"Error":"","Cause":"literal"}),Some(""),Some("literal"))]
#[case(json!({"ErrorPath":"$.error","CausePath":"States.Format('item {}', $.n)"}),Some("BusinessError"),Some("item 3"))]
fn test_should_preserve_fail_omission_and_dynamic_string_fields(
    #[case] fields: Value,
    #[case] error: Option<&str>,
    #[case] cause: Option<&str>,
) {
    let mut state = json!({"Type":"Fail"});
    state
        .as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    let (mut d, base) = driver(
        &json!({"StartAt":"F","States":{"F":state}}),
        &json!({"error":"BusinessError","n":3}),
    );
    let _ = d.advance(now(base, 0), &mut Source::default());
    match d.outcome().unwrap().as_ref() {
        ExecutionOutcome::Failed(f) => {
            assert_eq!(f.error(), error);
            assert_eq!(f.cause(), cause);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[rstest]
#[case(json!(3))]
#[case(json!("x".repeat(257)))]
fn test_should_reject_invalid_dynamic_fail_values(#[case] value: Value) {
    let (mut d, base) = driver(
        &json!({"StartAt":"F","States":{"F":{"Type":"Fail","ErrorPath":"$.error"}}}),
        &json!({"error":value}),
    );
    let _ = d.advance(now(base, 0), &mut Source::default());
    assert_eq!(failure(&d), "States.Runtime");
    assert!(matches!(d.diagnostic(), Some(AslError::InvalidFailure)));
}

#[rstest]
#[case(json!({"Seconds":3}))]
#[case(json!({"SecondsPath":"$.seconds"}))]
#[case(json!({"Timestamp":"2026-10-26T00:00:03.999Z"}))]
#[case(json!({"TimestampPath":"$.date"}))]
fn test_should_park_wait_once_despite_wall_clock_jumps(#[case] source: Value) {
    let mut state = json!({"Type":"Wait","InputPath":"$.data","OutputPath":"$.seconds","End":true});
    state
        .as_object_mut()
        .unwrap()
        .extend(source.as_object().unwrap().clone());
    let (mut d, base) = driver(
        &json!({"StartAt":"W","States":{"W":state}}),
        &json!({"data":{"seconds":3,"date":"2026-10-26T00:00:03Z"}}),
    );
    assert!(
        matches!(d.advance(now(base,0),&mut Source::default()).step,DriverStep::Park(t) if t==base+Duration::from_secs(3))
    );
    let mut jumped = now(base, 1);
    jumped.wall += chrono::Duration::days(30);
    assert!(
        matches!(d.advance(jumped,&mut Source::default()).step,DriverStep::Park(t) if t==base+Duration::from_secs(3))
    );
    let mut back = now(base, 3);
    back.wall -= chrono::Duration::days(30);
    let _ = d.advance(back, &mut Source::default());
    assert_eq!(output(&d), &json!(3));
}

#[test]
fn test_should_finish_zero_wait_immediately_and_bound_long_wait_by_execution_deadline() {
    let (mut zero, base) = driver(
        &json!({"StartAt":"W","States":{"W":{"Type":"Wait","Seconds":0,"End":true}}}),
        &json!(true),
    );
    let _ = zero.advance(now(base, 0), &mut Source::default());
    assert_eq!(output(&zero), &json!(true));
    let (mut long, base) = driver(
        &json!({"TimeoutSeconds":2,"StartAt":"W","States":{"W":{"Type":"Wait","Seconds":30,"End":true}}}),
        &json!({}),
    );
    assert!(
        matches!(long.advance(now(base,0),&mut Source::default()).step,DriverStep::Park(t) if t==base+Duration::from_secs(2))
    );
    let _ = long.advance(now(base, 2), &mut Source::default());
    assert!(matches!(
        long.outcome().unwrap().as_ref(),
        ExecutionOutcome::TimedOut
    ));
}

#[test]
fn test_should_prepare_task_and_merge_selected_result_into_raw_input() {
    let (mut d, base) = driver(
        &task(
            &json!({"InputPath":"$.data","Parameters":{"sent.$":"$.n"},"ResultSelector":{"picked.$":"$.value"},"ResultPath":"$.result"}),
        ),
        &json!({"data":{"n":7},"keep":true}),
    );
    let call = dispatch(&mut d, base, 0);
    assert_eq!(call.parameters().value(), &json!({"sent":7}));
    let _ = d.complete(
        call.attempt(),
        Ok(Payload::from_value(&json!({"value":8})).unwrap()),
        now(base, 1),
        &mut Source::default(),
    );
    assert_eq!(
        output(&d),
        &json!({"data":{"n":7},"keep":true,"result":{"picked":8}})
    );
}

#[test]
fn test_should_resolve_timeout_path_from_raw_input_before_parameters() {
    let (mut d, base) = driver(
        &task(
            &json!({"InputPath":"$.data","Parameters":{"timeout":90},"TimeoutSecondsPath":"$.timeout"}),
        ),
        &json!({"timeout":2,"data":{"timeout":70}}),
    );
    let call = dispatch(&mut d, base, 0);
    assert_eq!(call.parameters().value(), &json!({"timeout":90}));
    assert_eq!(call.deadline(), base + Duration::from_secs(2));
}

#[rstest]
#[case(json!(0))]
#[case(json!(-1))]
#[case(json!(1.0))]
#[case(json!(100_000_000))]
#[case(json!("2"))]
#[case(Value::Null)]
fn test_should_reject_dynamic_timeout_values_before_dispatch(#[case] value: Value) {
    let (mut d, base) = driver(
        &task(&json!({"TimeoutSecondsPath":"$.timeout"})),
        &json!({"timeout":value}),
    );
    let _ = d.advance(now(base, 0), &mut Source::default());
    assert_eq!(failure(&d), "States.Runtime");
}

#[test]
fn test_should_retry_with_original_input_fresh_attempt_and_preserved_entry_time() {
    let (mut d, base) = driver(
        &task(
            &json!({"Parameters":{"n.$":"$.n","retry.$":"$$.State.RetryCount","time.$":"$$.State.EnteredTime","uuid.$":"States.UUID()"},"Retry":[{"ErrorEquals":["Oops"],"MaxAttempts":1}]}),
        ),
        &json!({"n":7}),
    );
    let first = dispatch(&mut d, base, 0);
    assert_eq!(first.parameters().value().get("retry"), Some(&json!(0)));
    let result = fail_task(&mut d, &first, base, 1, "Oops");
    assert!(matches!(result.step,DriverStep::Park(t) if t==base+Duration::from_secs(2)));
    assert_eq!(d.retry_count(), 1);
    wake(&mut d, base, 2);
    let second = dispatch(&mut d, base, 2);
    assert_ne!(first.attempt(), second.attempt());
    assert_eq!(second.parameters().value().get("retry"), Some(&json!(1)));
    assert_eq!(
        first.parameters().value().get("time"),
        second.parameters().value().get("time")
    );
    assert_eq!(d.input().value(), &json!({"n":7}));
    let stale = d.complete(
        first.attempt(),
        Ok(Payload::parse(b"0").unwrap()),
        now(base, 2),
        &mut Source::default(),
    );
    assert!(matches!(stale.step, DriverStep::Park(_)));
    assert!(d.outcome().is_none());
    let _ = d.complete(
        second.attempt(),
        Ok(Payload::parse(b"9").unwrap()),
        now(base, 3),
        &mut Source::default(),
    );
    assert_eq!(output(&d), &json!(9));
}

#[test]
fn test_should_track_retries_per_matcher_and_not_fall_through_exhausted_first_match() {
    let (mut d, base) = driver(
        &task(
            &json!({"Retry":[{"ErrorEquals":["A"],"MaxAttempts":1},{"ErrorEquals":["B"],"MaxAttempts":1},{"ErrorEquals":["States.ALL"],"MaxAttempts":5}],"Catch":[{"ErrorEquals":["States.ALL"],"Next":"Caught","ResultPath":"$.failure"}]}),
        ),
        &json!({"keep":true}),
    );
    let first = dispatch(&mut d, base, 0);
    let _ = fail_task(&mut d, &first, base, 0, "A");
    wake(&mut d, base, 1);
    let second = dispatch(&mut d, base, 1);
    let _ = fail_task(&mut d, &second, base, 1, "B");
    wake(&mut d, base, 2);
    let third = dispatch(&mut d, base, 2);
    let _ = fail_task(&mut d, &third, base, 2, "A");
    assert_eq!(
        d.input().value(),
        &json!({"keep":true,"failure":{"Error":"A","Cause":"reason"}})
    );
    let _ = d.advance(now(base, 2), &mut Source::default());
    assert_eq!(d.retry_count(), 0);
    assert!(matches!(
        d.outcome().unwrap().as_ref(),
        ExecutionOutcome::Succeeded(_)
    ));
}

#[rstest]
#[case(Value::Null,json!({"keep":true}))]
#[case(json!("$"),json!({"Error":"Oops","Cause":"reason"}))]
#[case(json!("$.error"),json!({"keep":true,"error":{"Error":"Oops","Cause":"reason"}}))]
fn test_should_apply_catch_result_path_after_zero_retry(
    #[case] path: Value,
    #[case] expected: Value,
) {
    let (mut d, base) = driver(
        &task(
            &json!({"Retry":[{"ErrorEquals":["Oops"],"MaxAttempts":0},{"ErrorEquals":["States.ALL"]}],"Catch":[{"ErrorEquals":["States.ALL"],"Next":"Caught","ResultPath":path}]}),
        ),
        &json!({"keep":true}),
    );
    let call = dispatch(&mut d, base, 0);
    let _ = fail_task(&mut d, &call, base, 0, "Oops");
    assert_eq!(d.input().value(), &expected);
}

#[test]
fn test_should_use_capped_exponential_backoff_and_saturate_huge_rate() {
    for (rate, second) in [(3.0, 4), (1e308, 4)] {
        let (mut d, base) = driver(
            &task(
                &json!({"Retry":[{"ErrorEquals":["Oops"],"IntervalSeconds":2,"BackoffRate":rate,"MaxDelaySeconds":4,"MaxAttempts":2}]}),
            ),
            &json!({}),
        );
        let first = dispatch(&mut d, base, 0);
        assert!(
            matches!(fail_task(&mut d,&first,base,0,"Oops").step,DriverStep::Park(t) if t==base+Duration::from_secs(2))
        );
        wake(&mut d, base, 2);
        let second_call = dispatch(&mut d, base, 2);
        assert!(
            matches!(fail_task(&mut d,&second_call,base,2,"Oops").step,DriverStep::Park(t) if t==base+Duration::from_secs(2+second))
        );
    }
}

#[test]
fn test_should_sample_full_jitter_with_bounded_unbiased_rejection() {
    let (mut d, base) = driver(
        &task(&json!({"Retry":[{"ErrorEquals":["Oops"],"JitterStrategy":"FULL"}]})),
        &json!({}),
    );
    let call = dispatch(&mut d, base, 0);
    let mut source = Source {
        draws: VecDeque::from([0, u64::MAX]),
        count: 0,
    };
    let update = d.complete(
        call.attempt(),
        Err(TaskFailure::new("Oops", None).unwrap()),
        now(base, 0),
        &mut source,
    );
    let expected = Duration::from_nanos(u64::MAX % 1_000_000_001);
    assert!(matches!(update.step,DriverStep::Park(t) if t==base+expected));
    assert_eq!(source.count, 2);
    let (mut d, base) = driver(
        &task(&json!({"Retry":[{"ErrorEquals":["Oops"],"JitterStrategy":"FULL"}]})),
        &json!({}),
    );
    let call = dispatch(&mut d, base, 0);
    let mut source = Source {
        draws: VecDeque::from(vec![0; 128]),
        count: 0,
    };
    let _ = d.complete(
        call.attempt(),
        Err(TaskFailure::new("Oops", None).unwrap()),
        now(base, 0),
        &mut source,
    );
    assert_eq!(source.count, 128);
    assert_eq!(failure(&d), "Rustack.InternalError");
}

#[test]
fn test_should_not_resume_retry_at_overall_deadline() {
    let mut definition = task(&json!({"Retry":[{"ErrorEquals":["Oops"],"IntervalSeconds":99}]}));
    definition["TimeoutSeconds"] = json!(3);
    let (mut d, base) = driver(&definition, &json!({}));
    let call = dispatch(&mut d, base, 0);
    assert!(
        matches!(fail_task(&mut d,&call,base,1,"Oops").step,DriverStep::Park(t) if t==base+Duration::from_secs(3))
    );
    let _ = d.advance(now(base, 3), &mut Source::default());
    assert!(matches!(
        d.outcome().unwrap().as_ref(),
        ExecutionOutcome::TimedOut
    ));
}

#[test]
fn test_should_cancel_exact_task_deadline_once_and_recover_timeout() {
    let (mut d, base) = driver(
        &task(
            &json!({"TimeoutSeconds":2,"Catch":[{"ErrorEquals":["States.Timeout"],"Next":"Caught"}]}),
        ),
        &json!({}),
    );
    let call = dispatch(&mut d, base, 0);
    let result = d.complete(
        call.attempt(),
        Ok(Payload::parse(b"true").unwrap()),
        now(base, 2),
        &mut Source::default(),
    );
    assert_eq!(result.cancellation(), Some(call.attempt()));
    assert!(matches!(result.step, DriverStep::Continue));
    let duplicate = d.complete(
        call.attempt(),
        Ok(Payload::parse(b"false").unwrap()),
        now(base, 2),
        &mut Source::default(),
    );
    assert_eq!(duplicate.cancellation(), None);
    let _ = d.advance(now(base, 2), &mut Source::default());
    assert_eq!(output(&d).get("Error"), Some(&json!("States.Timeout")));
}

#[test]
fn test_should_prioritize_execution_deadline_over_task_result_and_stop() {
    for stop in [false, true] {
        let mut definition = task(
            &json!({"TimeoutSeconds":2,"Catch":[{"ErrorEquals":["States.ALL"],"Next":"Caught"}]}),
        );
        definition["TimeoutSeconds"] = json!(2);
        let (mut d, base) = driver(&definition, &json!({}));
        let call = dispatch(&mut d, base, 0);
        let update = if stop {
            d.stop(TaskFailure::new("Stopped", None).unwrap(), now(base, 2))
        } else {
            d.complete(
                call.attempt(),
                Ok(Payload::parse(b"1").unwrap()),
                now(base, 2),
                &mut Source::default(),
            )
        };
        assert_eq!(update.cancellation(), Some(call.attempt()));
        assert!(matches!(
            d.outcome().unwrap().as_ref(),
            ExecutionOutcome::TimedOut
        ));
    }
}

#[test]
fn test_should_settle_stop_once_ignore_late_completion_and_preserve_terminal_arc() {
    let (mut d, base) = driver(
        &task(
            &json!({"Retry":[{"ErrorEquals":["States.ALL"]}],"Catch":[{"ErrorEquals":["States.ALL"],"Next":"Caught"}]}),
        ),
        &json!({}),
    );
    let call = dispatch(&mut d, base, 0);
    let stopped = d.stop(
        TaskFailure::new("Stopped", Some("cancel")).unwrap(),
        now(base, 1),
    );
    assert_eq!(stopped.cancellation(), Some(call.attempt()));
    let outcome = d.outcome().unwrap().clone();
    assert!(matches!(outcome.as_ref(), ExecutionOutcome::Aborted(_)));
    let result = d.complete(
        call.attempt(),
        Ok(Payload::parse(b"true").unwrap()),
        now(base, 2),
        &mut Source::default(),
    );
    assert_eq!(result.cancellation(), None);
    let _ = d.stop(TaskFailure::new("Other", None).unwrap(), now(base, 0));
    assert!(Arc::ptr_eq(&outcome, d.outcome().unwrap()));
}

#[test]
fn test_should_ignore_duplicate_success_and_fail_clock_regression_with_cleanup_obligation() {
    let (mut d, base) = driver(&task(&json!({})), &json!({}));
    let call = dispatch(&mut d, base, 1);
    let update = d.advance(now(base, 0), &mut Source::default());
    assert_eq!(update.cancellation(), Some(call.attempt()));
    assert_eq!(failure(&d), "Rustack.InternalError");
    let (mut d, base) = driver(&task(&json!({})), &json!({}));
    let call = dispatch(&mut d, base, 0);
    let _ = d.complete(
        call.attempt(),
        Ok(Payload::parse(b"1").unwrap()),
        now(base, 1),
        &mut Source::default(),
    );
    let _ = fail_task(&mut d, &call, base, 2, "Oops");
    assert_eq!(output(&d), &json!(1));
}

#[test]
fn test_should_not_catch_runtime_or_wildcard_data_limit_but_allow_explicit_data_limit() {
    for (error, catch_name, handled) in [
        ("States.Runtime", "States.Runtime", false),
        ("States.DataLimitExceeded", "States.ALL", false),
        ("States.DataLimitExceeded", "States.DataLimitExceeded", true),
    ] {
        let (mut d, base) = driver(
            &task(&json!({"Catch":[{"ErrorEquals":[catch_name],"Next":"Caught"}]})),
            &json!({}),
        );
        let call = dispatch(&mut d, base, 0);
        let _ = fail_task(&mut d, &call, base, 0, error);
        assert_eq!(d.outcome().is_none(), handled);
        if !handled {
            assert_eq!(failure(&d), error);
        }
    }
}

#[test]
fn test_should_fail_tight_loop_with_transition_limit_and_allow_exact_budget() {
    let (mut d, base) = driver(
        &json!({"StartAt":"Loop","States":{"Loop":{"Type":"Choice","Choices":[{"Variable":"$","BooleanEquals":true,"Next":"Loop"}],"Default":"Done"},"Done":{"Type":"Succeed"}}}),
        &json!(true),
    );
    d.remaining = 3;
    for _ in 0..3 {
        wake(&mut d, base, 0);
    }
    let _ = d.advance(now(base, 0), &mut Source::default());
    assert_eq!(failure(&d), "Rustack.TransitionLimitExceeded");
    let (mut d, base) = driver(
        &json!({"StartAt":"P","States":{"P":{"Type":"Pass","Next":"Done"},"Done":{"Type":"Succeed"}}}),
        &json!(true),
    );
    d.remaining = 2;
    wake(&mut d, base, 0);
    let _ = d.advance(now(base, 0), &mut Source::default());
    assert_eq!(output(&d), &json!(true));
}

#[test]
fn test_should_validate_failure_bounds_and_redact_sensitive_driver_data() {
    assert!(TaskFailure::new("", None).is_err());
    assert!(TaskFailure::new("x\ny", None).is_err());
    assert!(TaskFailure::new(&"x".repeat(257), None).is_err());
    assert!(TaskFailure::new("Error", Some(&"x".repeat(32_769))).is_err());
    let (mut d, base) = driver(&task(&json!({})), &json!({"secret":"sensitive-marker"}));
    let call = dispatch(&mut d, base, 0);
    assert!(!format!("{call:?} {d:?}").contains("sensitive-marker"));
    let _ = fail_task(&mut d, &call, base, 1, "sensitive-marker");
    assert!(!format!("{d:?}").contains("sensitive-marker"));
}

#[test]
fn test_should_support_real_system_source_for_nonblocking_pure_callers() {
    let source = &mut SystemUuidSource::default();
    let _ = source.random_u64().unwrap();
}

#[test]
fn test_should_fail_exhausted_retry_budget_before_scheduling_a_timer() {
    let (mut d, base) = driver(
        &task(
            &json!({"Retry":[{"ErrorEquals":["Oops"],"IntervalSeconds":99,"JitterStrategy":"FULL"}]}),
        ),
        &json!({}),
    );
    d.remaining = 1;
    let call = dispatch(&mut d, base, 0);
    let mut source = Source::default();
    let result = d.complete(
        call.attempt(),
        Err(TaskFailure::new("Oops", None).unwrap()),
        now(base, 1),
        &mut source,
    );
    assert!(matches!(result.step, DriverStep::Settled(_)));
    assert_eq!(failure(&d), "Rustack.TransitionLimitExceeded");
    assert_eq!(d.retry_count(), 0);
    assert_eq!(source.count, 0);
}

#[test]
fn test_should_classify_failed_catch_merge_as_runtime_without_retrying_recovery() {
    let (mut d, base) = driver(
        &task(
            &json!({"Catch":[{"ErrorEquals":["States.ALL"],"ResultPath":"$.scalar.child","Next":"Caught"}]}),
        ),
        &json!({"scalar":1}),
    );
    let call = dispatch(&mut d, base, 0);
    let _ = fail_task(&mut d, &call, base, 0, "Oops");
    assert_eq!(failure(&d), "States.Runtime");
    assert!(matches!(d.diagnostic(), Some(AslError::Merge)));
}

#[test]
fn test_should_recover_explicit_result_and_parameter_data_limits() {
    for field in ["Parameters", "ResultSelector"] {
        let mut extra = json!({"Catch":[{"ErrorEquals":["States.DataLimitExceeded"],"ResultPath":null,"Next":"Caught"}]});
        // The source template fits1MiB, but the expanded payload exceeds256KiB.
        extra[field] = json!({"large":"x".repeat(131_073),"more":"x".repeat(131_073)});
        let (mut d, base) = driver(&task(&extra), &json!({"keep":true}));
        let first = d.advance(now(base, 0), &mut Source::default());
        if field == "ResultSelector" {
            let DriverStep::Dispatch(call) = first.step else {
                panic!("expected Task")
            };
            let _ = d.complete(
                call.attempt(),
                Ok(Payload::parse(b"{}").unwrap()),
                now(base, 0),
                &mut Source::default(),
            );
        } else {
            assert!(matches!(first.step, DriverStep::Continue));
        }
        assert!(matches!(d.diagnostic(), Some(AslError::DataLimit)));
        let _ = d.advance(now(base, 0), &mut Source::default());
        assert_eq!(output(&d), &json!({"keep":true}));
    }
}

#[test]
fn test_should_fail_uncatchable_result_selection_errors() {
    let (mut d, base) = driver(
        &task(
            &json!({"ResultSelector":{"missing.$":"$.missing"},"Catch":[{"ErrorEquals":["States.ALL"],"Next":"Caught"}]}),
        ),
        &json!({}),
    );
    let call = dispatch(&mut d, base, 0);
    let _ = d.complete(
        call.attempt(),
        Ok(Payload::parse(b"{}").unwrap()),
        now(base, 0),
        &mut Source::default(),
    );
    assert_eq!(failure(&d), "States.Runtime");
    assert!(matches!(d.diagnostic(), Some(AslError::Missing)));
}

#[test]
fn test_should_capture_new_entry_time_and_reset_retry_state_after_catch() {
    let (mut d, base) = driver(
        &json!({"StartAt":"First","States":{
            "First":{"Type":"Task","Resource":"arn:aws:lambda:us-east-1:000000000000:function:Worker","Parameters":{"time.$":"$$.State.EnteredTime"},"Retry":[{"ErrorEquals":["Oops"],"MaxAttempts":1}],"Catch":[{"ErrorEquals":["Oops"],"ResultPath":null,"Next":"Second"}],"End":true},
            "Second":{"Type":"Task","Resource":"arn:aws:lambda:us-east-1:000000000000:function:Worker","Parameters":{"time.$":"$$.State.EnteredTime","count.$":"$$.State.RetryCount"},"End":true}
        }}),
        &json!({}),
    );
    let first = dispatch(&mut d, base, 0);
    let _ = fail_task(&mut d, &first, base, 0, "Oops");
    wake(&mut d, base, 1);
    let retry = dispatch(&mut d, base, 1);
    let _ = fail_task(&mut d, &retry, base, 1, "Oops");
    let next = dispatch(&mut d, base, 2);
    assert_ne!(
        first.parameters().value().get("time"),
        next.parameters().value().get("time")
    );
    assert_eq!(next.parameters().value().get("count"), Some(&json!(0)));
    assert_ne!(next.attempt(), retry.attempt());
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(512))]
    #[test]
    fn test_should_keep_one_terminal_outcome_under_arbitrary_control_order(ops in proptest::collection::vec((0_u8..5,0_u64..120),1..128)) {
        let (mut d,base)=driver(&task(&json!({"TimeoutSeconds":3,"Retry":[{"ErrorEquals":["Oops"],"MaxAttempts":3}],"Catch":[{"ErrorEquals":["States.ALL"],"Next":"Caught"}]})),&json!({"secret":"not-in-debug"}));
        let call=dispatch(&mut d,base,0);
        let mut terminal=None;
        let mut source=Source::default();
        for (op,time) in ops {
            let sample=now(base,time);
            let update=match op {
                0=>d.advance(sample,&mut source),
                1=>d.complete(call.attempt(),Ok(Payload::parse(b"true").unwrap()),sample,&mut source),
                2=>d.complete(call.attempt(),Err(TaskFailure::new("Oops",None).unwrap()),sample,&mut source),
                _=>d.stop(TaskFailure::new("Stopped",None).unwrap(),sample),
            };
            if let Some(outcome)=&terminal {
                proptest::prop_assert!(matches!(update.step,DriverStep::Settled(_)));
                proptest::prop_assert_eq!(update.cancellation(),None);
                proptest::prop_assert!(Arc::ptr_eq(outcome,d.outcome().unwrap()));
            } else if let Some(outcome)=d.outcome() { terminal=Some(outcome.clone()); }
            let debug=format!("{update:?} {d:?}");
            proptest::prop_assert!(!debug.contains("not-in-debug"));
        }
    }
}

#[test]
fn test_should_keep_typed_encoding_failure_internal_and_bypass_business_recovery() {
    use std::error::Error;
    let (mut d, base) = driver(
        &task(
            &json!({"Retry":[{"ErrorEquals":["States.ALL"]}],"Catch":[{"ErrorEquals":["States.ALL"],"Next":"Caught"}]}),
        ),
        &json!({}),
    );
    let encoding = serde_json::from_str::<Value>("{").unwrap_err();
    let step = d.evaluation_failure(
        AslError::Encoding(encoding),
        now(base, 0),
        &mut Source::default(),
        true,
    );
    assert!(matches!(step, DriverStep::Settled(_)));
    assert_eq!(failure(&d), "Rustack.InternalError");
    assert!(matches!(d.diagnostic(), Some(AslError::Encoding(_))));
    assert!(d.diagnostic().unwrap().source().is_some());
    assert_eq!(d.retry_count(), 0);
}

#[rstest]
#[case(262_142, true)]
#[case(262_143, false)]
#[case(262_145, false)]
fn test_should_apply_encoded_payload_limit_to_compiled_pass_string_result(
    #[case] length: usize,
    #[case] accepted: bool,
) {
    let (mut d, base) = driver(
        &json!({"StartAt":"P","States":{"P":{"Type":"Pass","Result":"x".repeat(length),"End":true}}}),
        &json!({}),
    );
    let _ = d.advance(now(base, 0), &mut Source::default());
    if accepted {
        assert_eq!(output(&d).as_str().unwrap().len(), length);
    } else {
        assert_eq!(failure(&d), "States.DataLimitExceeded");
    }
}
