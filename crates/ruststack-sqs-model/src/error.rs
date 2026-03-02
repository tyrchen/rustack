//! SQS error types.
//!
//! SQS errors use JSON format with a `__type` field and include an
//! `x-amzn-query-error` header for `awsQueryCompatible` support.

use std::fmt;

/// Well-known SQS error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum SqsErrorCode {
    /// Queue does not exist.
    #[default]
    NonExistentQueue,
    /// Queue already exists with different attributes.
    QueueAlreadyExists,
    /// Queue was deleted within last 60 seconds.
    QueueDeletedRecently,
    /// Invalid parameter value.
    InvalidParameterValue,
    /// Required parameter missing.
    MissingParameter,
    /// Invalid attribute name.
    InvalidAttributeName,
    /// Invalid attribute value.
    InvalidAttributeValue,
    /// Message is not currently in flight.
    MessageNotInflight,
    /// Receipt handle is invalid.
    ReceiptHandleIsInvalid,
    /// Batch request contains no entries.
    EmptyBatchRequest,
    /// More than 10 entries in batch.
    TooManyEntriesInBatchRequest,
    /// Duplicate IDs in batch request.
    BatchEntryIdsNotDistinct,
    /// Batch request exceeds size limit.
    BatchRequestTooLong,
    /// Invalid batch entry ID format.
    InvalidBatchEntryId,
    /// Another purge within 60 seconds.
    PurgeQueueInProgress,
    /// Queue limit exceeded.
    OverLimit,
    /// Resource not found (message move task).
    ResourceNotFoundException,
    /// Unsupported operation for queue type.
    UnsupportedOperation,
    /// Invalid security token or credentials.
    InvalidSecurity,
    /// Missing action header.
    MissingAction,
    /// Internal server error.
    InternalError,
}

impl SqsErrorCode {
    /// JSON `__type` field value (also used for `x-amzn-query-error` code).
    #[must_use]
    pub fn error_type(&self) -> &'static str {
        match self {
            Self::NonExistentQueue => "AWS.SimpleQueueService.NonExistentQueue",
            Self::QueueAlreadyExists => "QueueAlreadyExists",
            Self::QueueDeletedRecently => "AWS.SimpleQueueService.QueueDeletedRecently",
            Self::InvalidParameterValue => "InvalidParameterValue",
            Self::MissingParameter => "MissingParameter",
            Self::InvalidAttributeName => "InvalidAttributeName",
            Self::InvalidAttributeValue => "InvalidAttributeValue",
            Self::MessageNotInflight => "MessageNotInflight",
            Self::ReceiptHandleIsInvalid => "ReceiptHandleIsInvalid",
            Self::EmptyBatchRequest => "AWS.SimpleQueueService.EmptyBatchRequest",
            Self::TooManyEntriesInBatchRequest => {
                "AWS.SimpleQueueService.TooManyEntriesInBatchRequest"
            }
            Self::BatchEntryIdsNotDistinct => "AWS.SimpleQueueService.BatchEntryIdsNotDistinct",
            Self::BatchRequestTooLong => "AWS.SimpleQueueService.BatchRequestTooLong",
            Self::InvalidBatchEntryId => "AWS.SimpleQueueService.InvalidBatchEntryId",
            Self::PurgeQueueInProgress => "AWS.SimpleQueueService.PurgeQueueInProgress",
            Self::OverLimit => "OverLimit",
            Self::ResourceNotFoundException => "ResourceNotFoundException",
            Self::UnsupportedOperation => "AWS.SimpleQueueService.UnsupportedOperation",
            Self::InvalidSecurity => "InvalidSecurity",
            Self::MissingAction => "MissingAction",
            Self::InternalError => "InternalError",
        }
    }

    /// HTTP status code for this error.
    #[must_use]
    pub fn status_code(&self) -> http::StatusCode {
        match self {
            Self::PurgeQueueInProgress | Self::OverLimit => http::StatusCode::FORBIDDEN,
            Self::ResourceNotFoundException => http::StatusCode::NOT_FOUND,
            Self::InternalError => http::StatusCode::INTERNAL_SERVER_ERROR,
            _ => http::StatusCode::BAD_REQUEST,
        }
    }

    /// Whether this is a sender (client) fault vs receiver (server) fault.
    #[must_use]
    pub fn is_sender_fault(&self) -> bool {
        !matches!(self, Self::InternalError)
    }

    /// Fault type for the `x-amzn-query-error` header.
    #[must_use]
    pub fn fault(&self) -> &'static str {
        if self.is_sender_fault() {
            "Sender"
        } else {
            "Receiver"
        }
    }

    /// Full `x-amzn-query-error` header value: `"Code;Fault"`.
    #[must_use]
    pub fn query_error_header(&self) -> String {
        format!("{};{}", self.error_type(), self.fault())
    }
}

impl fmt::Display for SqsErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.error_type())
    }
}

/// An SQS error response.
#[derive(Debug)]
pub struct SqsError {
    /// The error code.
    pub code: SqsErrorCode,
    /// A human-readable error message.
    pub message: String,
}

impl fmt::Display for SqsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SqsError({}): {}", self.code, self.message)
    }
}

impl std::error::Error for SqsError {}

impl SqsError {
    /// Create a new `SqsError` with a code and message.
    #[must_use]
    pub fn new(code: SqsErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    // -- Convenience constructors --

    /// Queue does not exist.
    #[must_use]
    pub fn non_existent_queue(message: impl Into<String>) -> Self {
        Self::new(SqsErrorCode::NonExistentQueue, message)
    }

    /// Queue already exists with different attributes.
    #[must_use]
    pub fn queue_already_exists(message: impl Into<String>) -> Self {
        Self::new(SqsErrorCode::QueueAlreadyExists, message)
    }

    /// Queue deleted recently.
    #[must_use]
    pub fn queue_deleted_recently(message: impl Into<String>) -> Self {
        Self::new(SqsErrorCode::QueueDeletedRecently, message)
    }

    /// Invalid parameter value.
    #[must_use]
    pub fn invalid_parameter_value(message: impl Into<String>) -> Self {
        Self::new(SqsErrorCode::InvalidParameterValue, message)
    }

    /// Missing parameter.
    #[must_use]
    pub fn missing_parameter(message: impl Into<String>) -> Self {
        Self::new(SqsErrorCode::MissingParameter, message)
    }

    /// Invalid attribute name.
    #[must_use]
    pub fn invalid_attribute_name(message: impl Into<String>) -> Self {
        Self::new(SqsErrorCode::InvalidAttributeName, message)
    }

    /// Receipt handle is invalid.
    #[must_use]
    pub fn receipt_handle_is_invalid() -> Self {
        Self::new(
            SqsErrorCode::ReceiptHandleIsInvalid,
            "The input receipt handle is invalid.",
        )
    }

    /// Purge queue in progress.
    #[must_use]
    pub fn purge_queue_in_progress() -> Self {
        Self::new(
            SqsErrorCode::PurgeQueueInProgress,
            "Only one PurgeQueue operation on a queue is allowed every 60 seconds.",
        )
    }

    /// Empty batch request.
    #[must_use]
    pub fn empty_batch_request() -> Self {
        Self::new(
            SqsErrorCode::EmptyBatchRequest,
            "There should be at least one SendMessageBatchRequestEntry in the request.",
        )
    }

    /// Too many entries in batch request.
    #[must_use]
    pub fn too_many_entries_in_batch() -> Self {
        Self::new(
            SqsErrorCode::TooManyEntriesInBatchRequest,
            "Maximum number of entries per request are 10.",
        )
    }

    /// Batch entry IDs not distinct.
    #[must_use]
    pub fn batch_entry_ids_not_distinct() -> Self {
        Self::new(
            SqsErrorCode::BatchEntryIdsNotDistinct,
            "Two or more batch entries in the request have the same Id.",
        )
    }

    /// Missing action header.
    #[must_use]
    pub fn missing_action() -> Self {
        Self::new(
            SqsErrorCode::MissingAction,
            "Missing required header: X-Amz-Target or Action parameter",
        )
    }

    /// Unknown operation.
    #[must_use]
    pub fn unknown_operation(name: &str) -> Self {
        Self::new(
            SqsErrorCode::InvalidParameterValue,
            format!("Unrecognized operation: {name}"),
        )
    }

    /// Internal server error.
    #[must_use]
    pub fn internal_error(message: impl Into<String>) -> Self {
        Self::new(SqsErrorCode::InternalError, message)
    }

    /// Resource not found.
    #[must_use]
    pub fn resource_not_found(message: impl Into<String>) -> Self {
        Self::new(SqsErrorCode::ResourceNotFoundException, message)
    }
}

/// Create an `SqsError` from an error code.
///
/// # Examples
///
/// ```
/// use ruststack_sqs_model::sqs_error;
/// use ruststack_sqs_model::error::SqsErrorCode;
///
/// let err = sqs_error!(NonExistentQueue, "Queue not found");
/// assert_eq!(err.code, SqsErrorCode::NonExistentQueue);
/// ```
#[macro_export]
macro_rules! sqs_error {
    ($code:ident, $msg:expr) => {
        $crate::error::SqsError::new($crate::error::SqsErrorCode::$code, $msg)
    };
}
