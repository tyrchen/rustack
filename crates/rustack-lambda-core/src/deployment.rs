//! Typed deployment transitions and stale-result protection at the store boundary.
//!
//! A retained worker owns each candidate until settlement. Store keys bind resource
//! lifetime, original revision and deployment generation; metadata is never a live lease.

#[cfg(unix)]
mod manager;
use std::fmt;

#[cfg(unix)]
pub use manager::{ImageDeploymentConfig, ImageDeploymentManager, ImageDeploymentStatus};
use rustack_lambda_model::types::ImageConfig;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;
use uuid::Uuid;

use crate::{
    artifact::{Architecture, ImageArtifact, ImageReference, PinnedImage},
    error::LambdaServiceError,
    identity::FunctionResourceId,
    storage::{FunctionRecord, FunctionStore, VersionRecord},
};

/// A nonnil identity for one accepted deployment attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[non_exhaustive]
pub struct DeploymentGeneration(Uuid);

impl TryFrom<Uuid> for DeploymentGeneration {
    type Error = LambdaServiceError;
    fn try_from(value: Uuid) -> Result<Self, Self::Error> {
        if value.is_nil() {
            return Err(invalid("Deployment generation cannot be nil"));
        }
        Ok(Self(value))
    }
}
impl TryFrom<String> for DeploymentGeneration {
    type Error = LambdaServiceError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() != 36 {
            return Err(invalid("Invalid deployment generation"));
        }
        let id = Uuid::parse_str(&value).map_err(|_| invalid("Invalid deployment generation"))?;
        Self::try_from(id)
    }
}
impl From<DeploymentGeneration> for String {
    fn from(value: DeploymentGeneration) -> Self {
        value.0.to_string()
    }
}

/// Bounded public failure vocabulary; no backend response or secret is retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum ImageDeploymentFailure {
    /// The requested content is unavailable.
    ImageNotFound,
    /// Image metadata or its effective entry configuration is invalid.
    InvalidImage,
    /// The selected architecture cannot run under the configured policy.
    UnsupportedPlatform,
    /// Docker is unavailable or its ownership health is degraded.
    RuntimeUnavailable,
    /// Deployment exceeded its absolute deadline.
    DeadlineExceeded,
    /// Deployment was cancelled before settlement.
    Cancelled,
    /// Its retained supervisor failed.
    SupervisorFailed,
}
impl ImageDeploymentFailure {
    /// Static, bounded API reason with no untrusted backend text.
    ///
    /// ```
    /// use rustack_lambda_core::deployment::ImageDeploymentFailure;
    /// assert!(ImageDeploymentFailure::InvalidImage.reason().len() < 256);
    /// ```
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::ImageNotFound => "Requested image content is unavailable",
            Self::InvalidImage => "Image metadata or entry configuration is invalid",
            Self::UnsupportedPlatform => "Image platform is unsupported by this runtime",
            Self::RuntimeUnavailable => "Image runtime is unavailable",
            Self::DeadlineExceeded => "Image deployment deadline exceeded",
            Self::Cancelled => "Image deployment was cancelled",
            Self::SupervisorFailed => "Image deployment supervisor failed",
        }
    }

    /// AWS-compatible reason category for the local failure.
    ///
    /// ```
    /// use rustack_lambda_core::deployment::ImageDeploymentFailure;
    /// assert_eq!(ImageDeploymentFailure::InvalidImage.code(), "InvalidImage");
    /// ```
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::ImageNotFound => "ImageDeleted",
            Self::InvalidImage | Self::UnsupportedPlatform => "InvalidImage",
            Self::RuntimeUnavailable
            | Self::DeadlineExceeded
            | Self::Cancelled
            | Self::SupervisorFailed => "InternalError",
        }
    }
}

/// One authoritative state for both function readiness and update progress.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "detail",
    rename_all = "camelCase",
    deny_unknown_fields
)]
#[non_exhaustive]
pub enum DeploymentState {
    /// Usable revision with no failed or pending update.
    #[default]
    Ready,
    /// Initial deployment; no previous executable revision exists.
    Creating(DeploymentGeneration),
    /// Initial deployment failed.
    CreateFailed(ImageDeploymentFailure),
    /// Previous revision remains usable while a candidate prepares.
    Updating(DeploymentGeneration),
    /// Previous revision remains usable after a failed update.
    UpdateFailed(ImageDeploymentFailure),
}
impl DeploymentState {
    /// Public Lambda function state.
    ///
    /// ```
    /// assert_eq!(rustack_lambda_core::deployment::DeploymentState::Ready.state(), "Active");
    /// ```
    #[must_use]
    pub const fn state(self) -> &'static str {
        match self {
            Self::Ready | Self::Updating(_) | Self::UpdateFailed(_) => "Active",
            Self::Creating(_) => "Pending",
            Self::CreateFailed(_) => "Failed",
        }
    }

    /// Public Lambda update status.
    ///
    /// ```
    /// assert_eq!(rustack_lambda_core::deployment::DeploymentState::Ready.update_status(), "Successful");
    /// ```
    #[must_use]
    pub const fn update_status(self) -> &'static str {
        match self {
            Self::Ready => "Successful",
            Self::Creating(_) | Self::Updating(_) => "InProgress",
            Self::CreateFailed(_) | Self::UpdateFailed(_) => "Failed",
        }
    }

    /// Public failure reason, if a deployment failed.
    ///
    /// ```
    /// assert!(rustack_lambda_core::deployment::DeploymentState::Ready.failure().is_none());
    /// ```
    #[must_use]
    pub const fn failure(self) -> Option<ImageDeploymentFailure> {
        match self {
            Self::CreateFailed(reason) | Self::UpdateFailed(reason) => Some(reason),
            _ => None,
        }
    }

    /// `StateReason` describes initial readiness failure, not a failed update.
    pub(crate) const fn creation_failure(self) -> Option<ImageDeploymentFailure> {
        match self {
            Self::CreateFailed(failure) => Some(failure),
            _ => None,
        }
    }

    pub(crate) fn ensure_mutable(self) -> Result<(), LambdaServiceError> {
        if matches!(self, Self::Creating(_) | Self::Updating(_)) {
            return Err(conflict("A function deployment is already in progress"));
        }
        Ok(())
    }

    pub(crate) fn restore(legacy: &str, saved: Option<Self>) -> Result<Self, LambdaServiceError> {
        let state = match saved {
            Some(state) if state.state() == legacy => state,
            Some(_) => return Err(invalid("Snapshot deployment states contradict each other")),
            None => match legacy {
                "Active" => Self::Ready,
                "Pending" => Self::CreateFailed(ImageDeploymentFailure::Cancelled),
                "Failed" => Self::CreateFailed(ImageDeploymentFailure::InvalidImage),
                _ => return Err(invalid("Unsupported snapshot function state")),
            },
        };
        Ok(match state {
            Self::Creating(_) => Self::CreateFailed(ImageDeploymentFailure::Cancelled),
            Self::Updating(_) => Self::UpdateFailed(ImageDeploymentFailure::Cancelled),
            state => state,
        })
    }
}

/// Closed candidate issued by a store transaction, owned by a deployment worker.
///
/// Debug omits its image reference, entry configuration and environment.
///
/// Construction is restricted to validated store transactions:
///
/// ```compile_fail
/// use rustack_lambda_core::deployment::ImageDeployment;
/// let forged = ImageDeployment::builder();
/// ```
#[derive(Clone, TypedBuilder)]
#[builder(
    builder_method(vis = ""),
    build_method(vis = ""),
    builder_type(vis = "")
)]
#[non_exhaustive]
pub struct ImageDeployment {
    name: String,
    resource: FunctionResourceId,
    original_revision: String,
    generation: DeploymentGeneration,
    initial: bool,
    candidate: VersionRecord,
    architecture: Architecture,
    reference: ImageReference,
}
impl fmt::Debug for ImageDeployment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageDeployment")
            .field("resource", &self.resource)
            .field("generation", &self.generation)
            .field("initial", &self.initial)
            .finish_non_exhaustive()
    }
}
impl ImageDeployment {
    fn new(
        record: &FunctionRecord,
        candidate: VersionRecord,
        initial: bool,
    ) -> Result<Self, LambdaServiceError> {
        crate::resolver::FunctionName::parse(&record.name)?;
        if record.latest.revision_id.len() != 36
            || Uuid::parse_str(&record.latest.revision_id).is_err()
        {
            return Err(invalid("Invalid deployment revision identity"));
        }
        let architecture = crate::function_config::validate_execution_settings(&candidate)?;
        let reference = candidate
            .image_uri
            .as_deref()
            .ok_or_else(|| invalid("Image reference is required"))?
            .parse()
            .map_err(|_| invalid("Invalid image reference"))?;
        Ok(Self::builder()
            .name(record.name.clone())
            .resource(record.resource_id)
            .original_revision(record.latest.revision_id.clone())
            .generation(DeploymentGeneration(Uuid::new_v4()))
            .initial(initial)
            .candidate(candidate)
            .architecture(architecture)
            .reference(reference)
            .build())
    }

    /// Reference captured at acceptance for bounded image preparation.
    ///
    /// ```no_run
    /// # fn inspect(job: &rustack_lambda_core::deployment::ImageDeployment) {
    /// let reference = job.reference();
    /// # }
    /// ```
    #[must_use]
    pub fn reference(&self) -> &ImageReference {
        &self.reference
    }

    /// Validated requested architecture for this candidate.
    ///
    /// ```no_run
    /// # fn inspect(job: &rustack_lambda_core::deployment::ImageDeployment) {
    /// let architecture = job.architecture();
    /// # }
    /// ```
    #[must_use]
    pub const fn architecture(&self) -> Architecture {
        self.architecture
    }

    /// Complete entry overrides; omitted and empty arrays remain distinct.
    ///
    /// ```no_run
    /// # fn inspect(job: &rustack_lambda_core::deployment::ImageDeployment) {
    /// let overrides = job.image_config();
    /// # }
    /// ```
    #[must_use]
    pub fn image_config(&self) -> Option<&ImageConfig> {
        self.candidate.image_config.as_ref()
    }

    #[cfg(unix)]
    pub(crate) fn is_current(&self, store: &FunctionStore) -> Result<bool, LambdaServiceError> {
        Ok(store
            .get(&self.name)?
            .as_ref()
            .is_some_and(|record| self.matches(record)))
    }

    fn matches(&self, record: &FunctionRecord) -> bool {
        let state = if self.initial {
            DeploymentState::Creating(self.generation)
        } else {
            DeploymentState::Updating(self.generation)
        };
        record.resource_id == self.resource
            && record.latest.revision_id == self.original_revision
            && record.latest.deployment == state
    }
}

impl FunctionStore {
    /// Insert an initial Pending image deployment after the caller reserves work.
    ///
    /// # Errors
    /// Rejects duplicate/invalid records and non-image or already resolved candidates.
    ///
    /// ```no_run
    /// # fn accept(store: &rustack_lambda_core::storage::FunctionStore, record: rustack_lambda_core::storage::FunctionRecord) -> Result<(), rustack_lambda_core::error::LambdaServiceError> {
    /// let candidate = store.insert_image_deployment(record)?;
    /// # Ok(()) }
    /// ```
    pub fn insert_image_deployment(
        &self,
        record: FunctionRecord,
    ) -> Result<ImageDeployment, LambdaServiceError> {
        self.reserve_image_create(record).map(|(job, _)| job)
    }

    pub(crate) fn reserve_image_create(
        &self,
        mut record: FunctionRecord,
    ) -> Result<(ImageDeployment, FunctionRecord), LambdaServiceError> {
        if record.latest.package_type != "Image"
            || !matches!(record.latest.image, ImageArtifact::Unresolved)
            || record.latest.deployment != DeploymentState::Ready
            || !record.versions.is_empty()
        {
            return Err(invalid(
                "Initial image candidate must be unresolved and unpublished",
            ));
        }
        record.latest.code_sha256.clear();
        record.latest.code_size = 0;
        let job = ImageDeployment::new(&record, record.latest.clone(), true)?;
        record.latest.deployment = DeploymentState::Creating(job.generation);
        self.insert(record.clone())?;
        Ok((job, record))
    }

    /// Reserve an image update while preserving the previous usable revision.
    ///
    /// # Errors
    /// Rejects stale resource/revision keys, pending work, package changes or invalid settings.
    ///
    /// ```no_run
    /// # fn accept(store: &rustack_lambda_core::storage::FunctionStore, record: &rustack_lambda_core::storage::FunctionRecord) -> Result<(), rustack_lambda_core::error::LambdaServiceError> {
    /// let job = store.begin_image_update(&record.name, record.resource_id,
    ///     &record.latest.revision_id, "worker:v2".parse().map_err(|_| rustack_lambda_core::error::LambdaServiceError::DockerNotAvailable)?,
    ///     rustack_lambda_core::artifact::Architecture::Arm64)?;
    /// # Ok(()) }
    /// ```
    pub fn begin_image_update(
        &self,
        name: &str,
        resource: FunctionResourceId,
        expected: &str,
        reference: ImageReference,
        architecture: Architecture,
    ) -> Result<ImageDeployment, LambdaServiceError> {
        self.reserve_image_update(name, resource, expected, reference, architecture, None)
            .map(|(job, _)| job)
    }

    pub(crate) fn reserve_image_update(
        &self,
        name: &str,
        resource: FunctionResourceId,
        expected: &str,
        reference: ImageReference,
        architecture: Architecture,
        requested: Option<&str>,
    ) -> Result<(ImageDeployment, FunctionRecord), LambdaServiceError> {
        self.try_update_if_revision(name, None, |record| {
            check_revision(requested, &record.latest.revision_id)?;
            if record.resource_id != resource || record.latest.revision_id != expected {
                return Err(conflict("Function resource changed"));
            }
            record.latest.deployment.ensure_mutable()?;
            if record.latest.package_type != "Image" {
                return Err(invalid("Deployment package type cannot change"));
            }
            let mut candidate = record.latest.clone();
            candidate.image_uri = Some(reference.into());
            candidate.architectures = vec![architecture.as_str().into()];
            candidate.image = ImageArtifact::Unresolved;
            candidate.code_sha256.clear();
            candidate.code_size = 0;
            candidate.deployment = DeploymentState::Ready;
            let initial = matches!(record.latest.deployment, DeploymentState::CreateFailed(_));
            let job = ImageDeployment::new(record, candidate.clone(), initial)?;
            if initial {
                record.latest = candidate;
            }
            record.latest.deployment = if initial {
                DeploymentState::Creating(job.generation)
            } else {
                DeploymentState::Updating(job.generation)
            };
            Ok((job, record.clone()))
        })
    }

    /// Atomically settle a candidate only while all accepted identity keys match.
    ///
    /// Stale/duplicate/deleted results return false and cannot mutate live metadata.
    /// The caller must retain and join the worker independently of this transaction.
    ///
    /// # Errors
    /// Rejects a revoked/mismatching lease or an invalid candidate without committing it.
    ///
    /// ```no_run
    /// # fn finish(store: &rustack_lambda_core::storage::FunctionStore, job: &rustack_lambda_core::deployment::ImageDeployment) -> Result<(), rustack_lambda_core::error::LambdaServiceError> {
    /// store.settle_image_deployment(job, Err(rustack_lambda_core::deployment::ImageDeploymentFailure::Cancelled))?;
    /// # Ok(()) }
    /// ```
    pub fn settle_image_deployment(
        &self,
        job: &ImageDeployment,
        outcome: Result<PinnedImage, ImageDeploymentFailure>,
    ) -> Result<bool, LambdaServiceError> {
        let Some(before) = self.get(&job.name)? else {
            return Ok(false);
        };
        if !job.matches(&before) {
            return Ok(false);
        }
        let prepared = match outcome {
            Ok(image) => {
                if !image.is_live()
                    || image.resolved().metadata().requested_reference != job.reference
                    || image.resolved().metadata().architecture != job.architecture
                {
                    return Err(invalid(
                        "Prepared image does not match deployment candidate",
                    ));
                }
                image
                    .resolved()
                    .metadata()
                    .execution
                    .plan(job.image_config())
                    .map_err(|_| invalid("Invalid effective image entry configuration"))?;
                let mut candidate = job.candidate.clone();
                candidate.code_sha256 = image
                    .resolved()
                    .code_sha256()
                    .map_err(|_| invalid("Invalid image fingerprint"))?;
                candidate.code_size = image.unpacked_size().get();
                candidate.image = ImageArtifact::Pinned(image);
                candidate.deployment = DeploymentState::Ready;
                candidate.revision_id = Uuid::new_v4().to_string();
                candidate.last_modified = crate::provider::now_iso8601();
                Ok(candidate)
            }
            Err(failure) => Err(failure),
        };
        match self.update(&job.name, |record| {
            if !job.matches(record) {
                return false;
            }
            match prepared {
                Ok(candidate) => record.latest = candidate,
                Err(failure) => {
                    record.latest.deployment = if job.initial {
                        DeploymentState::CreateFailed(failure)
                    } else {
                        DeploymentState::UpdateFailed(failure)
                    }
                }
            }
            true
        }) {
            Err(LambdaServiceError::FunctionNotFound { .. }) => Ok(false),
            result => result,
        }
    }
}

pub(crate) fn check_revision(
    requested: Option<&str>,
    actual: &str,
) -> Result<(), LambdaServiceError> {
    if let Some(requested) = requested {
        if requested.is_empty()
            || requested.len() > 100
            || !requested
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
        {
            return Err(invalid("Invalid RevisionId"));
        }
        if requested != actual {
            return Err(LambdaServiceError::PreconditionFailed);
        }
    }
    Ok(())
}

fn invalid(message: &str) -> LambdaServiceError {
    LambdaServiceError::InvalidParameter {
        message: message.into(),
    }
}
fn conflict(message: &str) -> LambdaServiceError {
    LambdaServiceError::ResourceConflict {
        message: message.into(),
    }
}

#[cfg(test)]
#[path = "deployment_tests.rs"]
mod tests;
