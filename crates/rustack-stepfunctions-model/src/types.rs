//! Auto-generated from AWS Step Functions Smithy model. DO NOT EDIT.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Step Functions `EncryptionType` enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum EncryptionType {
    /// The `AWS_OWNED_KEY` wire variant.
    #[default]
    #[serde(rename = "AWS_OWNED_KEY")]
    AwsOwnedKey,
    /// The `CUSTOMER_MANAGED_KMS_KEY` wire variant.
    #[serde(rename = "CUSTOMER_MANAGED_KMS_KEY")]
    CustomerManagedKmsKey,
}

impl EncryptionType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AwsOwnedKey => "AWS_OWNED_KEY",
            Self::CustomerManagedKmsKey => "CUSTOMER_MANAGED_KMS_KEY",
        }
    }
}

impl std::fmt::Display for EncryptionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for EncryptionType {
    fn from(s: &str) -> Self {
        match s {
            "AWS_OWNED_KEY" => Self::AwsOwnedKey,
            "CUSTOMER_MANAGED_KMS_KEY" => Self::CustomerManagedKmsKey,
            _ => Self::default(),
        }
    }
}

/// Step Functions `ExecutionRedriveFilter` enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ExecutionRedriveFilter {
    /// The `NOT_REDRIVEN` wire variant.
    #[default]
    #[serde(rename = "NOT_REDRIVEN")]
    NotRedriven,
    /// The `REDRIVEN` wire variant.
    #[serde(rename = "REDRIVEN")]
    Redriven,
}

impl ExecutionRedriveFilter {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotRedriven => "NOT_REDRIVEN",
            Self::Redriven => "REDRIVEN",
        }
    }
}

impl std::fmt::Display for ExecutionRedriveFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ExecutionRedriveFilter {
    fn from(s: &str) -> Self {
        match s {
            "NOT_REDRIVEN" => Self::NotRedriven,
            "REDRIVEN" => Self::Redriven,
            _ => Self::default(),
        }
    }
}

/// Step Functions `ExecutionRedriveStatus` enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ExecutionRedriveStatus {
    /// The `NOT_REDRIVABLE` wire variant.
    #[default]
    #[serde(rename = "NOT_REDRIVABLE")]
    NotRedrivable,
    /// The `REDRIVABLE` wire variant.
    #[serde(rename = "REDRIVABLE")]
    Redrivable,
    /// The `REDRIVABLE_BY_MAP_RUN` wire variant.
    #[serde(rename = "REDRIVABLE_BY_MAP_RUN")]
    RedrivableByMapRun,
}

impl ExecutionRedriveStatus {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotRedrivable => "NOT_REDRIVABLE",
            Self::Redrivable => "REDRIVABLE",
            Self::RedrivableByMapRun => "REDRIVABLE_BY_MAP_RUN",
        }
    }
}

impl std::fmt::Display for ExecutionRedriveStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ExecutionRedriveStatus {
    fn from(s: &str) -> Self {
        match s {
            "NOT_REDRIVABLE" => Self::NotRedrivable,
            "REDRIVABLE" => Self::Redrivable,
            "REDRIVABLE_BY_MAP_RUN" => Self::RedrivableByMapRun,
            _ => Self::default(),
        }
    }
}

/// Step Functions `ExecutionStatus` enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ExecutionStatus {
    /// The `ABORTED` wire variant.
    #[default]
    #[serde(rename = "ABORTED")]
    Aborted,
    /// The `FAILED` wire variant.
    #[serde(rename = "FAILED")]
    Failed,
    /// The `PENDING_REDRIVE` wire variant.
    #[serde(rename = "PENDING_REDRIVE")]
    PendingRedrive,
    /// The `RUNNING` wire variant.
    #[serde(rename = "RUNNING")]
    Running,
    /// The `SUCCEEDED` wire variant.
    #[serde(rename = "SUCCEEDED")]
    Succeeded,
    /// The `TIMED_OUT` wire variant.
    #[serde(rename = "TIMED_OUT")]
    TimedOut,
}

impl ExecutionStatus {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Aborted => "ABORTED",
            Self::Failed => "FAILED",
            Self::PendingRedrive => "PENDING_REDRIVE",
            Self::Running => "RUNNING",
            Self::Succeeded => "SUCCEEDED",
            Self::TimedOut => "TIMED_OUT",
        }
    }
}

impl std::fmt::Display for ExecutionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ExecutionStatus {
    fn from(s: &str) -> Self {
        match s {
            "ABORTED" => Self::Aborted,
            "FAILED" => Self::Failed,
            "PENDING_REDRIVE" => Self::PendingRedrive,
            "RUNNING" => Self::Running,
            "SUCCEEDED" => Self::Succeeded,
            "TIMED_OUT" => Self::TimedOut,
            _ => Self::default(),
        }
    }
}

/// Step Functions `HistoryEventType` enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum HistoryEventType {
    /// The `ActivityFailed` wire variant.
    #[default]
    ActivityFailed,
    /// The `ActivityScheduleFailed` wire variant.
    ActivityScheduleFailed,
    /// The `ActivityScheduled` wire variant.
    ActivityScheduled,
    /// The `ActivityStarted` wire variant.
    ActivityStarted,
    /// The `ActivitySucceeded` wire variant.
    ActivitySucceeded,
    /// The `ActivityTimedOut` wire variant.
    ActivityTimedOut,
    /// The `ChoiceStateEntered` wire variant.
    ChoiceStateEntered,
    /// The `ChoiceStateExited` wire variant.
    ChoiceStateExited,
    /// The `EvaluationFailed` wire variant.
    EvaluationFailed,
    /// The `ExecutionAborted` wire variant.
    ExecutionAborted,
    /// The `ExecutionFailed` wire variant.
    ExecutionFailed,
    /// The `ExecutionRedriven` wire variant.
    ExecutionRedriven,
    /// The `ExecutionStarted` wire variant.
    ExecutionStarted,
    /// The `ExecutionSucceeded` wire variant.
    ExecutionSucceeded,
    /// The `ExecutionTimedOut` wire variant.
    ExecutionTimedOut,
    /// The `FailStateEntered` wire variant.
    FailStateEntered,
    /// The `LambdaFunctionFailed` wire variant.
    LambdaFunctionFailed,
    /// The `LambdaFunctionScheduleFailed` wire variant.
    LambdaFunctionScheduleFailed,
    /// The `LambdaFunctionScheduled` wire variant.
    LambdaFunctionScheduled,
    /// The `LambdaFunctionStartFailed` wire variant.
    LambdaFunctionStartFailed,
    /// The `LambdaFunctionStarted` wire variant.
    LambdaFunctionStarted,
    /// The `LambdaFunctionSucceeded` wire variant.
    LambdaFunctionSucceeded,
    /// The `LambdaFunctionTimedOut` wire variant.
    LambdaFunctionTimedOut,
    /// The `MapIterationAborted` wire variant.
    MapIterationAborted,
    /// The `MapIterationFailed` wire variant.
    MapIterationFailed,
    /// The `MapIterationStarted` wire variant.
    MapIterationStarted,
    /// The `MapIterationSucceeded` wire variant.
    MapIterationSucceeded,
    /// The `MapRunAborted` wire variant.
    MapRunAborted,
    /// The `MapRunFailed` wire variant.
    MapRunFailed,
    /// The `MapRunRedriven` wire variant.
    MapRunRedriven,
    /// The `MapRunStarted` wire variant.
    MapRunStarted,
    /// The `MapRunSucceeded` wire variant.
    MapRunSucceeded,
    /// The `MapStateAborted` wire variant.
    MapStateAborted,
    /// The `MapStateEntered` wire variant.
    MapStateEntered,
    /// The `MapStateExited` wire variant.
    MapStateExited,
    /// The `MapStateFailed` wire variant.
    MapStateFailed,
    /// The `MapStateStarted` wire variant.
    MapStateStarted,
    /// The `MapStateSucceeded` wire variant.
    MapStateSucceeded,
    /// The `ParallelStateAborted` wire variant.
    ParallelStateAborted,
    /// The `ParallelStateEntered` wire variant.
    ParallelStateEntered,
    /// The `ParallelStateExited` wire variant.
    ParallelStateExited,
    /// The `ParallelStateFailed` wire variant.
    ParallelStateFailed,
    /// The `ParallelStateStarted` wire variant.
    ParallelStateStarted,
    /// The `ParallelStateSucceeded` wire variant.
    ParallelStateSucceeded,
    /// The `PassStateEntered` wire variant.
    PassStateEntered,
    /// The `PassStateExited` wire variant.
    PassStateExited,
    /// The `SucceedStateEntered` wire variant.
    SucceedStateEntered,
    /// The `SucceedStateExited` wire variant.
    SucceedStateExited,
    /// The `TaskFailed` wire variant.
    TaskFailed,
    /// The `TaskScheduled` wire variant.
    TaskScheduled,
    /// The `TaskStartFailed` wire variant.
    TaskStartFailed,
    /// The `TaskStarted` wire variant.
    TaskStarted,
    /// The `TaskStateAborted` wire variant.
    TaskStateAborted,
    /// The `TaskStateEntered` wire variant.
    TaskStateEntered,
    /// The `TaskStateExited` wire variant.
    TaskStateExited,
    /// The `TaskSubmitFailed` wire variant.
    TaskSubmitFailed,
    /// The `TaskSubmitted` wire variant.
    TaskSubmitted,
    /// The `TaskSucceeded` wire variant.
    TaskSucceeded,
    /// The `TaskTimedOut` wire variant.
    TaskTimedOut,
    /// The `WaitStateAborted` wire variant.
    WaitStateAborted,
    /// The `WaitStateEntered` wire variant.
    WaitStateEntered,
    /// The `WaitStateExited` wire variant.
    WaitStateExited,
}

impl HistoryEventType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ActivityFailed => "ActivityFailed",
            Self::ActivityScheduleFailed => "ActivityScheduleFailed",
            Self::ActivityScheduled => "ActivityScheduled",
            Self::ActivityStarted => "ActivityStarted",
            Self::ActivitySucceeded => "ActivitySucceeded",
            Self::ActivityTimedOut => "ActivityTimedOut",
            Self::ChoiceStateEntered => "ChoiceStateEntered",
            Self::ChoiceStateExited => "ChoiceStateExited",
            Self::EvaluationFailed => "EvaluationFailed",
            Self::ExecutionAborted => "ExecutionAborted",
            Self::ExecutionFailed => "ExecutionFailed",
            Self::ExecutionRedriven => "ExecutionRedriven",
            Self::ExecutionStarted => "ExecutionStarted",
            Self::ExecutionSucceeded => "ExecutionSucceeded",
            Self::ExecutionTimedOut => "ExecutionTimedOut",
            Self::FailStateEntered => "FailStateEntered",
            Self::LambdaFunctionFailed => "LambdaFunctionFailed",
            Self::LambdaFunctionScheduleFailed => "LambdaFunctionScheduleFailed",
            Self::LambdaFunctionScheduled => "LambdaFunctionScheduled",
            Self::LambdaFunctionStartFailed => "LambdaFunctionStartFailed",
            Self::LambdaFunctionStarted => "LambdaFunctionStarted",
            Self::LambdaFunctionSucceeded => "LambdaFunctionSucceeded",
            Self::LambdaFunctionTimedOut => "LambdaFunctionTimedOut",
            Self::MapIterationAborted => "MapIterationAborted",
            Self::MapIterationFailed => "MapIterationFailed",
            Self::MapIterationStarted => "MapIterationStarted",
            Self::MapIterationSucceeded => "MapIterationSucceeded",
            Self::MapRunAborted => "MapRunAborted",
            Self::MapRunFailed => "MapRunFailed",
            Self::MapRunRedriven => "MapRunRedriven",
            Self::MapRunStarted => "MapRunStarted",
            Self::MapRunSucceeded => "MapRunSucceeded",
            Self::MapStateAborted => "MapStateAborted",
            Self::MapStateEntered => "MapStateEntered",
            Self::MapStateExited => "MapStateExited",
            Self::MapStateFailed => "MapStateFailed",
            Self::MapStateStarted => "MapStateStarted",
            Self::MapStateSucceeded => "MapStateSucceeded",
            Self::ParallelStateAborted => "ParallelStateAborted",
            Self::ParallelStateEntered => "ParallelStateEntered",
            Self::ParallelStateExited => "ParallelStateExited",
            Self::ParallelStateFailed => "ParallelStateFailed",
            Self::ParallelStateStarted => "ParallelStateStarted",
            Self::ParallelStateSucceeded => "ParallelStateSucceeded",
            Self::PassStateEntered => "PassStateEntered",
            Self::PassStateExited => "PassStateExited",
            Self::SucceedStateEntered => "SucceedStateEntered",
            Self::SucceedStateExited => "SucceedStateExited",
            Self::TaskFailed => "TaskFailed",
            Self::TaskScheduled => "TaskScheduled",
            Self::TaskStartFailed => "TaskStartFailed",
            Self::TaskStarted => "TaskStarted",
            Self::TaskStateAborted => "TaskStateAborted",
            Self::TaskStateEntered => "TaskStateEntered",
            Self::TaskStateExited => "TaskStateExited",
            Self::TaskSubmitFailed => "TaskSubmitFailed",
            Self::TaskSubmitted => "TaskSubmitted",
            Self::TaskSucceeded => "TaskSucceeded",
            Self::TaskTimedOut => "TaskTimedOut",
            Self::WaitStateAborted => "WaitStateAborted",
            Self::WaitStateEntered => "WaitStateEntered",
            Self::WaitStateExited => "WaitStateExited",
        }
    }
}

impl std::fmt::Display for HistoryEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for HistoryEventType {
    fn from(s: &str) -> Self {
        match s {
            "ActivityFailed" => Self::ActivityFailed,
            "ActivityScheduleFailed" => Self::ActivityScheduleFailed,
            "ActivityScheduled" => Self::ActivityScheduled,
            "ActivityStarted" => Self::ActivityStarted,
            "ActivitySucceeded" => Self::ActivitySucceeded,
            "ActivityTimedOut" => Self::ActivityTimedOut,
            "ChoiceStateEntered" => Self::ChoiceStateEntered,
            "ChoiceStateExited" => Self::ChoiceStateExited,
            "EvaluationFailed" => Self::EvaluationFailed,
            "ExecutionAborted" => Self::ExecutionAborted,
            "ExecutionFailed" => Self::ExecutionFailed,
            "ExecutionRedriven" => Self::ExecutionRedriven,
            "ExecutionStarted" => Self::ExecutionStarted,
            "ExecutionSucceeded" => Self::ExecutionSucceeded,
            "ExecutionTimedOut" => Self::ExecutionTimedOut,
            "FailStateEntered" => Self::FailStateEntered,
            "LambdaFunctionFailed" => Self::LambdaFunctionFailed,
            "LambdaFunctionScheduleFailed" => Self::LambdaFunctionScheduleFailed,
            "LambdaFunctionScheduled" => Self::LambdaFunctionScheduled,
            "LambdaFunctionStartFailed" => Self::LambdaFunctionStartFailed,
            "LambdaFunctionStarted" => Self::LambdaFunctionStarted,
            "LambdaFunctionSucceeded" => Self::LambdaFunctionSucceeded,
            "LambdaFunctionTimedOut" => Self::LambdaFunctionTimedOut,
            "MapIterationAborted" => Self::MapIterationAborted,
            "MapIterationFailed" => Self::MapIterationFailed,
            "MapIterationStarted" => Self::MapIterationStarted,
            "MapIterationSucceeded" => Self::MapIterationSucceeded,
            "MapRunAborted" => Self::MapRunAborted,
            "MapRunFailed" => Self::MapRunFailed,
            "MapRunRedriven" => Self::MapRunRedriven,
            "MapRunStarted" => Self::MapRunStarted,
            "MapRunSucceeded" => Self::MapRunSucceeded,
            "MapStateAborted" => Self::MapStateAborted,
            "MapStateEntered" => Self::MapStateEntered,
            "MapStateExited" => Self::MapStateExited,
            "MapStateFailed" => Self::MapStateFailed,
            "MapStateStarted" => Self::MapStateStarted,
            "MapStateSucceeded" => Self::MapStateSucceeded,
            "ParallelStateAborted" => Self::ParallelStateAborted,
            "ParallelStateEntered" => Self::ParallelStateEntered,
            "ParallelStateExited" => Self::ParallelStateExited,
            "ParallelStateFailed" => Self::ParallelStateFailed,
            "ParallelStateStarted" => Self::ParallelStateStarted,
            "ParallelStateSucceeded" => Self::ParallelStateSucceeded,
            "PassStateEntered" => Self::PassStateEntered,
            "PassStateExited" => Self::PassStateExited,
            "SucceedStateEntered" => Self::SucceedStateEntered,
            "SucceedStateExited" => Self::SucceedStateExited,
            "TaskFailed" => Self::TaskFailed,
            "TaskScheduled" => Self::TaskScheduled,
            "TaskStartFailed" => Self::TaskStartFailed,
            "TaskStarted" => Self::TaskStarted,
            "TaskStateAborted" => Self::TaskStateAborted,
            "TaskStateEntered" => Self::TaskStateEntered,
            "TaskStateExited" => Self::TaskStateExited,
            "TaskSubmitFailed" => Self::TaskSubmitFailed,
            "TaskSubmitted" => Self::TaskSubmitted,
            "TaskSucceeded" => Self::TaskSucceeded,
            "TaskTimedOut" => Self::TaskTimedOut,
            "WaitStateAborted" => Self::WaitStateAborted,
            "WaitStateEntered" => Self::WaitStateEntered,
            "WaitStateExited" => Self::WaitStateExited,
            _ => Self::default(),
        }
    }
}

/// Step Functions `IncludedData` enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum IncludedData {
    /// The `ALL_DATA` wire variant.
    #[default]
    #[serde(rename = "ALL_DATA")]
    AllData,
    /// The `METADATA_ONLY` wire variant.
    #[serde(rename = "METADATA_ONLY")]
    MetadataOnly,
}

impl IncludedData {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AllData => "ALL_DATA",
            Self::MetadataOnly => "METADATA_ONLY",
        }
    }
}

impl std::fmt::Display for IncludedData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for IncludedData {
    fn from(s: &str) -> Self {
        match s {
            "ALL_DATA" => Self::AllData,
            "METADATA_ONLY" => Self::MetadataOnly,
            _ => Self::default(),
        }
    }
}

/// Step Functions `InspectionLevel` enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum InspectionLevel {
    /// The `DEBUG` wire variant.
    #[default]
    #[serde(rename = "DEBUG")]
    Debug,
    /// The `INFO` wire variant.
    #[serde(rename = "INFO")]
    Info,
    /// The `TRACE` wire variant.
    #[serde(rename = "TRACE")]
    Trace,
}

impl InspectionLevel {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Trace => "TRACE",
        }
    }
}

impl std::fmt::Display for InspectionLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for InspectionLevel {
    fn from(s: &str) -> Self {
        match s {
            "DEBUG" => Self::Debug,
            "INFO" => Self::Info,
            "TRACE" => Self::Trace,
            _ => Self::default(),
        }
    }
}

/// Step Functions `LogLevel` enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum LogLevel {
    /// The `ALL` wire variant.
    #[default]
    #[serde(rename = "ALL")]
    All,
    /// The `ERROR` wire variant.
    #[serde(rename = "ERROR")]
    Error,
    /// The `FATAL` wire variant.
    #[serde(rename = "FATAL")]
    Fatal,
    /// The `OFF` wire variant.
    #[serde(rename = "OFF")]
    Off,
}

impl LogLevel {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::All => "ALL",
            Self::Error => "ERROR",
            Self::Fatal => "FATAL",
            Self::Off => "OFF",
        }
    }
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for LogLevel {
    fn from(s: &str) -> Self {
        match s {
            "ALL" => Self::All,
            "ERROR" => Self::Error,
            "FATAL" => Self::Fatal,
            "OFF" => Self::Off,
            _ => Self::default(),
        }
    }
}

/// Step Functions `MapRunStatus` enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum MapRunStatus {
    /// The `ABORTED` wire variant.
    #[default]
    #[serde(rename = "ABORTED")]
    Aborted,
    /// The `FAILED` wire variant.
    #[serde(rename = "FAILED")]
    Failed,
    /// The `RUNNING` wire variant.
    #[serde(rename = "RUNNING")]
    Running,
    /// The `SUCCEEDED` wire variant.
    #[serde(rename = "SUCCEEDED")]
    Succeeded,
}

impl MapRunStatus {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Aborted => "ABORTED",
            Self::Failed => "FAILED",
            Self::Running => "RUNNING",
            Self::Succeeded => "SUCCEEDED",
        }
    }
}

impl std::fmt::Display for MapRunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for MapRunStatus {
    fn from(s: &str) -> Self {
        match s {
            "ABORTED" => Self::Aborted,
            "FAILED" => Self::Failed,
            "RUNNING" => Self::Running,
            "SUCCEEDED" => Self::Succeeded,
            _ => Self::default(),
        }
    }
}

/// Step Functions `MockResponseValidationMode` enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum MockResponseValidationMode {
    /// The `NONE` wire variant.
    #[default]
    #[serde(rename = "NONE")]
    None,
    /// The `PRESENT` wire variant.
    #[serde(rename = "PRESENT")]
    Present,
    /// The `STRICT` wire variant.
    #[serde(rename = "STRICT")]
    Strict,
}

impl MockResponseValidationMode {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::Present => "PRESENT",
            Self::Strict => "STRICT",
        }
    }
}

impl std::fmt::Display for MockResponseValidationMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for MockResponseValidationMode {
    fn from(s: &str) -> Self {
        match s {
            "NONE" => Self::None,
            "PRESENT" => Self::Present,
            "STRICT" => Self::Strict,
            _ => Self::default(),
        }
    }
}

/// Step Functions `StateMachineStatus` enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum StateMachineStatus {
    /// The `ACTIVE` wire variant.
    #[default]
    #[serde(rename = "ACTIVE")]
    Active,
    /// The `DELETING` wire variant.
    #[serde(rename = "DELETING")]
    Deleting,
}

impl StateMachineStatus {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Deleting => "DELETING",
        }
    }
}

impl std::fmt::Display for StateMachineStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for StateMachineStatus {
    fn from(s: &str) -> Self {
        match s {
            "ACTIVE" => Self::Active,
            "DELETING" => Self::Deleting,
            _ => Self::default(),
        }
    }
}

/// Step Functions `StateMachineType` enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum StateMachineType {
    /// The `EXPRESS` wire variant.
    #[default]
    #[serde(rename = "EXPRESS")]
    Express,
    /// The `STANDARD` wire variant.
    #[serde(rename = "STANDARD")]
    Standard,
}

impl StateMachineType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Express => "EXPRESS",
            Self::Standard => "STANDARD",
        }
    }
}

impl std::fmt::Display for StateMachineType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for StateMachineType {
    fn from(s: &str) -> Self {
        match s {
            "EXPRESS" => Self::Express,
            "STANDARD" => Self::Standard,
            _ => Self::default(),
        }
    }
}

/// Step Functions `SyncExecutionStatus` enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum SyncExecutionStatus {
    /// The `FAILED` wire variant.
    #[default]
    #[serde(rename = "FAILED")]
    Failed,
    /// The `SUCCEEDED` wire variant.
    #[serde(rename = "SUCCEEDED")]
    Succeeded,
    /// The `TIMED_OUT` wire variant.
    #[serde(rename = "TIMED_OUT")]
    TimedOut,
}

impl SyncExecutionStatus {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Failed => "FAILED",
            Self::Succeeded => "SUCCEEDED",
            Self::TimedOut => "TIMED_OUT",
        }
    }
}

impl std::fmt::Display for SyncExecutionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for SyncExecutionStatus {
    fn from(s: &str) -> Self {
        match s {
            "FAILED" => Self::Failed,
            "SUCCEEDED" => Self::Succeeded,
            "TIMED_OUT" => Self::TimedOut,
            _ => Self::default(),
        }
    }
}

/// Step Functions `TestExecutionStatus` enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum TestExecutionStatus {
    /// The `CAUGHT_ERROR` wire variant.
    #[default]
    #[serde(rename = "CAUGHT_ERROR")]
    CaughtError,
    /// The `FAILED` wire variant.
    #[serde(rename = "FAILED")]
    Failed,
    /// The `RETRIABLE` wire variant.
    #[serde(rename = "RETRIABLE")]
    Retriable,
    /// The `SUCCEEDED` wire variant.
    #[serde(rename = "SUCCEEDED")]
    Succeeded,
}

impl TestExecutionStatus {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CaughtError => "CAUGHT_ERROR",
            Self::Failed => "FAILED",
            Self::Retriable => "RETRIABLE",
            Self::Succeeded => "SUCCEEDED",
        }
    }
}

impl std::fmt::Display for TestExecutionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for TestExecutionStatus {
    fn from(s: &str) -> Self {
        match s {
            "CAUGHT_ERROR" => Self::CaughtError,
            "FAILED" => Self::Failed,
            "RETRIABLE" => Self::Retriable,
            "SUCCEEDED" => Self::Succeeded,
            _ => Self::default(),
        }
    }
}

/// Step Functions `ValidateStateMachineDefinitionResultCode` enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ValidateStateMachineDefinitionResultCode {
    /// The `FAIL` wire variant.
    #[default]
    #[serde(rename = "FAIL")]
    Fail,
    /// The `OK` wire variant.
    #[serde(rename = "OK")]
    Ok,
}

impl ValidateStateMachineDefinitionResultCode {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Fail => "FAIL",
            Self::Ok => "OK",
        }
    }
}

impl std::fmt::Display for ValidateStateMachineDefinitionResultCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ValidateStateMachineDefinitionResultCode {
    fn from(s: &str) -> Self {
        match s {
            "FAIL" => Self::Fail,
            "OK" => Self::Ok,
            _ => Self::default(),
        }
    }
}

/// Step Functions `ValidateStateMachineDefinitionSeverity` enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ValidateStateMachineDefinitionSeverity {
    /// The `ERROR` wire variant.
    #[default]
    #[serde(rename = "ERROR")]
    Error,
    /// The `WARNING` wire variant.
    #[serde(rename = "WARNING")]
    Warning,
}

impl ValidateStateMachineDefinitionSeverity {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Error => "ERROR",
            Self::Warning => "WARNING",
        }
    }
}

impl std::fmt::Display for ValidateStateMachineDefinitionSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ValidateStateMachineDefinitionSeverity {
    fn from(s: &str) -> Self {
        match s {
            "ERROR" => Self::Error,
            "WARNING" => Self::Warning,
            _ => Self::default(),
        }
    }
}

/// Step Functions `ActivityFailedEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ActivityFailedEventDetails {
    /// The `cause` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// The `error` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl std::fmt::Debug for ActivityFailedEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActivityFailedEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `ActivityListItem`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ActivityListItem {
    /// The `activityArn` wire member.
    pub activity_arn: String,
    /// The `creationDate` wire member.
    #[serde(with = "crate::epoch_seconds")]
    pub creation_date: chrono::DateTime<chrono::Utc>,
    /// The `name` wire member.
    pub name: String,
}

impl std::fmt::Debug for ActivityListItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActivityListItem").finish_non_exhaustive()
    }
}
/// Step Functions `ActivityScheduleFailedEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ActivityScheduleFailedEventDetails {
    /// The `cause` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// The `error` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl std::fmt::Debug for ActivityScheduleFailedEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActivityScheduleFailedEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `ActivityScheduledEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ActivityScheduledEventDetails {
    /// The `heartbeatInSeconds` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heartbeat_in_seconds: Option<i64>,
    /// The `input` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// The `inputDetails` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_details: Option<HistoryEventExecutionDataDetails>,
    /// The `resource` wire member.
    pub resource: String,
    /// The `timeoutInSeconds` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_in_seconds: Option<i64>,
}

impl std::fmt::Debug for ActivityScheduledEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActivityScheduledEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `ActivityStartedEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ActivityStartedEventDetails {
    /// The `workerName` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_name: Option<String>,
}

impl std::fmt::Debug for ActivityStartedEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActivityStartedEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `ActivitySucceededEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ActivitySucceededEventDetails {
    /// The `output` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// The `outputDetails` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_details: Option<HistoryEventExecutionDataDetails>,
}

impl std::fmt::Debug for ActivitySucceededEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActivitySucceededEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `ActivityTimedOutEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ActivityTimedOutEventDetails {
    /// The `cause` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// The `error` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl std::fmt::Debug for ActivityTimedOutEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActivityTimedOutEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `AssignedVariablesDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct AssignedVariablesDetails {
    /// The `truncated` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
}

impl std::fmt::Debug for AssignedVariablesDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssignedVariablesDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `BillingDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct BillingDetails {
    /// The `billedDurationInMilliseconds` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billed_duration_in_milliseconds: Option<i64>,
    /// The `billedMemoryUsedInMB` wire member.
    #[serde(rename = "billedMemoryUsedInMB")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billed_memory_used_in_mb: Option<i64>,
}

impl std::fmt::Debug for BillingDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BillingDetails").finish_non_exhaustive()
    }
}
/// Step Functions `CloudWatchEventsExecutionDataDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct CloudWatchEventsExecutionDataDetails {
    /// The `included` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub included: Option<bool>,
}

impl std::fmt::Debug for CloudWatchEventsExecutionDataDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudWatchEventsExecutionDataDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `CloudWatchLogsLogGroup`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct CloudWatchLogsLogGroup {
    /// The `logGroupArn` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_group_arn: Option<String>,
}

impl std::fmt::Debug for CloudWatchLogsLogGroup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudWatchLogsLogGroup")
            .finish_non_exhaustive()
    }
}
/// Step Functions `EncryptionConfiguration`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct EncryptionConfiguration {
    /// The `kmsDataKeyReusePeriodSeconds` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kms_data_key_reuse_period_seconds: Option<i32>,
    /// The `kmsKeyId` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kms_key_id: Option<String>,
    /// The `type` wire member.
    pub r#type: EncryptionType,
}

impl std::fmt::Debug for EncryptionConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncryptionConfiguration")
            .finish_non_exhaustive()
    }
}
/// Step Functions `EvaluationFailedEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct EvaluationFailedEventDetails {
    /// The `cause` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// The `error` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The `location` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// The `state` wire member.
    pub state: String,
}

impl std::fmt::Debug for EvaluationFailedEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EvaluationFailedEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `ExecutionAbortedEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ExecutionAbortedEventDetails {
    /// The `cause` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// The `error` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl std::fmt::Debug for ExecutionAbortedEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutionAbortedEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `ExecutionFailedEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ExecutionFailedEventDetails {
    /// The `cause` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// The `error` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl std::fmt::Debug for ExecutionFailedEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutionFailedEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `ExecutionListItem`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
#[derive(typed_builder::TypedBuilder)]
pub struct ExecutionListItem {
    /// The `executionArn` wire member.
    pub execution_arn: String,
    /// The `itemCount` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_count: Option<i32>,
    /// The `mapRunArn` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map_run_arn: Option<String>,
    /// The `name` wire member.
    pub name: String,
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
}

impl std::fmt::Debug for ExecutionListItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutionListItem").finish_non_exhaustive()
    }
}
/// Step Functions `ExecutionRedrivenEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ExecutionRedrivenEventDetails {
    /// The `redriveCount` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redrive_count: Option<i32>,
}

impl std::fmt::Debug for ExecutionRedrivenEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutionRedrivenEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `ExecutionStartedEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ExecutionStartedEventDetails {
    /// The `input` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// The `inputDetails` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_details: Option<HistoryEventExecutionDataDetails>,
    /// The `roleArn` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_arn: Option<String>,
    /// The `stateMachineAliasArn` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_machine_alias_arn: Option<String>,
    /// The `stateMachineVersionArn` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_machine_version_arn: Option<String>,
}

impl std::fmt::Debug for ExecutionStartedEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutionStartedEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `ExecutionSucceededEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ExecutionSucceededEventDetails {
    /// The `output` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// The `outputDetails` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_details: Option<HistoryEventExecutionDataDetails>,
}

impl std::fmt::Debug for ExecutionSucceededEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutionSucceededEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `ExecutionTimedOutEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ExecutionTimedOutEventDetails {
    /// The `cause` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// The `error` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl std::fmt::Debug for ExecutionTimedOutEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutionTimedOutEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `HistoryEvent`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
#[derive(typed_builder::TypedBuilder)]
pub struct HistoryEvent {
    /// The `activityFailedEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity_failed_event_details: Option<ActivityFailedEventDetails>,
    /// The `activityScheduleFailedEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity_schedule_failed_event_details: Option<ActivityScheduleFailedEventDetails>,
    /// The `activityScheduledEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity_scheduled_event_details: Option<ActivityScheduledEventDetails>,
    /// The `activityStartedEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity_started_event_details: Option<ActivityStartedEventDetails>,
    /// The `activitySucceededEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity_succeeded_event_details: Option<ActivitySucceededEventDetails>,
    /// The `activityTimedOutEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity_timed_out_event_details: Option<ActivityTimedOutEventDetails>,
    /// The `evaluationFailedEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evaluation_failed_event_details: Option<EvaluationFailedEventDetails>,
    /// The `executionAbortedEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_aborted_event_details: Option<ExecutionAbortedEventDetails>,
    /// The `executionFailedEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_failed_event_details: Option<ExecutionFailedEventDetails>,
    /// The `executionRedrivenEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_redriven_event_details: Option<ExecutionRedrivenEventDetails>,
    /// The `executionStartedEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_started_event_details: Option<ExecutionStartedEventDetails>,
    /// The `executionSucceededEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_succeeded_event_details: Option<ExecutionSucceededEventDetails>,
    /// The `executionTimedOutEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_timed_out_event_details: Option<ExecutionTimedOutEventDetails>,
    /// The `id` wire member.
    pub id: i64,
    /// The `lambdaFunctionFailedEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lambda_function_failed_event_details: Option<LambdaFunctionFailedEventDetails>,
    /// The `lambdaFunctionScheduleFailedEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lambda_function_schedule_failed_event_details:
        Option<LambdaFunctionScheduleFailedEventDetails>,
    /// The `lambdaFunctionScheduledEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lambda_function_scheduled_event_details: Option<LambdaFunctionScheduledEventDetails>,
    /// The `lambdaFunctionStartFailedEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lambda_function_start_failed_event_details: Option<LambdaFunctionStartFailedEventDetails>,
    /// The `lambdaFunctionSucceededEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lambda_function_succeeded_event_details: Option<LambdaFunctionSucceededEventDetails>,
    /// The `lambdaFunctionTimedOutEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lambda_function_timed_out_event_details: Option<LambdaFunctionTimedOutEventDetails>,
    /// The `mapIterationAbortedEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map_iteration_aborted_event_details: Option<MapIterationEventDetails>,
    /// The `mapIterationFailedEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map_iteration_failed_event_details: Option<MapIterationEventDetails>,
    /// The `mapIterationStartedEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map_iteration_started_event_details: Option<MapIterationEventDetails>,
    /// The `mapIterationSucceededEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map_iteration_succeeded_event_details: Option<MapIterationEventDetails>,
    /// The `mapRunFailedEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map_run_failed_event_details: Option<MapRunFailedEventDetails>,
    /// The `mapRunRedrivenEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map_run_redriven_event_details: Option<MapRunRedrivenEventDetails>,
    /// The `mapRunStartedEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map_run_started_event_details: Option<MapRunStartedEventDetails>,
    /// The `mapStateStartedEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map_state_started_event_details: Option<MapStateStartedEventDetails>,
    /// The `previousEventId` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_event_id: Option<i64>,
    /// The `stateEnteredEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_entered_event_details: Option<StateEnteredEventDetails>,
    /// The `stateExitedEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_exited_event_details: Option<StateExitedEventDetails>,
    /// The `taskFailedEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_failed_event_details: Option<TaskFailedEventDetails>,
    /// The `taskScheduledEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_scheduled_event_details: Option<TaskScheduledEventDetails>,
    /// The `taskStartFailedEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_start_failed_event_details: Option<TaskStartFailedEventDetails>,
    /// The `taskStartedEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_started_event_details: Option<TaskStartedEventDetails>,
    /// The `taskSubmitFailedEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_submit_failed_event_details: Option<TaskSubmitFailedEventDetails>,
    /// The `taskSubmittedEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_submitted_event_details: Option<TaskSubmittedEventDetails>,
    /// The `taskSucceededEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_succeeded_event_details: Option<TaskSucceededEventDetails>,
    /// The `taskTimedOutEventDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_timed_out_event_details: Option<TaskTimedOutEventDetails>,
    /// The `timestamp` wire member.
    #[serde(with = "crate::epoch_seconds")]
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// The `type` wire member.
    pub r#type: HistoryEventType,
}

impl std::fmt::Debug for HistoryEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HistoryEvent").finish_non_exhaustive()
    }
}
/// Step Functions `HistoryEventExecutionDataDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct HistoryEventExecutionDataDetails {
    /// The `truncated` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
}

impl std::fmt::Debug for HistoryEventExecutionDataDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HistoryEventExecutionDataDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `InspectionData`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
#[derive(typed_builder::TypedBuilder)]
pub struct InspectionData {
    /// The `afterArguments` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_arguments: Option<String>,
    /// The `afterInputPath` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_input_path: Option<String>,
    /// The `afterItemBatcher` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_item_batcher: Option<String>,
    /// The `afterItemSelector` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_item_selector: Option<String>,
    /// The `afterItemsPath` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_items_path: Option<String>,
    /// The `afterItemsPointer` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_items_pointer: Option<String>,
    /// The `afterParameters` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_parameters: Option<String>,
    /// The `afterResultPath` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_result_path: Option<String>,
    /// The `afterResultSelector` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_result_selector: Option<String>,
    /// The `errorDetails` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_details: Option<InspectionErrorDetails>,
    /// The `input` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// The `maxConcurrency` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_concurrency: Option<i32>,
    /// The `request` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<InspectionDataRequest>,
    /// The `response` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<InspectionDataResponse>,
    /// The `result` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// The `toleratedFailureCount` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tolerated_failure_count: Option<i32>,
    /// The `toleratedFailurePercentage` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tolerated_failure_percentage: Option<f32>,
    /// The `variables` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variables: Option<String>,
}

impl std::fmt::Debug for InspectionData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InspectionData").finish_non_exhaustive()
    }
}
/// Step Functions `InspectionDataRequest`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct InspectionDataRequest {
    /// The `body` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// The `headers` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<String>,
    /// The `method` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// The `protocol` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    /// The `url` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl std::fmt::Debug for InspectionDataRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InspectionDataRequest")
            .finish_non_exhaustive()
    }
}
/// Step Functions `InspectionDataResponse`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct InspectionDataResponse {
    /// The `body` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// The `headers` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<String>,
    /// The `protocol` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    /// The `statusCode` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_code: Option<String>,
    /// The `statusMessage` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
}

impl std::fmt::Debug for InspectionDataResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InspectionDataResponse")
            .finish_non_exhaustive()
    }
}
/// Step Functions `InspectionErrorDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct InspectionErrorDetails {
    /// The `catchIndex` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catch_index: Option<i32>,
    /// The `retryBackoffIntervalSeconds` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_backoff_interval_seconds: Option<i32>,
    /// The `retryIndex` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_index: Option<i32>,
}

impl std::fmt::Debug for InspectionErrorDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InspectionErrorDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `LambdaFunctionFailedEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct LambdaFunctionFailedEventDetails {
    /// The `cause` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// The `error` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl std::fmt::Debug for LambdaFunctionFailedEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LambdaFunctionFailedEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `LambdaFunctionScheduleFailedEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct LambdaFunctionScheduleFailedEventDetails {
    /// The `cause` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// The `error` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl std::fmt::Debug for LambdaFunctionScheduleFailedEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LambdaFunctionScheduleFailedEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `LambdaFunctionScheduledEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct LambdaFunctionScheduledEventDetails {
    /// The `input` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// The `inputDetails` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_details: Option<HistoryEventExecutionDataDetails>,
    /// The `resource` wire member.
    pub resource: String,
    /// The `taskCredentials` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_credentials: Option<TaskCredentials>,
    /// The `timeoutInSeconds` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_in_seconds: Option<i64>,
}

impl std::fmt::Debug for LambdaFunctionScheduledEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LambdaFunctionScheduledEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `LambdaFunctionStartFailedEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct LambdaFunctionStartFailedEventDetails {
    /// The `cause` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// The `error` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl std::fmt::Debug for LambdaFunctionStartFailedEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LambdaFunctionStartFailedEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `LambdaFunctionSucceededEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct LambdaFunctionSucceededEventDetails {
    /// The `output` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// The `outputDetails` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_details: Option<HistoryEventExecutionDataDetails>,
}

impl std::fmt::Debug for LambdaFunctionSucceededEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LambdaFunctionSucceededEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `LambdaFunctionTimedOutEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct LambdaFunctionTimedOutEventDetails {
    /// The `cause` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// The `error` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl std::fmt::Debug for LambdaFunctionTimedOutEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LambdaFunctionTimedOutEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `LogDestination`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct LogDestination {
    /// The `cloudWatchLogsLogGroup` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cloud_watch_logs_log_group: Option<CloudWatchLogsLogGroup>,
}

impl std::fmt::Debug for LogDestination {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogDestination").finish_non_exhaustive()
    }
}
/// Step Functions `LoggingConfiguration`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct LoggingConfiguration {
    /// The `destinations` wire member.
    #[serde(default)]
    pub destinations: Vec<LogDestination>,
    /// The `includeExecutionData` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_execution_data: Option<bool>,
    /// The `level` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<LogLevel>,
}

impl std::fmt::Debug for LoggingConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoggingConfiguration")
            .finish_non_exhaustive()
    }
}
/// Step Functions `MapIterationEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct MapIterationEventDetails {
    /// The `index` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<i32>,
    /// The `name` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl std::fmt::Debug for MapIterationEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MapIterationEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `MapRunExecutionCounts`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
#[derive(typed_builder::TypedBuilder)]
pub struct MapRunExecutionCounts {
    /// The `aborted` wire member.
    pub aborted: i64,
    /// The `failed` wire member.
    pub failed: i64,
    /// The `failuresNotRedrivable` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failures_not_redrivable: Option<i64>,
    /// The `pending` wire member.
    pub pending: i64,
    /// The `pendingRedrive` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_redrive: Option<i64>,
    /// The `resultsWritten` wire member.
    pub results_written: i64,
    /// The `running` wire member.
    pub running: i64,
    /// The `succeeded` wire member.
    pub succeeded: i64,
    /// The `timedOut` wire member.
    pub timed_out: i64,
    /// The `total` wire member.
    pub total: i64,
}

impl std::fmt::Debug for MapRunExecutionCounts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MapRunExecutionCounts")
            .finish_non_exhaustive()
    }
}
/// Step Functions `MapRunFailedEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct MapRunFailedEventDetails {
    /// The `cause` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// The `error` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl std::fmt::Debug for MapRunFailedEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MapRunFailedEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `MapRunItemCounts`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
#[derive(typed_builder::TypedBuilder)]
pub struct MapRunItemCounts {
    /// The `aborted` wire member.
    pub aborted: i64,
    /// The `failed` wire member.
    pub failed: i64,
    /// The `failuresNotRedrivable` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failures_not_redrivable: Option<i64>,
    /// The `pending` wire member.
    pub pending: i64,
    /// The `pendingRedrive` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_redrive: Option<i64>,
    /// The `resultsWritten` wire member.
    pub results_written: i64,
    /// The `running` wire member.
    pub running: i64,
    /// The `succeeded` wire member.
    pub succeeded: i64,
    /// The `timedOut` wire member.
    pub timed_out: i64,
    /// The `total` wire member.
    pub total: i64,
}

impl std::fmt::Debug for MapRunItemCounts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MapRunItemCounts").finish_non_exhaustive()
    }
}
/// Step Functions `MapRunListItem`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct MapRunListItem {
    /// The `executionArn` wire member.
    pub execution_arn: String,
    /// The `mapRunArn` wire member.
    pub map_run_arn: String,
    /// The `startDate` wire member.
    #[serde(with = "crate::epoch_seconds")]
    pub start_date: chrono::DateTime<chrono::Utc>,
    /// The `stateMachineArn` wire member.
    pub state_machine_arn: String,
    /// The `stopDate` wire member.
    #[serde(default, with = "crate::epoch_seconds::option")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_date: Option<chrono::DateTime<chrono::Utc>>,
}

impl std::fmt::Debug for MapRunListItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MapRunListItem").finish_non_exhaustive()
    }
}
/// Step Functions `MapRunRedrivenEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct MapRunRedrivenEventDetails {
    /// The `mapRunArn` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map_run_arn: Option<String>,
    /// The `redriveCount` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redrive_count: Option<i32>,
}

impl std::fmt::Debug for MapRunRedrivenEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MapRunRedrivenEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `MapRunStartedEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct MapRunStartedEventDetails {
    /// The `mapRunArn` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map_run_arn: Option<String>,
}

impl std::fmt::Debug for MapRunStartedEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MapRunStartedEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `MapStateStartedEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct MapStateStartedEventDetails {
    /// The `length` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length: Option<i32>,
}

impl std::fmt::Debug for MapStateStartedEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MapStateStartedEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `MockErrorOutput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct MockErrorOutput {
    /// The `cause` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// The `error` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl std::fmt::Debug for MockErrorOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockErrorOutput").finish_non_exhaustive()
    }
}
/// Step Functions `MockInput`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct MockInput {
    /// The `errorOutput` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_output: Option<MockErrorOutput>,
    /// The `fieldValidationMode` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_validation_mode: Option<MockResponseValidationMode>,
    /// The `result` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
}

impl std::fmt::Debug for MockInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockInput").finish_non_exhaustive()
    }
}
/// Step Functions `RoutingConfigurationListItem`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct RoutingConfigurationListItem {
    /// The `stateMachineVersionArn` wire member.
    pub state_machine_version_arn: String,
    /// The `weight` wire member.
    pub weight: i32,
}

impl std::fmt::Debug for RoutingConfigurationListItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoutingConfigurationListItem")
            .finish_non_exhaustive()
    }
}
/// Step Functions `StateEnteredEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct StateEnteredEventDetails {
    /// The `input` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// The `inputDetails` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_details: Option<HistoryEventExecutionDataDetails>,
    /// The `name` wire member.
    pub name: String,
}

impl std::fmt::Debug for StateEnteredEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateEnteredEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `StateExitedEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct StateExitedEventDetails {
    /// The `assignedVariables` wire member.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub assigned_variables: HashMap<String, String>,
    /// The `assignedVariablesDetails` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assigned_variables_details: Option<AssignedVariablesDetails>,
    /// The `name` wire member.
    pub name: String,
    /// The `output` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// The `outputDetails` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_details: Option<HistoryEventExecutionDataDetails>,
}

impl std::fmt::Debug for StateExitedEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateExitedEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `StateMachineAliasListItem`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct StateMachineAliasListItem {
    /// The `creationDate` wire member.
    #[serde(with = "crate::epoch_seconds")]
    pub creation_date: chrono::DateTime<chrono::Utc>,
    /// The `stateMachineAliasArn` wire member.
    pub state_machine_alias_arn: String,
}

impl std::fmt::Debug for StateMachineAliasListItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateMachineAliasListItem")
            .finish_non_exhaustive()
    }
}
/// Step Functions `StateMachineListItem`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct StateMachineListItem {
    /// The `creationDate` wire member.
    #[serde(with = "crate::epoch_seconds")]
    pub creation_date: chrono::DateTime<chrono::Utc>,
    /// The `name` wire member.
    pub name: String,
    /// The `stateMachineArn` wire member.
    pub state_machine_arn: String,
    /// The `type` wire member.
    pub r#type: StateMachineType,
}

impl std::fmt::Debug for StateMachineListItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateMachineListItem")
            .finish_non_exhaustive()
    }
}
/// Step Functions `StateMachineVersionListItem`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct StateMachineVersionListItem {
    /// The `creationDate` wire member.
    #[serde(with = "crate::epoch_seconds")]
    pub creation_date: chrono::DateTime<chrono::Utc>,
    /// The `stateMachineVersionArn` wire member.
    pub state_machine_version_arn: String,
}

impl std::fmt::Debug for StateMachineVersionListItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateMachineVersionListItem")
            .finish_non_exhaustive()
    }
}
/// Step Functions `Tag`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct Tag {
    /// The `key` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// The `value` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

impl std::fmt::Debug for Tag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tag").finish_non_exhaustive()
    }
}
/// Step Functions `TaskCredentials`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct TaskCredentials {
    /// The `roleArn` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_arn: Option<String>,
}

impl std::fmt::Debug for TaskCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskCredentials").finish_non_exhaustive()
    }
}
/// Step Functions `TaskFailedEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct TaskFailedEventDetails {
    /// The `cause` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// The `error` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The `resource` wire member.
    pub resource: String,
    /// The `resourceType` wire member.
    pub resource_type: String,
}

impl std::fmt::Debug for TaskFailedEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskFailedEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `TaskScheduledEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
#[derive(typed_builder::TypedBuilder)]
pub struct TaskScheduledEventDetails {
    /// The `heartbeatInSeconds` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heartbeat_in_seconds: Option<i64>,
    /// The `parameters` wire member.
    pub parameters: String,
    /// The `region` wire member.
    pub region: String,
    /// The `resource` wire member.
    pub resource: String,
    /// The `resourceType` wire member.
    pub resource_type: String,
    /// The `taskCredentials` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_credentials: Option<TaskCredentials>,
    /// The `timeoutInSeconds` wire member.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_in_seconds: Option<i64>,
}

impl std::fmt::Debug for TaskScheduledEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskScheduledEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `TaskStartFailedEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct TaskStartFailedEventDetails {
    /// The `cause` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// The `error` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The `resource` wire member.
    pub resource: String,
    /// The `resourceType` wire member.
    pub resource_type: String,
}

impl std::fmt::Debug for TaskStartFailedEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskStartFailedEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `TaskStartedEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct TaskStartedEventDetails {
    /// The `resource` wire member.
    pub resource: String,
    /// The `resourceType` wire member.
    pub resource_type: String,
}

impl std::fmt::Debug for TaskStartedEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskStartedEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `TaskSubmitFailedEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct TaskSubmitFailedEventDetails {
    /// The `cause` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// The `error` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The `resource` wire member.
    pub resource: String,
    /// The `resourceType` wire member.
    pub resource_type: String,
}

impl std::fmt::Debug for TaskSubmitFailedEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskSubmitFailedEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `TaskSubmittedEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct TaskSubmittedEventDetails {
    /// The `output` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// The `outputDetails` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_details: Option<HistoryEventExecutionDataDetails>,
    /// The `resource` wire member.
    pub resource: String,
    /// The `resourceType` wire member.
    pub resource_type: String,
}

impl std::fmt::Debug for TaskSubmittedEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskSubmittedEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `TaskSucceededEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct TaskSucceededEventDetails {
    /// The `output` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// The `outputDetails` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_details: Option<HistoryEventExecutionDataDetails>,
    /// The `resource` wire member.
    pub resource: String,
    /// The `resourceType` wire member.
    pub resource_type: String,
}

impl std::fmt::Debug for TaskSucceededEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskSucceededEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `TaskTimedOutEventDetails`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct TaskTimedOutEventDetails {
    /// The `cause` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// The `error` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The `resource` wire member.
    pub resource: String,
    /// The `resourceType` wire member.
    pub resource_type: String,
}

impl std::fmt::Debug for TaskTimedOutEventDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskTimedOutEventDetails")
            .finish_non_exhaustive()
    }
}
/// Step Functions `TestStateConfiguration`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct TestStateConfiguration {
    /// The `errorCausedByState` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_caused_by_state: Option<String>,
    /// The `mapItemReaderData` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map_item_reader_data: Option<String>,
    /// The `mapIterationFailureCount` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map_iteration_failure_count: Option<i32>,
    /// The `retrierRetryCount` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retrier_retry_count: Option<i32>,
}

impl std::fmt::Debug for TestStateConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestStateConfiguration")
            .finish_non_exhaustive()
    }
}
/// Step Functions `TracingConfiguration`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct TracingConfiguration {
    /// The `enabled` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

impl std::fmt::Debug for TracingConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TracingConfiguration")
            .finish_non_exhaustive()
    }
}
/// Step Functions `ValidateStateMachineDefinitionDiagnostic`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ValidateStateMachineDefinitionDiagnostic {
    /// The `code` wire member.
    pub code: String,
    /// The `location` wire member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// The `message` wire member.
    pub message: String,
    /// The `severity` wire member.
    pub severity: ValidateStateMachineDefinitionSeverity,
}

impl std::fmt::Debug for ValidateStateMachineDefinitionDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidateStateMachineDefinitionDiagnostic")
            .finish_non_exhaustive()
    }
}
