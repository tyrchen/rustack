//! Only generated private alias identities can reach image mutation endpoints.

use std::fmt;

use bytes::Bytes;
use http::{Method, StatusCode};
use tokio::time::Instant;
use uuid::Uuid;

use super::{
    DirectoryLease, DockerError, DockerPort, Operation, RawRequest, decode, escaped,
    models::{ImageDeleteResponseItem, ImageInspect},
    mutation::MutationGuard,
    unit,
};
use crate::artifact::{ImageReference, Sha256Digest};

/// Trusted runtime owner/run identity; never populated from a Lambda request.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ImagePinScope {
    lease: DirectoryLease,
}
impl ImagePinScope {
    pub(super) fn directory(&self) -> &DirectoryLease {
        &self.lease
    }

    /// Retain a real directory lease for this namespace and its allocated pins.
    ///
    /// ```no_run
    /// # fn scope(lease: rustack_lambda_core::docker::DirectoryLease) {
    /// let scope = rustack_lambda_core::docker::ImagePinScope::new(lease);
    /// # }
    /// ```
    #[must_use]
    pub fn new(lease: DirectoryLease) -> Self {
        Self { lease }
    }

    #[cfg(test)]
    pub(super) fn testing(owner: Uuid, run: Uuid) -> Result<Self, DockerError> {
        if owner.is_nil() || run.is_nil() {
            return Err(DockerError::Configuration);
        }
        Ok(Self::new(DirectoryLease::testing(owner, run)))
    }
    /// Allocate a fresh private alias before issuing any Docker mutation.
    ///
    /// # Errors
    /// Returns validation failure if the generated reference cannot validate.
    pub(super) fn allocate(&self) -> Result<ImagePin, DockerError> {
        ImagePin::historical(self.lease.clone(), self.lease.run_id(), Uuid::new_v4())
    }
}

/// An allocated alias; arbitrary source references cannot become cleanup keys.
#[derive(Clone)]
#[non_exhaustive]
pub struct ImagePin {
    pub(super) id: Uuid,
    pub(super) run: Uuid,
    reference: ImageReference,
    scope: ImagePinScope,
}
impl ImagePin {
    pub(super) fn historical(
        lease: DirectoryLease,
        run: Uuid,
        id: Uuid,
    ) -> Result<Self, DockerError> {
        if run.is_nil() || id.is_nil() {
            return Err(DockerError::Configuration);
        }
        let reference = format!(
            "rustack-internal/lambda-{}/{}:{}",
            lease.owner_id().simple(),
            run.simple(),
            id.simple()
        )
        .parse()?;
        Ok(Self {
            id,
            run,
            reference,
            scope: ImagePinScope::new(lease),
        })
    }
    pub(super) fn directory(&self) -> &DirectoryLease {
        &self.scope.lease
    }
}
impl fmt::Debug for ImagePin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImagePin")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}
impl ImagePin {
    /// Exact private alias; its namespace is reserved for the owning runtime.
    ///
    /// ```no_run
    /// # fn reference(pin:&rustack_lambda_core::docker::ImagePin) {
    /// let reference = pin.reference();
    /// # }
    /// ```
    #[must_use]
    pub fn reference(&self) -> &ImageReference {
        &self.reference
    }
}

impl DockerPort {
    /// Create only the allocated private alias, targeting a captured image ID.
    ///
    /// # Errors
    /// Returns bounded wire failure. Any uncertain outcome retains pin ownership.
    ///
    /// ```no_run
    /// # async fn tag(port:&rustack_lambda_core::docker::DockerPort,id:&rustack_lambda_core::artifact::Sha256Digest,pin:&rustack_lambda_core::docker::ImagePin) {
    /// let result = port.tag_image_pin(id,pin,tokio::time::Instant::now()+std::time::Duration::from_secs(5)).await;
    /// # }
    /// ```
    pub async fn tag_image_pin(
        &self,
        id: &Sha256Digest,
        pin: &ImagePin,
        deadline: Instant,
    ) -> Result<(), DockerError> {
        self.tag_image_pin_inner(id, pin, deadline, None).await
    }
    pub(super) async fn tag_image_pin_tracked(
        &self,
        id: &Sha256Digest,
        pin: &ImagePin,
        deadline: Instant,
        mutation: MutationGuard,
    ) -> Result<(), DockerError> {
        self.tag_image_pin_inner(id, pin, deadline, Some(mutation))
            .await
    }
    async fn tag_image_pin_inner(
        &self,
        id: &Sha256Digest,
        pin: &ImagePin,
        deadline: Instant,
        mutation: Option<MutationGuard>,
    ) -> Result<(), DockerError> {
        let query = form_urlencoded::Serializer::new(String::new())
            .append_pair("repo", pin.reference.repository())
            .append_pair("tag", &pin.id.simple().to_string())
            .finish();
        let reply = self
            .raw
            .request(
                RawRequest {
                    pull: None,
                    binding: None,
                    mutation,
                    operation: Operation::TagPin,
                    method: Method::POST,
                    target: format!(
                        "/v{}/images/{}/tag?{query}",
                        self.version,
                        escaped(id.as_str())
                    ),
                    body: Bytes::new(),
                    expected: StatusCode::CREATED,
                },
                deadline,
            )
            .await?;
        unit(reply, deadline)
    }

    /// Inspect a known private alias using reserved cleanup capacity.
    ///
    /// # Errors
    /// Returns missing alias status, bounded metadata or transport failure.
    ///
    /// ```no_run
    /// # async fn inspect(port:&rustack_lambda_core::docker::DockerPort,pin:&rustack_lambda_core::docker::ImagePin) {
    /// let result = port.inspect_image_pin(pin,tokio::time::Instant::now()+std::time::Duration::from_secs(5)).await;
    /// # }
    /// ```
    pub async fn inspect_image_pin(
        &self,
        pin: &ImagePin,
        deadline: Instant,
    ) -> Result<ImageInspect, DockerError> {
        decode(
            self.call(
                Operation::InspectPin,
                Method::GET,
                format!("/images/{}/json", escaped(pin.reference.as_str())),
                Bytes::new(),
                StatusCode::OK,
                deadline,
            )
            .await?,
            deadline,
        )
    }

    /// Remove only an allocated private alias, without force or ancestor pruning.
    ///
    /// # Errors
    /// Returns status, bounded response or transport failure. The owner verifies
    /// identity under its exclusive namespace before removal and confirms absence.
    ///
    /// ```no_run
    /// # async fn remove(port:&rustack_lambda_core::docker::DockerPort,pin:&rustack_lambda_core::docker::ImagePin) {
    /// let result = port.remove_image_pin(pin,tokio::time::Instant::now()+std::time::Duration::from_secs(5)).await;
    /// # }
    /// ```
    pub async fn remove_image_pin(
        &self,
        pin: &ImagePin,
        deadline: Instant,
    ) -> Result<Vec<ImageDeleteResponseItem>, DockerError> {
        decode(
            self.call(
                Operation::RemovePin,
                Method::DELETE,
                format!(
                    "/images/{}?force=false&noprune=true",
                    escaped(pin.reference.as_str())
                ),
                Bytes::new(),
                StatusCode::OK,
                deadline,
            )
            .await?,
            deadline,
        )
    }

    pub(super) async fn image_manifests(
        &self,
        id: &Sha256Digest,
        deadline: Instant,
    ) -> Result<ImageInspect, DockerError> {
        let suffix = if self.api_version().minor() >= 48 {
            "?manifests=true"
        } else {
            ""
        };
        decode(
            self.call(
                Operation::Image,
                Method::GET,
                format!("/images/{}/json{suffix}", escaped(id.as_str())),
                Bytes::new(),
                StatusCode::OK,
                deadline,
            )
            .await?,
            deadline,
        )
    }
}
