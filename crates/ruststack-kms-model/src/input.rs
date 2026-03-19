//! Auto-generated from AWS KMS Smithy model. DO NOT EDIT.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::types::{
    CustomerMasterKeySpec, DataKeyPairSpec, DataKeySpec, DryRunModifierType,
    EncryptionAlgorithmSpec, GrantConstraints, GrantOperation, KeySpec, KeyUsageType,
    MacAlgorithmSpec, MessageType, OriginType, RecipientInfo, SigningAlgorithmSpec, Tag,
};

/// KMS CancelKeyDeletionInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CancelKeyDeletionInput {
    pub key_id: String,
}

/// KMS CreateAliasInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CreateAliasInput {
    pub alias_name: String,
    pub target_key_id: String,
}

/// KMS CreateGrantInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CreateGrantInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub constraints: Option<GrantConstraints>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grant_tokens: Vec<String>,
    pub grantee_principal: String,
    pub key_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<GrantOperation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retiring_principal: Option<String>,
}

/// KMS CreateKeyInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CreateKeyInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bypass_policy_lockout_safety_check: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_key_store_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub customer_master_key_spec: Option<CustomerMasterKeySpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_spec: Option<KeySpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_usage: Option<KeyUsageType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multi_region: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<OriginType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Tag>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xks_key_id: Option<String>,
}

/// KMS DecryptInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DecryptInput {
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::blob::option::serialize",
        deserialize_with = "crate::blob::option::deserialize"
    )]
    pub ciphertext_blob: Option<bytes::Bytes>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dry_run_modifiers: Vec<DryRunModifierType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryption_algorithm: Option<EncryptionAlgorithmSpec>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub encryption_context: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grant_tokens: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient: Option<RecipientInfo>,
}

/// KMS DeleteAliasInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteAliasInput {
    pub alias_name: String,
}

/// KMS DescribeKeyInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeKeyInput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grant_tokens: Vec<String>,
    pub key_id: String,
}

/// KMS DisableKeyInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DisableKeyInput {
    pub key_id: String,
}

/// KMS DisableKeyRotationInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DisableKeyRotationInput {
    pub key_id: String,
}

/// KMS EnableKeyInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EnableKeyInput {
    pub key_id: String,
}

/// KMS EnableKeyRotationInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EnableKeyRotationInput {
    pub key_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation_period_in_days: Option<i32>,
}

/// KMS EncryptInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EncryptInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryption_algorithm: Option<EncryptionAlgorithmSpec>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub encryption_context: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grant_tokens: Vec<String>,
    pub key_id: String,
    #[serde(with = "crate::blob")]
    pub plaintext: bytes::Bytes,
}

/// KMS GenerateDataKeyInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GenerateDataKeyInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub encryption_context: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grant_tokens: Vec<String>,
    pub key_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_spec: Option<DataKeySpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number_of_bytes: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient: Option<RecipientInfo>,
}

/// KMS GenerateDataKeyPairInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GenerateDataKeyPairInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub encryption_context: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grant_tokens: Vec<String>,
    pub key_id: String,
    pub key_pair_spec: DataKeyPairSpec,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient: Option<RecipientInfo>,
}

/// KMS GenerateDataKeyPairWithoutPlaintextInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GenerateDataKeyPairWithoutPlaintextInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub encryption_context: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grant_tokens: Vec<String>,
    pub key_id: String,
    pub key_pair_spec: DataKeyPairSpec,
}

/// KMS GenerateDataKeyWithoutPlaintextInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GenerateDataKeyWithoutPlaintextInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub encryption_context: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grant_tokens: Vec<String>,
    pub key_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_spec: Option<DataKeySpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number_of_bytes: Option<i32>,
}

/// KMS GenerateMacInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GenerateMacInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grant_tokens: Vec<String>,
    pub key_id: String,
    pub mac_algorithm: MacAlgorithmSpec,
    #[serde(with = "crate::blob")]
    pub message: bytes::Bytes,
}

/// KMS GenerateRandomInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GenerateRandomInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_key_store_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number_of_bytes: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient: Option<RecipientInfo>,
}

/// KMS GetKeyPolicyInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetKeyPolicyInput {
    pub key_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_name: Option<String>,
}

/// KMS GetKeyRotationStatusInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetKeyRotationStatusInput {
    pub key_id: String,
}

/// KMS GetPublicKeyInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetPublicKeyInput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grant_tokens: Vec<String>,
    pub key_id: String,
}

/// KMS ListAliasesInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListAliasesInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marker: Option<String>,
}

/// KMS ListGrantsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListGrantsInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grantee_principal: Option<String>,
    pub key_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marker: Option<String>,
}

/// KMS ListKeyPoliciesInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListKeyPoliciesInput {
    pub key_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marker: Option<String>,
}

/// KMS ListKeysInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListKeysInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marker: Option<String>,
}

/// KMS ListResourceTagsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListResourceTagsInput {
    pub key_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marker: Option<String>,
}

/// KMS ListRetirableGrantsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListRetirableGrantsInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marker: Option<String>,
    pub retiring_principal: String,
}

/// KMS PutKeyPolicyInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutKeyPolicyInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bypass_policy_lockout_safety_check: Option<bool>,
    pub key_id: String,
    pub policy: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_name: Option<String>,
}

/// KMS ReEncryptInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ReEncryptInput {
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::blob::option::serialize",
        deserialize_with = "crate::blob::option::deserialize"
    )]
    pub ciphertext_blob: Option<bytes::Bytes>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_encryption_algorithm: Option<EncryptionAlgorithmSpec>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub destination_encryption_context: HashMap<String, String>,
    pub destination_key_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dry_run_modifiers: Vec<DryRunModifierType>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grant_tokens: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_encryption_algorithm: Option<EncryptionAlgorithmSpec>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub source_encryption_context: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_key_id: Option<String>,
}

/// KMS RetireGrantInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RetireGrantInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
}

/// KMS RevokeGrantInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RevokeGrantInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    pub grant_id: String,
    pub key_id: String,
}

/// KMS ScheduleKeyDeletionInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ScheduleKeyDeletionInput {
    pub key_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_window_in_days: Option<i32>,
}

/// KMS SignInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SignInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grant_tokens: Vec<String>,
    pub key_id: String,
    #[serde(with = "crate::blob")]
    pub message: bytes::Bytes,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_type: Option<MessageType>,
    pub signing_algorithm: SigningAlgorithmSpec,
}

/// KMS TagResourceInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TagResourceInput {
    pub key_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Tag>,
}

/// KMS UntagResourceInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UntagResourceInput {
    pub key_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tag_keys: Vec<String>,
}

/// KMS UpdateAliasInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UpdateAliasInput {
    pub alias_name: String,
    pub target_key_id: String,
}

/// KMS UpdateKeyDescriptionInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UpdateKeyDescriptionInput {
    pub description: String,
    pub key_id: String,
}

/// KMS VerifyInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct VerifyInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grant_tokens: Vec<String>,
    pub key_id: String,
    #[serde(with = "crate::blob")]
    pub message: bytes::Bytes,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_type: Option<MessageType>,
    #[serde(with = "crate::blob")]
    pub signature: bytes::Bytes,
    pub signing_algorithm: SigningAlgorithmSpec,
}

/// KMS VerifyMacInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct VerifyMacInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grant_tokens: Vec<String>,
    pub key_id: String,
    #[serde(with = "crate::blob")]
    pub mac: bytes::Bytes,
    pub mac_algorithm: MacAlgorithmSpec,
    #[serde(with = "crate::blob")]
    pub message: bytes::Bytes,
}
