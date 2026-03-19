//! Auto-generated from AWS KMS Smithy model. DO NOT EDIT.
//!
//! KMS errors use JSON format with a `__type` field containing the
//! short error type name (e.g., `ResourceNotFoundException`).

use std::fmt;

/// Well-known KMS error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum KmsErrorCode {
    /// AlreadyExistsException error.
    #[default]
    AlreadyExistsException,
    /// CloudHsmClusterInvalidConfigurationException error.
    CloudHsmClusterInvalidConfigurationException,
    /// CustomKeyStoreInvalidStateException error.
    CustomKeyStoreInvalidStateException,
    /// CustomKeyStoreNotFoundException error.
    CustomKeyStoreNotFoundException,
    /// DependencyTimeoutException error.
    DependencyTimeoutException,
    /// DisabledException error.
    DisabledException,
    /// DryRunOperationException error.
    DryRunOperationException,
    /// IncorrectKeyException error.
    IncorrectKeyException,
    /// InvalidAction error.
    InvalidAction,
    /// InvalidAliasNameException error.
    InvalidAliasNameException,
    /// InvalidArnException error.
    InvalidArnException,
    /// InvalidCiphertextException error.
    InvalidCiphertextException,
    /// InvalidGrantIdException error.
    InvalidGrantIdException,
    /// InvalidGrantTokenException error.
    InvalidGrantTokenException,
    /// InvalidKeyUsageException error.
    InvalidKeyUsageException,
    /// InvalidMarkerException error.
    InvalidMarkerException,
    /// KMSInternalException error.
    KMSInternalException,
    /// KMSInvalidMacException error.
    KMSInvalidMacException,
    /// KMSInvalidSignatureException error.
    KMSInvalidSignatureException,
    /// KMSInvalidStateException error.
    KMSInvalidStateException,
    /// KeyUnavailableException error.
    KeyUnavailableException,
    /// LimitExceededException error.
    LimitExceededException,
    /// MalformedPolicyDocumentException error.
    MalformedPolicyDocumentException,
    /// MissingAction error.
    MissingAction,
    /// NotFoundException error.
    NotFoundException,
    /// TagException error.
    TagException,
    /// UnsupportedOperationException error.
    UnsupportedOperationException,
    /// XksKeyAlreadyInUseException error.
    XksKeyAlreadyInUseException,
    /// XksKeyInvalidConfigurationException error.
    XksKeyInvalidConfigurationException,
    /// XksKeyNotFoundException error.
    XksKeyNotFoundException,
}

impl KmsErrorCode {
    /// Returns the short error type string for the JSON `__type` field.
    #[must_use]
    pub fn error_type(&self) -> &'static str {
        self.as_str()
    }

    /// Returns the short error code string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AlreadyExistsException => "AlreadyExistsException",
            Self::CloudHsmClusterInvalidConfigurationException => {
                "CloudHsmClusterInvalidConfigurationException"
            }
            Self::CustomKeyStoreInvalidStateException => "CustomKeyStoreInvalidStateException",
            Self::CustomKeyStoreNotFoundException => "CustomKeyStoreNotFoundException",
            Self::DependencyTimeoutException => "DependencyTimeoutException",
            Self::DisabledException => "DisabledException",
            Self::DryRunOperationException => "DryRunOperationException",
            Self::IncorrectKeyException => "IncorrectKeyException",
            Self::InvalidAction => "InvalidAction",
            Self::InvalidAliasNameException => "InvalidAliasNameException",
            Self::InvalidArnException => "InvalidArnException",
            Self::InvalidCiphertextException => "InvalidCiphertextException",
            Self::InvalidGrantIdException => "InvalidGrantIdException",
            Self::InvalidGrantTokenException => "InvalidGrantTokenException",
            Self::InvalidKeyUsageException => "InvalidKeyUsageException",
            Self::InvalidMarkerException => "InvalidMarkerException",
            Self::KMSInternalException => "KMSInternalException",
            Self::KMSInvalidMacException => "KMSInvalidMacException",
            Self::KMSInvalidSignatureException => "KMSInvalidSignatureException",
            Self::KMSInvalidStateException => "KMSInvalidStateException",
            Self::KeyUnavailableException => "KeyUnavailableException",
            Self::LimitExceededException => "LimitExceededException",
            Self::MalformedPolicyDocumentException => "MalformedPolicyDocumentException",
            Self::MissingAction => "MissingAction",
            Self::NotFoundException => "NotFoundException",
            Self::TagException => "TagException",
            Self::UnsupportedOperationException => "UnsupportedOperationException",
            Self::XksKeyAlreadyInUseException => "XksKeyAlreadyInUseException",
            Self::XksKeyInvalidConfigurationException => "XksKeyInvalidConfigurationException",
            Self::XksKeyNotFoundException => "XksKeyNotFoundException",
        }
    }

    /// Returns the default HTTP status code for this error.
    #[must_use]
    pub fn default_status_code(&self) -> http::StatusCode {
        match self {
            Self::CloudHsmClusterInvalidConfigurationException
            | Self::CustomKeyStoreInvalidStateException
            | Self::CustomKeyStoreNotFoundException
            | Self::IncorrectKeyException
            | Self::InvalidAction
            | Self::InvalidAliasNameException
            | Self::InvalidArnException
            | Self::InvalidCiphertextException
            | Self::InvalidGrantIdException
            | Self::InvalidGrantTokenException
            | Self::InvalidKeyUsageException
            | Self::InvalidMarkerException
            | Self::KMSInvalidMacException
            | Self::KMSInvalidSignatureException
            | Self::LimitExceededException
            | Self::MalformedPolicyDocumentException
            | Self::MissingAction
            | Self::TagException
            | Self::UnsupportedOperationException
            | Self::XksKeyAlreadyInUseException
            | Self::XksKeyInvalidConfigurationException
            | Self::XksKeyNotFoundException => http::StatusCode::BAD_REQUEST,
            Self::NotFoundException => http::StatusCode::NOT_FOUND,
            Self::AlreadyExistsException
            | Self::DisabledException
            | Self::KMSInvalidStateException => http::StatusCode::CONFLICT,
            Self::DryRunOperationException => http::StatusCode::PRECONDITION_FAILED,
            Self::KMSInternalException | Self::KeyUnavailableException => {
                http::StatusCode::INTERNAL_SERVER_ERROR
            }
            Self::DependencyTimeoutException => http::StatusCode::SERVICE_UNAVAILABLE,
        }
    }
}

impl fmt::Display for KmsErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A KMS error response.
#[derive(Debug)]
pub struct KmsError {
    /// The error code.
    pub code: KmsErrorCode,
    /// A human-readable error message.
    pub message: String,
    /// The HTTP status code.
    pub status_code: http::StatusCode,
    /// The underlying source error, if any.
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl fmt::Display for KmsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "KmsError({}): {}", self.code, self.message)
    }
}

impl std::error::Error for KmsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e.as_ref() as &(dyn std::error::Error + 'static))
    }
}

impl KmsError {
    /// Create a new `KmsError` from an error code.
    #[must_use]
    pub fn new(code: KmsErrorCode) -> Self {
        Self {
            status_code: code.default_status_code(),
            message: code.as_str().to_owned(),
            code,
            source: None,
        }
    }

    /// Create a new `KmsError` with a custom message.
    #[must_use]
    pub fn with_message(code: KmsErrorCode, message: impl Into<String>) -> Self {
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
        Self::with_message(KmsErrorCode::KMSInternalException, message)
    }

    /// Validation error.
    #[must_use]
    pub fn validation(message: impl Into<String>) -> Self {
        Self::with_message(KmsErrorCode::InvalidArnException, message)
    }

    /// Missing action header.
    #[must_use]
    pub fn missing_action() -> Self {
        Self::with_message(
            KmsErrorCode::MissingAction,
            "Missing required header: X-Amz-Target",
        )
    }

    /// Unknown operation.
    #[must_use]
    pub fn unknown_operation(target: &str) -> Self {
        Self::with_message(
            KmsErrorCode::InvalidAction,
            format!("Operation {target} is not supported."),
        )
    }

    /// Not implemented.
    #[must_use]
    pub fn not_implemented(operation: &str) -> Self {
        Self::with_message(
            KmsErrorCode::KMSInternalException,
            format!("Operation {operation} is not yet implemented"),
        )
    }
}

/// Create an `KmsError` from an error code.
///
/// # Examples
///
/// ```ignore
/// let err = kms_error!(AlreadyExistsException);
/// assert_eq!(err.code, KmsErrorCode::AlreadyExistsException);
/// ```
#[macro_export]
macro_rules! kms_error {
    ($code:ident) => {
        $crate::error::KmsError::new($crate::error::KmsErrorCode::$code)
    };
    ($code:ident, $msg:expr) => {
        $crate::error::KmsError::with_message($crate::error::KmsErrorCode::$code, $msg)
    };
}
