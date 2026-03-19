//! Auto-generated from AWS KMS Smithy model. DO NOT EDIT.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// KMS CustomerMasterKeySpec enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum CustomerMasterKeySpec {
    /// Default variant.
    #[default]
    #[serde(rename = "ECC_NIST_P256")]
    EccNistP256,
    #[serde(rename = "ECC_NIST_P384")]
    EccNistP384,
    #[serde(rename = "ECC_NIST_P521")]
    EccNistP521,
    #[serde(rename = "ECC_SECG_P256K1")]
    EccSecgP256k1,
    #[serde(rename = "HMAC_224")]
    Hmac224,
    #[serde(rename = "HMAC_256")]
    Hmac256,
    #[serde(rename = "HMAC_384")]
    Hmac384,
    #[serde(rename = "HMAC_512")]
    Hmac512,
    #[serde(rename = "RSA_2048")]
    Rsa2048,
    #[serde(rename = "RSA_3072")]
    Rsa3072,
    #[serde(rename = "RSA_4096")]
    Rsa4096,
    #[serde(rename = "SM2")]
    Sm2,
    #[serde(rename = "SYMMETRIC_DEFAULT")]
    SymmetricDefault,
}

impl CustomerMasterKeySpec {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::EccNistP256 => "ECC_NIST_P256",
            Self::EccNistP384 => "ECC_NIST_P384",
            Self::EccNistP521 => "ECC_NIST_P521",
            Self::EccSecgP256k1 => "ECC_SECG_P256K1",
            Self::Hmac224 => "HMAC_224",
            Self::Hmac256 => "HMAC_256",
            Self::Hmac384 => "HMAC_384",
            Self::Hmac512 => "HMAC_512",
            Self::Rsa2048 => "RSA_2048",
            Self::Rsa3072 => "RSA_3072",
            Self::Rsa4096 => "RSA_4096",
            Self::Sm2 => "SM2",
            Self::SymmetricDefault => "SYMMETRIC_DEFAULT",
        }
    }
}

impl std::fmt::Display for CustomerMasterKeySpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for CustomerMasterKeySpec {
    fn from(s: &str) -> Self {
        match s {
            "ECC_NIST_P256" => Self::EccNistP256,
            "ECC_NIST_P384" => Self::EccNistP384,
            "ECC_NIST_P521" => Self::EccNistP521,
            "ECC_SECG_P256K1" => Self::EccSecgP256k1,
            "HMAC_224" => Self::Hmac224,
            "HMAC_256" => Self::Hmac256,
            "HMAC_384" => Self::Hmac384,
            "HMAC_512" => Self::Hmac512,
            "RSA_2048" => Self::Rsa2048,
            "RSA_3072" => Self::Rsa3072,
            "RSA_4096" => Self::Rsa4096,
            "SM2" => Self::Sm2,
            "SYMMETRIC_DEFAULT" => Self::SymmetricDefault,
            _ => Self::default(),
        }
    }
}

/// KMS DataKeyPairSpec enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum DataKeyPairSpec {
    /// Default variant.
    #[default]
    #[serde(rename = "ECC_NIST_EDWARDS25519")]
    EccNistEdwards25519,
    #[serde(rename = "ECC_NIST_P256")]
    EccNistP256,
    #[serde(rename = "ECC_NIST_P384")]
    EccNistP384,
    #[serde(rename = "ECC_NIST_P521")]
    EccNistP521,
    #[serde(rename = "ECC_SECG_P256K1")]
    EccSecgP256k1,
    #[serde(rename = "RSA_2048")]
    Rsa2048,
    #[serde(rename = "RSA_3072")]
    Rsa3072,
    #[serde(rename = "RSA_4096")]
    Rsa4096,
    #[serde(rename = "SM2")]
    Sm2,
}

impl DataKeyPairSpec {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::EccNistEdwards25519 => "ECC_NIST_EDWARDS25519",
            Self::EccNistP256 => "ECC_NIST_P256",
            Self::EccNistP384 => "ECC_NIST_P384",
            Self::EccNistP521 => "ECC_NIST_P521",
            Self::EccSecgP256k1 => "ECC_SECG_P256K1",
            Self::Rsa2048 => "RSA_2048",
            Self::Rsa3072 => "RSA_3072",
            Self::Rsa4096 => "RSA_4096",
            Self::Sm2 => "SM2",
        }
    }
}

impl std::fmt::Display for DataKeyPairSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for DataKeyPairSpec {
    fn from(s: &str) -> Self {
        match s {
            "ECC_NIST_EDWARDS25519" => Self::EccNistEdwards25519,
            "ECC_NIST_P256" => Self::EccNistP256,
            "ECC_NIST_P384" => Self::EccNistP384,
            "ECC_NIST_P521" => Self::EccNistP521,
            "ECC_SECG_P256K1" => Self::EccSecgP256k1,
            "RSA_2048" => Self::Rsa2048,
            "RSA_3072" => Self::Rsa3072,
            "RSA_4096" => Self::Rsa4096,
            "SM2" => Self::Sm2,
            _ => Self::default(),
        }
    }
}

/// KMS DataKeySpec enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum DataKeySpec {
    /// Default variant.
    #[default]
    #[serde(rename = "AES_128")]
    Aes128,
    #[serde(rename = "AES_256")]
    Aes256,
}

impl DataKeySpec {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Aes128 => "AES_128",
            Self::Aes256 => "AES_256",
        }
    }
}

impl std::fmt::Display for DataKeySpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for DataKeySpec {
    fn from(s: &str) -> Self {
        match s {
            "AES_128" => Self::Aes128,
            "AES_256" => Self::Aes256,
            _ => Self::default(),
        }
    }
}

/// KMS DryRunModifierType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum DryRunModifierType {
    /// Default variant.
    #[default]
    #[serde(rename = "IGNORE_CIPHERTEXT")]
    IgnoreCiphertext,
}

impl DryRunModifierType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::IgnoreCiphertext => "IGNORE_CIPHERTEXT",
        }
    }
}

impl std::fmt::Display for DryRunModifierType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for DryRunModifierType {
    fn from(s: &str) -> Self {
        match s {
            "IGNORE_CIPHERTEXT" => Self::IgnoreCiphertext,
            _ => Self::default(),
        }
    }
}

/// KMS EncryptionAlgorithmSpec enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum EncryptionAlgorithmSpec {
    /// Default variant.
    #[default]
    #[serde(rename = "RSAES_OAEP_SHA_1")]
    RsaesOaepSha1,
    #[serde(rename = "RSAES_OAEP_SHA_256")]
    RsaesOaepSha256,
    #[serde(rename = "SM2PKE")]
    Sm2pke,
    #[serde(rename = "SYMMETRIC_DEFAULT")]
    SymmetricDefault,
}

impl EncryptionAlgorithmSpec {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RsaesOaepSha1 => "RSAES_OAEP_SHA_1",
            Self::RsaesOaepSha256 => "RSAES_OAEP_SHA_256",
            Self::Sm2pke => "SM2PKE",
            Self::SymmetricDefault => "SYMMETRIC_DEFAULT",
        }
    }
}

impl std::fmt::Display for EncryptionAlgorithmSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for EncryptionAlgorithmSpec {
    fn from(s: &str) -> Self {
        match s {
            "RSAES_OAEP_SHA_1" => Self::RsaesOaepSha1,
            "RSAES_OAEP_SHA_256" => Self::RsaesOaepSha256,
            "SM2PKE" => Self::Sm2pke,
            "SYMMETRIC_DEFAULT" => Self::SymmetricDefault,
            _ => Self::default(),
        }
    }
}

/// KMS ExpirationModelType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ExpirationModelType {
    /// Default variant.
    #[default]
    #[serde(rename = "KEY_MATERIAL_DOES_NOT_EXPIRE")]
    KeyMaterialDoesNotExpire,
    #[serde(rename = "KEY_MATERIAL_EXPIRES")]
    KeyMaterialExpires,
}

impl ExpirationModelType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::KeyMaterialDoesNotExpire => "KEY_MATERIAL_DOES_NOT_EXPIRE",
            Self::KeyMaterialExpires => "KEY_MATERIAL_EXPIRES",
        }
    }
}

impl std::fmt::Display for ExpirationModelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ExpirationModelType {
    fn from(s: &str) -> Self {
        match s {
            "KEY_MATERIAL_DOES_NOT_EXPIRE" => Self::KeyMaterialDoesNotExpire,
            "KEY_MATERIAL_EXPIRES" => Self::KeyMaterialExpires,
            _ => Self::default(),
        }
    }
}

/// KMS GrantOperation enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum GrantOperation {
    /// Default variant.
    #[default]
    CreateGrant,
    Decrypt,
    DeriveSharedSecret,
    DescribeKey,
    Encrypt,
    GenerateDataKey,
    GenerateDataKeyPair,
    GenerateDataKeyPairWithoutPlaintext,
    GenerateDataKeyWithoutPlaintext,
    GenerateMac,
    GetPublicKey,
    ReEncryptFrom,
    ReEncryptTo,
    RetireGrant,
    Sign,
    Verify,
    VerifyMac,
}

impl GrantOperation {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CreateGrant => "CreateGrant",
            Self::Decrypt => "Decrypt",
            Self::DeriveSharedSecret => "DeriveSharedSecret",
            Self::DescribeKey => "DescribeKey",
            Self::Encrypt => "Encrypt",
            Self::GenerateDataKey => "GenerateDataKey",
            Self::GenerateDataKeyPair => "GenerateDataKeyPair",
            Self::GenerateDataKeyPairWithoutPlaintext => "GenerateDataKeyPairWithoutPlaintext",
            Self::GenerateDataKeyWithoutPlaintext => "GenerateDataKeyWithoutPlaintext",
            Self::GenerateMac => "GenerateMac",
            Self::GetPublicKey => "GetPublicKey",
            Self::ReEncryptFrom => "ReEncryptFrom",
            Self::ReEncryptTo => "ReEncryptTo",
            Self::RetireGrant => "RetireGrant",
            Self::Sign => "Sign",
            Self::Verify => "Verify",
            Self::VerifyMac => "VerifyMac",
        }
    }
}

impl std::fmt::Display for GrantOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for GrantOperation {
    fn from(s: &str) -> Self {
        match s {
            "CreateGrant" => Self::CreateGrant,
            "Decrypt" => Self::Decrypt,
            "DeriveSharedSecret" => Self::DeriveSharedSecret,
            "DescribeKey" => Self::DescribeKey,
            "Encrypt" => Self::Encrypt,
            "GenerateDataKey" => Self::GenerateDataKey,
            "GenerateDataKeyPair" => Self::GenerateDataKeyPair,
            "GenerateDataKeyPairWithoutPlaintext" => Self::GenerateDataKeyPairWithoutPlaintext,
            "GenerateDataKeyWithoutPlaintext" => Self::GenerateDataKeyWithoutPlaintext,
            "GenerateMac" => Self::GenerateMac,
            "GetPublicKey" => Self::GetPublicKey,
            "ReEncryptFrom" => Self::ReEncryptFrom,
            "ReEncryptTo" => Self::ReEncryptTo,
            "RetireGrant" => Self::RetireGrant,
            "Sign" => Self::Sign,
            "Verify" => Self::Verify,
            "VerifyMac" => Self::VerifyMac,
            _ => Self::default(),
        }
    }
}

/// KMS KeyAgreementAlgorithmSpec enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum KeyAgreementAlgorithmSpec {
    /// Default variant.
    #[default]
    #[serde(rename = "ECDH")]
    Ecdh,
}

impl KeyAgreementAlgorithmSpec {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ecdh => "ECDH",
        }
    }
}

impl std::fmt::Display for KeyAgreementAlgorithmSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for KeyAgreementAlgorithmSpec {
    fn from(s: &str) -> Self {
        match s {
            "ECDH" => Self::Ecdh,
            _ => Self::default(),
        }
    }
}

/// KMS KeyEncryptionMechanism enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum KeyEncryptionMechanism {
    /// Default variant.
    #[default]
    #[serde(rename = "RSAES_OAEP_SHA_256")]
    RsaesOaepSha256,
}

impl KeyEncryptionMechanism {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RsaesOaepSha256 => "RSAES_OAEP_SHA_256",
        }
    }
}

impl std::fmt::Display for KeyEncryptionMechanism {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for KeyEncryptionMechanism {
    fn from(s: &str) -> Self {
        match s {
            "RSAES_OAEP_SHA_256" => Self::RsaesOaepSha256,
            _ => Self::default(),
        }
    }
}

/// KMS KeyManagerType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum KeyManagerType {
    /// Default variant.
    #[default]
    #[serde(rename = "AWS")]
    Aws,
    #[serde(rename = "CUSTOMER")]
    Customer,
}

impl KeyManagerType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Aws => "AWS",
            Self::Customer => "CUSTOMER",
        }
    }
}

impl std::fmt::Display for KeyManagerType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for KeyManagerType {
    fn from(s: &str) -> Self {
        match s {
            "AWS" => Self::Aws,
            "CUSTOMER" => Self::Customer,
            _ => Self::default(),
        }
    }
}

/// KMS KeySpec enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum KeySpec {
    /// Default variant.
    #[default]
    #[serde(rename = "ECC_NIST_EDWARDS25519")]
    EccNistEdwards25519,
    #[serde(rename = "ECC_NIST_P256")]
    EccNistP256,
    #[serde(rename = "ECC_NIST_P384")]
    EccNistP384,
    #[serde(rename = "ECC_NIST_P521")]
    EccNistP521,
    #[serde(rename = "ECC_SECG_P256K1")]
    EccSecgP256k1,
    #[serde(rename = "HMAC_224")]
    Hmac224,
    #[serde(rename = "HMAC_256")]
    Hmac256,
    #[serde(rename = "HMAC_384")]
    Hmac384,
    #[serde(rename = "HMAC_512")]
    Hmac512,
    #[serde(rename = "ML_DSA_44")]
    MlDsa44,
    #[serde(rename = "ML_DSA_65")]
    MlDsa65,
    #[serde(rename = "ML_DSA_87")]
    MlDsa87,
    #[serde(rename = "RSA_2048")]
    Rsa2048,
    #[serde(rename = "RSA_3072")]
    Rsa3072,
    #[serde(rename = "RSA_4096")]
    Rsa4096,
    #[serde(rename = "SM2")]
    Sm2,
    #[serde(rename = "SYMMETRIC_DEFAULT")]
    SymmetricDefault,
}

impl KeySpec {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::EccNistEdwards25519 => "ECC_NIST_EDWARDS25519",
            Self::EccNistP256 => "ECC_NIST_P256",
            Self::EccNistP384 => "ECC_NIST_P384",
            Self::EccNistP521 => "ECC_NIST_P521",
            Self::EccSecgP256k1 => "ECC_SECG_P256K1",
            Self::Hmac224 => "HMAC_224",
            Self::Hmac256 => "HMAC_256",
            Self::Hmac384 => "HMAC_384",
            Self::Hmac512 => "HMAC_512",
            Self::MlDsa44 => "ML_DSA_44",
            Self::MlDsa65 => "ML_DSA_65",
            Self::MlDsa87 => "ML_DSA_87",
            Self::Rsa2048 => "RSA_2048",
            Self::Rsa3072 => "RSA_3072",
            Self::Rsa4096 => "RSA_4096",
            Self::Sm2 => "SM2",
            Self::SymmetricDefault => "SYMMETRIC_DEFAULT",
        }
    }
}

impl std::fmt::Display for KeySpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for KeySpec {
    fn from(s: &str) -> Self {
        match s {
            "ECC_NIST_EDWARDS25519" => Self::EccNistEdwards25519,
            "ECC_NIST_P256" => Self::EccNistP256,
            "ECC_NIST_P384" => Self::EccNistP384,
            "ECC_NIST_P521" => Self::EccNistP521,
            "ECC_SECG_P256K1" => Self::EccSecgP256k1,
            "HMAC_224" => Self::Hmac224,
            "HMAC_256" => Self::Hmac256,
            "HMAC_384" => Self::Hmac384,
            "HMAC_512" => Self::Hmac512,
            "ML_DSA_44" => Self::MlDsa44,
            "ML_DSA_65" => Self::MlDsa65,
            "ML_DSA_87" => Self::MlDsa87,
            "RSA_2048" => Self::Rsa2048,
            "RSA_3072" => Self::Rsa3072,
            "RSA_4096" => Self::Rsa4096,
            "SM2" => Self::Sm2,
            "SYMMETRIC_DEFAULT" => Self::SymmetricDefault,
            _ => Self::default(),
        }
    }
}

/// KMS KeyState enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum KeyState {
    /// Default variant.
    #[default]
    Creating,
    Disabled,
    Enabled,
    PendingDeletion,
    PendingImport,
    PendingReplicaDeletion,
    Unavailable,
    Updating,
}

impl KeyState {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Creating => "Creating",
            Self::Disabled => "Disabled",
            Self::Enabled => "Enabled",
            Self::PendingDeletion => "PendingDeletion",
            Self::PendingImport => "PendingImport",
            Self::PendingReplicaDeletion => "PendingReplicaDeletion",
            Self::Unavailable => "Unavailable",
            Self::Updating => "Updating",
        }
    }
}

impl std::fmt::Display for KeyState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for KeyState {
    fn from(s: &str) -> Self {
        match s {
            "Creating" => Self::Creating,
            "Disabled" => Self::Disabled,
            "Enabled" => Self::Enabled,
            "PendingDeletion" => Self::PendingDeletion,
            "PendingImport" => Self::PendingImport,
            "PendingReplicaDeletion" => Self::PendingReplicaDeletion,
            "Unavailable" => Self::Unavailable,
            "Updating" => Self::Updating,
            _ => Self::default(),
        }
    }
}

/// KMS KeyUsageType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum KeyUsageType {
    /// Default variant.
    #[default]
    #[serde(rename = "ENCRYPT_DECRYPT")]
    EncryptDecrypt,
    #[serde(rename = "GENERATE_VERIFY_MAC")]
    GenerateVerifyMac,
    #[serde(rename = "KEY_AGREEMENT")]
    KeyAgreement,
    #[serde(rename = "SIGN_VERIFY")]
    SignVerify,
}

impl KeyUsageType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::EncryptDecrypt => "ENCRYPT_DECRYPT",
            Self::GenerateVerifyMac => "GENERATE_VERIFY_MAC",
            Self::KeyAgreement => "KEY_AGREEMENT",
            Self::SignVerify => "SIGN_VERIFY",
        }
    }
}

impl std::fmt::Display for KeyUsageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for KeyUsageType {
    fn from(s: &str) -> Self {
        match s {
            "ENCRYPT_DECRYPT" => Self::EncryptDecrypt,
            "GENERATE_VERIFY_MAC" => Self::GenerateVerifyMac,
            "KEY_AGREEMENT" => Self::KeyAgreement,
            "SIGN_VERIFY" => Self::SignVerify,
            _ => Self::default(),
        }
    }
}

/// KMS MacAlgorithmSpec enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum MacAlgorithmSpec {
    /// Default variant.
    #[default]
    #[serde(rename = "HMAC_SHA_224")]
    HmacSha224,
    #[serde(rename = "HMAC_SHA_256")]
    HmacSha256,
    #[serde(rename = "HMAC_SHA_384")]
    HmacSha384,
    #[serde(rename = "HMAC_SHA_512")]
    HmacSha512,
}

impl MacAlgorithmSpec {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::HmacSha224 => "HMAC_SHA_224",
            Self::HmacSha256 => "HMAC_SHA_256",
            Self::HmacSha384 => "HMAC_SHA_384",
            Self::HmacSha512 => "HMAC_SHA_512",
        }
    }
}

impl std::fmt::Display for MacAlgorithmSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for MacAlgorithmSpec {
    fn from(s: &str) -> Self {
        match s {
            "HMAC_SHA_224" => Self::HmacSha224,
            "HMAC_SHA_256" => Self::HmacSha256,
            "HMAC_SHA_384" => Self::HmacSha384,
            "HMAC_SHA_512" => Self::HmacSha512,
            _ => Self::default(),
        }
    }
}

/// KMS MessageType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum MessageType {
    /// Default variant.
    #[default]
    #[serde(rename = "DIGEST")]
    Digest,
    #[serde(rename = "EXTERNAL_MU")]
    ExternalMu,
    #[serde(rename = "RAW")]
    Raw,
}

impl MessageType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Digest => "DIGEST",
            Self::ExternalMu => "EXTERNAL_MU",
            Self::Raw => "RAW",
        }
    }
}

impl std::fmt::Display for MessageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for MessageType {
    fn from(s: &str) -> Self {
        match s {
            "DIGEST" => Self::Digest,
            "EXTERNAL_MU" => Self::ExternalMu,
            "RAW" => Self::Raw,
            _ => Self::default(),
        }
    }
}

/// KMS MultiRegionKeyType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum MultiRegionKeyType {
    /// Default variant.
    #[default]
    #[serde(rename = "PRIMARY")]
    Primary,
    #[serde(rename = "REPLICA")]
    Replica,
}

impl MultiRegionKeyType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Primary => "PRIMARY",
            Self::Replica => "REPLICA",
        }
    }
}

impl std::fmt::Display for MultiRegionKeyType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for MultiRegionKeyType {
    fn from(s: &str) -> Self {
        match s {
            "PRIMARY" => Self::Primary,
            "REPLICA" => Self::Replica,
            _ => Self::default(),
        }
    }
}

/// KMS OriginType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum OriginType {
    /// Default variant.
    #[default]
    #[serde(rename = "AWS_CLOUDHSM")]
    AwsCloudhsm,
    #[serde(rename = "AWS_KMS")]
    AwsKms,
    #[serde(rename = "EXTERNAL")]
    External,
    #[serde(rename = "EXTERNAL_KEY_STORE")]
    ExternalKeyStore,
}

impl OriginType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AwsCloudhsm => "AWS_CLOUDHSM",
            Self::AwsKms => "AWS_KMS",
            Self::External => "EXTERNAL",
            Self::ExternalKeyStore => "EXTERNAL_KEY_STORE",
        }
    }
}

impl std::fmt::Display for OriginType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for OriginType {
    fn from(s: &str) -> Self {
        match s {
            "AWS_CLOUDHSM" => Self::AwsCloudhsm,
            "AWS_KMS" => Self::AwsKms,
            "EXTERNAL" => Self::External,
            "EXTERNAL_KEY_STORE" => Self::ExternalKeyStore,
            _ => Self::default(),
        }
    }
}

/// KMS SigningAlgorithmSpec enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum SigningAlgorithmSpec {
    /// Default variant.
    #[default]
    #[serde(rename = "ECDSA_SHA_256")]
    EcdsaSha256,
    #[serde(rename = "ECDSA_SHA_384")]
    EcdsaSha384,
    #[serde(rename = "ECDSA_SHA_512")]
    EcdsaSha512,
    #[serde(rename = "ED25519_PH_SHA_512")]
    Ed25519PhSha512,
    #[serde(rename = "ED25519_SHA_512")]
    Ed25519Sha512,
    #[serde(rename = "ML_DSA_SHAKE_256")]
    MlDsaShake256,
    #[serde(rename = "RSASSA_PKCS1_V1_5_SHA_256")]
    RsassaPkcs1V15Sha256,
    #[serde(rename = "RSASSA_PKCS1_V1_5_SHA_384")]
    RsassaPkcs1V15Sha384,
    #[serde(rename = "RSASSA_PKCS1_V1_5_SHA_512")]
    RsassaPkcs1V15Sha512,
    #[serde(rename = "RSASSA_PSS_SHA_256")]
    RsassaPssSha256,
    #[serde(rename = "RSASSA_PSS_SHA_384")]
    RsassaPssSha384,
    #[serde(rename = "RSASSA_PSS_SHA_512")]
    RsassaPssSha512,
    #[serde(rename = "SM2DSA")]
    Sm2dsa,
}

impl SigningAlgorithmSpec {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::EcdsaSha256 => "ECDSA_SHA_256",
            Self::EcdsaSha384 => "ECDSA_SHA_384",
            Self::EcdsaSha512 => "ECDSA_SHA_512",
            Self::Ed25519PhSha512 => "ED25519_PH_SHA_512",
            Self::Ed25519Sha512 => "ED25519_SHA_512",
            Self::MlDsaShake256 => "ML_DSA_SHAKE_256",
            Self::RsassaPkcs1V15Sha256 => "RSASSA_PKCS1_V1_5_SHA_256",
            Self::RsassaPkcs1V15Sha384 => "RSASSA_PKCS1_V1_5_SHA_384",
            Self::RsassaPkcs1V15Sha512 => "RSASSA_PKCS1_V1_5_SHA_512",
            Self::RsassaPssSha256 => "RSASSA_PSS_SHA_256",
            Self::RsassaPssSha384 => "RSASSA_PSS_SHA_384",
            Self::RsassaPssSha512 => "RSASSA_PSS_SHA_512",
            Self::Sm2dsa => "SM2DSA",
        }
    }
}

impl std::fmt::Display for SigningAlgorithmSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for SigningAlgorithmSpec {
    fn from(s: &str) -> Self {
        match s {
            "ECDSA_SHA_256" => Self::EcdsaSha256,
            "ECDSA_SHA_384" => Self::EcdsaSha384,
            "ECDSA_SHA_512" => Self::EcdsaSha512,
            "ED25519_PH_SHA_512" => Self::Ed25519PhSha512,
            "ED25519_SHA_512" => Self::Ed25519Sha512,
            "ML_DSA_SHAKE_256" => Self::MlDsaShake256,
            "RSASSA_PKCS1_V1_5_SHA_256" => Self::RsassaPkcs1V15Sha256,
            "RSASSA_PKCS1_V1_5_SHA_384" => Self::RsassaPkcs1V15Sha384,
            "RSASSA_PKCS1_V1_5_SHA_512" => Self::RsassaPkcs1V15Sha512,
            "RSASSA_PSS_SHA_256" => Self::RsassaPssSha256,
            "RSASSA_PSS_SHA_384" => Self::RsassaPssSha384,
            "RSASSA_PSS_SHA_512" => Self::RsassaPssSha512,
            "SM2DSA" => Self::Sm2dsa,
            _ => Self::default(),
        }
    }
}

/// KMS AliasListEntry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AliasListEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias_name: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::epoch_seconds::option::serialize",
        deserialize_with = "crate::epoch_seconds::option::deserialize"
    )]
    pub creation_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::epoch_seconds::option::serialize",
        deserialize_with = "crate::epoch_seconds::option::deserialize"
    )]
    pub last_updated_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_key_id: Option<String>,
}

/// KMS GrantConstraints.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GrantConstraints {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub encryption_context_equals: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub encryption_context_subset: HashMap<String, String>,
}

/// KMS GrantListEntry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GrantListEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub constraints: Option<GrantConstraints>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::epoch_seconds::option::serialize",
        deserialize_with = "crate::epoch_seconds::option::deserialize"
    )]
    pub creation_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grantee_principal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issuing_account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<GrantOperation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retiring_principal: Option<String>,
}

/// KMS KeyListEntry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct KeyListEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
}

/// KMS KeyMetadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct KeyMetadata {
    #[serde(rename = "AWSAccountId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aws_account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cloud_hsm_cluster_id: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::epoch_seconds::option::serialize",
        deserialize_with = "crate::epoch_seconds::option::deserialize"
    )]
    pub creation_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_key_material_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_key_store_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub customer_master_key_spec: Option<CustomerMasterKeySpec>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::epoch_seconds::option::serialize",
        deserialize_with = "crate::epoch_seconds::option::deserialize"
    )]
    pub deletion_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub encryption_algorithms: Vec<EncryptionAlgorithmSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiration_model: Option<ExpirationModelType>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub key_agreement_algorithms: Vec<KeyAgreementAlgorithmSpec>,
    pub key_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_manager: Option<KeyManagerType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_spec: Option<KeySpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_state: Option<KeyState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_usage: Option<KeyUsageType>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mac_algorithms: Vec<MacAlgorithmSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multi_region: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multi_region_configuration: Option<MultiRegionConfiguration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<OriginType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_deletion_window_in_days: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signing_algorithms: Vec<SigningAlgorithmSpec>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::epoch_seconds::option::serialize",
        deserialize_with = "crate::epoch_seconds::option::deserialize"
    )]
    pub valid_to: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xks_key_configuration: Option<XksKeyConfigurationType>,
}

/// KMS MultiRegionConfiguration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MultiRegionConfiguration {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multi_region_key_type: Option<MultiRegionKeyType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_key: Option<MultiRegionKey>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replica_keys: Vec<MultiRegionKey>,
}

/// KMS MultiRegionKey.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MultiRegionKey {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
}

/// KMS RecipientInfo.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RecipientInfo {
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::blob::option::serialize",
        deserialize_with = "crate::blob::option::deserialize"
    )]
    pub attestation_document: Option<bytes::Bytes>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_encryption_algorithm: Option<KeyEncryptionMechanism>,
}

/// KMS Tag.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Tag {
    pub tag_key: String,
    pub tag_value: String,
}

/// KMS XksKeyConfigurationType.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct XksKeyConfigurationType {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}
