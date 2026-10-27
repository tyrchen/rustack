//! Ready-before-timer ordering and retained typed worker failures.

use std::sync::Arc;

use rustack_stepfunctions_core::{
    ExecutionLimits, ExecutionSupervisor, HistoryLimits, Registry, RegistryLimits,
    RegistrySupervisor, RuntimeLimits, TenantScope,
};
use tokio::{
    sync::{Notify, mpsc, watch},
    task::JoinHandle,
    time::advance,
};

use super::*;

fn owner() -> Result<(Owner, RegistrySupervisor)> {
    let runtime = ExecutionSupervisor::new(
        RuntimeLimits::default(),
        HistoryLimits::new(25000, 1024 * 1024)?,
        8 * 1024 * 1024,
        None,
    )?;
    let (registry, supervisor) = Registry::new(
        TenantScope::default(),
        RegistryLimits::default(),
        runtime,
        ExecutionLimits::default(),
    )?;
    let (_, inbox) = mpsc::channel(1);
    let (closing, _) = watch::channel(false);
    let (ready, _) = watch::channel(true);
    Ok((
        Owner {
            registry,
            bytes: 1024 * 1024,
            inbox,
            closing,
            ready,
            active: None,
            cpu: None,
            io: None,
            receipt: None,
            fatal: false,
            hooks: None,
        },
        supervisor,
    ))
}
#[tokio::test(start_paused = true)]
async fn test_should_reject_ready_io_first_observed_after_absolute_deadline() -> Result<()> {
    let (mut owner, mut registry) = owner()?;
    let gate = Arc::new(Notify::new());
    let signal = gate.clone();
    owner.io = Some(tokio::spawn(async move {
        signal.notify_one();
        Ok(io::Output::Written)
    }));
    let deadline = Instant::now() + Duration::from_secs(1);
    gate.notified().await;
    assert!(owner.io.as_ref().is_some_and(JoinHandle::is_finished));
    advance(Duration::from_secs(2)).await;
    assert!(owner.join_io(deadline).await.is_err());
    assert!(owner.io.is_none(), "physical task was not joined");
    drop(owner);
    registry
        .shutdown(Instant::now() + Duration::from_secs(1))
        .await?;
    Ok(())
}
fn panic_source() -> Result<Materialized> {
    panic!("private-cpu-payload");
}
fn panic_io() -> Result<io::Output> {
    panic!("private-io-payload");
}
#[tokio::test]
async fn test_should_retain_typed_cpu_and_io_failures() -> Result<()> {
    for cpu in [false, true] {
        let (mut owner, mut registry) = owner()?;
        let result = if cpu {
            owner.cpu = Some(spawn_blocking(panic_source));
            owner.join_cpu().await.map(|_| ())
        } else {
            owner.io = Some(tokio::spawn(async { panic_io() }));
            owner
                .join_io(Instant::now() + Duration::from_secs(2))
                .await
                .map(|_| ())
        };
        let error = result.err().context("missing worker failure")?;
        assert!(
            error
                .downcast_ref::<WorkerFailure>()
                .is_some_and(|failure| failure.join_error().is_panic())
        );
        assert!(!format!("{error:#?}").contains("private-"));
        assert!(owner.fatal);
        drop(owner);
        registry
            .shutdown(Instant::now() + Duration::from_secs(2))
            .await?;
    }
    Ok(())
}
