//! Auto-generated from AWS Secrets Manager Smithy model. DO NOT EDIT.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Secrets Manager FilterNameStringType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum FilterNameStringType {
    /// Default variant.
    #[default]
    #[serde(rename = "all")]
    All,
    #[serde(rename = "description")]
    Description,
    #[serde(rename = "name")]
    Name,
    #[serde(rename = "owning-service")]
    OwningService,
    #[serde(rename = "primary-region")]
    PrimaryRegion,
    #[serde(rename = "tag-key")]
    TagKey,
    #[serde(rename = "tag-value")]
    TagValue,
}

impl FilterNameStringType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Description => "description",
            Self::Name => "name",
            Self::OwningService => "owning-service",
            Self::PrimaryRegion => "primary-region",
            Self::TagKey => "tag-key",
            Self::TagValue => "tag-value",
        }
    }
}

impl std::fmt::Display for FilterNameStringType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for FilterNameStringType {
    fn from(s: &str) -> Self {
        match s {
            "all" => Self::All,
            "description" => Self::Description,
            "name" => Self::Name,
            "owning-service" => Self::OwningService,
            "primary-region" => Self::PrimaryRegion,
            "tag-key" => Self::TagKey,
            "tag-value" => Self::TagValue,
            _ => Self::default(),
        }
    }
}

/// Secrets Manager SortByType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum SortByType {
    /// Default variant.
    #[default]
    #[serde(rename = "created-date")]
    CreatedDate,
    #[serde(rename = "last-accessed-date")]
    LastAccessedDate,
    #[serde(rename = "last-changed-date")]
    LastChangedDate,
    #[serde(rename = "name")]
    Name,
}

impl SortByType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CreatedDate => "created-date",
            Self::LastAccessedDate => "last-accessed-date",
            Self::LastChangedDate => "last-changed-date",
            Self::Name => "name",
        }
    }
}

impl std::fmt::Display for SortByType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for SortByType {
    fn from(s: &str) -> Self {
        match s {
            "created-date" => Self::CreatedDate,
            "last-accessed-date" => Self::LastAccessedDate,
            "last-changed-date" => Self::LastChangedDate,
            "name" => Self::Name,
            _ => Self::default(),
        }
    }
}

/// Secrets Manager SortOrderType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum SortOrderType {
    /// Default variant.
    #[default]
    #[serde(rename = "asc")]
    Asc,
    #[serde(rename = "desc")]
    Desc,
}

impl SortOrderType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Asc => "asc",
            Self::Desc => "desc",
        }
    }
}

impl std::fmt::Display for SortOrderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for SortOrderType {
    fn from(s: &str) -> Self {
        match s {
            "asc" => Self::Asc,
            "desc" => Self::Desc,
            _ => Self::default(),
        }
    }
}

/// Secrets Manager StatusType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum StatusType {
    /// Default variant.
    #[default]
    Failed,
    InProgress,
    InSync,
}

impl StatusType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Failed => "Failed",
            Self::InProgress => "InProgress",
            Self::InSync => "InSync",
        }
    }
}

impl std::fmt::Display for StatusType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for StatusType {
    fn from(s: &str) -> Self {
        match s {
            "Failed" => Self::Failed,
            "InProgress" => Self::InProgress,
            "InSync" => Self::InSync,
            _ => Self::default(),
        }
    }
}

/// Secrets Manager APIErrorType.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct APIErrorType {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_id: Option<String>,
}

/// Secrets Manager ExternalSecretRotationMetadataItem.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ExternalSecretRotationMetadataItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

/// Secrets Manager Filter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Filter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<FilterNameStringType>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
}

/// Secrets Manager ReplicaRegionType.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ReplicaRegionType {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kms_key_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
}

/// Secrets Manager ReplicationStatusType.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ReplicationStatusType {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kms_key_id: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::epoch_seconds::option::serialize",
        deserialize_with = "crate::epoch_seconds::option::deserialize"
    )]
    pub last_accessed_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<StatusType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
}

/// Secrets Manager RotationRulesType.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RotationRulesType {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub automatically_after_days: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule_expression: Option<String>,
}

/// Secrets Manager SecretListEntry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SecretListEntry {
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation_enabled: Option<bool>,
    #[serde(rename = "RotationLambdaARN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation_lambda_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation_rules: Option<RotationRulesType>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub secret_versions_to_stages: HashMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Tag>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
}

/// Secrets Manager SecretValueEntry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SecretValueEntry {
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

/// Secrets Manager SecretVersionsListEntry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SecretVersionsListEntry {
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::epoch_seconds::option::serialize",
        deserialize_with = "crate::epoch_seconds::option::deserialize"
    )]
    pub created_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kms_key_ids: Vec<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::epoch_seconds::option::serialize",
        deserialize_with = "crate::epoch_seconds::option::deserialize"
    )]
    pub last_accessed_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub version_stages: Vec<String>,
}

/// Secrets Manager Tag.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Tag {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

/// Secrets Manager ValidationErrorsEntry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ValidationErrorsEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}
