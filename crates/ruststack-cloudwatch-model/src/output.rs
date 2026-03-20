//! Auto-generated from AWS CloudWatch Smithy model. DO NOT EDIT.

use serde::{Deserialize, Serialize};

use crate::types::{
    AlarmHistoryItem, AnomalyDetector, CompositeAlarm, DashboardEntry, DashboardValidationMessage,
    Datapoint, InsightRule, MessageData, Metric, MetricAlarm, MetricDataResult, MetricStreamEntry,
    MetricStreamFilter, MetricStreamOutputFormat, MetricStreamStatisticsConfiguration,
    PartialFailure, Tag,
};

/// CloudWatch DeleteAnomalyDetectorOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteAnomalyDetectorOutput {}

/// CloudWatch DeleteDashboardsOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteDashboardsOutput {}

/// CloudWatch DeleteInsightRulesOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteInsightRulesOutput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<PartialFailure>,
}

/// CloudWatch DeleteMetricStreamOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteMetricStreamOutput {}

/// CloudWatch DescribeAlarmHistoryOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeAlarmHistoryOutput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alarm_history_items: Vec<AlarmHistoryItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// CloudWatch DescribeAlarmsForMetricOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeAlarmsForMetricOutput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metric_alarms: Vec<MetricAlarm>,
}

/// CloudWatch DescribeAlarmsOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeAlarmsOutput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub composite_alarms: Vec<CompositeAlarm>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metric_alarms: Vec<MetricAlarm>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// CloudWatch DescribeAnomalyDetectorsOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeAnomalyDetectorsOutput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anomaly_detectors: Vec<AnomalyDetector>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// CloudWatch DescribeInsightRulesOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeInsightRulesOutput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub insight_rules: Vec<InsightRule>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// CloudWatch GetDashboardOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetDashboardOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dashboard_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dashboard_body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dashboard_name: Option<String>,
}

/// CloudWatch GetMetricDataOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetMetricDataOutput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<MessageData>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metric_data_results: Vec<MetricDataResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// CloudWatch GetMetricStatisticsOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetMetricStatisticsOutput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub datapoints: Vec<Datapoint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// CloudWatch GetMetricStreamOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetMetricStreamOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creation_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_filters: Vec<MetricStreamFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firehose_arn: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include_filters: Vec<MetricStreamFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_linked_accounts_metrics: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_update_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_format: Option<MetricStreamOutputFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub statistics_configurations: Vec<MetricStreamStatisticsConfiguration>,
}

/// CloudWatch ListDashboardsOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListDashboardsOutput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dashboard_entries: Vec<DashboardEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// CloudWatch ListMetricStreamsOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListMetricStreamsOutput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<MetricStreamEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// CloudWatch ListMetricsOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListMetricsOutput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metrics: Vec<Metric>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub owning_accounts: Vec<String>,
}

/// CloudWatch ListTagsForResourceOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListTagsForResourceOutput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Tag>,
}

/// CloudWatch PutAnomalyDetectorOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutAnomalyDetectorOutput {}

/// CloudWatch PutDashboardOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutDashboardOutput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dashboard_validation_messages: Vec<DashboardValidationMessage>,
}

/// CloudWatch PutInsightRuleOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutInsightRuleOutput {}

/// CloudWatch PutManagedInsightRulesOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutManagedInsightRulesOutput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<PartialFailure>,
}

/// CloudWatch PutMetricStreamOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutMetricStreamOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
}

/// CloudWatch TagResourceOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TagResourceOutput {}

/// CloudWatch UntagResourceOutput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UntagResourceOutput {}
