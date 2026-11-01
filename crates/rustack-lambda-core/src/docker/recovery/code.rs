//! Historical loader-before-volume retirement; unknown absence retains pins.

use bytes::Bytes;
use http::{Method, StatusCode};
use tokio::time::Instant;

use crate::docker::{
    DirectoryLease, DockerError, DockerPort, Operation, check_deadline, escaped,
    ownership::codes::{CodeRecord, LoaderState, VolumeState},
    unit,
};

/// Durable code-record accounting; neither resource counts nor readiness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct CodeRecovery {
    /// Records durably erased by this pass after both effects retired.
    pub settled: usize,
    /// Records still retained, including unresolved issued effects.
    pub remaining: usize,
}

pub(super) async fn reconcile_code(
    directory: &DirectoryLease,
    docker: &DockerPort,
    deadline: Instant,
) -> Result<CodeRecovery, DockerError> {
    docker.require_directory(directory)?;
    // Namespace recovery calls us only after recorded consumers have joined.
    if !directory.container_records(deadline).await?.is_empty() {
        return Err(DockerError::Quarantined);
    }
    let records = directory.code_records(deadline).await?;
    if records
        .iter()
        .any(|record| record.run == directory.run_id())
    {
        return Err(DockerError::Quarantined);
    }
    let mut report = CodeRecovery {
        settled: 0,
        remaining: records.len(),
    };
    for record in records {
        check_deadline(deadline)?;
        if recover(directory, docker, record, deadline).await? {
            report.settled = report.settled.checked_add(1).ok_or(DockerError::Protocol)?;
            report.remaining = report
                .remaining
                .checked_sub(1)
                .ok_or(DockerError::Protocol)?;
        }
    }
    check_deadline(deadline)?;
    Ok(report)
}
async fn save(
    directory: &DirectoryLease,
    record: &mut CodeRecord,
    next: CodeRecord,
    deadline: Instant,
) -> Result<(), DockerError> {
    directory
        .advance_code(record.clone(), next.clone(), deadline)
        .await?;
    *record = next;
    Ok(())
}
async fn recover(
    directory: &DirectoryLease,
    docker: &DockerPort,
    mut record: CodeRecord,
    deadline: Instant,
) -> Result<bool, DockerError> {
    if retire_effects(directory, docker, &mut record, deadline).await? {
        directory.settle_code(record, deadline).await?;
        Ok(true)
    } else {
        Ok(false)
    }
}
pub(in crate::docker) async fn retire_effects(
    directory: &DirectoryLease,
    docker: &DockerPort,
    record: &mut CodeRecord,
    deadline: Instant,
) -> Result<bool, DockerError> {
    if record.volume == VolumeState::Planned {
        let mut next = record.clone();
        next.volume = VolumeState::Retired;
        next.loader = LoaderState::Retired;
        save(directory, record, next, deadline).await?;
    }
    if record.volume == VolumeState::Uncertain {
        let inspected = match docker.inspect_volume(&record.volume_key()?, deadline).await {
            Err(DockerError::Status(StatusCode::NOT_FOUND)) => return Ok(false),
            result => result?,
        };
        record.owned_volume(directory.owner_id(), &inspected)?;
        let mut next = record.clone();
        next.volume = VolumeState::Known;
        save(directory, record, next, deadline).await?;
    }
    if !Box::pin(retire_loader(directory, docker, record, deadline)).await? {
        return Ok(false);
    }
    if !retire_volume(directory, docker, record, deadline).await? {
        return Ok(false);
    }
    Ok(true)
}
pub(in crate::docker) async fn retire_loader(
    directory: &DirectoryLease,
    docker: &DockerPort,
    record: &mut CodeRecord,
    deadline: Instant,
) -> Result<bool, DockerError> {
    match record.loader {
        LoaderState::Retired => return Ok(true),
        LoaderState::Planned => return loader_retired(directory, record, deadline).await,
        LoaderState::Uncertain | LoaderState::Known(_) => {}
    }
    let inspected = match docker
        .inspect_container(&record.loader_key()?, deadline)
        .await
    {
        Err(DockerError::Status(StatusCode::NOT_FOUND))
            if matches!(record.loader, LoaderState::Known(_)) =>
        {
            return loader_retired(directory, record, deadline).await;
        }
        Err(DockerError::Status(StatusCode::NOT_FOUND)) => return Ok(false),
        result => result?,
    };
    let id = record.owned_loader(directory.owner_id(), &inspected)?;
    if record.loader == LoaderState::Uncertain {
        let mut next = record.clone();
        next.loader = LoaderState::Known(id);
        save(directory, record, next, deadline).await?;
    }
    let key = record.loader_key()?;
    let checked = match docker.inspect_container(&key, deadline).await {
        Err(DockerError::Status(StatusCode::NOT_FOUND)) => {
            return loader_retired(directory, record, deadline).await;
        }
        result => result?,
    };
    record.owned_loader(directory.owner_id(), &checked)?;
    let _ = async {
        unit(
            docker
                .call(
                    Operation::Remove,
                    Method::DELETE,
                    format!("/containers/{}?force=false", escaped(key.as_str())),
                    Bytes::new(),
                    StatusCode::NO_CONTENT,
                    deadline,
                )
                .await?,
            deadline,
        )
    }
    .await;
    match docker.inspect_container(&key, deadline).await {
        Err(DockerError::Status(StatusCode::NOT_FOUND)) => {
            loader_retired(directory, record, deadline).await
        }
        Ok(_) => Ok(false),
        Err(error) => Err(error),
    }
}
async fn loader_retired(
    directory: &DirectoryLease,
    record: &mut CodeRecord,
    deadline: Instant,
) -> Result<bool, DockerError> {
    let mut next = record.clone();
    next.loader = LoaderState::Retired;
    save(directory, record, next, deadline).await?;
    Ok(true)
}
async fn retire_volume(
    directory: &DirectoryLease,
    docker: &DockerPort,
    record: &mut CodeRecord,
    deadline: Instant,
) -> Result<bool, DockerError> {
    if record.volume == VolumeState::Retired {
        return Ok(true);
    }
    if record.volume != VolumeState::Known || record.loader != LoaderState::Retired {
        return Err(DockerError::Quarantined);
    }
    let key = record.volume_key()?;
    let inspected = match docker.inspect_volume(&key, deadline).await {
        Err(DockerError::Status(StatusCode::NOT_FOUND)) => {
            return volume_retired(directory, record, deadline).await;
        }
        result => result?,
    };
    record.owned_volume(directory.owner_id(), &inspected)?;
    let _ = docker.remove_volume(&key, deadline).await;
    match docker.inspect_volume(&key, deadline).await {
        Err(DockerError::Status(StatusCode::NOT_FOUND)) => {
            volume_retired(directory, record, deadline).await
        }
        Ok(_) => Ok(false),
        Err(error) => Err(error),
    }
}
async fn volume_retired(
    directory: &DirectoryLease,
    record: &mut CodeRecord,
    deadline: Instant,
) -> Result<bool, DockerError> {
    let mut next = record.clone();
    next.volume = VolumeState::Retired;
    save(directory, record, next, deadline).await?;
    Ok(true)
}

#[cfg(test)]
mod tests;
