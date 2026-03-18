//! EventBridge output types for Phase 0 through Phase 3 operations.
//!
//! All output structs use `PascalCase` JSON field naming to match the EventBridge
//! wire protocol (`awsJson1_1`). Optional fields are omitted when `None`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::{
    DeadLetterConfig, EventBus, PutEventsResultEntry, PutTargetsResultEntry,
    RemoveTargetsResultEntry, Rule, Tag, Target,
};

// ===========================================================================
// Phase 0
// ===========================================================================

// ---------------------------------------------------------------------------
// CreateEventBus
// ---------------------------------------------------------------------------

/// Output for the `CreateEventBus` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CreateEventBusOutput {
    /// The ARN of the new event bus.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_bus_arn: Option<String>,

    /// The description of the event bus.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// The dead-letter queue configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dead_letter_config: Option<DeadLetterConfig>,

    /// The KMS key identifier for encryption.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kms_key_identifier: Option<String>,
}

// ---------------------------------------------------------------------------
// DeleteEventBus
// ---------------------------------------------------------------------------

/// Output for the `DeleteEventBus` operation (empty).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteEventBusOutput {}

// ---------------------------------------------------------------------------
// DescribeEventBus
// ---------------------------------------------------------------------------

/// Output for the `DescribeEventBus` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeEventBusOutput {
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

    /// The dead-letter queue configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dead_letter_config: Option<DeadLetterConfig>,

    /// The KMS key identifier for encryption.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kms_key_identifier: Option<String>,
}

// ---------------------------------------------------------------------------
// ListEventBuses
// ---------------------------------------------------------------------------

/// Output for the `ListEventBuses` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListEventBusesOutput {
    /// The event buses.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_buses: Vec<EventBus>,

    /// The token for the next page of results, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

// ---------------------------------------------------------------------------
// PutRule
// ---------------------------------------------------------------------------

/// Output for the `PutRule` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutRuleOutput {
    /// The ARN of the rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_arn: Option<String>,
}

// ---------------------------------------------------------------------------
// DeleteRule
// ---------------------------------------------------------------------------

/// Output for the `DeleteRule` operation (empty).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteRuleOutput {}

// ---------------------------------------------------------------------------
// DescribeRule
// ---------------------------------------------------------------------------

/// Output for the `DescribeRule` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeRuleOutput {
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

    /// The AWS service that manages the rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managed_by: Option<String>,

    /// The name of the event bus associated with the rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_bus_name: Option<String>,

    /// The account ID of the creator of the rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
}

// ---------------------------------------------------------------------------
// ListRules
// ---------------------------------------------------------------------------

/// Output for the `ListRules` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListRulesOutput {
    /// The rules that match the request.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<Rule>,

    /// The token for the next page of results, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

// ---------------------------------------------------------------------------
// EnableRule
// ---------------------------------------------------------------------------

/// Output for the `EnableRule` operation (empty).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EnableRuleOutput {}

// ---------------------------------------------------------------------------
// DisableRule
// ---------------------------------------------------------------------------

/// Output for the `DisableRule` operation (empty).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DisableRuleOutput {}

// ---------------------------------------------------------------------------
// PutTargets
// ---------------------------------------------------------------------------

/// Output for the `PutTargets` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutTargetsOutput {
    /// The number of failed entries.
    pub failed_entry_count: i32,

    /// The failed entries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed_entries: Vec<PutTargetsResultEntry>,
}

// ---------------------------------------------------------------------------
// RemoveTargets
// ---------------------------------------------------------------------------

/// Output for the `RemoveTargets` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RemoveTargetsOutput {
    /// The number of failed entries.
    pub failed_entry_count: i32,

    /// The failed entries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed_entries: Vec<RemoveTargetsResultEntry>,
}

// ---------------------------------------------------------------------------
// ListTargetsByRule
// ---------------------------------------------------------------------------

/// Output for the `ListTargetsByRule` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListTargetsByRuleOutput {
    /// The targets assigned to the rule.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<Target>,

    /// The token for the next page of results, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

// ---------------------------------------------------------------------------
// PutEvents
// ---------------------------------------------------------------------------

/// Output for the `PutEvents` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutEventsOutput {
    /// The number of failed entries.
    pub failed_entry_count: i32,

    /// The result entries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<PutEventsResultEntry>,
}

// ---------------------------------------------------------------------------
// TestEventPattern
// ---------------------------------------------------------------------------

/// Output for the `TestEventPattern` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TestEventPatternOutput {
    /// Whether the event matches the pattern.
    pub result: bool,
}

// ===========================================================================
// Phase 1
// ===========================================================================

// ---------------------------------------------------------------------------
// TagResource
// ---------------------------------------------------------------------------

/// Output for the `TagResource` operation (empty).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TagResourceOutput {}

// ---------------------------------------------------------------------------
// UntagResource
// ---------------------------------------------------------------------------

/// Output for the `UntagResource` operation (empty).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UntagResourceOutput {}

// ---------------------------------------------------------------------------
// ListTagsForResource
// ---------------------------------------------------------------------------

/// Output for the `ListTagsForResource` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListTagsForResourceOutput {
    /// The tags associated with the resource.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Tag>,
}

// ---------------------------------------------------------------------------
// PutPermission
// ---------------------------------------------------------------------------

/// Output for the `PutPermission` operation (empty).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutPermissionOutput {}

// ---------------------------------------------------------------------------
// RemovePermission
// ---------------------------------------------------------------------------

/// Output for the `RemovePermission` operation (empty).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RemovePermissionOutput {}

// ---------------------------------------------------------------------------
// ListRuleNamesByTarget
// ---------------------------------------------------------------------------

/// Output for the `ListRuleNamesByTarget` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListRuleNamesByTargetOutput {
    /// The names of the rules that reference the specified target.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rule_names: Vec<String>,

    /// The token for the next page of results, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

// ===========================================================================
// Phase 2
// ===========================================================================

// ---------------------------------------------------------------------------
// UpdateEventBus
// ---------------------------------------------------------------------------

/// Output for the `UpdateEventBus` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UpdateEventBusOutput {
    /// The ARN of the updated event bus.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,

    /// The name of the updated event bus.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// The description of the updated event bus.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// The dead-letter queue configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dead_letter_config: Option<DeadLetterConfig>,

    /// The KMS key identifier for encryption.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kms_key_identifier: Option<String>,
}

// ===========================================================================
// Phase 3 (stubs)
// ===========================================================================

/// Generic output wrapper for stub operations that return raw JSON.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GenericOutput {
    /// The raw JSON value.
    pub value: Value,
}
