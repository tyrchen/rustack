//! Actual ASL child integration through registry and retained frame owners.

use rustack_stepfunctions_model::types::HistoryEventType;

use super::*;
use crate::{ClockSample, EvaluationContext, ExecutionDriver};

type TestResult = Result<(), Box<dyn Error>>;
fn nested_registry(
    open: usize,
    port: Option<Arc<dyn LambdaTaskPort>>,
) -> (Registry, RegistrySupervisor) {
    let runtime = ExecutionSupervisor::new(
        RuntimeLimits::new(open, 1, 1).unwrap(),
        HistoryLimits::default(),
        256 * 1024 * 1024,
        port,
    )
    .unwrap();
    Registry::new(
        TenantScope::default(),
        RegistryLimits::default(),
        runtime,
        ExecutionLimits::default(),
    )
    .unwrap()
}
async fn publish(
    reg: &Registry,
    name: &str,
    definition: serde_json::Value,
) -> Result<StateMachineArn, Box<dyn Error>> {
    let artifact = AslCompiler::new(TenantScope::default(), TaskSupport::Lambda)
        .with_child_support(true)
        .compile(&serde_json::to_vec(&definition)?)?;
    let RegistryResult::Machine(machine) = result(
        reg,
        RegistryCommand::Create(
            name.parse()?,
            PreparedDefinition::new(artifact.machine().clone()),
            role(),
            Tags::default(),
        ),
    )
    .await?
    else {
        return Err("machine".into());
    };
    Ok(machine.arn().clone())
}
fn parent(child: &StateMachineArn, suffix: &str, input: serde_json::Value) -> serde_json::Value {
    let mut definition = json!({"StartAt":"T","States":{"T":{"Type":"Task","Resource":format!("arn:aws:states:::states:startExecution{suffix}"),"Parameters":{"StateMachineArn":child.as_str()},"End":true}}});
    definition["States"]["T"]["Parameters"]["Input"] = input;
    definition
}

async fn launch(
    reg: &Registry,
    arn: &StateMachineArn,
) -> Result<ExecutionSnapshot, Box<dyn Error>> {
    let RegistryResult::Execution(execution) = result(
        reg,
        RegistryCommand::Start(arn.clone(), None, Payload::parse(b"{}")?, None),
    )
    .await?
    else {
        return Err("execution".into());
    };
    Ok(execution)
}
async fn finish(
    execution: &ExecutionSnapshot,
) -> Result<Arc<crate::ExecutionView>, Box<dyn Error>> {
    Ok(execution.handle().wait_cleanup(deadline()).await?)
}
fn succeeded(view: &crate::ExecutionView) -> Result<&Payload, Box<dyn Error>> {
    match view.outcome().map(Arc::as_ref) {
        Some(ExecutionOutcome::Succeeded(value)) => Ok(value),
        other => Err(format!("expected success, got {other:?}").into()),
    }
}
async fn child_record(
    reg: &Registry,
    child: &StateMachineArn,
) -> Result<ExecutionSnapshot, Box<dyn Error>> {
    timeout(Duration::from_secs(5), async {
        loop {
            if let RegistryResult::Executions(records) =
                result(reg, RegistryCommand::Executions(child.clone())).await?
            {
                if let Some(record) = records.first() {
                    return Ok(record.clone());
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await?
}
#[rstest]
#[case("")]
#[case(".sync")]
#[case(".sync:2")]
#[tokio::test]
async fn test_should_execute_nested_asl_and_project_actual_causal_history(
    #[case] suffix: &str,
) -> TestResult {
    let (reg, mut owner) = nested_registry(8, None);
    let child = publish(
        &reg,
        "Child",
        json!({"StartAt":"D","States":{"D":{"Type":"Pass","End":true}}}),
    )
    .await?;
    let machine = publish(
        &reg,
        "Parent",
        parent(&child, suffix, json!(" { \"answer\":42 } ")),
    )
    .await?;
    let execution = launch(&reg, &machine).await?;
    let view = finish(&execution).await?;
    let output = succeeded(&view)?.value();
    let record = child_record(&reg, &child).await?;
    assert_eq!(output["ExecutionArn"], record.arn().as_str());
    assert_eq!(output["StartDate"], record.start_date().timestamp_millis());
    if suffix == ".sync" {
        assert_eq!(output["Input"], " { \"answer\":42 } ");
        assert_eq!(output["Output"], "{\"answer\":42}");
    }
    if suffix == ".sync:2" {
        assert_eq!(output["Input"], json!({"answer":42}));
        assert_eq!(output["Output"], json!({"answer":42}));
    }
    let page = view.history().page(1, 100, true)?;
    let kinds: Vec<_> = page.events().iter().map(|e| e.r#type.clone()).collect();
    let mut expected = vec![
        HistoryEventType::ExecutionStarted,
        HistoryEventType::TaskStateEntered,
        HistoryEventType::TaskScheduled,
        HistoryEventType::TaskStarted,
    ];
    if !suffix.is_empty() {
        expected.push(HistoryEventType::TaskSubmitted);
    }
    expected.extend([
        HistoryEventType::TaskSucceeded,
        HistoryEventType::TaskStateExited,
        HistoryEventType::ExecutionSucceeded,
    ]);
    assert_eq!(kinds, expected);
    for (index, event) in page.events().iter().enumerate() {
        assert_eq!(event.id, i64::try_from(index + 1)?);
        let previous = if index == 1 { 0 } else { i64::try_from(index)? };
        assert_eq!(event.previous_event_id, Some(previous));
        if let Some(details) = &event.execution_started_event_details {
            assert_eq!(details.role_arn.as_deref(), Some(role().as_str()));
        }
        if let Some(d) = &event.task_submitted_event_details {
            assert_eq!(d.resource_type, "states");
            assert_eq!(d.resource, format!("startExecution{suffix}"));
            let accepted: serde_json::Value =
                serde_json::from_str(d.output.as_deref().ok_or("submitted")?)?;
            assert_eq!(accepted["ExecutionArn"], record.arn().as_str());
        }
    }
    for event in view.history().page(1, 100, false)?.events() {
        if let Some(d) = &event.task_submitted_event_details {
            assert!(d.output.is_none() && d.output_details.is_none());
        }
    }
    owner.shutdown(deadline()).await?;
    Ok(())
}
#[tokio::test]
async fn test_should_run_child_lambda_with_only_one_global_dispatch_slot() -> TestResult {
    let (tx, mut rx) = mpsc::channel(1);
    let (reg, mut owner) = nested_registry(8, Some(Arc::new(Port(tx))));
    let child = publish(&reg, "Child", json!({"StartAt":"L","States":{"L":{"Type":"Task","Resource":"arn:aws:lambda:us-east-1:000000000000:function:Worker","End":true}}})).await?;
    let machine = publish(&reg, "Parent", parent(&child, ".sync:2", json!({"n":21}))).await?;
    let execution = launch(&reg, &machine).await?;
    let mut responder = timeout(Duration::from_secs(5), rx.recv())
        .await?
        .ok_or("child lambda not dispatched")?;
    responder.started()?;
    responder.finish(Ok(Payload::parse(b"42")?));
    let view = finish(&execution).await?;
    assert_eq!(succeeded(&view)?.value()["Output"], 42);
    owner.shutdown(deadline()).await?;
    Ok(())
}
#[rstest]
#[case("")]
#[case(".sync")]
#[case(".sync:2")]
#[tokio::test]
async fn test_should_detach_start_but_cancel_and_join_owned_child_on_parent_stop(
    #[case] suffix: &str,
) -> TestResult {
    let (reg, mut owner) = nested_registry(8, None);
    let child = publish(
        &reg,
        "Child",
        json!({"StartAt":"W","States":{"W":{"Type":"Wait","Seconds":3600,"End":true}}}),
    )
    .await?;
    let mut definition = parent(&child, suffix, json!({}));
    definition["States"]["T"]
        .as_object_mut()
        .ok_or("Task")?
        .remove("End");
    definition["States"]["T"]["Next"] = json!("W");
    definition["States"]["W"] = json!({"Type":"Wait","Seconds":3600,"End":true});
    let machine = publish(&reg, "Parent", definition).await?;
    let execution = launch(&reg, &machine).await?;
    let record = child_record(&reg, &child).await?;
    execution
        .handle()
        .request_stop(TaskFailure::new("Stopped", None)?);
    finish(&execution).await?;
    if suffix.is_empty() {
        assert!(record.handle().snapshot().outcome().is_none());
    } else {
        assert!(matches!(
            finish(&record).await?.outcome().map(Arc::as_ref),
            Some(ExecutionOutcome::Aborted(_))
        ));
    }
    owner.shutdown(deadline()).await?;
    Ok(())
}
#[tokio::test]
async fn test_should_fail_nested_open_capacity_without_deadlock_or_child_name_claim() -> TestResult
{
    let (reg, mut owner) = nested_registry(1, None);
    let child = publish(
        &reg,
        "Child",
        json!({"StartAt":"D","States":{"D":{"Type":"Succeed"}}}),
    )
    .await?;
    let machine = publish(&reg, "Parent", parent(&child, ".sync:2", json!({}))).await?;
    let execution = launch(&reg, &machine).await?;
    let view = finish(&execution).await?;
    assert!(
        matches!(view.outcome().map(Arc::as_ref), Some(ExecutionOutcome::Failed(f)) if f.error() == Some("StepFunctions.ExecutionLimitExceeded"))
    );
    assert!(
        matches!(result(&reg, RegistryCommand::Executions(child)).await?, RegistryResult::Executions(v) if v.is_empty())
    );
    owner.shutdown(deadline()).await?;
    Ok(())
}
#[rstest]
#[case(json!(null), json!({}))]
#[case(json!("null"), json!(null))]
#[case(json!([1,2]), json!([1,2]))]
#[tokio::test]
async fn test_should_preserve_nested_input_normalization(
    #[case] input: serde_json::Value,
    #[case] expected: serde_json::Value,
) -> TestResult {
    let (reg, mut owner) = nested_registry(8, None);
    let child = publish(
        &reg,
        "Child",
        json!({"StartAt":"D","States":{"D":{"Type":"Pass","End":true}}}),
    )
    .await?;
    let machine = publish(&reg, "Parent", parent(&child, ".sync:2", input)).await?;
    assert_eq!(
        succeeded(finish(&launch(&reg, &machine).await?).await?.as_ref())?.value()["Output"],
        expected
    );
    owner.shutdown(deadline()).await?;
    Ok(())
}
#[tokio::test]
async fn test_should_map_child_failure_and_runtime_dynamic_parameter_errors() -> TestResult {
    let (reg, mut owner) = nested_registry(8, None);
    let child = publish(&reg, "Child", json!({"StartAt":"F","States":{"F":{"Type":"Fail","Error":"ChildFailure","Cause":"reason"}}})).await?;
    for (index, definition, expected) in [
        (0, parent(&child, ".sync:2", json!({})), "States.TaskFailed"),
        (
            1,
            json!({"StartAt":"T","States":{"T":{"Type":"Task","Resource":"arn:aws:states:::states:startExecution.sync:2","Parameters":{"StateMachineArn.$":"$.missing"},"End":true}}}),
            "States.Runtime",
        ),
        (
            2,
            json!({"StartAt":"T","States":{"T":{"Type":"Task","Resource":"arn:aws:states:::states:startExecution.sync:2","Parameters":{"StateMachineArn.$":"$$.Execution.Name"},"End":true}}}),
            "States.Runtime",
        ),
    ] {
        let machine = publish(&reg, &format!("Parent{index}"), definition).await?;
        let view = finish(&launch(&reg, &machine).await?).await?;
        assert!(
            matches!(view.outcome().map(Arc::as_ref), Some(ExecutionOutcome::Failed(f)) if f.error() == Some(expected)),
            "{view:?}"
        );
    }
    owner.shutdown(deadline()).await?;
    Ok(())
}

#[tokio::test]
async fn test_should_keep_observed_child_success_when_cleanup_outlives_parent_task_deadline()
-> TestResult {
    let (tx, mut rx) = mpsc::channel(1);
    let (reg, mut owner) = nested_registry(8, Some(Arc::new(Port(tx))));
    let child = publish(&reg, "Child", json!({"StartAt":"L","States":{"L":{"Type":"Task","Resource":"arn:aws:lambda:us-east-1:000000000000:function:Worker","End":true}}})).await?;
    let mut definition = parent(&child, ".sync:2", json!({}));
    definition["States"]["T"]["TimeoutSeconds"] = json!(1);
    let machine = publish(&reg, "Parent", definition).await?;
    let execution = launch(&reg, &machine).await?;
    let mut responder = timeout(Duration::from_secs(5), rx.recv())
        .await?
        .ok_or("child call")?;
    responder.started()?;
    let pending = responder.complete(Ok(Payload::parse(b"42")?));
    let record = child_record(&reg, &child).await?;
    assert!(matches!(
        record
            .handle()
            .wait(deadline())
            .await?
            .outcome()
            .map(Arc::as_ref),
        Some(ExecutionOutcome::Succeeded(_))
    ));
    timeout(Duration::from_secs(1), async {
        loop {
            if execution
                .handle()
                .snapshot()
                .history()
                .page(1, 100, true)
                .unwrap()
                .events()
                .iter()
                .any(|e| e.r#type == HistoryEventType::TaskSucceeded)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await?;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(
        execution
            .handle()
            .wait_cleanup(Instant::now() + Duration::from_millis(10))
            .await
            .is_err()
    );
    pending.settle();
    assert_eq!(
        succeeded(finish(&execution).await?.as_ref())?.value()["Output"],
        42
    );
    owner.shutdown(deadline()).await?;
    Ok(())
}
#[tokio::test]
async fn test_should_fail_child_cpu_panic_fatally_without_retry_and_retain_typed_source()
-> TestResult {
    let mut runtime = ExecutionSupervisor::new(
        RuntimeLimits::new(8, 1, 1)?,
        HistoryLimits::default(),
        256 * 1024 * 1024,
        None,
    )?;
    runtime.inject_child_cpu_panic();
    let (reg, mut owner) = Registry::new(
        TenantScope::default(),
        RegistryLimits::default(),
        runtime,
        ExecutionLimits::default(),
    )?;
    let child = publish(
        &reg,
        "Child",
        json!({"StartAt":"D","States":{"D":{"Type":"Succeed"}}}),
    )
    .await?;
    let mut definition = parent(&child, ".sync:2", json!({}));
    definition["States"]["T"]["Retry"] =
        json!([{"ErrorEquals":["States.ALL"],"IntervalSeconds":1,"MaxAttempts":2}]);
    definition["States"]["T"]["Catch"] = json!([{"ErrorEquals":["States.ALL"],"Next":"D"}]);
    definition["States"]["D"] = json!({"Type":"Succeed"});
    let machine = publish(&reg, "Parent", definition).await?;
    let execution = launch(&reg, &machine).await?;
    let view = execution.handle().wait(deadline()).await?;
    assert!(
        matches!(view.outcome().map(Arc::as_ref),Some(ExecutionOutcome::Failed(f)) if f.error()==Some("Rustack.InternalError"))
    );
    assert!(matches!(view.diagnostic(),Some(ExecutionError::Worker(source)) if source.is_panic()));
    assert_eq!(
        view.history()
            .page(1, 100, true)?
            .events()
            .iter()
            .filter(|e| e.r#type == HistoryEventType::TaskScheduled)
            .count(),
        1
    );
    assert!(owner.shutdown(deadline()).await.is_err());
    Ok(())
}
#[tokio::test]
async fn test_should_enforce_depth_on_recursive_asl_child_execution() -> TestResult {
    let (reg, mut owner) = nested_registry(32, None);
    let expected: StateMachineArn =
        "arn:aws:states:us-east-1:000000000000:stateMachine:Recursive".parse()?;
    let machine = publish(&reg, "Recursive", parent(&expected, ".sync:2", json!({}))).await?;
    let execution = launch(&reg, &machine).await?;
    let view = finish(&execution).await?;
    assert!(
        matches!(view.outcome().map(Arc::as_ref),Some(ExecutionOutcome::Failed(f)) if f.error()==Some("States.TaskFailed"))
    );
    let RegistryResult::Executions(records) =
        result(&reg, RegistryCommand::Executions(machine)).await?
    else {
        return Err("records".into());
    };
    assert_eq!(records.len(), 17);
    assert_eq!(
        records.iter().map(ExecutionSnapshot::nesting_depth).max(),
        Some(16)
    );
    owner.shutdown(deadline()).await?;
    Ok(())
}

#[tokio::test]
async fn test_should_cancel_child_at_parent_task_timeout_and_wait_for_physical_cleanup()
-> TestResult {
    let (tx, mut rx) = mpsc::channel(1);
    let (reg, mut owner) = nested_registry(8, Some(Arc::new(Port(tx))));
    let child = publish(&reg, "Child", json!({"StartAt":"L","States":{"L":{"Type":"Task","Resource":"arn:aws:lambda:us-east-1:000000000000:function:Worker","End":true}}})).await?;
    let mut definition = parent(&child, ".sync:2", json!({}));
    definition["States"]["T"]["TimeoutSeconds"] = json!(1);
    let machine = publish(&reg, "Parent", definition).await?;
    let execution = launch(&reg, &machine).await?;
    let mut responder = timeout(Duration::from_secs(5), rx.recv())
        .await?
        .ok_or("child call")?;
    responder.started()?;
    let mut cancel = responder.cancellation();
    timeout(Duration::from_secs(3), cancel.wait_for(|cancel| *cancel)).await??;
    assert!(
        execution
            .handle()
            .wait_cleanup(Instant::now() + Duration::from_millis(10))
            .await
            .is_err()
    );
    responder.finish(Err(TaskFailure::service_exception()));
    let view = finish(&execution).await?;
    assert!(
        matches!(view.outcome().map(Arc::as_ref), Some(ExecutionOutcome::Failed(f)) if f.error()==Some("States.Timeout"))
    );
    owner.shutdown(deadline()).await?;
    Ok(())
}

#[tokio::test]
async fn test_should_reject_unregistered_child_machine_without_issuing_work() -> TestResult {
    let machine = AslCompiler::default()
        .with_child_support(true)
        .compile(&serde_json::to_vec(&parent(&arn(), ".sync:2", json!({})))?)?
        .machine()
        .clone();
    let driver = ExecutionDriver::for_structured_owner(
        machine,
        Payload::parse(b"{}")?,
        EvaluationContext::from_value(&json!({}))?,
        ClockSample::new(Instant::now().into_std(), chrono::Utc::now()),
        ExecutionLimits::default(),
    )?;
    let mut runtime = ExecutionSupervisor::new(
        RuntimeLimits::default(),
        HistoryLimits::default(),
        256 * 1024 * 1024,
        None,
    )?;
    assert!(matches!(
        runtime.start_structured(driver.clone()),
        Err(ExecutionError::Unavailable)
    ));
    assert!(matches!(
        runtime.start(driver),
        Err(ExecutionError::Unavailable)
    ));
    runtime.drain(deadline()).await?;
    Ok(())
}
