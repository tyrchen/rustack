//! Validated local Docker identities, versions and redacted errors.

use std::{fmt, io, os::unix::fs::FileTypeExt, path::PathBuf, str::FromStr, sync::Arc};

use http::StatusCode;
use rustack_core::json::JsonError;
use tokio::time::{Instant, timeout_at};

use super::check_deadline;
use crate::artifact::ArtifactError;

/// Docker boundary failure classes; no peer response body is displayed.
#[derive(Clone, thiserror::Error)]
#[non_exhaustive]
pub enum DockerError {
    /// Invalid local configuration or identifier.
    #[error("invalid Docker configuration")]
    Configuration,
    /// The local daemon's API range is incompatible.
    #[error("unsupported Docker API version")]
    Version,
    /// A declared/actual byte or collection allowance was exceeded.
    #[error("Docker byte budget exceeded")]
    TooLarge,
    /// A framing/header/response is malformed.
    #[error("invalid Docker protocol")]
    Protocol,
    /// A cancelled partial stream cannot be restarted.
    #[error("Docker stream is invalidated")]
    Poisoned,
    /// Absolute IO deadline expired.
    #[error("Docker deadline exceeded")]
    Deadline,
    /// Wire service has stopped admission.
    #[error("Docker wire is stopped")]
    Stopped,
    /// A bounded mailbox has no capacity.
    #[error("Docker wire is busy")]
    Busy,
    /// An attached preparation caller cancelled before receiving an artifact.
    #[error("Docker preparation cancelled")]
    Cancelled,
    /// An owned image mutation/cleanup is not yet proven settled.
    #[error("Docker image cleanup is quarantined")]
    Quarantined,
    /// An owned task panicked or failed to join.
    #[error("Docker supervision failed")]
    Worker,
    /// A bounded output allocation could not be reserved.
    #[error("Docker allocation failed")]
    Allocation,
    /// Request/model encoding failed without exposing input text.
    #[error("invalid Docker request schema")]
    Schema,
    /// Non-success HTTP response; daemon error text is intentionally excluded.
    #[error("Docker response status {0}")]
    Status(StatusCode),
    /// Local socket/stream IO failed.
    #[error("Docker IO failed")]
    Io(#[source] Arc<io::Error>),
    /// HTTP transport failed.
    #[error("Docker HTTP failed")]
    Http(#[source] Arc<hyper::Error>),
    /// Bounded metadata validation failed.
    #[error(transparent)]
    Json(#[from] JsonError),
    /// Image inspection cannot establish a valid executable artifact.
    #[error(transparent)]
    Image(#[from] ArtifactError),
}

impl fmt::Debug for DockerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}
impl From<io::Error> for DockerError {
    fn from(error: io::Error) -> Self {
        Self::Io(Arc::new(error))
    }
}
impl From<hyper::Error> for DockerError {
    fn from(error: hyper::Error) -> Self {
        Self::Http(Arc::new(error))
    }
}

/// Explicit local Unix socket, independent of ambient Docker environment.
#[derive(Clone)]
#[non_exhaustive]
pub struct DockerSocket(PathBuf);

impl fmt::Debug for DockerSocket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DockerSocket(<redacted>)")
    }
}
impl FromStr for DockerSocket {
    type Err = DockerError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let path = value
            .strip_prefix("unix://")
            .ok_or(DockerError::Configuration)?;
        if !path.starts_with('/') || path.len() > 4096 || path.chars().any(char::is_control) {
            return Err(DockerError::Configuration);
        }
        Ok(Self(PathBuf::from(path)))
    }
}
impl DockerSocket {
    pub(super) async fn verified(&self, deadline: Instant) -> Result<PathBuf, DockerError> {
        check_deadline(deadline)?;
        let path = timeout_at(deadline, tokio::fs::canonicalize(&self.0))
            .await
            .map_err(|_| DockerError::Deadline)??;
        let metadata = timeout_at(deadline, tokio::fs::metadata(&path))
            .await
            .map_err(|_| DockerError::Deadline)??;
        if Instant::now() >= deadline {
            return Err(DockerError::Deadline);
        }
        if !metadata.file_type().is_socket() {
            return Err(DockerError::Configuration);
        }
        Ok(path)
    }
}

/// Validated Docker object name or content identifier used as a path component.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DockerKey(String);

impl fmt::Debug for DockerKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DockerKey(<redacted>)")
    }
}
impl FromStr for DockerKey {
    type Err = DockerError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() > 128
            || !value
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_alphanumeric())
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
        {
            return Err(DockerError::Configuration);
        }
        Ok(Self(value.to_owned()))
    }
}
impl DockerKey {
    /// Borrow the validated spelling; it is still encoded when placed in a URI.
    ///
    /// ```
    /// let key: rustack_lambda_core::docker::DockerKey = "rustack-relay-123".parse()?;
    /// assert_eq!(key.as_str(), "rustack-relay-123");
    /// # Ok::<(), rustack_lambda_core::docker::DockerError>(())
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A bounded Docker API version; negotiations select the local1.44..1.53 range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub struct ApiVersion(u16);

impl FromStr for ApiVersion {
    type Err = DockerError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let minor = value.strip_prefix("1.").ok_or(DockerError::Version)?;
        if minor.is_empty() || minor.len() > 3 || !minor.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(DockerError::Version);
        }
        let parsed = minor.parse::<u16>().map_err(|_| DockerError::Version)?;
        if parsed.to_string() != minor {
            return Err(DockerError::Version);
        }
        Ok(Self(parsed))
    }
}
impl ApiVersion {
    pub(super) fn negotiate(maximum: &str, minimum: &str) -> Result<Self, DockerError> {
        let selected = maximum.parse::<Self>()?.min(Self(53));
        let minimum: Self = minimum.parse()?;
        if selected < Self(44) || selected < minimum {
            return Err(DockerError::Version);
        }
        Ok(selected)
    }

    /// Numeric API minor version, without its major version or URI prefix.
    ///
    /// ```
    /// let version: rustack_lambda_core::docker::ApiVersion = "1.53".parse()?;
    /// assert_eq!(version.minor(), 53);
    /// # Ok::<(), rustack_lambda_core::docker::DockerError>(())
    /// ```
    #[must_use]
    pub fn minor(self) -> u16 {
        self.0
    }
}
impl fmt::Display for ApiVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "1.{}", self.0)
    }
}
