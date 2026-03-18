//! Ciphertext blob serialization format for symmetric encryption.
//!
//! Format: `[KeyID 36 bytes (UUID)] [IV 12 bytes] [Auth Tag 16 bytes] [Ciphertext N bytes]`
//!
//! For asymmetric encryption the ciphertext is just the raw encrypted bytes
//! prefixed with the key ID.

use ruststack_kms_model::error::{KmsError, KmsErrorCode};

/// Size of a UUID string (e.g., "550e8400-e29b-41d4-a716-446655440000").
const KEY_ID_LEN: usize = 36;
/// Size of AES-GCM nonce/IV.
pub const IV_LEN: usize = 12;
/// Size of AES-GCM authentication tag.
pub const TAG_LEN: usize = 16;
/// Minimum ciphertext blob size for symmetric encryption (key ID + IV + tag).
const MIN_SYMMETRIC_LEN: usize = KEY_ID_LEN + IV_LEN + TAG_LEN;

/// Build a symmetric ciphertext blob.
///
/// Format: `[key_id (36 bytes)] [iv (12 bytes)] [tag (16 bytes)] [ciphertext]`
pub fn build_symmetric_blob(key_id: &str, iv: &[u8], tag: &[u8], ciphertext: &[u8]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(KEY_ID_LEN + IV_LEN + TAG_LEN + ciphertext.len());
    blob.extend_from_slice(key_id.as_bytes());
    blob.extend_from_slice(iv);
    blob.extend_from_slice(tag);
    blob.extend_from_slice(ciphertext);
    blob
}

/// Parse a symmetric ciphertext blob into its components.
///
/// Returns `(key_id, iv, tag, ciphertext)`.
#[allow(clippy::type_complexity)]
pub fn parse_symmetric_blob(blob: &[u8]) -> Result<(&str, &[u8], &[u8], &[u8]), KmsError> {
    if blob.len() < MIN_SYMMETRIC_LEN {
        return Err(KmsError::with_message(
            KmsErrorCode::InvalidCiphertextException,
            "Ciphertext blob is too short",
        ));
    }

    let key_id_bytes = &blob[..KEY_ID_LEN];
    let key_id = std::str::from_utf8(key_id_bytes).map_err(|_| {
        KmsError::with_message(
            KmsErrorCode::InvalidCiphertextException,
            "Invalid key ID in ciphertext blob",
        )
    })?;

    let iv = &blob[KEY_ID_LEN..KEY_ID_LEN + IV_LEN];
    let tag = &blob[KEY_ID_LEN + IV_LEN..KEY_ID_LEN + IV_LEN + TAG_LEN];
    let ciphertext = &blob[KEY_ID_LEN + IV_LEN + TAG_LEN..];

    Ok((key_id, iv, tag, ciphertext))
}

/// Build an asymmetric ciphertext blob (key ID prefix + raw ciphertext).
pub fn build_asymmetric_blob(key_id: &str, ciphertext: &[u8]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(KEY_ID_LEN + ciphertext.len());
    blob.extend_from_slice(key_id.as_bytes());
    blob.extend_from_slice(ciphertext);
    blob
}

/// Parse an asymmetric ciphertext blob.
///
/// Returns `(key_id, ciphertext)`.
pub fn parse_asymmetric_blob(blob: &[u8]) -> Result<(&str, &[u8]), KmsError> {
    if blob.len() <= KEY_ID_LEN {
        return Err(KmsError::with_message(
            KmsErrorCode::InvalidCiphertextException,
            "Ciphertext blob is too short for asymmetric decryption",
        ));
    }

    let key_id = std::str::from_utf8(&blob[..KEY_ID_LEN]).map_err(|_| {
        KmsError::with_message(
            KmsErrorCode::InvalidCiphertextException,
            "Invalid key ID in ciphertext blob",
        )
    })?;

    Ok((key_id, &blob[KEY_ID_LEN..]))
}
