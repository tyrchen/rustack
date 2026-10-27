//! Historical alias cleanup only after container ownership has settled.

use http::StatusCode;
use tokio::time::Instant;

use super::super::{
    DirectoryLease, DockerError, DockerPort, check_deadline, ownership::ImageRecord,
};

/// Pin-only recovery accounting, not full backend readiness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ImagePinRecovery {
    /// Records durably erased during this pass.
    pub settled: usize,
    /// Records still retained, including current-run pins never touched here.
    pub remaining: usize,
}

/// Retire recorded historical private aliases after historical containers settle.
///
/// Never replay tag, delete source tags or infer unknown absence as settlement.
/// A positive matching alias is durably committed before removal; an absent
/// committed alias can be erased. Cancellation preserves resumable ownership.
///
/// # Errors
/// Returns ownership/budget/wire failures, Quarantined for remaining historical
/// containers or mismatching alias identity, and protocol failures. This does
/// not grant readiness or release another owner's live allocations. The trusted
/// caller must bind this directory to the same daemon that received its intents;
/// a fresh socket connection alone is not persistent daemon identity.
///
/// ```no_run
/// # async fn recover(directory: &rustack_lambda_core::docker::DirectoryLease, docker: &rustack_lambda_core::docker::DockerPort) -> Result<(), rustack_lambda_core::docker::DockerError> {
/// let report = rustack_lambda_core::docker::reconcile_image_pins(directory, docker,
///     tokio::time::Instant::now() + std::time::Duration::from_secs(30)).await?;
/// assert_eq!(report.remaining, 0);
/// # Ok(()) }
/// ```
pub async fn reconcile_image_pins(
    directory: &DirectoryLease,
    docker: &DockerPort,
    deadline: Instant,
) -> Result<ImagePinRecovery, DockerError> {
    if directory
        .container_records(deadline)
        .await?
        .iter()
        .any(|record| record.run != directory.run_id())
    {
        return Err(DockerError::Quarantined);
    }
    let records = directory.image_records(deadline).await?;
    let mut report = ImagePinRecovery {
        settled: 0,
        remaining: records.len(),
    };
    for record in records {
        if record.run == directory.run_id() {
            continue;
        }
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
async fn recover(
    directory: &DirectoryLease,
    docker: &DockerPort,
    record: ImageRecord,
    deadline: Instant,
) -> Result<bool, DockerError> {
    let pin = record.pin(directory)?;
    let expected = record.expected()?;
    let inspected = match docker.inspect_image_pin(&pin, deadline).await {
        Err(DockerError::Status(StatusCode::NOT_FOUND)) if record.committed => {
            directory.settle_image(record, deadline).await?;
            return Ok(true);
        }
        Err(DockerError::Status(StatusCode::NOT_FOUND)) => return Ok(false),
        result => result?,
    };
    if inspected.id.as_deref() != Some(expected.as_str()) {
        return Err(DockerError::Quarantined);
    }
    directory.commit_image(record.clone(), deadline).await?;
    let checked = match docker.inspect_image_pin(&pin, deadline).await {
        Err(DockerError::Status(StatusCode::NOT_FOUND)) => {
            directory.settle_image(record, deadline).await?;
            return Ok(true);
        }
        result => result?,
    };
    if checked.id.as_deref() != Some(expected.as_str()) {
        return Err(DockerError::Quarantined);
    }
    let _ = docker.remove_image_pin(&pin, deadline).await;
    match docker.inspect_image_pin(&pin, deadline).await {
        Err(DockerError::Status(StatusCode::NOT_FOUND)) => {
            directory.settle_image(record, deadline).await?;
            Ok(true)
        }
        Ok(_) => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests;
