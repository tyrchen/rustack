//! Auto-generated from AWS Kinesis Smithy model. DO NOT EDIT.

use serde::{Deserialize, Serialize};

/// Kinesis ConsumerStatus enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ConsumerStatus {
    /// Default variant.
    #[default]
    #[serde(rename = "ACTIVE")]
    Active,
    #[serde(rename = "CREATING")]
    Creating,
    #[serde(rename = "DELETING")]
    Deleting,
}

impl ConsumerStatus {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Creating => "CREATING",
            Self::Deleting => "DELETING",
        }
    }
}

impl std::fmt::Display for ConsumerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ConsumerStatus {
    fn from(s: &str) -> Self {
        match s {
            "ACTIVE" => Self::Active,
            "CREATING" => Self::Creating,
            "DELETING" => Self::Deleting,
            _ => Self::default(),
        }
    }
}

/// Kinesis EncryptionType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum EncryptionType {
    /// Default variant.
    #[default]
    #[serde(rename = "KMS")]
    Kms,
    #[serde(rename = "NONE")]
    None,
}

impl EncryptionType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Kms => "KMS",
            Self::None => "NONE",
        }
    }
}

impl std::fmt::Display for EncryptionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for EncryptionType {
    fn from(s: &str) -> Self {
        match s {
            "KMS" => Self::Kms,
            "NONE" => Self::None,
            _ => Self::default(),
        }
    }
}

/// Kinesis MetricsName enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum MetricsName {
    /// Default variant.
    #[default]
    #[serde(rename = "ALL")]
    All,
    IncomingBytes,
    IncomingRecords,
    IteratorAgeMilliseconds,
    OutgoingBytes,
    OutgoingRecords,
    ReadProvisionedThroughputExceeded,
    WriteProvisionedThroughputExceeded,
}

impl MetricsName {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::All => "ALL",
            Self::IncomingBytes => "IncomingBytes",
            Self::IncomingRecords => "IncomingRecords",
            Self::IteratorAgeMilliseconds => "IteratorAgeMilliseconds",
            Self::OutgoingBytes => "OutgoingBytes",
            Self::OutgoingRecords => "OutgoingRecords",
            Self::ReadProvisionedThroughputExceeded => "ReadProvisionedThroughputExceeded",
            Self::WriteProvisionedThroughputExceeded => "WriteProvisionedThroughputExceeded",
        }
    }
}

impl std::fmt::Display for MetricsName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for MetricsName {
    fn from(s: &str) -> Self {
        match s {
            "ALL" => Self::All,
            "IncomingBytes" => Self::IncomingBytes,
            "IncomingRecords" => Self::IncomingRecords,
            "IteratorAgeMilliseconds" => Self::IteratorAgeMilliseconds,
            "OutgoingBytes" => Self::OutgoingBytes,
            "OutgoingRecords" => Self::OutgoingRecords,
            "ReadProvisionedThroughputExceeded" => Self::ReadProvisionedThroughputExceeded,
            "WriteProvisionedThroughputExceeded" => Self::WriteProvisionedThroughputExceeded,
            _ => Self::default(),
        }
    }
}

/// Kinesis ScalingType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ScalingType {
    /// Default variant.
    #[default]
    #[serde(rename = "UNIFORM_SCALING")]
    UniformScaling,
}

impl ScalingType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UniformScaling => "UNIFORM_SCALING",
        }
    }
}

impl std::fmt::Display for ScalingType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ScalingType {
    fn from(s: &str) -> Self {
        match s {
            "UNIFORM_SCALING" => Self::UniformScaling,
            _ => Self::default(),
        }
    }
}

/// Kinesis ShardFilterType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ShardFilterType {
    /// Default variant.
    #[default]
    #[serde(rename = "AFTER_SHARD_ID")]
    AfterShardId,
    #[serde(rename = "AT_LATEST")]
    AtLatest,
    #[serde(rename = "AT_TIMESTAMP")]
    AtTimestamp,
    #[serde(rename = "AT_TRIM_HORIZON")]
    AtTrimHorizon,
    #[serde(rename = "FROM_TIMESTAMP")]
    FromTimestamp,
    #[serde(rename = "FROM_TRIM_HORIZON")]
    FromTrimHorizon,
}

impl ShardFilterType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AfterShardId => "AFTER_SHARD_ID",
            Self::AtLatest => "AT_LATEST",
            Self::AtTimestamp => "AT_TIMESTAMP",
            Self::AtTrimHorizon => "AT_TRIM_HORIZON",
            Self::FromTimestamp => "FROM_TIMESTAMP",
            Self::FromTrimHorizon => "FROM_TRIM_HORIZON",
        }
    }
}

impl std::fmt::Display for ShardFilterType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ShardFilterType {
    fn from(s: &str) -> Self {
        match s {
            "AFTER_SHARD_ID" => Self::AfterShardId,
            "AT_LATEST" => Self::AtLatest,
            "AT_TIMESTAMP" => Self::AtTimestamp,
            "AT_TRIM_HORIZON" => Self::AtTrimHorizon,
            "FROM_TIMESTAMP" => Self::FromTimestamp,
            "FROM_TRIM_HORIZON" => Self::FromTrimHorizon,
            _ => Self::default(),
        }
    }
}

/// Kinesis ShardIteratorType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ShardIteratorType {
    /// Default variant.
    #[default]
    #[serde(rename = "AFTER_SEQUENCE_NUMBER")]
    AfterSequenceNumber,
    #[serde(rename = "AT_SEQUENCE_NUMBER")]
    AtSequenceNumber,
    #[serde(rename = "AT_TIMESTAMP")]
    AtTimestamp,
    #[serde(rename = "LATEST")]
    Latest,
    #[serde(rename = "TRIM_HORIZON")]
    TrimHorizon,
}

impl ShardIteratorType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AfterSequenceNumber => "AFTER_SEQUENCE_NUMBER",
            Self::AtSequenceNumber => "AT_SEQUENCE_NUMBER",
            Self::AtTimestamp => "AT_TIMESTAMP",
            Self::Latest => "LATEST",
            Self::TrimHorizon => "TRIM_HORIZON",
        }
    }
}

impl std::fmt::Display for ShardIteratorType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ShardIteratorType {
    fn from(s: &str) -> Self {
        match s {
            "AFTER_SEQUENCE_NUMBER" => Self::AfterSequenceNumber,
            "AT_SEQUENCE_NUMBER" => Self::AtSequenceNumber,
            "AT_TIMESTAMP" => Self::AtTimestamp,
            "LATEST" => Self::Latest,
            "TRIM_HORIZON" => Self::TrimHorizon,
            _ => Self::default(),
        }
    }
}

/// Kinesis StreamMode enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum StreamMode {
    /// Default variant.
    #[default]
    #[serde(rename = "ON_DEMAND")]
    OnDemand,
    #[serde(rename = "PROVISIONED")]
    Provisioned,
}

impl StreamMode {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OnDemand => "ON_DEMAND",
            Self::Provisioned => "PROVISIONED",
        }
    }
}

impl std::fmt::Display for StreamMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for StreamMode {
    fn from(s: &str) -> Self {
        match s {
            "ON_DEMAND" => Self::OnDemand,
            "PROVISIONED" => Self::Provisioned,
            _ => Self::default(),
        }
    }
}

/// Kinesis StreamStatus enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum StreamStatus {
    /// Default variant.
    #[default]
    #[serde(rename = "ACTIVE")]
    Active,
    #[serde(rename = "CREATING")]
    Creating,
    #[serde(rename = "DELETING")]
    Deleting,
    #[serde(rename = "UPDATING")]
    Updating,
}

impl StreamStatus {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Creating => "CREATING",
            Self::Deleting => "DELETING",
            Self::Updating => "UPDATING",
        }
    }
}

impl std::fmt::Display for StreamStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for StreamStatus {
    fn from(s: &str) -> Self {
        match s {
            "ACTIVE" => Self::Active,
            "CREATING" => Self::Creating,
            "DELETING" => Self::Deleting,
            "UPDATING" => Self::Updating,
            _ => Self::default(),
        }
    }
}

/// Kinesis ChildShard.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ChildShard {
    pub hash_key_range: HashKeyRange,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parent_shards: Vec<String>,
    pub shard_id: String,
}

/// Kinesis Consumer.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Consumer {
    #[serde(rename = "ConsumerARN")]
    pub consumer_arn: String,
    #[serde(with = "crate::epoch_seconds")]
    pub consumer_creation_timestamp: chrono::DateTime<chrono::Utc>,
    pub consumer_name: String,
    pub consumer_status: ConsumerStatus,
}

/// Kinesis ConsumerDescription.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ConsumerDescription {
    #[serde(rename = "ConsumerARN")]
    pub consumer_arn: String,
    #[serde(with = "crate::epoch_seconds")]
    pub consumer_creation_timestamp: chrono::DateTime<chrono::Utc>,
    pub consumer_name: String,
    pub consumer_status: ConsumerStatus,
    #[serde(rename = "StreamARN")]
    pub stream_arn: String,
}

/// Kinesis EnhancedMetrics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EnhancedMetrics {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shard_level_metrics: Vec<MetricsName>,
}

/// Kinesis HashKeyRange.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct HashKeyRange {
    pub ending_hash_key: String,
    pub starting_hash_key: String,
}

/// Kinesis PutRecordsRequestEntry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutRecordsRequestEntry {
    #[serde(with = "crate::blob")]
    pub data: bytes::Bytes,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explicit_hash_key: Option<String>,
    pub partition_key: String,
}

/// Kinesis PutRecordsResultEntry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutRecordsResultEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shard_id: Option<String>,
}

/// Kinesis Record.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Record {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, with = "crate::epoch_seconds::option")]
    pub approximate_arrival_timestamp: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(with = "crate::blob")]
    pub data: bytes::Bytes,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryption_type: Option<EncryptionType>,
    pub partition_key: String,
    pub sequence_number: String,
}

/// Kinesis SequenceNumberRange.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SequenceNumberRange {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ending_sequence_number: Option<String>,
    pub starting_sequence_number: String,
}

/// Kinesis Shard.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Shard {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjacent_parent_shard_id: Option<String>,
    pub hash_key_range: HashKeyRange,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_shard_id: Option<String>,
    pub sequence_number_range: SequenceNumberRange,
    pub shard_id: String,
}

/// Kinesis ShardFilter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ShardFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shard_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, with = "crate::epoch_seconds::option")]
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
    pub r#type: ShardFilterType,
}

/// Kinesis StartingPosition.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct StartingPosition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, with = "crate::epoch_seconds::option")]
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
    pub r#type: ShardIteratorType,
}

/// Kinesis StreamDescription.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct StreamDescription {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryption_type: Option<EncryptionType>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enhanced_monitoring: Vec<EnhancedMetrics>,
    pub has_more_shards: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    pub retention_period_hours: i32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shards: Vec<Shard>,
    #[serde(rename = "StreamARN")]
    pub stream_arn: String,
    #[serde(with = "crate::epoch_seconds")]
    pub stream_creation_timestamp: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_mode_details: Option<StreamModeDetails>,
    pub stream_name: String,
    pub stream_status: StreamStatus,
}

/// Kinesis StreamDescriptionSummary.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct StreamDescriptionSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consumer_count: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryption_type: Option<EncryptionType>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enhanced_monitoring: Vec<EnhancedMetrics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_record_size_in_ki_b: Option<i32>,
    pub open_shard_count: i32,
    pub retention_period_hours: i32,
    #[serde(rename = "StreamARN")]
    pub stream_arn: String,
    #[serde(with = "crate::epoch_seconds")]
    pub stream_creation_timestamp: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_mode_details: Option<StreamModeDetails>,
    pub stream_name: String,
    pub stream_status: StreamStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warm_throughput: Option<WarmThroughputObject>,
}

/// Kinesis StreamModeDetails.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct StreamModeDetails {
    pub stream_mode: StreamMode,
}

/// Kinesis StreamSummary.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct StreamSummary {
    #[serde(rename = "StreamARN")]
    pub stream_arn: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, with = "crate::epoch_seconds::option")]
    pub stream_creation_timestamp: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_mode_details: Option<StreamModeDetails>,
    pub stream_name: String,
    pub stream_status: StreamStatus,
}

/// Kinesis SubscribeToShardEvent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SubscribeToShardEvent {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub child_shards: Vec<ChildShard>,
    pub continuation_sequence_number: String,
    pub millis_behind_latest: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub records: Vec<Record>,
}

/// Kinesis SubscribeToShardEventStream.
///
/// This is a union type used by the SubscribeToShard HTTP/2 event stream.
/// Since SubscribeToShard is deferred, error variant types are represented
/// as opaque JSON values.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SubscribeToShardEventStream {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscribe_to_shard_event: Option<SubscribeToShardEvent>,
}

/// Kinesis Tag.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Tag {
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

/// Kinesis WarmThroughputObject.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct WarmThroughputObject {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_mi_bps: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_mi_bps: Option<i32>,
}
