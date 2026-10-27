//! Prepare every fallible record/artifact before the startup publication boundary.

#[cfg(unix)]
use std::time::Duration;
use std::{
    collections::{BTreeMap, HashSet},
    iter::once,
};

#[cfg(unix)]
use tokio::time::Instant;

use super::{FunctionRecord, FunctionStore, FunctionStoreSnapshot, StagedArtifact};
#[cfg(unix)]
use crate::{
    artifact::{ImageArtifact, PinnedImage},
    docker::{DockerError, ImageLeasePort},
};
use crate::{
    error::LambdaServiceError,
    resolver::{FunctionName, Qualifier, resolve_function_ref},
};

#[derive(Debug)]
pub(crate) struct PreparedImport<'a> {
    store: &'a FunctionStore,
    functions: BTreeMap<String, FunctionRecord>,
    artifacts: Vec<StagedArtifact<'a>>,
}
impl PreparedImport<'_> {
    /// Infallible startup publication after all validation and ownership checks.
    pub(crate) fn commit(self) {
        self.store.functions.clear();
        for (name, record) in self.functions {
            self.store.functions.insert(name, record);
        }
        for artifact in self.artifacts {
            artifact.commit();
        }
        self.store.changes.send_replace(());
    }
    #[cfg(unix)]
    pub(crate) fn commit_before(self, deadline: Instant) -> Result<(), LambdaServiceError> {
        check_deadline(deadline)?;
        self.commit();
        Ok(())
    }
}
impl FunctionStore {
    pub(crate) fn check_snapshot_counts(
        &self,
        functions: usize,
        revisions: usize,
    ) -> Result<(), LambdaServiceError> {
        if self.functions.len() > functions {
            return Err(invalid("Image archive function count exceeds its profile"));
        }
        let mut count = 0usize;
        for record in &self.functions {
            count = count
                .checked_add(1)
                .and_then(|n| n.checked_add(record.versions.len()))
                .ok_or_else(|| invalid("Image archive revision count overflow"))?;
            if count > revisions {
                return Err(invalid("Image archive revision count exceeds its profile"));
            }
        }
        Ok(())
    }
    pub(super) async fn prepare_snapshot(
        &self,
        snapshot: FunctionStoreSnapshot,
    ) -> Result<PreparedImport<'_>, LambdaServiceError> {
        self.validate_root().await?;
        let mut staged = BTreeMap::new();
        let mut identities = HashSet::new();
        let mut artifacts = Vec::new();
        for function in snapshot.functions {
            if !identities.insert(function.resource_id) {
                return Err(invalid("Duplicate snapshot function resource identity"));
            }
            FunctionName::parse(&function.name)?;
            let (name, qualifier) = resolve_function_ref(&function.arn)?;
            if name != function.name || qualifier.is_some() {
                return Err(invalid("Snapshot function ARN mismatch"));
            }
            for (alias, record) in &function.aliases {
                Qualifier::parse(alias)?;
                Qualifier::parse(&record.function_version)?;
            }
            let (record, allocations) = function.into_record(self).await?;
            artifacts.extend(allocations);
            self.validate_record(&record)?;
            if staged.insert(record.name.clone(), record).is_some() {
                return Err(invalid("Duplicate snapshot function"));
            }
        }
        Ok(PreparedImport {
            store: self,
            functions: staged,
            artifacts,
        })
    }

    #[cfg(unix)]
    pub(crate) async fn prepare_image_snapshot(
        &self,
        snapshot: FunctionStoreSnapshot,
        images: &ImageLeasePort,
        deadline: Instant,
    ) -> Result<PreparedImport<'_>, LambdaServiceError> {
        check_deadline(deadline)?;
        let mut candidate = self.prepare_snapshot(snapshot).await?;
        // Validate all usable revisions and effective plans before any pin effect.
        for record in candidate.functions.values() {
            for alias in record.aliases.values() {
                let target = alias
                    .function_version
                    .parse::<u64>()
                    .map_err(|_| invalid("Invalid restored alias version"))?;
                if !record.versions.contains_key(&target) {
                    return Err(invalid("Restored alias target is missing"));
                }
                if let Some(weights) = &alias.routing_config {
                    for (version, weight) in weights {
                        let target = version
                            .parse::<u64>()
                            .map_err(|_| invalid("Invalid restored alias routing"))?;
                        if !record.versions.contains_key(&target)
                            || !weight.is_finite()
                            || !(0.0..=1.0).contains(weight)
                        {
                            return Err(invalid("Invalid restored alias routing"));
                        }
                    }
                }
            }
            for version in once(&record.latest).chain(record.versions.values()) {
                if version.deployment.state() == "Active" {
                    let saved = version.image.resolved().ok_or_else(|| {
                        invalid("Usable restored image has no immutable metadata")
                    })?;
                    saved
                        .metadata()
                        .execution
                        .plan(version.image_config.as_ref())
                        .map_err(|_| invalid("Invalid restored image execution plan"))?;
                }
            }
        }
        let mut leases: BTreeMap<Vec<u8>, PinnedImage> = BTreeMap::new();
        for record in candidate.functions.values_mut() {
            for version in once(&mut record.latest).chain(record.versions.values_mut()) {
                check_deadline(deadline)?;
                if version.deployment.state() != "Active" {
                    continue;
                }
                let saved = version
                    .image
                    .resolved()
                    .ok_or_else(|| invalid("Usable restored image has no immutable metadata"))?;
                let key = serde_json::to_vec(saved)
                    .map_err(|_| invalid("Invalid restored image metadata"))?;
                let image = if let Some(image) = leases.get(&key) {
                    image.clone()
                } else {
                    let image = images
                        .restore(saved, version.image_config.as_ref(), deadline)
                        .await
                        .map_err(image_error)?;
                    leases.insert(key, image.clone());
                    image
                };
                if image.unpacked_size().get() != version.code_size {
                    return Err(invalid("Restored image storage proof mismatch"));
                }
                version.image = ImageArtifact::Pinned(image);
            }
        }
        for record in candidate.functions.values() {
            self.validate_record(record)?;
        }
        images.available().map_err(image_error)?;
        check_deadline(deadline)?;
        Ok(candidate)
    }
}
fn invalid(message: &str) -> LambdaServiceError {
    LambdaServiceError::InvalidParameter {
        message: message.into(),
    }
}
#[cfg(unix)]
pub(crate) fn check_deadline(deadline: Instant) -> Result<(), LambdaServiceError> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|value| !value.is_zero())
        .ok_or_else(|| image_error(DockerError::Deadline))?;
    if remaining > Duration::from_secs(3600) {
        return Err(image_error(DockerError::Configuration));
    }
    Ok(())
}
#[cfg(unix)]
pub(crate) fn image_error(source: DockerError) -> LambdaServiceError {
    LambdaServiceError::ImageRestore {
        source: Box::new(source),
    }
}
