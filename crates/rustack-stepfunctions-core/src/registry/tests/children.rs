//! Child port contracts through the actual registry/runtime, independent of ASL routing.

use std::{
    future::{Future, poll_fn},
    task::Poll,
};

use rustack_core::json::JsonError;

use super::*;
use crate::{
    AslError, ChildError, ChildExecution, ChildPattern, ChildStartParameters, ChildSubmission,
};

type TestResult = Result<(), Box<dyn Error>>;
fn wait() -> serde_json::Value {
    json!({"StartAt":"W","States":{"W":{"Type":"Wait","Seconds":60,"End":true}}})
}
async fn publish(
    reg: &Registry,
    name: &str,
    definition: serde_json::Value,
) -> Result<StateMachineArn, Box<dyn Error>> {
    let prepared = PreparedDefinition::new(
        AslCompiler::new(TenantScope::default(), TaskSupport::Lambda)
            .compile(&serde_json::to_vec(&definition)?)?
            .machine()
            .clone(),
    );
    let RegistryResult::Machine(machine) = result(
        reg,
        RegistryCommand::Create(name.parse()?, prepared, role(), Tags::default()),
    )
    .await?
    else {
        return Err("machine".into());
    };
    Ok(machine.arn().clone())
}
fn parameters(
    machine: &StateMachineArn,
    name: Option<&str>,
    input: serde_json::Value,
) -> Result<ChildStartParameters, Box<dyn Error>> {
    let mut value = json!({"StateMachineArn":machine.as_str()});
    value
        .as_object_mut()
        .ok_or("parameters")?
        .insert("Input".into(), input);
    if let Some(name) = name {
        value["Name"] = json!(name);
    }
    Ok(ChildStartParameters::parse(
        &TenantScope::default(),
        &Payload::from_value(&value)?,
    )?)
}
async fn accepted(mut submission: ChildSubmission) -> Result<ChildExecution, Box<dyn Error>> {
    submission.admitted(deadline()).await?;
    Ok(submission.into_execution()?)
}
async fn make_parent(reg: &Registry) -> Result<ExecutionSnapshot, Box<dyn Error>> {
    machine(reg, 1, true).await;
    Ok(start(reg, "parent", "{}").await?)
}
async fn committed_child(
    reg: &Registry,
    machine: &StateMachineArn,
) -> Result<ExecutionSnapshot, Box<dyn Error>> {
    timeout(Duration::from_secs(5), async {
        loop {
            let RegistryResult::Executions(records) =
                result(reg, RegistryCommand::Executions(machine.clone())).await?
            else {
                return Err("records".into());
            };
            if let Some(child) = records.first() {
                return Ok(child.clone());
            }
            tokio::task::yield_now().await;
        }
    })
    .await?
}
fn underlying(error: &ChildError) -> &ChildError {
    match error {
        ChildError::Admission(source) => underlying(source),
        other => other,
    }
}

#[test]
fn test_should_validate_child_parameters_input_normalization_and_runtime_failure_mapping()
-> TestResult {
    let scope = TenantScope::default();
    let machine = arn();
    for (input, raw) in [
        (json!(null), "{}"),
        (json!("null"), "null"),
        (json!(" { \"n\":7 } "), " { \"n\":7 } "),
        (json!([1, null]), "[1,null]"),
        (json!(false), "false"),
    ] {
        assert_eq!(
            parameters(&machine, None, input)?.input().original_bytes(),
            raw.as_bytes()
        );
    }
    let omitted = ChildStartParameters::parse(
        &scope,
        &Payload::from_value(&json!({"StateMachineArn":machine.as_str()}))?,
    )?;
    assert_eq!(omitted.input().original_bytes(), b"{}");
    for value in [
        json!({"StateMachineArn":"bad"}),
        json!({"StateMachineArn":"arn:aws:states:us-west-2:000000000000:stateMachine:Child"}),
        json!({"StateMachineArn":machine.as_str(),"Name":"invalid:name"}),
        json!({"StateMachineArn":machine.as_str(),"Name":null}),
        json!({"StateMachineArn":machine.as_str(),"Input":"invalid-json"}),
        json!({"StateMachineArn":machine.as_str(),"Unknown":true}),
    ] {
        let error = ChildStartParameters::parse(&scope, &Payload::from_value(&value)?).unwrap_err();
        assert_eq!(error.failure().details().error(), Some("States.Runtime"));
    }
    Ok(())
}
#[tokio::test]
async fn test_should_capture_fast_child_once_and_map_exact_optimized_envelope_profiles()
-> TestResult {
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    let parent = make_parent(&reg).await?;
    let machine = publish(
        &reg,
        "Child",
        json!({"StartAt":"D","States":{"D":{"Type":"Pass","End":true}}}),
    )
    .await?;
    for pattern in [
        ChildPattern::RequestResponse,
        ChildPattern::Sync,
        ChildPattern::Sync2,
    ] {
        let child = reg.child_port().start(
            parent.child_parent(),
            parameters(&machine, None, json!(" { \"n\":7 } "))?,
            pattern,
            deadline(),
        )?;
        let mut child = accepted(child).await?;
        child.wait(deadline()).await?;
        let result = child
            .result()?
            .map_err(|failure| format!("unexpected failure {failure:?}"))?;
        assert!(
            result.value()["StartDate"]
                .as_i64()
                .is_some_and(|time| (chrono::Utc::now().timestamp_millis() - time).abs() < 60_000)
        );
        assert_eq!(
            result.value()["ExecutionArn"],
            child.snapshot().arn().as_str()
        );
        assert_eq!(child.snapshot().nesting_depth(), 1);
        assert_eq!(
            child
                .snapshot()
                .parent_execution_arn()
                .map(ExecutionArn::as_str),
            Some(parent.arn().as_str())
        );
        if pattern == ChildPattern::RequestResponse {
            assert_eq!(result.value().as_object().ok_or("object")?.len(), 2);
        } else {
            assert_eq!(result.value()["Status"], "SUCCEEDED");
            assert_eq!(result.value()["InputDetails"]["Included"], true);
            assert_eq!(result.value()["OutputDetails"]["Included"], true);
            assert!(result.value()["StopDate"].as_i64() >= result.value()["StartDate"].as_i64());
            if pattern == ChildPattern::Sync {
                assert_eq!(result.value()["Input"], " { \"n\":7 } ");
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(
                        result.value()["Output"].as_str().ok_or("output")?
                    )?,
                    json!({"n":7})
                );
            } else {
                assert_eq!(result.value()["Input"], json!({"n":7}));
                assert_eq!(result.value()["Output"], json!({"n":7}));
            }
        }
        child.wait_cleanup(deadline()).await?;
    }
    owner.shutdown(deadline()).await?;
    Ok(())
}
#[tokio::test]
async fn test_should_preserve_admission_after_cancelled_waiter_and_late_ready_observer()
-> TestResult {
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    let parent = make_parent(&reg).await?;
    let child_machine = publish(&reg, "Child", wait()).await?;
    tokio::time::pause();
    let mut submission = reg.child_port().start(
        parent.child_parent(),
        parameters(&child_machine, Some("once"), json!({}))?,
        ChildPattern::Sync2,
        deadline(),
    )?;
    assert!(matches!(
        submission.admitted(Instant::now()).await,
        Err(ChildError::Deadline)
    ));
    let observer = Instant::now() + Duration::from_secs(1);
    {
        let mut waiter = Box::pin(submission.admitted(observer));
        assert!(poll_fn(|cx| Poll::Ready(matches!(waiter.as_mut().poll(cx), Poll::Pending))).await);
    }
    let mut waiter = Box::pin(submission.admitted(observer));
    assert!(poll_fn(|cx| Poll::Ready(matches!(waiter.as_mut().poll(cx), Poll::Pending))).await);
    committed_child(&reg, &child_machine).await?;
    tokio::time::advance(Duration::from_secs(2)).await;
    assert!(matches!(waiter.await, Err(ChildError::Deadline)));
    submission.admitted(deadline()).await?;
    assert!(matches!(
        submission.admitted(Instant::now()).await,
        Err(ChildError::Deadline)
    ));
    let mut child = submission.into_execution()?;
    child.cancel();
    child.wait_cleanup(deadline()).await?;
    owner.shutdown(deadline()).await?;
    Ok(())
}
#[tokio::test]
async fn test_should_cancel_unissued_start_without_claiming_name_and_cancel_after_lost_reply()
-> TestResult {
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    let parent = make_parent(&reg).await?;
    let machine = publish(&reg, "Child", wait()).await?;
    let mut unissued = reg.child_port().start(
        parent.child_parent(),
        parameters(&machine, Some("once"), json!({}))?,
        ChildPattern::Sync,
        deadline(),
    )?;
    unissued.cancel();
    let failure = unissued.admitted(deadline()).await.unwrap_err();
    assert!(matches!(underlying(&failure), ChildError::Cancelled));
    let submission = reg.child_port().start(
        parent.child_parent(),
        parameters(&machine, Some("once"), json!({}))?,
        ChildPattern::Sync,
        deadline(),
    )?;
    let record = committed_child(&reg, &machine).await?;
    drop(submission);
    let complete = record.wait(deadline()).await?;
    assert!(matches!(
        complete.view().outcome().map(Arc::as_ref),
        Some(ExecutionOutcome::Aborted(_))
    ));
    owner.shutdown(deadline()).await?;
    Ok(())
}
#[tokio::test]
async fn test_should_leave_accepted_detached_child_alive_after_parent_stop_and_handle_drop()
-> TestResult {
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    let parent = make_parent(&reg).await?;
    let machine = publish(&reg, "Child", wait()).await?;
    let child = accepted(reg.child_port().start(
        parent.child_parent(),
        parameters(&machine, None, json!({}))?,
        ChildPattern::RequestResponse,
        deadline(),
    )?)
    .await?;
    let child_arn = child.snapshot().arn().clone();
    child.cancel();
    drop(child);
    reg.stop(parent.arn().clone(), None, None, deadline())
        .await?;
    let RegistryResult::Execution(snapshot) =
        result(&reg, RegistryCommand::Execution(child_arn)).await?
    else {
        return Err("child".into());
    };
    assert!(snapshot.handle().snapshot().outcome().is_none());
    owner.shutdown(deadline()).await?;
    Ok(())
}
#[tokio::test]
async fn test_should_reject_foreign_and_closed_parent_capabilities_before_start() -> TestResult {
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    let parent = make_parent(&reg).await?;
    let machine = publish(&reg, "Child", wait()).await?;
    let (other, mut other_owner) = registry(RegistryLimits::default(), None);
    let foreign = make_parent(&other).await?;
    for capability in [foreign.child_parent(), {
        reg.stop(parent.arn().clone(), None, None, deadline())
            .await?;
        parent.child_parent()
    }] {
        let mut submission = reg.child_port().start(
            capability,
            parameters(&machine, None, json!({}))?,
            ChildPattern::Sync,
            deadline(),
        )?;
        let error = submission.admitted(deadline()).await.unwrap_err();
        assert!(matches!(underlying(&error), ChildError::Parent));
    }
    let RegistryResult::Executions(records) =
        result(&reg, RegistryCommand::Executions(machine)).await?
    else {
        return Err("records".into());
    };
    assert!(records.is_empty());
    other_owner.shutdown(deadline()).await?;
    owner.shutdown(deadline()).await?;
    Ok(())
}
#[tokio::test]
async fn test_should_reject_idempotent_owned_cycles_and_preserve_original_lineage() -> TestResult {
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    let parent = make_parent(&reg).await?;
    let mut self_join = reg.child_port().start(
        parent.child_parent(),
        parameters(&arn(), Some("parent"), json!({}))?,
        ChildPattern::Sync,
        deadline(),
    )?;
    let error = self_join.admitted(deadline()).await.unwrap_err();
    assert!(matches!(underlying(&error), ChildError::Cycle));
    let mut child = accepted(reg.child_port().start(
        parent.child_parent(),
        parameters(&arn(), Some("child"), json!({}))?,
        ChildPattern::Sync,
        deadline(),
    )?)
    .await?;
    let mut ancestor = reg.child_port().start(
        child.snapshot().child_parent(),
        parameters(&arn(), Some("parent"), json!({}))?,
        ChildPattern::Sync2,
        deadline(),
    )?;
    let error = ancestor.admitted(deadline()).await.unwrap_err();
    assert!(matches!(underlying(&error), ChildError::Cycle));
    let replay = accepted(reg.child_port().start(
        parent.child_parent(),
        parameters(&arn(), Some("child"), json!({}))?,
        ChildPattern::RequestResponse,
        deadline(),
    )?)
    .await?;
    assert_eq!(
        replay.snapshot().arn().as_str(),
        child.snapshot().arn().as_str()
    );
    assert_eq!(replay.snapshot().nesting_depth(), 1);
    child.cancel();
    child.wait_cleanup(deadline()).await?;
    owner.shutdown(deadline()).await?;
    Ok(())
}
#[tokio::test]
async fn test_should_bound_new_child_lineage_at_exact_sixteen_levels() -> TestResult {
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    let root = make_parent(&reg).await?;
    let mut current = root;
    let mut children = Vec::new();
    for level in 1..=16 {
        let child = accepted(reg.child_port().start(
            current.child_parent(),
            parameters(&arn(), Some(&format!("depth-{level}")), json!({}))?,
            ChildPattern::Sync2,
            deadline(),
        )?)
        .await?;
        assert_eq!(child.snapshot().nesting_depth(), level);
        assert_eq!(
            child
                .snapshot()
                .parent_execution_arn()
                .map(ExecutionArn::as_str),
            Some(current.arn().as_str())
        );
        current = child.snapshot().clone();
        children.push(child);
    }
    let mut too_deep = reg.child_port().start(
        current.child_parent(),
        parameters(&arn(), Some("depth-17"), json!({}))?,
        ChildPattern::Sync2,
        deadline(),
    )?;
    assert_eq!(
        too_deep
            .admitted(deadline())
            .await
            .unwrap_err()
            .failure()
            .details()
            .error(),
        Some("StepFunctions.ExecutionLimitExceeded")
    );
    owner.shutdown(deadline()).await?;
    Ok(())
}
#[tokio::test]
async fn test_should_bound_active_join_depth_even_when_all_children_were_external_roots()
-> TestResult {
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    machine(&reg, 1, true).await;
    let mut roots = Vec::new();
    for level in 0..=17 {
        roots.push(start(&reg, &format!("root-{level}"), "{}").await?);
    }
    let mut children = Vec::new();
    for level in 0..16 {
        let parent = roots.get(level).ok_or("parent")?;
        let child = accepted(reg.child_port().start(
            parent.child_parent(),
            parameters(&arn(), Some(&format!("root-{}", level + 1)), json!({}))?,
            ChildPattern::Sync,
            deadline(),
        )?)
        .await?;
        assert_eq!(child.snapshot().nesting_depth(), 0);
        children.push(child);
    }
    let mut too_deep = reg.child_port().start(
        roots.get(16).ok_or("parent")?.child_parent(),
        parameters(&arn(), Some("root-17"), json!({}))?,
        ChildPattern::Sync,
        deadline(),
    )?;
    assert_eq!(
        too_deep
            .admitted(deadline())
            .await
            .unwrap_err()
            .failure()
            .details()
            .error(),
        Some("StepFunctions.ExecutionLimitExceeded")
    );
    owner.shutdown(deadline()).await?;
    Ok(())
}

#[tokio::test]
async fn test_should_retain_owned_child_cleanup_and_cancel_descendants_without_dispatch_reservation()
-> TestResult {
    let (sender, mut receiver) = mpsc::channel(4);
    let runtime = ExecutionSupervisor::new(
        RuntimeLimits::new(32, 1, 4)?,
        HistoryLimits::default(),
        256 * 1024 * 1024,
        Some(Arc::new(Port(sender))),
    )?;
    let (reg, mut owner) = Registry::new(
        TenantScope::default(),
        RegistryLimits::default(),
        runtime,
        ExecutionLimits::default(),
    )?;
    let root = make_parent(&reg).await?;
    let child_machine = publish(&reg, "Child", wait()).await?;
    let leaf_machine=publish(&reg,"Leaf",json!({"StartAt":"Call","States":{"Call":{"Type":"Task","Resource":"arn:aws:lambda:us-east-1:000000000000:function:Worker","End":true}}})).await?;
    let mut child = accepted(reg.child_port().start(
        root.child_parent(),
        parameters(&child_machine, None, json!({}))?,
        ChildPattern::Sync,
        deadline(),
    )?)
    .await?;
    let mut leaf = accepted(reg.child_port().start(
        child.snapshot().child_parent(),
        parameters(&leaf_machine, None, json!({}))?,
        ChildPattern::Sync,
        deadline(),
    )?)
    .await?;
    let mut responder = timeout(Duration::from_secs(5), receiver.recv())
        .await?
        .ok_or("call")?;
    responder.started()?;
    reg.stop(root.arn().clone(), None, None, deadline()).await?;
    let mut cancellation = responder.cancellation();
    timeout(Duration::from_secs(3), async {
        while !*cancellation.borrow_and_update() {
            cancellation.changed().await?;
        }
        Ok::<_, Box<dyn Error>>(())
    })
    .await??;
    assert!(matches!(
        child.wait(deadline()).await?.outcome().map(Arc::as_ref),
        Some(ExecutionOutcome::Aborted(_))
    ));
    assert!(matches!(
        leaf.wait(deadline()).await?.outcome().map(Arc::as_ref),
        Some(ExecutionOutcome::Aborted(_))
    ));
    assert!(matches!(
        leaf.wait_cleanup(Instant::now() + Duration::from_millis(10))
            .await,
        Err(ChildError::Execution(ExecutionError::Deadline))
    ));
    child.wait_cleanup(deadline()).await?;
    result(&reg, RegistryCommand::Machines).await?;
    let charged = reg.shared.child_capacity.available_permits();
    assert_eq!(charged, 31);
    drop(leaf);
    result(&reg, RegistryCommand::Machines).await?;
    assert_eq!(reg.shared.child_capacity.available_permits(), charged);
    responder.finish(Ok(Payload::parse(b"null")?));
    timeout(Duration::from_secs(3), async {
        while reg.shared.child_capacity.available_permits() != 32 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    owner.shutdown(deadline()).await?;
    Ok(())
}
#[tokio::test]
async fn test_should_bound_child_calls_separately_and_rollback_exhausted_open_start() -> TestResult
{
    let runtime = ExecutionSupervisor::new(
        RuntimeLimits::new(2, 1, 2)?,
        HistoryLimits::default(),
        256 * 1024 * 1024,
        None,
    )?;
    let limits = RegistryLimits::new(RegistryCapacities::default(), Duration::from_secs(60), 1)?;
    let (reg, mut owner) = Registry::new(
        TenantScope::default(),
        limits,
        runtime,
        ExecutionLimits::default(),
    )?;
    let root = make_parent(&reg).await?;
    let child_machine = publish(&reg, "Child", wait()).await?;
    let mut child = accepted(reg.child_port().start(
        root.child_parent(),
        parameters(&child_machine, Some("child"), json!({}))?,
        ChildPattern::Sync,
        deadline(),
    )?)
    .await?;
    let mut full = reg.child_port().start(
        root.child_parent(),
        parameters(&child_machine, Some("later"), json!({}))?,
        ChildPattern::Sync,
        deadline(),
    )?;
    assert_eq!(
        full.admitted(deadline())
            .await
            .unwrap_err()
            .failure()
            .details()
            .error(),
        Some("StepFunctions.ExecutionLimitExceeded")
    );
    let replay = accepted(reg.child_port().start(
        root.child_parent(),
        parameters(&child_machine, Some("child"), json!({}))?,
        ChildPattern::Sync,
        deadline(),
    )?)
    .await?;
    assert_eq!(reg.shared.child_capacity.available_permits(), 0);
    assert!(matches!(
        reg.child_port().start(
            root.child_parent(),
            parameters(&child_machine, Some("saturated"), json!({}))?,
            ChildPattern::Sync,
            deadline()
        ),
        Err(ChildError::Capacity)
    ));
    result(&reg, RegistryCommand::Machines).await?;
    child.cancel();
    child.wait_cleanup(deadline()).await?;
    drop(replay);
    timeout(Duration::from_secs(3), async {
        while reg.shared.child_capacity.available_permits() != 2 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    let later = accepted(reg.child_port().start(
        root.child_parent(),
        parameters(&child_machine, Some("later"), json!({}))?,
        ChildPattern::Sync,
        deadline(),
    )?)
    .await?;
    drop(later);
    owner.shutdown(deadline()).await?;
    Ok(())
}
#[tokio::test]
async fn test_should_map_failed_or_timed_out_child_and_bound_unicode_failure_cause() -> TestResult {
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    let root = make_parent(&reg).await?;
    for (name, definition, input, status) in [
        (
            "Failure",
            json!({"StartAt":"F","States":{"F":{"Type":"Fail","Error":"BusinessFailure","CausePath":"$.cause"}}}),
            json!({"cause":"\0中".repeat(2000)}),
            "FAILED",
        ),
        (
            "Timed",
            json!({"TimeoutSeconds":1,"StartAt":"W","States":{"W":{"Type":"Wait","Seconds":60,"End":true}}}),
            json!({}),
            "TIMED_OUT",
        ),
    ] {
        let machine = publish(&reg, name, definition).await?;
        let mut child = accepted(reg.child_port().start(
            root.child_parent(),
            parameters(&machine, None, input)?,
            ChildPattern::Sync2,
            deadline(),
        )?)
        .await?;
        child.wait(deadline()).await?;
        let failure = child.result()?.unwrap_err();
        assert_eq!(failure.details().error(), Some("States.TaskFailed"));
        let text = failure.details().cause().ok_or("cause")?;
        assert!(text.len() <= 32768);
        let cause: serde_json::Value = serde_json::from_str(text)?;
        assert_eq!(cause["Status"], status);
        assert_eq!(cause["ExecutionArn"], child.snapshot().arn().as_str());
        if status == "FAILED" {
            assert_eq!(cause["Error"], "BusinessFailure");
            assert_eq!(cause["CauseTruncated"], true);
            assert!(cause["Cause"].as_str().ok_or("cause")?.len() <= 4096);
        }
        child.wait_cleanup(deadline()).await?;
    }
    owner.shutdown(deadline()).await?;
    Ok(())
}
#[tokio::test]
async fn test_should_enforce_combined_optimized_envelope_payload_limit() -> TestResult {
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    let root = make_parent(&reg).await?;
    let machine = publish(
        &reg,
        "Child",
        json!({"StartAt":"D","States":{"D":{"Type":"Pass","End":true}}}),
    )
    .await?;
    let mut child = accepted(reg.child_port().start(
        root.child_parent(),
        parameters(&machine, None, json!({"large":"x".repeat(140_000)}))?,
        ChildPattern::Sync2,
        deadline(),
    )?)
    .await?;
    child.wait(deadline()).await?;
    assert_eq!(
        child.result().unwrap_err().failure().details().error(),
        Some("States.DataLimitExceeded")
    );
    child.wait_cleanup(deadline()).await?;
    owner.shutdown(deadline()).await?;
    Ok(())
}

#[tokio::test]
async fn test_should_keep_weak_child_port_usable_without_public_registry_and_close_without_cycle()
-> TestResult {
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    let root = make_parent(&reg).await?;
    let machine = publish(
        &reg,
        "Child",
        json!({"StartAt":"D","States":{"D":{"Type":"Succeed"}}}),
    )
    .await?;
    let port = reg.child_port();
    drop(reg);
    let mut child = accepted(port.start(
        root.child_parent(),
        parameters(&machine, None, json!({}))?,
        ChildPattern::Sync2,
        deadline(),
    )?)
    .await?;
    child.wait(deadline()).await?;
    child.wait_cleanup(deadline()).await?;
    owner.shutdown(deadline()).await?;
    assert!(matches!(
        port.start(
            root.child_parent(),
            parameters(&machine, None, json!({}))?,
            ChildPattern::Sync,
            deadline()
        ),
        Err(ChildError::Unavailable)
    ));
    Ok(())
}
#[tokio::test]
async fn test_should_preserve_null_scalar_and_array_input_types_in_sync_results() -> TestResult {
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    let root = make_parent(&reg).await?;
    let machine = publish(
        &reg,
        "Child",
        json!({"StartAt":"D","States":{"D":{"Type":"Pass","End":true}}}),
    )
    .await?;
    for (input, expected) in [
        (json!(null), json!({})),
        (json!("null"), json!(null)),
        (json!([1, null]), json!([1, null])),
        (json!(false), json!(false)),
    ] {
        for pattern in [ChildPattern::Sync, ChildPattern::Sync2] {
            let mut child = accepted(reg.child_port().start(
                root.child_parent(),
                parameters(&machine, None, input.clone())?,
                pattern,
                deadline(),
            )?)
            .await?;
            child.wait(deadline()).await?;
            let output = child
                .result()?
                .map_err(|failure| format!("unexpected failure {failure:?}"))?;
            let value = &output.value()["Input"];
            let value = if pattern == ChildPattern::Sync {
                serde_json::from_str(value.as_str().ok_or("serialized input")?)?
            } else {
                value.clone()
            };
            assert_eq!(value, expected);
            child.wait_cleanup(deadline()).await?;
        }
    }
    owner.shutdown(deadline()).await?;
    Ok(())
}

#[tokio::test]
async fn test_should_cache_late_ready_admission_error_without_losing_its_typed_result() -> TestResult
{
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    let root = make_parent(&reg).await?;
    let target = "arn:aws:states:us-east-1:000000000000:stateMachine:Missing".parse()?;
    tokio::time::pause();
    let observe = Instant::now() + Duration::from_secs(1);
    let mut submission = reg.child_port().start(
        root.child_parent(),
        parameters(&target, Some("missing"), json!({}))?,
        ChildPattern::Sync2,
        deadline(),
    )?;
    let mut waiter = Box::pin(submission.admitted(observe));
    assert!(poll_fn(|cx| Poll::Ready(matches!(waiter.as_mut().poll(cx), Poll::Pending))).await);
    while reg.shared.child_capacity.available_permits() != 128 {
        tokio::task::yield_now().await;
    }
    tokio::time::advance(Duration::from_secs(2)).await;
    assert!(matches!(waiter.await, Err(ChildError::Deadline)));
    let error = submission.admitted(deadline()).await.unwrap_err();
    assert_eq!(
        error.failure().details().error(),
        Some("StepFunctions.StateMachineDoesNotExist")
    );
    owner.shutdown(deadline()).await?;
    Ok(())
}
#[tokio::test]
async fn test_should_not_cancel_new_child_attempt_when_old_completed_owner_is_dropped() -> TestResult
{
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    let root = make_parent(&reg).await?;
    let fast = publish(
        &reg,
        "Fast",
        json!({"StartAt":"D","States":{"D":{"Type":"Pass","Result":1,"End":true}}}),
    )
    .await?;
    let slow = publish(&reg, "Slow", wait()).await?;
    let mut first = accepted(reg.child_port().start(
        root.child_parent(),
        parameters(&fast, None, json!({}))?,
        ChildPattern::Sync,
        deadline(),
    )?)
    .await?;
    first.wait(deadline()).await?;
    first.wait_cleanup(deadline()).await?;
    let mut second = accepted(reg.child_port().start(
        root.child_parent(),
        parameters(&slow, None, json!({}))?,
        ChildPattern::Sync,
        deadline(),
    )?)
    .await?;
    first.cancel();
    drop(first);
    result(&reg, RegistryCommand::Machines).await?;
    assert!(matches!(
        second
            .wait(Instant::now() + Duration::from_millis(10))
            .await,
        Err(ChildError::Execution(ExecutionError::Deadline))
    ));
    second.cancel();
    second.wait_cleanup(deadline()).await?;
    owner.shutdown(deadline()).await?;
    Ok(())
}

#[tokio::test]
async fn test_should_observe_fast_child_termination_even_before_start_receipt_is_consumed()
-> TestResult {
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    let root = make_parent(&reg).await?;
    let machine = publish(
        &reg,
        "Child",
        json!({"StartAt":"D","States":{"D":{"Type":"Pass","Result":42,"End":true}}}),
    )
    .await?;
    let mut submission = reg.child_port().start(
        root.child_parent(),
        parameters(&machine, Some("fast"), json!({}))?,
        ChildPattern::Sync2,
        deadline(),
    )?;
    let record = committed_child(&reg, &machine).await?;
    let completed = record.wait(deadline()).await?;
    assert!(
        matches!(completed.view().outcome().map(Arc::as_ref),Some(ExecutionOutcome::Succeeded(value)) if value.value()==&json!(42))
    );
    submission.admitted(deadline()).await?;
    let mut child = submission.into_execution()?;
    child.wait(deadline()).await?;
    let result = child.result()?.map_err(|_| "child failed")?;
    assert_eq!(result.value()["Output"], 42);
    child.wait_cleanup(deadline()).await?;
    owner.shutdown(deadline()).await?;
    Ok(())
}

#[test]
fn test_should_preserve_byte_string_and_key_quota_error_classification() {
    for error in [JsonError::Bytes, JsonError::String, JsonError::Key] {
        assert_eq!(
            ChildError::Data(AslError::Json(error))
                .failure()
                .details()
                .error(),
            Some("States.DataLimitExceeded")
        );
    }
}
