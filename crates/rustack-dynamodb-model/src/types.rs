//! Shared DynamoDB types for the 12 MVP operations.
//!
//! All types follow the DynamoDB JSON wire format with `PascalCase` field names.
//! Structs use `#[serde(rename_all = "PascalCase")]` to match the DynamoDB API.
//!
//! Enum variants use idiomatic Rust `PascalCase` naming with `#[serde(rename)]`
//! attributes to map to the `SCREAMING_SNAKE_CASE` wire format that DynamoDB uses.
//!
//! # MVP Operations Covered
//!
//! - Table management: `CreateTable`, `DeleteTable`, `DescribeTable`, `ListTables`
//! - Item CRUD: `PutItem`, `GetItem`, `UpdateItem`, `DeleteItem`
//! - Queries: `Query`, `Scan`
//! - Batch: `BatchGetItem`, `BatchWriteItem`

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::attribute_value::AttributeValue;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Key type within a key schema element.
///
/// `Hash` denotes the partition key; `Range` denotes the sort key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KeyType {
    /// Partition key.
    #[serde(rename = "HASH")]
    Hash,
    /// Sort key.
    #[serde(rename = "RANGE")]
    Range,
}

impl KeyType {
    /// Returns the DynamoDB wire-format string representation of this key type.
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

/// Scalar attribute types supported in key schema and attribute definitions.
///
/// DynamoDB only allows `S`, `N`, and `B` for key attributes, but the wire
/// protocol may receive other values which must be rejected with a
/// `ValidationException` rather than a deserialization error.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ScalarAttributeType {
    /// String type.
    S,
    /// Number type.
    N,
    /// Binary type.
    B,
    /// An unknown/invalid attribute type received from the client.
    Unknown(String),
}

impl ScalarAttributeType {
    /// Returns the DynamoDB wire-format string representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::S => "S",
            Self::N => "N",
            Self::B => "B",
            Self::Unknown(s) => s.as_str(),
        }
    }

    /// Returns `true` if this is a valid key attribute type (S, N, or B).
    #[must_use]
    pub fn is_valid_key_type(&self) -> bool {
        matches!(self, Self::S | Self::N | Self::B)
    }
}

impl Serialize for ScalarAttributeType {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ScalarAttributeType {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "S" => Ok(Self::S),
            "N" => Ok(Self::N),
            "B" => Ok(Self::B),
            _ => Ok(Self::Unknown(s)),
        }
    }
}

impl std::fmt::Display for ScalarAttributeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Current status of a DynamoDB table.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TableStatus {
    /// The table is being created.
    #[serde(rename = "CREATING")]
    Creating,
    /// The table is ready for use.
    #[serde(rename = "ACTIVE")]
    Active,
    /// The table is being deleted.
    #[serde(rename = "DELETING")]
    Deleting,
    /// The table is being updated (e.g., GSI changes).
    #[serde(rename = "UPDATING")]
    Updating,
    /// The table is being archived.
    #[serde(rename = "ARCHIVING")]
    Archiving,
    /// The table has been archived.
    #[serde(rename = "ARCHIVED")]
    Archived,
    /// The table is inaccessible due to encryption credentials issues.
    #[serde(rename = "INACCESSIBLE_ENCRYPTION_CREDENTIALS")]
    InaccessibleEncryptionCredentials,
}

impl TableStatus {
    /// Returns the DynamoDB wire-format string representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Creating => "CREATING",
            Self::Active => "ACTIVE",
            Self::Deleting => "DELETING",
            Self::Updating => "UPDATING",
            Self::Archiving => "ARCHIVING",
            Self::Archived => "ARCHIVED",
            Self::InaccessibleEncryptionCredentials => "INACCESSIBLE_ENCRYPTION_CREDENTIALS",
        }
    }
}

impl std::fmt::Display for TableStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Continuous backups status for a DynamoDB table.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ContinuousBackupsStatus {
    /// Continuous backups are enabled.
    #[serde(rename = "ENABLED")]
    Enabled,
    /// Continuous backups are disabled.
    #[serde(rename = "DISABLED")]
    Disabled,
}

/// Point-in-time recovery status for a DynamoDB table.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PointInTimeRecoveryStatus {
    /// Point-in-time recovery is enabled.
    #[serde(rename = "ENABLED")]
    Enabled,
    /// Point-in-time recovery is disabled.
    #[serde(rename = "DISABLED")]
    Disabled,
}

/// Billing mode for a DynamoDB table.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum BillingMode {
    /// Provisioned capacity mode with explicit RCU/WCU settings.
    Provisioned,
    /// On-demand capacity mode (pay per request).
    #[default]
    PayPerRequest,
    /// An unknown billing mode value received from the client.
    Unknown(String),
}

impl BillingMode {
    /// Returns the DynamoDB wire-format string representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Provisioned => "PROVISIONED",
            Self::PayPerRequest => "PAY_PER_REQUEST",
            Self::Unknown(s) => s.as_str(),
        }
    }
}

impl Serialize for BillingMode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for BillingMode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "PROVISIONED" => Ok(Self::Provisioned),
            "PAY_PER_REQUEST" => Ok(Self::PayPerRequest),
            _ => Ok(Self::Unknown(s)),
        }
    }
}

impl std::fmt::Display for BillingMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Projection type for secondary indexes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ProjectionType {
    /// All attributes from the table are projected into the index.
    #[default]
    #[serde(rename = "ALL")]
    All,
    /// Only the index and primary keys are projected.
    #[serde(rename = "KEYS_ONLY")]
    KeysOnly,
    /// Only specified non-key attributes are projected alongside keys.
    #[serde(rename = "INCLUDE")]
    Include,
}

impl ProjectionType {
    /// Returns the DynamoDB wire-format string representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::All => "ALL",
            Self::KeysOnly => "KEYS_ONLY",
            Self::Include => "INCLUDE",
        }
    }
}

impl std::fmt::Display for ProjectionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Stream view type controlling what data is captured in DynamoDB Streams.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StreamViewType {
    /// Only the key attributes of the modified item.
    #[serde(rename = "KEYS_ONLY")]
    KeysOnly,
    /// The entire item as it appears after modification.
    #[serde(rename = "NEW_IMAGE")]
    NewImage,
    /// The entire item as it appeared before modification.
    #[serde(rename = "OLD_IMAGE")]
    OldImage,
    /// Both the new and old item images.
    #[serde(rename = "NEW_AND_OLD_IMAGES")]
    NewAndOldImages,
}

impl StreamViewType {
    /// Returns the DynamoDB wire-format string representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::KeysOnly => "KEYS_ONLY",
            Self::NewImage => "NEW_IMAGE",
            Self::OldImage => "OLD_IMAGE",
            Self::NewAndOldImages => "NEW_AND_OLD_IMAGES",
        }
    }
}

impl std::fmt::Display for StreamViewType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// SSE (Server-Side Encryption) type.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum SseType {
    /// AWS-owned key encryption (default, free).
    #[default]
    #[serde(rename = "AES256")]
    Aes256,
    /// AWS KMS managed key encryption.
    #[serde(rename = "KMS")]
    Kms,
}

impl SseType {
    /// Returns the DynamoDB wire-format string representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Aes256 => "AES256",
            Self::Kms => "KMS",
        }
    }
}

impl std::fmt::Display for SseType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// SSE status.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SseStatus {
    /// SSE is being enabled.
    #[serde(rename = "ENABLING")]
    Enabling,
    /// SSE is active.
    #[serde(rename = "ENABLED")]
    Enabled,
    /// SSE is being disabled.
    #[serde(rename = "DISABLING")]
    Disabling,
    /// SSE is disabled.
    #[serde(rename = "DISABLED")]
    Disabled,
    /// SSE is being updated.
    #[serde(rename = "UPDATING")]
    Updating,
}

impl SseStatus {
    /// Returns the DynamoDB wire-format string representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Enabling => "ENABLING",
            Self::Enabled => "ENABLED",
            Self::Disabling => "DISABLING",
            Self::Disabled => "DISABLED",
            Self::Updating => "UPDATING",
        }
    }
}

impl std::fmt::Display for SseStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Index status for global secondary indexes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IndexStatus {
    /// The index is being created.
    #[serde(rename = "CREATING")]
    Creating,
    /// The index is being updated.
    #[serde(rename = "UPDATING")]
    Updating,
    /// The index is being deleted.
    #[serde(rename = "DELETING")]
    Deleting,
    /// The index is active and ready for use.
    #[serde(rename = "ACTIVE")]
    Active,
}

impl IndexStatus {
    /// Returns the DynamoDB wire-format string representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Creating => "CREATING",
            Self::Updating => "UPDATING",
            Self::Deleting => "DELETING",
            Self::Active => "ACTIVE",
        }
    }
}

impl std::fmt::Display for IndexStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Determines what values are returned by write operations.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ReturnValue {
    /// Nothing is returned.
    #[default]
    #[serde(rename = "NONE")]
    None,
    /// Returns all attributes of the item as they appeared before the operation.
    #[serde(rename = "ALL_OLD")]
    AllOld,
    /// Returns only the updated attributes as they appeared before the operation.
    #[serde(rename = "UPDATED_OLD")]
    UpdatedOld,
    /// Returns all attributes of the item as they appear after the operation.
    #[serde(rename = "ALL_NEW")]
    AllNew,
    /// Returns only the updated attributes as they appear after the operation.
    #[serde(rename = "UPDATED_NEW")]
    UpdatedNew,
}

impl ReturnValue {
    /// Returns the DynamoDB wire-format string representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::AllOld => "ALL_OLD",
            Self::UpdatedOld => "UPDATED_OLD",
            Self::AllNew => "ALL_NEW",
            Self::UpdatedNew => "UPDATED_NEW",
        }
    }
}

impl std::fmt::Display for ReturnValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Controls whether consumed capacity information is returned.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ReturnConsumedCapacity {
    /// Return consumed capacity for the table and any indexes involved.
    #[serde(rename = "INDEXES")]
    Indexes,
    /// Return only the total consumed capacity.
    #[serde(rename = "TOTAL")]
    Total,
    /// Do not return consumed capacity (default).
    #[default]
    #[serde(rename = "NONE")]
    None,
}

impl ReturnConsumedCapacity {
    /// Returns the DynamoDB wire-format string representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Indexes => "INDEXES",
            Self::Total => "TOTAL",
            Self::None => "NONE",
        }
    }

    /// Returns `true` if capacity tracking should be performed.
    #[must_use]
    pub fn should_report(&self) -> bool {
        !matches!(self, Self::None)
    }

    /// Returns `true` if per-index capacity should be reported.
    #[must_use]
    pub fn should_report_indexes(&self) -> bool {
        matches!(self, Self::Indexes)
    }
}

impl std::fmt::Display for ReturnConsumedCapacity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Controls whether item collection metrics are returned for writes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ReturnItemCollectionMetrics {
    /// Return item collection size estimates.
    #[serde(rename = "SIZE")]
    Size,
    /// Do not return item collection metrics (default).
    #[default]
    #[serde(rename = "NONE")]
    None,
}

impl ReturnItemCollectionMetrics {
    /// Returns the DynamoDB wire-format string representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Size => "SIZE",
            Self::None => "NONE",
        }
    }

    /// Returns `true` if metrics should be collected.
    #[must_use]
    pub fn should_report(&self) -> bool {
        matches!(self, Self::Size)
    }
}

impl std::fmt::Display for ReturnItemCollectionMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Attributes to retrieve in a `Query` or `Scan` operation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Select {
    /// All attributes of the item.
    #[default]
    #[serde(rename = "ALL_ATTRIBUTES")]
    AllAttributes,
    /// All projected attributes (for index queries).
    #[serde(rename = "ALL_PROJECTED_ATTRIBUTES")]
    AllProjectedAttributes,
    /// Only the attributes specified in `ProjectionExpression`.
    #[serde(rename = "SPECIFIC_ATTRIBUTES")]
    SpecificAttributes,
    /// Only the count of matching items (no item data).
    #[serde(rename = "COUNT")]
    Count,
}

impl Select {
    /// Returns the DynamoDB wire-format string representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AllAttributes => "ALL_ATTRIBUTES",
            Self::AllProjectedAttributes => "ALL_PROJECTED_ATTRIBUTES",
            Self::SpecificAttributes => "SPECIFIC_ATTRIBUTES",
            Self::Count => "COUNT",
        }
    }
}

impl std::fmt::Display for Select {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Logical operator for combining multiple conditions (legacy API).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ConditionalOperator {
    /// All conditions must be true.
    #[default]
    #[serde(rename = "AND")]
    And,
    /// At least one condition must be true.
    #[serde(rename = "OR")]
    Or,
}

impl ConditionalOperator {
    /// Returns the DynamoDB wire-format string representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::And => "AND",
            Self::Or => "OR",
        }
    }
}

impl std::fmt::Display for ConditionalOperator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Comparison operator for legacy `Condition` filters.
///
/// These are used with the legacy `ScanFilter`, `QueryFilter`, `KeyConditions`,
/// and `Expected` parameters. Modern applications should use expression-based
/// APIs (`FilterExpression`, `KeyConditionExpression`, `ConditionExpression`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ComparisonOperator {
    /// Equal to.
    #[serde(rename = "EQ")]
    Eq,
    /// Not equal to.
    #[serde(rename = "NE")]
    Ne,
    /// Less than or equal to.
    #[serde(rename = "LE")]
    Le,
    /// Less than.
    #[serde(rename = "LT")]
    Lt,
    /// Greater than or equal to.
    #[serde(rename = "GE")]
    Ge,
    /// Greater than.
    #[serde(rename = "GT")]
    Gt,
    /// Attribute does not exist.
    #[serde(rename = "NOT_NULL")]
    NotNull,
    /// Attribute exists (with any value, including null).
    #[serde(rename = "NULL")]
    Null,
    /// Attribute value contains the specified substring or set member.
    #[serde(rename = "CONTAINS")]
    Contains,
    /// Attribute value does not contain the specified substring or set member.
    #[serde(rename = "NOT_CONTAINS")]
    NotContains,
    /// Attribute value begins with the specified substring.
    #[serde(rename = "BEGINS_WITH")]
    BeginsWith,
    /// Attribute value is a member of the specified list.
    #[serde(rename = "IN")]
    In,
    /// Attribute value is between two values (inclusive).
    #[serde(rename = "BETWEEN")]
    Between,
}

impl ComparisonOperator {
    /// Returns the DynamoDB wire-format string representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Eq => "EQ",
            Self::Ne => "NE",
            Self::Le => "LE",
            Self::Lt => "LT",
            Self::Ge => "GE",
            Self::Gt => "GT",
            Self::NotNull => "NOT_NULL",
            Self::Null => "NULL",
            Self::Contains => "CONTAINS",
            Self::NotContains => "NOT_CONTAINS",
            Self::BeginsWith => "BEGINS_WITH",
            Self::In => "IN",
            Self::Between => "BETWEEN",
        }
    }
}

impl std::fmt::Display for ComparisonOperator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Structs - Key Schema & Attributes
// ---------------------------------------------------------------------------

/// An element of the key schema for a table or index.
///
/// Specifies an attribute name and whether it serves as a `HASH` (partition)
/// or `RANGE` (sort) key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct KeySchemaElement {
    /// The name of the key attribute.
    pub attribute_name: String,
    /// The role of the attribute in the key schema (`HASH` or `RANGE`).
    pub key_type: KeyType,
}

/// An attribute definition specifying the attribute name and its scalar type.
///
/// Used in `CreateTable` to declare attributes that participate in key schemas
/// or secondary indexes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AttributeDefinition {
    /// The name of the attribute.
    pub attribute_name: String,
    /// The scalar data type of the attribute (`S`, `N`, or `B`).
    pub attribute_type: ScalarAttributeType,
}

// ---------------------------------------------------------------------------
// Structs - Billing & Throughput
// ---------------------------------------------------------------------------

/// Summary of the billing mode for a table.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct BillingModeSummary {
    /// The billing mode currently in effect.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_mode: Option<BillingMode>,
    /// The date and time (epoch seconds) when the billing mode was last set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_update_to_pay_per_request_date_time: Option<f64>,
}

/// Provisioned throughput settings for a table or GSI (input).
///
/// Specifies the read and write capacity units to provision.
/// For on-demand tables this is accepted but not enforced.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ProvisionedThroughput {
    /// The maximum number of strongly consistent reads per second.
    pub read_capacity_units: i64,
    /// The maximum number of writes per second.
    pub write_capacity_units: i64,
}

/// Provisioned throughput description (output) including timestamps.
///
/// Returned in `DescribeTable` responses with additional metadata about
/// when capacity was last changed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ProvisionedThroughputDescription {
    /// The number of read capacity units provisioned.
    pub read_capacity_units: i64,
    /// The number of write capacity units provisioned.
    pub write_capacity_units: i64,
    /// The number of provisioned throughput decreases for this day.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number_of_decreases_today: Option<i64>,
    /// The date and time (epoch seconds) of the last provisioned throughput increase.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_increase_date_time: Option<f64>,
    /// The date and time (epoch seconds) of the last provisioned throughput decrease.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_decrease_date_time: Option<f64>,
}

/// Warm throughput description for a base table.
///
/// Recent AWS providers wait for this nested status to become `ACTIVE` after
/// table creation, even when warm throughput was not explicitly configured.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TableWarmThroughputDescription {
    /// The table warm read units per second.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_units_per_second: Option<i64>,
    /// The table warm write units per second.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write_units_per_second: Option<i64>,
    /// The warm throughput status.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<TableStatus>,
}

/// Warm throughput description for a global secondary index.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GlobalSecondaryIndexWarmThroughputDescription {
    /// The index warm read units per second.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_units_per_second: Option<i64>,
    /// The index warm write units per second.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write_units_per_second: Option<i64>,
    /// The index warm throughput status.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<IndexStatus>,
}

/// Point-in-time recovery settings applied to a table.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PointInTimeRecoverySpecification {
    /// Whether point-in-time recovery is enabled.
    pub point_in_time_recovery_enabled: bool,
    /// Optional recovery period in days.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_period_in_days: Option<i32>,
}

/// Point-in-time recovery description for a table.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PointInTimeRecoveryDescription {
    /// The current point-in-time recovery status.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub point_in_time_recovery_status: Option<PointInTimeRecoveryStatus>,
    /// The configured recovery period in days.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_period_in_days: Option<i32>,
    /// Earliest restorable timestamp as epoch seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub earliest_restorable_date_time: Option<f64>,
    /// Latest restorable timestamp as epoch seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_restorable_date_time: Option<f64>,
}

/// Continuous backups description for a table.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ContinuousBackupsDescription {
    /// Whether continuous backups are enabled for the table.
    pub continuous_backups_status: ContinuousBackupsStatus,
    /// The nested point-in-time recovery description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub point_in_time_recovery_description: Option<PointInTimeRecoveryDescription>,
}

// ---------------------------------------------------------------------------
// Structs - Projection
// ---------------------------------------------------------------------------

/// Projection settings for a secondary index.
///
/// Controls which attributes are copied (projected) from the base table
/// into the index.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Projection {
    /// The set of attributes projected into the index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projection_type: Option<ProjectionType>,
    /// The non-key attributes to project when `projection_type` is `INCLUDE`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub non_key_attributes: Vec<String>,
}

// ---------------------------------------------------------------------------
// Structs - Secondary Indexes (Input)
// ---------------------------------------------------------------------------

/// Global secondary index definition (input for `CreateTable`).
///
/// A GSI has its own key schema, projection, and optional provisioned throughput.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GlobalSecondaryIndex {
    /// The name of the global secondary index.
    pub index_name: String,
    /// The key schema for this index (partition key, optional sort key).
    pub key_schema: Vec<KeySchemaElement>,
    /// The attributes projected into this index.
    pub projection: Projection,
    /// The provisioned throughput for this index (required for `PROVISIONED` mode).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provisioned_throughput: Option<ProvisionedThroughput>,
}

/// Global secondary index description (output from `DescribeTable`).
///
/// Contains runtime metadata in addition to the index definition.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GlobalSecondaryIndexDescription {
    /// The name of the global secondary index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_name: Option<String>,
    /// The key schema for this index.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub key_schema: Vec<KeySchemaElement>,
    /// The projection settings for this index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projection: Option<Projection>,
    /// The current status of the index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_status: Option<IndexStatus>,
    /// Whether the index is currently backfilling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backfilling: Option<bool>,
    /// The provisioned throughput settings for this index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provisioned_throughput: Option<ProvisionedThroughputDescription>,
    /// The total size of the index in bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_size_bytes: Option<i64>,
    /// The warm throughput status and capacity values for this index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warm_throughput: Option<GlobalSecondaryIndexWarmThroughputDescription>,
    /// The number of items in the index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_count: Option<i64>,
    /// The Amazon Resource Name (ARN) of the index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_arn: Option<String>,
}

/// Local secondary index definition (input for `CreateTable`).
///
/// An LSI shares the partition key with the base table but uses a different sort key.
/// LSIs must be defined at table creation time and cannot be modified afterward.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LocalSecondaryIndex {
    /// The name of the local secondary index.
    pub index_name: String,
    /// The key schema for this index.
    pub key_schema: Vec<KeySchemaElement>,
    /// The attributes projected into this index.
    pub projection: Projection,
}

/// Local secondary index description (output from `DescribeTable`).
///
/// Contains runtime metadata in addition to the index definition.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LocalSecondaryIndexDescription {
    /// The name of the local secondary index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_name: Option<String>,
    /// The key schema for this index.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub key_schema: Vec<KeySchemaElement>,
    /// The projection settings for this index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projection: Option<Projection>,
    /// The total size of the index in bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_size_bytes: Option<i64>,
    /// The number of items in the index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_count: Option<i64>,
    /// The Amazon Resource Name (ARN) of the index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_arn: Option<String>,
}

// ---------------------------------------------------------------------------
// Structs - Streams
// ---------------------------------------------------------------------------

/// Stream specification for enabling DynamoDB Streams on a table.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct StreamSpecification {
    /// Whether DynamoDB Streams is enabled on the table.
    pub stream_enabled: bool,
    /// What information is written to the stream when an item is modified.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_view_type: Option<StreamViewType>,
}

// ---------------------------------------------------------------------------
// Structs - Server-Side Encryption
// ---------------------------------------------------------------------------

/// SSE specification (input for `CreateTable` / `UpdateTable`).
///
/// Specifies the desired encryption settings for a table.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SSESpecification {
    /// Whether server-side encryption is enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// The encryption type (`AES256` or `KMS`).
    #[serde(rename = "SSEType", skip_serializing_if = "Option::is_none")]
    pub sse_type: Option<SseType>,
    /// The KMS key ARN for `KMS` encryption type.
    #[serde(rename = "KMSMasterKeyId", skip_serializing_if = "Option::is_none")]
    pub kms_master_key_id: Option<String>,
}

/// SSE description (output from `DescribeTable`).
///
/// Describes the current encryption state of a table.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SSEDescription {
    /// The current status of server-side encryption.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<SseStatus>,
    /// The encryption type.
    #[serde(rename = "SSEType", skip_serializing_if = "Option::is_none")]
    pub sse_type: Option<SseType>,
    /// The KMS key ARN used for encryption.
    #[serde(rename = "KMSMasterKeyId", skip_serializing_if = "Option::is_none")]
    pub kms_master_key_id: Option<String>,
    /// The date and time (epoch seconds) when the KMS key became inaccessible.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inaccessible_encryption_date_time: Option<f64>,
}

// ---------------------------------------------------------------------------
// Structs - Time to Live
// ---------------------------------------------------------------------------

/// Time-to-Live specification for enabling or disabling TTL on a table.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TimeToLiveSpecification {
    /// The name of the TTL attribute used to store the expiration time.
    pub attribute_name: String,
    /// Whether TTL is enabled (`true`) or disabled (`false`).
    pub enabled: bool,
}

/// Time-to-Live description with status information.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TimeToLiveDescription {
    /// The name of the TTL attribute.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attribute_name: Option<String>,
    /// The TTL status: `ENABLED`, `DISABLED`, `ENABLING`, or `DISABLING`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_to_live_status: Option<String>,
}

// ---------------------------------------------------------------------------
// Structs - Transaction Types
// ---------------------------------------------------------------------------

/// A single write action within a `TransactWriteItems` request.
///
/// Exactly one of the four fields must be set.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TransactWriteItem {
    /// A condition check against an existing item (no mutation).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition_check: Option<ConditionCheck>,
    /// A put (insert or replace) action.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub put: Option<TransactPut>,
    /// A delete action.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delete: Option<TransactDelete>,
    /// An update action.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update: Option<TransactUpdate>,
}

/// A condition check within a transaction (no mutation).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ConditionCheck {
    /// The table containing the item.
    pub table_name: String,
    /// The primary key of the item to check.
    pub key: HashMap<String, AttributeValue>,
    /// The condition expression that must evaluate to true.
    pub condition_expression: String,
    /// Substitution tokens for attribute names.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression_attribute_names: Option<HashMap<String, String>>,
    /// Substitution tokens for attribute values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression_attribute_values: Option<HashMap<String, AttributeValue>>,
    /// Determines whether to return item attributes on condition check failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_values_on_condition_check_failure: Option<String>,
}

/// A put action within a transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TransactPut {
    /// The table to put the item into.
    pub table_name: String,
    /// The item to put.
    pub item: HashMap<String, AttributeValue>,
    /// An optional condition expression.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition_expression: Option<String>,
    /// Substitution tokens for attribute names.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression_attribute_names: Option<HashMap<String, String>>,
    /// Substitution tokens for attribute values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression_attribute_values: Option<HashMap<String, AttributeValue>>,
    /// Determines whether to return item attributes on condition check failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_values_on_condition_check_failure: Option<String>,
}

/// A delete action within a transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TransactDelete {
    /// The table containing the item to delete.
    pub table_name: String,
    /// The primary key of the item to delete.
    pub key: HashMap<String, AttributeValue>,
    /// An optional condition expression.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition_expression: Option<String>,
    /// Substitution tokens for attribute names.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression_attribute_names: Option<HashMap<String, String>>,
    /// Substitution tokens for attribute values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression_attribute_values: Option<HashMap<String, AttributeValue>>,
    /// Determines whether to return item attributes on condition check failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_values_on_condition_check_failure: Option<String>,
}

/// An update action within a transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TransactUpdate {
    /// The table containing the item to update.
    pub table_name: String,
    /// The primary key of the item to update.
    pub key: HashMap<String, AttributeValue>,
    /// The update expression defining the mutations.
    pub update_expression: String,
    /// An optional condition expression.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition_expression: Option<String>,
    /// Substitution tokens for attribute names.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression_attribute_names: Option<HashMap<String, String>>,
    /// Substitution tokens for attribute values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression_attribute_values: Option<HashMap<String, AttributeValue>>,
    /// Determines whether to return item attributes on condition check failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_values_on_condition_check_failure: Option<String>,
}

/// A single get action within a `TransactGetItems` request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TransactGetItem {
    /// The get operation to perform.
    pub get: Get,
}

/// A get operation targeting a single item by primary key.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Get {
    /// The table containing the item.
    pub table_name: String,
    /// The primary key of the item to retrieve.
    pub key: HashMap<String, AttributeValue>,
    /// A projection expression to limit returned attributes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projection_expression: Option<String>,
    /// Substitution tokens for attribute names.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression_attribute_names: Option<HashMap<String, String>>,
}

/// A reason why a transaction item was cancelled.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CancellationReason {
    /// The cancellation reason code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// A human-readable cancellation reason message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// The item that failed the condition check, if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<HashMap<String, AttributeValue>>,
}

/// A response item from a `TransactGetItems` operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ItemResponse {
    /// The retrieved item, or `None` if not found.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<HashMap<String, AttributeValue>>,
}

// ---------------------------------------------------------------------------
// Structs - Tags
// ---------------------------------------------------------------------------

/// A key-value tag associated with a DynamoDB resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Tag {
    /// The tag key (up to 128 Unicode characters).
    pub key: String,
    /// The tag value (up to 256 Unicode characters).
    pub value: String,
}

// ---------------------------------------------------------------------------
// Structs - Table Description
// ---------------------------------------------------------------------------

/// Comprehensive description of a DynamoDB table.
///
/// Returned by `DescribeTable`, `CreateTable`, and `DeleteTable` responses.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TableDescription {
    /// The name of the table.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_name: Option<String>,
    /// The current status of the table.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_status: Option<TableStatus>,
    /// The key schema for the table.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub key_schema: Vec<KeySchemaElement>,
    /// The attribute definitions for the table.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attribute_definitions: Vec<AttributeDefinition>,
    /// The date and time (epoch seconds) when the table was created.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creation_date_time: Option<f64>,
    /// The number of items in the table.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_count: Option<i64>,
    /// The total size of the table in bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_size_bytes: Option<i64>,
    /// The Amazon Resource Name (ARN) of the table.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_arn: Option<String>,
    /// A unique identifier for the table.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_id: Option<String>,
    /// The billing mode summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_mode_summary: Option<BillingModeSummary>,
    /// The provisioned throughput settings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provisioned_throughput: Option<ProvisionedThroughputDescription>,
    /// The warm throughput status and capacity values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warm_throughput: Option<TableWarmThroughputDescription>,
    /// The global secondary indexes on the table.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub global_secondary_indexes: Vec<GlobalSecondaryIndexDescription>,
    /// The local secondary indexes on the table.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub local_secondary_indexes: Vec<LocalSecondaryIndexDescription>,
    /// The stream specification for the table.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_specification: Option<StreamSpecification>,
    /// The latest stream ARN if streams are enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_stream_arn: Option<String>,
    /// The latest stream label if streams are enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_stream_label: Option<String>,
    /// The server-side encryption description.
    #[serde(rename = "SSEDescription", skip_serializing_if = "Option::is_none")]
    pub sse_description: Option<SSEDescription>,
    /// The deletion protection setting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deletion_protection_enabled: Option<bool>,
}

// ---------------------------------------------------------------------------
// Structs - Consumed Capacity
// ---------------------------------------------------------------------------

/// Capacity units consumed by an individual table or index.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Capacity {
    /// The total read capacity units consumed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_capacity_units: Option<f64>,
    /// The total write capacity units consumed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write_capacity_units: Option<f64>,
    /// The total capacity units consumed (read + write).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capacity_units: Option<f64>,
}

/// Total capacity consumed by an operation across table and indexes.
///
/// Returned when `ReturnConsumedCapacity` is set to `TOTAL` or `INDEXES`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ConsumedCapacity {
    /// The name of the table that was affected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_name: Option<String>,
    /// The total capacity units consumed by the operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capacity_units: Option<f64>,
    /// The total read capacity units consumed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_capacity_units: Option<f64>,
    /// The total write capacity units consumed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write_capacity_units: Option<f64>,
    /// The capacity consumed by the table (excluding indexes).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table: Option<Capacity>,
    /// The capacity consumed by each local secondary index.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub local_secondary_indexes: HashMap<String, Capacity>,
    /// The capacity consumed by each global secondary index.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub global_secondary_indexes: HashMap<String, Capacity>,
}

// ---------------------------------------------------------------------------
// Structs - Item Collection Metrics
// ---------------------------------------------------------------------------

/// Metrics about an item collection (items sharing the same partition key).
///
/// Returned for tables with local secondary indexes when
/// `ReturnItemCollectionMetrics` is `SIZE`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ItemCollectionMetrics {
    /// The partition key value of the item collection.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub item_collection_key: HashMap<String, AttributeValue>,
    /// An estimate of the item collection size in gigabytes (lower and upper bound).
    #[serde(
        rename = "SizeEstimateRangeGB",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub size_estimate_range_gb: Vec<f64>,
}

// ---------------------------------------------------------------------------
// Structs - Legacy Condition (for ScanFilter / QueryFilter / KeyConditions)
// ---------------------------------------------------------------------------

/// A condition for legacy filtering operations.
///
/// Used with `ScanFilter`, `QueryFilter`, `KeyConditions`, and `Expected`
/// parameters. Modern applications should use expression-based APIs instead.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Condition {
    /// The comparison operator.
    pub comparison_operator: ComparisonOperator,
    /// The attribute values to compare against.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attribute_value_list: Vec<AttributeValue>,
}

// ---------------------------------------------------------------------------
// Structs - Legacy API Types
// ---------------------------------------------------------------------------

/// Action to perform on an attribute during an `UpdateItem` operation (legacy API).
///
/// Used with `AttributeUpdates` parameter. Modern applications should use
/// `UpdateExpression` instead.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum AttributeAction {
    /// Set the attribute value.
    #[default]
    #[serde(rename = "PUT")]
    Put,
    /// Delete the attribute (for scalars) or remove elements from a set.
    #[serde(rename = "DELETE")]
    Delete,
    /// Add to a number or set attribute.
    #[serde(rename = "ADD")]
    Add,
}

impl AttributeAction {
    /// Returns the DynamoDB wire-format string representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Put => "PUT",
            Self::Delete => "DELETE",
            Self::Add => "ADD",
        }
    }
}

impl std::fmt::Display for AttributeAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An attribute value update for the legacy `AttributeUpdates` parameter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AttributeValueUpdate {
    /// The new value for the attribute.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<AttributeValue>,
    /// The action to perform on the attribute.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<AttributeAction>,
}

/// Expected attribute value for the legacy `Expected` parameter (conditional writes).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ExpectedAttributeValue {
    /// The value to compare against (legacy simple form).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<AttributeValue>,
    /// Whether the attribute must exist (`true`) or not exist (`false`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exists: Option<bool>,
    /// The comparison operator (extended form).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comparison_operator: Option<ComparisonOperator>,
    /// The attribute values to compare against (extended form).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attribute_value_list: Vec<AttributeValue>,
}

// ---------------------------------------------------------------------------
// Structs - Batch Operations
// ---------------------------------------------------------------------------

/// A set of keys and optional projection for `BatchGetItem`.
///
/// Describes the items to retrieve from a single table.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct KeysAndAttributes {
    /// The primary keys of the items to retrieve.
    pub keys: Vec<HashMap<String, AttributeValue>>,
    /// The attributes to retrieve. If not specified, all attributes are returned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projection_expression: Option<String>,
    /// Expression attribute names for substitution in `projection_expression`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression_attribute_names: Option<HashMap<String, String>>,
    /// Whether to use a consistent read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consistent_read: Option<bool>,
    /// Legacy: attribute names to retrieve (use `projection_expression` instead).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attributes_to_get: Vec<String>,
}

/// A single write request within a `BatchWriteItem` operation.
///
/// Exactly one of `put_request` or `delete_request` must be specified.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct WriteRequest {
    /// A request to put an item.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub put_request: Option<PutRequest>,
    /// A request to delete an item.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delete_request: Option<DeleteRequest>,
}

/// A request to put an item within a `BatchWriteItem` operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutRequest {
    /// The item attributes to put.
    pub item: HashMap<String, AttributeValue>,
}

/// A request to delete an item within a `BatchWriteItem` operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteRequest {
    /// The primary key of the item to delete.
    pub key: HashMap<String, AttributeValue>,
}

// ---------------------------------------------------------------------------
// Type aliases for common DynamoDB item shapes
// ---------------------------------------------------------------------------

/// A DynamoDB item represented as a map of attribute names to values.
pub type Item = HashMap<String, AttributeValue>;

/// A DynamoDB key represented as a map of key attribute names to values.
pub type Key = HashMap<String, AttributeValue>;

/// Expression attribute names mapping (`#name` placeholders to attribute names).
pub type ExpressionAttributeNames = HashMap<String, String>;

/// Expression attribute values mapping (`:value` placeholders to attribute values).
pub type ExpressionAttributeValues = HashMap<String, AttributeValue>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_serialize_key_schema_element() {
        let elem = KeySchemaElement {
            attribute_name: "pk".to_owned(),
            key_type: KeyType::Hash,
        };
        let json = serde_json::to_string(&elem).expect("serialize KeySchemaElement");
        assert_eq!(json, r#"{"AttributeName":"pk","KeyType":"HASH"}"#);
    }

    #[test]
    fn test_should_roundtrip_attribute_definition() {
        let def = AttributeDefinition {
            attribute_name: "id".to_owned(),
            attribute_type: ScalarAttributeType::S,
        };
        let json = serde_json::to_string(&def).expect("serialize AttributeDefinition");
        let parsed: AttributeDefinition =
            serde_json::from_str(&json).expect("deserialize AttributeDefinition");
        assert_eq!(def.attribute_name, parsed.attribute_name);
        assert_eq!(def.attribute_type, parsed.attribute_type);
    }

    #[test]
    fn test_should_serialize_table_status() {
        let status = TableStatus::Active;
        let json = serde_json::to_string(&status).expect("serialize TableStatus");
        assert_eq!(json, r#""ACTIVE""#);
    }

    #[test]
    fn test_should_default_billing_mode_to_pay_per_request() {
        let mode = BillingMode::default();
        assert_eq!(mode, BillingMode::PayPerRequest);
    }

    #[test]
    fn test_should_default_return_value_to_none() {
        let rv = ReturnValue::default();
        assert_eq!(rv, ReturnValue::None);
    }

    #[test]
    fn test_should_serialize_provisioned_throughput() {
        let pt = ProvisionedThroughput {
            read_capacity_units: 5,
            write_capacity_units: 10,
        };
        let json = serde_json::to_string(&pt).expect("serialize ProvisionedThroughput");
        assert_eq!(json, r#"{"ReadCapacityUnits":5,"WriteCapacityUnits":10}"#);
    }

    #[test]
    fn test_should_skip_none_fields_in_table_description() {
        let desc = TableDescription {
            table_name: Some("TestTable".to_owned()),
            table_status: Some(TableStatus::Active),
            ..Default::default()
        };
        let json = serde_json::to_string(&desc).expect("serialize TableDescription");
        assert!(json.contains(r#""TableName":"TestTable""#));
        assert!(json.contains(r#""TableStatus":"ACTIVE""#));
        // Fields that are None or empty should be absent
        assert!(!json.contains("TableArn"));
        assert!(!json.contains("KeySchema"));
        assert!(!json.contains("GlobalSecondaryIndexes"));
    }

    #[test]
    fn test_should_roundtrip_projection() {
        let proj = Projection {
            projection_type: Some(ProjectionType::Include),
            non_key_attributes: vec!["email".to_owned(), "name".to_owned()],
        };
        let json = serde_json::to_string(&proj).expect("serialize Projection");
        let parsed: Projection = serde_json::from_str(&json).expect("deserialize Projection");
        assert_eq!(proj.projection_type, parsed.projection_type);
        assert_eq!(proj.non_key_attributes, parsed.non_key_attributes);
    }

    #[test]
    fn test_should_serialize_write_request_with_put() {
        let mut item = HashMap::new();
        item.insert("id".to_owned(), AttributeValue::S("123".to_owned()));
        let req = WriteRequest {
            put_request: Some(PutRequest { item }),
            delete_request: Option::None,
        };
        let json = serde_json::to_string(&req).expect("serialize WriteRequest");
        assert!(json.contains("PutRequest"));
        assert!(!json.contains("DeleteRequest"));
    }

    #[test]
    fn test_should_serialize_write_request_with_delete() {
        let mut key = HashMap::new();
        key.insert("id".to_owned(), AttributeValue::S("456".to_owned()));
        let req = WriteRequest {
            put_request: Option::None,
            delete_request: Some(DeleteRequest { key }),
        };
        let json = serde_json::to_string(&req).expect("serialize WriteRequest");
        assert!(json.contains("DeleteRequest"));
        assert!(!json.contains("PutRequest"));
    }

    #[test]
    fn test_should_serialize_condition() {
        let cond = Condition {
            comparison_operator: ComparisonOperator::Eq,
            attribute_value_list: vec![AttributeValue::S("test".to_owned())],
        };
        let json = serde_json::to_string(&cond).expect("serialize Condition");
        assert!(json.contains(r#""ComparisonOperator":"EQ""#));
        assert!(json.contains("AttributeValueList"));
    }

    #[test]
    fn test_should_roundtrip_consumed_capacity() {
        let cap = ConsumedCapacity {
            table_name: Some("Orders".to_owned()),
            capacity_units: Some(5.0),
            ..Default::default()
        };
        let json = serde_json::to_string(&cap).expect("serialize ConsumedCapacity");
        let parsed: ConsumedCapacity =
            serde_json::from_str(&json).expect("deserialize ConsumedCapacity");
        assert_eq!(cap.table_name, parsed.table_name);
        assert_eq!(cap.capacity_units, parsed.capacity_units);
    }

    #[test]
    fn test_should_serialize_sse_specification() {
        let sse = SSESpecification {
            enabled: Some(true),
            sse_type: Some(SseType::Kms),
            kms_master_key_id: Some("arn:aws:kms:us-east-1:123456789012:key/abc".to_owned()),
        };
        let json = serde_json::to_string(&sse).expect("serialize SSESpecification");
        assert!(json.contains(r#""SSEType":"KMS""#));
        assert!(json.contains(r#""KMSMasterKeyId":"arn:aws:kms"#));
    }

    #[test]
    fn test_should_serialize_tag() {
        let tag = Tag {
            key: "Environment".to_owned(),
            value: "Production".to_owned(),
        };
        let json = serde_json::to_string(&tag).expect("serialize Tag");
        assert_eq!(json, r#"{"Key":"Environment","Value":"Production"}"#);
    }

    #[test]
    fn test_should_serialize_stream_specification() {
        let spec = StreamSpecification {
            stream_enabled: true,
            stream_view_type: Some(StreamViewType::NewAndOldImages),
        };
        let json = serde_json::to_string(&spec).expect("serialize StreamSpecification");
        assert!(json.contains(r#""StreamEnabled":true"#));
        assert!(json.contains(r#""StreamViewType":"NEW_AND_OLD_IMAGES""#));
    }

    #[test]
    fn test_should_serialize_keys_and_attributes() {
        let mut key = HashMap::new();
        key.insert("pk".to_owned(), AttributeValue::S("user-1".to_owned()));
        let ka = KeysAndAttributes {
            keys: vec![key],
            projection_expression: Some("id, #n".to_owned()),
            expression_attribute_names: Some(HashMap::from([("#n".to_owned(), "name".to_owned())])),
            consistent_read: Some(true),
            attributes_to_get: Vec::new(),
        };
        let json = serde_json::to_string(&ka).expect("serialize KeysAndAttributes");
        assert!(json.contains("ProjectionExpression"));
        assert!(json.contains("ExpressionAttributeNames"));
        assert!(json.contains("ConsistentRead"));
        assert!(!json.contains("AttributesToGet"));
    }

    #[test]
    fn test_should_roundtrip_global_secondary_index() {
        let gsi = GlobalSecondaryIndex {
            index_name: "gsi-email".to_owned(),
            key_schema: vec![KeySchemaElement {
                attribute_name: "email".to_owned(),
                key_type: KeyType::Hash,
            }],
            projection: Projection {
                projection_type: Some(ProjectionType::All),
                non_key_attributes: Vec::new(),
            },
            provisioned_throughput: Option::None,
        };
        let json = serde_json::to_string(&gsi).expect("serialize GlobalSecondaryIndex");
        let parsed: GlobalSecondaryIndex =
            serde_json::from_str(&json).expect("deserialize GlobalSecondaryIndex");
        assert_eq!(gsi.index_name, parsed.index_name);
        assert_eq!(gsi.key_schema.len(), parsed.key_schema.len());
    }

    #[test]
    fn test_should_display_all_enum_variants() {
        // Verify Display impl produces correct DynamoDB wire-format strings
        assert_eq!(KeyType::Hash.to_string(), "HASH");
        assert_eq!(KeyType::Range.to_string(), "RANGE");
        assert_eq!(ScalarAttributeType::S.to_string(), "S");
        assert_eq!(ScalarAttributeType::N.to_string(), "N");
        assert_eq!(ScalarAttributeType::B.to_string(), "B");
        assert_eq!(TableStatus::Active.to_string(), "ACTIVE");
        assert_eq!(TableStatus::Creating.to_string(), "CREATING");
        assert_eq!(BillingMode::PayPerRequest.to_string(), "PAY_PER_REQUEST");
        assert_eq!(ProjectionType::KeysOnly.to_string(), "KEYS_ONLY");
        assert_eq!(
            StreamViewType::NewAndOldImages.to_string(),
            "NEW_AND_OLD_IMAGES"
        );
        assert_eq!(SseType::Kms.to_string(), "KMS");
        assert_eq!(SseStatus::Enabled.to_string(), "ENABLED");
        assert_eq!(IndexStatus::Active.to_string(), "ACTIVE");
        assert_eq!(ReturnValue::AllNew.to_string(), "ALL_NEW");
        assert_eq!(ReturnConsumedCapacity::Indexes.to_string(), "INDEXES");
        assert_eq!(ReturnItemCollectionMetrics::Size.to_string(), "SIZE");
        assert_eq!(Select::Count.to_string(), "COUNT");
        assert_eq!(ConditionalOperator::And.to_string(), "AND");
        assert_eq!(ComparisonOperator::BeginsWith.to_string(), "BEGINS_WITH");
    }

    #[test]
    fn test_should_report_consumed_capacity_settings() {
        assert!(!ReturnConsumedCapacity::None.should_report());
        assert!(ReturnConsumedCapacity::Total.should_report());
        assert!(ReturnConsumedCapacity::Indexes.should_report());
        assert!(!ReturnConsumedCapacity::Total.should_report_indexes());
        assert!(ReturnConsumedCapacity::Indexes.should_report_indexes());
    }

    #[test]
    fn test_should_report_item_collection_metrics_settings() {
        assert!(!ReturnItemCollectionMetrics::None.should_report());
        assert!(ReturnItemCollectionMetrics::Size.should_report());
    }

    #[test]
    fn test_should_deserialize_table_description_from_dynamodb_json() {
        let json = r#"{
            "TableName": "Users",
            "TableStatus": "ACTIVE",
            "KeySchema": [
                {"AttributeName": "pk", "KeyType": "HASH"},
                {"AttributeName": "sk", "KeyType": "RANGE"}
            ],
            "AttributeDefinitions": [
                {"AttributeName": "pk", "AttributeType": "S"},
                {"AttributeName": "sk", "AttributeType": "S"}
            ],
            "CreationDateTime": 1709136000.0,
            "ItemCount": 42,
            "TableSizeBytes": 8192,
            "TableArn": "arn:aws:dynamodb:us-east-1:123456789012:table/Users",
            "TableId": "abc-123-def",
            "ProvisionedThroughput": {
                "ReadCapacityUnits": 5,
                "WriteCapacityUnits": 10,
                "NumberOfDecreasesToday": 0
            },
            "BillingModeSummary": {
                "BillingMode": "PROVISIONED"
            },
            "WarmThroughput": {
                "ReadUnitsPerSecond": 12000,
                "WriteUnitsPerSecond": 4000,
                "Status": "ACTIVE"
            }
        }"#;
        let desc: TableDescription =
            serde_json::from_str(json).expect("deserialize TableDescription");
        assert_eq!(desc.table_name.as_deref(), Some("Users"));
        assert_eq!(desc.table_status, Some(TableStatus::Active));
        assert_eq!(desc.key_schema.len(), 2);
        assert_eq!(desc.attribute_definitions.len(), 2);
        assert_eq!(desc.item_count, Some(42));
        assert_eq!(desc.table_size_bytes, Some(8192));
        assert_eq!(desc.table_id.as_deref(), Some("abc-123-def"));
        assert_eq!(
            desc.billing_mode_summary
                .as_ref()
                .and_then(|b| b.billing_mode.as_ref()),
            Some(&BillingMode::Provisioned)
        );
        assert_eq!(
            desc.warm_throughput
                .as_ref()
                .and_then(|w| w.status.as_ref()),
            Some(&TableStatus::Active)
        );
    }

    #[test]
    fn test_should_roundtrip_all_comparison_operators() {
        let operators = [
            ComparisonOperator::Eq,
            ComparisonOperator::Ne,
            ComparisonOperator::Le,
            ComparisonOperator::Lt,
            ComparisonOperator::Ge,
            ComparisonOperator::Gt,
            ComparisonOperator::NotNull,
            ComparisonOperator::Null,
            ComparisonOperator::Contains,
            ComparisonOperator::NotContains,
            ComparisonOperator::BeginsWith,
            ComparisonOperator::In,
            ComparisonOperator::Between,
        ];
        for op in &operators {
            let json = serde_json::to_string(op).expect("serialize ComparisonOperator");
            let parsed: ComparisonOperator =
                serde_json::from_str(&json).expect("deserialize ComparisonOperator");
            assert_eq!(op, &parsed);
        }
    }

    #[test]
    fn test_should_roundtrip_all_return_values() {
        let values = [
            ReturnValue::None,
            ReturnValue::AllOld,
            ReturnValue::UpdatedOld,
            ReturnValue::AllNew,
            ReturnValue::UpdatedNew,
        ];
        for rv in &values {
            let json = serde_json::to_string(rv).expect("serialize ReturnValue");
            let parsed: ReturnValue = serde_json::from_str(&json).expect("deserialize ReturnValue");
            assert_eq!(rv, &parsed);
        }
    }

    #[test]
    fn test_should_serialize_item_collection_metrics() {
        let mut key = HashMap::new();
        key.insert("pk".to_owned(), AttributeValue::S("user-1".to_owned()));
        let metrics = ItemCollectionMetrics {
            item_collection_key: key,
            size_estimate_range_gb: vec![0.5, 1.0],
        };
        let json = serde_json::to_string(&metrics).expect("serialize ItemCollectionMetrics");
        assert!(json.contains("ItemCollectionKey"));
        assert!(json.contains("SizeEstimateRangeGB"));
    }

    #[test]
    fn test_should_serialize_billing_mode_summary() {
        let summary = BillingModeSummary {
            billing_mode: Some(BillingMode::PayPerRequest),
            last_update_to_pay_per_request_date_time: Some(1_709_136_000.0),
        };
        let json = serde_json::to_string(&summary).expect("serialize BillingModeSummary");
        assert!(json.contains(r#""BillingMode":"PAY_PER_REQUEST""#));
        assert!(json.contains("LastUpdateToPayPerRequestDateTime"));
    }

    #[test]
    fn test_should_serialize_gsi_description_with_all_fields() {
        let desc = GlobalSecondaryIndexDescription {
            index_name: Some("gsi-status".to_owned()),
            key_schema: vec![KeySchemaElement {
                attribute_name: "status".to_owned(),
                key_type: KeyType::Hash,
            }],
            projection: Some(Projection {
                projection_type: Some(ProjectionType::All),
                ..Default::default()
            }),
            index_status: Some(IndexStatus::Active),
            backfilling: Some(false),
            provisioned_throughput: Some(ProvisionedThroughputDescription {
                read_capacity_units: 5,
                write_capacity_units: 5,
                ..Default::default()
            }),
            index_size_bytes: Some(1024),
            warm_throughput: Some(GlobalSecondaryIndexWarmThroughputDescription {
                read_units_per_second: Some(12_000),
                write_units_per_second: Some(4_000),
                status: Some(IndexStatus::Active),
            }),
            item_count: Some(10),
            index_arn: Some(
                "arn:aws:dynamodb:us-east-1:123456789012:table/T/index/gsi-status".to_owned(),
            ),
        };
        let json = serde_json::to_string(&desc).expect("serialize GlobalSecondaryIndexDescription");
        let parsed: GlobalSecondaryIndexDescription =
            serde_json::from_str(&json).expect("deserialize GlobalSecondaryIndexDescription");
        assert_eq!(desc.index_name, parsed.index_name);
        assert_eq!(desc.index_status, parsed.index_status);
        assert_eq!(
            parsed
                .warm_throughput
                .as_ref()
                .and_then(|w| w.status.as_ref()),
            Some(&IndexStatus::Active)
        );
        assert_eq!(desc.item_count, parsed.item_count);
    }
}
