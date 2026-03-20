//! Auto-generated from AWS DynamoDB Streams Smithy model. DO NOT EDIT.

use serde::{Deserialize, Serialize};

use crate::types::{ShardFilter, ShardIteratorType};

/// DynamoDB Streams DescribeStreamInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeStreamInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclusive_start_shard_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shard_filter: Option<ShardFilter>,
    pub stream_arn: String,
}

/// DynamoDB Streams GetRecordsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetRecordsInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
    pub shard_iterator: String,
}

/// DynamoDB Streams GetShardIteratorInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetShardIteratorInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<String>,
    pub shard_id: String,
    pub shard_iterator_type: ShardIteratorType,
    pub stream_arn: String,
}

/// DynamoDB Streams ListStreamsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListStreamsInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclusive_start_stream_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_name: Option<String>,
}
