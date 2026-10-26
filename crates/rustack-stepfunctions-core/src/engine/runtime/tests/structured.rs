//! Actual asynchronous task-port admission and causal frame history.

use std::{error::Error, time::Duration};

use chrono::Utc;
use serde_json::{Value, json};
use tokio::time::{Instant, timeout};

use super::{deadline, fixture, next, task};
use crate::{
    AslCompiler, ClockSample, EvaluationContext, ExecutionDriver, ExecutionLimits,
    ExecutionOutcome, Payload, StateSupport, TaskFailure, TaskSupport, TenantScope,
};

type TestResult = Result<(), Box<dyn Error>>;
fn driver(value: &Value, input: &Value) -> Result<ExecutionDriver, Box<dyn Error>> {
    let compiled = AslCompiler::new(TenantScope::default(), TaskSupport::Lambda)
        .with_state_support(StateSupport::Structured)
        .compile(&serde_json::to_vec(value)?)?;
    let input = Payload::from_value(input)?;
    Ok(ExecutionDriver::for_structured_owner(
        compiled.machine().clone(),
        input.clone(),
        EvaluationContext::from_value(
            &json!({"Execution":{"Input":input.value(),"Id":"structured"}}),
        )?,
        ClockSample::new(Instant::now().into_std(), Utc::now()),
        ExecutionLimits::new(Duration::from_secs(60), 10_000)?,
    )?)
}
fn parallel() -> Value {
    json!({"StartAt":"Parallel","States":{"Parallel":{"Type":"Parallel","Branches":[task(),task()],"End":true}}})
}

#[tokio::test]
async fn test_should_start_overlapping_calls_and_preserve_parallel_result_and_causal_order()
-> TestResult {
    let (mut owner, mut calls, _) = fixture(2, 2, crate::HistoryLimits::default());
    let mut handle = owner.start_structured(driver(&parallel(), &json!({"input":1}))?)?;
    let (_, mut first) = next(&mut calls).await;
    let (_, mut second) = next(&mut calls).await;
    first.started()?;
    second.started()?;
    second.finish(Ok(Payload::parse(b"2")?));
    first.finish(Ok(Payload::parse(b"1")?));
    let view = handle.wait(deadline()).await?;
    let Some(outcome) = view.outcome() else {
        return Err("missing outcome".into());
    };
    let ExecutionOutcome::Succeeded(output) = outcome.as_ref() else {
        return Err(format!("unexpected outcome {outcome:?}").into());
    };
    assert_eq!(output.value(), &json!([1, 2]));
    let page = view.history().page(1, 100, true)?;
    let events = page.events();
    let branch_entries = events
        .iter()
        .filter(|e| {
            e.r#type == rustack_stepfunctions_model::types::HistoryEventType::TaskStateEntered
        })
        .collect::<Vec<_>>();
    assert_eq!(branch_entries.len(), 2);
    assert_eq!(
        branch_entries
            .first()
            .ok_or("first branch")?
            .previous_event_id,
        branch_entries
            .get(1)
            .ok_or("second branch")?
            .previous_event_id
    );
    assert!(events.iter().any(|e| {
        e.id > 3
            && e.previous_event_id
                .is_some_and(|previous| previous < e.id - 1)
    }));
    owner.drain(deadline()).await?;
    assert_eq!(owner.shared.frames.capacity.available_frames(), 4096);
    Ok(())
}

#[tokio::test]
async fn test_should_run_nested_map_and_empty_map_with_actual_actor_history() -> TestResult {
    let (mut owner, mut calls, _) = fixture(2, 2, crate::HistoryLimits::default());
    let value = json!({"StartAt":"Map","States":{"Map":{"Type":"Map","MaxConcurrency":2,"ItemProcessor":task(),"End":true}}});
    let mut handle = owner.start_structured(driver(&value, &json!([10, 20]))?)?;
    let (first, mut first_response) = next(&mut calls).await;
    let (second, mut second_response) = next(&mut calls).await;
    first_response.started()?;
    second_response.started()?;
    assert_eq!(first.dispatch().parameters().value(), &json!(10));
    assert_eq!(second.dispatch().parameters().value(), &json!(20));
    second_response.finish(Ok(Payload::parse(b"40")?));
    first_response.finish(Ok(Payload::parse(b"20")?));
    let view = handle.wait(deadline()).await?;
    let ExecutionOutcome::Succeeded(output) = view.outcome().ok_or("outcome")?.as_ref() else {
        return Err("expected success".into());
    };
    assert_eq!(output.value(), &json!([20, 40]));
    let events = view.history().page(1, 100, true)?;
    assert_eq!(
        events
            .events()
            .iter()
            .filter(|e| e.map_iteration_started_event_details.is_some())
            .count(),
        2
    );
    let mut empty = owner.start_structured(driver(&value, &json!([]))?)?;
    let view = empty.wait(deadline()).await?;
    let ExecutionOutcome::Succeeded(output) = view.outcome().ok_or("outcome")?.as_ref() else {
        return Err("expected empty success".into());
    };
    assert_eq!(output.value(), &json!([]));
    assert!(
        timeout(Duration::from_millis(10), calls.recv())
            .await
            .is_err()
    );
    owner.drain(deadline()).await?;
    Ok(())
}

#[tokio::test]
async fn test_should_stop_parallel_calls_and_join_cleanup_independently_of_terminal_publication()
-> TestResult {
    let (mut owner, mut calls, _) = fixture(1, 2, crate::HistoryLimits::default());
    let mut handle = owner.start_structured(driver(&parallel(), &json!({}))?)?;
    let (_, mut first) = next(&mut calls).await;
    let (_, mut second) = next(&mut calls).await;
    first.started()?;
    second.started()?;
    let view = handle
        .stop(TaskFailure::new("Stopped", None)?, deadline())
        .await?;
    assert!(matches!(
        view.outcome().map(AsRef::as_ref),
        Some(ExecutionOutcome::Aborted(_))
    ));
    assert!(*first.cancellation().borrow());
    assert!(*second.cancellation().borrow());
    assert!(matches!(
        owner
            .drain(Instant::now() + Duration::from_millis(10))
            .await,
        Err(crate::ExecutionError::Deadline)
    ));
    first.finish(Err(TaskFailure::new("Cancelled", None)?));
    second.finish(Err(TaskFailure::new("Cancelled", None)?));
    owner.drain(deadline()).await?;
    assert_eq!(owner.shared.frames.capacity.available_frames(), 4096);
    Ok(())
}

#[tokio::test]
async fn test_should_preserve_nested_failure_catch_after_descendant_cleanup() -> TestResult {
    let (mut owner, mut calls, _) = fixture(1, 3, crate::HistoryLimits::default());
    let map =
        json!({"StartAt":"Map","States":{"Map":{"Type":"Map","ItemProcessor":task(),"End":true}}});
    let mut value = parallel();
    value["States"]["Parallel"]["Branches"] = json!([task(), map]);
    value["States"]["Parallel"]["Catch"] = json!([{"ErrorEquals":["Oops"],"Next":"Handled"}]);
    value["States"]["Handled"] = json!({"Type":"Pass","Result":"caught","End":true});
    let mut handle = owner.start_structured(driver(&value, &json!([1, 2]))?)?;
    let (_, mut failed) = next(&mut calls).await;
    let (_, mut first) = next(&mut calls).await;
    let (_, mut second) = next(&mut calls).await;
    failed.started()?;
    first.started()?;
    second.started()?;
    failed.finish(Err(TaskFailure::new("Oops", None)?));
    timeout(Duration::from_secs(5), async {
        while !*first.cancellation().borrow() || !*second.cancellation().borrow() {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    assert!(handle.snapshot().outcome().is_none());
    first.finish(Err(TaskFailure::new("Cancelled", None)?));
    second.finish(Err(TaskFailure::new("Cancelled", None)?));
    let view = handle.wait(deadline()).await?;
    let ExecutionOutcome::Succeeded(output) = view.outcome().ok_or("outcome")?.as_ref() else {
        return Err("nested catch lost original failure".into());
    };
    assert_eq!(output.value(), &json!("caught"));
    let history = view.history().page(1, 100, true)?;
    assert_eq!(
        history
            .events()
            .iter()
            .filter(|event| event.map_iteration_aborted_event_details.is_some())
            .count(),
        2
    );
    owner.drain(deadline()).await?;
    Ok(())
}

#[tokio::test]
async fn test_should_abort_every_committed_map_iteration_before_root_terminal() -> TestResult {
    let (mut owner, mut calls, _) = fixture(1, 2, crate::HistoryLimits::default());
    let value =
        json!({"StartAt":"Map","States":{"Map":{"Type":"Map","ItemProcessor":task(),"End":true}}});
    let mut handle = owner.start_structured(driver(&value, &json!([1, 2]))?)?;
    let (_, mut first) = next(&mut calls).await;
    let (_, mut second) = next(&mut calls).await;
    first.started()?;
    second.started()?;
    let view = handle
        .stop(TaskFailure::new("Stopped", None)?, deadline())
        .await?;
    let page = view.history().page(1, 100, true)?;
    let started = page
        .events()
        .iter()
        .filter_map(|event| {
            event
                .map_iteration_started_event_details
                .as_ref()
                .and_then(|details| details.index)
        })
        .collect::<std::collections::BTreeSet<_>>();
    let aborted = page
        .events()
        .iter()
        .filter_map(|event| {
            event
                .map_iteration_aborted_event_details
                .as_ref()
                .and_then(|details| details.index)
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(started, aborted);
    assert_eq!(aborted.len(), 2);
    first.finish(Err(TaskFailure::new("Cancelled", None)?));
    second.finish(Err(TaskFailure::new("Cancelled", None)?));
    owner.drain(deadline()).await?;
    Ok(())
}

#[tokio::test]
async fn test_should_not_abort_exited_state_or_publish_discarded_next_state() -> TestResult {
    let (mut owner, mut calls, _) = fixture(1, 1, crate::HistoryLimits::default());
    let gate = super::WorkerHook::install(&mut owner, 3, false, false);
    let value = json!({"StartAt":"Work","States":{"Work":{"Type":"Task","Resource":"arn:aws:lambda:us-east-1:000000000000:function:Worker","Next":"Next"},"Next":{"Type":"Pass","End":true}}});
    let mut handle = owner.start_structured(driver(&value, &json!({}))?)?;
    let (_, mut response) = next(&mut calls).await;
    response.started()?;
    response.finish(Ok(Payload::parse(b"1")?));
    timeout(Duration::from_secs(5), gate.entered.notified()).await?;
    let view = handle
        .stop(TaskFailure::new("Stopped", None)?, deadline())
        .await?;
    let page = view.history().page(1, 100, true)?;
    assert!(page.events().iter().any(|event| event.r#type
        == rustack_stepfunctions_model::types::HistoryEventType::TaskStateExited));
    assert!(!page.events().iter().any(|event| event.r#type
        == rustack_stepfunctions_model::types::HistoryEventType::TaskStateAborted));
    assert!(!page.events().iter().any(|event| {
        event
            .state_entered_event_details
            .as_ref()
            .is_some_and(|details| details.name == "Next")
    }));
    gate.release();
    owner.drain(deadline()).await?;
    Ok(())
}

#[tokio::test]
async fn test_should_join_completed_cpu_work_after_pending_task_deadline_expires() -> TestResult {
    let (mut owner, mut calls, _) = fixture(1, 1, crate::HistoryLimits::default());
    let held = owner.shared.pools.dispatch.clone().acquire_owned().await?;
    let gate = super::WorkerHook::install(&mut owner, 1, false, false);
    let value = json!({"StartAt":"Work","States":{"Work":{"Type":"Task","Resource":"arn:aws:lambda:us-east-1:000000000000:function:Worker","TimeoutSeconds":1,"Catch":[{"ErrorEquals":["States.Timeout"],"Next":"Done"}],"End":true},"Done":{"Type":"Pass","Result":"caught","End":true}}});
    let mut handle = owner.start_structured(driver(&value, &json!({}))?)?;
    timeout(Duration::from_secs(5), gate.entered.notified()).await?;
    tokio::time::sleep(Duration::from_millis(1100)).await;
    gate.release();
    let view = handle.wait(deadline()).await?;
    let ExecutionOutcome::Succeeded(output) = view.outcome().ok_or("outcome")?.as_ref() else {
        return Err("Task timeout became overall timeout".into());
    };
    assert_eq!(output.value(), &json!("caught"));
    assert!(
        timeout(Duration::from_millis(10), calls.recv())
            .await
            .is_err()
    );
    drop(held);
    owner.drain(deadline()).await?;
    Ok(())
}

#[tokio::test]
async fn test_should_publish_stop_and_retain_root_lease_while_cpu_work_is_uncommitted() -> TestResult
{
    let (mut owner, mut calls, _) = fixture(1, 1, crate::HistoryLimits::default());
    let gate = super::WorkerHook::install(&mut owner, 0, false, false);
    let mut handle = owner.start_structured(driver(&parallel(), &json!({}))?)?;
    timeout(Duration::from_secs(5), gate.entered.notified()).await?;
    let view = handle
        .stop(TaskFailure::new("Stopped", None)?, deadline())
        .await?;
    assert!(matches!(
        view.outcome().map(AsRef::as_ref),
        Some(ExecutionOutcome::Aborted(_))
    ));
    assert!(owner.shared.frames.capacity.available_frames() < 4096);
    assert!(matches!(
        owner
            .drain(Instant::now() + Duration::from_millis(10))
            .await,
        Err(crate::ExecutionError::Deadline)
    ));
    gate.release();
    owner.drain(deadline()).await?;
    assert_eq!(owner.shared.frames.capacity.available_frames(), 4096);
    assert!(
        timeout(Duration::from_millis(10), calls.recv())
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_should_suppress_late_admission_history_and_keep_task_timeout() -> TestResult {
    let port = std::sync::Arc::new(super::RejectPort {
        stop: None,
        delay: Duration::from_millis(1100),
    });
    let mut owner = crate::ExecutionSupervisor::new(
        crate::RuntimeLimits::new(1, 1, 1)?,
        crate::HistoryLimits::default(),
        256 * 1024 * 1024,
        Some(port),
    )?;
    let value = json!({"StartAt":"Work","States":{"Work":{"Type":"Task","Resource":"arn:aws:lambda:us-east-1:000000000000:function:Worker","TimeoutSeconds":1,"End":true}}});
    let mut handle = owner.start_structured(driver(&value, &json!({}))?)?;
    let view = handle.wait(deadline()).await?;
    let ExecutionOutcome::Failed(failure) = view.outcome().ok_or("outcome")?.as_ref() else {
        return Err("expected Task timeout".into());
    };
    assert_eq!(failure.error(), Some("States.Timeout"));
    let page = view.history().page(1, 100, true)?;
    assert!(!page.events().iter().any(|event| matches!(
        event.r#type,
        rustack_stepfunctions_model::types::HistoryEventType::LambdaFunctionScheduleFailed
            | rustack_stepfunctions_model::types::HistoryEventType::LambdaFunctionStartFailed
    )));
    owner.drain(deadline()).await?;
    Ok(())
}

#[tokio::test]
async fn test_should_retain_ancestor_frame_capacity_after_actor_abort_with_accepted_calls()
-> TestResult {
    let (mut owner, mut calls, _) = fixture(1, 2, crate::HistoryLimits::default());
    let mut handle = owner.start_structured(driver(&parallel(), &json!({}))?)?;
    let (_, mut first) = next(&mut calls).await;
    let (_, mut second) = next(&mut calls).await;
    first.started()?;
    second.started()?;
    owner.state.jobs.abort_all();
    owner.reap_next().await;
    let view = handle.wait(deadline()).await?;
    assert!(matches!(
        view.outcome().map(AsRef::as_ref),
        Some(ExecutionOutcome::Failed(_))
    ));
    assert!(matches!(
        owner.drain(deadline()).await,
        Err(crate::ExecutionError::Quarantined)
    ));
    assert_eq!(owner.shared.pools.dispatch.available_permits(), 0);
    assert!(owner.shared.frames.capacity.available_frames() <= 4093);
    assert!(*first.cancellation().borrow());
    assert!(*second.cancellation().borrow());
    first.finish(Err(TaskFailure::new("Cancelled", None)?));
    second.finish(Err(TaskFailure::new("Cancelled", None)?));
    Ok(())
}
