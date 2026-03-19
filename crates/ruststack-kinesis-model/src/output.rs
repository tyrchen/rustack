//! Auto-generated from AWS Kinesis Smithy model. DO NOT EDIT.

use serde::{Deserialize, Serialize};

use crate::types::{
    ChildShard, Consumer, ConsumerDescription, EncryptionType, PutRecordsResultEntry, Record,
    Shard, StreamDescription, StreamDescriptionSummary, StreamSummary, SubscribeToShardEventStream,
    Tag,
};

/// Kinesis DescribeLimitsOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeLimitsOutput {
    pub on_demand_stream_count: i32,
    pub on_demand_stream_count_limit: i32,
    pub open_shard_count: i32,
    pub shard_limit: i32,
}

/// Kinesis DescribeStreamConsumerOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeStreamConsumerOutput {
    pub consumer_description: ConsumerDescription,
}

/// Kinesis DescribeStreamOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeStreamOutput {
    pub stream_description: StreamDescription,
}

/// Kinesis DescribeStreamSummaryOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeStreamSummaryOutput {
    pub stream_description_summary: StreamDescriptionSummary,
}

/// Kinesis GetRecordsOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetRecordsOutput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub child_shards: Vec<ChildShard>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub millis_behind_latest: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_shard_iterator: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub records: Vec<Record>,
}

/// Kinesis GetResourcePolicyOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetResourcePolicyOutput {
    pub policy: String,
}

/// Kinesis GetShardIteratorOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetShardIteratorOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shard_iterator: Option<String>,
}

/// Kinesis ListShardsOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListShardsOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shards: Vec<Shard>,
}

/// Kinesis ListStreamConsumersOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListStreamConsumersOutput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub consumers: Vec<Consumer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// Kinesis ListStreamsOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListStreamsOutput {
    pub has_more_streams: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stream_names: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stream_summaries: Vec<StreamSummary>,
}

/// Kinesis ListTagsForStreamOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListTagsForStreamOutput {
    pub has_more_tags: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Tag>,
}

/// Kinesis PutRecordOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutRecordOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryption_type: Option<EncryptionType>,
    pub sequence_number: String,
    pub shard_id: String,
}

/// Kinesis PutRecordsOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutRecordsOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryption_type: Option<EncryptionType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed_record_count: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub records: Vec<PutRecordsResultEntry>,
}

/// Kinesis RegisterStreamConsumerOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RegisterStreamConsumerOutput {
    pub consumer: Consumer,
}

/// Kinesis SubscribeToShardOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SubscribeToShardOutput {
    pub event_stream: SubscribeToShardEventStream,
}

/// Kinesis UpdateShardCountOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UpdateShardCountOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_shard_count: Option<i32>,
    #[serde(rename = "StreamARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_shard_count: Option<i32>,
}
