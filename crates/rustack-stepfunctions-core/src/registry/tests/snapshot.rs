//! Durable export follows physical cleanup, not business termination or observers.

use std::{
    error::Error,
    future::{Future, poll_fn},
    str,
    sync::Arc,
    task::Poll,
    time::Duration,
};

use rstest::rstest;
use serde_json::{Value, json};
use tokio::{
    sync::mpsc,
    time::{Instant, timeout_at},
};

use super::{Port, arn, deadline, definition, machine, observe, registry, result, role, start};
use crate::{
    AslCompiler, CleanupState, ExecutionLimits, ExecutionSupervisor, HistoryLimits,
    LambdaTaskCommand, LambdaTaskPort, Payload, PreparedDefinition, Registry, RegistryCapacities,
    RegistryCommand, RegistryError, RegistryLimits, RoleArn, RuntimeLimits, SnapshotLimits, Tags,
    TaskCall, TaskFailure, TaskSupport, TenantScope,
};

type TestResult = Result<(), Box<dyn Error>>;

fn rows<'a>(value: &'a Value, table: &str) -> Result<&'a [Value], Box<dyn Error>> {
    value
        .get(table)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| "missing snapshot rows".into())
}
fn row<'a>(
    value: &'a Value,
    table: &str,
    field: &str,
    id: &str,
) -> Result<&'a Value, Box<dyn Error>> {
    rows(value, table)?
        .iter()
        .find(|row| row.get(field).and_then(Value::as_str) == Some(id))
        .ok_or_else(|| "missing snapshot row".into())
}
fn exported(registry: &Registry) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::to_value(
        registry.export_snapshot(SnapshotLimits::default())?,
    )?)
}

#[rstest]
#[case(0, false)]
#[case(1, true)]
#[case(1024*1024*1024, true)]
#[case(1024*1024*1024+1, false)]
fn test_should_validate_snapshot_source_copy_budget(#[case] bytes: usize, #[case] valid: bool) {
    assert_eq!(SnapshotLimits::try_from(bytes).is_ok(), valid);
}

#[tokio::test]
async fn test_should_export_only_the_sealed_deterministic_cut_after_controller_loss() -> TestResult
{
    let (reg, owner) = registry(RegistryLimits::default(), None);
    assert!(matches!(
        reg.export_snapshot(SnapshotLimits::default()),
        Err(RegistryError::Unavailable)
    ));
    machine(&reg, 42, false).await;
    let execution = observe(start(&reg, "done", "{} ").await?).await;
    let mut sealed = reg.shared.sealed.subscribe();
    drop(owner);
    let _capture = timeout_at(deadline(), sealed.wait_for(Option::is_some)).await??;
    let first = exported(&reg)?;
    tokio::task::yield_now().await;
    let second = exported(&reg)?;
    assert_eq!(first, second);
    assert_eq!(first.get("schemaVersion"), Some(&json!(1)));
    assert_eq!(first.get("profileVersion"), Some(&json!(1)));
    assert_eq!(first.get("stateSupport"), Some(&json!("sequential")));
    assert_eq!(first.get("lambdaSupport"), Some(&json!(false)));
    assert_eq!(first.get("callbackSupport"), Some(&json!(false)));
    let saved = row(&first, "executions", "arn", execution.arn().as_str())?;
    assert_eq!(saved.get("input"), Some(&json!("{} ")));
    assert_eq!(saved.pointer("/outcome/status"), Some(&json!("SUCCEEDED")));
    assert_eq!(saved.pointer("/outcome/output"), Some(&json!("42")));
    assert!(saved.get("traceHeader").is_none());
    assert_eq!(
        format!("{:?}", reg.export_snapshot(SnapshotLimits::default())?),
        "RegistrySnapshot(<redacted>)"
    );
    Ok(())
}

#[tokio::test]
async fn test_should_keep_original_allocations_until_last_sealed_handle_is_dropped() -> TestResult {
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    let machine = machine(&reg, 7, false).await;
    let execution = observe(start(&reg, "held", "{}").await?).await;
    let revision = Arc::downgrade(machine.revision());
    let record = Arc::downgrade(&execution.record);
    drop(machine);
    drop(execution);
    owner.shutdown(deadline()).await?;
    let snapshot = reg.export_snapshot(SnapshotLimits::default())?;
    drop(owner);
    assert!(revision.upgrade().is_some());
    assert!(record.upgrade().is_some());
    drop(reg);
    assert!(revision.upgrade().is_none());
    assert!(record.upgrade().is_none());
    assert_eq!(
        rows(&serde_json::to_value(snapshot)?, "executions")?.len(),
        1
    );
    Ok(())
}

#[tokio::test]
async fn test_should_preserve_old_revisions_identities_roles_tags_and_exact_inputs() -> TestResult {
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    let old = machine(&reg, 1, true).await;
    let old_source = str::from_utf8(old.revision().machine().original_bytes())?.to_owned();
    let old_run = start(&reg, "a-old", " \n{ \"n\": 7 }\n ").await?;
    let new_role: RoleArn = "arn:aws:iam::000000000000:role/NewRole".parse()?;
    result(
        &reg,
        RegistryCommand::Update(arn(), Some(definition(2, false)), Some(new_role.clone())),
    )
    .await?;
    let new_run = observe(start(&reg, "b-new", "{}").await?).await;
    reg.stop(old_run.arn().clone(), None, Some(String::new()), deadline())
        .await?;
    result(&reg, RegistryCommand::Delete(arn())).await?;
    let recreated = machine(&reg, 3, false).await;
    result(
        &reg,
        RegistryCommand::Tag(arn(), Tags::new(vec![("team".into(), "archive".into())])?),
    )
    .await?;
    owner.shutdown(deadline()).await?;
    let archive = exported(&reg)?;
    assert_eq!(rows(&archive, "resources")?.len(), 2);
    assert_eq!(rows(&archive, "revisions")?.len(), 3);
    assert_eq!(rows(&archive, "machines")?.len(), 1);
    let captured = row(&archive, "executions", "arn", old_run.arn().as_str())?;
    assert_eq!(captured.get("resourceId"), Some(&json!(old.resource_id())));
    assert_eq!(
        captured.get("revisionId"),
        Some(&json!(old.revision().id()))
    );
    assert_eq!(captured.get("input"), Some(&json!(" \n{ \"n\": 7 }\n ")));
    assert!(captured.pointer("/outcome/error").is_none());
    assert_eq!(captured.pointer("/outcome/cause"), Some(&json!("")));
    let old_revision = row(&archive, "revisions", "revisionId", old.revision().id())?;
    assert_eq!(old_revision.get("definition"), Some(&json!(old_source)));
    assert_eq!(
        old_revision.get("sourceHash"),
        Some(&json!(old.revision().source_hash()))
    );
    assert_eq!(
        old_revision.get("roleArn"),
        Some(&json!(old.revision().role().as_str()))
    );
    let second_revision = row(&archive, "revisions", "revisionId", new_run.revision().id())?;
    assert_eq!(
        second_revision.get("roleArn"),
        Some(&json!(new_role.as_str()))
    );
    let current = row(&archive, "machines", "resourceId", recreated.resource_id())?;
    assert_eq!(current.pointer("/tags/team"), Some(&json!("archive")));
    assert_eq!(current.get("status"), Some(&json!("active")));
    Ok(())
}

#[tokio::test]
async fn test_should_enforce_exact_conservative_budget_before_copy_and_allow_retry() -> TestResult {
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    let machine = machine(&reg, 42, false).await;
    let execution = observe(start(&reg, "budget", "{}").await?).await;
    owner.shutdown(deadline()).await?;
    let bytes = 5 * 4096
        + reg.scope().partition().len()
        + reg.scope().region().len()
        + reg.scope().account().len()
        + machine.revision().machine().original_bytes().len()
        + machine.revision().role().as_str().len()
        + 2
        + 2
        + execution.handle().snapshot().history().charged_bytes()
        + 512;
    let before = serde_json::to_vec(&reg.export_snapshot(SnapshotLimits::try_from(bytes)?)?)?;
    assert!(matches!(
        reg.export_snapshot(SnapshotLimits::try_from(bytes - 1)?),
        Err(RegistryError::Capacity)
    ));
    assert_eq!(
        before,
        serde_json::to_vec(&reg.export_snapshot(SnapshotLimits::try_from(bytes)?)?)?
    );
    Ok(())
}

#[tokio::test]
async fn test_should_not_seal_terminal_business_history_while_physical_cleanup_is_pending()
-> TestResult {
    let (tx, mut rx) = mpsc::channel(1);
    let limits = RegistryLimits::new(RegistryCapacities::default(), Duration::from_secs(60), 1)?;
    let (reg, mut owner) = registry(limits, Some(Arc::new(Port(tx))));
    let prepared=PreparedDefinition::new(AslCompiler::new(TenantScope::default(),TaskSupport::Lambda).compile(br#"{"StartAt":"T","States":{"T":{"Type":"Task","Resource":"arn:aws:lambda:us-east-1:000000000000:function:Worker","End":true}}}"#)?.machine().clone());
    result(
        &reg,
        RegistryCommand::Create("Machine".parse()?, prepared, role(), Tags::default()),
    )
    .await?;
    let execution = start(&reg, "physical", "{}").await?;
    let responder = timeout_at(deadline(), rx.recv())
        .await?
        .ok_or("missing actual task owner")?;
    let held = reg.submit(RegistryCommand::Machines)?;
    assert!(matches!(
        reg.submit(RegistryCommand::Machines),
        Err(RegistryError::Capacity)
    ));
    assert!(matches!(
        reg.export_snapshot(SnapshotLimits::default()),
        Err(RegistryError::Unavailable)
    ));
    assert!(matches!(
        owner
            .shutdown(Instant::now() + Duration::from_millis(5))
            .await,
        Err(RegistryError::Deadline)
    ));
    let mut observer = Box::pin(owner.shutdown(deadline()));
    poll_fn(|context| {
        assert!(matches!(observer.as_mut().poll(context), Poll::Pending));
        Poll::Ready(())
    })
    .await;
    drop(observer);
    let view = execution.handle().wait(deadline()).await?;
    assert!(view.outcome().is_some());
    assert_ne!(view.cleanup(), CleanupState::Clean);
    assert!(matches!(
        reg.export_snapshot(SnapshotLimits::default()),
        Err(RegistryError::Unavailable)
    ));
    responder.finish(Ok(Payload::parse(b"{}")?));
    owner.shutdown(deadline()).await?;
    assert_eq!(rows(&exported(&reg)?, "executions")?.len(), 1);
    drop(held);
    Ok(())
}

#[tokio::test]
async fn test_should_export_every_history_event_beyond_api_page_boundaries() -> TestResult {
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    let prepared=PreparedDefinition::new(AslCompiler::default().compile(br#"{"StartAt":"P","States":{"P":{"Type":"Pass","Parameters":{"n.$":"States.MathAdd($.n,1)"},"Next":"C"},"C":{"Type":"Choice","Choices":[{"Variable":"$.n","NumericLessThan":600,"Next":"P"}],"Default":"D"},"D":{"Type":"Succeed"}}}"#)?.machine().clone());
    result(
        &reg,
        RegistryCommand::Create("Machine".parse()?, prepared, role(), Tags::default()),
    )
    .await?;
    let execution = observe(start(&reg, "many", "{\"n\":0}").await?).await;
    owner.shutdown(deadline()).await?;
    let view = execution.handle().snapshot();
    assert!(view.history().len() > 1000);
    assert!(view.history().page(1, 1000, true)?.next_id().is_some());
    let archive = exported(&reg)?;
    let saved = row(&archive, "executions", "arn", execution.arn().as_str())?
        .get("history")
        .and_then(Value::as_array)
        .ok_or("missing full history")?;
    assert_eq!(saved.len(), usize::try_from(view.history().len())?);
    for (saved, event) in saved.iter().zip(view.history().snapshot_events()?) {
        assert_eq!(
            saved.get("eventJson"),
            Some(&json!(serde_json::to_string(event)?))
        );
        assert_eq!(
            saved.get("timestamp"),
            Some(&serde_json::to_value(event.timestamp)?)
        );
    }
    Ok(())
}

#[tokio::test]
async fn test_should_never_export_a_failed_or_aborted_root() -> TestResult {
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    machine(&reg, 42, false).await;
    owner.join.as_ref().ok_or("missing registry root")?.abort();
    assert!(owner.shutdown(deadline()).await.is_err());
    assert!(matches!(
        reg.export_snapshot(SnapshotLimits::default()),
        Err(RegistryError::Unavailable)
    ));
    Ok(())
}

#[derive(Debug)]
struct PanickingDrop;
impl LambdaTaskPort for PanickingDrop {
    fn start(&self, _command: LambdaTaskCommand) -> Result<TaskCall, TaskFailure> {
        Err(TaskFailure::service_exception())
    }
}
impl Drop for PanickingDrop {
    fn drop(&mut self) {
        panic!("injected port destruction failure");
    }
}

#[tokio::test]
async fn test_should_not_publish_a_cut_before_drained_owner_destruction_succeeds() -> TestResult {
    let (reg, mut owner) = registry(RegistryLimits::default(), Some(Arc::new(PanickingDrop)));
    let machine = machine(&reg, 42, false).await;
    let execution = observe(start(&reg, "drop-panic", "{}").await?).await;
    let revision = Arc::downgrade(machine.revision());
    let record = Arc::downgrade(&execution.record);
    drop(machine);
    drop(execution);
    assert!(owner.shutdown(deadline()).await.is_err());
    assert!(matches!(
        reg.export_snapshot(SnapshotLimits::default()),
        Err(RegistryError::Unavailable)
    ));
    assert!(revision.upgrade().is_some());
    assert!(record.upgrade().is_some());
    Ok(())
}

#[rstest]
#[case(r#"{"StartAt":"F","States":{"F":{"Type":"Fail"}}}"#,json!({"status":"FAILED"}))]
#[case(r#"{"StartAt":"F","States":{"F":{"Type":"Fail","Error":"","Cause":""}}}"#,json!({"status":"FAILED","error":"","cause":""}))]
#[tokio::test]
async fn test_should_preserve_failed_outcome_omission_and_empty_fields(
    #[case] source: &str,
    #[case] expected: Value,
) -> TestResult {
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    let prepared = PreparedDefinition::new(
        AslCompiler::default()
            .compile(source.as_bytes())?
            .machine()
            .clone(),
    );
    result(
        &reg,
        RegistryCommand::Create("Machine".parse()?, prepared, role(), Tags::default()),
    )
    .await?;
    let execution = observe(start(&reg, "failed", "{}").await?).await;
    owner.shutdown(deadline()).await?;
    assert_eq!(
        row(
            &exported(&reg)?,
            "executions",
            "arn",
            execution.arn().as_str()
        )?
        .get("outcome"),
        Some(&expected)
    );
    Ok(())
}

#[tokio::test]
async fn test_should_export_absolute_timeout_without_inventing_error_fields() -> TestResult {
    let runtime = ExecutionSupervisor::new(
        RuntimeLimits::default(),
        HistoryLimits::default(),
        256 * 1024 * 1024,
        None,
    )?;
    let (reg, mut owner) = Registry::new(
        TenantScope::default(),
        RegistryLimits::default(),
        runtime,
        ExecutionLimits::new(Duration::from_secs(1), 100)?,
    )?;
    machine(&reg, 42, true).await;
    let execution = observe(start(&reg, "timed-out", "{}").await?).await;
    owner.shutdown(deadline()).await?;
    assert_eq!(
        row(
            &exported(&reg)?,
            "executions",
            "arn",
            execution.arn().as_str()
        )?
        .get("outcome"),
        Some(&json!({"status":"TIMED_OUT"}))
    );
    Ok(())
}

#[tokio::test]
async fn test_should_preserve_empty_stop_failure_fields() -> TestResult {
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    machine(&reg, 42, true).await;
    let execution = start(&reg, "empty-stop", "{}").await?;
    reg.stop(
        execution.arn().clone(),
        Some(String::new()),
        Some(String::new()),
        deadline(),
    )
    .await?;
    owner.shutdown(deadline()).await?;
    assert_eq!(
        row(
            &exported(&reg)?,
            "executions",
            "arn",
            execution.arn().as_str()
        )?
        .get("outcome"),
        Some(&json!({"status":"ABORTED","error":"","cause":""}))
    );
    Ok(())
}

#[tokio::test]
async fn test_should_seal_only_after_uncertain_physical_receipt_confirms_clean() -> TestResult {
    let (tx, mut rx) = mpsc::channel(1);
    let (reg, mut owner) = registry(RegistryLimits::default(), Some(Arc::new(Port(tx))));
    let prepared=PreparedDefinition::new(AslCompiler::new(TenantScope::default(),TaskSupport::Lambda).compile(br#"{"StartAt":"T","States":{"T":{"Type":"Task","Resource":"arn:aws:lambda:us-east-1:000000000000:function:Worker","End":true}}}"#)?.machine().clone());
    result(
        &reg,
        RegistryCommand::Create("Machine".parse()?, prepared, role(), Tags::default()),
    )
    .await?;
    let execution = start(&reg, "quarantined", "{}").await?;
    let responder = timeout_at(deadline(), rx.recv())
        .await?
        .ok_or("missing actual task owner")?;
    let receipt = responder.quarantine(TaskFailure::service_exception());
    execution.handle().wait(deadline()).await?;
    assert!(matches!(
        owner
            .shutdown(Instant::now() + Duration::from_millis(5))
            .await,
        Err(RegistryError::Deadline)
    ));
    assert!(matches!(
        reg.export_snapshot(SnapshotLimits::default()),
        Err(RegistryError::Unavailable)
    ));
    receipt.settle();
    owner.shutdown(deadline()).await?;
    assert_eq!(rows(&exported(&reg)?, "executions")?.len(), 1);
    Ok(())
}

#[derive(Debug)]
struct PanickingStart(mpsc::Sender<()>);
impl LambdaTaskPort for PanickingStart {
    fn start(&self, _command: LambdaTaskCommand) -> Result<TaskCall, TaskFailure> {
        let _sent = self.0.try_send(());
        panic!("injected actor start failure");
    }
}

#[tokio::test]
async fn test_should_not_seal_permanently_quarantined_actor_ownership() -> TestResult {
    let (tx, mut rx) = mpsc::channel(1);
    let (reg, mut owner) = registry(
        RegistryLimits::default(),
        Some(Arc::new(PanickingStart(tx))),
    );
    let prepared=PreparedDefinition::new(AslCompiler::new(TenantScope::default(),TaskSupport::Lambda).compile(br#"{"StartAt":"T","States":{"T":{"Type":"Task","Resource":"arn:aws:lambda:us-east-1:000000000000:function:Worker","End":true}}}"#)?.machine().clone());
    result(
        &reg,
        RegistryCommand::Create("Machine".parse()?, prepared, role(), Tags::default()),
    )
    .await?;
    let execution = start(&reg, "actor-quarantine", "{}").await?;
    timeout_at(deadline(), rx.recv())
        .await?
        .ok_or("actor did not enter injected failure")?;
    assert!(owner.shutdown(deadline()).await.is_err());
    assert_eq!(
        execution.handle().snapshot().cleanup(),
        CleanupState::Quarantined
    );
    assert!(matches!(
        reg.export_snapshot(SnapshotLimits::default()),
        Err(RegistryError::Unavailable)
    ));
    assert!(owner.shutdown(deadline()).await.is_err());
    assert!(matches!(
        reg.export_snapshot(SnapshotLimits::default()),
        Err(RegistryError::Unavailable)
    ));
    Ok(())
}
