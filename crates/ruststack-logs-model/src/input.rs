//! Auto-generated from AWS CloudWatch Logs Smithy model. DO NOT EDIT.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::types::{
    Distribution, Entity, ExportTaskStatusCode, InputLogEvent, LogGroupClass, MetricTransformation,
    OrderBy, PolicyScope, QueryLanguage, QueryStatus,
};

/// CloudWatch Logs AssociateKmsKeyInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssociateKmsKeyInput {
    pub kms_key_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_group_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_identifier: Option<String>,
}

/// CloudWatch Logs CancelExportTaskInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelExportTaskInput {
    pub task_id: String,
}

/// CloudWatch Logs CreateExportTaskInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateExportTaskInput {
    pub destination: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_prefix: Option<String>,
    pub from: i64,
    pub log_group_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_stream_name_prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_name: Option<String>,
    pub to: i64,
}

/// CloudWatch Logs CreateLogGroupInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateLogGroupInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deletion_protection_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kms_key_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_group_class: Option<LogGroupClass>,
    pub log_group_name: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
}

/// CloudWatch Logs CreateLogStreamInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateLogStreamInput {
    pub log_group_name: String,
    pub log_stream_name: String,
}

/// CloudWatch Logs DeleteDestinationInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteDestinationInput {
    pub destination_name: String,
}

/// CloudWatch Logs DeleteLogGroupInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteLogGroupInput {
    pub log_group_name: String,
}

/// CloudWatch Logs DeleteLogStreamInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteLogStreamInput {
    pub log_group_name: String,
    pub log_stream_name: String,
}

/// CloudWatch Logs DeleteMetricFilterInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteMetricFilterInput {
    pub filter_name: String,
    pub log_group_name: String,
}

/// CloudWatch Logs DeleteQueryDefinitionInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteQueryDefinitionInput {
    pub query_definition_id: String,
}

/// CloudWatch Logs DeleteResourcePolicyInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteResourcePolicyInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_revision_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_arn: Option<String>,
}

/// CloudWatch Logs DeleteRetentionPolicyInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteRetentionPolicyInput {
    pub log_group_name: String,
}

/// CloudWatch Logs DeleteSubscriptionFilterInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteSubscriptionFilterInput {
    pub filter_name: String,
    pub log_group_name: String,
}

/// CloudWatch Logs DescribeDestinationsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribeDestinationsInput {
    #[serde(rename = "DestinationNamePrefix")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_name_prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// CloudWatch Logs DescribeExportTasksInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribeExportTasksInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_code: Option<ExportTaskStatusCode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

/// CloudWatch Logs DescribeLogGroupsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribeLogGroupsInput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub account_identifiers: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_linked_accounts: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_group_class: Option<LogGroupClass>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub log_group_identifiers: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_group_name_pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_group_name_prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// CloudWatch Logs DescribeLogStreamsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribeLogStreamsInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub descending: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_group_identifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_group_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_stream_name_prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_by: Option<OrderBy>,
}

/// CloudWatch Logs DescribeMetricFiltersInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribeMetricFiltersInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter_name_prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_group_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric_namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// CloudWatch Logs DescribeQueriesInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribeQueriesInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_group_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_language: Option<QueryLanguage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<QueryStatus>,
}

/// CloudWatch Logs DescribeQueryDefinitionsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribeQueryDefinitionsInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_definition_name_prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_language: Option<QueryLanguage>,
}

/// CloudWatch Logs DescribeResourcePoliciesInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribeResourcePoliciesInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_scope: Option<PolicyScope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_arn: Option<String>,
}

/// CloudWatch Logs DescribeSubscriptionFiltersInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribeSubscriptionFiltersInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter_name_prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
    pub log_group_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// CloudWatch Logs DisassociateKmsKeyInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisassociateKmsKeyInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_group_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_identifier: Option<String>,
}

/// CloudWatch Logs FilterLogEventsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterLogEventsInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter_pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interleaved: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_group_identifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_group_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_stream_name_prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub log_stream_names: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unmask: Option<bool>,
}

/// CloudWatch Logs GetLogEventsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetLogEventsInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_group_identifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_group_name: Option<String>,
    pub log_stream_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_from_head: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unmask: Option<bool>,
}

/// CloudWatch Logs GetQueryResultsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetQueryResultsInput {
    pub query_id: String,
}

/// CloudWatch Logs ListTagsForResourceInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTagsForResourceInput {
    pub resource_arn: String,
}

/// CloudWatch Logs ListTagsLogGroupInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTagsLogGroupInput {
    pub log_group_name: String,
}

/// CloudWatch Logs PutDestinationInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PutDestinationInput {
    pub destination_name: String,
    pub role_arn: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
    pub target_arn: String,
}

/// CloudWatch Logs PutDestinationPolicyInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PutDestinationPolicyInput {
    pub access_policy: String,
    pub destination_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub force_update: Option<bool>,
}

/// CloudWatch Logs PutLogEventsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PutLogEventsInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity: Option<Entity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub log_events: Vec<InputLogEvent>,
    pub log_group_name: String,
    pub log_stream_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_token: Option<String>,
}

/// CloudWatch Logs PutMetricFilterInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PutMetricFilterInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apply_on_transformed_logs: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emit_system_field_dimensions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_selection_criteria: Option<String>,
    pub filter_name: String,
    pub filter_pattern: String,
    pub log_group_name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metric_transformations: Vec<MetricTransformation>,
}

/// CloudWatch Logs PutQueryDefinitionInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PutQueryDefinitionInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_token: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub log_group_names: Vec<String>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_definition_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_language: Option<QueryLanguage>,
    pub query_string: String,
}

/// CloudWatch Logs PutResourcePolicyInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PutResourcePolicyInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_revision_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_document: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_arn: Option<String>,
}

/// CloudWatch Logs PutRetentionPolicyInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PutRetentionPolicyInput {
    pub log_group_name: String,
    pub retention_in_days: i32,
}

/// CloudWatch Logs PutSubscriptionFilterInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PutSubscriptionFilterInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apply_on_transformed_logs: Option<bool>,
    pub destination_arn: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distribution: Option<Distribution>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emit_system_fields: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_selection_criteria: Option<String>,
    pub filter_name: String,
    pub filter_pattern: String,
    pub log_group_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_arn: Option<String>,
}

/// CloudWatch Logs StartQueryInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartQueryInput {
    pub end_time: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub log_group_identifiers: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_group_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub log_group_names: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_language: Option<QueryLanguage>,
    pub query_string: String,
    pub start_time: i64,
}

/// CloudWatch Logs StopQueryInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StopQueryInput {
    pub query_id: String,
}

/// CloudWatch Logs TagLogGroupInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagLogGroupInput {
    pub log_group_name: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
}

/// CloudWatch Logs TagResourceInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagResourceInput {
    pub resource_arn: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
}

/// CloudWatch Logs TestMetricFilterInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestMetricFilterInput {
    pub filter_pattern: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub log_event_messages: Vec<String>,
}

/// CloudWatch Logs UntagLogGroupInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UntagLogGroupInput {
    pub log_group_name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// CloudWatch Logs UntagResourceInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UntagResourceInput {
    pub resource_arn: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tag_keys: Vec<String>,
}
