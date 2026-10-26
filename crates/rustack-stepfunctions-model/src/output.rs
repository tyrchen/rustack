//! Auto-generated from AWS Step Functions Smithy model. DO NOT EDIT.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::types::{
    ActivityListItem, BillingDetails, CloudWatchEventsExecutionDataDetails,
    EncryptionConfiguration, ExecutionListItem, ExecutionRedriveStatus, ExecutionStatus,
    HistoryEvent, InspectionData, LoggingConfiguration, MapRunExecutionCounts, MapRunItemCounts,
    MapRunListItem, MapRunStatus, RoutingConfigurationListItem, StateMachineAliasListItem,
    StateMachineListItem, StateMachineStatus, StateMachineType, StateMachineVersionListItem,
    SyncExecutionStatus, Tag, TestExecutionStatus, TracingConfiguration,
    ValidateStateMachineDefinitionDiagnostic, ValidateStateMachineDefinitionResultCode,
};

/// Step Functions `CreateActivityOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct CreateActivityOutput {
    /// The `activityArn` wire member.
    pub activity_arn: String,
    /// The `creationDate` wire member.
    #[serde(with = "crate::epoch_seconds")]
    pub creation_date: chrono::DateTime<chrono::Utc>,
}

impl std::fmt::Debug for CreateActivityOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreateActivityOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `CreateStateMachineAliasOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct CreateStateMachineAliasOutput {
    /// The `creationDate` wire member.
    #[serde(with = "crate::epoch_seconds")]
    pub creation_date: chrono::DateTime<chrono::Utc>,
    /// The `stateMachineAliasArn` wire member.
    pub state_machine_alias_arn: String,
}

impl std::fmt::Debug for CreateStateMachineAliasOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreateStateMachineAliasOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `CreateStateMachineOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct CreateStateMachineOutput {
    /// The `creationDate` wire member.
    #[serde(with = "crate::epoch_seconds")]
    pub creation_date: chrono::DateTime<chrono::Utc>,
    /// The `stateMachineArn` wire member.
    pub state_machine_arn: String,
    /// The `stateMachineVersionArn` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_machine_version_arn: Option<String>,
}

impl std::fmt::Debug for CreateStateMachineOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreateStateMachineOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `DeleteActivityOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct DeleteActivityOutput {}

impl std::fmt::Debug for DeleteActivityOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeleteActivityOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `DeleteStateMachineAliasOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct DeleteStateMachineAliasOutput {}

impl std::fmt::Debug for DeleteStateMachineAliasOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeleteStateMachineAliasOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `DeleteStateMachineOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct DeleteStateMachineOutput {}

impl std::fmt::Debug for DeleteStateMachineOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeleteStateMachineOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `DeleteStateMachineVersionOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct DeleteStateMachineVersionOutput {}

impl std::fmt::Debug for DeleteStateMachineVersionOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeleteStateMachineVersionOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `DescribeActivityOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct DescribeActivityOutput {
    /// The `activityArn` wire member.
    pub activity_arn: String,
    /// The `creationDate` wire member.
    #[serde(with = "crate::epoch_seconds")]
    pub creation_date: chrono::DateTime<chrono::Utc>,
    /// The `encryptionConfiguration` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryption_configuration: Option<EncryptionConfiguration>,
    /// The `name` wire member.
    pub name: String,
}

impl std::fmt::Debug for DescribeActivityOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DescribeActivityOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `DescribeExecutionOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
#[derive(typed_builder::TypedBuilder)]
pub struct DescribeExecutionOutput {
    /// The `cause` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// The `error` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The `executionArn` wire member.
    pub execution_arn: String,
    /// The `input` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// The `inputDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_details: Option<CloudWatchEventsExecutionDataDetails>,
    /// The `mapRunArn` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map_run_arn: Option<String>,
    /// The `name` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The `output` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// The `outputDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_details: Option<CloudWatchEventsExecutionDataDetails>,
    /// The `redriveCount` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redrive_count: Option<i32>,
    /// The `redriveDate` wire member.
    #[builder(default)]
    #[serde(default, with = "crate::epoch_seconds::option")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redrive_date: Option<chrono::DateTime<chrono::Utc>>,
    /// The `redriveStatus` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redrive_status: Option<ExecutionRedriveStatus>,
    /// The `redriveStatusReason` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redrive_status_reason: Option<String>,
    /// The `startDate` wire member.
    #[serde(with = "crate::epoch_seconds")]
    pub start_date: chrono::DateTime<chrono::Utc>,
    /// The `stateMachineAliasArn` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_machine_alias_arn: Option<String>,
    /// The `stateMachineArn` wire member.
    pub state_machine_arn: String,
    /// The `stateMachineVersionArn` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_machine_version_arn: Option<String>,
    /// The `status` wire member.
    pub status: ExecutionStatus,
    /// The `stopDate` wire member.
    #[builder(default)]
    #[serde(default, with = "crate::epoch_seconds::option")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_date: Option<chrono::DateTime<chrono::Utc>>,
    /// The `traceHeader` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_header: Option<String>,
}

impl std::fmt::Debug for DescribeExecutionOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DescribeExecutionOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `DescribeMapRunOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
#[derive(typed_builder::TypedBuilder)]
pub struct DescribeMapRunOutput {
    /// The `executionArn` wire member.
    pub execution_arn: String,
    /// The `executionCounts` wire member.
    pub execution_counts: MapRunExecutionCounts,
    /// The `itemCounts` wire member.
    pub item_counts: MapRunItemCounts,
    /// The `mapRunArn` wire member.
    pub map_run_arn: String,
    /// The `maxConcurrency` wire member.
    pub max_concurrency: i32,
    /// The `redriveCount` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redrive_count: Option<i32>,
    /// The `redriveDate` wire member.
    #[builder(default)]
    #[serde(default, with = "crate::epoch_seconds::option")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redrive_date: Option<chrono::DateTime<chrono::Utc>>,
    /// The `startDate` wire member.
    #[serde(with = "crate::epoch_seconds")]
    pub start_date: chrono::DateTime<chrono::Utc>,
    /// The `status` wire member.
    pub status: MapRunStatus,
    /// The `stopDate` wire member.
    #[builder(default)]
    #[serde(default, with = "crate::epoch_seconds::option")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_date: Option<chrono::DateTime<chrono::Utc>>,
    /// The `toleratedFailureCount` wire member.
    pub tolerated_failure_count: i64,
    /// The `toleratedFailurePercentage` wire member.
    pub tolerated_failure_percentage: f32,
}

impl std::fmt::Debug for DescribeMapRunOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DescribeMapRunOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `DescribeStateMachineAliasOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
#[derive(typed_builder::TypedBuilder)]
pub struct DescribeStateMachineAliasOutput {
    /// The `creationDate` wire member.
    #[builder(default)]
    #[serde(default, with = "crate::epoch_seconds::option")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creation_date: Option<chrono::DateTime<chrono::Utc>>,
    /// The `description` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The `name` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The `routingConfiguration` wire member.
    #[builder(default)]
    #[serde(default)]
    pub routing_configuration: Vec<RoutingConfigurationListItem>,
    /// The `stateMachineAliasArn` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_machine_alias_arn: Option<String>,
    /// The `updateDate` wire member.
    #[builder(default)]
    #[serde(default, with = "crate::epoch_seconds::option")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_date: Option<chrono::DateTime<chrono::Utc>>,
}

impl std::fmt::Debug for DescribeStateMachineAliasOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DescribeStateMachineAliasOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `DescribeStateMachineForExecutionOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
#[derive(typed_builder::TypedBuilder)]
pub struct DescribeStateMachineForExecutionOutput {
    /// The `definition` wire member.
    pub definition: String,
    /// The `encryptionConfiguration` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryption_configuration: Option<EncryptionConfiguration>,
    /// The `label` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The `loggingConfiguration` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logging_configuration: Option<LoggingConfiguration>,
    /// The `mapRunArn` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map_run_arn: Option<String>,
    /// The `name` wire member.
    pub name: String,
    /// The `revisionId` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
    /// The `roleArn` wire member.
    pub role_arn: String,
    /// The `stateMachineArn` wire member.
    pub state_machine_arn: String,
    /// The `tracingConfiguration` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracing_configuration: Option<TracingConfiguration>,
    /// The `updateDate` wire member.
    #[serde(with = "crate::epoch_seconds")]
    pub update_date: chrono::DateTime<chrono::Utc>,
    /// The `variableReferences` wire member.
    #[builder(default)]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub variable_references: HashMap<String, Vec<String>>,
}

impl std::fmt::Debug for DescribeStateMachineForExecutionOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DescribeStateMachineForExecutionOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `DescribeStateMachineOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
#[derive(typed_builder::TypedBuilder)]
pub struct DescribeStateMachineOutput {
    /// The `creationDate` wire member.
    #[serde(with = "crate::epoch_seconds")]
    pub creation_date: chrono::DateTime<chrono::Utc>,
    /// The `definition` wire member.
    pub definition: String,
    /// The `description` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The `encryptionConfiguration` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryption_configuration: Option<EncryptionConfiguration>,
    /// The `label` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The `loggingConfiguration` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logging_configuration: Option<LoggingConfiguration>,
    /// The `name` wire member.
    pub name: String,
    /// The `revisionId` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
    /// The `roleArn` wire member.
    pub role_arn: String,
    /// The `stateMachineArn` wire member.
    pub state_machine_arn: String,
    /// The `status` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<StateMachineStatus>,
    /// The `tracingConfiguration` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracing_configuration: Option<TracingConfiguration>,
    /// The `type` wire member.
    pub r#type: StateMachineType,
    /// The `variableReferences` wire member.
    #[builder(default)]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub variable_references: HashMap<String, Vec<String>>,
}

impl std::fmt::Debug for DescribeStateMachineOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DescribeStateMachineOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `GetActivityTaskOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct GetActivityTaskOutput {
    /// The `input` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// The `taskToken` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_token: Option<String>,
}

impl std::fmt::Debug for GetActivityTaskOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GetActivityTaskOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `GetExecutionHistoryOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct GetExecutionHistoryOutput {
    /// The `events` wire member.
    pub events: Vec<HistoryEvent>,
    /// The `nextToken` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

impl std::fmt::Debug for GetExecutionHistoryOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GetExecutionHistoryOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `ListActivitiesOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ListActivitiesOutput {
    /// The `activities` wire member.
    pub activities: Vec<ActivityListItem>,
    /// The `nextToken` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

impl std::fmt::Debug for ListActivitiesOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListActivitiesOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `ListExecutionsOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ListExecutionsOutput {
    /// The `executions` wire member.
    pub executions: Vec<ExecutionListItem>,
    /// The `nextToken` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

impl std::fmt::Debug for ListExecutionsOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListExecutionsOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `ListMapRunsOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ListMapRunsOutput {
    /// The `mapRuns` wire member.
    pub map_runs: Vec<MapRunListItem>,
    /// The `nextToken` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

impl std::fmt::Debug for ListMapRunsOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListMapRunsOutput").finish_non_exhaustive()
    }
}
/// Step Functions `ListStateMachineAliasesOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ListStateMachineAliasesOutput {
    /// The `nextToken` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    /// The `stateMachineAliases` wire member.
    pub state_machine_aliases: Vec<StateMachineAliasListItem>,
}

impl std::fmt::Debug for ListStateMachineAliasesOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListStateMachineAliasesOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `ListStateMachineVersionsOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ListStateMachineVersionsOutput {
    /// The `nextToken` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    /// The `stateMachineVersions` wire member.
    pub state_machine_versions: Vec<StateMachineVersionListItem>,
}

impl std::fmt::Debug for ListStateMachineVersionsOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListStateMachineVersionsOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `ListStateMachinesOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ListStateMachinesOutput {
    /// The `nextToken` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    /// The `stateMachines` wire member.
    pub state_machines: Vec<StateMachineListItem>,
}

impl std::fmt::Debug for ListStateMachinesOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListStateMachinesOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `ListTagsForResourceOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ListTagsForResourceOutput {
    /// The `tags` wire member.
    #[serde(default)]
    pub tags: Vec<Tag>,
}

impl std::fmt::Debug for ListTagsForResourceOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListTagsForResourceOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `PublishStateMachineVersionOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct PublishStateMachineVersionOutput {
    /// The `creationDate` wire member.
    #[serde(with = "crate::epoch_seconds")]
    pub creation_date: chrono::DateTime<chrono::Utc>,
    /// The `stateMachineVersionArn` wire member.
    pub state_machine_version_arn: String,
}

impl std::fmt::Debug for PublishStateMachineVersionOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PublishStateMachineVersionOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `RedriveExecutionOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct RedriveExecutionOutput {
    /// The `redriveDate` wire member.
    #[serde(with = "crate::epoch_seconds")]
    pub redrive_date: chrono::DateTime<chrono::Utc>,
}

impl std::fmt::Debug for RedriveExecutionOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedriveExecutionOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `SendTaskFailureOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct SendTaskFailureOutput {}

impl std::fmt::Debug for SendTaskFailureOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SendTaskFailureOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `SendTaskHeartbeatOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct SendTaskHeartbeatOutput {}

impl std::fmt::Debug for SendTaskHeartbeatOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SendTaskHeartbeatOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `SendTaskSuccessOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct SendTaskSuccessOutput {}

impl std::fmt::Debug for SendTaskSuccessOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SendTaskSuccessOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `StartExecutionOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct StartExecutionOutput {
    /// The `executionArn` wire member.
    pub execution_arn: String,
    /// The `startDate` wire member.
    #[serde(with = "crate::epoch_seconds")]
    pub start_date: chrono::DateTime<chrono::Utc>,
}

impl std::fmt::Debug for StartExecutionOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StartExecutionOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `StartSyncExecutionOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
#[derive(typed_builder::TypedBuilder)]
pub struct StartSyncExecutionOutput {
    /// The `billingDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_details: Option<BillingDetails>,
    /// The `cause` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// The `error` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The `executionArn` wire member.
    pub execution_arn: String,
    /// The `input` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// The `inputDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_details: Option<CloudWatchEventsExecutionDataDetails>,
    /// The `name` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The `output` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// The `outputDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_details: Option<CloudWatchEventsExecutionDataDetails>,
    /// The `startDate` wire member.
    #[serde(with = "crate::epoch_seconds")]
    pub start_date: chrono::DateTime<chrono::Utc>,
    /// The `stateMachineArn` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_machine_arn: Option<String>,
    /// The `status` wire member.
    pub status: SyncExecutionStatus,
    /// The `stopDate` wire member.
    #[serde(with = "crate::epoch_seconds")]
    pub stop_date: chrono::DateTime<chrono::Utc>,
    /// The `traceHeader` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_header: Option<String>,
}

impl std::fmt::Debug for StartSyncExecutionOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StartSyncExecutionOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `StopExecutionOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct StopExecutionOutput {
    /// The `stopDate` wire member.
    #[serde(with = "crate::epoch_seconds")]
    pub stop_date: chrono::DateTime<chrono::Utc>,
}

impl std::fmt::Debug for StopExecutionOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StopExecutionOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `TagResourceOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct TagResourceOutput {}

impl std::fmt::Debug for TagResourceOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TagResourceOutput").finish_non_exhaustive()
    }
}
/// Step Functions `TestStateOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
#[derive(typed_builder::TypedBuilder)]
pub struct TestStateOutput {
    /// The `cause` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// The `error` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The `inspectionData` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inspection_data: Option<InspectionData>,
    /// The `nextState` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_state: Option<String>,
    /// The `output` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// The `status` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<TestExecutionStatus>,
}

impl std::fmt::Debug for TestStateOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestStateOutput").finish_non_exhaustive()
    }
}
/// Step Functions `UntagResourceOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct UntagResourceOutput {}

impl std::fmt::Debug for UntagResourceOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UntagResourceOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `UpdateMapRunOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct UpdateMapRunOutput {}

impl std::fmt::Debug for UpdateMapRunOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateMapRunOutput").finish_non_exhaustive()
    }
}
/// Step Functions `UpdateStateMachineAliasOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct UpdateStateMachineAliasOutput {
    /// The `updateDate` wire member.
    #[serde(with = "crate::epoch_seconds")]
    pub update_date: chrono::DateTime<chrono::Utc>,
}

impl std::fmt::Debug for UpdateStateMachineAliasOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateStateMachineAliasOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `UpdateStateMachineOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct UpdateStateMachineOutput {
    /// The `revisionId` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
    /// The `stateMachineVersionArn` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_machine_version_arn: Option<String>,
    /// The `updateDate` wire member.
    #[serde(with = "crate::epoch_seconds")]
    pub update_date: chrono::DateTime<chrono::Utc>,
}

impl std::fmt::Debug for UpdateStateMachineOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateStateMachineOutput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `ValidateStateMachineDefinitionOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ValidateStateMachineDefinitionOutput {
    /// The `diagnostics` wire member.
    pub diagnostics: Vec<ValidateStateMachineDefinitionDiagnostic>,
    /// The `result` wire member.
    pub result: ValidateStateMachineDefinitionResultCode,
    /// The `truncated` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
}

impl std::fmt::Debug for ValidateStateMachineDefinitionOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidateStateMachineDefinitionOutput")
            .finish_non_exhaustive()
    }
}
