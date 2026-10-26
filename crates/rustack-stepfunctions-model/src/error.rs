//! Auto-generated from AWS Step Functions Smithy model. DO NOT EDIT.
//!
//! Step Functions errors use JSON format with a `__type` field containing the
//! short error type name (e.g., `ResourceNotFoundException`).

use std::fmt;

/// Well-known Step Functions error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum StepFunctionsErrorCode {
    /// `ActivityAlreadyExists` error.
    ActivityAlreadyExists,
    /// `ActivityDoesNotExist` error.
    ActivityDoesNotExist,
    /// `ActivityLimitExceeded` error.
    ActivityLimitExceeded,
    /// `ActivityWorkerLimitExceeded` error.
    ActivityWorkerLimitExceeded,
    /// `ConflictException` error.
    ConflictException,
    /// `ExecutionAlreadyExists` error.
    ExecutionAlreadyExists,
    /// `ExecutionDoesNotExist` error.
    ExecutionDoesNotExist,
    /// `ExecutionLimitExceeded` error.
    ExecutionLimitExceeded,
    /// `ExecutionNotRedrivable` error.
    ExecutionNotRedrivable,
    /// `InvalidArn` error.
    InvalidArn,
    /// `InvalidDefinition` error.
    InvalidDefinition,
    /// `InvalidEncryptionConfiguration` error.
    InvalidEncryptionConfiguration,
    /// `InvalidExecutionInput` error.
    InvalidExecutionInput,
    /// `InvalidLoggingConfiguration` error.
    InvalidLoggingConfiguration,
    /// `InvalidName` error.
    InvalidName,
    /// `InvalidOutput` error.
    InvalidOutput,
    /// `InvalidToken` error.
    InvalidToken,
    /// `InvalidTracingConfiguration` error.
    InvalidTracingConfiguration,
    /// `KmsAccessDeniedException` error.
    KmsAccessDeniedException,
    /// `KmsInvalidStateException` error.
    KmsInvalidStateException,
    /// `KmsThrottlingException` error.
    KmsThrottlingException,
    /// `MissingRequiredParameter` error.
    MissingRequiredParameter,
    /// `ResourceNotFound` error.
    ResourceNotFound,
    /// `ServiceQuotaExceededException` error.
    ServiceQuotaExceededException,
    /// `StateMachineAlreadyExists` error.
    StateMachineAlreadyExists,
    /// `StateMachineDeleting` error.
    StateMachineDeleting,
    /// `StateMachineDoesNotExist` error.
    StateMachineDoesNotExist,
    /// `StateMachineLimitExceeded` error.
    StateMachineLimitExceeded,
    /// `StateMachineTypeNotSupported` error.
    StateMachineTypeNotSupported,
    /// `TaskDoesNotExist` error.
    TaskDoesNotExist,
    /// `TaskTimedOut` error.
    TaskTimedOut,
    /// `TooManyTags` error.
    TooManyTags,
    /// `UnknownOperationException` error.
    UnknownOperationException,
    /// `UnsupportedOperation` error.
    UnsupportedOperation,
    /// `ValidationException` error.
    #[default]
    ValidationException,
}

impl StepFunctionsErrorCode {
    /// Returns the short error type string for the JSON `__type` field.
    #[must_use]
    pub fn error_type(&self) -> &'static str {
        self.as_str()
    }

    /// Returns the short error code string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ActivityAlreadyExists => "ActivityAlreadyExists",
            Self::ActivityDoesNotExist => "ActivityDoesNotExist",
            Self::ActivityLimitExceeded => "ActivityLimitExceeded",
            Self::ActivityWorkerLimitExceeded => "ActivityWorkerLimitExceeded",
            Self::ConflictException => "ConflictException",
            Self::ExecutionAlreadyExists => "ExecutionAlreadyExists",
            Self::ExecutionDoesNotExist => "ExecutionDoesNotExist",
            Self::ExecutionLimitExceeded => "ExecutionLimitExceeded",
            Self::ExecutionNotRedrivable => "ExecutionNotRedrivable",
            Self::InvalidArn => "InvalidArn",
            Self::InvalidDefinition => "InvalidDefinition",
            Self::InvalidEncryptionConfiguration => "InvalidEncryptionConfiguration",
            Self::InvalidExecutionInput => "InvalidExecutionInput",
            Self::InvalidLoggingConfiguration => "InvalidLoggingConfiguration",
            Self::InvalidName => "InvalidName",
            Self::InvalidOutput => "InvalidOutput",
            Self::InvalidToken => "InvalidToken",
            Self::InvalidTracingConfiguration => "InvalidTracingConfiguration",
            Self::KmsAccessDeniedException => "KmsAccessDeniedException",
            Self::KmsInvalidStateException => "KmsInvalidStateException",
            Self::KmsThrottlingException => "KmsThrottlingException",
            Self::MissingRequiredParameter => "MissingRequiredParameter",
            Self::ResourceNotFound => "ResourceNotFound",
            Self::ServiceQuotaExceededException => "ServiceQuotaExceededException",
            Self::StateMachineAlreadyExists => "StateMachineAlreadyExists",
            Self::StateMachineDeleting => "StateMachineDeleting",
            Self::StateMachineDoesNotExist => "StateMachineDoesNotExist",
            Self::StateMachineLimitExceeded => "StateMachineLimitExceeded",
            Self::StateMachineTypeNotSupported => "StateMachineTypeNotSupported",
            Self::TaskDoesNotExist => "TaskDoesNotExist",
            Self::TaskTimedOut => "TaskTimedOut",
            Self::TooManyTags => "TooManyTags",
            Self::UnknownOperationException => "UnknownOperationException",
            Self::UnsupportedOperation => "UnsupportedOperation",
            Self::ValidationException => "ValidationException",
        }
    }

    /// Returns the default HTTP status code for this error.
    #[must_use]
    pub fn default_status_code(&self) -> http::StatusCode {
        match self {
            Self::ActivityAlreadyExists
            | Self::ActivityDoesNotExist
            | Self::ActivityLimitExceeded
            | Self::ActivityWorkerLimitExceeded
            | Self::ExecutionAlreadyExists
            | Self::ExecutionDoesNotExist
            | Self::ExecutionLimitExceeded
            | Self::ExecutionNotRedrivable
            | Self::InvalidArn
            | Self::InvalidDefinition
            | Self::InvalidEncryptionConfiguration
            | Self::InvalidExecutionInput
            | Self::InvalidLoggingConfiguration
            | Self::InvalidName
            | Self::InvalidOutput
            | Self::InvalidToken
            | Self::InvalidTracingConfiguration
            | Self::KmsAccessDeniedException
            | Self::KmsInvalidStateException
            | Self::KmsThrottlingException
            | Self::MissingRequiredParameter
            | Self::StateMachineAlreadyExists
            | Self::StateMachineDeleting
            | Self::StateMachineDoesNotExist
            | Self::StateMachineLimitExceeded
            | Self::StateMachineTypeNotSupported
            | Self::TaskDoesNotExist
            | Self::TaskTimedOut
            | Self::TooManyTags
            | Self::UnsupportedOperation
            | Self::ValidationException => http::StatusCode::BAD_REQUEST,
            Self::ServiceQuotaExceededException => http::StatusCode::INTERNAL_SERVER_ERROR,
            Self::ResourceNotFound | Self::UnknownOperationException => http::StatusCode::NOT_FOUND,
            Self::ConflictException => http::StatusCode::CONFLICT,
        }
    }
}

impl fmt::Display for StepFunctionsErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl StepFunctionsErrorCode {
    /// Default bounded protocol message.
    #[must_use]
    pub fn default_message(&self) -> &'static str {
        match self {
            Self::ActivityAlreadyExists => "ActivityAlreadyExists",
            Self::ActivityDoesNotExist => "ActivityDoesNotExist",
            Self::ActivityLimitExceeded => "ActivityLimitExceeded",
            Self::ActivityWorkerLimitExceeded => "ActivityWorkerLimitExceeded",
            Self::ConflictException => "ConflictException",
            Self::ExecutionAlreadyExists => "ExecutionAlreadyExists",
            Self::ExecutionDoesNotExist => "ExecutionDoesNotExist",
            Self::ExecutionLimitExceeded => "ExecutionLimitExceeded",
            Self::ExecutionNotRedrivable => "ExecutionNotRedrivable",
            Self::InvalidArn => "InvalidArn",
            Self::InvalidDefinition => "InvalidDefinition",
            Self::InvalidEncryptionConfiguration => "InvalidEncryptionConfiguration",
            Self::InvalidExecutionInput => "InvalidExecutionInput",
            Self::InvalidLoggingConfiguration => "InvalidLoggingConfiguration",
            Self::InvalidName => "InvalidName",
            Self::InvalidOutput => "InvalidOutput",
            Self::InvalidToken => "InvalidToken",
            Self::InvalidTracingConfiguration => "InvalidTracingConfiguration",
            Self::KmsAccessDeniedException => "KmsAccessDeniedException",
            Self::KmsInvalidStateException => "KmsInvalidStateException",
            Self::KmsThrottlingException => "KmsThrottlingException",
            Self::MissingRequiredParameter => "MissingRequiredParameter",
            Self::ResourceNotFound => "ResourceNotFound",
            Self::ServiceQuotaExceededException => "ServiceQuotaExceededException",
            Self::StateMachineAlreadyExists => "StateMachineAlreadyExists",
            Self::StateMachineDeleting => "StateMachineDeleting",
            Self::StateMachineDoesNotExist => "StateMachineDoesNotExist",
            Self::StateMachineLimitExceeded => "StateMachineLimitExceeded",
            Self::StateMachineTypeNotSupported => "StateMachineTypeNotSupported",
            Self::TaskDoesNotExist => "TaskDoesNotExist",
            Self::TaskTimedOut => "TaskTimedOut",
            Self::TooManyTags => "TooManyTags",
            Self::UnknownOperationException => "Unknown Step Functions operation",
            Self::UnsupportedOperation => {
                "Operation is outside the supported Step Functions profile"
            }
            Self::ValidationException => "ValidationException",
        }
    }
}
/// An Step Functions error response.
#[non_exhaustive]
pub struct StepFunctionsError {
    /// The error code.
    pub code: StepFunctionsErrorCode,
    /// A human-readable error message.
    pub message: String,
    /// The HTTP status code.
    pub status_code: http::StatusCode,
    /// The underlying source error, if any.
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl fmt::Debug for StepFunctionsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StepFunctionsError(<redacted>)")
    }
}
impl fmt::Display for StepFunctionsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StepFunctionsError(<redacted>)")
    }
}

impl std::error::Error for StepFunctionsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e.as_ref() as &(dyn std::error::Error + 'static))
    }
}

impl StepFunctionsError {
    /// Create a new `StepFunctionsError` from an error code.
    #[must_use]
    pub fn new(code: StepFunctionsErrorCode) -> Self {
        Self {
            status_code: code.default_status_code(),
            message: code.default_message().to_owned(),
            code,
            source: None,
        }
    }

    /// Create a new `StepFunctionsError` with a custom message.
    #[must_use]
    pub fn with_message(code: StepFunctionsErrorCode, message: impl Into<String>) -> Self {
        Self {
            status_code: code.default_status_code(),
            message: message.into(),
            code,
            source: None,
        }
    }

    /// Returns the `__type` string for the JSON error response.
    #[must_use]
    pub fn error_type(&self) -> &'static str {
        self.code.error_type()
    }

    /// Validation error.
    #[must_use]
    pub fn validation(message: impl Into<String>) -> Self {
        Self::with_message(StepFunctionsErrorCode::ValidationException, message)
    }
}

/// Create an `StepFunctionsError` from an error code.
///
/// # Examples
///
/// ```
/// use rustack_stepfunctions_model::{stepfunctions_error, StepFunctionsErrorCode};
/// let err = stepfunctions_error!(ValidationException);
/// assert_eq!(err.code, StepFunctionsErrorCode::ValidationException);
/// ```
#[macro_export]
macro_rules! stepfunctions_error {
    ($code:ident) => {
        $crate::error::StepFunctionsError::new($crate::error::StepFunctionsErrorCode::$code)
    };
    ($code:ident, $msg:expr) => {
        $crate::error::StepFunctionsError::with_message(
            $crate::error::StepFunctionsErrorCode::$code,
            $msg,
        )
    };
}
