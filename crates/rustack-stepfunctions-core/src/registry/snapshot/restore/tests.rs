//! A prepared graph becomes visible once, without executing archived work.

use std::error::Error;

use serde_json::{Value, json};

use super::super::{RegistrySnapshot, SnapshotLimits};
use crate::{
    ExecutionOutcome, Registry, RegistryCommand, RegistryLimits, RegistryResult,
    registry::tests::{arn, deadline, machine, observe, registry, result, start},
};

type TestResult = Result<(), Box<dyn Error>>;
async fn document() -> Result<RegistrySnapshot, Box<dyn Error>> {
    let (source, mut owner) = registry(RegistryLimits::default(), None);
    machine(&source, 7, false).await;
    observe(start(&source, "stored", " {\"n\":1}\n").await?).await;
    owner.shutdown(deadline()).await?;
    Ok(source.export_snapshot(SnapshotLimits::default())?)
}
async fn restored(registry: &Registry, snapshot: RegistrySnapshot) -> TestResult {
    let prepared = registry.prepare_snapshot(snapshot, SnapshotLimits::default())?;
    assert!(!registry.status().ready);
    let mut receipt = registry.restore_snapshot(prepared)?;
    assert!(matches!(
        receipt.wait(deadline()).await?,
        RegistryResult::Restored
    ));
    assert!(registry.status().ready);
    Ok(())
}
#[tokio::test]
async fn test_should_round_trip_real_cbor_and_continue_with_a_new_execution() -> TestResult {
    let snapshot = document().await?;
    let before = serde_json::to_value(&snapshot)?;
    let mut bytes = Vec::new();
    ciborium::into_writer(&snapshot, &mut bytes)?;
    let decoded: RegistrySnapshot = ciborium::from_reader(bytes.as_slice())?;
    let (target, mut owner) = registry(RegistryLimits::default(), None);
    restored(&target, decoded).await?;
    let saved_arn = before
        .pointer("/executions/0/arn")
        .and_then(Value::as_str)
        .ok_or("missing saved ARN")?
        .parse()?;
    let RegistryResult::Execution(saved) =
        result(&target, RegistryCommand::Execution(saved_arn)).await?
    else {
        return Err("missing restored execution".into());
    };
    assert_eq!(saved.input().original_bytes(), b" {\"n\":1}\n");
    assert!(saved.record.deadline.is_none());
    let new = observe(start(&target, "new", "{}").await?).await;
    for execution in [&new, &saved] {
        assert!(
            matches!(execution.view().outcome().map(AsRef::as_ref), Some(ExecutionOutcome::Succeeded(payload)) if payload.value() == &json!(7))
        );
    }
    assert!(matches!(
        result(&target, RegistryCommand::Machine(arn())).await?,
        RegistryResult::Machine(_)
    ));
    owner.shutdown(deadline()).await?;
    let after = serde_json::to_value(target.export_snapshot(SnapshotLimits::default())?)?;
    let old = after
        .get("executions")
        .and_then(Value::as_array)
        .ok_or("missing restored rows")?
        .iter()
        .find(|row| row.get("arn") == before.pointer("/executions/0/arn"))
        .ok_or("missing saved row")?;
    assert_eq!(old.get("history"), before.pointer("/executions/0/history"));
    assert_eq!(
        old.get("outcome"),
        Some(&json!({"status":"SUCCEEDED","output":"7"}))
    );
    Ok(())
}

#[path = "tests/admission.rs"]
mod admission;
#[path = "tests/retention.rs"]
mod retention;
#[path = "tests/validation.rs"]
mod validation;

#[path = "tests/history.rs"]
mod history;
