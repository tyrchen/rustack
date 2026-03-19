//! Base64 serde helpers for binary blob fields.
//!
//! AWS Kinesis transmits binary data (record data) as base64-encoded strings
//! in JSON. This module provides custom serde serializers/deserializers for
//! `bytes::Bytes` that handle the base64 encoding.

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
