//! Auto-generated from AWS CloudWatch Logs Smithy model. DO NOT EDIT.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// CloudWatch Logs DataProtectionStatus enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum DataProtectionStatus {
    /// Default variant.
    #[default]
    #[serde(rename = "ACTIVATED")]
    Activated,
    #[serde(rename = "ARCHIVED")]
    Archived,
    #[serde(rename = "DELETED")]
    Deleted,
    #[serde(rename = "DISABLED")]
    Disabled,
}

impl DataProtectionStatus {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Activated => "ACTIVATED",
            Self::Archived => "ARCHIVED",
            Self::Deleted => "DELETED",
            Self::Disabled => "DISABLED",
        }
    }
}

impl std::fmt::Display for DataProtectionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for DataProtectionStatus {
    fn from(s: &str) -> Self {
        match s {
            "ACTIVATED" => Self::Activated,
            "ARCHIVED" => Self::Archived,
            "DELETED" => Self::Deleted,
            "DISABLED" => Self::Disabled,
            _ => Self::default(),
        }
    }
}

/// CloudWatch Logs Distribution enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Distribution {
    /// Default variant.
    #[default]
    ByLogStream,
    Random,
}

impl Distribution {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ByLogStream => "ByLogStream",
            Self::Random => "Random",
        }
    }
}

impl std::fmt::Display for Distribution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for Distribution {
    fn from(s: &str) -> Self {
        match s {
            "ByLogStream" => Self::ByLogStream,
            "Random" => Self::Random,
            _ => Self::default(),
        }
    }
}

/// CloudWatch Logs EntityRejectionErrorType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum EntityRejectionErrorType {
    /// Default variant.
    #[default]
    EntitySizeTooLarge,
    InvalidAttributes,
    InvalidEntity,
    #[serde(rename = "InvalidKeyAttributes")]
    InvalidKeyAttribute,
    InvalidTypeValue,
    MissingRequiredFields,
    UnsupportedLogGroupType,
}

impl EntityRejectionErrorType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::EntitySizeTooLarge => "EntitySizeTooLarge",
            Self::InvalidAttributes => "InvalidAttributes",
            Self::InvalidEntity => "InvalidEntity",
            Self::InvalidKeyAttribute => "InvalidKeyAttributes",
            Self::InvalidTypeValue => "InvalidTypeValue",
            Self::MissingRequiredFields => "MissingRequiredFields",
            Self::UnsupportedLogGroupType => "UnsupportedLogGroupType",
        }
    }
}

impl std::fmt::Display for EntityRejectionErrorType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for EntityRejectionErrorType {
    fn from(s: &str) -> Self {
        match s {
            "EntitySizeTooLarge" => Self::EntitySizeTooLarge,
            "InvalidAttributes" => Self::InvalidAttributes,
            "InvalidEntity" => Self::InvalidEntity,
            "InvalidKeyAttributes" => Self::InvalidKeyAttribute,
            "InvalidTypeValue" => Self::InvalidTypeValue,
            "MissingRequiredFields" => Self::MissingRequiredFields,
            "UnsupportedLogGroupType" => Self::UnsupportedLogGroupType,
            _ => Self::default(),
        }
    }
}

/// CloudWatch Logs ExportTaskStatusCode enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ExportTaskStatusCode {
    /// Default variant.
    #[default]
    #[serde(rename = "CANCELLED")]
    Cancelled,
    #[serde(rename = "COMPLETED")]
    Completed,
    #[serde(rename = "FAILED")]
    Failed,
    #[serde(rename = "PENDING")]
    Pending,
    #[serde(rename = "PENDING_CANCEL")]
    PendingCancel,
    #[serde(rename = "RUNNING")]
    Running,
}

impl ExportTaskStatusCode {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cancelled => "CANCELLED",
            Self::Completed => "COMPLETED",
            Self::Failed => "FAILED",
            Self::Pending => "PENDING",
            Self::PendingCancel => "PENDING_CANCEL",
            Self::Running => "RUNNING",
        }
    }
}

impl std::fmt::Display for ExportTaskStatusCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ExportTaskStatusCode {
    fn from(s: &str) -> Self {
        match s {
            "CANCELLED" => Self::Cancelled,
            "COMPLETED" => Self::Completed,
            "FAILED" => Self::Failed,
            "PENDING" => Self::Pending,
            "PENDING_CANCEL" => Self::PendingCancel,
            "RUNNING" => Self::Running,
            _ => Self::default(),
        }
    }
}

/// CloudWatch Logs InheritedProperty enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum InheritedProperty {
    /// Default variant.
    #[default]
    #[serde(rename = "ACCOUNT_DATA_PROTECTION")]
    AccountDataProtection,
}

impl InheritedProperty {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AccountDataProtection => "ACCOUNT_DATA_PROTECTION",
        }
    }
}

impl std::fmt::Display for InheritedProperty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for InheritedProperty {
    fn from(s: &str) -> Self {
        match s {
            "ACCOUNT_DATA_PROTECTION" => Self::AccountDataProtection,
            _ => Self::default(),
        }
    }
}

/// CloudWatch Logs LogGroupClass enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum LogGroupClass {
    /// Default variant.
    #[default]
    #[serde(rename = "DELIVERY")]
    Delivery,
    #[serde(rename = "INFREQUENT_ACCESS")]
    InfrequentAccess,
    #[serde(rename = "STANDARD")]
    Standard,
}

impl LogGroupClass {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Delivery => "DELIVERY",
            Self::InfrequentAccess => "INFREQUENT_ACCESS",
            Self::Standard => "STANDARD",
        }
    }
}

impl std::fmt::Display for LogGroupClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for LogGroupClass {
    fn from(s: &str) -> Self {
        match s {
            "DELIVERY" => Self::Delivery,
            "INFREQUENT_ACCESS" => Self::InfrequentAccess,
            "STANDARD" => Self::Standard,
            _ => Self::default(),
        }
    }
}

/// CloudWatch Logs OrderBy enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum OrderBy {
    /// Default variant.
    #[default]
    LastEventTime,
    LogStreamName,
}

impl OrderBy {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::LastEventTime => "LastEventTime",
            Self::LogStreamName => "LogStreamName",
        }
    }
}

impl std::fmt::Display for OrderBy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for OrderBy {
    fn from(s: &str) -> Self {
        match s {
            "LastEventTime" => Self::LastEventTime,
            "LogStreamName" => Self::LogStreamName,
            _ => Self::default(),
        }
    }
}

/// CloudWatch Logs PolicyScope enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum PolicyScope {
    /// Default variant.
    #[default]
    #[serde(rename = "ACCOUNT")]
    Account,
    #[serde(rename = "RESOURCE")]
    Resource,
}

impl PolicyScope {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Account => "ACCOUNT",
            Self::Resource => "RESOURCE",
        }
    }
}

impl std::fmt::Display for PolicyScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for PolicyScope {
    fn from(s: &str) -> Self {
        match s {
            "ACCOUNT" => Self::Account,
            "RESOURCE" => Self::Resource,
            _ => Self::default(),
        }
    }
}

/// CloudWatch Logs QueryLanguage enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum QueryLanguage {
    /// Default variant.
    #[default]
    #[serde(rename = "CWLI")]
    Cwli,
    #[serde(rename = "PPL")]
    Ppl,
    #[serde(rename = "SQL")]
    Sql,
}

impl QueryLanguage {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cwli => "CWLI",
            Self::Ppl => "PPL",
            Self::Sql => "SQL",
        }
    }
}

impl std::fmt::Display for QueryLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for QueryLanguage {
    fn from(s: &str) -> Self {
        match s {
            "CWLI" => Self::Cwli,
            "PPL" => Self::Ppl,
            "SQL" => Self::Sql,
            _ => Self::default(),
        }
    }
}

/// CloudWatch Logs QueryStatus enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum QueryStatus {
    /// Default variant.
    #[default]
    Cancelled,
    Complete,
    Failed,
    Running,
    Scheduled,
    Timeout,
    Unknown,
}

impl QueryStatus {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cancelled => "Cancelled",
            Self::Complete => "Complete",
            Self::Failed => "Failed",
            Self::Running => "Running",
            Self::Scheduled => "Scheduled",
            Self::Timeout => "Timeout",
            Self::Unknown => "Unknown",
        }
    }
}

impl std::fmt::Display for QueryStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for QueryStatus {
    fn from(s: &str) -> Self {
        match s {
            "Cancelled" => Self::Cancelled,
            "Complete" => Self::Complete,
            "Failed" => Self::Failed,
            "Running" => Self::Running,
            "Scheduled" => Self::Scheduled,
            "Timeout" => Self::Timeout,
            "Unknown" => Self::Unknown,
            _ => Self::default(),
        }
    }
}

/// CloudWatch Logs StandardUnit enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum StandardUnit {
    /// Default variant.
    #[default]
    Bits,
    #[serde(rename = "Bits/Second")]
    BitsSecond,
    Bytes,
    #[serde(rename = "Bytes/Second")]
    BytesSecond,
    Count,
    #[serde(rename = "Count/Second")]
    CountSecond,
    Gigabits,
    #[serde(rename = "Gigabits/Second")]
    GigabitsSecond,
    Gigabytes,
    #[serde(rename = "Gigabytes/Second")]
    GigabytesSecond,
    Kilobits,
    #[serde(rename = "Kilobits/Second")]
    KilobitsSecond,
    Kilobytes,
    #[serde(rename = "Kilobytes/Second")]
    KilobytesSecond,
    Megabits,
    #[serde(rename = "Megabits/Second")]
    MegabitsSecond,
    Megabytes,
    #[serde(rename = "Megabytes/Second")]
    MegabytesSecond,
    Microseconds,
    Milliseconds,
    None,
    Percent,
    Seconds,
    Terabits,
    #[serde(rename = "Terabits/Second")]
    TerabitsSecond,
    Terabytes,
    #[serde(rename = "Terabytes/Second")]
    TerabytesSecond,
}

impl StandardUnit {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Bits => "Bits",
            Self::BitsSecond => "Bits/Second",
            Self::Bytes => "Bytes",
            Self::BytesSecond => "Bytes/Second",
            Self::Count => "Count",
            Self::CountSecond => "Count/Second",
            Self::Gigabits => "Gigabits",
            Self::GigabitsSecond => "Gigabits/Second",
            Self::Gigabytes => "Gigabytes",
            Self::GigabytesSecond => "Gigabytes/Second",
            Self::Kilobits => "Kilobits",
            Self::KilobitsSecond => "Kilobits/Second",
            Self::Kilobytes => "Kilobytes",
            Self::KilobytesSecond => "Kilobytes/Second",
            Self::Megabits => "Megabits",
            Self::MegabitsSecond => "Megabits/Second",
            Self::Megabytes => "Megabytes",
            Self::MegabytesSecond => "Megabytes/Second",
            Self::Microseconds => "Microseconds",
            Self::Milliseconds => "Milliseconds",
            Self::None => "None",
            Self::Percent => "Percent",
            Self::Seconds => "Seconds",
            Self::Terabits => "Terabits",
            Self::TerabitsSecond => "Terabits/Second",
            Self::Terabytes => "Terabytes",
            Self::TerabytesSecond => "Terabytes/Second",
        }
    }
}

impl std::fmt::Display for StandardUnit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for StandardUnit {
    fn from(s: &str) -> Self {
        match s {
            "Bits" => Self::Bits,
            "Bits/Second" => Self::BitsSecond,
            "Bytes" => Self::Bytes,
            "Bytes/Second" => Self::BytesSecond,
            "Count" => Self::Count,
            "Count/Second" => Self::CountSecond,
            "Gigabits" => Self::Gigabits,
            "Gigabits/Second" => Self::GigabitsSecond,
            "Gigabytes" => Self::Gigabytes,
            "Gigabytes/Second" => Self::GigabytesSecond,
            "Kilobits" => Self::Kilobits,
            "Kilobits/Second" => Self::KilobitsSecond,
            "Kilobytes" => Self::Kilobytes,
            "Kilobytes/Second" => Self::KilobytesSecond,
            "Megabits" => Self::Megabits,
            "Megabits/Second" => Self::MegabitsSecond,
            "Megabytes" => Self::Megabytes,
            "Megabytes/Second" => Self::MegabytesSecond,
            "Microseconds" => Self::Microseconds,
            "Milliseconds" => Self::Milliseconds,
            "None" => Self::None,
            "Percent" => Self::Percent,
            "Seconds" => Self::Seconds,
            "Terabits" => Self::Terabits,
            "Terabits/Second" => Self::TerabitsSecond,
            "Terabytes" => Self::Terabytes,
            "Terabytes/Second" => Self::TerabytesSecond,
            _ => Self::default(),
        }
    }
}

/// CloudWatch Logs Destination.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Destination {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_policy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creation_time: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_arn: Option<String>,
}

/// CloudWatch Logs Entity.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entity {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub attributes: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub key_attributes: HashMap<String, String>,
}

/// CloudWatch Logs ExportTask.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportTask {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_info: Option<ExportTaskExecutionInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_group_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ExportTaskStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<i64>,
}

/// CloudWatch Logs ExportTaskExecutionInfo.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportTaskExecutionInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_time: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creation_time: Option<i64>,
}

/// CloudWatch Logs ExportTaskStatus.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportTaskStatus {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<ExportTaskStatusCode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// CloudWatch Logs FilteredLogEvent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilteredLogEvent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingestion_time: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_stream_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
}

/// CloudWatch Logs InputLogEvent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputLogEvent {
    pub message: String,
    pub timestamp: i64,
}

/// CloudWatch Logs LogGroup.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogGroup {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bearer_token_authentication_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creation_time: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_protection_status: Option<DataProtectionStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deletion_protection_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inherited_properties: Vec<InheritedProperty>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kms_key_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_group_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_group_class: Option<LogGroupClass>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_group_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric_filter_count: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retention_in_days: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stored_bytes: Option<i64>,
}

/// CloudWatch Logs LogStream.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogStream {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creation_time: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_event_timestamp: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_event_timestamp: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_ingestion_time: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_stream_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stored_bytes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_sequence_token: Option<String>,
}

/// CloudWatch Logs MetricFilter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apply_on_transformed_logs: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creation_time: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emit_system_field_dimensions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_selection_criteria: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter_pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_group_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metric_transformations: Vec<MetricTransformation>,
}

/// CloudWatch Logs MetricFilterMatchRecord.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricFilterMatchRecord {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_number: Option<i64>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub extracted_values: HashMap<String, String>,
}

/// CloudWatch Logs MetricTransformation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricTransformation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<f64>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub dimensions: HashMap<String, String>,
    pub metric_name: String,
    pub metric_namespace: String,
    pub metric_value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<StandardUnit>,
}

/// CloudWatch Logs OutputLogEvent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputLogEvent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingestion_time: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
}

/// CloudWatch Logs QueryDefinition.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub log_group_names: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_definition_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_language: Option<QueryLanguage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_string: Option<String>,
}

/// CloudWatch Logs QueryInfo.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create_time: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_group_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_language: Option<QueryLanguage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_string: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<QueryStatus>,
}

/// CloudWatch Logs QueryStatistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryStatistics {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_scanned: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_bytes_skipped: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_records_skipped: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_groups_scanned: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub records_matched: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub records_scanned: Option<f64>,
}

/// CloudWatch Logs RejectedEntityInfo.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RejectedEntityInfo {
    pub error_type: EntityRejectionErrorType,
}

/// CloudWatch Logs RejectedLogEventsInfo.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RejectedLogEventsInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expired_log_event_end_index: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub too_new_log_event_start_index: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub too_old_log_event_end_index: Option<i32>,
}

/// CloudWatch Logs ResourcePolicy.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourcePolicy {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_updated_time: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_document: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_scope: Option<PolicyScope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
}

/// CloudWatch Logs ResultField.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultField {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

/// CloudWatch Logs SearchedLogStream.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchedLogStream {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_stream_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub searched_completely: Option<bool>,
}

/// CloudWatch Logs SubscriptionFilter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apply_on_transformed_logs: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creation_time: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distribution: Option<Distribution>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emit_system_fields: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_selection_criteria: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter_pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_group_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_arn: Option<String>,
}
