//! Auto-generated from AWS DynamoDB Streams Smithy model. DO NOT EDIT.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// DynamoDB Streams KeyType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum KeyType {
    /// Default variant.
    #[default]
    #[serde(rename = "HASH")]
    Hash,
    #[serde(rename = "RANGE")]
    Range,
}

impl KeyType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Hash => "HASH",
            Self::Range => "RANGE",
        }
    }
}

impl std::fmt::Display for KeyType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for KeyType {
    fn from(s: &str) -> Self {
        match s {
            "HASH" => Self::Hash,
            "RANGE" => Self::Range,
            _ => Self::default(),
        }
    }
}

/// DynamoDB Streams OperationType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum OperationType {
    /// Default variant.
    #[default]
    #[serde(rename = "INSERT")]
    Insert,
    #[serde(rename = "MODIFY")]
    Modify,
    #[serde(rename = "REMOVE")]
    Remove,
}

impl OperationType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Insert => "INSERT",
            Self::Modify => "MODIFY",
            Self::Remove => "REMOVE",
        }
    }
}

impl std::fmt::Display for OperationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for OperationType {
    fn from(s: &str) -> Self {
        match s {
            "INSERT" => Self::Insert,
            "MODIFY" => Self::Modify,
            "REMOVE" => Self::Remove,
            _ => Self::default(),
        }
    }
}

/// DynamoDB Streams ShardFilterType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ShardFilterType {
    /// Default variant.
    #[default]
    #[serde(rename = "CHILD_SHARDS")]
    ChildShards,
}

impl ShardFilterType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ChildShards => "CHILD_SHARDS",
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
            "CHILD_SHARDS" => Self::ChildShards,
            _ => Self::default(),
        }
    }
}

/// DynamoDB Streams ShardIteratorType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ShardIteratorType {
    /// Default variant.
    #[default]
    #[serde(rename = "AFTER_SEQUENCE_NUMBER")]
    AfterSequenceNumber,
    #[serde(rename = "AT_SEQUENCE_NUMBER")]
    AtSequenceNumber,
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
            "LATEST" => Self::Latest,
            "TRIM_HORIZON" => Self::TrimHorizon,
            _ => Self::default(),
        }
    }
}

/// DynamoDB Streams StreamStatus enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum StreamStatus {
    /// Default variant.
    #[default]
    #[serde(rename = "DISABLED")]
    Disabled,
    #[serde(rename = "DISABLING")]
    Disabling,
    #[serde(rename = "ENABLED")]
    Enabled,
    #[serde(rename = "ENABLING")]
    Enabling,
}

impl StreamStatus {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Disabled => "DISABLED",
            Self::Disabling => "DISABLING",
            Self::Enabled => "ENABLED",
            Self::Enabling => "ENABLING",
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
            "DISABLED" => Self::Disabled,
            "DISABLING" => Self::Disabling,
            "ENABLED" => Self::Enabled,
            "ENABLING" => Self::Enabling,
            _ => Self::default(),
        }
    }
}

/// DynamoDB Streams StreamViewType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum StreamViewType {
    /// Default variant.
    #[default]
    #[serde(rename = "KEYS_ONLY")]
    KeysOnly,
    #[serde(rename = "NEW_AND_OLD_IMAGES")]
    NewAndOldImages,
    #[serde(rename = "NEW_IMAGE")]
    NewImage,
    #[serde(rename = "OLD_IMAGE")]
    OldImage,
}

impl StreamViewType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::KeysOnly => "KEYS_ONLY",
            Self::NewAndOldImages => "NEW_AND_OLD_IMAGES",
            Self::NewImage => "NEW_IMAGE",
            Self::OldImage => "OLD_IMAGE",
        }
    }
}

impl std::fmt::Display for StreamViewType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for StreamViewType {
    fn from(s: &str) -> Self {
        match s {
            "KEYS_ONLY" => Self::KeysOnly,
            "NEW_AND_OLD_IMAGES" => Self::NewAndOldImages,
            "NEW_IMAGE" => Self::NewImage,
            "OLD_IMAGE" => Self::OldImage,
            _ => Self::default(),
        }
    }
}

/// DynamoDB Streams AttributeValue.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AttributeValue {
    #[serde(rename = "NS")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ns: Option<Vec<String>>,
    #[serde(rename = "NULL")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub null: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub b: Option<bytes::Bytes>,
    #[serde(rename = "BOOL")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bool: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub l: Option<Vec<AttributeValue>>,
    #[serde(rename = "BS")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bs: Option<Vec<bytes::Bytes>>,
    #[serde(rename = "SS")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ss: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub s: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub m: Option<HashMap<String, AttributeValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<String>,
}

/// DynamoDB Streams Identity.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Identity {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub principal_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
}

/// DynamoDB Streams KeySchemaElement.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct KeySchemaElement {
    pub attribute_name: String,
    pub key_type: KeyType,
}

/// DynamoDB Streams Record.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Record {
    #[serde(rename = "awsRegion")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aws_region: Option<String>,
    #[serde(rename = "dynamodb")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynamodb: Option<StreamRecord>,
    #[serde(rename = "eventID")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    #[serde(rename = "eventName")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_name: Option<OperationType>,
    #[serde(rename = "eventSource")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_source: Option<String>,
    #[serde(rename = "eventVersion")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_version: Option<String>,
    #[serde(rename = "userIdentity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_identity: Option<Identity>,
}

/// DynamoDB Streams SequenceNumberRange.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SequenceNumberRange {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ending_sequence_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starting_sequence_number: Option<String>,
}

/// DynamoDB Streams Shard.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Shard {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_shard_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number_range: Option<SequenceNumberRange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shard_id: Option<String>,
}

/// DynamoDB Streams ShardFilter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ShardFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shard_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<ShardFilterType>,
}

/// DynamoDB Streams Stream.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Stream {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_name: Option<String>,
}

/// DynamoDB Streams StreamDescription.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct StreamDescription {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creation_request_date_time: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub key_schema: Vec<KeySchemaElement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_evaluated_shard_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shards: Vec<Shard>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_status: Option<StreamStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_view_type: Option<StreamViewType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_name: Option<String>,
}

/// DynamoDB Streams StreamRecord.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct StreamRecord {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approximate_creation_date_time: Option<f64>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub keys: HashMap<String, AttributeValue>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub new_image: HashMap<String, AttributeValue>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub old_image: HashMap<String, AttributeValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_view_type: Option<StreamViewType>,
}
