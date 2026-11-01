//! Internal Lambda service error types.
//!
//! These errors represent business logic failures and are converted to
//! [`LambdaError`] for the HTTP response layer.

#[cfg(unix)]
use std::{error::Error, fmt};

use rustack_lambda_model::error::{LambdaError, LambdaErrorCode};
#[cfg(unix)]
use tokio::task::JoinError;

#[cfg(unix)]
use crate::docker::DockerError;

/// Typed validation worker failure; diagnostics never expose its panic payload.
#[cfg(unix)]
#[non_exhaustive]
pub struct ImageRestoreWorker {
    source: JoinError,
}
#[cfg(unix)]
impl From<JoinError> for ImageRestoreWorker {
    fn from(source: JoinError) -> Self {
        Self { source }
    }
}
#[cfg(unix)]
impl ImageRestoreWorker {
    /// Whether the original retained worker panicked.
    ///
    /// ```no_run
    /// # fn inspect(error: &rustack_lambda_core::error::ImageRestoreWorker) { let panicked = error.is_panic(); }
    /// ```
    #[must_use]
    pub fn is_panic(&self) -> bool {
        self.source.is_panic()
    }
    /// Whether the original retained worker was canceled.
    ///
    /// ```no_run
    /// # fn inspect(error: &rustack_lambda_core::error::ImageRestoreWorker) { let cancelled = error.is_cancelled(); }
    /// ```
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.source.is_cancelled()
    }
}
#[cfg(unix)]
impl fmt::Debug for ImageRestoreWorker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
#[cfg(unix)]
impl fmt::Display for ImageRestoreWorker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.is_panic() {
            "image archive validation worker panicked"
        } else {
            "image archive validation worker cancelled"
        })
    }
}
// Retain JoinError for typed classification; chaining its Display would leak
// arbitrary panic payload through startup's anyhow error rendering.
#[cfg(unix)]
impl Error for ImageRestoreWorker {}

/// Internal service error type for Lambda operations.
#[derive(Debug, thiserror::Error)]
pub enum LambdaServiceError {
    /// Invocation or accepted Event capacity is exhausted.
    #[error("Lambda concurrency or accepted-work capacity exhausted")]
    TooManyRequests,
    /// Artifact filesystem operation failed.
    #[error("Lambda artifact IO failed: {source}")]
    ArtifactIo {
        /// Underlying filesystem failure.
        #[source]
        source: std::io::Error,
    },
    /// Function does not exist.
    #[error("Function not found: {name}")]
    FunctionNotFound {
        /// Function name that was not found.
        name: String,
    },

    /// Published version does not exist.
    #[error("Version not found: {function_name}:{version}")]
    VersionNotFound {
        /// Function name.
        function_name: String,
        /// Version string.
        version: String,
    },

    /// Alias does not exist on the function.
    #[error("Alias not found: {function_name}:{alias}")]
    AliasNotFound {
        /// Function name.
        function_name: String,
        /// Alias name.
        alias: String,
    },

    /// Resource already exists (duplicate function name, alias, etc.).
    #[error("Resource already exists: {message}")]
    ResourceConflict {
        /// Description of the conflict.
        message: String,
    },

    /// An explicit optimistic-update precondition did not match.
    #[error("The supplied precondition does not match the current function revision")]
    PreconditionFailed,

    /// Invalid parameter value.
    #[error("Invalid parameter: {message}")]
    InvalidParameter {
        /// Description of the invalid parameter.
        message: String,
    },

    /// Invalid zip file content.
    #[error("Invalid zip file: {message}")]
    InvalidZipFile {
        /// Description of the zip file error.
        message: String,
    },

    /// Invalid ARN format.
    #[error("Invalid ARN: {arn}")]
    InvalidArn {
        /// The malformed ARN string.
        arn: String,
    },

    /// Resource exists but is not ready for the operation.
    #[error("Function not ready: {message}")]
    ResourceNotReady {
        /// Description of why the resource is not ready.
        message: String,
    },

    /// Docker runtime is not available for invocation.
    #[error("Docker not available")]
    DockerNotAvailable,

    /// Immutable image archive reacquisition failed without publishing resources.
    #[cfg(unix)]
    #[error("Lambda immutable image restore failed")]
    ImageRestore {
        /// Redacted typed daemon/ownership/deadline provenance.
        #[source]
        source: Box<DockerError>,
    },

    /// Retained image metadata validation worker failed.
    #[cfg(unix)]
    #[error("Lambda image archive validation worker failed")]
    ImageRestoreWorker {
        /// Typed worker provenance with redacted panic text.
        #[source]
        source: ImageRestoreWorker,
    },

    /// Event source mapping does not exist.
    #[error("Event source mapping not found: {uuid}")]
    EventSourceMappingNotFound {
        /// UUID that was not found.
        uuid: String,
    },

    /// Event invoke config does not exist.
    #[error("Event invoke config not found: {function_name}:{qualifier}")]
    EventInvokeConfigNotFound {
        /// Function name.
        function_name: String,
        /// Qualifier.
        qualifier: String,
    },

    /// Policy statement not found.
    #[error("Policy not found: {sid}")]
    PolicyNotFound {
        /// Statement ID that was not found.
        sid: String,
    },

    /// Request payload exceeds the size limit.
    #[error("Request too large: {message}")]
    RequestTooLarge {
        /// Description of the size violation.
        message: String,
    },

    /// Internal error (catch-all for unexpected failures).
    #[error("Internal error: {message}")]
    Internal {
        /// Description of the internal error.
        message: String,
    },
}

impl From<LambdaServiceError> for LambdaError {
    fn from(err: LambdaServiceError) -> Self {
        match err {
            LambdaServiceError::TooManyRequests => LambdaError::new(
                LambdaErrorCode::TooManyRequestsException,
                "Lambda concurrency or accepted-work capacity exhausted",
            ),
            LambdaServiceError::ArtifactIo { .. } => {
                LambdaError::service_error("Lambda artifact IO failed")
            }
            LambdaServiceError::FunctionNotFound { ref name } => {
                LambdaError::resource_not_found(format!(
                    "Function not found: arn:aws:lambda:us-east-1:000000000000:function:{name}"
                ))
            }
            LambdaServiceError::VersionNotFound {
                ref function_name,
                ref version,
            } => LambdaError::resource_not_found(format!(
                "Function not found: \
                 arn:aws:lambda:us-east-1:000000000000:function:{function_name}:{version}"
            )),
            LambdaServiceError::AliasNotFound {
                ref function_name,
                ref alias,
            } => LambdaError::resource_not_found(format!(
                "Function not found: \
                 arn:aws:lambda:us-east-1:000000000000:function:{function_name}:{alias}"
            )),
            LambdaServiceError::ResourceConflict { ref message } => {
                LambdaError::resource_conflict(message)
            }
            LambdaServiceError::PreconditionFailed => LambdaError::new(
                LambdaErrorCode::PreconditionFailedException,
                "The supplied precondition does not match the current function revision",
            ),
            LambdaServiceError::InvalidParameter { ref message } => {
                LambdaError::invalid_parameter(message)
            }
            LambdaServiceError::InvalidZipFile { ref message } => {
                LambdaError::invalid_parameter(format!("Invalid zip file: {message}"))
            }
            LambdaServiceError::InvalidArn { ref arn } => {
                LambdaError::invalid_parameter(format!("Invalid ARN: {arn}"))
            }
            LambdaServiceError::ResourceNotReady { ref message } => {
                LambdaError::new(LambdaErrorCode::ResourceNotReadyException, message.clone())
            }
            #[cfg(unix)]
            LambdaServiceError::ImageRestore { .. }
            | LambdaServiceError::ImageRestoreWorker { .. } => {
                LambdaError::service_error("Immutable image archive restore failed")
            }
            LambdaServiceError::DockerNotAvailable => LambdaError::service_error(
                "Docker execution is not available. Set LAMBDA_DOCKER_ENABLED=true to enable.",
            ),
            LambdaServiceError::RequestTooLarge { ref message } => {
                LambdaError::new(LambdaErrorCode::RequestTooLargeException, message.clone())
            }
            LambdaServiceError::EventSourceMappingNotFound { ref uuid } => {
                LambdaError::resource_not_found(format!(
                    "The resource you requested does not exist. (Service: Lambda, Status Code: \
                     404, Request ID: 00000000-0000-0000-0000-000000000000, Extended Request ID: \
                     null) UUID: {uuid}"
                ))
            }
            LambdaServiceError::PolicyNotFound { ref sid } => {
                LambdaError::resource_not_found(format!("No policy is found for: {sid}"))
            }
            LambdaServiceError::EventInvokeConfigNotFound {
                ref function_name,
                ref qualifier,
            } => LambdaError::resource_not_found(format!(
                "The function {function_name} doesn't have an EventInvokeConfig for qualifier \
                 {qualifier}"
            )),
            LambdaServiceError::Internal { ref message } => {
                LambdaError::service_error(message.clone())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_convert_function_not_found_to_lambda_error() {
        let err = LambdaServiceError::FunctionNotFound {
            name: "my-func".to_owned(),
        };
        let lambda_err: LambdaError = err.into();
        assert_eq!(lambda_err.code, LambdaErrorCode::ResourceNotFoundException);
        assert!(lambda_err.message.contains("my-func"));
    }

    #[test]
    fn test_should_convert_resource_conflict_to_lambda_error() {
        let err = LambdaServiceError::ResourceConflict {
            message: "already exists".to_owned(),
        };
        let lambda_err: LambdaError = err.into();
        assert_eq!(lambda_err.code, LambdaErrorCode::ResourceConflictException);
    }

    #[test]
    fn test_should_convert_docker_not_available_to_service_error() {
        let err = LambdaServiceError::DockerNotAvailable;
        let lambda_err: LambdaError = err.into();
        assert_eq!(lambda_err.code, LambdaErrorCode::ServiceException);
    }

    #[test]
    fn test_should_convert_invalid_parameter_to_lambda_error() {
        let err = LambdaServiceError::InvalidParameter {
            message: "bad value".to_owned(),
        };
        let lambda_err: LambdaError = err.into();
        assert_eq!(
            lambda_err.code,
            LambdaErrorCode::InvalidParameterValueException
        );
    }
}

#[cfg(test)]
mod precondition_tests {
    use super::*;

    #[test]
    fn test_should_encode_stale_revision_as_http_412_with_the_aws_error_code() {
        let error: LambdaError = LambdaServiceError::PreconditionFailed.into();
        assert_eq!(error.code.as_str(), "PreconditionFailedException");
        assert_eq!(
            error.code.status_code(),
            http::StatusCode::PRECONDITION_FAILED
        );
    }
}
