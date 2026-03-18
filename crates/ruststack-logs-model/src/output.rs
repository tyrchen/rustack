//! Auto-generated from AWS CloudWatch Logs Smithy model. DO NOT EDIT.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::types::{
    Destination, ExportTask, FilteredLogEvent, LogGroup, LogStream, MetricFilter,
    MetricFilterMatchRecord, OutputLogEvent, QueryDefinition, QueryInfo, QueryLanguage,
    QueryStatistics, QueryStatus, RejectedEntityInfo, RejectedLogEventsInfo, ResourcePolicy,
    ResultField, SearchedLogStream, SubscriptionFilter,
};

/// CloudWatch Logs CreateExportTaskResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateExportTaskResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

/// CloudWatch Logs DeleteQueryDefinitionResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteQueryDefinitionResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
}

/// CloudWatch Logs DescribeDestinationsResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribeDestinationsResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub destinations: Vec<Destination>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// CloudWatch Logs DescribeExportTasksResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribeExportTasksResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub export_tasks: Vec<ExportTask>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// CloudWatch Logs DescribeLogGroupsResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribeLogGroupsResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub log_groups: Vec<LogGroup>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// CloudWatch Logs DescribeLogStreamsResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribeLogStreamsResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub log_streams: Vec<LogStream>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// CloudWatch Logs DescribeMetricFiltersResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribeMetricFiltersResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metric_filters: Vec<MetricFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// CloudWatch Logs DescribeQueriesResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribeQueriesResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queries: Vec<QueryInfo>,
}

/// CloudWatch Logs DescribeQueryDefinitionsResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribeQueryDefinitionsResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub query_definitions: Vec<QueryDefinition>,
}

/// CloudWatch Logs DescribeResourcePoliciesResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribeResourcePoliciesResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resource_policies: Vec<ResourcePolicy>,
}

/// CloudWatch Logs DescribeSubscriptionFiltersResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribeSubscriptionFiltersResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subscription_filters: Vec<SubscriptionFilter>,
}

/// CloudWatch Logs FilterLogEventsResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterLogEventsResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<FilteredLogEvent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub searched_log_streams: Vec<SearchedLogStream>,
}

/// CloudWatch Logs GetLogEventsResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetLogEventsResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<OutputLogEvent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_backward_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_forward_token: Option<String>,
}

/// CloudWatch Logs GetQueryResultsResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetQueryResultsResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryption_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_language: Option<QueryLanguage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub results: Vec<Vec<ResultField>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub statistics: Option<QueryStatistics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<QueryStatus>,
}

/// CloudWatch Logs ListTagsForResourceResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTagsForResourceResponse {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
}

/// CloudWatch Logs ListTagsLogGroupResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTagsLogGroupResponse {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
}

/// CloudWatch Logs PutDestinationResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PutDestinationResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<Destination>,
}

/// CloudWatch Logs PutLogEventsResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PutLogEventsResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_sequence_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejected_entity_info: Option<RejectedEntityInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejected_log_events_info: Option<RejectedLogEventsInfo>,
}

/// CloudWatch Logs PutQueryDefinitionResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PutQueryDefinitionResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_definition_id: Option<String>,
}

/// CloudWatch Logs PutResourcePolicyResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PutResourcePolicyResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_policy: Option<ResourcePolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
}

/// CloudWatch Logs StartQueryResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartQueryResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_id: Option<String>,
}

/// CloudWatch Logs StopQueryResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StopQueryResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
}

/// CloudWatch Logs TestMetricFilterResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestMetricFilterResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matches: Vec<MetricFilterMatchRecord>,
}
