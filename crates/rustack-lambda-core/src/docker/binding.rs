//! Persist installation identity and retain exact directory ownership on IO.

use std::{fmt, sync::Arc, time::Duration};

use bytes::Bytes;
use http::{Method, StatusCode};
use serde::Deserialize;
use tokio::time::Instant;
use uuid::Uuid;

use super::{
    DirectoryLease, DockerError, DockerPort, Operation, RawReply, RawRequest, check_deadline,
    decode,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct DaemonIdentity(Uuid);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IdentityResponse {
    #[serde(rename = "ID")]
    id: String,
}

impl fmt::Debug for IdentityResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("IdentityResponse(<redacted>)")
    }
}

impl fmt::Debug for DaemonIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DaemonIdentity(<redacted>)")
    }
}

impl TryFrom<&str> for DaemonIdentity {
    type Error = DockerError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if value.len() != 36 {
            return Err(DockerError::Protocol);
        }
        let id = Uuid::try_parse(value).map_err(|_| DockerError::Protocol)?;
        if id.is_nil() || id.hyphenated().to_string() != value {
            return Err(DockerError::Protocol);
        }
        Ok(Self(id))
    }
}

impl DaemonIdentity {
    pub(super) fn from_uuid(id: Uuid) -> Result<Self, DockerError> {
        if id.is_nil() {
            return Err(DockerError::Protocol);
        }
        Ok(Self(id))
    }

    pub(super) fn uuid(self) -> Uuid {
        self.0
    }

    pub(super) fn decode(reply: RawReply, deadline: Instant) -> Result<Self, DockerError> {
        let response: IdentityResponse = decode(reply, deadline)?;
        Self::try_from(response.id.as_str())
    }
}

#[derive(Debug)]
pub(super) struct DaemonBinding {
    pub(super) identity: DaemonIdentity,
    pub(super) directory: DirectoryLease,
}

impl DockerPort {
    /// Persist this daemon's installation identity and retain the exact lease.
    ///
    /// The returned port checks identity on every actual operation connection.
    /// First binding requires empty container/image journals; existing bindings
    /// never silently move to another daemon. This does not grant readiness.
    ///
    /// # Errors
    /// Rejects invalid budgets/identity, mismatched or unsafe durable state,
    /// unprovable historical effects, IO deadlines and wire failures.
    ///
    /// ```no_run
    /// # async fn bind(port: &rustack_lambda_core::docker::DockerPort, directory: &rustack_lambda_core::docker::DirectoryLease) -> Result<(), rustack_lambda_core::docker::DockerError> {
    /// let bound = port.bind_directory(directory,
    ///     tokio::time::Instant::now() + std::time::Duration::from_secs(5)).await?;
    /// let scope = rustack_lambda_core::docker::ImagePinScope::new(directory.clone());
    /// let mut images = rustack_lambda_core::docker::ImageLeaseManager::start(bound, scope, Default::default())?;
    /// images.shutdown(tokio::time::Instant::now() + std::time::Duration::from_secs(5)).await?;
    /// # Ok(()) }
    /// ```
    pub async fn bind_directory(
        &self,
        directory: &DirectoryLease,
        deadline: Instant,
    ) -> Result<Self, DockerError> {
        check_deadline(deadline)?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or(DockerError::Deadline)?;
        if remaining > Duration::from_secs(60) {
            return Err(DockerError::Configuration);
        }
        let identity = async {
            let reply = self
                .raw
                .request(
                    RawRequest {
                        pull: None,
                        binding: None,
                        mutation: None,
                        operation: Operation::Info,
                        method: Method::GET,
                        target: "/info".into(),
                        body: Bytes::new(),
                        expected: StatusCode::OK,
                    },
                    deadline,
                )
                .await?;
            DaemonIdentity::decode(reply, deadline)
        }
        .await
        .map_err(|source| DockerError::IdentityCheck(Arc::new(source)))?;
        directory.bind_daemon(identity, deadline).await?;
        check_deadline(deadline)?;
        let mut bound = self.clone();
        bound.raw.binding = Some(Arc::new(DaemonBinding {
            identity,
            directory: directory.clone(),
        }));
        Ok(bound)
    }

    pub(super) fn require_directory(&self, directory: &DirectoryLease) -> Result<(), DockerError> {
        if self
            .raw
            .binding
            .as_ref()
            .is_some_and(|binding| binding.directory.same_instance(directory))
        {
            return Ok(());
        }
        #[cfg(test)]
        if self.raw.binding.is_none() && directory.synthetic() {
            return Ok(());
        }
        Err(DockerError::DaemonBinding)
    }
}

#[cfg(test)]
mod tests;
