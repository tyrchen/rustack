//! Shard iterator token encoding and decoding.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

/// A shard iterator token encoding the stream name, shard ID, position, and nonce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardIteratorToken {
    /// The stream name.
    pub stream_name: String,
    /// The shard ID.
    pub shard_id: String,
    /// The logical position in the shard record log.
    pub position: usize,
    /// A random nonce for uniqueness.
    pub nonce: String,
}

impl ShardIteratorToken {
    /// Encode the token as a base64 string.
    #[must_use]
    pub fn encode(&self) -> String {
        let raw = format!(
            "{}:{}:{}:{}",
            self.stream_name, self.shard_id, self.position, self.nonce
        );
        STANDARD.encode(raw.as_bytes())
    }

    /// Decode a base64-encoded iterator token string.
    pub fn decode(encoded: &str) -> Result<Self, anyhow::Error> {
        let decoded_bytes = STANDARD.decode(encoded)?;
        let decoded = String::from_utf8(decoded_bytes)?;
        let parts: Vec<&str> = decoded.splitn(4, ':').collect();
        if parts.len() != 4 {
            anyhow::bail!(
                "invalid shard iterator token: expected 4 parts, got {}",
                parts.len()
            );
        }
        let position: usize = parts[2].parse()?;
        Ok(Self {
            stream_name: parts[0].to_owned(),
            shard_id: parts[1].to_owned(),
            position,
            nonce: parts[3].to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_roundtrip_encode_decode() {
        let token = ShardIteratorToken {
            stream_name: "my-stream".to_owned(),
            shard_id: "shardId-000000000000".to_owned(),
            position: 42,
            nonce: "abc123".to_owned(),
        };
        let encoded = token.encode();
        let decoded = ShardIteratorToken::decode(&encoded).unwrap();
        assert_eq!(token, decoded);
    }

    #[test]
    fn test_should_fail_on_invalid_base64() {
        assert!(ShardIteratorToken::decode("not-valid-base64!!!").is_err());
    }

    #[test]
    fn test_should_fail_on_wrong_part_count() {
        let raw = STANDARD.encode(b"only:two");
        assert!(ShardIteratorToken::decode(&raw).is_err());
    }

    #[test]
    fn test_should_handle_colons_in_nonce() {
        let token = ShardIteratorToken {
            stream_name: "stream".to_owned(),
            shard_id: "shard".to_owned(),
            position: 0,
            nonce: "nonce:with:colons".to_owned(),
        };
        let encoded = token.encode();
        let decoded = ShardIteratorToken::decode(&encoded).unwrap();
        assert_eq!(token, decoded);
    }
}
