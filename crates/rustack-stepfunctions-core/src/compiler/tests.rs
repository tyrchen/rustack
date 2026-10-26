use std::{error::Error, num::NonZeroU32};

use proptest::prelude::*;
use rstest::rstest;
use serde_json::{Map, Value, json};

use crate::{
    AslCompiler, AslError, CatchResult, CompileError, DiagnosticSeverity, FailureView, Integration,
    Jitter, StateView, TaskSupport, TaskTimeout, TenantScope, Transition,
};

type TestResult = Result<(), Box<dyn Error>>;
fn enabled() -> AslCompiler {
    AslCompiler::new(TenantScope::default(), TaskSupport::Lambda)
}
fn definition(state: &Value) -> Value {
    json!({"StartAt":"Run","States":{"Run":state}})
}
fn compile(value: &Value) -> Result<crate::Compilation, CompileError> {
    enabled().compile(&serde_json::to_vec(value).map_err(|_| {
        super::Location::root().error(super::Code::InvalidSchema, "test encoding failed")
    })?)
}
fn task() -> Value {
    json!({"Type":"Task","Resource":"arn:aws:states:::lambda:invoke","End":true})
}

#[test]
fn test_should_compile_all_six_instruction_forms_and_resolve_scoped_targets() -> TestResult {
    let value = json!({"StartAt":"Pass","TimeoutSeconds":60,"Version":"1.0","QueryLanguage":"JSONPath","States":{
        "Pass":{"Type":"Pass","Result":{"n":1},"Next":"Task"},
        "Task":{"Type":"Task","Resource":"arn:aws:lambda:us-east-1:000000000000:function:worker:alias","TimeoutSecondsPath":"$.timeout","Next":"Choice"},
        "Choice":{"Type":"Choice","Choices":[{"Variable":"$.n","NumericEquals":1,"Next":"Wait"}],"Default":"Fail"},
        "Wait":{"Type":"Wait","Seconds":0,"Next":"Success"},
        "Success":{"Type":"Succeed"},"Fail":{"Type":"Fail","ErrorPath":"States.Format('failure-{}', $.n)","Cause":"bounded"}
    }});
    let result = compile(&value)?;
    let machine = result.machine();
    assert_eq!(machine.state_count(), 6);
    assert_eq!(machine.timeout_seconds().map(NonZeroU32::get), Some(60));
    let StateView::Pass {
        result: literal,
        next: Transition::Next(next),
        ..
    } = machine.state(machine.start())?
    else {
        return Err("not Pass".into());
    };
    assert_eq!(literal, Some(&json!({"n":1})));
    assert_eq!(machine.state_name(next)?, "Task");
    let StateView::Task {
        resource,
        timeout,
        next: Transition::Next(next),
        recovery,
        ..
    } = machine.state(next)?
    else {
        return Err("not Task".into());
    };
    assert!(matches!(resource, Integration::DirectLambda(_)));
    assert!(matches!(timeout, TaskTimeout::Path(_)));
    assert!(recovery.retries().is_empty());
    let StateView::Choice {
        branches,
        default: Some(fail),
        ..
    } = machine.state(next)?
    else {
        return Err("not Choice".into());
    };
    let (_, wait) = branches.first().ok_or("missing choice")?;
    let StateView::Wait {
        next: Transition::Next(success),
        ..
    } = machine.state(*wait)?
    else {
        return Err("not Wait".into());
    };
    assert!(matches!(machine.state(success)?, StateView::Succeed { .. }));
    let StateView::Fail {
        error: FailureView::Dynamic(_),
        cause: FailureView::Literal("bounded"),
    } = machine.state(fail)?
    else {
        return Err("not Fail".into());
    };
    assert!(result.warnings().is_empty());
    Ok(())
}

#[test]
fn test_should_allow_loops_warn_unreachable_states_and_reject_unreachable_terminals() -> TestResult
{
    let value = json!({"StartAt":"Loop","States":{"Loop":{"Type":"Choice","Choices":[{"Variable":"$.again","BooleanEquals":true,"Next":"Loop"}],"Default":"Done"},"Done":{"Type":"Succeed"},"Unused":{"Type":"Fail"}}});
    let result = compile(&value)?;
    assert_eq!(result.warnings().len(), 1);
    let warning = result.warnings().first().ok_or("missing warning")?;
    assert_eq!(warning.severity(), DiagnosticSeverity::Warning);
    assert_eq!(warning.location(), "/States/Unused");
    let invalid = json!({"StartAt":"Loop","States":{"Loop":{"Type":"Pass","Next":"Loop"},"Unreachable":{"Type":"Succeed"}}});
    let error = compile(&invalid).err().ok_or("missing failure")?;
    assert_eq!(error.diagnostic().code(), "RUSTACK_NO_REACHABLE_TERMINAL");
    Ok(())
}

#[rstest]
#[case(json!({"Type":"Pass"}))]
#[case(json!({"Type":"Pass","End":false}))]
#[case(json!({"Type":"Pass","Next":"Run","End":true}))]
#[case(json!({"Type":"Pass","End":true,"Result":null,"Parameters":{}}))]
#[case(json!({"Type":"Pass","End":true,"Retry":[]}))]
#[case(json!({"Type":"Succeed","Next":"Run"}))]
#[case(json!({"Type":"Fail","End":true}))]
#[case(json!({"Type":"Fail","InputPath":"$"}))]
#[case(json!({"Type":"Fail","Error":"x","ErrorPath":"$.x"}))]
#[case(json!({"Type":"Fail","ErrorPath":"$[*]"}))]
#[case(json!({"Type":"Choice","Choices":[]}))]
#[case(json!({"Type":"Choice","Choices":[{"Variable":"$","IsNull":true,"Next":"Run"}],"End":true}))]
#[case(json!({"Type":"Wait","Seconds":1,"Timestamp":"1970-01-01T00:00:00Z","End":true}))]
#[case(json!({"Type":"Task","Resource":"arn:aws:states:::lambda:invoke","End":true,"TimeoutSeconds":1,"TimeoutSecondsPath":"$.n"}))]
#[case(json!({"Type":"Task","Resource":"arn:aws:states:::lambda:invoke","End":true,"Credentials":{}}))]
fn test_should_reject_invalid_state_fields_and_combinations(#[case] state: Value) {
    assert!(compile(&definition(&state)).is_err());
}

#[rstest]
#[case("arn:aws:states:::lambda:invoke.sync")]
#[case("arn:aws:states:::aws-sdk:lambda:invoke")]
#[case("arn:aws:sqs:::sendMessage.sync")]
#[case("arn:aws-cn:states:::lambda:invoke")]
#[case("arn:aws:lambda:us-west-2:000000000000:function:f")]
#[case("arn:aws:lambda:us-east-1:123456789012:function:f")]
#[case("arn:aws:lambda:us-east-1:000000000000:function:f:")]
#[case("arn:aws:lambda:us-east-1:000000000000:function:f:q:extra")]
#[case("arn:aws:lambda:us-east-1:000000000000:role:f")]
fn test_should_reject_nonallowlisted_or_cross_scope_resources(#[case] resource: &str) {
    let mut state = task();
    state["Resource"] = json!(resource);
    assert!(compile(&definition(&state)).is_err());
}

#[test]
fn test_should_distinguish_profile_and_dependency_rejection_and_static_parameter_errors()
-> TestResult {
    let data = serde_json::to_vec(&definition(&task()))?;
    let error = AslCompiler::default()
        .compile(&data)
        .err()
        .ok_or("missing dependency failure")?;
    assert_eq!(error.diagnostic().code(), "RUSTACK_DEPENDENCY_DISABLED");
    for resource in [
        "arn:aws:states:::states:startExecution.sync:2",
        "arn:aws:states:::lambda:invoke.waitForTaskToken",
    ] {
        let mut state = task();
        state["Resource"] = json!(resource);
        assert_eq!(
            compile(&definition(&state))
                .err()
                .ok_or("missing profile failure")?
                .diagnostic()
                .code(),
            "RUSTACK_PROFILE_UNAVAILABLE"
        );
    }
    for kind in ["Parallel", "Map"] {
        assert_eq!(
            compile(&definition(&json!({"Type":kind})))
                .err()
                .ok_or("missing state profile failure")?
                .diagnostic()
                .code(),
            "RUSTACK_PROFILE_UNAVAILABLE"
        );
    }
    for parameters in [
        json!({"InvocationType":"Event"}),
        json!({"Unknown":1}),
        json!({"FunctionName":"bad name"}),
        json!({"FunctionName":"arn:aws:lambda:us-west-2:000000000000:function:f"}),
        json!({"FunctionName":"arn:aws:lambda:us-east-1:000000000000:function:f:one","Qualifier":"two"}),
    ] {
        let mut state = task();
        state["Parameters"] = parameters;
        assert!(compile(&definition(&state)).is_err());
    }
    let mut state = task();
    state["Parameters"] = json!({"FunctionName":"arn:aws:lambda:us-east-1:000000000000:function:f:one","Qualifier":"one","InvocationType":"RequestResponse","Payload.$":"$"});
    compile(&definition(&state))?;
    Ok(())
}

#[test]
fn test_should_preserve_recovery_defaults_zero_attempts_and_same_scope_catch_targets() -> TestResult
{
    let mut state = task();
    state["Retry"] = json!([{"ErrorEquals":["Example"]},{"ErrorEquals":["States.ALL"],"MaxAttempts":0,"IntervalSeconds":2,"BackoffRate":3,"MaxDelaySeconds":4,"JitterStrategy":"FULL"}]);
    state["Catch"] = json!([{"ErrorEquals":["Example"],"Next":"Fail","ResultPath":"$.error"},{"ErrorEquals":["States.ALL"],"Next":"Fail","ResultPath":null}]);
    let result = compile(&json!({"StartAt":"Run","States":{"Run":state,"Fail":{"Type":"Fail"}}}))?;
    let StateView::Task { recovery, .. } = result.machine().state(result.machine().start())? else {
        return Err("not Task".into());
    };
    let first = recovery.retries().first().ok_or("missing retry")?;
    assert_eq!(first.max_attempts(), 3);
    assert_eq!(first.timing().interval_seconds().get(), 1);
    assert_eq!(first.timing().backoff_rate(), 2.0);
    assert_eq!(first.timing().jitter(), Jitter::None);
    let second = recovery.retries().get(1).ok_or("missing retry")?;
    assert_eq!(second.max_attempts(), 0);
    assert_eq!(
        second.timing().max_delay_seconds().map(NonZeroU32::get),
        Some(4)
    );
    assert_eq!(second.timing().jitter(), Jitter::Full);
    assert!(second.matches("Example"));
    assert!(!second.matches("States.Runtime"));
    assert!(!second.matches("States.DataLimitExceeded"));
    let catch = recovery.catches().first().ok_or("missing catch")?;
    assert!(matches!(catch.result_path(), CatchResult::Path(_)));
    assert_eq!(result.machine().state_name(catch.next())?, "Fail");
    assert!(matches!(
        recovery
            .catches()
            .get(1)
            .ok_or("missing catch")?
            .result_path(),
        CatchResult::Discard
    ));
    Ok(())
}

#[rstest]
#[case(json!([{"ErrorEquals":[]}]))]
#[case(json!([{"ErrorEquals":["x","x"]}]))]
#[case(json!([{"ErrorEquals":["States.ALL","x"]}]))]
#[case(json!([{"ErrorEquals":["States.ALL"]},{"ErrorEquals":["x"]}]))]
#[case(json!([{"ErrorEquals":["x"],"MaxAttempts":-1}]))]
#[case(json!([{"ErrorEquals":["x"],"MaxAttempts":100_000_000}]))]
#[case(json!([{"ErrorEquals":["x"],"IntervalSeconds":0}]))]
#[case(json!([{"ErrorEquals":["x"],"BackoffRate":0.5}]))]
#[case(json!([{"ErrorEquals":["x"],"MaxDelaySeconds":31_622_401}]))]
#[case(json!([{"ErrorEquals":["x"],"JitterStrategy":"random"}]))]
fn test_should_reject_invalid_retry_policy_fields_and_ranges(#[case] retry: Value) {
    let mut state = task();
    state["Retry"] = retry;
    assert!(compile(&definition(&state)).is_err());
}

#[test]
fn test_should_keep_diagnostics_sources_and_definition_data_redacted() -> TestResult {
    let input=br#" {"StartAt":"a/~","States":{"a/~":{"Type":"Pass","Parameters":{"bad.$":"invalid private text"},"End":true}}} "#;
    let error = enabled().compile(input).err().ok_or("missing error")?;
    assert_eq!(error.diagnostic().location(), "/States/a~1~0/Parameters");
    assert!(error.source().is_some());
    assert_eq!(format!("{error:?}"), "ASL definition compilation failed");
    assert_eq!(
        format!("{:?}", error.diagnostic()),
        "Diagnostic(<redacted>)"
    );
    let valid = br#" { "StartAt":"Done", "States":{"Done":{"Type":"Succeed"}} } "#;
    let compilation = enabled().compile(valid)?;
    assert_eq!(compilation.machine().original_bytes(), valid);
    assert_eq!(
        format!("{:?}", compilation.machine()),
        "CompiledMachine(<redacted>)"
    );
    assert!(matches!(
        compilation.machine().state(super::StateId(1000)),
        Err(AslError::InvalidStateId)
    ));
    assert!(
        enabled()
            .compile(br#"{"StartAt":"D","States":{"D":{"Type":"Succeed"},"D":{"Type":"Fail"}}}"#)
            .is_err()
    );
    Ok(())
}

#[test]
fn test_should_enforce_exact_state_name_and_state_count_limits() -> TestResult {
    let name = "中".repeat(42) + "ab";
    assert_eq!(name.len(), 128);
    compile(&json!({"StartAt":name,"States":{name:{"Type":"Succeed"}}}))?;
    let invalid = "x".repeat(129);
    assert!(compile(&json!({"StartAt":invalid,"States":{invalid:{"Type":"Succeed"}}})).is_err());
    let mut states = Map::new();
    for index in 0..1000 {
        states.insert(format!("s{index:04}"), json!({"Type":"Succeed"}));
    }
    let result = compile(&json!({"StartAt":"s0000","States":states}))?;
    assert_eq!(result.machine().state_count(), 1000);
    assert_eq!(result.warnings().len(), 999);
    states.insert("extra".into(), json!({"Type":"Succeed"}));
    assert!(compile(&json!({"StartAt":"s0000","States":states})).is_err());
    Ok(())
}

#[test]
fn test_should_enforce_exact_definition_bytes_depth_and_choice_count() -> TestResult {
    let mut value = definition(&json!({"Type":"Succeed"}));
    value["Comment"] = json!("");
    let fixed = serde_json::to_vec(&value)?.len();
    value["Comment"] = json!("x".repeat(1024 * 1024 - fixed));
    let mut exact = serde_json::to_vec(&value)?;
    assert_eq!(exact.len(), 1024 * 1024);
    enabled().compile(&exact)?;
    exact.push(b' ');
    assert!(enabled().compile(&exact).is_err());
    let mut nested = json!(0);
    for _ in 0..61 {
        nested = json!([nested]);
    }
    compile(&definition(
        &json!({"Type":"Pass","Result":nested,"End":true}),
    ))?;
    assert!(
        compile(&definition(
            &json!({"Type":"Pass","Result":[nested],"End":true})
        ))
        .is_err()
    );
    let rule = json!({"Variable":"$","IsPresent":true,"Next":"Done"});
    let mut machine = json!({"StartAt":"Run","States":{"Run":{"Type":"Choice","Choices":vec![rule.clone();128]},"Done":{"Type":"Succeed"}}});
    compile(&machine)?;
    machine["States"]["Run"]["Choices"] = json!(vec![rule; 129]);
    assert!(compile(&machine).is_err());
    Ok(())
}

#[test]
fn test_should_enforce_exact_aggregate_compiled_node_limit_across_states() -> TestResult {
    let full = format!("States.Array({})", vec!["0"; 128].join(","));
    let mut states = Map::new();
    for index in 0..492 {
        states.insert(
            format!("s{index:04}"),
            json!({"Type":"Pass","Parameters":{"x.$":full},"End":true}),
        );
    }
    let partial = format!("States.Array({})", vec!["0"; 95].join(","));
    states.insert(
        "z".into(),
        json!({"Type":"Pass","Parameters":{"x.$":partial},"End":true}),
    );
    let exact = json!({"StartAt":"s0000","States":states});
    let accepted = compile(&exact)?;
    assert_eq!(accepted.machine().state_count(), 493);
    let mut over = exact;
    over["States"]["z"]["Parameters"]["x.$"] =
        json!(format!("States.Array({})", vec!["0"; 96].join(",")));
    assert_eq!(
        compile(&over)
            .err()
            .ok_or("missing aggregate limit")?
            .diagnostic()
            .code(),
        "RUSTACK_LIMIT_EXCEEDED"
    );
    Ok(())
}

#[test]
fn test_should_validate_scope_and_recovery_target_and_fatal_matchers() -> TestResult {
    for (partition, region, account) in [
        ("foreign", "us-east-1", "000000000000"),
        ("aws", "Bad Region", "000000000000"),
        ("aws", "us-east-1", "short"),
    ] {
        assert!(TenantScope::new(partition, region, account).is_err());
    }
    let compiler = AslCompiler::new(
        TenantScope::new("aws-cn", "cn-north-1", "123456789012")?,
        TaskSupport::Lambda,
    );
    compiler.compile(&serde_json::to_vec(&definition(&json!({"Type":"Task","Resource":"arn:aws-cn:lambda:cn-north-1:123456789012:function:f:$LATEST","End":true})))?)?;
    let mut state = task();
    state["Catch"] = json!([{"ErrorEquals":["x"],"Next":"Missing"}]);
    let error = compile(&definition(&state))
        .err()
        .ok_or("missing catch failure")?;
    assert_eq!(error.diagnostic().location(), "/States/Run/Catch/0/Next");
    state["Catch"] = json!([]);
    state["Retry"] =
        json!([{"ErrorEquals":["States.TaskFailed"]},{"ErrorEquals":["States.DataLimitExceeded"]}]);
    let result = compile(&definition(&state))?;
    let StateView::Task { recovery, .. } = result.machine().state(result.machine().start())? else {
        return Err("missing task".into());
    };
    let wildcard = recovery.retries().first().ok_or("missing wildcard")?;
    assert!(wildcard.matches("Other"));
    assert!(!wildcard.matches("States.Timeout"));
    assert!(!wildcard.matches("States.Runtime"));
    assert!(!wildcard.matches("States.DataLimitExceeded"));
    assert!(
        recovery
            .retries()
            .get(1)
            .ok_or("missing explicit")?
            .matches("States.DataLimitExceeded")
    );
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]
    #[test]
    fn test_should_reject_arbitrary_definition_bytes_without_panicking(bytes in prop::collection::vec(any::<u8>(),0..4096)) {let _=AslCompiler::default().compile(&bytes);}
    #[test]
    fn test_should_compile_bounded_linear_graphs_with_correct_indexed_edges(count in 1_usize..80) {
        let mut states=Map::new();for index in 0..count {let state=if index+1==count {json!({"Type":"Succeed"})} else {json!({"Type":"Pass","Next":format!("s{}",index+1)})};states.insert(format!("s{index}"),state);}
        let c=compile(&json!({"StartAt":"s0","States":states})).map_err(|e|TestCaseError::fail(e.to_string()))?;
        let mut id=c.machine().start();for index in 0..count {prop_assert_eq!(c.machine().state_name(id).map_err(|e|TestCaseError::fail(e.to_string()))?,format!("s{index}"));match c.machine().state(id).map_err(|e|TestCaseError::fail(e.to_string()))? {StateView::Pass {next:Transition::Next(next),..}=>id=next,StateView::Succeed {..}=>prop_assert_eq!(index+1,count),_=>return Err(TestCaseError::fail("unexpected state"))}}
    }
}
