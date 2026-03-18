//! Auto-generated from AWS KMS Smithy model. DO NOT EDIT.

use serde::{Deserialize, Serialize};

use crate::types::{
    AliasListEntry, CustomerMasterKeySpec, DataKeyPairSpec, EncryptionAlgorithmSpec,
    GrantListEntry, KeyAgreementAlgorithmSpec, KeyListEntry, KeyMetadata, KeySpec, KeyState,
    KeyUsageType, MacAlgorithmSpec, SigningAlgorithmSpec, Tag,
};

/// KMS CancelKeyDeletionResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CancelKeyDeletionResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
}

/// KMS CreateGrantResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CreateGrantResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_token: Option<String>,
}

/// KMS CreateKeyResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CreateKeyResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_metadata: Option<KeyMetadata>,
}

/// KMS DecryptResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DecryptResponse {
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::blob::option::serialize",
        deserialize_with = "crate::blob::option::deserialize"
    )]
    pub ciphertext_for_recipient: Option<bytes::Bytes>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryption_algorithm: Option<EncryptionAlgorithmSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_material_id: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::blob::option::serialize",
        deserialize_with = "crate::blob::option::deserialize"
    )]
    pub plaintext: Option<bytes::Bytes>,
}

/// KMS DescribeKeyResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeKeyResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_metadata: Option<KeyMetadata>,
}

/// KMS EncryptResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EncryptResponse {
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::blob::option::serialize",
        deserialize_with = "crate::blob::option::deserialize"
    )]
    pub ciphertext_blob: Option<bytes::Bytes>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryption_algorithm: Option<EncryptionAlgorithmSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
}

/// KMS GenerateDataKeyPairResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GenerateDataKeyPairResponse {
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::blob::option::serialize",
        deserialize_with = "crate::blob::option::deserialize"
    )]
    pub ciphertext_for_recipient: Option<bytes::Bytes>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_material_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_pair_spec: Option<DataKeyPairSpec>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::blob::option::serialize",
        deserialize_with = "crate::blob::option::deserialize"
    )]
    pub private_key_ciphertext_blob: Option<bytes::Bytes>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::blob::option::serialize",
        deserialize_with = "crate::blob::option::deserialize"
    )]
    pub private_key_plaintext: Option<bytes::Bytes>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::blob::option::serialize",
        deserialize_with = "crate::blob::option::deserialize"
    )]
    pub public_key: Option<bytes::Bytes>,
}

/// KMS GenerateDataKeyPairWithoutPlaintextResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GenerateDataKeyPairWithoutPlaintextResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_material_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_pair_spec: Option<DataKeyPairSpec>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::blob::option::serialize",
        deserialize_with = "crate::blob::option::deserialize"
    )]
    pub private_key_ciphertext_blob: Option<bytes::Bytes>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::blob::option::serialize",
        deserialize_with = "crate::blob::option::deserialize"
    )]
    pub public_key: Option<bytes::Bytes>,
}

/// KMS GenerateDataKeyResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GenerateDataKeyResponse {
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::blob::option::serialize",
        deserialize_with = "crate::blob::option::deserialize"
    )]
    pub ciphertext_blob: Option<bytes::Bytes>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::blob::option::serialize",
        deserialize_with = "crate::blob::option::deserialize"
    )]
    pub ciphertext_for_recipient: Option<bytes::Bytes>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_material_id: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::blob::option::serialize",
        deserialize_with = "crate::blob::option::deserialize"
    )]
    pub plaintext: Option<bytes::Bytes>,
}

/// KMS GenerateDataKeyWithoutPlaintextResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GenerateDataKeyWithoutPlaintextResponse {
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::blob::option::serialize",
        deserialize_with = "crate::blob::option::deserialize"
    )]
    pub ciphertext_blob: Option<bytes::Bytes>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_material_id: Option<String>,
}

/// KMS GenerateMacResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GenerateMacResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::blob::option::serialize",
        deserialize_with = "crate::blob::option::deserialize"
    )]
    pub mac: Option<bytes::Bytes>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mac_algorithm: Option<MacAlgorithmSpec>,
}

/// KMS GenerateRandomResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GenerateRandomResponse {
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::blob::option::serialize",
        deserialize_with = "crate::blob::option::deserialize"
    )]
    pub ciphertext_for_recipient: Option<bytes::Bytes>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::blob::option::serialize",
        deserialize_with = "crate::blob::option::deserialize"
    )]
    pub plaintext: Option<bytes::Bytes>,
}

/// KMS GetKeyPolicyResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetKeyPolicyResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_name: Option<String>,
}

/// KMS GetKeyRotationStatusResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetKeyRotationStatusResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_rotation_enabled: Option<bool>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::epoch_seconds::option::serialize",
        deserialize_with = "crate::epoch_seconds::option::deserialize"
    )]
    pub next_rotation_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::epoch_seconds::option::serialize",
        deserialize_with = "crate::epoch_seconds::option::deserialize"
    )]
    pub on_demand_rotation_start_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation_period_in_days: Option<i32>,
}

/// KMS GetPublicKeyResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetPublicKeyResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub customer_master_key_spec: Option<CustomerMasterKeySpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub encryption_algorithms: Vec<EncryptionAlgorithmSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub key_agreement_algorithms: Vec<KeyAgreementAlgorithmSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_spec: Option<KeySpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_usage: Option<KeyUsageType>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::blob::option::serialize",
        deserialize_with = "crate::blob::option::deserialize"
    )]
    pub public_key: Option<bytes::Bytes>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signing_algorithms: Vec<SigningAlgorithmSpec>,
}

/// KMS ListAliasesResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListAliasesResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<AliasListEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_marker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
}

/// KMS ListGrantsResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListGrantsResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grants: Vec<GrantListEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_marker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
}

/// KMS ListKeyPoliciesResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListKeyPoliciesResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_marker: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub policy_names: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
}

/// KMS ListKeysResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListKeysResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<KeyListEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_marker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
}

/// KMS ListResourceTagsResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListResourceTagsResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_marker: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Tag>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
}

/// KMS ReEncryptResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ReEncryptResponse {
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::blob::option::serialize",
        deserialize_with = "crate::blob::option::deserialize"
    )]
    pub ciphertext_blob: Option<bytes::Bytes>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_encryption_algorithm: Option<EncryptionAlgorithmSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_key_material_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_encryption_algorithm: Option<EncryptionAlgorithmSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_key_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_key_material_id: Option<String>,
}

/// KMS ScheduleKeyDeletionResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ScheduleKeyDeletionResponse {
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::epoch_seconds::option::serialize",
        deserialize_with = "crate::epoch_seconds::option::deserialize"
    )]
    pub deletion_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_state: Option<KeyState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_window_in_days: Option<i32>,
}

/// KMS SignResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SignResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::blob::option::serialize",
        deserialize_with = "crate::blob::option::deserialize"
    )]
    pub signature: Option<bytes::Bytes>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signing_algorithm: Option<SigningAlgorithmSpec>,
}

/// KMS VerifyMacResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct VerifyMacResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mac_algorithm: Option<MacAlgorithmSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mac_valid: Option<bool>,
}

/// KMS VerifyResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct VerifyResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature_valid: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signing_algorithm: Option<SigningAlgorithmSpec>,
}
