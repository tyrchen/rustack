//! Startup-only composition; inventories cannot settle unproven mutations.

use std::time::Duration;

use tokio::time::Instant;

use super::{
    ContainerRecovery, ImagePinRecovery, inventory, reconcile_containers, reconcile_image_pins,
};
use crate::docker::{DirectoryLease, DockerError, DockerPort, EnvironmentError, check_deadline};

/// Recorded retirement counters; this is not a readiness capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct NamespaceRecovery {
    /// Historical container records settled/retained by this pass.
    pub containers: ContainerRecovery,
    /// Historical pin records settled/retained by this pass.
    pub pins: ImagePinRecovery,
}

/// Drain recorded historical effects and audit the complete observed namespace.
///
/// Call before starting any runtime producer, under one exclusive coordinator.
/// Unknown effects remain recorded; inventories never authorize adoption or
/// prove absence of unseen pre-journal legacy mutations. Earlier durable cleanup
/// may succeed before a later error; repeating the call resumes that state.
///
/// # Errors
/// Rejects unbound/foreign ports, current-run work, unrecorded owned resources,
/// inventory bounds, storage/wire/identity failures and expired budgets.
///
/// ```no_run
/// # async fn recover(directory: &rustack_lambda_core::docker::DirectoryLease, port: &rustack_lambda_core::docker::DockerPort) -> Result<(), rustack_lambda_core::docker::EnvironmentError> {
/// let bound = port.bind_directory(directory, tokio::time::Instant::now() + std::time::Duration::from_secs(10)).await?;
/// let report = rustack_lambda_core::docker::reconcile_namespace(directory, &bound,
///     tokio::time::Instant::now() + std::time::Duration::from_secs(30)).await?;
/// assert_eq!(report.containers.remaining, 0);
/// assert_eq!(report.pins.remaining, 0);
/// // Startup must still qualify pulls, resources, networking and provenance.
/// # Ok(()) }
/// ```
pub async fn reconcile_namespace(
    directory: &DirectoryLease,
    docker: &DockerPort,
    deadline: Instant,
) -> Result<NamespaceRecovery, EnvironmentError> {
    docker.require_directory(directory)?;
    check_deadline(deadline)?;
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .ok_or(DockerError::Deadline)?;
    if remaining > Duration::from_secs(60) {
        return Err(DockerError::Configuration.into());
    }
    let containers = directory.container_records(deadline).await?;
    let pins = directory.image_records(deadline).await?;
    if containers
        .iter()
        .any(|record| record.run == directory.run_id())
        || pins.iter().any(|record| record.run == directory.run_id())
    {
        return Err(DockerError::Quarantined.into());
    }
    let containers = reconcile_containers(directory, docker, deadline).await?;
    let mut report = NamespaceRecovery {
        containers,
        pins: ImagePinRecovery {
            settled: 0,
            remaining: pins.len(),
        },
    };
    if containers.remaining != 0 {
        check_deadline(deadline)?;
        return Ok(report);
    }
    inventory::containers(directory, docker, deadline).await?;
    report.pins = reconcile_image_pins(directory, docker, deadline).await?;
    if report.pins.remaining != 0 {
        check_deadline(deadline)?;
        return Ok(report);
    }
    inventory::images(directory, docker, deadline).await?;
    inventory::containers(directory, docker, deadline).await?;
    if !directory.container_records(deadline).await?.is_empty()
        || !directory.image_records(deadline).await?.is_empty()
    {
        return Err(DockerError::Quarantined.into());
    }
    check_deadline(deadline)?;
    Ok(report)
}

#[cfg(test)]
mod tests;
