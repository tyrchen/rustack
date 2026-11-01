//! Loss of service observers preserves prepared graphs and actual IO retirement.

use rustack_stepfunctions_core::{
    ExecutionLimits, ExecutionSupervisor, HistoryLimits, Payload, RegistryCapacities,
    RegistryCommand, RegistryLimits, RegistrySupervisor, RuntimeLimits, StateSupport, TenantScope,
};
use tokio::time::{advance, pause, resume, timeout};

use super::*;

fn registry() -> Result<(Registry, RegistrySupervisor)> {
    let runtime = ExecutionSupervisor::new(
        RuntimeLimits::default(),
        HistoryLimits::new(25000, 1024 * 1024)?,
        8 * 1024 * 1024,
        None,
    )?;
    Ok(Registry::new_with_state_support(
        TenantScope::default(),
        RegistryLimits::new(
            RegistryCapacities::new(1, 2, 1, 2)?,
            Duration::from_secs(60),
            8,
        )?,
        runtime,
        ExecutionLimits::default(),
        StateSupport::Structured,
    )?)
}
#[tokio::test]
async fn test_should_keep_prepared_graph_and_commit_once_after_service_observer_drop() -> Result<()>
{
    let directory = tempfile::tempdir()?;
    let mut source = runtime(RuntimeBudgets::default())?;
    let (registry, mut registry_owner) = registry()?;
    let hooks = Hooks::new(true, false);
    let release = Release(hooks.clone());
    let (archive, mut owner) = WorkflowArchive::new_owned(
        registry.clone(),
        1024 * 1024,
        Duration::from_secs(10),
        Some(hooks.clone()),
    )?;
    let result = async {
        start(&source, 0).await?;
        source.owner.quiesce(deadline()).await?;
        let metadata = source.archive.save_meta(directory.path()).await?;
        let client = archive.clone();
        let path = directory.path().to_owned();
        let observer = tokio::spawn(async move { client.load_meta(metadata, &path).await });
        timeout(Duration::from_secs(2), hooks.cpu_entered.notified()).await?;
        assert!(
            !registry.status().ready,
            "prepared lease did not block business admission"
        );
        assert!(registry.submit(RegistryCommand::Machines).is_err());
        observer.abort();
        assert!(
            observer
                .await
                .err()
                .is_some_and(|error| error.is_cancelled())
        );
        assert!(
            owner
                .shutdown(Instant::now() + Duration::from_millis(5))
                .await
                .is_err()
        );
        assert!(!registry.status().ready);
        hooks.release();
        owner.shutdown(deadline()).await?;
        let rows = registry
            .submit(RegistryCommand::Machines)?
            .wait(deadline())
            .await?;
        let RegistryResult::Machines(machines) = rows else {
            anyhow::bail!("missing committed machine");
        };
        assert_eq!(machines.len(), 1);
        let machine = machines.first().context("missing imported machine")?;
        let capacity = registry
            .submit(RegistryCommand::Start(
                machine.arn().clone(),
                Some("next".parse()?),
                Payload::parse(b"{}")?,
                None,
            ))?
            .wait(deadline())
            .await;
        assert!(
            matches!(capacity, Err(RegistryError::Capacity)),
            "import bypassed original record pool"
        );
        Ok(())
    }
    .await;
    drop(release);
    let archive = owner.shutdown(deadline()).await;
    let records = registry_owner.shutdown(deadline()).await;
    let original = source.owner.shutdown(deadline()).await;
    archive?;
    records?;
    original?;
    result
}
#[tokio::test]
async fn test_should_join_expired_io_after_observer_drop_without_starting_preparation() -> Result<()>
{
    let directory = tempfile::tempdir()?;
    let mut source = runtime(RuntimeBudgets::default())?;
    let (registry, mut registry_owner) = registry()?;
    let hooks = Hooks::new(false, true);
    let release = Release(hooks.clone());
    let (archive, mut owner) = WorkflowArchive::new_owned(
        registry.clone(),
        1024 * 1024,
        Duration::from_secs(1),
        Some(hooks.clone()),
    )?;
    let mut paused = false;
    let result = async {
        start(&source, 0).await?;
        source.owner.quiesce(deadline()).await?;
        let metadata = source.archive.save_meta(directory.path()).await?;
        pause();
        paused = true;
        let client = archive.clone();
        let path = directory.path().to_owned();
        let observer = tokio::spawn(async move { client.load_meta(metadata, &path).await });
        timeout(Duration::from_secs(2), hooks.io_entered.notified()).await?;
        advance(Duration::from_secs(2)).await;
        resume();
        paused = false;
        let mut ready = archive.ready.clone();
        timeout(Duration::from_secs(2), ready.wait_for(|ready| !*ready)).await??;
        observer.abort();
        assert!(
            observer
                .await
                .err()
                .is_some_and(|error| error.is_cancelled())
        );
        assert!(
            owner
                .shutdown(Instant::now() + Duration::from_millis(5))
                .await
                .is_err()
        );
        assert!(registry.status().ready, "IO delay started preparation");
        hooks.release();
        owner.shutdown(deadline()).await?;
        let rows = registry
            .submit(RegistryCommand::Machines)?
            .wait(deadline())
            .await?;
        assert!(matches!(rows,RegistryResult::Machines(machines) if machines.is_empty()));
        Ok(())
    }
    .await;
    if paused {
        resume();
    }
    drop(release);
    let archive = owner.shutdown(deadline()).await;
    let records = registry_owner.shutdown(deadline()).await;
    let original = source.owner.shutdown(deadline()).await;
    archive?;
    records?;
    original?;
    result
}
