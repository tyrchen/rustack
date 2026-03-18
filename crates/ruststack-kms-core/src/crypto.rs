//! Cryptographic operations using `aws-lc-rs`.
//!
//! Provides AES-256-GCM symmetric encryption, RSA OAEP encryption/decryption,
//! RSA and ECDSA signing/verification, and HMAC operations.

use std::collections::HashMap;

use aws_lc_rs::aead::{self, Aad, BoundKey, NONCE_LEN, Nonce, NonceSequence, SealingKey};
use aws_lc_rs::encoding::AsDer;
use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use aws_lc_rs::signature::{self, EcdsaKeyPair, KeyPair as _, RsaKeyPair};

use ruststack_kms_model::error::{KmsError, KmsErrorCode};
use ruststack_kms_model::types::{
    DataKeyPairSpec, DataKeySpec, EncryptionAlgorithmSpec, KeySpec, MacAlgorithmSpec,
    SigningAlgorithmSpec,
};

use crate::ciphertext;
use crate::key::KeyMaterial;

/// Thread-safe random number generator.
fn rng() -> &'static SystemRandom {
    static RNG: std::sync::OnceLock<SystemRandom> = std::sync::OnceLock::new();
    RNG.get_or_init(SystemRandom::new)
}

/// A one-shot nonce sequence that provides a single nonce then errors.
struct OneShotNonce(Option<[u8; NONCE_LEN]>);

impl NonceSequence for OneShotNonce {
    fn advance(&mut self) -> Result<Nonce, aws_lc_rs::error::Unspecified> {
        self.0
            .take()
            .map(Nonce::assume_unique_for_key)
            .ok_or(aws_lc_rs::error::Unspecified)
    }
}

// ---------------------------------------------------------------------------
// Key Generation
// ---------------------------------------------------------------------------

/// Generate key material for the given key spec.
pub fn generate_key_material(spec: &KeySpec) -> Result<KeyMaterial, KmsError> {
    match spec {
        KeySpec::SymmetricDefault => generate_symmetric_key(),
        KeySpec::Rsa2048 => generate_rsa_key(2048),
        KeySpec::Rsa3072 => generate_rsa_key(3072),
        KeySpec::Rsa4096 => generate_rsa_key(4096),
        KeySpec::EccNistP256 => generate_ec_key(&signature::ECDSA_P256_SHA256_ASN1_SIGNING),
        KeySpec::EccNistP384 => generate_ec_key(&signature::ECDSA_P384_SHA384_ASN1_SIGNING),
        KeySpec::EccNistP521 => generate_ec_p521_key(),
        KeySpec::Hmac224 => generate_hmac_key(28),
        KeySpec::Hmac256 => generate_hmac_key(32),
        KeySpec::Hmac384 => generate_hmac_key(48),
        KeySpec::Hmac512 => generate_hmac_key(64),
        _ => Err(KmsError::with_message(
            KmsErrorCode::UnsupportedOperationException,
            format!("Key spec {} is not supported", spec.as_str()),
        )),
    }
}

/// Generate a 256-bit symmetric key.
fn generate_symmetric_key() -> Result<KeyMaterial, KmsError> {
    let mut key = vec![0u8; 32];
    rng()
        .fill(&mut key)
        .map_err(|_| KmsError::internal_error("Failed to generate random bytes"))?;
    Ok(KeyMaterial::Symmetric { key })
}

/// Generate an RSA key pair.
fn generate_rsa_key(bits: usize) -> Result<KeyMaterial, KmsError> {
    let key_size = match bits {
        2048 => aws_lc_rs::rsa::KeySize::Rsa2048,
        3072 => aws_lc_rs::rsa::KeySize::Rsa3072,
        4096 => aws_lc_rs::rsa::KeySize::Rsa4096,
        _ => {
            return Err(KmsError::internal_error(format!(
                "Unsupported RSA key size: {bits}"
            )));
        }
    };

    let key_pair = RsaKeyPair::generate(key_size)
        .map_err(|e| KmsError::internal_error(format!("RSA key generation failed: {e}")))?;

    let private_key_der = key_pair.as_der().map_err(|e| {
        KmsError::internal_error(format!("Failed to serialize RSA private key: {e}"))
    })?;

    let public_key_der = key_pair.public_key().as_der().map_err(|e| {
        KmsError::internal_error(format!("Failed to serialize RSA public key: {e}"))
    })?;

    Ok(KeyMaterial::Rsa {
        private_key_der: private_key_der.as_ref().to_vec(),
        public_key_der: public_key_der.as_ref().to_vec(),
    })
}

/// Generate an ECDSA key pair for P-256 or P-384.
fn generate_ec_key(
    alg: &'static signature::EcdsaSigningAlgorithm,
) -> Result<KeyMaterial, KmsError> {
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(alg, rng())
        .map_err(|e| KmsError::internal_error(format!("EC key generation failed: {e}")))?;
    let kp = EcdsaKeyPair::from_pkcs8(alg, pkcs8.as_ref())
        .map_err(|e| KmsError::internal_error(format!("Failed to parse generated EC key: {e}")))?;
    let public_key_der = kp.public_key().as_ref().to_vec();
    Ok(KeyMaterial::Ec {
        private_key_der: pkcs8.as_ref().to_vec(),
        public_key_der,
    })
}

/// Generate a P-521 ECDSA key pair.
fn generate_ec_p521_key() -> Result<KeyMaterial, KmsError> {
    let alg = &signature::ECDSA_P521_SHA512_ASN1_SIGNING;
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(alg, rng())
        .map_err(|e| KmsError::internal_error(format!("P-521 key generation failed: {e}")))?;
    let kp = EcdsaKeyPair::from_pkcs8(alg, pkcs8.as_ref()).map_err(|e| {
        KmsError::internal_error(format!("Failed to parse generated P-521 key: {e}"))
    })?;
    let public_key_der = kp.public_key().as_ref().to_vec();
    Ok(KeyMaterial::Ec {
        private_key_der: pkcs8.as_ref().to_vec(),
        public_key_der,
    })
}

/// Generate an HMAC key of the specified length.
fn generate_hmac_key(len: usize) -> Result<KeyMaterial, KmsError> {
    let mut key = vec![0u8; len];
    rng()
        .fill(&mut key)
        .map_err(|_| KmsError::internal_error("Failed to generate HMAC key bytes"))?;
    Ok(KeyMaterial::Hmac { key })
}

/// Generate random bytes.
pub fn generate_random_bytes(num_bytes: usize) -> Result<Vec<u8>, KmsError> {
    let mut buf = vec![0u8; num_bytes];
    rng()
        .fill(&mut buf)
        .map_err(|_| KmsError::internal_error("Failed to generate random bytes"))?;
    Ok(buf)
}

// ---------------------------------------------------------------------------
// Encryption Context -> AAD
// ---------------------------------------------------------------------------

/// Convert an encryption context map to Additional Authenticated Data (AAD).
///
/// Sort keys lexicographically, then concatenate key+value bytes.
pub fn context_to_aad(context: &HashMap<String, String>) -> Vec<u8> {
    if context.is_empty() {
        return Vec::new();
    }
    let mut keys: Vec<&String> = context.keys().collect();
    keys.sort();
    let mut aad = Vec::new();
    for key in keys {
        aad.extend_from_slice(key.as_bytes());
        if let Some(val) = context.get(key) {
            aad.extend_from_slice(val.as_bytes());
        }
    }
    aad
}

// ---------------------------------------------------------------------------
// Symmetric Encryption (AES-256-GCM)
// ---------------------------------------------------------------------------

/// Encrypt plaintext using AES-256-GCM.
///
/// Returns the full ciphertext blob (key ID + IV + tag + ciphertext).
pub fn symmetric_encrypt(
    key_id: &str,
    key_bytes: &[u8],
    plaintext: &[u8],
    encryption_context: &HashMap<String, String>,
) -> Result<Vec<u8>, KmsError> {
    let aad_bytes = context_to_aad(encryption_context);

    // Generate random IV.
    let mut iv = [0u8; ciphertext::IV_LEN];
    rng()
        .fill(&mut iv)
        .map_err(|_| KmsError::internal_error("Failed to generate IV"))?;

    let unbound_key = aead::UnboundKey::new(&aead::AES_256_GCM, key_bytes)
        .map_err(|e| KmsError::internal_error(format!("Failed to create AES key: {e}")))?;

    let nonce_seq = OneShotNonce(Some(iv));
    let mut sealing_key = SealingKey::new(unbound_key, nonce_seq);

    let mut in_out = plaintext.to_vec();
    let tag = sealing_key
        .seal_in_place_separate_tag(Aad::from(aad_bytes), &mut in_out)
        .map_err(|e| KmsError::internal_error(format!("AES encryption failed: {e}")))?;

    Ok(ciphertext::build_symmetric_blob(
        key_id,
        &iv,
        tag.as_ref(),
        &in_out,
    ))
}

/// Decrypt a symmetric ciphertext blob.
///
/// Returns `(key_id, plaintext)`.
pub fn symmetric_decrypt(
    key_bytes: &[u8],
    blob: &[u8],
    encryption_context: &HashMap<String, String>,
) -> Result<(String, Vec<u8>), KmsError> {
    let (key_id, iv, tag, ct) = ciphertext::parse_symmetric_blob(blob)?;
    let aad_bytes = context_to_aad(encryption_context);

    let unbound_key = aead::UnboundKey::new(&aead::AES_256_GCM, key_bytes)
        .map_err(|e| KmsError::internal_error(format!("Failed to create AES key: {e}")))?;

    let nonce_seq = OneShotNonce(Some({
        let mut arr = [0u8; NONCE_LEN];
        arr.copy_from_slice(iv);
        arr
    }));
    let mut opening_key = aead::OpeningKey::new(unbound_key, nonce_seq);

    // Reconstruct ciphertext + tag for in-place decryption.
    let mut in_out = Vec::with_capacity(ct.len() + tag.len());
    in_out.extend_from_slice(ct);
    in_out.extend_from_slice(tag);

    let plaintext = opening_key
        .open_in_place(Aad::from(aad_bytes), &mut in_out)
        .map_err(|_| {
            KmsError::with_message(
                KmsErrorCode::InvalidCiphertextException,
                "Decryption failed: authentication tag mismatch",
            )
        })?;

    Ok((key_id.to_owned(), plaintext.to_vec()))
}

// ---------------------------------------------------------------------------
// RSA OAEP Encryption/Decryption
// ---------------------------------------------------------------------------

/// Encrypt plaintext using RSA OAEP.
pub fn rsa_oaep_encrypt(
    key_id: &str,
    public_key_der: &[u8],
    plaintext: &[u8],
    algorithm: &EncryptionAlgorithmSpec,
) -> Result<Vec<u8>, KmsError> {
    let oaep_alg = match algorithm {
        EncryptionAlgorithmSpec::RsaesOaepSha1 => &aws_lc_rs::rsa::OAEP_SHA1_MGF1SHA1,
        EncryptionAlgorithmSpec::RsaesOaepSha256 => &aws_lc_rs::rsa::OAEP_SHA256_MGF1SHA256,
        _ => {
            return Err(KmsError::with_message(
                KmsErrorCode::InvalidKeyUsageException,
                format!(
                    "Unsupported RSA encryption algorithm: {}",
                    algorithm.as_str()
                ),
            ));
        }
    };

    let public_key = aws_lc_rs::rsa::PublicEncryptingKey::from_der(public_key_der)
        .map_err(|e| KmsError::internal_error(format!("Failed to load RSA public key: {e}")))?;
    let oaep_key = aws_lc_rs::rsa::OaepPublicEncryptingKey::new(public_key)
        .map_err(|e| KmsError::internal_error(format!("Failed to create OAEP key: {e}")))?;

    let mut output = vec![0u8; oaep_key.ciphertext_size()];
    let ct = oaep_key
        .encrypt(oaep_alg, plaintext, &mut output, None)
        .map_err(|e| KmsError::internal_error(format!("RSA encryption failed: {e}")))?;

    Ok(ciphertext::build_asymmetric_blob(key_id, ct))
}

/// Decrypt ciphertext using RSA OAEP.
pub fn rsa_oaep_decrypt(
    private_key_der: &[u8],
    ct: &[u8],
    algorithm: &EncryptionAlgorithmSpec,
) -> Result<Vec<u8>, KmsError> {
    let oaep_alg = match algorithm {
        EncryptionAlgorithmSpec::RsaesOaepSha1 => &aws_lc_rs::rsa::OAEP_SHA1_MGF1SHA1,
        EncryptionAlgorithmSpec::RsaesOaepSha256 => &aws_lc_rs::rsa::OAEP_SHA256_MGF1SHA256,
        _ => {
            return Err(KmsError::with_message(
                KmsErrorCode::InvalidKeyUsageException,
                format!(
                    "Unsupported RSA decryption algorithm: {}",
                    algorithm.as_str()
                ),
            ));
        }
    };

    let private_key = aws_lc_rs::rsa::PrivateDecryptingKey::from_pkcs8(private_key_der)
        .map_err(|e| KmsError::internal_error(format!("Failed to load RSA private key: {e}")))?;
    let oaep_key = aws_lc_rs::rsa::OaepPrivateDecryptingKey::new(private_key)
        .map_err(|e| KmsError::internal_error(format!("Failed to create OAEP decrypt key: {e}")))?;

    let mut output = vec![0u8; oaep_key.min_output_size()];
    let plaintext = oaep_key
        .decrypt(oaep_alg, ct, &mut output, None)
        .map_err(|_| {
            KmsError::with_message(
                KmsErrorCode::InvalidCiphertextException,
                "RSA OAEP decryption failed",
            )
        })?;

    Ok(plaintext.to_vec())
}

// ---------------------------------------------------------------------------
// RSA Signing/Verification
// ---------------------------------------------------------------------------

/// Sign a message using an RSA key.
pub fn rsa_sign(
    private_key_der: &[u8],
    message: &[u8],
    algorithm: &SigningAlgorithmSpec,
) -> Result<Vec<u8>, KmsError> {
    let padding_alg = rsa_signing_algorithm(algorithm)?;

    let key_pair = RsaKeyPair::from_pkcs8(private_key_der)
        .map_err(|e| KmsError::internal_error(format!("Failed to load RSA key pair: {e}")))?;

    let mut sig = vec![0u8; key_pair.public_modulus_len()];
    key_pair
        .sign(padding_alg, rng(), message, &mut sig)
        .map_err(|e| KmsError::internal_error(format!("RSA signing failed: {e}")))?;

    Ok(sig)
}

/// Verify an RSA signature.
pub fn rsa_verify(
    public_key_der: &[u8],
    message: &[u8],
    sig: &[u8],
    algorithm: &SigningAlgorithmSpec,
) -> Result<bool, KmsError> {
    let verify_alg = rsa_verify_algorithm(algorithm)?;

    let public_key = signature::UnparsedPublicKey::new(verify_alg, public_key_der);
    Ok(public_key.verify(message, sig).is_ok())
}

/// Map signing algorithm to RSA padding algorithm for signing.
fn rsa_signing_algorithm(
    alg: &SigningAlgorithmSpec,
) -> Result<&'static dyn signature::RsaEncoding, KmsError> {
    match alg {
        SigningAlgorithmSpec::RsassaPkcs1V15Sha256 => Ok(&signature::RSA_PKCS1_SHA256),
        SigningAlgorithmSpec::RsassaPkcs1V15Sha384 => Ok(&signature::RSA_PKCS1_SHA384),
        SigningAlgorithmSpec::RsassaPkcs1V15Sha512 => Ok(&signature::RSA_PKCS1_SHA512),
        SigningAlgorithmSpec::RsassaPssSha256 => Ok(&signature::RSA_PSS_SHA256),
        SigningAlgorithmSpec::RsassaPssSha384 => Ok(&signature::RSA_PSS_SHA384),
        SigningAlgorithmSpec::RsassaPssSha512 => Ok(&signature::RSA_PSS_SHA512),
        _ => Err(KmsError::with_message(
            KmsErrorCode::UnsupportedOperationException,
            format!("Unsupported RSA signing algorithm: {}", alg.as_str()),
        )),
    }
}

/// Map signing algorithm to RSA verification algorithm.
fn rsa_verify_algorithm(
    alg: &SigningAlgorithmSpec,
) -> Result<&'static dyn signature::VerificationAlgorithm, KmsError> {
    match alg {
        SigningAlgorithmSpec::RsassaPkcs1V15Sha256 => Ok(&signature::RSA_PKCS1_2048_8192_SHA256),
        SigningAlgorithmSpec::RsassaPkcs1V15Sha384 => Ok(&signature::RSA_PKCS1_2048_8192_SHA384),
        SigningAlgorithmSpec::RsassaPkcs1V15Sha512 => Ok(&signature::RSA_PKCS1_2048_8192_SHA512),
        SigningAlgorithmSpec::RsassaPssSha256 => Ok(&signature::RSA_PSS_2048_8192_SHA256),
        SigningAlgorithmSpec::RsassaPssSha384 => Ok(&signature::RSA_PSS_2048_8192_SHA384),
        SigningAlgorithmSpec::RsassaPssSha512 => Ok(&signature::RSA_PSS_2048_8192_SHA512),
        _ => Err(KmsError::with_message(
            KmsErrorCode::UnsupportedOperationException,
            format!("Unsupported RSA verification algorithm: {}", alg.as_str()),
        )),
    }
}

// ---------------------------------------------------------------------------
// ECDSA Signing/Verification
// ---------------------------------------------------------------------------

/// Sign a message using an ECDSA key.
pub fn ecdsa_sign(
    private_key_der: &[u8],
    message: &[u8],
    algorithm: &SigningAlgorithmSpec,
) -> Result<Vec<u8>, KmsError> {
    let signing_alg = ecdsa_signing_algorithm(algorithm)?;
    let key_pair = EcdsaKeyPair::from_pkcs8(signing_alg, private_key_der)
        .map_err(|e| KmsError::internal_error(format!("Failed to load ECDSA key pair: {e}")))?;
    let sig = key_pair
        .sign(rng(), message)
        .map_err(|e| KmsError::internal_error(format!("ECDSA signing failed: {e}")))?;
    Ok(sig.as_ref().to_vec())
}

/// Verify an ECDSA signature.
pub fn ecdsa_verify(
    public_key_bytes: &[u8],
    message: &[u8],
    sig: &[u8],
    algorithm: &SigningAlgorithmSpec,
) -> Result<bool, KmsError> {
    let verify_alg = ecdsa_verify_algorithm(algorithm)?;
    let public_key = signature::UnparsedPublicKey::new(verify_alg, public_key_bytes);
    Ok(public_key.verify(message, sig).is_ok())
}

/// Map signing algorithm to ECDSA signing algorithm.
fn ecdsa_signing_algorithm(
    alg: &SigningAlgorithmSpec,
) -> Result<&'static signature::EcdsaSigningAlgorithm, KmsError> {
    match alg {
        SigningAlgorithmSpec::EcdsaSha256 => Ok(&signature::ECDSA_P256_SHA256_ASN1_SIGNING),
        SigningAlgorithmSpec::EcdsaSha384 => Ok(&signature::ECDSA_P384_SHA384_ASN1_SIGNING),
        SigningAlgorithmSpec::EcdsaSha512 => Ok(&signature::ECDSA_P521_SHA512_ASN1_SIGNING),
        _ => Err(KmsError::with_message(
            KmsErrorCode::UnsupportedOperationException,
            format!("Unsupported ECDSA signing algorithm: {}", alg.as_str()),
        )),
    }
}

/// Map signing algorithm to ECDSA verification algorithm.
fn ecdsa_verify_algorithm(
    alg: &SigningAlgorithmSpec,
) -> Result<&'static signature::EcdsaVerificationAlgorithm, KmsError> {
    match alg {
        SigningAlgorithmSpec::EcdsaSha256 => Ok(&signature::ECDSA_P256_SHA256_ASN1),
        SigningAlgorithmSpec::EcdsaSha384 => Ok(&signature::ECDSA_P384_SHA384_ASN1),
        SigningAlgorithmSpec::EcdsaSha512 => Ok(&signature::ECDSA_P521_SHA512_ASN1),
        _ => Err(KmsError::with_message(
            KmsErrorCode::UnsupportedOperationException,
            format!("Unsupported ECDSA verification algorithm: {}", alg.as_str()),
        )),
    }
}

// ---------------------------------------------------------------------------
// HMAC
// ---------------------------------------------------------------------------

/// Generate an HMAC tag.
pub fn hmac_generate(
    key_bytes: &[u8],
    message: &[u8],
    algorithm: &MacAlgorithmSpec,
) -> Result<Vec<u8>, KmsError> {
    let hmac_alg = hmac_algorithm(algorithm)?;
    let key = aws_lc_rs::hmac::Key::new(*hmac_alg, key_bytes);
    let tag = aws_lc_rs::hmac::sign(&key, message);
    Ok(tag.as_ref().to_vec())
}

/// Verify an HMAC tag.
pub fn hmac_verify(
    key_bytes: &[u8],
    message: &[u8],
    mac: &[u8],
    algorithm: &MacAlgorithmSpec,
) -> Result<bool, KmsError> {
    let hmac_alg = hmac_algorithm(algorithm)?;
    let key = aws_lc_rs::hmac::Key::new(*hmac_alg, key_bytes);
    Ok(aws_lc_rs::hmac::verify(&key, message, mac).is_ok())
}

/// Map MAC algorithm spec to aws-lc-rs HMAC algorithm.
fn hmac_algorithm(alg: &MacAlgorithmSpec) -> Result<&'static aws_lc_rs::hmac::Algorithm, KmsError> {
    match alg {
        MacAlgorithmSpec::HmacSha224 => Ok(&aws_lc_rs::hmac::HMAC_SHA224),
        MacAlgorithmSpec::HmacSha256 => Ok(&aws_lc_rs::hmac::HMAC_SHA256),
        MacAlgorithmSpec::HmacSha384 => Ok(&aws_lc_rs::hmac::HMAC_SHA384),
        MacAlgorithmSpec::HmacSha512 => Ok(&aws_lc_rs::hmac::HMAC_SHA512),
    }
}

// ---------------------------------------------------------------------------
// Data Key Generation
// ---------------------------------------------------------------------------

/// Generate a data key of the specified spec.
pub fn generate_data_key(spec: &DataKeySpec, num_bytes: Option<i32>) -> Result<Vec<u8>, KmsError> {
    let len = if let Some(n) = num_bytes {
        if !(1..=1024).contains(&n) {
            return Err(KmsError::with_message(
                KmsErrorCode::InvalidArnException,
                "NumberOfBytes must be between 1 and 1024",
            ));
        }
        usize::try_from(n).map_err(|_| {
            KmsError::with_message(
                KmsErrorCode::InvalidArnException,
                "NumberOfBytes must be positive",
            )
        })?
    } else {
        match spec {
            DataKeySpec::Aes128 => 16,
            DataKeySpec::Aes256 => 32,
        }
    };

    generate_random_bytes(len)
}

/// Generate a data key pair for the specified spec.
///
/// Returns `(private_key_der, public_key_der)`.
pub fn generate_data_key_pair(spec: &DataKeyPairSpec) -> Result<(Vec<u8>, Vec<u8>), KmsError> {
    match spec {
        DataKeyPairSpec::Rsa2048 => generate_rsa_pair_raw(2048),
        DataKeyPairSpec::Rsa3072 => generate_rsa_pair_raw(3072),
        DataKeyPairSpec::Rsa4096 => generate_rsa_pair_raw(4096),
        DataKeyPairSpec::EccNistP256 => {
            generate_ec_pair_raw(&signature::ECDSA_P256_SHA256_ASN1_SIGNING)
        }
        DataKeyPairSpec::EccNistP384 => {
            generate_ec_pair_raw(&signature::ECDSA_P384_SHA384_ASN1_SIGNING)
        }
        DataKeyPairSpec::EccNistP521 => {
            generate_ec_pair_raw(&signature::ECDSA_P521_SHA512_ASN1_SIGNING)
        }
        _ => Err(KmsError::with_message(
            KmsErrorCode::UnsupportedOperationException,
            format!("Data key pair spec {} is not supported", spec.as_str()),
        )),
    }
}

fn generate_rsa_pair_raw(bits: usize) -> Result<(Vec<u8>, Vec<u8>), KmsError> {
    let key_size = match bits {
        2048 => aws_lc_rs::rsa::KeySize::Rsa2048,
        3072 => aws_lc_rs::rsa::KeySize::Rsa3072,
        4096 => aws_lc_rs::rsa::KeySize::Rsa4096,
        _ => {
            return Err(KmsError::internal_error(format!(
                "Unsupported RSA data key pair size: {bits}"
            )));
        }
    };

    let key_pair = RsaKeyPair::generate(key_size).map_err(|e| {
        KmsError::internal_error(format!("RSA data key pair generation failed: {e}"))
    })?;

    let private_key_der = key_pair.as_der().map_err(|e| {
        KmsError::internal_error(format!("Failed to serialize RSA private key: {e}"))
    })?;

    let public_key_der = key_pair.public_key().as_der().map_err(|e| {
        KmsError::internal_error(format!("Failed to serialize RSA public key: {e}"))
    })?;

    Ok((
        private_key_der.as_ref().to_vec(),
        public_key_der.as_ref().to_vec(),
    ))
}

fn generate_ec_pair_raw(
    alg: &'static signature::EcdsaSigningAlgorithm,
) -> Result<(Vec<u8>, Vec<u8>), KmsError> {
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(alg, rng()).map_err(|e| {
        KmsError::internal_error(format!("EC data key pair generation failed: {e}"))
    })?;
    let kp = EcdsaKeyPair::from_pkcs8(alg, pkcs8.as_ref()).map_err(|e| {
        KmsError::internal_error(format!("Failed to parse generated EC data key: {e}"))
    })?;
    Ok((pkcs8.as_ref().to_vec(), kp.public_key().as_ref().to_vec()))
}

/// Get the DER-encoded public key from an RSA private key.
pub fn rsa_public_key_der(private_key_der: &[u8]) -> Result<Vec<u8>, KmsError> {
    let key_pair = RsaKeyPair::from_pkcs8(private_key_der)
        .map_err(|e| KmsError::internal_error(format!("Failed to load RSA key pair: {e}")))?;
    Ok(key_pair.public_key().as_ref().to_vec())
}
