//! Auto-generated from AWS CloudWatch Smithy model. DO NOT EDIT.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// CloudWatch ActionsSuppressedBy enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ActionsSuppressedBy {
    /// Default variant.
    #[default]
    Alarm,
    ExtensionPeriod,
    WaitPeriod,
}

impl ActionsSuppressedBy {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Alarm => "Alarm",
            Self::ExtensionPeriod => "ExtensionPeriod",
            Self::WaitPeriod => "WaitPeriod",
        }
    }
}

impl std::fmt::Display for ActionsSuppressedBy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ActionsSuppressedBy {
    fn from(s: &str) -> Self {
        match s {
            "Alarm" => Self::Alarm,
            "ExtensionPeriod" => Self::ExtensionPeriod,
            "WaitPeriod" => Self::WaitPeriod,
            _ => Self::default(),
        }
    }
}

/// CloudWatch AlarmType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum AlarmType {
    /// Default variant.
    #[default]
    CompositeAlarm,
    MetricAlarm,
}

impl AlarmType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CompositeAlarm => "CompositeAlarm",
            Self::MetricAlarm => "MetricAlarm",
        }
    }
}

impl std::fmt::Display for AlarmType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for AlarmType {
    fn from(s: &str) -> Self {
        match s {
            "CompositeAlarm" => Self::CompositeAlarm,
            "MetricAlarm" => Self::MetricAlarm,
            _ => Self::default(),
        }
    }
}

/// CloudWatch AnomalyDetectorStateValue enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum AnomalyDetectorStateValue {
    /// Default variant.
    #[default]
    #[serde(rename = "PENDING_TRAINING")]
    PendingTraining,
    #[serde(rename = "TRAINED")]
    Trained,
    #[serde(rename = "TRAINED_INSUFFICIENT_DATA")]
    TrainedInsufficientData,
}

impl AnomalyDetectorStateValue {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PendingTraining => "PENDING_TRAINING",
            Self::Trained => "TRAINED",
            Self::TrainedInsufficientData => "TRAINED_INSUFFICIENT_DATA",
        }
    }
}

impl std::fmt::Display for AnomalyDetectorStateValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for AnomalyDetectorStateValue {
    fn from(s: &str) -> Self {
        match s {
            "PENDING_TRAINING" => Self::PendingTraining,
            "TRAINED" => Self::Trained,
            "TRAINED_INSUFFICIENT_DATA" => Self::TrainedInsufficientData,
            _ => Self::default(),
        }
    }
}

/// CloudWatch AnomalyDetectorType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum AnomalyDetectorType {
    /// Default variant.
    #[default]
    #[serde(rename = "METRIC_MATH")]
    MetricMath,
    #[serde(rename = "SINGLE_METRIC")]
    SingleMetric,
}

impl AnomalyDetectorType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::MetricMath => "METRIC_MATH",
            Self::SingleMetric => "SINGLE_METRIC",
        }
    }
}

impl std::fmt::Display for AnomalyDetectorType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for AnomalyDetectorType {
    fn from(s: &str) -> Self {
        match s {
            "METRIC_MATH" => Self::MetricMath,
            "SINGLE_METRIC" => Self::SingleMetric,
            _ => Self::default(),
        }
    }
}

/// CloudWatch ComparisonOperator enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ComparisonOperator {
    /// Default variant.
    #[default]
    GreaterThanOrEqualToThreshold,
    GreaterThanThreshold,
    GreaterThanUpperThreshold,
    LessThanLowerOrGreaterThanUpperThreshold,
    LessThanLowerThreshold,
    LessThanOrEqualToThreshold,
    LessThanThreshold,
}

impl ComparisonOperator {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::GreaterThanOrEqualToThreshold => "GreaterThanOrEqualToThreshold",
            Self::GreaterThanThreshold => "GreaterThanThreshold",
            Self::GreaterThanUpperThreshold => "GreaterThanUpperThreshold",
            Self::LessThanLowerOrGreaterThanUpperThreshold => {
                "LessThanLowerOrGreaterThanUpperThreshold"
            }
            Self::LessThanLowerThreshold => "LessThanLowerThreshold",
            Self::LessThanOrEqualToThreshold => "LessThanOrEqualToThreshold",
            Self::LessThanThreshold => "LessThanThreshold",
        }
    }
}

impl std::fmt::Display for ComparisonOperator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ComparisonOperator {
    fn from(s: &str) -> Self {
        match s {
            "GreaterThanOrEqualToThreshold" => Self::GreaterThanOrEqualToThreshold,
            "GreaterThanThreshold" => Self::GreaterThanThreshold,
            "GreaterThanUpperThreshold" => Self::GreaterThanUpperThreshold,
            "LessThanLowerOrGreaterThanUpperThreshold" => {
                Self::LessThanLowerOrGreaterThanUpperThreshold
            }
            "LessThanLowerThreshold" => Self::LessThanLowerThreshold,
            "LessThanOrEqualToThreshold" => Self::LessThanOrEqualToThreshold,
            "LessThanThreshold" => Self::LessThanThreshold,
            _ => Self::default(),
        }
    }
}

/// CloudWatch EvaluationState enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum EvaluationState {
    /// Default variant.
    #[default]
    #[serde(rename = "EVALUATION_ERROR")]
    EvaluationError,
    #[serde(rename = "EVALUATION_FAILURE")]
    EvaluationFailure,
    #[serde(rename = "PARTIAL_DATA")]
    PartialData,
}

impl EvaluationState {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::EvaluationError => "EVALUATION_ERROR",
            Self::EvaluationFailure => "EVALUATION_FAILURE",
            Self::PartialData => "PARTIAL_DATA",
        }
    }
}

impl std::fmt::Display for EvaluationState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for EvaluationState {
    fn from(s: &str) -> Self {
        match s {
            "EVALUATION_ERROR" => Self::EvaluationError,
            "EVALUATION_FAILURE" => Self::EvaluationFailure,
            "PARTIAL_DATA" => Self::PartialData,
            _ => Self::default(),
        }
    }
}

/// CloudWatch HistoryItemType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum HistoryItemType {
    /// Default variant.
    #[default]
    Action,
    AlarmContributorAction,
    AlarmContributorStateUpdate,
    ConfigurationUpdate,
    StateUpdate,
}

impl HistoryItemType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Action => "Action",
            Self::AlarmContributorAction => "AlarmContributorAction",
            Self::AlarmContributorStateUpdate => "AlarmContributorStateUpdate",
            Self::ConfigurationUpdate => "ConfigurationUpdate",
            Self::StateUpdate => "StateUpdate",
        }
    }
}

impl std::fmt::Display for HistoryItemType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for HistoryItemType {
    fn from(s: &str) -> Self {
        match s {
            "Action" => Self::Action,
            "AlarmContributorAction" => Self::AlarmContributorAction,
            "AlarmContributorStateUpdate" => Self::AlarmContributorStateUpdate,
            "ConfigurationUpdate" => Self::ConfigurationUpdate,
            "StateUpdate" => Self::StateUpdate,
            _ => Self::default(),
        }
    }
}

/// CloudWatch MetricStreamOutputFormat enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum MetricStreamOutputFormat {
    /// Default variant.
    #[default]
    #[serde(rename = "json")]
    Json,
    #[serde(rename = "opentelemetry0.7")]
    OpenTelemetry07,
    #[serde(rename = "opentelemetry1.0")]
    OpenTelemetry10,
}

impl MetricStreamOutputFormat {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::OpenTelemetry07 => "opentelemetry0.7",
            Self::OpenTelemetry10 => "opentelemetry1.0",
        }
    }
}

impl std::fmt::Display for MetricStreamOutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for MetricStreamOutputFormat {
    fn from(s: &str) -> Self {
        match s {
            "json" => Self::Json,
            "opentelemetry0.7" => Self::OpenTelemetry07,
            "opentelemetry1.0" => Self::OpenTelemetry10,
            _ => Self::default(),
        }
    }
}

/// CloudWatch RecentlyActive enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum RecentlyActive {
    /// Default variant.
    #[default]
    #[serde(rename = "PT3H")]
    Pt3h,
}

impl RecentlyActive {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pt3h => "PT3H",
        }
    }
}

impl std::fmt::Display for RecentlyActive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for RecentlyActive {
    fn from(s: &str) -> Self {
        match s {
            "PT3H" => Self::Pt3h,
            _ => Self::default(),
        }
    }
}

/// CloudWatch ScanBy enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ScanBy {
    /// Default variant.
    #[default]
    TimestampAscending,
    TimestampDescending,
}

impl ScanBy {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::TimestampAscending => "TimestampAscending",
            Self::TimestampDescending => "TimestampDescending",
        }
    }
}

impl std::fmt::Display for ScanBy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ScanBy {
    fn from(s: &str) -> Self {
        match s {
            "TimestampAscending" => Self::TimestampAscending,
            "TimestampDescending" => Self::TimestampDescending,
            _ => Self::default(),
        }
    }
}

/// CloudWatch StandardUnit enum.
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

/// CloudWatch StateValue enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum StateValue {
    /// Default variant.
    #[default]
    #[serde(rename = "ALARM")]
    Alarm,
    #[serde(rename = "INSUFFICIENT_DATA")]
    InsufficientData,
    #[serde(rename = "OK")]
    Ok,
}

impl StateValue {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Alarm => "ALARM",
            Self::InsufficientData => "INSUFFICIENT_DATA",
            Self::Ok => "OK",
        }
    }
}

impl std::fmt::Display for StateValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for StateValue {
    fn from(s: &str) -> Self {
        match s {
            "ALARM" => Self::Alarm,
            "INSUFFICIENT_DATA" => Self::InsufficientData,
            "OK" => Self::Ok,
            _ => Self::default(),
        }
    }
}

/// CloudWatch Statistic enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Statistic {
    /// Default variant.
    #[default]
    Average,
    Maximum,
    Minimum,
    SampleCount,
    Sum,
}

impl Statistic {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Average => "Average",
            Self::Maximum => "Maximum",
            Self::Minimum => "Minimum",
            Self::SampleCount => "SampleCount",
            Self::Sum => "Sum",
        }
    }
}

impl std::fmt::Display for Statistic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for Statistic {
    fn from(s: &str) -> Self {
        match s {
            "Average" => Self::Average,
            "Maximum" => Self::Maximum,
            "Minimum" => Self::Minimum,
            "SampleCount" => Self::SampleCount,
            "Sum" => Self::Sum,
            _ => Self::default(),
        }
    }
}

/// CloudWatch StatusCode enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum StatusCode {
    /// Default variant.
    #[default]
    Complete,
    Forbidden,
    InternalError,
    PartialData,
}

impl StatusCode {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Complete => "Complete",
            Self::Forbidden => "Forbidden",
            Self::InternalError => "InternalError",
            Self::PartialData => "PartialData",
        }
    }
}

impl std::fmt::Display for StatusCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for StatusCode {
    fn from(s: &str) -> Self {
        match s {
            "Complete" => Self::Complete,
            "Forbidden" => Self::Forbidden,
            "InternalError" => Self::InternalError,
            "PartialData" => Self::PartialData,
            _ => Self::default(),
        }
    }
}

/// CloudWatch AlarmHistoryItem.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AlarmHistoryItem {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub alarm_contributor_attributes: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alarm_contributor_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alarm_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alarm_type: Option<AlarmType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_item_type: Option<HistoryItemType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

/// CloudWatch AnomalyDetector.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AnomalyDetector {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configuration: Option<AnomalyDetectorConfiguration>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dimensions: Vec<Dimension>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric_characteristics: Option<MetricCharacteristics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric_math_anomaly_detector: Option<MetricMathAnomalyDetector>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub single_metric_anomaly_detector: Option<SingleMetricAnomalyDetector>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stat: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_value: Option<AnomalyDetectorStateValue>,
}

/// CloudWatch AnomalyDetectorConfiguration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AnomalyDetectorConfiguration {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excluded_time_ranges: Vec<Range>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric_timezone: Option<String>,
}

/// CloudWatch CompositeAlarm.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CompositeAlarm {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions_suppressed_by: Option<ActionsSuppressedBy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions_suppressed_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions_suppressor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions_suppressor_extension_period: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions_suppressor_wait_period: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alarm_actions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alarm_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alarm_configuration_updated_timestamp: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alarm_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alarm_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alarm_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub insufficient_data_actions: Vec<String>,
    #[serde(rename = "OKActions")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ok_actions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_reason_data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_transitioned_timestamp: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_updated_timestamp: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_value: Option<StateValue>,
}

/// CloudWatch DashboardEntry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DashboardEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dashboard_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dashboard_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<i64>,
}

/// CloudWatch DashboardValidationMessage.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DashboardValidationMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// CloudWatch Datapoint.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Datapoint {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average: Option<f64>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub extended_statistics: HashMap<String, f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample_count: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sum: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<StandardUnit>,
}

/// CloudWatch Dimension.
#[derive(Debug, Clone, Default, Hash, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Dimension {
    /// Dimension name.
    pub name: String,
    /// Dimension value.
    pub value: String,
}

/// CloudWatch DimensionFilter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DimensionFilter {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

/// CloudWatch Entity.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Entity {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub attributes: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub key_attributes: HashMap<String, String>,
}

/// CloudWatch EntityMetricData.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EntityMetricData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity: Option<Entity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metric_data: Vec<MetricDatum>,
}

/// CloudWatch InsightRule.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct InsightRule {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apply_on_transformed_logs: Option<bool>,
    pub definition: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managed_rule: Option<bool>,
    pub name: String,
    pub schema: String,
    pub state: String,
}

/// CloudWatch LabelOptions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LabelOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
}

/// CloudWatch ManagedRule.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ManagedRule {
    #[serde(rename = "ResourceARN")]
    pub resource_arn: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Tag>,
    pub template_name: String,
}

/// CloudWatch MessageData.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MessageData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

/// CloudWatch Metric.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Metric {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dimensions: Vec<Dimension>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

/// CloudWatch MetricAlarm.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MetricAlarm {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alarm_actions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alarm_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alarm_configuration_updated_timestamp: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alarm_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alarm_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comparison_operator: Option<ComparisonOperator>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub datapoints_to_alarm: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dimensions: Vec<Dimension>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evaluate_low_sample_count_percentile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evaluation_periods: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evaluation_state: Option<EvaluationState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extended_statistic: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub insufficient_data_actions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metrics: Vec<MetricDataQuery>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(rename = "OKActions")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ok_actions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_reason_data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_transitioned_timestamp: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_updated_timestamp: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_value: Option<StateValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub statistic: Option<Statistic>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold_metric_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub treat_missing_data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<StandardUnit>,
}

/// CloudWatch MetricCharacteristics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MetricCharacteristics {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub periodic_spikes: Option<bool>,
}

/// CloudWatch MetricDataQuery.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MetricDataQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric_stat: Option<MetricStat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_data: Option<bool>,
}

/// CloudWatch MetricDataResult.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MetricDataResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<MessageData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_code: Option<StatusCode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub timestamps: Vec<chrono::DateTime<chrono::Utc>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<f64>,
}

/// CloudWatch MetricDatum.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MetricDatum {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub counts: Vec<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dimensions: Vec<Dimension>,
    pub metric_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub statistic_values: Option<StatisticSet>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_resolution: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<StandardUnit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<f64>,
}

/// CloudWatch MetricMathAnomalyDetector.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MetricMathAnomalyDetector {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metric_data_queries: Vec<MetricDataQuery>,
}

/// CloudWatch MetricStat.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MetricStat {
    pub metric: Metric,
    pub period: i32,
    pub stat: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<StandardUnit>,
}

/// CloudWatch MetricStreamEntry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MetricStreamEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creation_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firehose_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_update_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_format: Option<MetricStreamOutputFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

/// CloudWatch MetricStreamFilter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MetricStreamFilter {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metric_names: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

/// CloudWatch MetricStreamStatisticsConfiguration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MetricStreamStatisticsConfiguration {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_statistics: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include_metrics: Vec<MetricStreamStatisticsMetric>,
}

/// CloudWatch MetricStreamStatisticsMetric.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MetricStreamStatisticsMetric {
    pub metric_name: String,
    pub namespace: String,
}

/// CloudWatch PartialFailure.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PartialFailure {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exception_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_resource: Option<String>,
}

/// CloudWatch Range.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Range {
    pub end_time: chrono::DateTime<chrono::Utc>,
    pub start_time: chrono::DateTime<chrono::Utc>,
}

/// CloudWatch SingleMetricAnomalyDetector.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SingleMetricAnomalyDetector {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dimensions: Vec<Dimension>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stat: Option<String>,
}

/// CloudWatch StatisticSet.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct StatisticSet {
    pub maximum: f64,
    pub minimum: f64,
    pub sample_count: f64,
    pub sum: f64,
}

/// CloudWatch Tag.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Tag {
    pub key: String,
    pub value: String,
}
