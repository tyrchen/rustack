//! Full causal history, immutable old incarnations and selected state profiles.

use std::time::Duration;

use super::*;
use crate::{
    AslCompiler, ExecutionLimits, ExecutionSupervisor, HistoryLimits, Payload, PreparedDefinition,
    RuntimeLimits, StateSupport, Tags, TenantScope,
    registry::{
        snapshot::Outcome,
        tests::{definition, role},
    },
};

async fn exported_source(
    definition: Value,
    structured: bool,
) -> Result<RegistrySnapshot, Box<dyn Error>> {
    let support = if structured {
        StateSupport::Structured
    } else {
        StateSupport::Sequential
    };
    let runtime = ExecutionSupervisor::new(
        RuntimeLimits::default(),
        HistoryLimits::default(),
        256 * 1024 * 1024,
        None,
    )?;
    let (source, mut owner) = Registry::new_with_state_support(
        TenantScope::default(),
        RegistryLimits::default(),
        runtime,
        ExecutionLimits::default(),
        support,
    )?;
    let prepared = PreparedDefinition::new(
        AslCompiler::default()
            .with_state_support(support)
            .compile(&serde_json::to_vec(&definition)?)?
            .machine()
            .clone(),
    );
    result(
        &source,
        RegistryCommand::Create("Machine".parse()?, prepared, role(), Tags::default()),
    )
    .await?;
    observe(
        start(
            &source,
            "stored",
            if structured { "[1,2]" } else { "{\"n\":0}" },
        )
        .await?,
    )
    .await;
    owner.shutdown(deadline()).await?;
    Ok(source.export_snapshot(SnapshotLimits::default())?)
}
#[tokio::test]
async fn test_should_restore_full_history_beyond_api_pages_with_nanosecond_instants() -> TestResult
{
    let definition = json!({"StartAt":"P","States":{"P":{"Type":"Pass","Parameters":{"n.$":"States.MathAdd($.n,1)"},"Next":"C"},"C":{"Type":"Choice","Choices":[{"Variable":"$.n","NumericLessThan":600,"Next":"P"}],"Default":"D"},"D":{"Type":"Succeed"}}});
    let snapshot = exported_source(definition, false).await?;
    let expected = serde_json::to_value(&snapshot)?;
    assert!(
        snapshot
            .executions
            .first()
            .ok_or("missing history")?
            .history
            .len()
            > 1000
    );
    assert!(
        snapshot
            .executions
            .first()
            .ok_or("missing history")?
            .history
            .iter()
            .any(|row| row.timestamp.timestamp_subsec_nanos() % 1_000_000 != 0)
    );
    let (target, mut owner) = registry(RegistryLimits::default(), None);
    restored(&target, snapshot).await?;
    owner.shutdown(deadline()).await?;
    let actual = serde_json::to_value(target.export_snapshot(SnapshotLimits::default())?)?;
    assert_eq!(
        actual.pointer("/executions/0/history"),
        expected.pointer("/executions/0/history")
    );
    Ok(())
}

#[tokio::test]
async fn test_should_round_trip_structured_map_and_parallel_and_reject_weaker_target() -> TestResult
{
    let definition = json!({"StartAt":"P","States":{"P":{"Type":"Parallel","Branches":[{"StartAt":"M","States":{"M":{"Type":"Map","ItemsPath":"$","ItemProcessor":{"StartAt":"V","States":{"V":{"Type":"Pass","End":true}}},"End":true}}},{"StartAt":"D","States":{"D":{"Type":"Succeed"}}}],"End":true}}});
    let snapshot = exported_source(definition, true).await?;
    let stored = snapshot
        .executions
        .first()
        .ok_or("missing structured execution")?;
    assert!(matches!(stored.outcome, Outcome::Succeeded { .. }));
    assert_eq!(
        stored
            .history
            .iter()
            .filter(|event| event.event_json.contains("MapIterationSucceeded"))
            .count(),
        2
    );
    let expected = serde_json::to_value(&snapshot)?;
    let (weak, mut weak_owner) = registry(RegistryLimits::default(), None);
    assert!(
        weak.prepare_snapshot(snapshot, SnapshotLimits::default())
            .is_err()
    );
    weak_owner.shutdown(deadline()).await?;
    let runtime = ExecutionSupervisor::new(
        RuntimeLimits::default(),
        HistoryLimits::default(),
        256 * 1024 * 1024,
        None,
    )?;
    let (target, mut owner) = Registry::new_with_state_support(
        TenantScope::default(),
        RegistryLimits::default(),
        runtime,
        ExecutionLimits::default(),
        StateSupport::Structured,
    )?;
    restored(&target, serde_json::from_value(expected.clone())?).await?;
    owner.shutdown(deadline()).await?;
    let actual = serde_json::to_value(target.export_snapshot(SnapshotLimits::default())?)?;
    assert_eq!(
        actual.pointer("/executions/0/history"),
        expected.pointer("/executions/0/history")
    );
    Ok(())
}

#[tokio::test]
async fn test_should_preserve_recreated_identities_captured_source_roles_tags_and_trace()
-> TestResult {
    let (source, mut owner) = registry(RegistryLimits::default(), None);
    machine(&source, 1, false).await;
    observe(start(&source, "old", "{}").await?).await;
    result(&source, RegistryCommand::Delete(arn())).await?;
    let RegistryResult::Machine(current) = result(
        &source,
        RegistryCommand::Create(
            "Machine".parse()?,
            definition(2, false),
            "arn:aws:iam::000000000000:role/Updated".parse()?,
            Tags::new(vec![("Team".into(), "测试".into())])?,
        ),
    )
    .await?
    else {
        return Err("missing recreated machine".into());
    };
    let RegistryResult::Execution(new) = result(
        &source,
        RegistryCommand::Start(
            arn(),
            Some("current".parse()?),
            Payload::parse(b" {\"raw\":1}\n")?,
            Some("Root=trace\n".into()),
        ),
    )
    .await?
    else {
        return Err("missing new execution".into());
    };
    observe(new).await;
    owner.shutdown(deadline()).await?;
    let snapshot = source.export_snapshot(SnapshotLimits::default())?;
    let mut expected = serde_json::to_value(&snapshot)?;
    assert_eq!(snapshot.resources.len(), 2);
    let (target, mut owner) = registry(RegistryLimits::default(), None);
    restored(&target, snapshot).await?;
    let RegistryResult::Machine(saved) = result(&target, RegistryCommand::Machine(arn())).await?
    else {
        return Err("missing restored current machine".into());
    };
    assert_eq!(saved.resource_id(), current.resource_id());
    assert_eq!(
        saved.revision().machine().original_bytes(),
        current.revision().machine().original_bytes()
    );
    assert_eq!(
        saved.revision().role().as_str(),
        current.revision().role().as_str()
    );
    assert_eq!(saved.tags().values(), current.tags().values());
    owner.shutdown(deadline()).await?;
    let mut actual = serde_json::to_value(target.export_snapshot(SnapshotLimits::default())?)?;
    // UTC expiry is projected from the current paired clock; event/identity instants remain exact.
    for document in [&mut actual, &mut expected] {
        for execution in document
            .get_mut("executions")
            .and_then(Value::as_array_mut)
            .ok_or("missing executions")?
        {
            execution
                .as_object_mut()
                .ok_or("invalid execution")?
                .remove("expiresAt");
        }
    }
    for key in ["resources", "revisions", "machines", "executions"] {
        assert_eq!(actual.get(key), expected.get(key));
    }
    Ok(())
}

#[tokio::test]
async fn test_should_reject_history_limit_without_retaining_partial_charges() -> TestResult {
    let runtime = ExecutionSupervisor::new(
        RuntimeLimits::default(),
        HistoryLimits::new(2, 1024 * 1024)?,
        8 * 1024 * 1024,
        None,
    )?;
    let (target, mut owner) = Registry::new(
        TenantScope::default(),
        RegistryLimits::default(),
        runtime,
        ExecutionLimits::default(),
    )?;
    assert!(
        target
            .prepare_snapshot(document().await?, SnapshotLimits::default())
            .is_err()
    );
    assert_eq!(target.shared.restore.history.used(), 0);
    assert!(target.status().ready);
    let mut interrupted = document().await?;
    let row = interrupted
        .executions
        .first_mut()
        .ok_or("missing execution")?;
    row.history.pop();
    row.outcome = Outcome::Running {};
    row.stopped_at = None;
    row.expires_at = None;
    interrupted
        .names
        .first_mut()
        .ok_or("missing name")?
        .expires_at = None;
    assert!(
        target
            .prepare_snapshot(interrupted, SnapshotLimits::default())
            .is_err()
    );
    assert_eq!(target.shared.restore.history.used(), 0);
    owner.shutdown(deadline()).await?;
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn test_should_restore_failure_abort_and_timeout_outcomes_with_exact_terminal_history()
-> TestResult {
    for kind in ["failed", "aborted", "timed-out"] {
        let limits = ExecutionLimits::new(Duration::from_secs(1), 100)?;
        let runtime = ExecutionSupervisor::new(
            RuntimeLimits::default(),
            HistoryLimits::default(),
            256 * 1024 * 1024,
            None,
        )?;
        let (source, mut owner) = Registry::new(
            TenantScope::default(),
            RegistryLimits::default(),
            runtime,
            limits,
        )?;
        let value = if kind == "failed" {
            json!({"StartAt":"F","States":{"F":{"Type":"Fail","Error":"","Cause":"explicit cause"}}})
        } else {
            json!({"StartAt":"W","States":{"W":{"Type":"Wait","Seconds":60,"End":true}}})
        };
        let prepared = PreparedDefinition::new(
            AslCompiler::default()
                .compile(&serde_json::to_vec(&value)?)?
                .machine()
                .clone(),
        );
        result(
            &source,
            RegistryCommand::Create("Machine".parse()?, prepared, role(), Tags::default()),
        )
        .await?;
        let execution = start(&source, "stored", "{}").await?;
        if kind == "aborted" {
            source
                .stop(
                    execution.arn().clone(),
                    None,
                    Some(String::new()),
                    deadline(),
                )
                .await?;
        }
        observe(execution).await;
        owner.shutdown(deadline()).await?;
        let snapshot = source.export_snapshot(SnapshotLimits::default())?;
        let expected = serde_json::to_value(&snapshot)?;
        let expected_status = match kind {
            "failed" => "FAILED",
            "aborted" => "ABORTED",
            _ => "TIMED_OUT",
        };
        assert_eq!(
            expected.pointer("/executions/0/outcome/status"),
            Some(&json!(expected_status))
        );
        let (target, mut owner) = registry(RegistryLimits::default(), None);
        restored(&target, snapshot).await?;
        owner.shutdown(deadline()).await?;
        let actual = serde_json::to_value(target.export_snapshot(SnapshotLimits::default())?)?;
        assert_eq!(
            actual.pointer("/executions/0/outcome"),
            expected.pointer("/executions/0/outcome")
        );
        assert_eq!(
            actual.pointer("/executions/0/history"),
            expected.pointer("/executions/0/history")
        );
    }
    Ok(())
}
