//! Auto-generated from AWS STS Smithy model. DO NOT EDIT.
//!
//! STS errors use XML format following the awsQuery protocol with
//! `<ErrorResponse>` containing `<Code>`, `<Message>`, and `<Type>` fields.

use std::fmt;

/// Well-known STS error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum StsErrorCode {
    /// ExpiredTokenException error.
    #[default]
    ExpiredTokenException,
    /// IDPCommunicationErrorException error.
    IDPCommunicationErrorException,
    /// IDPRejectedClaimException error.
    IDPRejectedClaimException,
    /// InternalError error.
    InternalError,
    /// InvalidAction error.
    InvalidAction,
    /// InvalidAuthorizationMessageException error.
    InvalidAuthorizationMessageException,
    /// InvalidClientTokenIdException error.
    InvalidClientTokenIdException,
    /// InvalidIdentityTokenException error.
    InvalidIdentityTokenException,
    /// InvalidParameterValue error.
    InvalidParameterValue,
    /// MalformedPolicyDocumentException error.
    MalformedPolicyDocumentException,
    /// MissingAction error.
    MissingAction,
    /// PackedPolicyTooLargeException error.
    PackedPolicyTooLargeException,
    /// RegionDisabledException error.
    RegionDisabledException,
}

impl StsErrorCode {
    /// Returns the short error code string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ExpiredTokenException => "ExpiredTokenException",
            Self::IDPCommunicationErrorException => "IDPCommunicationError",
            Self::IDPRejectedClaimException => "IDPRejectedClaim",
            Self::InternalError => "InternalFailure",
            Self::InvalidAction => "InvalidAction",
            Self::InvalidAuthorizationMessageException => "InvalidAuthorizationMessageException",
            Self::InvalidClientTokenIdException => "InvalidClientTokenId",
            Self::InvalidIdentityTokenException => "InvalidIdentityToken",
            Self::InvalidParameterValue => "InvalidParameterValue",
            Self::MalformedPolicyDocumentException => "MalformedPolicyDocument",
            Self::MissingAction => "MissingAction",
            Self::PackedPolicyTooLargeException => "PackedPolicyTooLarge",
            Self::RegionDisabledException => "RegionDisabledException",
        }
    }

    /// Returns the AWS error code string for XML `<Code>` element.
    #[must_use]
    pub fn code(&self) -> &'static str {
        self.as_str()
    }

    /// Returns the default HTTP status code for this error.
    #[must_use]
    pub fn default_status_code(&self) -> http::StatusCode {
        self.status_code()
    }

    /// Returns the HTTP status code for this error.
    #[must_use]
    pub fn status_code(&self) -> http::StatusCode {
        match self {
            Self::MissingAction
            | Self::InvalidAction
            | Self::MalformedPolicyDocumentException
            | Self::PackedPolicyTooLargeException
            | Self::InvalidIdentityTokenException
            | Self::IDPRejectedClaimException
            | Self::InvalidParameterValue
            | Self::InvalidAuthorizationMessageException => http::StatusCode::BAD_REQUEST,

            Self::ExpiredTokenException
            | Self::InvalidClientTokenIdException
            | Self::RegionDisabledException => http::StatusCode::FORBIDDEN,

            Self::IDPCommunicationErrorException | Self::InternalError => {
                http::StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }

    /// Returns the fault type (`Sender` or `Receiver`).
    #[must_use]
    pub fn fault(&self) -> &'static str {
        match self {
            Self::InternalError | Self::IDPCommunicationErrorException => "Receiver",
            _ => "Sender",
        }
    }
}

impl fmt::Display for StsErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An STS error response.
#[derive(Debug)]
pub struct StsError {
    /// The error code.
    pub code: StsErrorCode,
    /// A human-readable error message.
    pub message: String,
    /// The HTTP status code.
    pub status_code: http::StatusCode,
    /// The underlying source error, if any.
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl fmt::Display for StsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StsError({}): {}", self.code, self.message)
    }
}

impl std::error::Error for StsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e.as_ref() as &(dyn std::error::Error + 'static))
    }
}

impl StsError {
    /// Create a new `StsError` from an error code.
    #[must_use]
    pub fn new(code: StsErrorCode) -> Self {
        Self {
            status_code: code.default_status_code(),
            message: code.as_str().to_owned(),
            code,
            source: None,
        }
    }

    /// Create a new `StsError` with a custom message.
    #[must_use]
    pub fn with_message(code: StsErrorCode, message: impl Into<String>) -> Self {
        Self {
            status_code: code.default_status_code(),
            message: message.into(),
            code,
            source: None,
        }
    }

    /// Internal error.
    #[must_use]
    pub fn internal_error(message: impl Into<String>) -> Self {
        Self::with_message(StsErrorCode::InternalError, message)
    }

    /// Missing action parameter.
    #[must_use]
    pub fn missing_action() -> Self {
        Self::with_message(
            StsErrorCode::MissingAction,
            "Missing required parameter: Action",
        )
    }

    /// Unknown operation.
    #[must_use]
    pub fn unknown_operation(target: &str) -> Self {
        Self::with_message(
            StsErrorCode::InvalidAction,
            format!("Operation {target} is not supported."),
        )
    }

    /// Not implemented.
    #[must_use]
    pub fn not_implemented(operation: &str) -> Self {
        Self::with_message(
            StsErrorCode::InternalError,
            format!("Operation {operation} is not yet implemented"),
        )
    }

    /// Invalid parameter value.
    #[must_use]
    pub fn invalid_parameter_value(message: impl Into<String>) -> Self {
        Self::with_message(StsErrorCode::InvalidParameterValue, message)
    }

    /// Invalid client token ID.
    #[must_use]
    pub fn invalid_client_token_id(message: impl Into<String>) -> Self {
        Self::with_message(StsErrorCode::InvalidClientTokenIdException, message)
    }

    /// Invalid identity token.
    #[must_use]
    pub fn invalid_identity_token(message: impl Into<String>) -> Self {
        Self::with_message(StsErrorCode::InvalidIdentityTokenException, message)
    }

    /// Malformed policy document.
    #[must_use]
    pub fn malformed_policy(message: impl Into<String>) -> Self {
        Self::with_message(StsErrorCode::MalformedPolicyDocumentException, message)
    }

    /// Invalid action.
    #[must_use]
    pub fn invalid_action(message: impl Into<String>) -> Self {
        Self::with_message(StsErrorCode::InvalidAction, message)
    }
}

/// Create an `StsError` from an error code.
///
/// # Examples
///
/// ```ignore
/// let err = sts_error!(ExpiredTokenException);
/// assert_eq!(err.code, StsErrorCode::ExpiredTokenException);
/// ```
#[macro_export]
macro_rules! sts_error {
    ($code:ident) => {
        $crate::error::StsError::new($crate::error::StsErrorCode::$code)
    };
    ($code:ident, $msg:expr) => {
        $crate::error::StsError::with_message($crate::error::StsErrorCode::$code, $msg)
    };
}
