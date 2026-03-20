//! Auto-generated from AWS CloudWatch Smithy model. DO NOT EDIT.

use serde::{Deserialize, Serialize};

use crate::types::{
    AlarmType, AnomalyDetectorConfiguration, AnomalyDetectorType, ComparisonOperator, Dimension,
    DimensionFilter, EntityMetricData, HistoryItemType, LabelOptions, ManagedRule,
    MetricCharacteristics, MetricDataQuery, MetricDatum, MetricMathAnomalyDetector,
    MetricStreamFilter, MetricStreamOutputFormat, MetricStreamStatisticsConfiguration,
    RecentlyActive, ScanBy, SingleMetricAnomalyDetector, StandardUnit, StateValue, Statistic, Tag,
};

/// CloudWatch DeleteAlarmsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteAlarmsInput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alarm_names: Vec<String>,
}

/// CloudWatch DeleteAnomalyDetectorInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteAnomalyDetectorInput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dimensions: Vec<Dimension>,
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
}

/// CloudWatch DeleteDashboardsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteDashboardsInput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dashboard_names: Vec<String>,
}

/// CloudWatch DeleteInsightRulesInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteInsightRulesInput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rule_names: Vec<String>,
}

/// CloudWatch DeleteMetricStreamInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteMetricStreamInput {
    pub name: String,
}

/// CloudWatch DescribeAlarmHistoryInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeAlarmHistoryInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alarm_contributor_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alarm_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alarm_types: Vec<AlarmType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_item_type: Option<HistoryItemType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_records: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scan_by: Option<ScanBy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_date: Option<chrono::DateTime<chrono::Utc>>,
}

/// CloudWatch DescribeAlarmsForMetricInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeAlarmsForMetricInput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dimensions: Vec<Dimension>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extended_statistic: Option<String>,
    pub metric_name: String,
    pub namespace: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub statistic: Option<Statistic>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<StandardUnit>,
}

/// CloudWatch DescribeAlarmsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeAlarmsInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alarm_name_prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alarm_names: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alarm_types: Vec<AlarmType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children_of_alarm_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_records: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parents_of_alarm_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_value: Option<StateValue>,
}

/// CloudWatch DescribeAnomalyDetectorsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeAnomalyDetectorsInput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anomaly_detector_types: Vec<AnomalyDetectorType>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dimensions: Vec<Dimension>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// CloudWatch DescribeInsightRulesInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeInsightRulesInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// CloudWatch DisableAlarmActionsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DisableAlarmActionsInput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alarm_names: Vec<String>,
}

/// CloudWatch EnableAlarmActionsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EnableAlarmActionsInput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alarm_names: Vec<String>,
}

/// CloudWatch GetDashboardInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetDashboardInput {
    pub dashboard_name: String,
}

/// CloudWatch GetMetricDataInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetMetricDataInput {
    pub end_time: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_options: Option<LabelOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_datapoints: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metric_data_queries: Vec<MetricDataQuery>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scan_by: Option<ScanBy>,
    pub start_time: chrono::DateTime<chrono::Utc>,
}

/// CloudWatch GetMetricStatisticsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetMetricStatisticsInput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dimensions: Vec<Dimension>,
    pub end_time: chrono::DateTime<chrono::Utc>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extended_statistics: Vec<String>,
    pub metric_name: String,
    pub namespace: String,
    pub period: i32,
    pub start_time: chrono::DateTime<chrono::Utc>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub statistics: Vec<Statistic>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<StandardUnit>,
}

/// CloudWatch GetMetricStreamInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetMetricStreamInput {
    pub name: String,
}

/// CloudWatch ListDashboardsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListDashboardsInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dashboard_name_prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// CloudWatch ListMetricStreamsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListMetricStreamsInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// CloudWatch ListMetricsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListMetricsInput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dimensions: Vec<DimensionFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_linked_accounts: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owning_account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recently_active: Option<RecentlyActive>,
}

/// CloudWatch ListTagsForResourceInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListTagsForResourceInput {
    #[serde(rename = "ResourceARN")]
    pub resource_arn: String,
}

/// CloudWatch PutAnomalyDetectorInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutAnomalyDetectorInput {
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
}

/// CloudWatch PutCompositeAlarmInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutCompositeAlarmInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions_suppressor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions_suppressor_extension_period: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions_suppressor_wait_period: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alarm_actions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alarm_description: Option<String>,
    pub alarm_name: String,
    pub alarm_rule: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub insufficient_data_actions: Vec<String>,
    #[serde(rename = "OKActions")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ok_actions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Tag>,
}

/// CloudWatch PutDashboardInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutDashboardInput {
    pub dashboard_body: String,
    pub dashboard_name: String,
}

/// CloudWatch PutInsightRuleInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutInsightRuleInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apply_on_transformed_logs: Option<bool>,
    pub rule_definition: String,
    pub rule_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_state: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Tag>,
}

/// CloudWatch PutManagedInsightRulesInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutManagedInsightRulesInput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub managed_rules: Vec<ManagedRule>,
}

/// CloudWatch PutMetricAlarmInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutMetricAlarmInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alarm_actions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alarm_description: Option<String>,
    pub alarm_name: String,
    pub comparison_operator: ComparisonOperator,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub datapoints_to_alarm: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dimensions: Vec<Dimension>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evaluate_low_sample_count_percentile: Option<String>,
    pub evaluation_periods: i32,
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
    pub statistic: Option<Statistic>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Tag>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold_metric_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub treat_missing_data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<StandardUnit>,
}

/// CloudWatch PutMetricDataInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutMetricDataInput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entity_metric_data: Vec<EntityMetricData>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metric_data: Vec<MetricDatum>,
    pub namespace: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict_entity_validation: Option<bool>,
}

/// CloudWatch PutMetricStreamInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutMetricStreamInput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_filters: Vec<MetricStreamFilter>,
    pub firehose_arn: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include_filters: Vec<MetricStreamFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_linked_accounts_metrics: Option<bool>,
    pub name: String,
    pub output_format: MetricStreamOutputFormat,
    pub role_arn: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub statistics_configurations: Vec<MetricStreamStatisticsConfiguration>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Tag>,
}

/// CloudWatch SetAlarmStateInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SetAlarmStateInput {
    pub alarm_name: String,
    pub state_reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_reason_data: Option<String>,
    pub state_value: StateValue,
}

/// CloudWatch TagResourceInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TagResourceInput {
    #[serde(rename = "ResourceARN")]
    pub resource_arn: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Tag>,
}

/// CloudWatch UntagResourceInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UntagResourceInput {
    #[serde(rename = "ResourceARN")]
    pub resource_arn: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tag_keys: Vec<String>,
}
