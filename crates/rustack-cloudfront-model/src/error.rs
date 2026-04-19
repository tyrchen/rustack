//! CloudFront error type.
//!
//! Unlike S3, CloudFront wraps error responses in `<ErrorResponse>`. The HTTP
//! layer is responsible for emitting the wire format; this module only
//! classifies errors and carries their HTTP status plus AWS error code.

use thiserror::Error;

/// Top-level CloudFront service error.
///
/// Each variant maps to an AWS error `Code` and an HTTP status.
#[derive(Debug, Error)]
pub enum CloudFrontError {
    /// The requested resource does not exist.
    #[error("{code}: {message}")]
    NoSuchResource {
        /// AWS error code, e.g. `NoSuchDistribution`.
        code: &'static str,
        /// Human-readable message.
        message: String,
    },

    /// The caller submitted an invalid argument.
    #[error("InvalidArgument: {0}")]
    InvalidArgument(String),

    /// The request references an argument that is missing.
    #[error("MissingArgument: {0}")]
    MissingArgument(String),

    /// The resource is already in use.
    #[error("ResourceInUse: {0}")]
    ResourceInUse(String),

    /// The distribution (or similar config resource) is not currently disabled
    /// and therefore cannot be deleted.
    #[error("DistributionNotDisabled: {0}")]
    DistributionNotDisabled(String),

    /// If-Match ETag does not match the current resource ETag.
    #[error("PreconditionFailed: {0}")]
    PreconditionFailed(String),

    /// If-Match header was required but not provided.
    #[error("InvalidIfMatchVersion: {0}")]
    InvalidIfMatchVersion(String),

    /// The submitted configuration is invalid (referential integrity, shape).
    #[error("InvalidArgument: {0}")]
    MalformedInput(String),

    /// Attempt to create a resource that already exists.
    #[error("{code}: {message}")]
    AlreadyExists {
        /// AWS error code, e.g. `DistributionAlreadyExists`.
        code: &'static str,
        /// Human-readable message.
        message: String,
    },

    /// Feature not supported / not implemented in Rustack.
    #[error("NotImplemented: {0}")]
    NotImplemented(String),

    /// Access denied.
    #[error("AccessDenied: {0}")]
    AccessDenied(String),

    /// Generic internal error.
    #[error("InternalServerError: {0}")]
    Internal(String),
}

impl CloudFrontError {
    /// AWS error code string for the wire `<Code>` element.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoSuchResource { code, .. } | Self::AlreadyExists { code, .. } => code,
            Self::InvalidArgument(_) | Self::MalformedInput(_) => "InvalidArgument",
            Self::MissingArgument(_) => "MissingArgument",
            Self::ResourceInUse(_) => "ResourceInUse",
            Self::DistributionNotDisabled(_) => "DistributionNotDisabled",
            Self::PreconditionFailed(_) => "PreconditionFailed",
            Self::InvalidIfMatchVersion(_) => "InvalidIfMatchVersion",
            Self::NotImplemented(_) => "NotImplemented",
            Self::AccessDenied(_) => "AccessDenied",
            Self::Internal(_) => "InternalServerError",
        }
    }

    /// HTTP status code for this error variant.
    #[must_use]
    pub fn http_status(&self) -> u16 {
        match self {
            Self::NoSuchResource { .. } => 404,
            Self::InvalidArgument(_)
            | Self::MalformedInput(_)
            | Self::MissingArgument(_)
            | Self::InvalidIfMatchVersion(_) => 400,
            Self::ResourceInUse(_)
            | Self::AlreadyExists { .. }
            | Self::DistributionNotDisabled(_) => 409,
            Self::PreconditionFailed(_) => 412,
            Self::NotImplemented(_) => 501,
            Self::AccessDenied(_) => 403,
            Self::Internal(_) => 500,
        }
    }

    /// Human-readable message.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::NoSuchResource { message, .. } | Self::AlreadyExists { message, .. } => {
                message.clone()
            }
            Self::InvalidArgument(m)
            | Self::MissingArgument(m)
            | Self::ResourceInUse(m)
            | Self::DistributionNotDisabled(m)
            | Self::PreconditionFailed(m)
            | Self::InvalidIfMatchVersion(m)
            | Self::MalformedInput(m)
            | Self::NotImplemented(m)
            | Self::AccessDenied(m)
            | Self::Internal(m) => m.clone(),
        }
    }
}

/// Constructors for convenience.
impl CloudFrontError {
    /// "NoSuchDistribution" shortcut.
    #[must_use]
    pub fn no_such_distribution(id: impl Into<String>) -> Self {
        Self::NoSuchResource {
            code: "NoSuchDistribution",
            message: format!("The specified distribution does not exist: {}", id.into()),
        }
    }

    /// "NoSuchInvalidation" shortcut.
    #[must_use]
    pub fn no_such_invalidation(id: impl Into<String>) -> Self {
        Self::NoSuchResource {
            code: "NoSuchInvalidation",
            message: format!("The specified invalidation does not exist: {}", id.into()),
        }
    }

    /// "NoSuchOriginAccessControl" shortcut.
    #[must_use]
    pub fn no_such_origin_access_control(id: impl Into<String>) -> Self {
        Self::NoSuchResource {
            code: "NoSuchOriginAccessControl",
            message: format!(
                "The specified origin access control does not exist: {}",
                id.into()
            ),
        }
    }

    /// "NoSuchCloudFrontOriginAccessIdentity" shortcut.
    #[must_use]
    pub fn no_such_oai(id: impl Into<String>) -> Self {
        Self::NoSuchResource {
            code: "NoSuchCloudFrontOriginAccessIdentity",
            message: format!(
                "The specified origin access identity does not exist: {}",
                id.into()
            ),
        }
    }

    /// "NoSuchCachePolicy" shortcut.
    #[must_use]
    pub fn no_such_cache_policy(id: impl Into<String>) -> Self {
        Self::NoSuchResource {
            code: "NoSuchCachePolicy",
            message: format!("The specified cache policy does not exist: {}", id.into()),
        }
    }

    /// "NoSuchOriginRequestPolicy" shortcut.
    #[must_use]
    pub fn no_such_origin_request_policy(id: impl Into<String>) -> Self {
        Self::NoSuchResource {
            code: "NoSuchOriginRequestPolicy",
            message: format!(
                "The specified origin request policy does not exist: {}",
                id.into()
            ),
        }
    }

    /// "NoSuchResponseHeadersPolicy" shortcut.
    #[must_use]
    pub fn no_such_response_headers_policy(id: impl Into<String>) -> Self {
        Self::NoSuchResource {
            code: "NoSuchResponseHeadersPolicy",
            message: format!(
                "The specified response headers policy does not exist: {}",
                id.into()
            ),
        }
    }

    /// "NoSuchPublicKey" shortcut.
    #[must_use]
    pub fn no_such_public_key(id: impl Into<String>) -> Self {
        Self::NoSuchResource {
            code: "NoSuchPublicKey",
            message: format!("The specified public key does not exist: {}", id.into()),
        }
    }

    /// "NoSuchResource" generic shortcut.
    #[must_use]
    pub fn no_such_resource(kind: &'static str, id: impl Into<String>) -> Self {
        Self::NoSuchResource {
            code: "NoSuchResource",
            message: format!("The specified {kind} does not exist: {}", id.into()),
        }
    }
}
