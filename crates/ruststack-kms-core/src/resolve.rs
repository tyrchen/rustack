//! Key ID resolution logic.
//!
//! KMS accepts key identifiers in several formats:
//! - UUID: `550e8400-e29b-41d4-a716-446655440000`
//! - ARN: `arn:aws:kms:us-east-1:000000000000:key/550e8400-...`
//! - Alias name: `alias/my-key`
//! - Alias ARN: `arn:aws:kms:us-east-1:000000000000:alias/my-key`

use ruststack_kms_model::error::{KmsError, KmsErrorCode};

use crate::state::KmsStore;

/// Resolve a key identifier to a key ID (UUID).
///
/// Supports UUID, ARN, alias name, and alias ARN formats.
pub fn resolve_key_id(store: &KmsStore, key_ref: &str) -> Result<String, KmsError> {
    // 1. Alias name (starts with "alias/").
    if key_ref.starts_with("alias/") {
        return resolve_alias(store, key_ref);
    }

    // 2. Alias ARN (contains ":alias/").
    if key_ref.contains(":alias/") {
        let alias_name = key_ref.rsplit_once(':').map_or(key_ref, |(_, rest)| rest);
        return resolve_alias(store, alias_name);
    }

    // 3. Key ARN (contains ":key/").
    if key_ref.contains(":key/") {
        let key_id = key_ref.rsplit('/').next().ok_or_else(|| {
            KmsError::with_message(
                KmsErrorCode::InvalidArnException,
                format!("Invalid key ARN: {key_ref}"),
            )
        })?;
        return Ok(key_id.to_owned());
    }

    // 4. Bare UUID.
    Ok(key_ref.to_owned())
}

/// Resolve an alias name to the target key ID.
fn resolve_alias(store: &KmsStore, alias_name: &str) -> Result<String, KmsError> {
    store
        .get_alias(alias_name)
        .map(|a| a.target_key_id)
        .ok_or_else(|| {
            KmsError::with_message(
                KmsErrorCode::NotFoundException,
                format!("Alias {alias_name} is not found."),
            )
        })
}
