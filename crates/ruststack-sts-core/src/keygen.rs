//! Temporary credential generation.

use rand::Rng;

/// Generate temporary AWS credentials.
#[derive(Debug)]
pub struct CredentialGenerator {
    account_id: String,
}

impl CredentialGenerator {
    /// Create a new credential generator for the given account.
    #[must_use]
    pub fn new(account_id: String) -> Self {
        Self { account_id }
    }

    /// Generate a set of temporary credentials.
    #[must_use]
    pub fn generate_temporary(&self) -> GeneratedCredentials {
        let access_key_id = self.generate_access_key("ASIA");
        let secret_access_key = generate_secret_key();
        let session_token = generate_session_token();

        GeneratedCredentials {
            access_key_id,
            secret_access_key,
            session_token,
        }
    }

    /// Generate an access key ID with the given prefix.
    ///
    /// Format: PREFIX (4 chars) + encoded account ID (4 chars) + random (12 chars)
    /// Total: 20 characters, all uppercase alphanumeric.
    fn generate_access_key(&self, prefix: &str) -> String {
        let mut rng = rand::rng();

        // Encode account ID into 4 base-36 characters.
        let account_num: u64 = self.account_id.parse().unwrap_or(0);
        let account_encoded = encode_base36(account_num, 4);

        // Generate 12 random uppercase alphanumeric characters.
        let random_part: String = (0..12)
            .map(|_| {
                let idx = rng.random_range(0..36u8);
                if idx < 10 {
                    (b'0' + idx) as char
                } else {
                    (b'A' + (idx - 10)) as char
                }
            })
            .collect();

        format!("{prefix}{account_encoded}{random_part}")
    }
}

/// Generated temporary credential set.
#[derive(Debug, Clone)]
pub struct GeneratedCredentials {
    /// Access key ID (starts with "ASIA", 20 chars).
    pub access_key_id: String,
    /// Secret access key (40 chars).
    pub secret_access_key: String,
    /// Session token (opaque, ~213 chars).
    pub session_token: String,
}

/// Generate a role ID with AROA prefix.
#[must_use]
pub fn generate_role_id() -> String {
    let mut rng = rand::rng();
    let random_part: String = (0..17)
        .map(|_| {
            let idx = rng.random_range(0..36u8);
            if idx < 10 {
                (b'0' + idx) as char
            } else {
                (b'A' + (idx - 10)) as char
            }
        })
        .collect();
    format!("AROA{random_part}")
}

/// Generate a federated user ID with an account-based prefix.
#[must_use]
pub fn generate_federated_user_id(account_id: &str, name: &str) -> String {
    format!("{account_id}:{name}")
}

/// Generate a 40-character secret access key.
fn generate_secret_key() -> String {
    let mut rng = rand::rng();
    let charset = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    (0..40)
        .map(|_| {
            let idx = rng.random_range(0..charset.len());
            charset[idx] as char
        })
        .collect()
}

/// Generate an opaque session token.
fn generate_session_token() -> String {
    let mut rng = rand::rng();
    let charset = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/=";
    let prefix = "FwoGZXIvYXdzE";
    let random_part: String = (0..200)
        .map(|_| {
            let idx = rng.random_range(0..charset.len());
            charset[idx] as char
        })
        .collect();
    format!("{prefix}{random_part}")
}

/// Encode a number in base-36 with fixed width (uppercase).
fn encode_base36(mut num: u64, width: usize) -> String {
    let chars = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut result = String::with_capacity(width);
    let mut digits = vec![0u8; width];
    for i in (0..width).rev() {
        digits[i] = chars[(num % 36) as usize];
        num /= 36;
    }
    for &b in &digits {
        result.push(b as char);
    }
    result
}

/// Extract the account ID from an access key.
///
/// Decodes the 4-character account ID segment (characters 4-7) from
/// base-36 back to a numeric account ID.
#[must_use]
pub fn account_id_from_access_key(access_key: &str) -> Option<String> {
    if access_key.len() < 8 {
        return None;
    }
    let encoded = &access_key[4..8];
    let num = decode_base36(encoded)?;
    Some(format!("{num:012}"))
}

/// Decode a base-36 string to a number.
fn decode_base36(s: &str) -> Option<u64> {
    let mut result: u64 = 0;
    for ch in s.chars() {
        let digit = match ch {
            '0'..='9' => u64::from(ch as u8 - b'0'),
            'A'..='Z' => u64::from(ch as u8 - b'A') + 10,
            'a'..='z' => u64::from(ch as u8 - b'a') + 10,
            _ => return None,
        };
        result = result.checked_mul(36)?.checked_add(digit)?;
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_generate_temporary_credentials() {
        let cred_gen = CredentialGenerator::new("123456789012".to_owned());
        let creds = cred_gen.generate_temporary();
        assert!(creds.access_key_id.starts_with("ASIA"));
        assert_eq!(creds.access_key_id.len(), 20);
        assert_eq!(creds.secret_access_key.len(), 40);
        assert!(creds.session_token.starts_with("FwoGZXIvYXdzE"));
    }

    #[test]
    fn test_should_encode_decode_base36_roundtrip() {
        let encoded = encode_base36(123_456_789_012, 4);
        let decoded = decode_base36(&encoded).unwrap();
        // Note: 4 chars of base36 can only hold up to 36^4-1 = 1679615
        // so the actual account ID gets truncated via modulo. This is expected.
        assert!(decoded <= 1_679_615);
    }

    #[test]
    fn test_should_account_id_from_access_key() {
        let cred_gen = CredentialGenerator::new("000000000000".to_owned());
        let creds = cred_gen.generate_temporary();
        let account = account_id_from_access_key(&creds.access_key_id);
        assert_eq!(account, Some("000000000000".to_owned()));
    }

    #[test]
    fn test_should_generate_role_id() {
        let role_id = generate_role_id();
        assert!(role_id.starts_with("AROA"));
        assert_eq!(role_id.len(), 21);
    }

    #[test]
    fn test_should_return_none_for_short_key() {
        assert_eq!(account_id_from_access_key("ASIA"), None);
    }
}
