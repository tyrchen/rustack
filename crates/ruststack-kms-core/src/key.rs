//! KMS key internal representation.

use std::collections::HashMap;

use chrono::{DateTime, Utc};

use ruststack_kms_model::types::{
    EncryptionAlgorithmSpec, KeySpec, KeyState, KeyUsageType, MacAlgorithmSpec, OriginType,
    SigningAlgorithmSpec,
};

/// Internal KMS key representation with metadata and crypto material.
#[derive(Clone)]
pub struct KmsKey {
    /// The key ID (UUID).
    pub key_id: String,
    /// The key ARN.
    pub arn: String,
    /// AWS account ID.
    pub account_id: String,
    /// AWS region.
    pub region: String,
    /// Key spec (algorithm type).
    pub key_spec: KeySpec,
    /// Key usage type.
    pub key_usage: KeyUsageType,
    /// Current key state.
    pub key_state: KeyState,
    /// Human-readable description.
    pub description: String,
    /// Whether the key is enabled.
    pub enabled: bool,
    /// Creation timestamp.
    pub creation_date: DateTime<Utc>,
    /// Scheduled deletion date (if pending deletion).
    pub deletion_date: Option<DateTime<Utc>>,
    /// Pending deletion window in days.
    pub pending_deletion_window_in_days: Option<i32>,
    /// Key origin.
    pub origin: OriginType,
    /// Whether this is a multi-region key.
    pub multi_region: bool,
    /// Key policy document (JSON string).
    pub policy: String,
    /// Tags associated with this key.
    pub tags: HashMap<String, String>,
    /// Whether key rotation is enabled.
    pub rotation_enabled: bool,
    /// Rotation period in days.
    pub rotation_period_in_days: Option<i32>,
    /// The raw cryptographic key material.
    pub key_material: KeyMaterial,
    /// Supported encryption algorithms.
    pub encryption_algorithms: Vec<EncryptionAlgorithmSpec>,
    /// Supported signing algorithms.
    pub signing_algorithms: Vec<SigningAlgorithmSpec>,
    /// Supported MAC algorithms.
    pub mac_algorithms: Vec<MacAlgorithmSpec>,
}

/// Custom `Debug` for `KmsKey` that redacts key material to prevent
/// accidental logging of sensitive cryptographic secrets.
#[allow(clippy::missing_fields_in_debug)]
impl std::fmt::Debug for KmsKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KmsKey")
            .field("key_id", &self.key_id)
            .field("arn", &self.arn)
            .field("key_spec", &self.key_spec)
            .field("key_usage", &self.key_usage)
            .field("key_state", &self.key_state)
            .field("enabled", &self.enabled)
            .field("key_material", &"<REDACTED>")
            .finish_non_exhaustive()
    }
}

/// Cryptographic key material for a KMS key.
///
/// Key material is never exposed in `Debug` output to prevent accidental
/// logging of sensitive cryptographic secrets.
#[derive(Clone)]
pub enum KeyMaterial {
    /// AES-256 symmetric key (32 bytes).
    Symmetric {
        /// Raw key bytes.
        key: Vec<u8>,
    },
    /// RSA key pair.
    Rsa {
        /// PKCS#8 DER-encoded private key.
        private_key_der: Vec<u8>,
        /// DER-encoded SubjectPublicKeyInfo.
        public_key_der: Vec<u8>,
    },
    /// ECDSA key pair.
    Ec {
        /// PKCS#8 DER-encoded private key.
        private_key_der: Vec<u8>,
        /// Uncompressed public key bytes.
        public_key_der: Vec<u8>,
    },
    /// HMAC key.
    Hmac {
        /// Raw HMAC key bytes.
        key: Vec<u8>,
    },
}

impl std::fmt::Debug for KeyMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Symmetric { .. } => f.write_str("Symmetric(<REDACTED>)"),
            Self::Rsa { .. } => f.write_str("Rsa(<REDACTED>)"),
            Self::Ec { .. } => f.write_str("Ec(<REDACTED>)"),
            Self::Hmac { .. } => f.write_str("Hmac(<REDACTED>)"),
        }
    }
}

impl KmsKey {
    /// Build the key ARN from components.
    pub fn build_arn(account_id: &str, region: &str, key_id: &str) -> String {
        format!("arn:aws:kms:{region}:{account_id}:key/{key_id}")
    }

    /// Check if the key is in a usable state for cryptographic operations.
    pub fn is_usable(&self) -> bool {
        self.key_state == KeyState::Enabled
    }

    /// Check if this key supports the given encryption algorithm.
    pub fn supports_encryption_algorithm(&self, alg: &EncryptionAlgorithmSpec) -> bool {
        self.encryption_algorithms.contains(alg)
    }

    /// Check if this key supports the given signing algorithm.
    pub fn supports_signing_algorithm(&self, alg: &SigningAlgorithmSpec) -> bool {
        self.signing_algorithms.contains(alg)
    }

    /// Check if this key supports the given MAC algorithm.
    pub fn supports_mac_algorithm(&self, alg: &MacAlgorithmSpec) -> bool {
        self.mac_algorithms.contains(alg)
    }
}
