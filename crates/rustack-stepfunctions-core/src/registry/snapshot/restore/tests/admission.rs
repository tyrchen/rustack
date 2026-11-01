//! Startup lease, exact owner and retained accepted commit contracts.

use std::{sync::Arc, time::Duration};

use chrono::Utc;
use tokio::{
    task::yield_now,
    time::{Instant, timeout_at},
};

use super::*;
use crate::{
    AslCompiler, CallbackAttempt, CallbackError, ChildError, ChildPattern, ChildStartParameters,
    ClockSample, DriverStep, EvaluationContext, ExecutionDriver, ExecutionLimits, FrameCapacity,
    FrameLimits, FrameScheduler, FrameUpdate, Payload, RegistryCapacities, RegistryError,
    SystemUuidSource, TaskSupport, TenantScope,
    registry::tests::{definition, role},
};

#[tokio::test]
async fn test_should_release_prepared_quotas_before_reopening_startup() -> TestResult {
    let (target, mut owner) = registry(RegistryLimits::default(), None);
    let revisions = target.shared.restore.pools.revisions.available_permits();
    let records = target.shared.restore.pools.records.available_permits();
    let tags = target.shared.restore.pools.tags.available_permits();
    let prepared = target.prepare_snapshot(document().await?, SnapshotLimits::default())?;
    assert!(!target.status().ready);
    assert!(target.shared.restore.history.used() > 0);
    assert_eq!(
        target.shared.restore.pools.revisions.available_permits(),
        revisions - 1
    );
    assert_eq!(
        target.shared.restore.pools.records.available_permits(),
        records - 1
    );
    assert_eq!(
        target.shared.restore.pools.tags.available_permits(),
        tags - 1
    );
    assert!(matches!(
        target.submit(RegistryCommand::Machines),
        Err(RegistryError::Unavailable)
    ));
    assert!(matches!(
        target.prepare_snapshot(document().await?, SnapshotLimits::default()),
        Err(RegistryError::Unavailable)
    ));
    drop(prepared);
    assert!(target.status().ready);
    assert_eq!(target.shared.restore.history.used(), 0);
    assert_eq!(
        target.shared.restore.pools.revisions.available_permits(),
        revisions
    );
    assert_eq!(
        target.shared.restore.pools.records.available_permits(),
        records
    );
    assert_eq!(target.shared.restore.pools.tags.available_permits(), tags);
    restored(&target, document().await?).await?;
    owner.shutdown(deadline()).await?;
    Ok(())
}

#[tokio::test]
async fn test_should_reject_foreign_preparation_and_release_its_source_lease() -> TestResult {
    let (first, mut one) = registry(RegistryLimits::default(), None);
    let (second, mut two) = registry(RegistryLimits::default(), None);
    let prepared = first.prepare_snapshot(document().await?, SnapshotLimits::default())?;
    assert!(matches!(
        second.restore_snapshot(prepared),
        Err(RegistryError::Validation)
    ));
    assert!(first.status().ready);
    assert_eq!(first.shared.restore.history.used(), 0);
    restored(&first, document().await?).await?;
    restored(&second, document().await?).await?;
    one.shutdown(deadline()).await?;
    two.shutdown(deadline()).await?;
    Ok(())
}

#[tokio::test]
async fn test_should_keep_startup_open_for_invalid_domain_but_close_after_valid_business()
-> TestResult {
    let (target, mut owner) = registry(RegistryLimits::default(), None);
    let foreign = "arn:aws:states:us-west-2:000000000000:stateMachine:Machine".parse()?;
    assert!(target.submit(RegistryCommand::Machine(foreign)).is_err());
    let prepared = target.prepare_snapshot(document().await?, SnapshotLimits::default())?;
    drop(prepared);
    assert!(
        matches!(result(&target, RegistryCommand::Machines).await?, RegistryResult::Machines(rows) if rows.is_empty())
    );
    assert!(matches!(
        target.prepare_snapshot(document().await?, SnapshotLimits::default()),
        Err(RegistryError::Unavailable)
    ));
    owner.shutdown(deadline()).await?;
    Ok(())
}

#[tokio::test]
async fn test_should_commit_after_receipt_loss_and_prevent_second_restore() -> TestResult {
    let (target, mut owner) = registry(RegistryLimits::default(), None);
    let prepared = target.prepare_snapshot(document().await?, SnapshotLimits::default())?;
    drop(target.restore_snapshot(prepared)?);
    // Receipt loss releases observation only; this actor read is ordered after commit.
    timeout_at(deadline(), async {
        while !target.status().ready {
            yield_now().await;
        }
    })
    .await?;
    assert!(matches!(
        result(&target, RegistryCommand::Machine(arn())).await?,
        RegistryResult::Machine(_)
    ));
    assert!(matches!(
        target.prepare_snapshot(document().await?, SnapshotLimits::default()),
        Err(RegistryError::Unavailable)
    ));
    owner.shutdown(deadline()).await?;
    Ok(())
}

#[tokio::test]
async fn test_should_release_rejected_closed_or_saturated_preparation() -> TestResult {
    for close in [false, true] {
        let (target, mut owner) = registry(RegistryLimits::default(), None);
        let prepared = target.prepare_snapshot(document().await?, SnapshotLimits::default())?;
        let permit = target.shared.capacity.clone().try_acquire_many_owned(128)?;
        if close {
            owner.shutdown(deadline()).await?;
        }
        assert!(target.restore_snapshot(prepared).is_err());
        assert_eq!(target.shared.restore.history.used(), 0);
        assert!(!target.shared.restore.preparing());
        drop(permit);
        if !close {
            restored(&target, document().await?).await?;
            owner.shutdown(deadline()).await?;
        }
    }
    Ok(())
}

#[tokio::test]
async fn test_should_hold_imported_revision_charge_through_caller_snapshot() -> TestResult {
    let (target, mut owner) = registry(RegistryLimits::default(), None);
    restored(&target, document().await?).await?;
    let RegistryResult::Machine(saved) = result(&target, RegistryCommand::Machine(arn())).await?
    else {
        return Err("missing machine".into());
    };
    let before = target.shared.restore.pools.revisions.available_permits();
    result(&target, RegistryCommand::Update(arn(), None, Some(role()))).await?;
    assert_eq!(
        target.shared.restore.pools.revisions.available_permits(),
        before - 1
    );
    // The old execution and this read view still retain the old revision.
    assert_eq!(saved.revision().id().len(), 36);
    owner.shutdown(deadline()).await?;
    Ok(())
}

#[tokio::test]
async fn test_should_rollback_partially_charged_replacement_on_quota_exhaustion() -> TestResult {
    let (source, mut source_owner) = registry(RegistryLimits::default(), None);
    machine(&source, 1, false).await;
    observe(start(&source, "one", "{}").await?).await;
    result(
        &source,
        RegistryCommand::Update(arn(), Some(definition(2, false)), None),
    )
    .await?;
    observe(start(&source, "two", "{}").await?).await;
    source_owner.shutdown(deadline()).await?;
    let value = serde_json::to_value(source.export_snapshot(SnapshotLimits::default())?)?;
    for capacities in [
        RegistryCapacities::new(1, 1, 2, 2)?,
        RegistryCapacities::new(1, 2, 1, 2)?,
        RegistryCapacities::new(1, 2, 2, 1)?,
    ] {
        let limits = RegistryLimits::new(capacities, Duration::from_secs(60), 128)?;
        let (target, mut owner) = registry(limits, None);
        let before = [
            target.shared.restore.pools.revisions.available_permits(),
            target.shared.restore.pools.tags.available_permits(),
            target.shared.restore.pools.records.available_permits(),
        ];
        assert!(matches!(
            target.prepare_snapshot(
                serde_json::from_value(value.clone())?,
                SnapshotLimits::default()
            ),
            Err(RegistryError::Capacity)
        ));
        assert!(target.status().ready);
        assert_eq!(target.shared.restore.history.used(), 0);
        assert_eq!(
            [
                target.shared.restore.pools.revisions.available_permits(),
                target.shared.restore.pools.tags.available_permits(),
                target.shared.restore.pools.records.available_permits()
            ],
            before
        );
        restored(&target, document().await?).await?;
        owner.shutdown(deadline()).await?;
    }
    Ok(())
}

#[tokio::test]
async fn test_should_reject_restored_records_as_callback_and_child_parents() -> TestResult {
    let snapshot = document().await?;
    let execution = snapshot
        .executions
        .first()
        .ok_or("missing execution")?
        .arn
        .parse()?;
    let (target, mut owner) = registry(RegistryLimits::default(), None);
    restored(&target, snapshot).await?;
    let RegistryResult::Execution(parent) =
        result(&target, RegistryCommand::Execution(execution)).await?
    else {
        return Err("missing parent".into());
    };
    let parameters = ChildStartParameters::parse(
        &TenantScope::default(),
        &Payload::parse(
            br#"{"StateMachineArn":"arn:aws:states:us-east-1:000000000000:stateMachine:Machine"}"#,
        )?,
    )?;
    assert!(matches!(
        target.child_port().start(
            parent.child_parent(),
            parameters,
            ChildPattern::Sync2,
            deadline()
        ),
        Err(ChildError::Invalid)
    ));
    let machine = AslCompiler::new(TenantScope::default(), TaskSupport::Lambda)
        .compile(br#"{"StartAt":"T","States":{"T":{"Type":"Task","Resource":"arn:aws:lambda:us-east-1:000000000000:function:Worker","End":true}}}"#)?.machine().clone();
    let now = ClockSample::new(Instant::now().into_std(), Utc::now());
    let driver = ExecutionDriver::for_structured_owner(
        machine,
        Payload::parse(b"{}")?,
        EvaluationContext::from_value(&json!({}))?,
        now,
        ExecutionLimits::default(),
    )?;
    let mut frames = FrameScheduler::new(
        driver,
        Arc::new(FrameCapacity::default()),
        FrameLimits::default(),
    )?;
    let mut source = SystemUuidSource::default();
    let mut call = None;
    for _ in 0..10 {
        if let FrameUpdate::Transition(transition) = frames.advance(now, &mut source) {
            if matches!(transition.step(), DriverStep::Dispatch(_)) {
                call = transition.call();
                break;
            }
        }
    }
    assert!(matches!(
        CallbackAttempt::new(
            &parent,
            call.ok_or("missing call identity")?,
            deadline(),
            None
        ),
        Err(CallbackError::TaskTimedOut)
    ));
    owner.shutdown(deadline()).await?;
    Ok(())
}
