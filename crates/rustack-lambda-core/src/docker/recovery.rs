//! Resumable historical container cleanup; unknown absence is never settlement.

use http::StatusCode;
use tokio::time::Instant;

pub use self::{
    code::CodeRecovery,
    images::{ImagePinRecovery, reconcile_image_pins},
    namespace::{NamespaceRecovery, reconcile_namespace},
};
use super::{
    DirectoryLease, DockerError, DockerPort, EnvironmentError, check_deadline,
    ownership::ContainerRecord,
};

mod code;
mod images;
pub(super) mod inventory;
mod namespace;

/// Container-only recovery accounting, not a complete backend readiness proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ContainerRecovery {
    /// Records durably erased during this pass.
    pub settled: usize,
    /// Records still retained, including current-run records never touched here.
    pub remaining: usize,
}

/// Reconcile previous-run recorded containers using exact identity and ID proofs.
///
/// Unknown404 responses retain the record; unresolved functions retain their
/// relay. Every destructive request follows durable publication of the full ID.
/// Cancellation or errors preserve records for a subsequent pass. This function
/// never creates containers and never touches current-run records.
///
/// # Errors
/// Returns invalid/expired budgets, unsafe/corrupt/unavailable ownership storage,
/// daemon transport/protocol failures or foreign resource identity. No error
/// grants readiness, and previous successfully settled records remain settled.
/// Requires a port from `DockerPort::bind_directory` for this exact lease.
///
/// ```no_run
/// # async fn recover(directory: &rustack_lambda_core::docker::DirectoryLease, docker: &rustack_lambda_core::docker::DockerPort) -> Result<(), rustack_lambda_core::docker::EnvironmentError> {
/// let report = rustack_lambda_core::docker::reconcile_containers(directory, docker,
///     tokio::time::Instant::now() + std::time::Duration::from_secs(30)).await?;
/// assert_eq!(report.remaining, 0);
/// // Image aliases and the other startup proofs still require qualification.
/// # Ok(()) }
/// ```
pub async fn reconcile_containers(
    directory: &DirectoryLease,
    docker: &DockerPort,
    deadline: Instant,
) -> Result<ContainerRecovery, EnvironmentError> {
    docker.require_directory(directory)?;
    check_deadline(deadline)?;
    let mut records = directory.container_records(deadline).await?;
    records.sort_by_key(|record| (!record.function(), record.environment));
    let mut report = ContainerRecovery {
        settled: 0,
        remaining: records.len(),
    };
    let mut blocked = Vec::new();
    for record in records {
        check_deadline(deadline)?;
        if record.run == directory.run_id() {
            continue;
        }
        if !record.function() && blocked.contains(&record.environment) {
            continue;
        }
        if Box::pin(recover(directory, docker, record.clone(), deadline)).await? {
            report.settled += 1;
            report.remaining -= 1;
        } else if record.function() {
            blocked.push(record.environment);
        }
    }
    check_deadline(deadline)?;
    Ok(report)
}

async fn recover(
    directory: &DirectoryLease,
    docker: &DockerPort,
    mut record: ContainerRecord,
    deadline: Instant,
) -> Result<bool, EnvironmentError> {
    let key = record.known_key()?.unwrap_or(record.key()?);
    let inspected = match docker.inspect_container(&key, deadline).await {
        Err(DockerError::Status(StatusCode::NOT_FOUND)) if record.known.is_some() => {
            directory.settle_container(record, deadline).await?;
            return Ok(true);
        }
        Err(DockerError::Status(StatusCode::NOT_FOUND)) => return Ok(false),
        result => result?,
    };
    let id = record.owned(directory.owner_id(), &inspected)?;
    directory
        .know_container(record.clone(), id.clone(), deadline)
        .await?;
    // Re-read the durable identity so subsequent mutation never relies on an
    // unpersisted local success or a name which can be reused.
    record = directory
        .container_records(deadline)
        .await?
        .into_iter()
        .find(|saved| {
            saved.environment == record.environment && saved.function() == record.function()
        })
        .ok_or(DockerError::Quarantined)?;
    let current = match docker.inspect_container(&id, deadline).await {
        Err(DockerError::Status(StatusCode::NOT_FOUND)) => {
            directory.settle_container(record, deadline).await?;
            return Ok(true);
        }
        result => result?,
    };
    record.owned(directory.owner_id(), &current)?;
    match current.state.and_then(|state| state.running) {
        Some(true) => {
            let _ = docker.kill_container(&id, deadline).await;
            let exit = docker.wait_container(&id, deadline).await?;
            if exit.error.is_some() {
                return Err(DockerError::Quarantined.into());
            }
        }
        Some(false) => {}
        None => return Err(DockerError::Protocol.into()),
    }
    check_deadline(deadline)?;
    let _ = docker.remove_container(&id, deadline).await;
    match docker.inspect_container(&id, deadline).await {
        Err(DockerError::Status(StatusCode::NOT_FOUND)) => {
            directory.settle_container(record, deadline).await?;
            Ok(true)
        }
        Ok(_) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests;
