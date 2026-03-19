//! Auto-generated from AWS Secrets Manager Smithy model. DO NOT EDIT.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::types::{
    APIErrorType, ExternalSecretRotationMetadataItem, ReplicationStatusType, RotationRulesType,
    SecretListEntry, SecretValueEntry, SecretVersionsListEntry, Tag, ValidationErrorsEntry,
};

/// Secrets Manager BatchGetSecretValueResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct BatchGetSecretValueResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<APIErrorType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secret_values: Vec<SecretValueEntry>,
}

/// Secrets Manager CancelRotateSecretResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CancelRotateSecretResponse {
    #[serde(rename = "ARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_id: Option<String>,
}

/// Secrets Manager CreateSecretResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CreateSecretResponse {
    #[serde(rename = "ARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replication_status: Vec<ReplicationStatusType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_id: Option<String>,
}

/// Secrets Manager DeleteResourcePolicyResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteResourcePolicyResponse {
    #[serde(rename = "ARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Secrets Manager DeleteSecretResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteSecretResponse {
    #[serde(rename = "ARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::epoch_seconds::option::serialize",
        deserialize_with = "crate::epoch_seconds::option::deserialize"
    )]
    pub deletion_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Secrets Manager DescribeSecretResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeSecretResponse {
    #[serde(rename = "ARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::epoch_seconds::option::serialize",
        deserialize_with = "crate::epoch_seconds::option::deserialize"
    )]
    pub created_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::epoch_seconds::option::serialize",
        deserialize_with = "crate::epoch_seconds::option::deserialize"
    )]
    pub deleted_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub external_secret_rotation_metadata: Vec<ExternalSecretRotationMetadataItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_secret_rotation_role_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kms_key_id: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::epoch_seconds::option::serialize",
        deserialize_with = "crate::epoch_seconds::option::deserialize"
    )]
    pub last_accessed_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::epoch_seconds::option::serialize",
        deserialize_with = "crate::epoch_seconds::option::deserialize"
    )]
    pub last_changed_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::epoch_seconds::option::serialize",
        deserialize_with = "crate::epoch_seconds::option::deserialize"
    )]
    pub last_rotated_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::epoch_seconds::option::serialize",
        deserialize_with = "crate::epoch_seconds::option::deserialize"
    )]
    pub next_rotation_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owning_service: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_region: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replication_status: Vec<ReplicationStatusType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation_enabled: Option<bool>,
    #[serde(rename = "RotationLambdaARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation_lambda_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation_rules: Option<RotationRulesType>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Tag>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub version_ids_to_stages: HashMap<String, Vec<String>>,
}

/// Secrets Manager GetRandomPasswordResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetRandomPasswordResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub random_password: Option<String>,
}

/// Secrets Manager GetResourcePolicyResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetResourcePolicyResponse {
    #[serde(rename = "ARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_policy: Option<String>,
}

/// Secrets Manager GetSecretValueResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetSecretValueResponse {
    #[serde(rename = "ARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::epoch_seconds::option::serialize",
        deserialize_with = "crate::epoch_seconds::option::deserialize"
    )]
    pub created_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        with = "crate::blob::option"
    )]
    pub secret_binary: Option<bytes::Bytes>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_string: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub version_stages: Vec<String>,
}

/// Secrets Manager ListSecretVersionIdsResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListSecretVersionIdsResponse {
    #[serde(rename = "ARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub versions: Vec<SecretVersionsListEntry>,
}

/// Secrets Manager ListSecretsResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListSecretsResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secret_list: Vec<SecretListEntry>,
}

/// Secrets Manager PutResourcePolicyResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutResourcePolicyResponse {
    #[serde(rename = "ARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Secrets Manager PutSecretValueResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutSecretValueResponse {
    #[serde(rename = "ARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub version_stages: Vec<String>,
}

/// Secrets Manager RemoveRegionsFromReplicationResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RemoveRegionsFromReplicationResponse {
    #[serde(rename = "ARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replication_status: Vec<ReplicationStatusType>,
}

/// Secrets Manager ReplicateSecretToRegionsResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ReplicateSecretToRegionsResponse {
    #[serde(rename = "ARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replication_status: Vec<ReplicationStatusType>,
}

/// Secrets Manager RestoreSecretResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RestoreSecretResponse {
    #[serde(rename = "ARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Secrets Manager RotateSecretResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RotateSecretResponse {
    #[serde(rename = "ARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_id: Option<String>,
}

/// Secrets Manager StopReplicationToReplicaResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct StopReplicationToReplicaResponse {
    #[serde(rename = "ARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
}

/// Secrets Manager UpdateSecretResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UpdateSecretResponse {
    #[serde(rename = "ARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_id: Option<String>,
}

/// Secrets Manager UpdateSecretVersionStageResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UpdateSecretVersionStageResponse {
    #[serde(rename = "ARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Secrets Manager ValidateResourcePolicyResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ValidateResourcePolicyResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_validation_passed: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation_errors: Vec<ValidationErrorsEntry>,
}
