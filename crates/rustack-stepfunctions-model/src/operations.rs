//! Auto-generated from AWS Step Functions Smithy model. DO NOT EDIT.

/// All supported `StepFunctionsOperation` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StepFunctionsOperation {
    /// The `CreateActivity` operation.
    CreateActivity,
    /// The `CreateStateMachine` operation.
    CreateStateMachine,
    /// The `CreateStateMachineAlias` operation.
    CreateStateMachineAlias,
    /// The `DeleteActivity` operation.
    DeleteActivity,
    /// The `DeleteStateMachine` operation.
    DeleteStateMachine,
    /// The `DeleteStateMachineAlias` operation.
    DeleteStateMachineAlias,
    /// The `DeleteStateMachineVersion` operation.
    DeleteStateMachineVersion,
    /// The `DescribeActivity` operation.
    DescribeActivity,
    /// The `DescribeExecution` operation.
    DescribeExecution,
    /// The `DescribeMapRun` operation.
    DescribeMapRun,
    /// The `DescribeStateMachine` operation.
    DescribeStateMachine,
    /// The `DescribeStateMachineAlias` operation.
    DescribeStateMachineAlias,
    /// The `DescribeStateMachineForExecution` operation.
    DescribeStateMachineForExecution,
    /// The `GetActivityTask` operation.
    GetActivityTask,
    /// The `GetExecutionHistory` operation.
    GetExecutionHistory,
    /// The `ListActivities` operation.
    ListActivities,
    /// The `ListExecutions` operation.
    ListExecutions,
    /// The `ListMapRuns` operation.
    ListMapRuns,
    /// The `ListStateMachineAliases` operation.
    ListStateMachineAliases,
    /// The `ListStateMachines` operation.
    ListStateMachines,
    /// The `ListStateMachineVersions` operation.
    ListStateMachineVersions,
    /// The `ListTagsForResource` operation.
    ListTagsForResource,
    /// The `PublishStateMachineVersion` operation.
    PublishStateMachineVersion,
    /// The `RedriveExecution` operation.
    RedriveExecution,
    /// The `SendTaskFailure` operation.
    SendTaskFailure,
    /// The `SendTaskHeartbeat` operation.
    SendTaskHeartbeat,
    /// The `SendTaskSuccess` operation.
    SendTaskSuccess,
    /// The `StartExecution` operation.
    StartExecution,
    /// The `StartSyncExecution` operation.
    StartSyncExecution,
    /// The `StopExecution` operation.
    StopExecution,
    /// The `TagResource` operation.
    TagResource,
    /// The `TestState` operation.
    TestState,
    /// The `UntagResource` operation.
    UntagResource,
    /// The `UpdateMapRun` operation.
    UpdateMapRun,
    /// The `UpdateStateMachine` operation.
    UpdateStateMachine,
    /// The `UpdateStateMachineAlias` operation.
    UpdateStateMachineAlias,
    /// The `ValidateStateMachineDefinition` operation.
    ValidateStateMachineDefinition,
}

impl StepFunctionsOperation {
    /// Returns the AWS operation name string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CreateActivity => "CreateActivity",
            Self::CreateStateMachine => "CreateStateMachine",
            Self::CreateStateMachineAlias => "CreateStateMachineAlias",
            Self::DeleteActivity => "DeleteActivity",
            Self::DeleteStateMachine => "DeleteStateMachine",
            Self::DeleteStateMachineAlias => "DeleteStateMachineAlias",
            Self::DeleteStateMachineVersion => "DeleteStateMachineVersion",
            Self::DescribeActivity => "DescribeActivity",
            Self::DescribeExecution => "DescribeExecution",
            Self::DescribeMapRun => "DescribeMapRun",
            Self::DescribeStateMachine => "DescribeStateMachine",
            Self::DescribeStateMachineAlias => "DescribeStateMachineAlias",
            Self::DescribeStateMachineForExecution => "DescribeStateMachineForExecution",
            Self::GetActivityTask => "GetActivityTask",
            Self::GetExecutionHistory => "GetExecutionHistory",
            Self::ListActivities => "ListActivities",
            Self::ListExecutions => "ListExecutions",
            Self::ListMapRuns => "ListMapRuns",
            Self::ListStateMachineAliases => "ListStateMachineAliases",
            Self::ListStateMachines => "ListStateMachines",
            Self::ListStateMachineVersions => "ListStateMachineVersions",
            Self::ListTagsForResource => "ListTagsForResource",
            Self::PublishStateMachineVersion => "PublishStateMachineVersion",
            Self::RedriveExecution => "RedriveExecution",
            Self::SendTaskFailure => "SendTaskFailure",
            Self::SendTaskHeartbeat => "SendTaskHeartbeat",
            Self::SendTaskSuccess => "SendTaskSuccess",
            Self::StartExecution => "StartExecution",
            Self::StartSyncExecution => "StartSyncExecution",
            Self::StopExecution => "StopExecution",
            Self::TagResource => "TagResource",
            Self::TestState => "TestState",
            Self::UntagResource => "UntagResource",
            Self::UpdateMapRun => "UpdateMapRun",
            Self::UpdateStateMachine => "UpdateStateMachine",
            Self::UpdateStateMachineAlias => "UpdateStateMachineAlias",
            Self::ValidateStateMachineDefinition => "ValidateStateMachineDefinition",
        }
    }

    /// Parse an operation name string into a `StepFunctionsOperation`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "CreateActivity" => Some(Self::CreateActivity),
            "CreateStateMachine" => Some(Self::CreateStateMachine),
            "CreateStateMachineAlias" => Some(Self::CreateStateMachineAlias),
            "DeleteActivity" => Some(Self::DeleteActivity),
            "DeleteStateMachine" => Some(Self::DeleteStateMachine),
            "DeleteStateMachineAlias" => Some(Self::DeleteStateMachineAlias),
            "DeleteStateMachineVersion" => Some(Self::DeleteStateMachineVersion),
            "DescribeActivity" => Some(Self::DescribeActivity),
            "DescribeExecution" => Some(Self::DescribeExecution),
            "DescribeMapRun" => Some(Self::DescribeMapRun),
            "DescribeStateMachine" => Some(Self::DescribeStateMachine),
            "DescribeStateMachineAlias" => Some(Self::DescribeStateMachineAlias),
            "DescribeStateMachineForExecution" => Some(Self::DescribeStateMachineForExecution),
            "GetActivityTask" => Some(Self::GetActivityTask),
            "GetExecutionHistory" => Some(Self::GetExecutionHistory),
            "ListActivities" => Some(Self::ListActivities),
            "ListExecutions" => Some(Self::ListExecutions),
            "ListMapRuns" => Some(Self::ListMapRuns),
            "ListStateMachineAliases" => Some(Self::ListStateMachineAliases),
            "ListStateMachines" => Some(Self::ListStateMachines),
            "ListStateMachineVersions" => Some(Self::ListStateMachineVersions),
            "ListTagsForResource" => Some(Self::ListTagsForResource),
            "PublishStateMachineVersion" => Some(Self::PublishStateMachineVersion),
            "RedriveExecution" => Some(Self::RedriveExecution),
            "SendTaskFailure" => Some(Self::SendTaskFailure),
            "SendTaskHeartbeat" => Some(Self::SendTaskHeartbeat),
            "SendTaskSuccess" => Some(Self::SendTaskSuccess),
            "StartExecution" => Some(Self::StartExecution),
            "StartSyncExecution" => Some(Self::StartSyncExecution),
            "StopExecution" => Some(Self::StopExecution),
            "TagResource" => Some(Self::TagResource),
            "TestState" => Some(Self::TestState),
            "UntagResource" => Some(Self::UntagResource),
            "UpdateMapRun" => Some(Self::UpdateMapRun),
            "UpdateStateMachine" => Some(Self::UpdateStateMachine),
            "UpdateStateMachineAlias" => Some(Self::UpdateStateMachineAlias),
            "ValidateStateMachineDefinition" => Some(Self::ValidateStateMachineDefinition),
            _ => None,
        }
    }
}

impl std::fmt::Display for StepFunctionsOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
