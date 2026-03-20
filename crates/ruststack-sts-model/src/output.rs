//! Auto-generated from AWS STS Smithy model. DO NOT EDIT.

use serde::{Deserialize, Serialize};

use crate::types::{AssumedRoleUser, Credentials, FederatedUser};

/// STS AssumeRoleResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AssumeRoleResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assumed_role_user: Option<AssumedRoleUser>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials: Option<Credentials>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packed_policy_size: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_identity: Option<String>,
}

/// STS AssumeRoleWithSAMLResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AssumeRoleWithSAMLResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assumed_role_user: Option<AssumedRoleUser>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audience: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials: Option<Credentials>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issuer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_qualifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packed_policy_size: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_identity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_type: Option<String>,
}

/// STS AssumeRoleWithWebIdentityResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AssumeRoleWithWebIdentityResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assumed_role_user: Option<AssumedRoleUser>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audience: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials: Option<Credentials>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packed_policy_size: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_identity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_from_web_identity_token: Option<String>,
}

/// STS DecodeAuthorizationMessageResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DecodeAuthorizationMessageResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decoded_message: Option<String>,
}

/// STS GetAccessKeyInfoResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetAccessKeyInfoResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}

/// STS GetCallerIdentityResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetCallerIdentityResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
}

/// STS GetFederationTokenResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetFederationTokenResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials: Option<Credentials>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub federated_user: Option<FederatedUser>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packed_policy_size: Option<i32>,
}

/// STS GetSessionTokenResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetSessionTokenResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials: Option<Credentials>,
}
