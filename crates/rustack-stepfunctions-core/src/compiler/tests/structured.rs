//! Structured compiler boundary, lexical ownership and normalization tests.

use std::{sync::Arc, time::Instant};

use chrono::Utc;
use proptest::prelude::*;
use rstest::rstest;
use serde_json::{Map, Value, json};

use super::{TestResult, definition};
use crate::{
    AslCompiler, AslError, ClockSample, Compilation, CompileError, EvaluationContext,
    ExecutionDriver, ExecutionLimits, MapConcurrency, Payload, StateSupport, StateView,
    TaskSupport, TenantScope, Transition,
};

fn compiler() -> AslCompiler {
    AslCompiler::new(TenantScope::default(), TaskSupport::Lambda)
        .with_state_support(StateSupport::Structured)
}
fn compile(value: &Value) -> Result<Compilation, CompileError> {
    compiler().compile(&serde_json::to_vec(value).map_err(|_| {
        super::super::Location::root()
            .error(super::super::Code::InvalidSchema, "test encoding failed")
    })?)
}
fn leaf() -> Value {
    definition(&json!({"Type":"Succeed"}))
}
fn parallel(branches: Vec<Value>) -> Value {
    let mut state = json!({"Type":"Parallel","End":true});
    state["Branches"] = Value::Array(branches);
    definition(&state)
}
fn map() -> Value {
    definition(&json!({"Type":"Map","ItemProcessor":leaf(),"End":true}))
}
fn error_at(value: &Value, code: &str, location: &str) -> TestResult {
    let error = compile(value).err().ok_or("expected compile rejection")?;
    assert_eq!(error.diagnostic().code(), code);
    assert_eq!(error.diagnostic().location(), location);
    Ok(())
}

#[test]
fn test_should_resolve_nonlexical_source_order_under_unified_json_features() -> TestResult {
    let result = AslCompiler::default().compile(br#"{"StartAt":"Zulu","States":{"Zulu":{"Type":"Pass","Next":"Alpha"},"Alpha":{"Type":"Succeed"}}}"#)?;
    let machine = result.machine();
    assert_eq!(machine.state_name(machine.start())?, "Zulu");
    let StateView::Pass {
        next: Transition::Next(next),
        ..
    } = machine.state(machine.start())?
    else {
        return Err("incorrect entry instruction".into());
    };
    assert_eq!(machine.state_name(next)?, "Alpha");
    assert!(matches!(machine.state(next)?, StateView::Succeed { .. }));
    let result = compiler().compile(br#"{"StartAt":"Run","States":{"Run":{"Type":"Parallel","Branches":[{"StartAt":"Zulu","States":{"Zulu":{"Type":"Pass","Next":"Alpha"},"Alpha":{"Type":"Succeed"}}}],"End":true}}}"#)?;
    let StateView::Parallel { branches, .. } = result.machine().state(result.machine().start())?
    else {
        return Err("expected Parallel".into());
    };
    let branch = branches.first().ok_or("missing branch")?;
    assert_eq!(branch.state_name(branch.start())?, "Zulu");
    let StateView::Pass {
        next: Transition::Next(next),
        ..
    } = branch.state(branch.start())?
    else {
        return Err("incorrect branch entry".into());
    };
    assert_eq!(branch.state_name(next)?, "Alpha");
    Ok(())
}

#[test]
fn test_should_share_root_source_and_tenant_across_independent_lexical_indexes() -> TestResult {
    let value = parallel(vec![leaf(), map()]);
    let mut source = b" \n".to_vec();
    source.extend(serde_json::to_vec(&value)?);
    source.extend(b"\n ");
    let result = compiler().compile(&source)?;
    let machine = result.machine();
    assert_eq!(machine.original_bytes(), source);
    assert_eq!(machine.state_count(), 1);
    assert!(machine.requires_structured_execution());
    let StateView::Parallel { branches, .. } = machine.state(machine.start())? else {
        return Err("expected Parallel".into());
    };
    assert_eq!(branches.len(), 2);
    for branch in branches {
        assert_eq!(
            branch.original_bytes().as_ptr(),
            machine.original_bytes().as_ptr()
        );
        assert_eq!(branch.scope().region(), machine.scope().region());
        assert_eq!(branch.state_name(branch.start())?, "Run");
    }
    let first = branches.first().ok_or("missing branch")?;
    assert!(matches!(
        first.state(first.start())?,
        StateView::Succeed { .. }
    ));
    let second = branches.get(1).ok_or("missing Map branch")?;
    let StateView::Map { processor, .. } = second.state(second.start())? else {
        return Err("expected Map".into());
    };
    assert_eq!(
        processor.original_bytes().as_ptr(),
        machine.original_bytes().as_ptr()
    );
    assert_eq!(processor.state_name(processor.start())?, "Run");
    assert!(!processor.requires_structured_execution());
    assert!(matches!(
        processor.state(processor.start())?,
        StateView::Succeed { .. }
    ));
    Ok(())
}

#[rstest]
#[case(json!({"Type":"Pass","Next":"Outer"}), "/Next")]
#[case(json!({"Type":"Choice","Choices":[{"Variable":"$","IsNull":true,"Next":"Outer"}]}), "/Choices/0/Next")]
#[case(json!({"Type":"Task","Resource":"arn:aws:states:::lambda:invoke","Catch":[{"ErrorEquals":["BusinessError"],"Next":"Outer"}],"End":true}), "/Catch/0/Next")]
fn test_should_reject_nested_transitions_to_parent_scope(
    #[case] state: Value,
    #[case] suffix: &str,
) -> TestResult {
    let mut value = parallel(vec![definition(&state)]);
    value["States"]["Outer"] = json!({"Type":"Succeed"});
    error_at(
        &value,
        "RUSTACK_MISSING_TRANSITION_TARGET",
        &format!("/States/Run/Branches/0/States/Run{suffix}"),
    )
}

#[test]
fn test_should_reject_start_parent_sibling_and_processor_scope_escapes() -> TestResult {
    let mut value = parallel(vec![
        leaf(),
        json!({"StartAt":"Sibling","States":{"Sibling":{"Type":"Succeed"}}}),
    ]);
    value["States"]["Run"]["Branches"][0]["StartAt"] = json!("Sibling");
    error_at(
        &value,
        "RUSTACK_MISSING_TRANSITION_TARGET",
        "/States/Run/Branches/0/StartAt",
    )?;
    let mut value = map();
    value["States"]["Outer"] = json!({"Type":"Succeed"});
    value["States"]["Run"]["ItemProcessor"]["States"]["Run"] =
        json!({"Type":"Pass","Next":"Outer"});
    error_at(
        &value,
        "RUSTACK_MISSING_TRANSITION_TARGET",
        "/States/Run/ItemProcessor/States/Run/Next",
    )?;
    let value = json!({"StartAt":"Child","States":{"Run":{"Type":"Parallel","Branches":[{"StartAt":"Child","States":{"Child":{"Type":"Succeed"}}}],"End":true}}});
    error_at(&value, "RUSTACK_MISSING_TRANSITION_TARGET", "/StartAt")
}

#[test]
fn test_should_validate_all_nested_graphs_and_report_root_relative_escaped_warnings() -> TestResult
{
    let branch =
        json!({"StartAt":"Run","States":{"Run":{"Type":"Succeed"},"unused~/":{"Type":"Fail"}}});
    let result = compile(&parallel(vec![branch]))?;
    let warning = result.warnings().first().ok_or("missing nested warning")?;
    assert_eq!(
        warning.location(),
        "/States/Run/Branches/0/States/unused~0~1"
    );
    let cycle = definition(&json!({"Type":"Pass","Next":"Run"}));
    error_at(
        &parallel(vec![cycle]),
        "RUSTACK_NO_REACHABLE_TERMINAL",
        "/States/Run/Branches/0/States",
    )?;
    let invalid = json!({"StartAt":"Done","States":{"Done":{"Type":"Succeed"},"Unused":{"Type":"Parallel","Branches":[{"StartAt":"Missing","States":{"Run":{"Type":"Succeed"}}}],"End":true}}});
    error_at(
        &invalid,
        "RUSTACK_MISSING_TRANSITION_TARGET",
        "/States/Unused/Branches/0/StartAt",
    )
}

#[rstest]
#[case("ItemProcessor", "ItemSelector")]
#[case("ItemProcessor", "Parameters")]
#[case("Iterator", "ItemSelector")]
#[case("Iterator", "Parameters")]
fn test_should_normalize_legacy_and_current_map_fields_independently(
    #[case] processor_key: &str,
    #[case] selector_key: &str,
) -> TestResult {
    let mut state = json!({"Type":"Map","InputPath":"$.batch","ItemsPath":"$.items","ResultPath":"$.result","End":true});
    state[processor_key] = leaf();
    state[selector_key] =
        json!({"item.$":"$$.Map.Item.Value","index.$":"$$.Map.Item.Index","keep.$":"$.keep"});
    let result = compile(&definition(&state))?;
    let machine = result.machine();
    let StateView::Map {
        flow,
        items,
        selector: Some(selector),
        concurrency,
        ..
    } = machine.state(machine.start())?
    else {
        return Err("expected normalized Map".into());
    };
    let raw = Payload::from_value(&json!({"batch":{"items":[7,9],"keep":true},"outer":1}))?;
    let context = EvaluationContext::from(&Payload::from_value(
        &json!({"Map":{"Item":{"Index":1,"Value":9,"Source":"STATE_DATA"}}}),
    )?);
    let effective = flow.prepare(&raw, &context)?;
    assert_eq!(effective.value(), &json!({"items":[7,9],"keep":true}));
    assert_eq!(items.lookup(&effective, &context), Some(&json!([7, 9])));
    assert_eq!(
        selector.evaluate(&effective, &context)?.value(),
        &json!({"item":9,"index":1,"keep":true})
    );
    assert!(matches!(concurrency, MapConcurrency::Limit(limit) if limit.get() == 40));
    assert_eq!(
        flow.finish(&raw, &Payload::parse(b"[8,10]")?, &context)?
            .value(),
        &json!({"batch":{"items":[7,9],"keep":true},"outer":1,"result":[8,10]})
    );
    Ok(())
}

#[rstest]
#[case(json!({"ItemSelector":{},"Parameters":{}}), "/ItemSelector")]
#[case(json!({"Iterator":{"StartAt":"Run","States":{"Run":{"Type":"Succeed"}}}}), "/ItemProcessor")]
#[case(json!({"ItemSelector":null}), "/ItemSelector")]
#[case(json!({"ItemsPath":"$[*]"}), "/ItemsPath")]
#[case(json!({"MaxConcurrencyPath":"$.limits[*]"}), "/MaxConcurrencyPath")]
#[case(json!({"MaxConcurrency":41}), "/MaxConcurrency")]
#[case(json!({"MaxConcurrency":-1}), "/MaxConcurrency")]
#[case(json!({"MaxConcurrency":1.5}), "/MaxConcurrency")]
#[case(json!({"MaxConcurrency":null}), "/MaxConcurrency")]
#[case(json!({"MaxConcurrency":1,"MaxConcurrencyPath":"$.n"}), "/MaxConcurrencyPath")]
fn test_should_reject_map_conflicts_and_invalid_selectors_or_concurrency(
    #[case] additions: Value,
    #[case] suffix: &str,
) -> TestResult {
    let mut value = map();
    let state = value
        .get_mut("States")
        .and_then(|v| v.get_mut("Run"))
        .and_then(Value::as_object_mut)
        .ok_or("missing state")?;
    state.extend(additions.as_object().ok_or("missing additions")?.clone());
    let error = compile(&value).err().ok_or("expected map rejection")?;
    assert_eq!(
        error.diagnostic().location(),
        format!("/States/Run{suffix}")
    );
    Ok(())
}

#[rstest]
#[case(None, 40)]
#[case(Some(0), 40)]
#[case(Some(1), 1)]
#[case(Some(40), 40)]
fn test_should_normalize_static_map_concurrency(
    #[case] value: Option<u64>,
    #[case] expected: u8,
) -> TestResult {
    let mut definition = map();
    if let Some(value) = value {
        definition["States"]["Run"]["MaxConcurrency"] = json!(value);
    }
    let result = compile(&definition)?;
    let StateView::Map {
        items,
        selector,
        concurrency,
        ..
    } = result.machine().state(result.machine().start())?
    else {
        return Err("expected Map".into());
    };
    assert!(matches!(concurrency, MapConcurrency::Limit(limit) if limit.get() == expected));
    assert!(selector.is_none());
    let input = Payload::parse(b"[null,1]")?;
    assert_eq!(
        items.lookup(&input, &EvaluationContext::from(&input)),
        Some(input.value())
    );
    Ok(())
}

#[test]
fn test_should_compile_dynamic_concurrency_without_evaluating_definition_data() -> TestResult {
    let mut value = map();
    value["States"]["Run"]["MaxConcurrencyPath"] = json!("$.count");
    let result = compile(&value)?;
    let StateView::Map {
        concurrency: MapConcurrency::Path(path),
        ..
    } = result.machine().state(result.machine().start())?
    else {
        return Err("expected dynamic Map".into());
    };
    let input = Payload::parse(br#"{"count":3}"#)?;
    assert_eq!(
        path.lookup(&input, &EvaluationContext::from(&input)),
        Some(&json!(3))
    );
    Ok(())
}

#[rstest]
#[case(json!({}), true)]
#[case(json!({"Mode":"INLINE"}), true)]
#[case(json!({"Mode":"DISTRIBUTED"}), false)]
#[case(json!({"Mode":null}), false)]
#[case(json!({"ExecutionType":"STANDARD"}), false)]
#[case(json!(null), false)]
fn test_should_validate_inline_processor_configuration(
    #[case] config: Value,
    #[case] accepted: bool,
    #[values("ItemProcessor", "Iterator")] spelling: &str,
) {
    let mut value = map();
    let mut processor = value["States"]["Run"]
        .as_object_mut()
        .unwrap()
        .remove("ItemProcessor")
        .unwrap();
    processor["ProcessorConfig"] = config;
    value["States"]["Run"][spelling] = processor;
    assert_eq!(compile(&value).is_ok(), accepted);
}

#[rstest]
#[case("TimeoutSeconds", json!(1))]
#[case("Version", json!("1.0"))]
#[case("ProcessorConfig", json!({"Mode":"INLINE"}))]
fn test_should_reject_root_only_or_processor_only_fields_in_branches(
    #[case] key: &str,
    #[case] field: Value,
) -> TestResult {
    let mut branch = leaf();
    branch[key] = field;
    error_at(
        &parallel(vec![branch]),
        "RUSTACK_UNSUPPORTED_FIELD",
        &format!("/States/Run/Branches/0/{key}"),
    )
}

#[rstest]
#[case(json!([]))]
#[case(json!([null]))]
#[case(json!(null))]
fn test_should_reject_invalid_parallel_branches(#[case] branches: Value) {
    assert!(
        compile(&definition(
            &json!({"Type":"Parallel","Branches":branches,"End":true})
        ))
        .is_err()
    );
}

#[test]
fn test_should_resolve_enclosing_recovery_and_continuation_without_leaking_branch_targets()
-> TestResult {
    for mut state in [
        json!({"Type":"Parallel","Branches":[leaf()]}),
        json!({"Type":"Map","ItemProcessor":leaf()}),
    ] {
        state["Next"] = json!("Done");
        state["Retry"] = json!([{"ErrorEquals":["BusinessError"],"MaxAttempts":2}]);
        state["Catch"] = json!([{"ErrorEquals":["States.ALL"],"Next":"Recovered"}]);
        let value = json!({"StartAt":"Run","States":{"Run":state,"Done":{"Type":"Succeed"},"Recovered":{"Type":"Succeed"}}});
        let result = compile(&value)?;
        let machine = result.machine();
        let (StateView::Parallel { recovery, next, .. } | StateView::Map { recovery, next, .. }) =
            machine.state(machine.start())?
        else {
            return Err("expected structured state".into());
        };
        let Transition::Next(next) = next else {
            return Err("missing continuation".into());
        };
        assert_eq!(machine.state_name(next)?, "Done");
        assert_eq!(
            machine.state_name(recovery.catches().first().ok_or("missing catch")?.next())?,
            "Recovered"
        );
        assert_eq!(
            recovery
                .retries()
                .first()
                .ok_or("missing retry")?
                .max_attempts(),
            2
        );
        assert!(result.warnings().is_empty());
    }
    Ok(())
}

#[test]
fn test_should_enforce_shared_state_count_and_nested_scope_depth() -> TestResult {
    let mut states = Map::new();
    for index in 0..999 {
        states.insert(format!("s{index}"), json!({"Type":"Succeed"}));
    }
    let mut value = parallel(vec![json!({"StartAt":"s0","States":states})]);
    compile(&value)?;
    value["States"]["Run"]["Branches"][0]["States"]["extra"] = json!({"Type":"Succeed"});
    error_at(
        &value,
        "RUSTACK_LIMIT_EXCEEDED",
        "/States/Run/Branches/0/States",
    )?;
    let mut value = leaf();
    for _ in 0..16 {
        value = definition(&json!({"Type":"Map","ItemProcessor":value,"End":true}));
    }
    compile(&value)?;
    let over = definition(&json!({"Type":"Map","ItemProcessor":value,"End":true}));
    let error = compile(&over).err().ok_or("expected depth rejection")?;
    assert_eq!(error.diagnostic().code(), "RUSTACK_LIMIT_EXCEEDED");
    assert!(error.diagnostic().location().ends_with("/ItemProcessor"));
    let mut json_depth = leaf();
    for _ in 0..16 {
        json_depth = parallel(vec![json_depth]);
    }
    error_at(&json_depth, "RUSTACK_LIMIT_EXCEEDED", "")?;
    Ok(())
}

fn heavy_branch(full_states: usize, partial_arguments: usize) -> Value {
    let full = format!("States.Array({})", vec!["0"; 128].join(","));
    let mut states = Map::new();
    for index in 0..full_states {
        states.insert(
            format!("s{index:04}"),
            json!({"Type":"Pass","Parameters":{"x.$":full},"End":true}),
        );
    }
    let partial = format!("States.Array({})", vec!["0"; partial_arguments].join(","));
    states.insert(
        "z".into(),
        json!({"Type":"Pass","Parameters":{"x.$":partial},"End":true}),
    );
    json!({"StartAt":"s0000","States":states})
}

#[test]
fn test_should_charge_nested_scope_and_item_selector_units_to_one_exact_budget() -> TestResult {
    // 492*133 +98 (93 args +5) +2 Parallel units =65,536.
    compile(&parallel(vec![heavy_branch(492, 93)]))?;
    let over = parallel(vec![heavy_branch(492, 94)]);
    assert_eq!(
        compile(&over)
            .err()
            .ok_or("missing unit rejection")?
            .diagnostic()
            .code(),
        "RUSTACK_LIMIT_EXCEEDED"
    );
    let mut map = map();
    map["States"]["Run"]["ItemSelector"] =
        json!({"x.$":format!("States.Array({})", vec!["0";128].join(","))});
    // 491*133 +96 +135 (Map +selector +processor) +2 =65,536.
    compile(&parallel(vec![heavy_branch(491, 91), map.clone()]))?;
    assert_eq!(
        compile(&parallel(vec![heavy_branch(491, 92), map]))
            .err()
            .ok_or("missing selector unit rejection")?
            .diagnostic()
            .code(),
        "RUSTACK_LIMIT_EXCEEDED"
    );
    Ok(())
}

#[test]
fn test_should_keep_default_compiler_and_sequential_driver_admission_closed() -> TestResult {
    for value in [parallel(vec![leaf()]), map()] {
        let source = serde_json::to_vec(&value)?;
        let error = AslCompiler::default()
            .compile(&source)
            .err()
            .ok_or("default compiler accepted structured states")?;
        assert_eq!(error.diagnostic().code(), "RUSTACK_PROFILE_UNAVAILABLE");
        let result = compile(&value)?;
        let input = Payload::parse(b"{}")?;
        let driver = ExecutionDriver::new(
            result.machine().clone(),
            input.clone(),
            EvaluationContext::from(&input),
            ClockSample::new(Instant::now(), Utc::now()),
            ExecutionLimits::default(),
        );
        assert!(matches!(driver, Err(AslError::StructuredExecutionRequired)));
    }
    let value = parallel(vec![definition(
        &json!({"Type":"Task","Resource":"arn:aws:states:::lambda:invoke","End":true}),
    )]);
    assert!(compile(&value)?.machine().uses_lambda());
    let error = AslCompiler::default()
        .with_state_support(StateSupport::Structured)
        .compile(&serde_json::to_vec(&value)?)
        .err()
        .ok_or("missing nested Lambda dependency rejection")?;
    assert_eq!(error.diagnostic().code(), "RUSTACK_DEPENDENCY_DISABLED");
    assert_eq!(
        error.diagnostic().location(),
        "/States/Run/Branches/0/States/Run/Resource"
    );
    Ok(())
}

#[test]
fn test_should_preflight_duplicate_keys_and_redact_nested_source_details() -> TestResult {
    let bytes = br#"{"StartAt":"Run","States":{"Run":{"Type":"Map","ItemProcessor":{"StartAt":"Run","States":{"Run":{"Type":"Succeed","Comment":"secret-a","Comment":"secret-b"}}},"End":true}}}"#;
    let error = compiler()
        .compile(bytes)
        .err()
        .ok_or("accepted duplicate nested keys")?;
    assert!(!format!("{error:?} {error}").contains("secret-"));
    let value = parallel(vec![definition(
        &json!({"Type":"Fail","Cause":"secret-value"}),
    )]);
    let result = compile(&value)?;
    assert!(!format!("{result:?} {:?}", result.machine()).contains("secret-value"));
    let StateView::Parallel { branches, .. } = result.machine().state(result.machine().start())?
    else {
        return Err("expected Parallel".into());
    };
    let branch = branches.first().ok_or("missing branch")?;
    assert!(!format!("{:?} {:?}", branch, branch.state(branch.start())?).contains("secret-value"));
    assert_eq!(Arc::strong_count(branch), 1);
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn test_should_preserve_nested_lexical_name_reuse(depth in 0_usize..17) {
        let mut value = leaf();
        for level in 0..depth { value = if level % 2 == 0 { parallel(vec![value]) } else { definition(&json!({"Type":"Map","ItemProcessor":value,"End":true})) }; }
        let result = compile(&value).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let mut machine = result.machine().clone();
        for _ in 0..depth {
            prop_assert_eq!(machine.state_name(machine.start()).map_err(|e| TestCaseError::fail(e.to_string()))?, "Run");
            let branch = match machine.state(machine.start()).map_err(|e| TestCaseError::fail(e.to_string()))? {
                StateView::Parallel { branches, .. } => branches.first().ok_or_else(|| TestCaseError::fail("missing branch"))?.clone(),
                StateView::Map { processor, .. } => processor.clone(),
                _ => return Err(TestCaseError::fail("expected structured state")),
            };
            prop_assert_eq!(branch.original_bytes().as_ptr(), machine.original_bytes().as_ptr());
            machine = branch;
        }
        prop_assert!(matches!(machine.state(machine.start()), Ok(StateView::Succeed { .. })), "expected terminal Succeed");
    }
}
