//! One paired clock cut controls both maintenance and durable TTL projection.

use std::{error::Error, sync::Arc, time::Duration};

use chrono::{TimeDelta, Utc};
use serde_json::{Value, json};
use tokio::time::{Instant, advance};

use super::{create, definition, owner, settled, start};
use crate::{
    ClockSample, LambdaTaskCommand, LambdaTaskPort, RegistryCapacities, SnapshotLimits, TaskCall,
    TaskFailure,
};

type TestResult = Result<(), Box<dyn Error>>;

#[tokio::test(start_paused = true)]
async fn test_should_project_fractional_ttls_from_the_same_cut_despite_delayed_sealing()
-> TestResult {
    let mut root = owner(RegistryCapacities::default(), None);
    let mut state = root.state.take().ok_or("missing test state")?;
    let arn = create(&mut state, definition());
    let execution = start(&mut state, &arn, "clock")?;
    settled(&execution).await;
    state
        .runtime
        .drain(Instant::now() + Duration::from_secs(5))
        .await?;
    let retention = Duration::from_nanos(60_123_456_789);
    state.resources.limits.retention = retention;
    let wall = Utc::now()
        .checked_add_signed(TimeDelta::minutes(1))
        .ok_or("invalid test clock")?;
    let clock = ClockSample::new(Instant::now().into_std(), wall);
    state.maintain(Instant::from_std(clock.monotonic));
    advance(Duration::from_secs(10)).await;
    let capture = state.seal_at(clock)?;
    let archive = serde_json::to_value(capture.export(SnapshotLimits::default())?)?;
    assert_eq!(
        archive.get("capturedAt"),
        Some(&serde_json::to_value(wall)?)
    );
    let record_expiry = wall
        .checked_add_signed(TimeDelta::from_std(retention)?)
        .ok_or("invalid test expiry")?;
    let name_expiry = wall
        .checked_add_signed(TimeDelta::hours(2160))
        .ok_or("invalid name expiry")?;
    assert_eq!(
        archive.pointer("/executions/0/expiresAt"),
        Some(&serde_json::to_value(record_expiry)?)
    );
    assert_eq!(
        archive.pointer("/names/0/expiresAt"),
        Some(&serde_json::to_value(name_expiry)?)
    );
    Ok(())
}

#[tokio::test]
async fn test_should_export_name_reservation_after_independent_record_expiry() -> TestResult {
    let mut root = owner(RegistryCapacities::default(), None);
    let mut state = root.state.take().ok_or("missing test state")?;
    let arn = create(&mut state, definition());
    let execution = start(&mut state, &arn, "retained-name")?;
    settled(&execution).await;
    state
        .runtime
        .drain(Instant::now() + Duration::from_secs(5))
        .await?;
    let closed = Instant::now();
    let wall = Utc::now();
    state.maintain(closed);
    let capture = state.seal_at(ClockSample::new(
        (closed + Duration::from_secs(61)).into_std(),
        wall + TimeDelta::seconds(61),
    ))?;
    let archive = serde_json::to_value(capture.export(SnapshotLimits::default())?)?;
    assert_eq!(archive.get("executions"), Some(&json!([])));
    assert_eq!(
        archive.pointer("/names/0/arn"),
        Some(&json!(execution.arn().as_str()))
    );
    assert_eq!(
        archive.pointer("/names/0/expiresAt"),
        Some(&serde_json::to_value(wall + TimeDelta::hours(2160))?)
    );
    assert_eq!(
        archive
            .get("resources")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(1)
    );
    Ok(())
}

#[derive(Debug)]
struct PanicAfterDrain;
impl LambdaTaskPort for PanicAfterDrain {
    fn start(&self, _command: LambdaTaskCommand) -> Result<TaskCall, TaskFailure> {
        Err(TaskFailure::service_exception())
    }
}
impl Drop for PanicAfterDrain {
    fn drop(&mut self) {
        panic!("injected drained port destruction failure");
    }
}

#[tokio::test]
async fn test_should_keep_allocation_permits_charged_when_drained_state_destruction_panics()
-> TestResult {
    let mut root = owner(
        RegistryCapacities::default(),
        Some(Arc::new(PanicAfterDrain)),
    );
    let state = root.state.as_mut().ok_or("missing owner state")?;
    let arn = create(state, definition());
    let execution = start(state, &arn, "retained-permits")?;
    settled(&execution).await;
    let revision = Arc::downgrade(execution.revision());
    let record = Arc::downgrade(&execution.record);
    let revisions = state.resources.revisions.clone();
    let records = state.executions.capacity.clone();
    let revision_count = revisions.available_permits();
    let record_count = records.available_permits();
    let shared = state.children.registry.clone();
    let closing = shared.closing.clone();
    drop(execution);
    closing.send_replace(true);
    let result = tokio::spawn(root.run(closing.subscribe())).await;
    assert!(result.is_err());
    assert!(shared.sealed.borrow().is_none());
    assert!(revision.upgrade().is_some());
    assert!(record.upgrade().is_some());
    assert_eq!(revisions.available_permits(), revision_count);
    assert_eq!(records.available_permits(), record_count);
    Ok(())
}
