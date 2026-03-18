//! KMS provider implementing all 39 operations.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;

use ruststack_kms_model::error::{KmsError, KmsErrorCode};
use ruststack_kms_model::input::{
    CancelKeyDeletionInput, CreateAliasInput, CreateGrantInput, CreateKeyInput, DecryptInput,
    DeleteAliasInput, DescribeKeyInput, DisableKeyInput, DisableKeyRotationInput, EnableKeyInput,
    EnableKeyRotationInput, EncryptInput, GenerateDataKeyInput, GenerateDataKeyPairInput,
    GenerateDataKeyPairWithoutPlaintextInput, GenerateDataKeyWithoutPlaintextInput,
    GenerateMacInput, GenerateRandomInput, GetKeyPolicyInput, GetKeyRotationStatusInput,
    GetPublicKeyInput, ListAliasesInput, ListGrantsInput, ListKeyPoliciesInput, ListKeysInput,
    ListResourceTagsInput, ListRetirableGrantsInput, PutKeyPolicyInput, ReEncryptInput,
    RetireGrantInput, RevokeGrantInput, ScheduleKeyDeletionInput, SignInput, TagResourceInput,
    UntagResourceInput, UpdateAliasInput, UpdateKeyDescriptionInput, VerifyInput, VerifyMacInput,
};
use ruststack_kms_model::output::{
    CancelKeyDeletionResponse, CreateGrantResponse, CreateKeyResponse, DecryptResponse,
    DescribeKeyResponse, EncryptResponse, GenerateDataKeyPairResponse,
    GenerateDataKeyPairWithoutPlaintextResponse, GenerateDataKeyResponse,
    GenerateDataKeyWithoutPlaintextResponse, GenerateMacResponse, GenerateRandomResponse,
    GetKeyPolicyResponse, GetKeyRotationStatusResponse, GetPublicKeyResponse, ListAliasesResponse,
    ListGrantsResponse, ListKeyPoliciesResponse, ListKeysResponse, ListResourceTagsResponse,
    ReEncryptResponse, ScheduleKeyDeletionResponse, SignResponse, VerifyMacResponse,
    VerifyResponse,
};
use ruststack_kms_model::types::{
    AliasListEntry, DataKeySpec, EncryptionAlgorithmSpec, GrantListEntry, KeyListEntry,
    KeyManagerType, KeyMetadata, KeySpec, KeyState, KeyUsageType, OriginType, Tag,
};

use crate::ciphertext;
use crate::config::KmsConfig;
use crate::crypto;
use crate::key::{KeyMaterial, KmsKey};
use crate::resolve::resolve_key_id;
use crate::state::{AliasEntry, GrantEntry, KmsStore};
use crate::validation;

/// The KMS business logic provider.
#[derive(Debug)]
pub struct RustStackKms {
    store: Arc<KmsStore>,
    config: KmsConfig,
}

impl RustStackKms {
    /// Create a new KMS provider.
    pub fn new(config: KmsConfig) -> Self {
        let store = Arc::new(KmsStore::new(
            config.default_account_id.clone(),
            config.default_region.clone(),
        ));
        Self { store, config }
    }

    /// Build a key ARN.
    fn key_arn(&self, key_id: &str) -> String {
        KmsKey::build_arn(
            &self.config.default_account_id,
            &self.config.default_region,
            key_id,
        )
    }

    /// Build an alias ARN.
    fn alias_arn(&self, alias_name: &str) -> String {
        format!(
            "arn:aws:kms:{}:{}:{}",
            self.config.default_region, self.config.default_account_id, alias_name
        )
    }

    /// Resolve a key reference and return the key, validating it exists.
    fn resolve_and_get_key(&self, key_ref: &str) -> Result<KmsKey, KmsError> {
        let key_id = resolve_key_id(&self.store, key_ref)?;
        self.store.get_key(&key_id).ok_or_else(|| {
            KmsError::with_message(
                KmsErrorCode::NotFoundException,
                format!("Key '{key_ref}' does not exist"),
            )
        })
    }

    /// Build [`KeyMetadata`] from internal key.
    fn build_key_metadata(&self, key: &KmsKey) -> KeyMetadata {
        KeyMetadata {
            aws_account_id: Some(key.account_id.clone()),
            arn: Some(key.arn.clone()),
            key_id: key.key_id.clone(),
            creation_date: Some(key.creation_date),
            enabled: Some(key.enabled),
            description: Some(key.description.clone()),
            key_usage: Some(key.key_usage.clone()),
            key_state: Some(key.key_state.clone()),
            key_spec: Some(key.key_spec.clone()),
            origin: Some(key.origin.clone()),
            key_manager: Some(KeyManagerType::Customer),
            multi_region: Some(key.multi_region),
            encryption_algorithms: key.encryption_algorithms.clone(),
            signing_algorithms: key.signing_algorithms.clone(),
            mac_algorithms: key.mac_algorithms.clone(),
            deletion_date: key.deletion_date,
            pending_deletion_window_in_days: key.pending_deletion_window_in_days,
            ..KeyMetadata::default()
        }
    }

    // ===================================================================
    // Phase 0 - Key Management
    // ===================================================================

    /// Create a new KMS key.
    pub fn create_key(&self, input: CreateKeyInput) -> Result<CreateKeyResponse, KmsError> {
        let key_spec = input.key_spec.unwrap_or(KeySpec::SymmetricDefault);
        let key_usage = input
            .key_usage
            .unwrap_or_else(|| validation::default_key_usage(&key_spec));

        validation::validate_key_spec_usage(&key_spec, &key_usage)?;

        let key_material = crypto::generate_key_material(&key_spec)?;

        let key_id = uuid::Uuid::new_v4().to_string();
        let arn = self.key_arn(&key_id);

        let encryption_algorithms = if key_usage == KeyUsageType::EncryptDecrypt {
            validation::encryption_algorithms_for_spec(&key_spec)
        } else {
            vec![]
        };
        let signing_algorithms = if key_usage == KeyUsageType::SignVerify {
            validation::signing_algorithms_for_spec(&key_spec)
        } else {
            vec![]
        };
        let mac_algorithms = if key_usage == KeyUsageType::GenerateVerifyMac {
            validation::mac_algorithms_for_spec(&key_spec)
        } else {
            vec![]
        };

        // Process tags.
        let mut tags = HashMap::new();
        for tag in &input.tags {
            validation::validate_tag(&tag.tag_key, &tag.tag_value)?;
            tags.insert(tag.tag_key.clone(), tag.tag_value.clone());
        }

        let default_policy = format!(
            r#"{{"Version":"2012-10-17","Statement":[{{"Sid":"Enable Root Access","Effect":"Allow","Principal":{{"AWS":"arn:aws:iam::{}:root"}},"Action":"kms:*","Resource":"*"}}]}}"#,
            self.config.default_account_id
        );

        let key = KmsKey {
            key_id: key_id.clone(),
            arn: arn.clone(),
            account_id: self.config.default_account_id.clone(),
            region: self.config.default_region.clone(),
            key_spec: key_spec.clone(),
            key_usage: key_usage.clone(),
            key_state: KeyState::Enabled,
            description: input.description.unwrap_or_default(),
            enabled: true,
            creation_date: Utc::now(),
            deletion_date: None,
            pending_deletion_window_in_days: None,
            origin: OriginType::AwsKms,
            multi_region: input.multi_region.unwrap_or(false),
            policy: input.policy.unwrap_or(default_policy),
            tags,
            rotation_enabled: false,
            rotation_period_in_days: None,
            key_material,
            encryption_algorithms,
            signing_algorithms,
            mac_algorithms,
        };

        let metadata = self.build_key_metadata(&key);
        self.store.put_key(key);

        Ok(CreateKeyResponse {
            key_metadata: Some(metadata),
        })
    }

    /// Describe a key.
    pub fn describe_key(&self, input: &DescribeKeyInput) -> Result<DescribeKeyResponse, KmsError> {
        let key = self.resolve_and_get_key(&input.key_id)?;
        Ok(DescribeKeyResponse {
            key_metadata: Some(self.build_key_metadata(&key)),
        })
    }

    /// List keys.
    pub fn list_keys(&self, _input: &ListKeysInput) -> Result<ListKeysResponse, KmsError> {
        let keys: Vec<KeyListEntry> = self
            .store
            .keys
            .iter()
            .map(|entry| {
                let key = entry.value();
                KeyListEntry {
                    key_id: Some(key.key_id.clone()),
                    key_arn: Some(key.arn.clone()),
                }
            })
            .collect();

        Ok(ListKeysResponse {
            keys,
            truncated: Some(false),
            next_marker: None,
        })
    }

    /// Enable a key.
    pub fn enable_key(&self, input: &EnableKeyInput) -> Result<(), KmsError> {
        let key_id = resolve_key_id(&self.store, &input.key_id)?;
        let mut key_ref = self.store.keys.get_mut(&key_id).ok_or_else(|| {
            KmsError::with_message(
                KmsErrorCode::NotFoundException,
                format!("Key '{}' does not exist", input.key_id),
            )
        })?;
        let key = key_ref.value_mut();
        if key.key_state != KeyState::Disabled {
            return Err(KmsError::with_message(
                KmsErrorCode::KMSInvalidStateException,
                format!(
                    "Key {} is in state {}, cannot enable",
                    key.key_id,
                    key.key_state.as_str()
                ),
            ));
        }
        key.key_state = KeyState::Enabled;
        key.enabled = true;
        Ok(())
    }

    /// Disable a key.
    pub fn disable_key(&self, input: &DisableKeyInput) -> Result<(), KmsError> {
        let key_id = resolve_key_id(&self.store, &input.key_id)?;
        let mut key_ref = self.store.keys.get_mut(&key_id).ok_or_else(|| {
            KmsError::with_message(
                KmsErrorCode::NotFoundException,
                format!("Key '{}' does not exist", input.key_id),
            )
        })?;
        let key = key_ref.value_mut();
        if key.key_state != KeyState::Enabled {
            return Err(KmsError::with_message(
                KmsErrorCode::KMSInvalidStateException,
                format!(
                    "Key {} is in state {}, cannot disable",
                    key.key_id,
                    key.key_state.as_str()
                ),
            ));
        }
        key.key_state = KeyState::Disabled;
        key.enabled = false;
        Ok(())
    }

    /// Schedule key deletion.
    pub fn schedule_key_deletion(
        &self,
        input: &ScheduleKeyDeletionInput,
    ) -> Result<ScheduleKeyDeletionResponse, KmsError> {
        let key_id = resolve_key_id(&self.store, &input.key_id)?;
        let mut key_ref = self.store.keys.get_mut(&key_id).ok_or_else(|| {
            KmsError::with_message(
                KmsErrorCode::NotFoundException,
                format!("Key '{}' does not exist", input.key_id),
            )
        })?;
        let key = key_ref.value_mut();
        if key.key_state != KeyState::Enabled && key.key_state != KeyState::Disabled {
            return Err(KmsError::with_message(
                KmsErrorCode::KMSInvalidStateException,
                format!(
                    "Key {} is in state {}, cannot schedule deletion",
                    key.key_id,
                    key.key_state.as_str()
                ),
            ));
        }

        let days = input.pending_window_in_days.unwrap_or(30);
        let deletion_date = Utc::now() + chrono::Duration::days(i64::from(days));

        key.key_state = KeyState::PendingDeletion;
        key.enabled = false;
        key.deletion_date = Some(deletion_date);
        key.pending_deletion_window_in_days = Some(days);

        Ok(ScheduleKeyDeletionResponse {
            key_id: Some(key.key_id.clone()),
            deletion_date: Some(deletion_date),
            key_state: Some(KeyState::PendingDeletion),
            pending_window_in_days: Some(days),
        })
    }

    /// Cancel key deletion.
    pub fn cancel_key_deletion(
        &self,
        input: &CancelKeyDeletionInput,
    ) -> Result<CancelKeyDeletionResponse, KmsError> {
        let key_id = resolve_key_id(&self.store, &input.key_id)?;
        let mut key_ref = self.store.keys.get_mut(&key_id).ok_or_else(|| {
            KmsError::with_message(
                KmsErrorCode::NotFoundException,
                format!("Key '{}' does not exist", input.key_id),
            )
        })?;
        let key = key_ref.value_mut();
        if key.key_state != KeyState::PendingDeletion {
            return Err(KmsError::with_message(
                KmsErrorCode::KMSInvalidStateException,
                format!("Key {} is not pending deletion", key.key_id),
            ));
        }
        key.key_state = KeyState::Disabled;
        key.enabled = false;
        key.deletion_date = None;
        key.pending_deletion_window_in_days = None;

        Ok(CancelKeyDeletionResponse {
            key_id: Some(key.key_id.clone()),
        })
    }

    /// Update key description.
    pub fn update_key_description(
        &self,
        input: &UpdateKeyDescriptionInput,
    ) -> Result<(), KmsError> {
        let key_id = resolve_key_id(&self.store, &input.key_id)?;
        let mut key_ref = self.store.keys.get_mut(&key_id).ok_or_else(|| {
            KmsError::with_message(
                KmsErrorCode::NotFoundException,
                format!("Key '{}' does not exist", input.key_id),
            )
        })?;
        key_ref
            .value_mut()
            .description
            .clone_from(&input.description);
        Ok(())
    }

    // ===================================================================
    // Phase 0 - Cryptographic Operations
    // ===================================================================

    /// Encrypt plaintext.
    pub fn encrypt(&self, input: &EncryptInput) -> Result<EncryptResponse, KmsError> {
        let key = self.resolve_and_get_key(&input.key_id)?;
        validation::validate_key_enabled(&key)?;
        validation::validate_key_usage(&key, &KeyUsageType::EncryptDecrypt)?;

        let alg = input
            .encryption_algorithm
            .clone()
            .unwrap_or(EncryptionAlgorithmSpec::SymmetricDefault);

        let blob = match &key.key_material {
            KeyMaterial::Symmetric { key: key_bytes } => crypto::symmetric_encrypt(
                &key.key_id,
                key_bytes,
                &input.plaintext,
                &input.encryption_context,
            )?,
            KeyMaterial::Rsa { public_key_der, .. } => {
                crypto::rsa_oaep_encrypt(&key.key_id, public_key_der, &input.plaintext, &alg)?
            }
            _ => {
                return Err(KmsError::with_message(
                    KmsErrorCode::InvalidKeyUsageException,
                    "Key does not support encryption",
                ));
            }
        };

        Ok(EncryptResponse {
            ciphertext_blob: Some(bytes::Bytes::from(blob)),
            key_id: Some(key.arn.clone()),
            encryption_algorithm: Some(alg),
        })
    }

    /// Decrypt ciphertext.
    pub fn decrypt(&self, input: &DecryptInput) -> Result<DecryptResponse, KmsError> {
        let blob = input.ciphertext_blob.as_ref().ok_or_else(|| {
            KmsError::with_message(
                KmsErrorCode::InvalidCiphertextException,
                "CiphertextBlob is required",
            )
        })?;

        let alg = input
            .encryption_algorithm
            .clone()
            .unwrap_or(EncryptionAlgorithmSpec::SymmetricDefault);

        // For symmetric, extract key ID from blob.
        // For asymmetric, extract from blob header.
        let (key_id_from_blob, key) = if alg == EncryptionAlgorithmSpec::SymmetricDefault {
            let (kid, _, _, _) = ciphertext::parse_symmetric_blob(blob)?;
            let key = self.resolve_and_get_key(kid)?;
            (kid.to_owned(), key)
        } else {
            let (kid, _) = ciphertext::parse_asymmetric_blob(blob)?;
            let key = self.resolve_and_get_key(kid)?;
            (kid.to_owned(), key)
        };

        // If key_id is provided, verify it matches.
        if let Some(ref provided_key_id) = input.key_id {
            let resolved = resolve_key_id(&self.store, provided_key_id)?;
            if resolved != key_id_from_blob {
                return Err(KmsError::with_message(
                    KmsErrorCode::IncorrectKeyException,
                    "The key ID in the ciphertext does not match the specified key ID",
                ));
            }
        }

        validation::validate_key_enabled(&key)?;
        validation::validate_key_usage(&key, &KeyUsageType::EncryptDecrypt)?;

        let plaintext = match &key.key_material {
            KeyMaterial::Symmetric { key: key_bytes } => {
                let (_, pt) =
                    crypto::symmetric_decrypt(key_bytes, blob, &input.encryption_context)?;
                pt
            }
            KeyMaterial::Rsa {
                private_key_der, ..
            } => {
                let (_, ct) = ciphertext::parse_asymmetric_blob(blob)?;
                crypto::rsa_oaep_decrypt(private_key_der, ct, &alg)?
            }
            _ => {
                return Err(KmsError::with_message(
                    KmsErrorCode::InvalidKeyUsageException,
                    "Key does not support decryption",
                ));
            }
        };

        Ok(DecryptResponse {
            plaintext: Some(bytes::Bytes::from(plaintext)),
            key_id: Some(key.arn.clone()),
            encryption_algorithm: Some(alg),
            ..DecryptResponse::default()
        })
    }

    /// Re-encrypt ciphertext under a different key.
    pub fn re_encrypt(&self, input: &ReEncryptInput) -> Result<ReEncryptResponse, KmsError> {
        let blob = input.ciphertext_blob.as_ref().ok_or_else(|| {
            KmsError::with_message(
                KmsErrorCode::InvalidCiphertextException,
                "CiphertextBlob is required",
            )
        })?;

        let src_alg = input
            .source_encryption_algorithm
            .clone()
            .unwrap_or(EncryptionAlgorithmSpec::SymmetricDefault);

        // Decrypt with source key.
        let decrypt_input = DecryptInput {
            ciphertext_blob: Some(blob.clone()),
            encryption_algorithm: Some(src_alg.clone()),
            encryption_context: input.source_encryption_context.clone(),
            key_id: input.source_key_id.clone(),
            ..DecryptInput::default()
        };
        let decrypt_result = self.decrypt(&decrypt_input)?;
        let plaintext = decrypt_result
            .plaintext
            .ok_or_else(|| KmsError::internal_error("Decrypt returned no plaintext"))?;

        let dst_alg = input
            .destination_encryption_algorithm
            .clone()
            .unwrap_or(EncryptionAlgorithmSpec::SymmetricDefault);

        // Encrypt with destination key.
        let encrypt_input = EncryptInput {
            key_id: input.destination_key_id.clone(),
            plaintext,
            encryption_algorithm: Some(dst_alg.clone()),
            encryption_context: input.destination_encryption_context.clone(),
            ..EncryptInput::default()
        };
        let encrypt_result = self.encrypt(&encrypt_input)?;

        Ok(ReEncryptResponse {
            ciphertext_blob: encrypt_result.ciphertext_blob,
            key_id: encrypt_result.key_id,
            source_key_id: decrypt_result.key_id,
            source_encryption_algorithm: Some(src_alg),
            destination_encryption_algorithm: Some(dst_alg),
            ..ReEncryptResponse::default()
        })
    }

    /// Generate a data key.
    pub fn generate_data_key(
        &self,
        input: &GenerateDataKeyInput,
    ) -> Result<GenerateDataKeyResponse, KmsError> {
        let key = self.resolve_and_get_key(&input.key_id)?;
        validation::validate_key_enabled(&key)?;
        validation::validate_key_usage(&key, &KeyUsageType::EncryptDecrypt)?;

        let spec = input.key_spec.clone().unwrap_or(DataKeySpec::Aes256);
        let plaintext_bytes = crypto::generate_data_key(&spec, input.number_of_bytes)?;

        // Encrypt the data key under the KMS key.
        let encrypted = match &key.key_material {
            KeyMaterial::Symmetric { key: key_bytes } => crypto::symmetric_encrypt(
                &key.key_id,
                key_bytes,
                &plaintext_bytes,
                &input.encryption_context,
            )?,
            _ => {
                return Err(KmsError::with_message(
                    KmsErrorCode::InvalidKeyUsageException,
                    "GenerateDataKey requires a symmetric encryption key",
                ));
            }
        };

        Ok(GenerateDataKeyResponse {
            plaintext: Some(bytes::Bytes::from(plaintext_bytes)),
            ciphertext_blob: Some(bytes::Bytes::from(encrypted)),
            key_id: Some(key.arn.clone()),
            ..GenerateDataKeyResponse::default()
        })
    }

    /// Generate a data key without plaintext.
    pub fn generate_data_key_without_plaintext(
        &self,
        input: &GenerateDataKeyWithoutPlaintextInput,
    ) -> Result<GenerateDataKeyWithoutPlaintextResponse, KmsError> {
        let key = self.resolve_and_get_key(&input.key_id)?;
        validation::validate_key_enabled(&key)?;
        validation::validate_key_usage(&key, &KeyUsageType::EncryptDecrypt)?;

        let spec = input.key_spec.clone().unwrap_or(DataKeySpec::Aes256);
        let plaintext_bytes = crypto::generate_data_key(&spec, input.number_of_bytes)?;

        let encrypted = match &key.key_material {
            KeyMaterial::Symmetric { key: key_bytes } => crypto::symmetric_encrypt(
                &key.key_id,
                key_bytes,
                &plaintext_bytes,
                &input.encryption_context,
            )?,
            _ => {
                return Err(KmsError::with_message(
                    KmsErrorCode::InvalidKeyUsageException,
                    "GenerateDataKeyWithoutPlaintext requires a symmetric encryption key",
                ));
            }
        };

        Ok(GenerateDataKeyWithoutPlaintextResponse {
            ciphertext_blob: Some(bytes::Bytes::from(encrypted)),
            key_id: Some(key.arn.clone()),
            ..GenerateDataKeyWithoutPlaintextResponse::default()
        })
    }

    /// Generate a data key pair.
    pub fn generate_data_key_pair(
        &self,
        input: &GenerateDataKeyPairInput,
    ) -> Result<GenerateDataKeyPairResponse, KmsError> {
        let key = self.resolve_and_get_key(&input.key_id)?;
        validation::validate_key_enabled(&key)?;
        validation::validate_key_usage(&key, &KeyUsageType::EncryptDecrypt)?;

        let (private_der, public_der) = crypto::generate_data_key_pair(&input.key_pair_spec)?;

        // Encrypt the private key under the KMS key.
        let encrypted_private = match &key.key_material {
            KeyMaterial::Symmetric { key: key_bytes } => crypto::symmetric_encrypt(
                &key.key_id,
                key_bytes,
                &private_der,
                &input.encryption_context,
            )?,
            _ => {
                return Err(KmsError::with_message(
                    KmsErrorCode::InvalidKeyUsageException,
                    "GenerateDataKeyPair requires a symmetric encryption key",
                ));
            }
        };

        Ok(GenerateDataKeyPairResponse {
            private_key_plaintext: Some(bytes::Bytes::from(private_der)),
            private_key_ciphertext_blob: Some(bytes::Bytes::from(encrypted_private)),
            public_key: Some(bytes::Bytes::from(public_der)),
            key_id: Some(key.arn.clone()),
            key_pair_spec: Some(input.key_pair_spec.clone()),
            ..GenerateDataKeyPairResponse::default()
        })
    }

    /// Generate a data key pair without plaintext.
    pub fn generate_data_key_pair_without_plaintext(
        &self,
        input: &GenerateDataKeyPairWithoutPlaintextInput,
    ) -> Result<GenerateDataKeyPairWithoutPlaintextResponse, KmsError> {
        let key = self.resolve_and_get_key(&input.key_id)?;
        validation::validate_key_enabled(&key)?;
        validation::validate_key_usage(&key, &KeyUsageType::EncryptDecrypt)?;

        let (private_der, public_der) = crypto::generate_data_key_pair(&input.key_pair_spec)?;

        let encrypted_private = match &key.key_material {
            KeyMaterial::Symmetric { key: key_bytes } => crypto::symmetric_encrypt(
                &key.key_id,
                key_bytes,
                &private_der,
                &input.encryption_context,
            )?,
            _ => {
                return Err(KmsError::with_message(
                    KmsErrorCode::InvalidKeyUsageException,
                    "GenerateDataKeyPairWithoutPlaintext requires a symmetric encryption key",
                ));
            }
        };

        Ok(GenerateDataKeyPairWithoutPlaintextResponse {
            private_key_ciphertext_blob: Some(bytes::Bytes::from(encrypted_private)),
            public_key: Some(bytes::Bytes::from(public_der)),
            key_id: Some(key.arn.clone()),
            key_pair_spec: Some(input.key_pair_spec.clone()),
            ..GenerateDataKeyPairWithoutPlaintextResponse::default()
        })
    }

    /// Sign a message.
    pub fn sign(&self, input: &SignInput) -> Result<SignResponse, KmsError> {
        let key = self.resolve_and_get_key(&input.key_id)?;
        validation::validate_key_enabled(&key)?;
        validation::validate_key_usage(&key, &KeyUsageType::SignVerify)?;

        if !key.supports_signing_algorithm(&input.signing_algorithm) {
            return Err(KmsError::with_message(
                KmsErrorCode::InvalidKeyUsageException,
                format!(
                    "Algorithm {} is not supported for key {}",
                    input.signing_algorithm.as_str(),
                    key.key_id
                ),
            ));
        }

        let sig = match &key.key_material {
            KeyMaterial::Rsa {
                private_key_der, ..
            } => crypto::rsa_sign(private_key_der, &input.message, &input.signing_algorithm)?,
            KeyMaterial::Ec {
                private_key_der, ..
            } => crypto::ecdsa_sign(private_key_der, &input.message, &input.signing_algorithm)?,
            _ => {
                return Err(KmsError::with_message(
                    KmsErrorCode::InvalidKeyUsageException,
                    "Key does not support signing",
                ));
            }
        };

        Ok(SignResponse {
            key_id: Some(key.arn.clone()),
            signature: Some(bytes::Bytes::from(sig)),
            signing_algorithm: Some(input.signing_algorithm.clone()),
        })
    }

    /// Verify a signature.
    pub fn verify(&self, input: &VerifyInput) -> Result<VerifyResponse, KmsError> {
        let key = self.resolve_and_get_key(&input.key_id)?;
        validation::validate_key_enabled(&key)?;
        validation::validate_key_usage(&key, &KeyUsageType::SignVerify)?;

        let valid = match &key.key_material {
            KeyMaterial::Rsa { public_key_der, .. } => crypto::rsa_verify(
                public_key_der,
                &input.message,
                &input.signature,
                &input.signing_algorithm,
            )?,
            KeyMaterial::Ec { public_key_der, .. } => crypto::ecdsa_verify(
                public_key_der,
                &input.message,
                &input.signature,
                &input.signing_algorithm,
            )?,
            _ => {
                return Err(KmsError::with_message(
                    KmsErrorCode::InvalidKeyUsageException,
                    "Key does not support verification",
                ));
            }
        };

        if !valid {
            return Err(KmsError::with_message(
                KmsErrorCode::KMSInvalidSignatureException,
                "Signature verification failed",
            ));
        }

        Ok(VerifyResponse {
            key_id: Some(key.arn.clone()),
            signature_valid: Some(true),
            signing_algorithm: Some(input.signing_algorithm.clone()),
        })
    }

    /// Get public key.
    pub fn get_public_key(
        &self,
        input: &GetPublicKeyInput,
    ) -> Result<GetPublicKeyResponse, KmsError> {
        let key = self.resolve_and_get_key(&input.key_id)?;
        validation::validate_key_enabled(&key)?;

        let public_key_bytes = match &key.key_material {
            KeyMaterial::Rsa { public_key_der, .. } | KeyMaterial::Ec { public_key_der, .. } => {
                public_key_der.clone()
            }
            _ => {
                return Err(KmsError::with_message(
                    KmsErrorCode::InvalidKeyUsageException,
                    "Key does not have a public key (symmetric or HMAC keys)",
                ));
            }
        };

        Ok(GetPublicKeyResponse {
            key_id: Some(key.arn.clone()),
            public_key: Some(bytes::Bytes::from(public_key_bytes)),
            key_spec: Some(key.key_spec.clone()),
            key_usage: Some(key.key_usage.clone()),
            encryption_algorithms: key.encryption_algorithms.clone(),
            signing_algorithms: key.signing_algorithms.clone(),
            ..GetPublicKeyResponse::default()
        })
    }

    /// Generate MAC.
    pub fn generate_mac(&self, input: &GenerateMacInput) -> Result<GenerateMacResponse, KmsError> {
        let key = self.resolve_and_get_key(&input.key_id)?;
        validation::validate_key_enabled(&key)?;
        validation::validate_key_usage(&key, &KeyUsageType::GenerateVerifyMac)?;

        if !key.supports_mac_algorithm(&input.mac_algorithm) {
            return Err(KmsError::with_message(
                KmsErrorCode::InvalidKeyUsageException,
                format!(
                    "Algorithm {} is not supported for key {}",
                    input.mac_algorithm.as_str(),
                    key.key_id
                ),
            ));
        }

        let mac = match &key.key_material {
            KeyMaterial::Hmac { key: key_bytes } => {
                crypto::hmac_generate(key_bytes, &input.message, &input.mac_algorithm)?
            }
            _ => {
                return Err(KmsError::with_message(
                    KmsErrorCode::InvalidKeyUsageException,
                    "Key does not support MAC generation",
                ));
            }
        };

        Ok(GenerateMacResponse {
            mac: Some(bytes::Bytes::from(mac)),
            mac_algorithm: Some(input.mac_algorithm.clone()),
            key_id: Some(key.arn.clone()),
        })
    }

    /// Verify MAC.
    pub fn verify_mac(&self, input: &VerifyMacInput) -> Result<VerifyMacResponse, KmsError> {
        let key = self.resolve_and_get_key(&input.key_id)?;
        validation::validate_key_enabled(&key)?;
        validation::validate_key_usage(&key, &KeyUsageType::GenerateVerifyMac)?;

        let valid = match &key.key_material {
            KeyMaterial::Hmac { key: key_bytes } => {
                crypto::hmac_verify(key_bytes, &input.message, &input.mac, &input.mac_algorithm)?
            }
            _ => {
                return Err(KmsError::with_message(
                    KmsErrorCode::InvalidKeyUsageException,
                    "Key does not support MAC verification",
                ));
            }
        };

        if !valid {
            return Err(KmsError::with_message(
                KmsErrorCode::KMSInvalidMacException,
                "MAC verification failed",
            ));
        }

        Ok(VerifyMacResponse {
            key_id: Some(key.arn.clone()),
            mac_valid: Some(true),
            mac_algorithm: Some(input.mac_algorithm.clone()),
        })
    }

    /// Generate random bytes.
    pub fn generate_random(
        &self,
        input: &GenerateRandomInput,
    ) -> Result<GenerateRandomResponse, KmsError> {
        let num_bytes = input.number_of_bytes.unwrap_or(32) as usize;
        if num_bytes == 0 || num_bytes > 1024 {
            return Err(KmsError::with_message(
                KmsErrorCode::InvalidKeyUsageException,
                "NumberOfBytes must be between 1 and 1024",
            ));
        }
        let random = crypto::generate_random_bytes(num_bytes)?;
        Ok(GenerateRandomResponse {
            plaintext: Some(bytes::Bytes::from(random)),
            ..GenerateRandomResponse::default()
        })
    }

    // ===================================================================
    // Phase 1 - Aliases
    // ===================================================================

    /// Create an alias.
    pub fn create_alias(&self, input: &CreateAliasInput) -> Result<(), KmsError> {
        if !input.alias_name.starts_with("alias/") {
            return Err(KmsError::with_message(
                KmsErrorCode::InvalidAliasNameException,
                "Alias must start with 'alias/'",
            ));
        }

        if self.store.get_alias(&input.alias_name).is_some() {
            return Err(KmsError::with_message(
                KmsErrorCode::AlreadyExistsException,
                format!("Alias {} already exists", input.alias_name),
            ));
        }

        // Verify target key exists.
        let key_id = resolve_key_id(&self.store, &input.target_key_id)?;
        if self.store.get_key(&key_id).is_none() {
            return Err(KmsError::with_message(
                KmsErrorCode::NotFoundException,
                format!("Key '{}' does not exist", input.target_key_id),
            ));
        }

        let now = Utc::now();
        self.store.put_alias(AliasEntry {
            alias_name: input.alias_name.clone(),
            alias_arn: self.alias_arn(&input.alias_name),
            target_key_id: key_id,
            creation_date: now,
            last_updated_date: now,
        });

        Ok(())
    }

    /// Delete an alias.
    pub fn delete_alias(&self, input: &DeleteAliasInput) -> Result<(), KmsError> {
        self.store.remove_alias(&input.alias_name).ok_or_else(|| {
            KmsError::with_message(
                KmsErrorCode::NotFoundException,
                format!("Alias {} is not found.", input.alias_name),
            )
        })?;
        Ok(())
    }

    /// List aliases.
    pub fn list_aliases(&self, input: &ListAliasesInput) -> Result<ListAliasesResponse, KmsError> {
        let key_id = match &input.key_id {
            Some(kid) => Some(resolve_key_id(&self.store, kid)?),
            None => None,
        };

        let aliases: Vec<AliasListEntry> = self
            .store
            .list_aliases(key_id.as_deref())
            .into_iter()
            .map(|a| AliasListEntry {
                alias_name: Some(a.alias_name),
                alias_arn: Some(a.alias_arn),
                target_key_id: Some(a.target_key_id),
                creation_date: Some(a.creation_date),
                last_updated_date: Some(a.last_updated_date),
            })
            .collect();

        Ok(ListAliasesResponse {
            aliases,
            truncated: Some(false),
            next_marker: None,
        })
    }

    /// Update an alias to point to a different key.
    pub fn update_alias(&self, input: &UpdateAliasInput) -> Result<(), KmsError> {
        let key_id = resolve_key_id(&self.store, &input.target_key_id)?;
        if self.store.get_key(&key_id).is_none() {
            return Err(KmsError::with_message(
                KmsErrorCode::NotFoundException,
                format!("Key '{}' does not exist", input.target_key_id),
            ));
        }

        let mut alias_ref = self
            .store
            .aliases
            .get_mut(&input.alias_name)
            .ok_or_else(|| {
                KmsError::with_message(
                    KmsErrorCode::NotFoundException,
                    format!("Alias {} is not found.", input.alias_name),
                )
            })?;
        alias_ref.target_key_id = key_id;
        alias_ref.last_updated_date = Utc::now();
        Ok(())
    }

    // ===================================================================
    // Phase 1 - Tags
    // ===================================================================

    /// Tag a resource (key).
    pub fn tag_resource(&self, input: &TagResourceInput) -> Result<(), KmsError> {
        let key_id = resolve_key_id(&self.store, &input.key_id)?;
        let mut key_ref = self.store.keys.get_mut(&key_id).ok_or_else(|| {
            KmsError::with_message(
                KmsErrorCode::NotFoundException,
                format!("Key '{}' does not exist", input.key_id),
            )
        })?;

        for tag in &input.tags {
            validation::validate_tag(&tag.tag_key, &tag.tag_value)?;
            key_ref
                .tags
                .insert(tag.tag_key.clone(), tag.tag_value.clone());
        }

        if key_ref.tags.len() > validation::MAX_TAGS {
            return Err(KmsError::with_message(
                KmsErrorCode::TagException,
                format!("Tag limit of {} exceeded", validation::MAX_TAGS),
            ));
        }

        Ok(())
    }

    /// Untag a resource (key).
    pub fn untag_resource(&self, input: &UntagResourceInput) -> Result<(), KmsError> {
        let key_id = resolve_key_id(&self.store, &input.key_id)?;
        let mut key_ref = self.store.keys.get_mut(&key_id).ok_or_else(|| {
            KmsError::with_message(
                KmsErrorCode::NotFoundException,
                format!("Key '{}' does not exist", input.key_id),
            )
        })?;

        for tag_key in &input.tag_keys {
            key_ref.tags.remove(tag_key);
        }

        Ok(())
    }

    /// List resource tags.
    pub fn list_resource_tags(
        &self,
        input: &ListResourceTagsInput,
    ) -> Result<ListResourceTagsResponse, KmsError> {
        let key_id = resolve_key_id(&self.store, &input.key_id)?;
        let tags_map = self.store.get_tags(&key_id);

        let tags: Vec<Tag> = tags_map
            .into_iter()
            .map(|(k, v)| Tag {
                tag_key: k,
                tag_value: v,
            })
            .collect();

        Ok(ListResourceTagsResponse {
            tags,
            truncated: Some(false),
            next_marker: None,
        })
    }

    // ===================================================================
    // Phase 1 - Key Policies
    // ===================================================================

    /// Get key policy.
    pub fn get_key_policy(
        &self,
        input: &GetKeyPolicyInput,
    ) -> Result<GetKeyPolicyResponse, KmsError> {
        let key = self.resolve_and_get_key(&input.key_id)?;
        Ok(GetKeyPolicyResponse {
            policy: Some(key.policy.clone()),
            policy_name: Some("default".to_owned()),
        })
    }

    /// Put key policy.
    pub fn put_key_policy(&self, input: &PutKeyPolicyInput) -> Result<(), KmsError> {
        let key_id = resolve_key_id(&self.store, &input.key_id)?;
        let mut key_ref = self.store.keys.get_mut(&key_id).ok_or_else(|| {
            KmsError::with_message(
                KmsErrorCode::NotFoundException,
                format!("Key '{}' does not exist", input.key_id),
            )
        })?;
        key_ref.policy.clone_from(&input.policy);
        Ok(())
    }

    /// List key policies.
    pub fn list_key_policies(
        &self,
        input: &ListKeyPoliciesInput,
    ) -> Result<ListKeyPoliciesResponse, KmsError> {
        // Verify key exists.
        let _ = self.resolve_and_get_key(&input.key_id)?;

        Ok(ListKeyPoliciesResponse {
            policy_names: vec!["default".to_owned()],
            truncated: Some(false),
            next_marker: None,
        })
    }

    // ===================================================================
    // Phase 2 - Grants
    // ===================================================================

    /// Create a grant.
    pub fn create_grant(&self, input: &CreateGrantInput) -> Result<CreateGrantResponse, KmsError> {
        let key = self.resolve_and_get_key(&input.key_id)?;
        validation::validate_key_enabled(&key)?;

        let grant_id = uuid::Uuid::new_v4().to_string();
        let grant_token = uuid::Uuid::new_v4().to_string();

        let entry = GrantEntry {
            grant_id: grant_id.clone(),
            key_id: key.key_id.clone(),
            grantee_principal: input.grantee_principal.clone(),
            retiring_principal: input.retiring_principal.clone(),
            operations: input.operations.clone(),
            constraints: input.constraints.clone(),
            name: input.name.clone(),
            creation_date: Utc::now(),
            retired: false,
            issuing_account: format!("arn:aws:iam::{}:root", self.config.default_account_id),
        };

        self.store.put_grant(entry);

        Ok(CreateGrantResponse {
            grant_id: Some(grant_id),
            grant_token: Some(grant_token),
        })
    }

    /// List grants for a key.
    pub fn list_grants(&self, input: &ListGrantsInput) -> Result<ListGrantsResponse, KmsError> {
        let key_id = resolve_key_id(&self.store, &input.key_id)?;
        if self.store.get_key(&key_id).is_none() {
            return Err(KmsError::with_message(
                KmsErrorCode::NotFoundException,
                format!("Key '{}' does not exist", input.key_id),
            ));
        }

        let grants: Vec<GrantListEntry> = self
            .store
            .list_grants_for_key(&key_id)
            .into_iter()
            .map(|g| GrantListEntry {
                grant_id: Some(g.grant_id),
                key_id: Some(g.key_id),
                grantee_principal: Some(g.grantee_principal),
                retiring_principal: g.retiring_principal,
                operations: g.operations,
                constraints: g.constraints,
                name: g.name,
                creation_date: Some(g.creation_date),
                issuing_account: Some(g.issuing_account),
            })
            .collect();

        Ok(ListGrantsResponse {
            grants,
            truncated: Some(false),
            next_marker: None,
        })
    }

    /// Retire a grant.
    pub fn retire_grant(&self, input: &RetireGrantInput) -> Result<(), KmsError> {
        let grant_id = input.grant_id.as_ref().ok_or_else(|| {
            KmsError::with_message(KmsErrorCode::InvalidGrantIdException, "GrantId is required")
        })?;

        if !self.store.retire_grant(grant_id) {
            return Err(KmsError::with_message(
                KmsErrorCode::NotFoundException,
                format!("Grant {grant_id} is not found."),
            ));
        }
        Ok(())
    }

    /// Revoke a grant.
    pub fn revoke_grant(&self, input: &RevokeGrantInput) -> Result<(), KmsError> {
        // Verify key exists.
        let _ = self.resolve_and_get_key(&input.key_id)?;

        self.store.remove_grant(&input.grant_id).ok_or_else(|| {
            KmsError::with_message(
                KmsErrorCode::NotFoundException,
                format!("Grant {} is not found.", input.grant_id),
            )
        })?;
        Ok(())
    }

    /// List retirable grants.
    pub fn list_retirable_grants(
        &self,
        input: &ListRetirableGrantsInput,
    ) -> Result<ListGrantsResponse, KmsError> {
        let grants: Vec<GrantListEntry> = self
            .store
            .list_retirable_grants(&input.retiring_principal)
            .into_iter()
            .map(|g| GrantListEntry {
                grant_id: Some(g.grant_id),
                key_id: Some(g.key_id),
                grantee_principal: Some(g.grantee_principal),
                retiring_principal: g.retiring_principal,
                operations: g.operations,
                constraints: g.constraints,
                name: g.name,
                creation_date: Some(g.creation_date),
                issuing_account: Some(g.issuing_account),
            })
            .collect();

        Ok(ListGrantsResponse {
            grants,
            truncated: Some(false),
            next_marker: None,
        })
    }

    // ===================================================================
    // Phase 2 - Key Rotation
    // ===================================================================

    /// Enable key rotation.
    pub fn enable_key_rotation(&self, input: &EnableKeyRotationInput) -> Result<(), KmsError> {
        let key_id = resolve_key_id(&self.store, &input.key_id)?;
        let mut key_ref = self.store.keys.get_mut(&key_id).ok_or_else(|| {
            KmsError::with_message(
                KmsErrorCode::NotFoundException,
                format!("Key '{}' does not exist", input.key_id),
            )
        })?;
        let key = key_ref.value_mut();

        if key.key_spec != KeySpec::SymmetricDefault {
            return Err(KmsError::with_message(
                KmsErrorCode::UnsupportedOperationException,
                "Key rotation is only supported for symmetric encryption keys",
            ));
        }

        key.rotation_enabled = true;
        key.rotation_period_in_days = Some(input.rotation_period_in_days.unwrap_or(365));
        Ok(())
    }

    /// Disable key rotation.
    pub fn disable_key_rotation(&self, input: &DisableKeyRotationInput) -> Result<(), KmsError> {
        let key_id = resolve_key_id(&self.store, &input.key_id)?;
        let mut key_ref = self.store.keys.get_mut(&key_id).ok_or_else(|| {
            KmsError::with_message(
                KmsErrorCode::NotFoundException,
                format!("Key '{}' does not exist", input.key_id),
            )
        })?;
        key_ref.rotation_enabled = false;
        key_ref.rotation_period_in_days = None;
        Ok(())
    }

    /// Get key rotation status.
    pub fn get_key_rotation_status(
        &self,
        input: &GetKeyRotationStatusInput,
    ) -> Result<GetKeyRotationStatusResponse, KmsError> {
        let key = self.resolve_and_get_key(&input.key_id)?;

        Ok(GetKeyRotationStatusResponse {
            key_id: Some(key.arn.clone()),
            key_rotation_enabled: Some(key.rotation_enabled),
            rotation_period_in_days: key.rotation_period_in_days,
            ..GetKeyRotationStatusResponse::default()
        })
    }
}
