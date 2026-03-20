//! Auto-generated from AWS STS Smithy model. DO NOT EDIT.

use serde::{Deserialize, Serialize};

use crate::types::{PolicyDescriptorType, ProvidedContext, Tag};

/// STS AssumeRoleInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AssumeRoleInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub policy_arns: Vec<PolicyDescriptorType>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provided_contexts: Vec<ProvidedContext>,
    pub role_arn: String,
    pub role_session_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_identity: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Tag>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_code: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transitive_tag_keys: Vec<String>,
}

/// STS AssumeRoleWithSAMLInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AssumeRoleWithSAMLInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub policy_arns: Vec<PolicyDescriptorType>,
    pub principal_arn: String,
    pub role_arn: String,
    #[serde(rename = "SAMLAssertion")]
    pub saml_assertion: String,
}

/// STS AssumeRoleWithWebIdentityInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AssumeRoleWithWebIdentityInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub policy_arns: Vec<PolicyDescriptorType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    pub role_arn: String,
    pub role_session_name: String,
    pub web_identity_token: String,
}

/// STS DecodeAuthorizationMessageInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DecodeAuthorizationMessageInput {
    pub encoded_message: String,
}

/// STS GetAccessKeyInfoInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetAccessKeyInfoInput {
    pub access_key_id: String,
}

/// STS GetCallerIdentityInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetCallerIdentityInput {}

/// STS GetFederationTokenInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetFederationTokenInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<i32>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub policy_arns: Vec<PolicyDescriptorType>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Tag>,
}

/// STS GetSessionTokenInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetSessionTokenInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_code: Option<String>,
}
