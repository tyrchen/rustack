//! Input validation for KMS operations.

use ruststack_kms_model::error::{KmsError, KmsErrorCode};
use ruststack_kms_model::types::{
    EncryptionAlgorithmSpec, KeySpec, KeyState, KeyUsageType, MacAlgorithmSpec,
    SigningAlgorithmSpec,
};

use crate::key::KmsKey;

/// Maximum number of tags per key.
pub const MAX_TAGS: usize = 50;
/// Maximum tag key length.
pub const MAX_TAG_KEY_LEN: usize = 128;
/// Maximum tag value length.
pub const MAX_TAG_VALUE_LEN: usize = 256;

/// Determine the default key usage for a given key spec.
pub fn default_key_usage(spec: &KeySpec) -> KeyUsageType {
    match spec {
        KeySpec::EccNistP256
        | KeySpec::EccNistP384
        | KeySpec::EccNistP521
        | KeySpec::EccSecgP256k1
        | KeySpec::EccNistEdwards25519 => KeyUsageType::SignVerify,
        KeySpec::Hmac224 | KeySpec::Hmac256 | KeySpec::Hmac384 | KeySpec::Hmac512 => {
            KeyUsageType::GenerateVerifyMac
        }
        _ => KeyUsageType::EncryptDecrypt,
    }
}

/// Validate key spec / usage compatibility.
pub fn validate_key_spec_usage(spec: &KeySpec, usage: &KeyUsageType) -> Result<(), KmsError> {
    let valid = match spec {
        KeySpec::SymmetricDefault => *usage == KeyUsageType::EncryptDecrypt,
        KeySpec::Rsa2048 | KeySpec::Rsa3072 | KeySpec::Rsa4096 => {
            *usage == KeyUsageType::EncryptDecrypt || *usage == KeyUsageType::SignVerify
        }
        KeySpec::EccNistP256
        | KeySpec::EccNistP384
        | KeySpec::EccNistP521
        | KeySpec::EccSecgP256k1
        | KeySpec::EccNistEdwards25519 => *usage == KeyUsageType::SignVerify,
        KeySpec::Hmac224 | KeySpec::Hmac256 | KeySpec::Hmac384 | KeySpec::Hmac512 => {
            *usage == KeyUsageType::GenerateVerifyMac
        }
        _ => false,
    };

    if valid {
        Ok(())
    } else {
        Err(KmsError::with_message(
            KmsErrorCode::InvalidKeyUsageException,
            format!(
                "The key usage '{}' is not compatible with key spec '{}'",
                usage.as_str(),
                spec.as_str()
            ),
        ))
    }
}

/// Get supported encryption algorithms for a key spec + ENCRYPT_DECRYPT usage.
pub fn encryption_algorithms_for_spec(spec: &KeySpec) -> Vec<EncryptionAlgorithmSpec> {
    match spec {
        KeySpec::SymmetricDefault => vec![EncryptionAlgorithmSpec::SymmetricDefault],
        KeySpec::Rsa2048 | KeySpec::Rsa3072 | KeySpec::Rsa4096 => vec![
            EncryptionAlgorithmSpec::RsaesOaepSha1,
            EncryptionAlgorithmSpec::RsaesOaepSha256,
        ],
        _ => vec![],
    }
}

/// Get supported signing algorithms for a key spec + SIGN_VERIFY usage.
pub fn signing_algorithms_for_spec(spec: &KeySpec) -> Vec<SigningAlgorithmSpec> {
    match spec {
        KeySpec::Rsa2048 | KeySpec::Rsa3072 | KeySpec::Rsa4096 => vec![
            SigningAlgorithmSpec::RsassaPkcs1V15Sha256,
            SigningAlgorithmSpec::RsassaPkcs1V15Sha384,
            SigningAlgorithmSpec::RsassaPkcs1V15Sha512,
            SigningAlgorithmSpec::RsassaPssSha256,
            SigningAlgorithmSpec::RsassaPssSha384,
            SigningAlgorithmSpec::RsassaPssSha512,
        ],
        KeySpec::EccNistP256 => vec![SigningAlgorithmSpec::EcdsaSha256],
        KeySpec::EccNistP384 => vec![SigningAlgorithmSpec::EcdsaSha384],
        KeySpec::EccNistP521 => vec![SigningAlgorithmSpec::EcdsaSha512],
        _ => vec![],
    }
}

/// Get supported MAC algorithms for a key spec + GENERATE_VERIFY_MAC usage.
pub fn mac_algorithms_for_spec(spec: &KeySpec) -> Vec<MacAlgorithmSpec> {
    match spec {
        KeySpec::Hmac224 => vec![MacAlgorithmSpec::HmacSha224],
        KeySpec::Hmac256 => vec![MacAlgorithmSpec::HmacSha256],
        KeySpec::Hmac384 => vec![MacAlgorithmSpec::HmacSha384],
        KeySpec::Hmac512 => vec![MacAlgorithmSpec::HmacSha512],
        _ => vec![],
    }
}

/// Validate that a key is in a usable state for cryptographic operations.
pub fn validate_key_enabled(key: &KmsKey) -> Result<(), KmsError> {
    match key.key_state {
        KeyState::Enabled => Ok(()),
        KeyState::Disabled => Err(KmsError::with_message(
            KmsErrorCode::DisabledException,
            format!("{} is disabled.", key.arn),
        )),
        KeyState::PendingDeletion => Err(KmsError::with_message(
            KmsErrorCode::KMSInvalidStateException,
            format!("{} is pending deletion.", key.arn),
        )),
        _ => Err(KmsError::with_message(
            KmsErrorCode::KMSInvalidStateException,
            format!(
                "{} is in state {} which is not valid for this operation.",
                key.arn,
                key.key_state.as_str()
            ),
        )),
    }
}

/// Validate key usage matches the expected type.
pub fn validate_key_usage(key: &KmsKey, expected: &KeyUsageType) -> Result<(), KmsError> {
    if key.key_usage == *expected {
        Ok(())
    } else {
        Err(KmsError::with_message(
            KmsErrorCode::InvalidKeyUsageException,
            format!(
                "Key {} usage is {}, which does not match the expected {}.",
                key.key_id,
                key.key_usage.as_str(),
                expected.as_str()
            ),
        ))
    }
}

/// Validate tag key/value lengths.
pub fn validate_tag(key: &str, value: &str) -> Result<(), KmsError> {
    if key.len() > MAX_TAG_KEY_LEN {
        return Err(KmsError::with_message(
            KmsErrorCode::TagException,
            format!("Tag key length exceeds maximum of {MAX_TAG_KEY_LEN} characters"),
        ));
    }
    if value.len() > MAX_TAG_VALUE_LEN {
        return Err(KmsError::with_message(
            KmsErrorCode::TagException,
            format!("Tag value length exceeds maximum of {MAX_TAG_VALUE_LEN} characters"),
        ));
    }
    Ok(())
}
