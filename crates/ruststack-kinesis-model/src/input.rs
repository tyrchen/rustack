//! Auto-generated from AWS Kinesis Smithy model. DO NOT EDIT.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::types::{
    EncryptionType, PutRecordsRequestEntry, ScalingType, ShardFilter, ShardIteratorType,
    StartingPosition, StreamModeDetails,
};

/// Kinesis AddTagsToStreamInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AddTagsToStreamInput {
    #[serde(rename = "StreamARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_name: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
}

/// Kinesis CreateStreamInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CreateStreamInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_record_size_in_ki_b: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shard_count: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_mode_details: Option<StreamModeDetails>,
    pub stream_name: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warm_throughput_mi_bps: Option<i32>,
}

/// Kinesis DecreaseStreamRetentionPeriodInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DecreaseStreamRetentionPeriodInput {
    pub retention_period_hours: i32,
    #[serde(rename = "StreamARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_name: Option<String>,
}

/// Kinesis DeleteResourcePolicyInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteResourcePolicyInput {
    #[serde(rename = "ResourceARN")]
    pub resource_arn: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
}

/// Kinesis DeleteStreamInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteStreamInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enforce_consumer_deletion: Option<bool>,
    #[serde(rename = "StreamARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_name: Option<String>,
}

/// Kinesis DeregisterStreamConsumerInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeregisterStreamConsumerInput {
    #[serde(rename = "ConsumerARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consumer_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consumer_name: Option<String>,
    #[serde(rename = "StreamARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
}

/// Kinesis DescribeLimitsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeLimitsInput {}

/// Kinesis DescribeStreamConsumerInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeStreamConsumerInput {
    #[serde(rename = "ConsumerARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consumer_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consumer_name: Option<String>,
    #[serde(rename = "StreamARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
}

/// Kinesis DescribeStreamInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeStreamInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclusive_start_shard_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
    #[serde(rename = "StreamARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_name: Option<String>,
}

/// Kinesis DescribeStreamSummaryInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeStreamSummaryInput {
    #[serde(rename = "StreamARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_name: Option<String>,
}

/// Kinesis GetRecordsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetRecordsInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
    pub shard_iterator: String,
    #[serde(rename = "StreamARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
}

/// Kinesis GetResourcePolicyInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetResourcePolicyInput {
    #[serde(rename = "ResourceARN")]
    pub resource_arn: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
}

/// Kinesis GetShardIteratorInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetShardIteratorInput {
    pub shard_id: String,
    pub shard_iterator_type: ShardIteratorType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starting_sequence_number: Option<String>,
    #[serde(rename = "StreamARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, with = "crate::epoch_seconds::option")]
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

/// Kinesis IncreaseStreamRetentionPeriodInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct IncreaseStreamRetentionPeriodInput {
    pub retention_period_hours: i32,
    #[serde(rename = "StreamARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_name: Option<String>,
}

/// Kinesis ListShardsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListShardsInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclusive_start_shard_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shard_filter: Option<ShardFilter>,
    #[serde(rename = "StreamARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, with = "crate::epoch_seconds::option")]
    pub stream_creation_timestamp: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_name: Option<String>,
}

/// Kinesis ListStreamConsumersInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListStreamConsumersInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    #[serde(rename = "StreamARN")]
    pub stream_arn: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, with = "crate::epoch_seconds::option")]
    pub stream_creation_timestamp: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
}

/// Kinesis ListStreamsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListStreamsInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclusive_start_stream_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// Kinesis ListTagsForStreamInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListTagsForStreamInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclusive_start_tag_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
    #[serde(rename = "StreamARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_name: Option<String>,
}

/// Kinesis MergeShardsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MergeShardsInput {
    pub adjacent_shard_to_merge: String,
    pub shard_to_merge: String,
    #[serde(rename = "StreamARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_name: Option<String>,
}

/// Kinesis PutRecordInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutRecordInput {
    #[serde(with = "crate::blob")]
    pub data: bytes::Bytes,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explicit_hash_key: Option<String>,
    pub partition_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number_for_ordering: Option<String>,
    #[serde(rename = "StreamARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_name: Option<String>,
}

/// Kinesis PutRecordsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutRecordsInput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub records: Vec<PutRecordsRequestEntry>,
    #[serde(rename = "StreamARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_name: Option<String>,
}

/// Kinesis PutResourcePolicyInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutResourcePolicyInput {
    pub policy: String,
    #[serde(rename = "ResourceARN")]
    pub resource_arn: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
}

/// Kinesis RegisterStreamConsumerInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RegisterStreamConsumerInput {
    pub consumer_name: String,
    #[serde(rename = "StreamARN")]
    pub stream_arn: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
}

/// Kinesis RemoveTagsFromStreamInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RemoveTagsFromStreamInput {
    #[serde(rename = "StreamARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tag_keys: Vec<String>,
}

/// Kinesis SplitShardInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SplitShardInput {
    pub new_starting_hash_key: String,
    pub shard_to_split: String,
    #[serde(rename = "StreamARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_name: Option<String>,
}

/// Kinesis StartStreamEncryptionInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct StartStreamEncryptionInput {
    pub encryption_type: EncryptionType,
    pub key_id: String,
    #[serde(rename = "StreamARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_name: Option<String>,
}

/// Kinesis StopStreamEncryptionInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct StopStreamEncryptionInput {
    pub encryption_type: EncryptionType,
    pub key_id: String,
    #[serde(rename = "StreamARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_name: Option<String>,
}

/// Kinesis SubscribeToShardInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SubscribeToShardInput {
    #[serde(rename = "ConsumerARN")]
    pub consumer_arn: String,
    pub shard_id: String,
    pub starting_position: StartingPosition,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
}

/// Kinesis UpdateShardCountInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UpdateShardCountInput {
    pub scaling_type: ScalingType,
    #[serde(rename = "StreamARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_name: Option<String>,
    pub target_shard_count: i32,
}
