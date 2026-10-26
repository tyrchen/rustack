//! Bounded JSON preflight before typed deserialization.
//!
//! Encoded strings, decoded keys/values, aggregate members and container depth
//! are checked without building a dynamic Value tree. Errors redact input.

use std::{collections::BTreeSet, fmt, num::NonZeroUsize};

use serde::{
    Deserializer,
    de::{DeserializeOwned, DeserializeSeed, Error, MapAccess, SeqAccess, Visitor},
};

/// Redacted JSON validation and target-schema failure classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum JsonError {
    /// Invalid parser limits.
    #[error("invalid JSON limits")]
    InvalidLimits,
    /// Input exceeds its byte allowance.
    #[error("JSON byte limit exceeded")]
    Bytes,
    /// Nested containers exceed their allowance.
    #[error("JSON depth limit exceeded")]
    Depth,
    /// Aggregate object pairs/array elements exceed their allowance.
    #[error("JSON member limit exceeded")]
    Members,
    /// An encoded/decoded scalar string exceeds its allowance.
    #[error("JSON string limit exceeded")]
    String,
    /// A decoded object key exceeds its separate allowance.
    #[error("JSON key limit exceeded")]
    Key,
    /// Two decoded keys in one object are equal.
    #[error("duplicate JSON key")]
    Duplicate,
    /// Invalid JSON, including non-finite numbers.
    #[error("invalid JSON syntax")]
    Syntax,
    /// Bounded JSON does not match the requested domain type.
    #[error("invalid JSON schema")]
    Schema,
}

/// Validated byte, container-depth, member and string/key limits.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct JsonLimits {
    bytes: NonZeroUsize,
    depth: NonZeroUsize,
    members: NonZeroUsize,
    string_bytes: NonZeroUsize,
    key_bytes: NonZeroUsize,
}

impl JsonLimits {
    /// Validate limits before accepting untrusted bytes.
    ///
    /// # Errors
    /// Rejects zero limits, bytes over 16 MiB, depth over 64, members over 65,536,
    /// strings over 1 MiB. Strings cannot exceed the body limit; keys cannot
    /// exceed strings. Consumers choose their own tighter key allowance.
    ///
    /// ```
    /// let limits = rustack_core::json::JsonLimits::new(1024, 8, 64, 256, 64)?;
    /// # Ok::<(), rustack_core::json::JsonError>(())
    /// ```
    pub fn new(
        bytes: usize,
        depth: usize,
        members: usize,
        string_bytes: usize,
        key_bytes: usize,
    ) -> Result<Self, JsonError> {
        if bytes > 16 * 1024 * 1024
            || depth > 64
            || members > 65_536
            || string_bytes > 1024 * 1024
            || string_bytes > bytes
            || key_bytes > string_bytes
        {
            return Err(JsonError::InvalidLimits);
        }
        let nonzero = |value| NonZeroUsize::new(value).ok_or(JsonError::InvalidLimits);
        Ok(Self {
            bytes: nonzero(bytes)?,
            depth: nonzero(depth)?,
            members: nonzero(members)?,
            string_bytes: nonzero(string_bytes)?,
            key_bytes: nonzero(key_bytes)?,
        })
    }
}

/// Preflight bounded structure and decoded-key uniqueness without a value tree.
///
/// Depth counts containers: an object/array of scalars has depth one. Members
/// count all object pairs and array elements across the entire input.
///
/// # Errors
/// Returns a redacted syntax, duplicate or resource-limit class.
///
/// ```
/// use rustack_core::json::{JsonLimits, validate};
/// validate(br#"{"items":[1,true,null]}"#, JsonLimits::new(1024, 8, 64, 256, 64)?)?;
/// # Ok::<(), rustack_core::json::JsonError>(())
/// ```
pub fn validate(input: &[u8], limits: JsonLimits) -> Result<(), JsonError> {
    if input.len() > limits.bytes.get() {
        return Err(JsonError::Bytes);
    }
    encoded_strings(input, limits.string_bytes.get())?;
    let mut context = Scan {
        limits,
        remaining: limits.members.get(),
        failure: None,
    };
    let mut decoder = serde_json::Deserializer::from_slice(input);
    let result = Node {
        context: &mut context,
        depth: 0,
    }
    .deserialize(&mut decoder);
    if result.is_err() {
        return Err(context.failure.unwrap_or(JsonError::Syntax));
    }
    decoder.end().map_err(|_| JsonError::Syntax)
}

/// Preflight JSON before materializing a typed model; domain validation follows.
///
/// # Errors
/// Returns preflight classes or Schema without a deserializer's input text.
///
/// ```
/// use std::collections::BTreeMap;
/// use rustack_core::json::{JsonLimits, from_slice};
/// let model: BTreeMap<String, u64> = from_slice(br#"{"count":7}"#,
///     JsonLimits::new(1024, 8, 64, 256, 64)?)?;
/// assert_eq!(model.get("count"), Some(&7));
/// # Ok::<(), rustack_core::json::JsonError>(())
/// ```
pub fn from_slice<T: DeserializeOwned>(input: &[u8], limits: JsonLimits) -> Result<T, JsonError> {
    validate(input, limits)?;
    serde_json::from_slice(input).map_err(|_| JsonError::Schema)
}

fn encoded_strings(input: &[u8], decoded_limit: usize) -> Result<(), JsonError> {
    // A JSON escape uses at most six encoded bytes per decoded UTF-8 byte.
    // Cap the parser's scratch space before processing escaped strings.
    let maximum = decoded_limit
        .checked_mul(6)
        .ok_or(JsonError::InvalidLimits)?;
    let mut inside = false;
    let mut escaped = false;
    let mut length = 0_usize;
    for byte in input {
        if !inside {
            if *byte == b'"' {
                inside = true;
                length = 0;
            }
        } else if !escaped && *byte == b'"' {
            inside = false;
        } else {
            length = length.checked_add(1).ok_or(JsonError::String)?;
            if length > maximum {
                return Err(JsonError::String);
            }
            escaped = !escaped && *byte == b'\\';
        }
    }
    Ok(())
}

#[derive(Debug)]
struct Scan {
    limits: JsonLimits,
    remaining: usize,
    failure: Option<JsonError>,
}

impl Scan {
    fn reject<E: Error>(&mut self, error: JsonError) -> E {
        self.failure.get_or_insert(error);
        E::custom("invalid bounded JSON")
    }
    fn member<E: Error>(&mut self) -> Result<(), E> {
        match self.remaining.checked_sub(1) {
            Some(remaining) => {
                self.remaining = remaining;
                Ok(())
            }
            None => Err(self.reject(JsonError::Members)),
        }
    }
}

#[derive(Debug)]
struct Node<'a> {
    context: &'a mut Scan,
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for Node<'_> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, decoder: D) -> Result<(), D::Error> {
        decoder.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Node<'_> {
    type Value = ();
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("bounded JSON")
    }
    fn visit_bool<E: Error>(self, _: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_i64<E: Error>(self, _: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E: Error>(self, _: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E: Error>(self, value: f64) -> Result<(), E> {
        if value.is_finite() {
            Ok(())
        } else {
            Err(self.context.reject(JsonError::Syntax))
        }
    }
    fn visit_unit<E: Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_str<E: Error>(self, value: &str) -> Result<(), E> {
        if value.len() > self.context.limits.string_bytes.get() {
            return Err(self.context.reject(JsonError::String));
        }
        Ok(())
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut array: A) -> Result<(), A::Error> {
        if self.depth >= self.context.limits.depth.get() {
            return Err(self.context.reject(JsonError::Depth));
        }
        let depth = self
            .depth
            .checked_add(1)
            .ok_or_else(|| self.context.reject(JsonError::Depth))?;
        while array
            .next_element_seed(Element {
                context: self.context,
                depth,
            })?
            .is_some()
        {}
        Ok(())
    }
    fn visit_map<A: MapAccess<'de>>(self, mut object: A) -> Result<(), A::Error> {
        if self.depth >= self.context.limits.depth.get() {
            return Err(self.context.reject(JsonError::Depth));
        }
        let depth = self
            .depth
            .checked_add(1)
            .ok_or_else(|| self.context.reject(JsonError::Depth))?;
        let mut keys = BTreeSet::new();
        while let Some(key) = object.next_key::<String>()? {
            self.context.member()?;
            if key.len() > self.context.limits.key_bytes.get() {
                return Err(self.context.reject(JsonError::Key));
            }
            if !keys.insert(key) {
                return Err(self.context.reject(JsonError::Duplicate));
            }
            object.next_value_seed(Node {
                context: self.context,
                depth,
            })?;
        }
        Ok(())
    }
}

// Charge only actual elements, so an empty array remains valid after an
// enclosing object consumes the last available member.
#[derive(Debug)]
struct Element<'a> {
    context: &'a mut Scan,
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for Element<'_> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, decoder: D) -> Result<(), D::Error> {
        self.context.member()?;
        Node {
            context: self.context,
            depth: self.depth,
        }
        .deserialize(decoder)
    }
}

#[cfg(test)]
mod tests;
