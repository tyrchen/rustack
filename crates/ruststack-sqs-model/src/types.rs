//! Shared SQS types.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Serde helper for base64-encoded `Option<Vec<u8>>`.
///
/// AWS SQS JSON protocol encodes binary values as base64 strings, not JSON
/// arrays of integers. This module handles the conversion transparently.
mod base64_option {
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    /// Serialize `Option<Vec<u8>>` as an optional base64 string.
    #[allow(clippy::ref_option)] // serde(with) requires `&Option<T>` signature.
    pub fn serialize<S>(value: &Option<Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(bytes) => {
                let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                serializer.serialize_some(&encoded)
            }
            None => serializer.serialize_none(),
        }
    }

    /// Deserialize an optional base64 string into `Option<Vec<u8>>`.
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Vec<u8>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt: Option<String> = Option::deserialize(deserializer)?;
        match opt {
            Some(s) => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(&s)
                    .map_err(serde::de::Error::custom)?;
                Ok(Some(bytes))
            }
            None => Ok(None),
        }
    }
}

/// A message attribute value (user-defined).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MessageAttributeValue {
    /// The data type: `String`, `Number`, `Binary`, or custom types like
    /// `String.MyCustomType`.
    pub data_type: String,
    /// The string value (for `String` and `Number` types).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub string_value: Option<String>,
    /// The binary value (for `Binary` type), transmitted as base64.
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "base64_option",
        default
    )]
    pub binary_value: Option<Vec<u8>>,
}

/// A system message attribute value.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MessageSystemAttributeValue {
    /// The data type (always `String` for system attributes).
    pub data_type: String,
    /// The string value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub string_value: Option<String>,
    /// The binary value, transmitted as base64.
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "base64_option",
        default
    )]
    pub binary_value: Option<Vec<u8>>,
}

/// Queue attribute names that can be requested or set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum QueueAttributeName {
    /// All attributes.
    All,
    /// Approximate number of messages available.
    ApproximateNumberOfMessages,
    /// Approximate number of messages delayed.
    ApproximateNumberOfMessagesDelayed,
    /// Approximate number of messages not visible (in-flight).
    ApproximateNumberOfMessagesNotVisible,
    /// Content-based deduplication (FIFO only).
    ContentBasedDeduplication,
    /// Queue creation timestamp.
    CreatedTimestamp,
    /// Deduplication scope (FIFO only).
    DeduplicationScope,
    /// Default delay in seconds.
    DelaySeconds,
    /// FIFO throughput limit (FIFO only).
    FifoThroughputLimit,
    /// Whether the queue is FIFO.
    FifoQueue,
    /// KMS data key reuse period.
    KmsDataKeyReusePeriodSeconds,
    /// KMS master key ID.
    KmsMasterKeyId,
    /// Last modified timestamp.
    LastModifiedTimestamp,
    /// Maximum message size in bytes.
    MaximumMessageSize,
    /// Message retention period in seconds.
    MessageRetentionPeriod,
    /// IAM policy JSON.
    Policy,
    /// Queue ARN.
    QueueArn,
    /// Default wait time for receive operations.
    ReceiveMessageWaitTimeSeconds,
    /// Redrive allow policy.
    RedriveAllowPolicy,
    /// Redrive policy (DLQ configuration).
    RedrivePolicy,
    /// SQS-managed server-side encryption enabled.
    SqsManagedSseEnabled,
    /// Default visibility timeout.
    VisibilityTimeout,
}

impl QueueAttributeName {
    /// Parse an attribute name from a string.
    #[must_use]
    pub fn from_str_name(s: &str) -> Option<Self> {
        match s {
            "All" => Some(Self::All),
            "ApproximateNumberOfMessages" => Some(Self::ApproximateNumberOfMessages),
            "ApproximateNumberOfMessagesDelayed" => Some(Self::ApproximateNumberOfMessagesDelayed),
            "ApproximateNumberOfMessagesNotVisible" => {
                Some(Self::ApproximateNumberOfMessagesNotVisible)
            }
            "ContentBasedDeduplication" => Some(Self::ContentBasedDeduplication),
            "CreatedTimestamp" => Some(Self::CreatedTimestamp),
            "DeduplicationScope" => Some(Self::DeduplicationScope),
            "DelaySeconds" => Some(Self::DelaySeconds),
            "FifoThroughputLimit" => Some(Self::FifoThroughputLimit),
            "FifoQueue" => Some(Self::FifoQueue),
            "KmsDataKeyReusePeriodSeconds" => Some(Self::KmsDataKeyReusePeriodSeconds),
            "KmsMasterKeyId" => Some(Self::KmsMasterKeyId),
            "LastModifiedTimestamp" => Some(Self::LastModifiedTimestamp),
            "MaximumMessageSize" => Some(Self::MaximumMessageSize),
            "MessageRetentionPeriod" => Some(Self::MessageRetentionPeriod),
            "Policy" => Some(Self::Policy),
            "QueueArn" => Some(Self::QueueArn),
            "ReceiveMessageWaitTimeSeconds" => Some(Self::ReceiveMessageWaitTimeSeconds),
            "RedriveAllowPolicy" => Some(Self::RedriveAllowPolicy),
            "RedrivePolicy" => Some(Self::RedrivePolicy),
            "SqsManagedSseEnabled" => Some(Self::SqsManagedSseEnabled),
            "VisibilityTimeout" => Some(Self::VisibilityTimeout),
            _ => None,
        }
    }

    /// Return the string representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::All => "All",
            Self::ApproximateNumberOfMessages => "ApproximateNumberOfMessages",
            Self::ApproximateNumberOfMessagesDelayed => "ApproximateNumberOfMessagesDelayed",
            Self::ApproximateNumberOfMessagesNotVisible => "ApproximateNumberOfMessagesNotVisible",
            Self::ContentBasedDeduplication => "ContentBasedDeduplication",
            Self::CreatedTimestamp => "CreatedTimestamp",
            Self::DeduplicationScope => "DeduplicationScope",
            Self::DelaySeconds => "DelaySeconds",
            Self::FifoThroughputLimit => "FifoThroughputLimit",
            Self::FifoQueue => "FifoQueue",
            Self::KmsDataKeyReusePeriodSeconds => "KmsDataKeyReusePeriodSeconds",
            Self::KmsMasterKeyId => "KmsMasterKeyId",
            Self::LastModifiedTimestamp => "LastModifiedTimestamp",
            Self::MaximumMessageSize => "MaximumMessageSize",
            Self::MessageRetentionPeriod => "MessageRetentionPeriod",
            Self::Policy => "Policy",
            Self::QueueArn => "QueueArn",
            Self::ReceiveMessageWaitTimeSeconds => "ReceiveMessageWaitTimeSeconds",
            Self::RedriveAllowPolicy => "RedriveAllowPolicy",
            Self::RedrivePolicy => "RedrivePolicy",
            Self::SqsManagedSseEnabled => "SqsManagedSseEnabled",
            Self::VisibilityTimeout => "VisibilityTimeout",
        }
    }
}

/// System attribute names that can be requested when receiving messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MessageSystemAttributeName {
    /// All system attributes.
    All,
    /// Sender ID (account ID).
    SenderId,
    /// Timestamp when the message was sent.
    SentTimestamp,
    /// Approximate number of times the message has been received.
    ApproximateReceiveCount,
    /// Timestamp of the first receive.
    ApproximateFirstReceiveTimestamp,
    /// Sequence number (FIFO only).
    SequenceNumber,
    /// Message deduplication ID (FIFO only).
    MessageDeduplicationId,
    /// Message group ID (FIFO only).
    MessageGroupId,
    /// Source ARN if moved from DLQ.
    DeadLetterQueueSourceArn,
    /// The trace header for X-Ray.
    #[serde(rename = "AWSTraceHeader")]
    AwsTraceHeader,
}

impl MessageSystemAttributeName {
    /// Parse from a string.
    #[must_use]
    pub fn from_str_name(s: &str) -> Option<Self> {
        match s {
            "All" => Some(Self::All),
            "SenderId" => Some(Self::SenderId),
            "SentTimestamp" => Some(Self::SentTimestamp),
            "ApproximateReceiveCount" => Some(Self::ApproximateReceiveCount),
            "ApproximateFirstReceiveTimestamp" => Some(Self::ApproximateFirstReceiveTimestamp),
            "SequenceNumber" => Some(Self::SequenceNumber),
            "MessageDeduplicationId" => Some(Self::MessageDeduplicationId),
            "MessageGroupId" => Some(Self::MessageGroupId),
            "DeadLetterQueueSourceArn" => Some(Self::DeadLetterQueueSourceArn),
            "AWSTraceHeader" => Some(Self::AwsTraceHeader),
            _ => None,
        }
    }

    /// Return the string representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::All => "All",
            Self::SenderId => "SenderId",
            Self::SentTimestamp => "SentTimestamp",
            Self::ApproximateReceiveCount => "ApproximateReceiveCount",
            Self::ApproximateFirstReceiveTimestamp => "ApproximateFirstReceiveTimestamp",
            Self::SequenceNumber => "SequenceNumber",
            Self::MessageDeduplicationId => "MessageDeduplicationId",
            Self::MessageGroupId => "MessageGroupId",
            Self::DeadLetterQueueSourceArn => "DeadLetterQueueSourceArn",
            Self::AwsTraceHeader => "AWSTraceHeader",
        }
    }
}

/// A received message from a queue.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Message {
    /// The message ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    /// The receipt handle for deleting/changing visibility.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt_handle: Option<String>,
    /// The message body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// MD5 digest of the message body.
    #[serde(rename = "MD5OfBody")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub md5_of_body: Option<String>,
    /// MD5 digest of the message attributes.
    #[serde(rename = "MD5OfMessageAttributes")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub md5_of_message_attributes: Option<String>,
    /// User-defined message attributes.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub message_attributes: HashMap<String, MessageAttributeValue>,
    /// System attributes.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub attributes: HashMap<String, String>,
}

/// A single entry in a `SendMessageBatch` request.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SendMessageBatchRequestEntry {
    /// Caller-chosen ID for this entry (unique within the batch).
    pub id: String,
    /// The message body.
    pub message_body: String,
    /// Per-message delay in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delay_seconds: Option<i32>,
    /// Message deduplication ID (FIFO only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_deduplication_id: Option<String>,
    /// Message group ID (FIFO only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_group_id: Option<String>,
    /// User-defined message attributes.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub message_attributes: HashMap<String, MessageAttributeValue>,
    /// System message attributes.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub message_system_attributes: HashMap<String, MessageSystemAttributeValue>,
}

/// A successful result for one entry in a batch send.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SendMessageBatchResultEntry {
    /// The caller-chosen ID for this entry.
    pub id: String,
    /// The message ID assigned by SQS.
    pub message_id: String,
    /// MD5 of the message body.
    #[serde(rename = "MD5OfMessageBody")]
    pub md5_of_message_body: String,
    /// MD5 of message attributes.
    #[serde(rename = "MD5OfMessageAttributes")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub md5_of_message_attributes: Option<String>,
    /// Sequence number (FIFO only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<String>,
}

/// A failed result for one entry in a batch operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct BatchResultErrorEntry {
    /// The caller-chosen ID for this entry.
    pub id: String,
    /// Whether the error is a sender fault.
    pub sender_fault: bool,
    /// The error code.
    pub code: String,
    /// The error message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// A single entry in a `DeleteMessageBatch` request.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteMessageBatchRequestEntry {
    /// Caller-chosen ID for this entry.
    pub id: String,
    /// Receipt handle of the message to delete.
    pub receipt_handle: String,
}

/// A successful result for one entry in a batch delete.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteMessageBatchResultEntry {
    /// The caller-chosen ID for this entry.
    pub id: String,
}

/// A single entry in a `ChangeMessageVisibilityBatch` request.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ChangeMessageVisibilityBatchRequestEntry {
    /// Caller-chosen ID for this entry.
    pub id: String,
    /// Receipt handle of the message.
    pub receipt_handle: String,
    /// New visibility timeout in seconds.
    pub visibility_timeout: i32,
}

/// A successful result for one entry in a batch visibility change.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ChangeMessageVisibilityBatchResultEntry {
    /// The caller-chosen ID for this entry.
    pub id: String,
}

/// Redrive policy for dead-letter queue configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedrivePolicy {
    /// The ARN of the dead-letter queue.
    pub dead_letter_target_arn: String,
    /// Maximum number of receives before sending to DLQ.
    pub max_receive_count: i32,
}

/// Redrive allow policy controlling which source queues can use this queue as DLQ.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedriveAllowPolicy {
    /// Permission type: `allowAll`, `denyAll`, or `byQueue`.
    pub redrive_permission: String,
    /// Source queue ARNs (when permission is `byQueue`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_queue_arns: Vec<String>,
}

/// Status of a message move task.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListMessageMoveTasksResultEntry {
    /// The task handle (identifier).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_handle: Option<String>,
    /// The source ARN (DLQ).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_arn: Option<String>,
    /// The destination ARN.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_arn: Option<String>,
    /// The max messages per second.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_number_of_messages_per_second: Option<i32>,
    /// Approximate number of messages moved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approximate_number_of_messages_moved: Option<i64>,
    /// Approximate number of messages to move.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approximate_number_of_messages_to_move: Option<i64>,
    /// Failure reason if the task failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    /// Task status: RUNNING, COMPLETED, CANCELLING, CANCELLED, FAILED.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Task start timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_timestamp: Option<i64>,
}
