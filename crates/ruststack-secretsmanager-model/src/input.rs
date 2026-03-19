//! Auto-generated from AWS Secrets Manager Smithy model. DO NOT EDIT.

use serde::{Deserialize, Serialize};

use crate::types::{
    ExternalSecretRotationMetadataItem, Filter, ReplicaRegionType, RotationRulesType, SortByType,
    SortOrderType, Tag,
};

/// Secrets Manager BatchGetSecretValueInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct BatchGetSecretValueInput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filters: Vec<Filter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secret_id_list: Vec<String>,
}

/// Secrets Manager CancelRotateSecretInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CancelRotateSecretInput {
    pub secret_id: String,
}

/// Secrets Manager CreateSecretInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CreateSecretInput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add_replica_regions: Vec<ReplicaRegionType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_request_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub force_overwrite_replica_secret: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kms_key_id: Option<String>,
    pub name: String,
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        with = "crate::blob::option"
    )]
    pub secret_binary: Option<bytes::Bytes>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_string: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Tag>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
}

/// Secrets Manager DeleteResourcePolicyInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteResourcePolicyInput {
    pub secret_id: String,
}

/// Secrets Manager DeleteSecretInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteSecretInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub force_delete_without_recovery: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_window_in_days: Option<i64>,
    pub secret_id: String,
}

/// Secrets Manager DescribeSecretInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeSecretInput {
    pub secret_id: String,
}

/// Secrets Manager GetRandomPasswordInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetRandomPasswordInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_characters: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_lowercase: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_numbers: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_punctuation: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_uppercase: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_space: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password_length: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub require_each_included_type: Option<bool>,
}

/// Secrets Manager GetResourcePolicyInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetResourcePolicyInput {
    pub secret_id: String,
}

/// Secrets Manager GetSecretValueInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetSecretValueInput {
    pub secret_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_stage: Option<String>,
}

/// Secrets Manager ListSecretVersionIdsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListSecretVersionIdsInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_deprecated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    pub secret_id: String,
}

/// Secrets Manager ListSecretsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListSecretsInput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filters: Vec<Filter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_planned_deletion: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_by: Option<SortByType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_order: Option<SortOrderType>,
}

/// Secrets Manager PutResourcePolicyInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutResourcePolicyInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_public_policy: Option<bool>,
    pub resource_policy: String,
    pub secret_id: String,
}

/// Secrets Manager PutSecretValueInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutSecretValueInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_request_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation_token: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        with = "crate::blob::option"
    )]
    pub secret_binary: Option<bytes::Bytes>,
    pub secret_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_string: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub version_stages: Vec<String>,
}

/// Secrets Manager RemoveRegionsFromReplicationInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RemoveRegionsFromReplicationInput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remove_replica_regions: Vec<String>,
    pub secret_id: String,
}

/// Secrets Manager ReplicateSecretToRegionsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ReplicateSecretToRegionsInput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add_replica_regions: Vec<ReplicaRegionType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub force_overwrite_replica_secret: Option<bool>,
    pub secret_id: String,
}

/// Secrets Manager RestoreSecretInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RestoreSecretInput {
    pub secret_id: String,
}

/// Secrets Manager RotateSecretInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RotateSecretInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_request_token: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub external_secret_rotation_metadata: Vec<ExternalSecretRotationMetadataItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_secret_rotation_role_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotate_immediately: Option<bool>,
    #[serde(rename = "RotationLambdaARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation_lambda_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation_rules: Option<RotationRulesType>,
    pub secret_id: String,
}

/// Secrets Manager StopReplicationToReplicaInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct StopReplicationToReplicaInput {
    pub secret_id: String,
}

/// Secrets Manager TagResourceInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TagResourceInput {
    pub secret_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Tag>,
}

/// Secrets Manager UntagResourceInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UntagResourceInput {
    pub secret_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tag_keys: Vec<String>,
}

/// Secrets Manager UpdateSecretInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UpdateSecretInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_request_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kms_key_id: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        with = "crate::blob::option"
    )]
    pub secret_binary: Option<bytes::Bytes>,
    pub secret_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_string: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
}

/// Secrets Manager UpdateSecretVersionStageInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UpdateSecretVersionStageInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub move_to_version_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remove_from_version_id: Option<String>,
    pub secret_id: String,
    pub version_stage: String,
}

/// Secrets Manager ValidateResourcePolicyInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ValidateResourcePolicyInput {
    pub resource_policy: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_id: Option<String>,
}
