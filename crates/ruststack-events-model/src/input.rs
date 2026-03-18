//! EventBridge input types for Phase 0 through Phase 3 operations.
//!
//! All input structs use `PascalCase` JSON field naming to match the EventBridge
//! wire protocol (`awsJson1_1`). Optional fields are omitted when `None`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::{Condition, DeadLetterConfig, Tag, Target};

// ===========================================================================
// Phase 0
// ===========================================================================

// ---------------------------------------------------------------------------
// CreateEventBus
// ---------------------------------------------------------------------------

/// Input for the `CreateEventBus` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CreateEventBusInput {
    /// The name of the new event bus.
    pub name: String,

    /// A description of the event bus.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Tags to associate with the event bus.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Tag>,

    /// The name of the partner event source to associate with the event bus.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_source_name: Option<String>,

    /// Dead-letter queue configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dead_letter_config: Option<DeadLetterConfig>,

    /// The KMS key identifier for encryption.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kms_key_identifier: Option<String>,
}

// ---------------------------------------------------------------------------
// DeleteEventBus
// ---------------------------------------------------------------------------

/// Input for the `DeleteEventBus` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteEventBusInput {
    /// The name of the event bus to delete.
    pub name: String,
}

// ---------------------------------------------------------------------------
// DescribeEventBus
// ---------------------------------------------------------------------------

/// Input for the `DescribeEventBus` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeEventBusInput {
    /// The name of the event bus to describe. Defaults to the default event bus.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

// ---------------------------------------------------------------------------
// ListEventBuses
// ---------------------------------------------------------------------------

/// Input for the `ListEventBuses` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListEventBusesInput {
    /// Prefix to filter event bus names.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_prefix: Option<String>,

    /// The token for the next set of results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,

    /// The maximum number of results to return.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
}

// ---------------------------------------------------------------------------
// PutRule
// ---------------------------------------------------------------------------

/// Input for the `PutRule` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutRuleInput {
    /// The name of the rule.
    pub name: String,

    /// The event pattern in JSON format.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_pattern: Option<String>,

    /// The schedule expression (e.g., `"rate(5 minutes)"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule_expression: Option<String>,

    /// The state of the rule (`ENABLED` or `DISABLED`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,

    /// A description of the rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// The IAM role ARN associated with the rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_arn: Option<String>,

    /// Tags to associate with the rule.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Tag>,

    /// The name of the event bus to associate with the rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_bus_name: Option<String>,
}

// ---------------------------------------------------------------------------
// DeleteRule
// ---------------------------------------------------------------------------

/// Input for the `DeleteRule` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteRuleInput {
    /// The name of the rule to delete.
    pub name: String,

    /// The name of the event bus associated with the rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_bus_name: Option<String>,

    /// Whether to force-delete a managed rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub force: Option<bool>,
}

// ---------------------------------------------------------------------------
// DescribeRule
// ---------------------------------------------------------------------------

/// Input for the `DescribeRule` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeRuleInput {
    /// The name of the rule to describe.
    pub name: String,

    /// The name of the event bus associated with the rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_bus_name: Option<String>,
}

// ---------------------------------------------------------------------------
// ListRules
// ---------------------------------------------------------------------------

/// Input for the `ListRules` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListRulesInput {
    /// Prefix to filter rule names.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_prefix: Option<String>,

    /// The name of the event bus associated with the rules.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_bus_name: Option<String>,

    /// The token for the next set of results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,

    /// The maximum number of results to return.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
}

// ---------------------------------------------------------------------------
// EnableRule
// ---------------------------------------------------------------------------

/// Input for the `EnableRule` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EnableRuleInput {
    /// The name of the rule to enable.
    pub name: String,

    /// The name of the event bus associated with the rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_bus_name: Option<String>,
}

// ---------------------------------------------------------------------------
// DisableRule
// ---------------------------------------------------------------------------

/// Input for the `DisableRule` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DisableRuleInput {
    /// The name of the rule to disable.
    pub name: String,

    /// The name of the event bus associated with the rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_bus_name: Option<String>,
}

// ---------------------------------------------------------------------------
// PutTargets
// ---------------------------------------------------------------------------

/// Input for the `PutTargets` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutTargetsInput {
    /// The name of the rule to add targets to.
    pub rule: String,

    /// The targets to add to the rule.
    #[serde(default)]
    pub targets: Vec<Target>,

    /// The name of the event bus associated with the rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_bus_name: Option<String>,
}

// ---------------------------------------------------------------------------
// RemoveTargets
// ---------------------------------------------------------------------------

/// Input for the `RemoveTargets` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RemoveTargetsInput {
    /// The name of the rule to remove targets from.
    pub rule: String,

    /// The IDs of the targets to remove.
    #[serde(default)]
    pub ids: Vec<String>,

    /// The name of the event bus associated with the rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_bus_name: Option<String>,

    /// Whether to force-remove targets from a managed rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub force: Option<bool>,
}

// ---------------------------------------------------------------------------
// ListTargetsByRule
// ---------------------------------------------------------------------------

/// Input for the `ListTargetsByRule` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListTargetsByRuleInput {
    /// The name of the rule whose targets to list.
    pub rule: String,

    /// The name of the event bus associated with the rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_bus_name: Option<String>,

    /// The token for the next set of results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,

    /// The maximum number of results to return.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
}

// ---------------------------------------------------------------------------
// PutEvents
// ---------------------------------------------------------------------------

/// Input for the `PutEvents` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutEventsInput {
    /// The entries to publish to the event bus.
    #[serde(default)]
    pub entries: Vec<PutEventsRequestEntry>,

    /// The URL subdomain of the endpoint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint_id: Option<String>,
}

/// An event entry for the `PutEvents` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutEventsRequestEntry {
    /// The source of the event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,

    /// The detail type of the event.
    #[serde(rename = "DetailType", skip_serializing_if = "Option::is_none")]
    pub detail_type: Option<String>,

    /// The event detail as a JSON string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,

    /// AWS resources involved in the event.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<String>,

    /// The timestamp of the event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,

    /// The event bus to publish the event to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_bus_name: Option<String>,

    /// An AWS X-Ray trace header.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_header: Option<String>,
}

// ---------------------------------------------------------------------------
// TestEventPattern
// ---------------------------------------------------------------------------

/// Input for the `TestEventPattern` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TestEventPatternInput {
    /// The event pattern to test.
    pub event_pattern: String,

    /// The event to test against the pattern, in JSON format.
    pub event: String,
}

// ===========================================================================
// Phase 1
// ===========================================================================

// ---------------------------------------------------------------------------
// TagResource
// ---------------------------------------------------------------------------

/// Input for the `TagResource` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TagResourceInput {
    /// The ARN of the resource to tag.
    #[serde(rename = "ResourceARN")]
    pub resource_arn: String,

    /// The tags to associate with the resource.
    #[serde(default)]
    pub tags: Vec<Tag>,
}

// ---------------------------------------------------------------------------
// UntagResource
// ---------------------------------------------------------------------------

/// Input for the `UntagResource` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UntagResourceInput {
    /// The ARN of the resource to untag.
    #[serde(rename = "ResourceARN")]
    pub resource_arn: String,

    /// The tag keys to remove.
    #[serde(default)]
    pub tag_keys: Vec<String>,
}

// ---------------------------------------------------------------------------
// ListTagsForResource
// ---------------------------------------------------------------------------

/// Input for the `ListTagsForResource` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListTagsForResourceInput {
    /// The ARN of the resource whose tags to list.
    #[serde(rename = "ResourceARN")]
    pub resource_arn: String,
}

// ---------------------------------------------------------------------------
// PutPermission
// ---------------------------------------------------------------------------

/// Input for the `PutPermission` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutPermissionInput {
    /// The name of the event bus to modify.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_bus_name: Option<String>,

    /// The action to allow (e.g., `events:PutEvents`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,

    /// The AWS account ID or `*` for all accounts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub principal: Option<String>,

    /// An identifier for the statement in the policy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub statement_id: Option<String>,

    /// A condition for the permission.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<Condition>,

    /// A full JSON policy to set on the event bus.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
}

// ---------------------------------------------------------------------------
// RemovePermission
// ---------------------------------------------------------------------------

/// Input for the `RemovePermission` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RemovePermissionInput {
    /// The name of the event bus to modify.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_bus_name: Option<String>,

    /// The statement ID of the permission to remove.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub statement_id: Option<String>,

    /// Whether to remove all permissions from the event bus.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remove_all_permissions: Option<bool>,
}

// ---------------------------------------------------------------------------
// ListRuleNamesByTarget
// ---------------------------------------------------------------------------

/// Input for the `ListRuleNamesByTarget` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListRuleNamesByTargetInput {
    /// The ARN of the target to list rules for.
    pub target_arn: String,

    /// The name of the event bus associated with the rules.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_bus_name: Option<String>,

    /// The token for the next set of results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,

    /// The maximum number of results to return.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
}

// ===========================================================================
// Phase 2
// ===========================================================================

// ---------------------------------------------------------------------------
// UpdateEventBus
// ---------------------------------------------------------------------------

/// Input for the `UpdateEventBus` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UpdateEventBusInput {
    /// The name of the event bus to update.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// The updated description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// The updated dead-letter queue configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dead_letter_config: Option<DeadLetterConfig>,

    /// The updated KMS key identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kms_key_identifier: Option<String>,
}

// ===========================================================================
// Phase 3 (stubs)
// ===========================================================================

/// Generic input wrapper for stub operations that pass through raw JSON.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GenericInput {
    /// The raw JSON value.
    pub value: Value,
}
