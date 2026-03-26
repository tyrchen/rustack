//! Internal Lambda service error types.
//!
//! These errors represent business logic failures and are converted to
//! [`LambdaError`] for the HTTP response layer.

use ruststack_lambda_model::error::{LambdaError, LambdaErrorCode};

/// Internal service error type for Lambda operations.
#[derive(Debug, thiserror::Error)]
pub enum LambdaServiceError {
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
