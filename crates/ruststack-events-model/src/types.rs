//! Shared EventBridge types used across input, output, and internal representations.
//!
//! All types follow the EventBridge JSON wire format with `PascalCase` field names.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Tags
// ---------------------------------------------------------------------------

/// A tag associated with a resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Tag {
    /// The tag key.
    pub key: String,
    /// The tag value.
    pub value: String,
}

// ---------------------------------------------------------------------------
// Targets
// ---------------------------------------------------------------------------

/// A target for a rule, describing where matched events are sent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Target {
    /// The unique target identifier within the rule.
    pub id: String,

    /// The ARN of the target resource.
    pub arn: String,

    /// The IAM role ARN for EventBridge to use when invoking the target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_arn: Option<String>,

    /// Valid JSON text passed to the target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,

    /// JSONPath expression to select part of the event to pass to the target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_path: Option<String>,

    /// Settings to transform input before sending to the target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_transformer: Option<InputTransformer>,

    /// Parameters for a Run Command target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_command_parameters: Option<Value>,

    /// Parameters for an ECS task target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ecs_parameters: Option<Value>,

    /// Parameters for an AWS Batch job target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_parameters: Option<Value>,

    /// Parameters for an SQS queue target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sqs_parameters: Option<Value>,

    /// Parameters for an HTTP endpoint target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_parameters: Option<Value>,

    /// Parameters for a Redshift Data API target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redshift_data_parameters: Option<Value>,

    /// Parameters for a SageMaker pipeline target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sage_maker_pipeline_parameters: Option<Value>,

    /// Dead-letter queue configuration for failed invocations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dead_letter_config: Option<DeadLetterConfig>,

    /// Retry policy for failed invocations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_policy: Option<RetryPolicy>,

    /// Parameters for an AppSync target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_sync_parameters: Option<Value>,
}

// ---------------------------------------------------------------------------
// Input transformer
// ---------------------------------------------------------------------------

/// Settings to transform input before sending to the target.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct InputTransformer {
    /// Map of JSON paths to extract from the event.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub input_paths_map: HashMap<String, String>,

    /// Template string that uses extracted values.
    pub input_template: String,
}

// ---------------------------------------------------------------------------
// Dead-letter config
// ---------------------------------------------------------------------------

/// Dead-letter queue configuration for a target.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeadLetterConfig {
    /// The ARN of the SQS queue used as the dead-letter queue.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
}

// ---------------------------------------------------------------------------
// Retry policy
// ---------------------------------------------------------------------------

/// Retry policy for a target invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RetryPolicy {
    /// Maximum number of retry attempts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum_retry_attempts: Option<i32>,

    /// Maximum age of a request that EventBridge sends to a target (in seconds).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum_event_age_in_seconds: Option<i32>,
}

// ---------------------------------------------------------------------------
// Rule (list output)
// ---------------------------------------------------------------------------

/// A rule returned by `ListRules`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Rule {
    /// The name of the rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// The ARN of the rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,

    /// The event pattern for the rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_pattern: Option<String>,

    /// The schedule expression for the rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule_expression: Option<String>,

    /// The state of the rule (`ENABLED` or `DISABLED`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,

    /// The description of the rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// The IAM role ARN associated with the rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_arn: Option<String>,

    /// If the rule was created by an AWS service on behalf of your account.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managed_by: Option<String>,

    /// The name of the event bus associated with the rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_bus_name: Option<String>,
}

// ---------------------------------------------------------------------------
// EventBus (list output)
// ---------------------------------------------------------------------------

/// An event bus returned by `ListEventBuses`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EventBus {
    /// The name of the event bus.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// The ARN of the event bus.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,

    /// The description of the event bus.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// The policy for the event bus.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
}

// ---------------------------------------------------------------------------
// Result entries
// ---------------------------------------------------------------------------

/// An entry in a `PutEvents` response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutEventsResultEntry {
    /// The ID of the event that was successfully submitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,

    /// The error code if the event was not submitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,

    /// The error message if the event was not submitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// An entry in a `PutTargets` response indicating a failed target.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutTargetsResultEntry {
    /// The ID of the target that failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,

    /// The error code for the failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,

    /// The error message for the failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// An entry in a `RemoveTargets` response indicating a failed removal.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RemoveTargetsResultEntry {
    /// The ID of the target that failed to be removed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,

    /// The error code for the failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,

    /// The error message for the failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

// ---------------------------------------------------------------------------
// Condition
// ---------------------------------------------------------------------------

/// A condition used in `PutPermission` for policy-based access control.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Condition {
    /// The type of condition (e.g., `"StringEquals"`).
    #[serde(rename = "Type")]
    pub condition_type: String,

    /// The key for the condition (e.g., `"aws:PrincipalOrgID"`).
    pub key: String,

    /// The value for the condition.
    pub value: String,
}
