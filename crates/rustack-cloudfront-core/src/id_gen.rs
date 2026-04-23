//! Resource ID and ETag generators.
//!
//! CloudFront resource IDs are 14-character uppercase alphanumeric strings.
//! Distributions, OACs, cache policies, and most other resources start with
//! `E`; invalidations start with `I`.

use std::hash::{Hash, Hasher};

use rand::RngExt;

const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// Length of a CloudFront resource ID excluding the type-prefix letter.
const ID_BODY_LEN: usize = 13;

/// Generate a new 14-character distribution ID starting with `E`.
#[must_use]
pub fn new_distribution_id() -> String {
    new_id_with_prefix('E')
}

/// Generate a new 14-character invalidation ID starting with `I`.
#[must_use]
pub fn new_invalidation_id() -> String {
    new_id_with_prefix('I')
}

/// Generate a new 14-character ID with an arbitrary type-prefix letter.
#[must_use]
pub fn new_id_with_prefix(prefix: char) -> String {
    let mut rng = rand::rng();
    let mut buf = String::with_capacity(ID_BODY_LEN + 1);
    buf.push(prefix);
    for _ in 0..ID_BODY_LEN {
        let idx = rng.random_range(0..ALPHABET.len());
        buf.push(ALPHABET[idx] as char);
    }
    buf
}

/// Deterministically derive a 14-character ID from a seed string.
///
/// When the server is started with `CLOUDFRONT_DETERMINISTIC_IDS=true`, this
/// is used so snapshot tests can assert stable IDs across runs.
#[must_use]
pub fn deterministic_id_with_prefix(prefix: char, seed: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    seed.hash(&mut hasher);
    let mut value = hasher.finish();
    let mut buf = String::with_capacity(ID_BODY_LEN + 1);
    buf.push(prefix);
    for _ in 0..ID_BODY_LEN {
        let idx = (value % ALPHABET.len() as u64) as usize;
        buf.push(ALPHABET[idx] as char);
        value /= ALPHABET.len() as u64;
        // Re-mix to avoid collapsing to a single character once value is small.
        value = value
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
    }
    buf
}

/// Generate an opaque ETag token.
///
/// We use base32-style identifiers for visual consistency with AWS. The value
/// is never interpreted — callers treat it as an opaque string.
#[must_use]
pub fn new_etag() -> String {
    new_id_with_prefix('E')
}

/// Produce the `d{lowercased-id}.{suffix}` FQDN for a distribution.
#[must_use]
pub fn distribution_domain_name(distribution_id: &str, domain_suffix: &str) -> String {
    format!("{}.{}", distribution_id.to_ascii_lowercase(), domain_suffix)
}

/// Shape of a CloudFront resource ID: 14 chars, uppercase alnum, optional prefix.
#[must_use]
pub fn is_valid_resource_id(id: &str, expected_prefix: Option<char>) -> bool {
    if id.len() != ID_BODY_LEN + 1 {
        return false;
    }
    let mut iter = id.chars();
    let first = match iter.next() {
        Some(c) => c,
        None => return false,
    };
    if !first.is_ascii_uppercase() {
        return false;
    }
    if let Some(prefix) = expected_prefix {
        if first != prefix {
            return false;
        }
    }
    iter.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

/// S3 canonical user ID used for OAI responses. Matches AWS format: 64 hex chars.
#[must_use]
pub fn new_s3_canonical_user_id() -> String {
    let mut rng = rand::rng();
    const HEX: &[u8] = b"0123456789abcdef";
    let mut buf = String::with_capacity(64);
    for _ in 0..64 {
        buf.push(HEX[rng.random_range(0..HEX.len())] as char);
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_generate_14_char_id() {
        let id = new_distribution_id();
        assert_eq!(id.len(), 14);
        assert!(id.starts_with('E'));
        assert!(is_valid_resource_id(&id, Some('E')));
    }

    #[test]
    fn test_should_generate_deterministic_id() {
        let a = deterministic_id_with_prefix('E', "hello");
        let b = deterministic_id_with_prefix('E', "hello");
        assert_eq!(a, b);
        assert_ne!(a, deterministic_id_with_prefix('E', "world"));
    }

    #[test]
    fn test_should_derive_domain_name() {
        let name = distribution_domain_name("E1ABCDEF123456", "cloudfront.net");
        assert_eq!(name, "e1abcdef123456.cloudfront.net");
    }

    #[test]
    fn test_should_validate_resource_id_shape() {
        assert!(is_valid_resource_id("E1ABCDEF123456", Some('E')));
        assert!(!is_valid_resource_id("E1ABCDEF123456", Some('I')));
        assert!(!is_valid_resource_id("e1abcdef123456", Some('E')));
        assert!(!is_valid_resource_id("E1", Some('E')));
    }
}
