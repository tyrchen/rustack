//! Auto-generated from AWS DynamoDB Streams Smithy model. DO NOT EDIT.

use serde::{Deserialize, Serialize};

use crate::types::{Record, Stream, StreamDescription};

/// DynamoDB Streams DescribeStreamOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeStreamOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_description: Option<StreamDescription>,
}

/// DynamoDB Streams GetRecordsOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetRecordsOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_shard_iterator: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub records: Vec<Record>,
}

/// DynamoDB Streams GetShardIteratorOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetShardIteratorOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shard_iterator: Option<String>,
}

/// DynamoDB Streams ListStreamsOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListStreamsOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_evaluated_stream_arn: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub streams: Vec<Stream>,
}
