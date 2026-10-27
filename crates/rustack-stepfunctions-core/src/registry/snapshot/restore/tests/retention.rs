//! Imported absolute retention and interrupted execution classification.

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use chrono::{DateTime, TimeDelta, Utc};
use tokio::time::advance;

use super::*;
use crate::{
    AslCompiler, ExecutionArn, LambdaTaskCommand, LambdaTaskPort, Payload, PreparedDefinition,
    RegistryCapacities, RegistryError, Tags, TaskCall, TaskFailure, TaskSupport, TenantScope,
    registry::{snapshot::Outcome, tests::role},
};

fn set_expiry(data: &mut Value, kind: &str, time: DateTime<Utc>) -> TestResult {
    *data
        .pointer_mut(&format!("/{kind}/0/expiresAt"))
        .ok_or("missing expiry")? = json!(time);
    Ok(())
}
fn saved_arn(data: &Value) -> Result<ExecutionArn, Box<dyn Error>> {
    Ok(data
        .pointer("/executions/0/arn")
        .and_then(Value::as_str)
        .ok_or("missing ARN")?
        .parse()?)
}

#[tokio::test(start_paused = true)]
async fn test_should_preserve_imported_retention_across_changed_target_settings() -> TestResult {
    for retention in [60, 172_800] {
        let mut data = serde_json::to_value(document().await?)?;
        set_expiry(&mut data, "executions", Utc::now() + TimeDelta::seconds(10))?;
        set_expiry(&mut data, "names", Utc::now() + TimeDelta::seconds(30))?;
        let arn = saved_arn(&data)?;
        let limits = RegistryLimits::new(
            RegistryCapacities::default(),
            Duration::from_secs(retention),
            128,
        )?;
        let (target, mut owner) = registry(limits, None);
        restored(&target, serde_json::from_value(data)?).await?;
        advance(Duration::from_secs(20)).await;
        assert!(matches!(
            result(&target, RegistryCommand::Execution(arn.clone())).await,
            Err(RegistryError::ExecutionMissing)
        ));
        assert!(matches!(
            start(&target, "stored", "{}").await,
            Err(RegistryError::ExecutionExists)
        ));
        advance(Duration::from_secs(20)).await;
        observe(start(&target, "stored", "{}").await?).await;
        owner.shutdown(deadline()).await?;
    }
    Ok(())
}

#[tokio::test]
async fn test_should_validate_expired_rows_before_pruning_and_keep_unexpired_name() -> TestResult {
    let mut data = serde_json::to_value(document().await?)?;
    let now = Utc::now();
    *data.get_mut("capturedAt").ok_or("missing capture")? = json!(now - TimeDelta::hours(2));
    set_expiry(&mut data, "executions", now - TimeDelta::hours(1))?;
    set_expiry(&mut data, "names", now + TimeDelta::hours(1))?;
    let arn = saved_arn(&data)?;
    let (target, mut owner) = registry(RegistryLimits::default(), None);
    let mut corrupt = data.clone();
    *corrupt
        .pointer_mut("/revisions/0/sourceHash")
        .ok_or("missing hash")? = json!(vec![0; 32]);
    assert!(
        target
            .prepare_snapshot(serde_json::from_value(corrupt)?, SnapshotLimits::default())
            .is_err()
    );
    restored(&target, serde_json::from_value(data)?).await?;
    assert!(matches!(
        result(&target, RegistryCommand::Execution(arn)).await,
        Err(RegistryError::ExecutionMissing)
    ));
    assert!(matches!(
        start(&target, "stored", "{}").await,
        Err(RegistryError::ExecutionExists)
    ));
    owner.shutdown(deadline()).await?;
    let saved = serde_json::to_value(target.export_snapshot(SnapshotLimits::default())?)?;
    assert_eq!(saved.get("executions"), Some(&json!([])));
    assert_eq!(
        saved.get("names").and_then(Value::as_array).map(Vec::len),
        Some(1)
    );
    Ok(())
}

#[derive(Debug, Default)]
pub(super) struct CountingPort(AtomicUsize);
impl LambdaTaskPort for CountingPort {
    fn start(&self, _command: LambdaTaskCommand) -> Result<TaskCall, TaskFailure> {
        self.0.fetch_add(1, Ordering::SeqCst);
        let (call, responder) = TaskCall::channel();
        responder.finish(Ok(
            Payload::parse(b"42").map_err(|_| TaskFailure::throttled())?
        ));
        Ok(call)
    }
}
#[tokio::test]
async fn test_should_abort_interrupted_snapshot_without_replaying_lambda() -> TestResult {
    let port = Arc::new(CountingPort::default());
    let (source, mut source_owner) = registry(RegistryLimits::default(), Some(port.clone()));
    let definition = PreparedDefinition::new(AslCompiler::new(TenantScope::default(), TaskSupport::Lambda)
        .compile(br#"{"StartAt":"T","States":{"T":{"Type":"Task","Resource":"arn:aws:lambda:us-east-1:000000000000:function:Worker","End":true}}}"#)?.machine().clone());
    result(
        &source,
        RegistryCommand::Create("Machine".parse()?, definition, role(), Tags::default()),
    )
    .await?;
    observe(start(&source, "stored", "{}").await?).await;
    source_owner.shutdown(deadline()).await?;
    assert_eq!(port.0.load(Ordering::SeqCst), 1);
    let mut snapshot = source.export_snapshot(SnapshotLimits::default())?;
    let row = snapshot.executions.first_mut().ok_or("missing execution")?;
    row.outcome = Outcome::Running {};
    row.stopped_at = None;
    row.expires_at = None;
    row.history.pop();
    snapshot.names.first_mut().ok_or("missing name")?.expires_at = None;
    let arn = row.arn.parse()?;
    let (target, mut owner) = registry(RegistryLimits::default(), Some(port.clone()));
    restored(&target, snapshot).await?;
    let RegistryResult::Execution(saved) = result(&target, RegistryCommand::Execution(arn)).await?
    else {
        return Err("missing restored execution".into());
    };
    assert!(
        matches!(saved.view().outcome().map(AsRef::as_ref), Some(ExecutionOutcome::Aborted(failure)) if failure.details().error() == Some("Rustack.ExecutionInterrupted"))
    );
    assert!(saved.record.deadline.is_none());
    assert_eq!(port.0.load(Ordering::SeqCst), 1);
    observe(start(&target, "new", "{}").await?).await;
    assert_eq!(port.0.load(Ordering::SeqCst), 2);
    owner.shutdown(deadline()).await?;
    let captured = serde_json::to_value(target.export_snapshot(SnapshotLimits::default())?)?;
    assert_eq!(
        captured.pointer("/executions/1/outcome/error"),
        Some(&json!("Rustack.ExecutionInterrupted"))
    );
    Ok(())
}

pub(super) async fn lambda_document(optimized: bool) -> Result<RegistrySnapshot, Box<dyn Error>> {
    let port = Arc::new(CountingPort::default());
    let (source, mut owner) = registry(RegistryLimits::default(), Some(port));
    let state = if optimized {
        json!({"Type":"Task","Resource":"arn:aws:states:::lambda:invoke","Parameters":{"FunctionName":"Worker","Payload":{}},"TimeoutSeconds":30,"End":true})
    } else {
        json!({"Type":"Task","Resource":"arn:aws:lambda:us-east-1:000000000000:function:Worker","TimeoutSeconds":30,"End":true})
    };
    let prepared = PreparedDefinition::new(
        AslCompiler::new(TenantScope::default(), TaskSupport::Lambda)
            .compile(&serde_json::to_vec(
                &json!({"StartAt":"T","States":{"T":state}}),
            )?)?
            .machine()
            .clone(),
    );
    result(
        &source,
        RegistryCommand::Create("Machine".parse()?, prepared, role(), Tags::default()),
    )
    .await?;
    observe(start(&source, "stored", "{}").await?).await;
    owner.shutdown(deadline()).await?;
    Ok(source.export_snapshot(SnapshotLimits::default())?)
}
