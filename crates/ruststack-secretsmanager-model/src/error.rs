//! Auto-generated from AWS Secrets Manager Smithy model. DO NOT EDIT.
//!
//! Secrets Manager errors use JSON format with a `__type` field containing the
//! short error type name (e.g., `ResourceNotFoundException`).

use std::fmt;

/// Well-known Secrets Manager error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum SecretsManagerErrorCode {
    /// DecryptionFailure error.
    #[default]
    DecryptionFailure,
    /// EncryptionFailure error.
    EncryptionFailure,
    /// InternalServiceError error.
    InternalServiceError,
    /// InvalidAction error.
    InvalidAction,
    /// InvalidNextTokenException error.
    InvalidNextTokenException,
    /// InvalidParameterException error.
    InvalidParameterException,
    /// InvalidRequestException error.
    InvalidRequestException,
    /// LimitExceededException error.
    LimitExceededException,
    /// MalformedPolicyDocumentException error.
    MalformedPolicyDocumentException,
    /// MissingAction error.
    MissingAction,
    /// PreconditionNotMetException error.
    PreconditionNotMetException,
    /// PublicPolicyException error.
    PublicPolicyException,
    /// ResourceExistsException error.
    ResourceExistsException,
    /// ResourceNotFoundException error.
    ResourceNotFoundException,
}

impl SecretsManagerErrorCode {
    /// Returns the short error type string for the JSON `__type` field.
    #[must_use]
    pub fn error_type(&self) -> &'static str {
        self.as_str()
    }

    /// Returns the short error code string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DecryptionFailure => "DecryptionFailure",
            Self::EncryptionFailure => "EncryptionFailure",
            Self::InternalServiceError => "InternalServiceError",
            Self::InvalidAction => "InvalidAction",
            Self::InvalidNextTokenException => "InvalidNextTokenException",
            Self::InvalidParameterException => "InvalidParameterException",
            Self::InvalidRequestException => "InvalidRequestException",
            Self::LimitExceededException => "LimitExceededException",
            Self::MalformedPolicyDocumentException => "MalformedPolicyDocumentException",
            Self::MissingAction => "MissingAction",
            Self::PreconditionNotMetException => "PreconditionNotMetException",
            Self::PublicPolicyException => "PublicPolicyException",
            Self::ResourceExistsException => "ResourceExistsException",
            Self::ResourceNotFoundException => "ResourceNotFoundException",
        }
    }

    /// Returns the default HTTP status code for this error.
    #[must_use]
    pub fn default_status_code(&self) -> http::StatusCode {
        match self {
            Self::DecryptionFailure
            | Self::EncryptionFailure
            | Self::InvalidAction
            | Self::InvalidNextTokenException
            | Self::InvalidParameterException
            | Self::InvalidRequestException
            | Self::LimitExceededException
            | Self::MalformedPolicyDocumentException
            | Self::MissingAction
            | Self::PreconditionNotMetException
            | Self::PublicPolicyException
            | Self::ResourceExistsException
            | Self::ResourceNotFoundException => http::StatusCode::BAD_REQUEST,
            Self::InternalServiceError => http::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl fmt::Display for SecretsManagerErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An Secrets Manager error response.
#[derive(Debug)]
pub struct SecretsManagerError {
    /// The error code.
    pub code: SecretsManagerErrorCode,
    /// A human-readable error message.
    pub message: String,
    /// The HTTP status code.
    pub status_code: http::StatusCode,
    /// The underlying source error, if any.
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl fmt::Display for SecretsManagerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretsManagerError({}): {}", self.code, self.message)
    }
}

impl std::error::Error for SecretsManagerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e.as_ref() as &(dyn std::error::Error + 'static))
    }
}

impl SecretsManagerError {
    /// Create a new `SecretsManagerError` from an error code.
    #[must_use]
    pub fn new(code: SecretsManagerErrorCode) -> Self {
        Self {
            status_code: code.default_status_code(),
            message: code.as_str().to_owned(),
            code,
            source: None,
        }
    }

    /// Create a new `SecretsManagerError` with a custom message.
    #[must_use]
    pub fn with_message(code: SecretsManagerErrorCode, message: impl Into<String>) -> Self {
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

    /// Internal error.
    #[must_use]
    pub fn internal_error(message: impl Into<String>) -> Self {
        Self::with_message(SecretsManagerErrorCode::InternalServiceError, message)
    }

    /// Missing action header.
    #[must_use]
    pub fn missing_action() -> Self {
        Self::with_message(
            SecretsManagerErrorCode::MissingAction,
            "Missing required header: X-Amz-Target",
        )
    }

    /// Unknown operation.
    #[must_use]
    pub fn unknown_operation(target: &str) -> Self {
        Self::with_message(
            SecretsManagerErrorCode::InvalidAction,
            format!("Operation {target} is not supported."),
        )
    }

    /// Not implemented.
    #[must_use]
    pub fn not_implemented(operation: &str) -> Self {
        Self::with_message(
            SecretsManagerErrorCode::InternalServiceError,
            format!("Operation {operation} is not yet implemented"),
        )
    }
}

/// Create an `SecretsManagerError` from an error code.
///
/// # Examples
///
/// ```ignore
/// let err = secretsmanager_error!(DecryptionFailure);
/// assert_eq!(err.code, SecretsManagerErrorCode::DecryptionFailure);
/// ```
#[macro_export]
macro_rules! secretsmanager_error {
    ($code:ident) => {
        $crate::error::SecretsManagerError::new($crate::error::SecretsManagerErrorCode::$code)
    };
    ($code:ident, $msg:expr) => {
        $crate::error::SecretsManagerError::with_message(
            $crate::error::SecretsManagerErrorCode::$code,
            $msg,
        )
    };
}
