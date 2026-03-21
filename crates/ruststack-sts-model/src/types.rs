//! Auto-generated from AWS STS Smithy model. DO NOT EDIT.

use serde::{Deserialize, Serialize};

/// STS AssumedRoleUser.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AssumedRoleUser {
    pub arn: String,
    pub assumed_role_id: String,
}

/// STS Credentials.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Credentials {
    pub access_key_id: String,
    pub expiration: chrono::DateTime<chrono::Utc>,
    pub secret_access_key: String,
    pub session_token: String,
}

/// STS FederatedUser.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct FederatedUser {
    pub arn: String,
    pub federated_user_id: String,
}

/// STS PolicyDescriptorType.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PolicyDescriptorType {
    #[serde(rename = "arn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
}

/// STS ProvidedContext.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ProvidedContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_assertion: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_arn: Option<String>,
}

/// STS Tag.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Tag {
    pub key: String,
    pub value: String,
}
