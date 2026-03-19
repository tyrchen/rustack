//! Kinesis service error types.

use ruststack_kinesis_model::error::{KinesisError, KinesisErrorCode};

/// Kinesis service errors.
#[derive(Debug, thiserror::Error)]
pub enum KinesisServiceError {
    /// The requested resource was not found.
    #[error("Stream {name} under account {account_id} not found")]
    ResourceNotFound {
        /// Resource name.
        name: String,
        /// Account ID.
        account_id: String,
    },

    /// The resource is already in use.
    #[error("{message}")]
    ResourceInUse {
        /// Error message.
        message: String,
    },

    /// An invalid argument was provided.
    #[error("{message}")]
    InvalidArgument {
        /// Error message.
        message: String,
    },

    /// A limit has been exceeded.
    #[error("{message}")]
    LimitExceeded {
        /// Error message.
        message: String,
    },

    /// An internal error occurred.
    #[error("{message}")]
    InternalError {
        /// Error message.
        message: String,
    },
}

impl KinesisServiceError {
    /// Returns the corresponding `KinesisErrorCode`.
    #[must_use]
    pub fn error_code(&self) -> KinesisErrorCode {
        match self {
            Self::ResourceNotFound { .. } => KinesisErrorCode::ResourceNotFoundException,
            Self::ResourceInUse { .. } => KinesisErrorCode::ResourceInUseException,
            Self::InvalidArgument { .. } => KinesisErrorCode::InvalidArgumentException,
            Self::LimitExceeded { .. } => KinesisErrorCode::LimitExceededException,
            Self::InternalError { .. } => KinesisErrorCode::InternalFailureException,
        }
    }

    /// Converts to a `KinesisError` from the model crate.
    #[must_use]
    pub fn to_kinesis_error(&self) -> KinesisError {
        KinesisError::with_message(self.error_code(), self.to_string())
    }
}

impl From<KinesisServiceError> for KinesisError {
    fn from(err: KinesisServiceError) -> Self {
        err.to_kinesis_error()
    }
}
