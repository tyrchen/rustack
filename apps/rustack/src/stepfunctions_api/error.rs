//! Static service diagnostics and retained typed sources.

use std::{error::Error, fmt, sync::Arc};

use http::StatusCode;
use rustack_stepfunctions_core::{CallbackError, ExecutionError, RegistryError};
use rustack_stepfunctions_model::{
    StepFunctionsError, StepFunctionsErrorCode as Code, StepFunctionsOperation,
};
use thiserror::Error;
use tokio::{task::JoinError, time::Instant};

/// API-owner failures; Debug and public error text never project business data.
#[derive(Error)]
#[non_exhaustive]
pub enum ApiError {
    /// Invalid trusted capacities/scope/runtime configuration.
    #[error("invalid workflow API configuration")]
    Configuration,
    /// An observation expired while its owner remains live.
    #[error("workflow API observation expired")]
    Deadline,
    /// Work remains uncertain after unexpected worker failure.
    #[error("workflow API cleanup retained")]
    Quarantined,
    /// Supervised root/worker failure with redacted diagnostics.
    #[error("workflow API worker failed")]
    Worker(#[source] JoinError),
    /// Cached root failure.
    #[error("workflow API owner failed")]
    Owner(#[source] Arc<ApiError>),
}
impl fmt::Debug for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
pub(super) fn error(code: Code) -> StepFunctionsError {
    StepFunctionsError::new(code)
}
pub(super) fn source(code: Code, cause: impl Error + Send + Sync + 'static) -> StepFunctionsError {
    let mut error = error(code);
    error.source = Some(Box::new(cause));
    error
}
pub(super) fn registry(
    cause: RegistryError,
    operation: StepFunctionsOperation,
) -> StepFunctionsError {
    let code = match &cause {
        RegistryError::Validation => Code::ValidationException,
        RegistryError::MachineMissing => match operation {
            StepFunctionsOperation::TagResource
            | StepFunctionsOperation::UntagResource
            | StepFunctionsOperation::ListTagsForResource => Code::ResourceNotFound,
            _ => Code::StateMachineDoesNotExist,
        },
        RegistryError::TagLimit => Code::TooManyTags,
        RegistryError::MachineExists => Code::StateMachineAlreadyExists,
        RegistryError::Deleting => Code::StateMachineDeleting,
        RegistryError::ExecutionMissing => Code::ExecutionDoesNotExist,
        RegistryError::ExecutionExists => Code::ExecutionAlreadyExists,
        RegistryError::Capacity => match operation {
            StepFunctionsOperation::CreateStateMachine => Code::StateMachineLimitExceeded,
            StepFunctionsOperation::StartExecution => Code::ExecutionLimitExceeded,
            _ => Code::ThrottlingException,
        },
        RegistryError::Deadline => Code::RequestTimeoutException,
        RegistryError::Unavailable => Code::ServiceUnavailable,
        RegistryError::Execution(ExecutionError::Capacity) => Code::ExecutionLimitExceeded,
        _ => Code::InternalFailure,
    };
    let mut result = source(code, cause);
    if code == Code::ResourceNotFound {
        result.status_code = StatusCode::BAD_REQUEST;
    }
    result
}
pub(super) fn check(deadline: Instant) -> Result<(), StepFunctionsError> {
    if Instant::now() >= deadline {
        Err(error(Code::RequestTimeoutException))
    } else {
        Ok(())
    }
}

pub(super) fn callback(cause: CallbackError) -> StepFunctionsError {
    source(callback_code(&cause), cause)
}
fn callback_code(cause: &CallbackError) -> Code {
    match cause {
        CallbackError::Cached(cause) => callback_code(cause),
        CallbackError::InvalidToken => Code::InvalidToken,
        CallbackError::TaskTimedOut => Code::TaskTimedOut,
        CallbackError::Data(_) => Code::InvalidOutput,
        CallbackError::Validation => Code::ValidationException,
        CallbackError::Capacity => Code::ThrottlingException,
        CallbackError::Deadline => Code::RequestTimeoutException,
        CallbackError::Unavailable => Code::ServiceUnavailable,
        _ => Code::InternalFailure,
    }
}
