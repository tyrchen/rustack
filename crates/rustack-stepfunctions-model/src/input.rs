//! Auto-generated from AWS Step Functions Smithy model. DO NOT EDIT.

use serde::{Deserialize, Serialize};

use crate::types::{
    EncryptionConfiguration, ExecutionRedriveFilter, ExecutionStatus, IncludedData,
    InspectionLevel, LoggingConfiguration, MockInput, RoutingConfigurationListItem,
    StateMachineType, Tag, TestStateConfiguration, TracingConfiguration,
    ValidateStateMachineDefinitionSeverity,
};

/// Step Functions `CreateActivityInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct CreateActivityInput {
    /// The `encryptionConfiguration` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryption_configuration: Option<EncryptionConfiguration>,
    /// The `name` wire member.
    pub name: String,
    /// The `tags` wire member.
    #[serde(default)]
    pub tags: Vec<Tag>,
}

impl std::fmt::Debug for CreateActivityInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreateActivityInput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `CreateStateMachineAliasInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct CreateStateMachineAliasInput {
    /// The `description` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The `name` wire member.
    pub name: String,
    /// The `routingConfiguration` wire member.
    pub routing_configuration: Vec<RoutingConfigurationListItem>,
}

impl std::fmt::Debug for CreateStateMachineAliasInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreateStateMachineAliasInput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `CreateStateMachineInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
#[derive(typed_builder::TypedBuilder)]
pub struct CreateStateMachineInput {
    /// The `definition` wire member.
    pub definition: String,
    /// The `encryptionConfiguration` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryption_configuration: Option<EncryptionConfiguration>,
    /// The `loggingConfiguration` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logging_configuration: Option<LoggingConfiguration>,
    /// The `name` wire member.
    pub name: String,
    /// The `publish` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publish: Option<bool>,
    /// The `roleArn` wire member.
    pub role_arn: String,
    /// The `tags` wire member.
    #[builder(default)]
    #[serde(default)]
    pub tags: Vec<Tag>,
    /// The `tracingConfiguration` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracing_configuration: Option<TracingConfiguration>,
    /// The `type` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<StateMachineType>,
    /// The `versionDescription` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_description: Option<String>,
}

impl std::fmt::Debug for CreateStateMachineInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreateStateMachineInput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `DeleteActivityInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct DeleteActivityInput {
    /// The `activityArn` wire member.
    pub activity_arn: String,
}

impl std::fmt::Debug for DeleteActivityInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeleteActivityInput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `DeleteStateMachineAliasInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct DeleteStateMachineAliasInput {
    /// The `stateMachineAliasArn` wire member.
    pub state_machine_alias_arn: String,
}

impl std::fmt::Debug for DeleteStateMachineAliasInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeleteStateMachineAliasInput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `DeleteStateMachineInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct DeleteStateMachineInput {
    /// The `stateMachineArn` wire member.
    pub state_machine_arn: String,
}

impl std::fmt::Debug for DeleteStateMachineInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeleteStateMachineInput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `DeleteStateMachineVersionInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct DeleteStateMachineVersionInput {
    /// The `stateMachineVersionArn` wire member.
    pub state_machine_version_arn: String,
}

impl std::fmt::Debug for DeleteStateMachineVersionInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeleteStateMachineVersionInput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `DescribeActivityInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct DescribeActivityInput {
    /// The `activityArn` wire member.
    pub activity_arn: String,
}

impl std::fmt::Debug for DescribeActivityInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DescribeActivityInput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `DescribeExecutionInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct DescribeExecutionInput {
    /// The `executionArn` wire member.
    pub execution_arn: String,
    /// The `includedData` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub included_data: Option<IncludedData>,
}

impl std::fmt::Debug for DescribeExecutionInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DescribeExecutionInput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `DescribeMapRunInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct DescribeMapRunInput {
    /// The `mapRunArn` wire member.
    pub map_run_arn: String,
}

impl std::fmt::Debug for DescribeMapRunInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DescribeMapRunInput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `DescribeStateMachineAliasInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct DescribeStateMachineAliasInput {
    /// The `stateMachineAliasArn` wire member.
    pub state_machine_alias_arn: String,
}

impl std::fmt::Debug for DescribeStateMachineAliasInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DescribeStateMachineAliasInput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `DescribeStateMachineForExecutionInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct DescribeStateMachineForExecutionInput {
    /// The `executionArn` wire member.
    pub execution_arn: String,
    /// The `includedData` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub included_data: Option<IncludedData>,
}

impl std::fmt::Debug for DescribeStateMachineForExecutionInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DescribeStateMachineForExecutionInput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `DescribeStateMachineInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct DescribeStateMachineInput {
    /// The `includedData` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub included_data: Option<IncludedData>,
    /// The `stateMachineArn` wire member.
    pub state_machine_arn: String,
}

impl std::fmt::Debug for DescribeStateMachineInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DescribeStateMachineInput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `GetActivityTaskInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct GetActivityTaskInput {
    /// The `activityArn` wire member.
    pub activity_arn: String,
    /// The `workerName` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_name: Option<String>,
}

impl std::fmt::Debug for GetActivityTaskInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GetActivityTaskInput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `GetExecutionHistoryInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct GetExecutionHistoryInput {
    /// The `executionArn` wire member.
    pub execution_arn: String,
    /// The `includeExecutionData` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_execution_data: Option<bool>,
    /// The `maxResults` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<i32>,
    /// The `nextToken` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    /// The `reverseOrder` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reverse_order: Option<bool>,
}

impl std::fmt::Debug for GetExecutionHistoryInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GetExecutionHistoryInput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `ListActivitiesInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ListActivitiesInput {
    /// The `maxResults` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<i32>,
    /// The `nextToken` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

impl std::fmt::Debug for ListActivitiesInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListActivitiesInput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `ListExecutionsInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
#[derive(typed_builder::TypedBuilder)]
pub struct ListExecutionsInput {
    /// The `mapRunArn` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map_run_arn: Option<String>,
    /// The `maxResults` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<i32>,
    /// The `nextToken` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    /// The `redriveFilter` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redrive_filter: Option<ExecutionRedriveFilter>,
    /// The `stateMachineArn` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_machine_arn: Option<String>,
    /// The `statusFilter` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_filter: Option<ExecutionStatus>,
}

impl std::fmt::Debug for ListExecutionsInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListExecutionsInput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `ListMapRunsInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ListMapRunsInput {
    /// The `executionArn` wire member.
    pub execution_arn: String,
    /// The `maxResults` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<i32>,
    /// The `nextToken` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

impl std::fmt::Debug for ListMapRunsInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListMapRunsInput").finish_non_exhaustive()
    }
}
/// Step Functions `ListStateMachineAliasesInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ListStateMachineAliasesInput {
    /// The `maxResults` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<i32>,
    /// The `nextToken` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    /// The `stateMachineArn` wire member.
    pub state_machine_arn: String,
}

impl std::fmt::Debug for ListStateMachineAliasesInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListStateMachineAliasesInput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `ListStateMachineVersionsInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ListStateMachineVersionsInput {
    /// The `maxResults` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<i32>,
    /// The `nextToken` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    /// The `stateMachineArn` wire member.
    pub state_machine_arn: String,
}

impl std::fmt::Debug for ListStateMachineVersionsInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListStateMachineVersionsInput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `ListStateMachinesInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ListStateMachinesInput {
    /// The `maxResults` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<i32>,
    /// The `nextToken` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

impl std::fmt::Debug for ListStateMachinesInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListStateMachinesInput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `ListTagsForResourceInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ListTagsForResourceInput {
    /// The `resourceArn` wire member.
    pub resource_arn: String,
}

impl std::fmt::Debug for ListTagsForResourceInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListTagsForResourceInput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `PublishStateMachineVersionInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct PublishStateMachineVersionInput {
    /// The `description` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The `revisionId` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
    /// The `stateMachineArn` wire member.
    pub state_machine_arn: String,
}

impl std::fmt::Debug for PublishStateMachineVersionInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PublishStateMachineVersionInput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `RedriveExecutionInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct RedriveExecutionInput {
    /// The `clientToken` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_token: Option<String>,
    /// The `executionArn` wire member.
    pub execution_arn: String,
}

impl std::fmt::Debug for RedriveExecutionInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedriveExecutionInput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `SendTaskFailureInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct SendTaskFailureInput {
    /// The `cause` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// The `error` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The `taskToken` wire member.
    pub task_token: String,
}

impl std::fmt::Debug for SendTaskFailureInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SendTaskFailureInput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `SendTaskHeartbeatInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct SendTaskHeartbeatInput {
    /// The `taskToken` wire member.
    pub task_token: String,
}

impl std::fmt::Debug for SendTaskHeartbeatInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SendTaskHeartbeatInput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `SendTaskSuccessInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct SendTaskSuccessInput {
    /// The `output` wire member.
    pub output: String,
    /// The `taskToken` wire member.
    pub task_token: String,
}

impl std::fmt::Debug for SendTaskSuccessInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SendTaskSuccessInput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `StartExecutionInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct StartExecutionInput {
    /// The `input` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// The `name` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The `stateMachineArn` wire member.
    pub state_machine_arn: String,
    /// The `traceHeader` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_header: Option<String>,
}

impl std::fmt::Debug for StartExecutionInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StartExecutionInput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `StartSyncExecutionInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct StartSyncExecutionInput {
    /// The `includedData` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub included_data: Option<IncludedData>,
    /// The `input` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// The `name` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The `stateMachineArn` wire member.
    pub state_machine_arn: String,
    /// The `traceHeader` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_header: Option<String>,
}

impl std::fmt::Debug for StartSyncExecutionInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StartSyncExecutionInput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `StopExecutionInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct StopExecutionInput {
    /// The `cause` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// The `error` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The `executionArn` wire member.
    pub execution_arn: String,
}

impl std::fmt::Debug for StopExecutionInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StopExecutionInput").finish_non_exhaustive()
    }
}
/// Step Functions `TagResourceInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct TagResourceInput {
    /// The `resourceArn` wire member.
    pub resource_arn: String,
    /// The `tags` wire member.
    pub tags: Vec<Tag>,
}

impl std::fmt::Debug for TagResourceInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TagResourceInput").finish_non_exhaustive()
    }
}
/// Step Functions `TestStateInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
#[derive(typed_builder::TypedBuilder)]
pub struct TestStateInput {
    /// The `context` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// The `definition` wire member.
    pub definition: String,
    /// The `input` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// The `inspectionLevel` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inspection_level: Option<InspectionLevel>,
    /// The `mock` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mock: Option<MockInput>,
    /// The `revealSecrets` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reveal_secrets: Option<bool>,
    /// The `roleArn` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_arn: Option<String>,
    /// The `stateConfiguration` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_configuration: Option<TestStateConfiguration>,
    /// The `stateName` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_name: Option<String>,
    /// The `variables` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variables: Option<String>,
}

impl std::fmt::Debug for TestStateInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestStateInput").finish_non_exhaustive()
    }
}
/// Step Functions `UntagResourceInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct UntagResourceInput {
    /// The `resourceArn` wire member.
    pub resource_arn: String,
    /// The `tagKeys` wire member.
    pub tag_keys: Vec<String>,
}

impl std::fmt::Debug for UntagResourceInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UntagResourceInput").finish_non_exhaustive()
    }
}
/// Step Functions `UpdateMapRunInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct UpdateMapRunInput {
    /// The `mapRunArn` wire member.
    pub map_run_arn: String,
    /// The `maxConcurrency` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_concurrency: Option<i32>,
    /// The `toleratedFailureCount` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tolerated_failure_count: Option<i64>,
    /// The `toleratedFailurePercentage` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tolerated_failure_percentage: Option<f32>,
}

impl std::fmt::Debug for UpdateMapRunInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateMapRunInput").finish_non_exhaustive()
    }
}
/// Step Functions `UpdateStateMachineAliasInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct UpdateStateMachineAliasInput {
    /// The `description` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The `routingConfiguration` wire member.
    #[serde(default)]
    pub routing_configuration: Vec<RoutingConfigurationListItem>,
    /// The `stateMachineAliasArn` wire member.
    pub state_machine_alias_arn: String,
}

impl std::fmt::Debug for UpdateStateMachineAliasInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateStateMachineAliasInput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `UpdateStateMachineInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
#[derive(typed_builder::TypedBuilder)]
pub struct UpdateStateMachineInput {
    /// The `definition` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<String>,
    /// The `encryptionConfiguration` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryption_configuration: Option<EncryptionConfiguration>,
    /// The `loggingConfiguration` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logging_configuration: Option<LoggingConfiguration>,
    /// The `publish` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publish: Option<bool>,
    /// The `roleArn` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_arn: Option<String>,
    /// The `stateMachineArn` wire member.
    pub state_machine_arn: String,
    /// The `tracingConfiguration` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracing_configuration: Option<TracingConfiguration>,
    /// The `versionDescription` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_description: Option<String>,
}

impl std::fmt::Debug for UpdateStateMachineInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateStateMachineInput")
            .finish_non_exhaustive()
    }
}
/// Step Functions `ValidateStateMachineDefinitionInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ValidateStateMachineDefinitionInput {
    /// The `definition` wire member.
    pub definition: String,
    /// The `maxResults` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<i32>,
    /// The `severity` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<ValidateStateMachineDefinitionSeverity>,
    /// The `type` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<StateMachineType>,
}

impl std::fmt::Debug for ValidateStateMachineDefinitionInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidateStateMachineDefinitionInput")
            .finish_non_exhaustive()
    }
}
