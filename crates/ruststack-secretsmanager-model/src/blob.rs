//! Base64 serde helpers for binary blob fields.
//!
//! AWS Secrets Manager transmits binary data (`SecretBinary`)
//! as base64-encoded strings in JSON. This module provides custom serde
//! serializers/deserializers for `bytes::Bytes` that handle the base64 encoding.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Serialize `bytes::Bytes` as a base64 string.
pub fn serialize<S: Serializer>(bytes: &bytes::Bytes, s: S) -> Result<S::Ok, S::Error> {
    STANDARD.encode(bytes.as_ref()).serialize(s)
}

/// Deserialize a base64 string into `bytes::Bytes`.
pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<bytes::Bytes, D::Error> {
    let s = String::deserialize(d)?;
    STANDARD
        .decode(&s)
        .map(bytes::Bytes::from)
        .map_err(serde::de::Error::custom)
}

/// Serde helpers for `Option<bytes::Bytes>`.
pub mod option {
    use super::{Deserialize, Deserializer, Engine, STANDARD, Serialize, Serializer};

    /// Serialize `Option<bytes::Bytes>` as a base64 string or null.
    pub fn serialize<S: Serializer>(bytes: &Option<bytes::Bytes>, s: S) -> Result<S::Ok, S::Error> {
        match bytes {
            Some(b) => STANDARD.encode(b.as_ref()).serialize(s),
            None => s.serialize_none(),
        }
    }

    /// Deserialize an optional base64 string into `Option<bytes::Bytes>`.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<bytes::Bytes>, D::Error> {
        let opt: Option<String> = Option::deserialize(d)?;
        match opt {
            Some(s) => STANDARD
                .decode(&s)
                .map(|v| Some(bytes::Bytes::from(v)))
                .map_err(serde::de::Error::custom),
            None => Ok(None),
        }
    }
}
