//! Validated immutable JSON and bounded output materialization.

use std::{fmt, io, sync::Arc};

use bytes::Bytes;
use rustack_core::json::{JsonError, JsonLimits, from_slice};
use secrecy::zeroize::Zeroize;
use serde::Serialize;
use serde_json::{Value, json};

use crate::{AslError, CallbackToken};

/// Maximum original or materialized UTF-8 JSON payload size.
pub const MAX_PAYLOAD_BYTES: usize = 262_144;
const MAX_DEPTH: usize = 64;
const MAX_MEMBERS: usize = 65_536;

const PAYLOAD_LIMITS: TreeLimits = TreeLimits {
    bytes: MAX_PAYLOAD_BYTES,
    depth: MAX_DEPTH,
    members: MAX_MEMBERS,
};

const CONTEXT_LIMITS: TreeLimits = TreeLimits {
    bytes: 4 * 1024 * 1024,
    depth: 68,
    members: 2 * MAX_MEMBERS + 512,
};

#[derive(Clone, Copy)]
struct TreeLimits {
    bytes: usize,
    depth: usize,
    members: usize,
}

/// Immutable bounded JSON, retaining original bytes for execution idempotency.
#[derive(Clone)]
#[non_exhaustive]
pub struct Payload {
    value: Arc<Value>,
    bytes: Bytes,
}

impl fmt::Debug for Payload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Payload(<redacted>)")
    }
}

impl Payload {
    /// Validate JSON before constructing an immutable parsed tree.
    ///
    /// # Errors
    /// Rejects duplicate keys, invalid JSON and byte/depth/member/string limits.
    ///
    /// ```
    /// use rustack_stepfunctions_core::Payload;
    /// let input = Payload::parse(br#" {"n":null} "#)?;
    /// assert_eq!(input.original_bytes(), br#" {"n":null} "#);
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    pub fn parse(input: &[u8]) -> Result<Self, AslError> {
        if input.len() > MAX_PAYLOAD_BYTES {
            return Err(JsonError::Bytes.into());
        }
        Self::from_bytes(Bytes::copy_from_slice(input))
    }

    /// Validate a constructed tree before bounded compact serialization.
    ///
    /// # Errors
    /// Rejects excessive structure, strings or serialized byte size.
    ///
    /// ```
    /// use rustack_stepfunctions_core::Payload;
    /// let output = Payload::from_value(&serde_json::json!({"ok": true}))?;
    /// assert_eq!(output.value()["ok"], true);
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    pub fn from_value(value: &Value) -> Result<Self, AslError> {
        validate_value(value, PAYLOAD_LIMITS)?;
        let bytes = encode(value)?;
        Ok(Self {
            value: Arc::new(value.clone()),
            bytes,
        })
    }

    /// Inspect the immutable parsed value; mutations require a new validation.
    ///
    /// ```
    /// let input = rustack_stepfunctions_core::Payload::parse(b"null")?;
    /// assert!(input.value().is_null());
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    #[must_use]
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Read the exact source bytes, including whitespace when parsed from text.
    ///
    /// ```
    /// let input = rustack_stepfunctions_core::Payload::parse(b" 1 ")?;
    /// assert_eq!(input.original_bytes(), b" 1 ");
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    #[must_use]
    pub fn original_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn from_bytes(bytes: Bytes) -> Result<Self, AslError> {
        let limits = JsonLimits::new(
            MAX_PAYLOAD_BYTES,
            MAX_DEPTH,
            MAX_MEMBERS,
            MAX_PAYLOAD_BYTES,
            MAX_PAYLOAD_BYTES,
        )?;
        let value = from_slice(&bytes, limits)?;
        Ok(Self {
            value: Arc::new(value),
            bytes,
        })
    }
}

/// Immutable internal ASL context with headroom for embedded business payloads.
///
/// Context is bounded to4MiB,68 container levels and131,584 members. Selected
/// output still passes through the independent256KiB/depth64 payload boundary.
#[derive(Clone)]
#[non_exhaustive]
pub struct EvaluationContext {
    value: ContextStorage,
}
#[derive(Clone)]
enum ContextStorage {
    Plain(Arc<Value>),
    Sensitive(Arc<SensitiveContext>),
}
struct SensitiveContext {
    value: Value,
}
impl fmt::Debug for ContextStorage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ContextStorage(<redacted>)")
    }
}
impl fmt::Debug for SensitiveContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SensitiveContext(<redacted>)")
    }
}
impl SensitiveContext {
    fn clear_token(&mut self) {
        if let Some(Value::String(token)) = self
            .value
            .get_mut("Task")
            .and_then(|task| task.get_mut("Token"))
        {
            token.zeroize();
        }
    }
}
impl Drop for SensitiveContext {
    fn drop(&mut self) {
        self.clear_token();
    }
}

impl fmt::Debug for EvaluationContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EvaluationContext(<redacted>)")
    }
}

impl From<&Payload> for EvaluationContext {
    fn from(payload: &Payload) -> Self {
        Self {
            value: ContextStorage::Plain(payload.value.clone()),
        }
    }
}

impl EvaluationContext {
    /// Validate an internal context before cloning its tree.
    ///
    /// Borrowing prevents recursive destruction of rejected caller-owned trees.
    /// The typed engine owns schema/scope; callback preparation owns scoped Task.Token.
    ///
    /// # Errors
    /// Rejects the internal context's independent byte/depth/member/string limits.
    ///
    /// ```
    /// use rustack_stepfunctions_core::EvaluationContext;
    /// let context = EvaluationContext::from_value(&serde_json::json!({
    ///     "Execution": {"Input": {"message":"hello"}}
    /// }))?;
    /// assert!(context.value().get("Execution").is_some());
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    pub fn from_value(value: &Value) -> Result<Self, AslError> {
        validate_value(value, CONTEXT_LIMITS)?;
        encode_with_limit(value, CONTEXT_LIMITS.bytes)?;
        Ok(Self {
            value: ContextStorage::Plain(Arc::new(value.clone())),
        })
    }

    /// Inspect the immutable context; Debug remains redacted.
    ///
    /// ```
    /// use rustack_stepfunctions_core::{EvaluationContext, Payload};
    /// let input = Payload::parse(b"{}")?;
    /// assert!(EvaluationContext::from(&input).value().is_object());
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    #[must_use]
    pub fn value(&self) -> &Value {
        match &self.value {
            ContextStorage::Plain(value) => value,
            ContextStorage::Sensitive(value) => &value.value,
        }
    }

    pub(crate) fn for_callback(&self, token: &CallbackToken) -> Result<Self, AslError> {
        let fields = self
            .value()
            .as_object()
            .ok_or(AslError::InvalidCallbackContext)?;
        if fields.contains_key("Task") {
            return Err(AslError::InvalidCallbackContext);
        }
        // Measure only the nonsensitive base. The token is validated ASCII hex;
        // this fixed envelope needs no escaping and is never serialized for size.
        let extra = b"\"Task\":{\"Token\":\"\"}"
            .len()
            .checked_add(token.expose().len())
            .and_then(|size| size.checked_add(usize::from(!fields.is_empty())))
            .ok_or(AslError::DataLimit)?;
        let size = encode_with_limit(self.value(), CONTEXT_LIMITS.bytes)?.len();
        if size.checked_add(extra).ok_or(AslError::DataLimit)? > CONTEXT_LIMITS.bytes {
            return Err(AslError::DataLimit);
        }
        let mut value = fields.clone();
        value.insert("Task".into(), json!({"Token":token.expose()}));
        let sensitive = SensitiveContext {
            value: Value::Object(value),
        };
        validate_value(&sensitive.value, CONTEXT_LIMITS)?;
        Ok(Self {
            value: ContextStorage::Sensitive(Arc::new(sensitive)),
        })
    }
}

fn validate_value(value: &Value, limits: TreeLimits) -> Result<(), AslError> {
    let mut stack = vec![(value, 0_usize)];
    let mut remaining = limits.members;
    while let Some((value, depth)) = stack.pop() {
        let children = match value {
            Value::Array(values) => {
                check_container(depth, values.len(), &mut remaining, limits.depth)?;
                stack.extend(values.iter().map(|value| (value, depth.saturating_add(1))));
                continue;
            }
            Value::Object(values) => {
                check_container(depth, values.len(), &mut remaining, limits.depth)?;
                for key in values.keys() {
                    check_string(key)?;
                }
                values.values()
            }
            Value::String(value) => {
                check_string(value)?;
                continue;
            }
            _ => continue,
        };
        stack.extend(children.map(|value| (value, depth.saturating_add(1))));
    }
    Ok(())
}

pub(crate) fn validate_component(value: &Value) -> Result<(), AslError> {
    validate_value(
        value,
        TreeLimits {
            bytes: 1024 * 1024,
            depth: 64,
            members: 4096,
        },
    )?;
    encode_with_limit(value, 1024 * 1024)?;
    Ok(())
}

fn check_container(
    depth: usize,
    length: usize,
    remaining: &mut usize,
    max_depth: usize,
) -> Result<(), AslError> {
    if depth >= max_depth {
        return Err(JsonError::Depth.into());
    }
    *remaining = remaining.checked_sub(length).ok_or(JsonError::Members)?;
    Ok(())
}

fn check_string(value: &str) -> Result<(), AslError> {
    if value.len() > MAX_PAYLOAD_BYTES {
        Err(JsonError::String.into())
    } else {
        Ok(())
    }
}

pub(crate) fn encode<T: Serialize + ?Sized>(value: &T) -> Result<Bytes, AslError> {
    encode_with_limit(value, MAX_PAYLOAD_BYTES)
}

pub(crate) fn encode_with_limit<T: Serialize + ?Sized>(
    value: &T,
    limit: usize,
) -> Result<Bytes, AslError> {
    let mut writer = LimitedWriter {
        bytes: Vec::new(),
        full: false,
        limit,
    };
    if let Err(error) = serde_json::to_writer(&mut writer, value) {
        return if writer.full {
            Err(AslError::DataLimit)
        } else {
            Err(AslError::Encoding(error))
        };
    }
    Ok(Bytes::from(writer.bytes))
}

#[cfg(test)]
mod tests;

struct LimitedWriter {
    bytes: Vec<u8>,
    full: bool,
    limit: usize,
}

impl io::Write for LimitedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let needed = self.bytes.len().saturating_add(bytes.len());
        if needed > self.limit {
            self.full = true;
            return Err(io::Error::other("bounded ASL output exhausted"));
        }
        if needed > self.bytes.capacity() {
            let capacity = needed
                .max(self.bytes.capacity().saturating_mul(2))
                .clamp(64, self.limit);
            self.bytes
                .try_reserve_exact(capacity.saturating_sub(self.bytes.len()))
                .map_err(|_| io::Error::other("bounded ASL allocation failed"))?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn parse_component(input: &[u8]) -> Result<Value, AslError> {
    let limits = JsonLimits::new(1024 * 1024, 64, 4096, 262_144, 262_144)?;
    from_slice(input, limits).map_err(AslError::Json)
}
