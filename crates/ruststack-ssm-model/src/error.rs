//! SSM error types.
//!
//! SSM errors use JSON format with a `__type` field containing the
//! short error type name (e.g., `ParameterNotFound`), unlike DynamoDB
//! which uses fully-qualified names.

use std::fmt;

/// Well-known SSM error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum SsmErrorCode {
    /// Parameter not found.
    ParameterNotFound,
    /// Parameter already exists (PutParameter without Overwrite).
    ParameterAlreadyExists,
    /// Maximum version limit exceeded (100 versions per parameter).
    ParameterMaxVersionLimitExceeded,
    /// Requested parameter version not found.
    ParameterVersionNotFound,
    /// Too many labels on a parameter version (max 10).
    ParameterVersionLabelLimitExceeded,
    /// Hierarchy level limit exceeded (max 15 levels).
    HierarchyLevelLimitExceeded,
    /// Type mismatch for a parameter in a hierarchy.
    HierarchyTypeMismatch,
    /// Invalid allowed pattern regex.
    InvalidAllowedPatternException,
    /// Value does not match the allowed pattern.
    ParameterPatternMismatchException,
    /// Invalid filter key.
    InvalidFilterKey,
    /// Invalid filter option.
    InvalidFilterOption,
    /// Invalid filter value.
    InvalidFilterValue,
    /// Invalid next token for pagination.
    InvalidNextToken,
    /// Invalid resource ID.
    InvalidResourceId,
    /// Invalid resource type.
    InvalidResourceType,
    /// Unsupported parameter type.
    UnsupportedParameterType,
    /// Too many tags (max 50 per resource).
    TooManyTagsError,
    /// Internal server error.
    InternalServerError,
    /// Invalid security (encryption key issue).
    InvalidSecurity,
    /// Invalid action (unrecognized operation).
    InvalidAction,
    /// Missing action header.
    MissingAction,
    /// Validation error.
    #[default]
    ValidationException,
}

impl SsmErrorCode {
    /// Returns the short error type string for the JSON `__type` field.
    ///
    /// SSM uses short names like `"ParameterNotFound"`, not fully-qualified
    /// names like DynamoDB.
    #[must_use]
    pub fn error_type(&self) -> &'static str {
        self.as_str()
    }

    /// Returns the short error code string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ParameterNotFound => "ParameterNotFound",
            Self::ParameterAlreadyExists => "ParameterAlreadyExists",
            Self::ParameterMaxVersionLimitExceeded => "ParameterMaxVersionLimitExceeded",
            Self::ParameterVersionNotFound => "ParameterVersionNotFound",
            Self::ParameterVersionLabelLimitExceeded => "ParameterVersionLabelLimitExceeded",
            Self::HierarchyLevelLimitExceeded => "HierarchyLevelLimitExceeded",
            Self::HierarchyTypeMismatch => "HierarchyTypeMismatch",
            Self::InvalidAllowedPatternException => "InvalidAllowedPatternException",
            Self::ParameterPatternMismatchException => "ParameterPatternMismatchException",
            Self::InvalidFilterKey => "InvalidFilterKey",
            Self::InvalidFilterOption => "InvalidFilterOption",
            Self::InvalidFilterValue => "InvalidFilterValue",
            Self::InvalidNextToken => "InvalidNextToken",
            Self::InvalidResourceId => "InvalidResourceId",
            Self::InvalidResourceType => "InvalidResourceType",
            Self::UnsupportedParameterType => "UnsupportedParameterType",
            Self::TooManyTagsError => "TooManyTagsError",
            Self::InternalServerError => "InternalServerError",
            Self::InvalidSecurity => "InvalidSecurity",
            Self::InvalidAction => "InvalidAction",
            Self::MissingAction => "MissingAction",
            Self::ValidationException => "ValidationException",
        }
    }

    /// Returns the default HTTP status code for this error.
    #[must_use]
    pub fn default_status_code(&self) -> http::StatusCode {
        match self {
            Self::InternalServerError => http::StatusCode::INTERNAL_SERVER_ERROR,
            _ => http::StatusCode::BAD_REQUEST,
        }
    }
}

impl fmt::Display for SsmErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An SSM error response.
#[derive(Debug)]
pub struct SsmError {
    /// The error code.
    pub code: SsmErrorCode,
    /// A human-readable error message.
    pub message: String,
    /// The HTTP status code.
    pub status_code: http::StatusCode,
    /// The underlying source error, if any.
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl fmt::Display for SsmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SsmError({}): {}", self.code, self.message)
    }
}

impl std::error::Error for SsmError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e.as_ref() as &(dyn std::error::Error + 'static))
    }
}

impl SsmError {
    /// Create a new `SsmError` from an error code.
    #[must_use]
    pub fn new(code: SsmErrorCode) -> Self {
        Self {
            status_code: code.default_status_code(),
            message: code.as_str().to_owned(),
            code,
            source: None,
        }
    }

    /// Create a new `SsmError` with a custom message.
    #[must_use]
    pub fn with_message(code: SsmErrorCode, message: impl Into<String>) -> Self {
        Self {
            status_code: code.default_status_code(),
            message: message.into(),
            code,
            source: None,
        }
    }

    /// Set the source error.
    #[must_use]
    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    /// Returns the `__type` string for the JSON error response.
    #[must_use]
    pub fn error_type(&self) -> &'static str {
        self.code.error_type()
    }

    // -- Convenience constructors --

    /// Parameter not found.
    #[must_use]
    pub fn parameter_not_found(name: &str) -> Self {
        Self::with_message(SsmErrorCode::ParameterNotFound, name.to_owned())
    }

    /// Parameter already exists.
    #[must_use]
    pub fn parameter_already_exists(name: &str) -> Self {
        Self::with_message(
            SsmErrorCode::ParameterAlreadyExists,
            format!(
                "The parameter {name} already exists. To overwrite this value, \
                 set the overwrite option in the request to true."
            ),
        )
    }

    /// Validation error.
    #[must_use]
    pub fn validation(message: impl Into<String>) -> Self {
        Self::with_message(SsmErrorCode::ValidationException, message)
    }

    /// Internal server error.
    #[must_use]
    pub fn internal_error(message: impl Into<String>) -> Self {
        Self::with_message(SsmErrorCode::InternalServerError, message)
    }

    /// Missing action header.
    #[must_use]
    pub fn missing_action() -> Self {
        Self::with_message(
            SsmErrorCode::MissingAction,
            "Missing required header: X-Amz-Target",
        )
    }

    /// Unknown operation.
    #[must_use]
    pub fn unknown_operation(target: &str) -> Self {
        Self::with_message(
            SsmErrorCode::InvalidAction,
            format!(
                "Operation {target} is not supported. \
                 Only Parameter Store operations are implemented."
            ),
        )
    }

    /// Not implemented.
    #[must_use]
    pub fn not_implemented(operation: &str) -> Self {
        Self::with_message(
            SsmErrorCode::InternalServerError,
            format!("Operation {operation} is not yet implemented"),
        )
    }
}

/// Create an `SsmError` from an error code.
///
/// # Examples
///
/// ```
/// use ruststack_ssm_model::ssm_error;
/// use ruststack_ssm_model::error::SsmErrorCode;
///
/// let err = ssm_error!(ValidationException);
/// assert_eq!(err.code, SsmErrorCode::ValidationException);
///
/// let err = ssm_error!(ParameterNotFound, "Parameter /my/param not found");
/// assert_eq!(err.message, "Parameter /my/param not found");
/// ```
#[macro_export]
macro_rules! ssm_error {
    ($code:ident) => {
        $crate::error::SsmError::new($crate::error::SsmErrorCode::$code)
    };
    ($code:ident, $msg:expr) => {
        $crate::error::SsmError::with_message($crate::error::SsmErrorCode::$code, $msg)
    };
}
