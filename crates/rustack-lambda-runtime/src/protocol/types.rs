//! Validated metadata and closed control-message variants.

use std::{fmt, num::NonZeroU64, str::FromStr};

use bytes::Bytes;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use uuid::Uuid;

use super::{MAX_PAYLOAD_BYTES, ProtocolError};

macro_rules! identity {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(Uuid);

        impl Default for $name {
            fn default() -> Self {
                Self(Uuid::new_v4())
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
        impl FromStr for $name {
            type Err = ProtocolError;
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                if value.len() != 36 {
                    return Err(ProtocolError::InvalidMetadata);
                }
                let id = Uuid::parse_str(value).map_err(|_| ProtocolError::InvalidMetadata)?;
                if id.is_nil() || id.to_string() != value {
                    return Err(ProtocolError::InvalidMetadata);
                }
                Ok(Self(id))
            }
        }
        impl TryFrom<String> for $name {
            type Error = ProtocolError;
            fn try_from(value: String) -> Result<Self, Self::Error> {
                value.parse()
            }
        }
        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.to_string()
            }
        }
    };
}

identity!(
    EnvironmentId,
    "Canonical nonnil environment UUID; parse with `FromStr`, generate with `Default`."
);
identity!(
    AttemptId,
    "Canonical nonnil invocation attempt UUID; parse with `FromStr`, generate with `Default`."
);

/// Architecture advertised by the environment-local relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Architecture {
    /// Linux x86-64.
    #[serde(rename = "x86_64")]
    X86_64,
    /// Linux `AArch64`.
    #[serde(rename = "arm64")]
    Arm64,
}

/// Validated Lambda invocation ARN, including an optional version/alias.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct FunctionArn(String);

impl FromStr for FunctionArn {
    type Err = ProtocolError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() > 2048 {
            return Err(ProtocolError::InvalidMetadata);
        }
        let mut parts = value.split(':');
        let valid = matches!(parts.next(), Some("arn"))
            && matches!(parts.next(), Some("aws" | "aws-cn" | "aws-us-gov"))
            && matches!(parts.next(), Some("lambda"))
            && parts
                .next()
                .is_some_and(|region| identifier(region, 64, false))
            && parts.next().is_some_and(|account| {
                account.len() == 12 && account.bytes().all(|b| b.is_ascii_digit())
            })
            && matches!(parts.next(), Some("function"))
            && parts.next().is_some_and(|name| identifier(name, 64, true));
        if !valid {
            return Err(ProtocolError::InvalidMetadata);
        }
        if let Some(qualifier) = parts.next() {
            if qualifier != "$LATEST" && !identifier(qualifier, 128, true) {
                return Err(ProtocolError::InvalidMetadata);
            }
            if qualifier.bytes().all(|byte| byte.is_ascii_digit())
                && !qualifier.parse::<u64>().is_ok_and(|version| version != 0)
            {
                return Err(ProtocolError::InvalidMetadata);
            }
        }
        if parts.next().is_some() {
            return Err(ProtocolError::InvalidMetadata);
        }
        Ok(Self(value.into()))
    }
}

fn identifier(value: &str, maximum: usize, underscore: bool) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || (underscore && b == b'_'))
}

impl TryFrom<String> for FunctionArn {
    type Error = ProtocolError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}
impl From<FunctionArn> for String {
    fn from(value: FunctionArn) -> Self {
        value.0
    }
}
impl AsRef<str> for FunctionArn {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Nonempty printable ASCII trace metadata, at most256 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TraceId(String);

impl FromStr for TraceId {
    type Err = ProtocolError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty() || value.len() > 256 || !value.bytes().all(|b| (32..=126).contains(&b))
        {
            return Err(ProtocolError::InvalidMetadata);
        }
        Ok(Self(value.into()))
    }
}
impl TryFrom<String> for TraceId {
    type Error = ProtocolError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}
impl From<TraceId> for String {
    fn from(value: TraceId) -> Self {
        value.0
    }
}
impl AsRef<str> for TraceId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// A size-validated raw event/result whose Debug never displays contents.
#[derive(Clone, PartialEq, Eq)]
pub struct Payload(Bytes);

impl TryFrom<Bytes> for Payload {
    type Error = ProtocolError;
    fn try_from(value: Bytes) -> Result<Self, Self::Error> {
        if value.len() > MAX_PAYLOAD_BYTES {
            return Err(ProtocolError::TooLarge);
        }
        Ok(Self(value))
    }
}
impl From<Payload> for Bytes {
    fn from(value: Payload) -> Self {
        value.0
    }
}
impl AsRef<[u8]> for Payload {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}
impl fmt::Debug for Payload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Payload")
            .field("bytes", &self.0.len())
            .finish_non_exhaustive()
    }
}
impl Serialize for Payload {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&self.0)
    }
}
impl<'de> Deserialize<'de> for Payload {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct PayloadVisitor;
        impl de::Visitor<'_> for PayloadVisitor {
            type Value = Payload;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a bounded CBOR byte string")
            }
            fn visit_bytes<E: de::Error>(self, bytes: &[u8]) -> Result<Payload, E> {
                if bytes.len() > MAX_PAYLOAD_BYTES {
                    return Err(E::custom("payload size budget exceeded"));
                }
                Ok(Payload(Bytes::copy_from_slice(bytes)))
            }
            fn visit_byte_buf<E: de::Error>(self, bytes: Vec<u8>) -> Result<Payload, E> {
                Payload::try_from(Bytes::from(bytes)).map_err(E::custom)
            }
        }
        deserializer.deserialize_byte_buf(PayloadVisitor)
    }
}

/// Acknowledgement scope; cancellation always names exactly one attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "AckFields", into = "AckFields")]
pub enum Acknowledgement {
    /// The session invalidated this attempt; user-code exit is not proven.
    Cancel(AttemptId),
    /// The control session accepted closure.
    Close,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum AckOperation {
    Cancel,
    Close,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AckFields {
    operation: AckOperation,
    #[serde(skip_serializing_if = "Option::is_none")]
    attempt: Option<AttemptId>,
}
impl TryFrom<AckFields> for Acknowledgement {
    type Error = ProtocolError;
    fn try_from(value: AckFields) -> Result<Self, Self::Error> {
        match (value.operation, value.attempt) {
            (AckOperation::Cancel, Some(attempt)) => Ok(Self::Cancel(attempt)),
            (AckOperation::Close, None) => Ok(Self::Close),
            _ => Err(ProtocolError::InvalidMetadata),
        }
    }
}
impl From<Acknowledgement> for AckFields {
    fn from(value: Acknowledgement) -> Self {
        match value {
            Acknowledgement::Cancel(attempt) => Self {
                operation: AckOperation::Cancel,
                attempt: Some(attempt),
            },
            Acknowledgement::Close => Self {
                operation: AckOperation::Close,
                attempt: None,
            },
        }
    }
}

/// Whether the function posted a success response or a handler error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    /// Runtime /response.
    Success,
    /// Runtime /error, distinct from a transport or initialization failure.
    FunctionError,
}

/// Closed control schema; payload and invocation metadata are validated types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
#[non_exhaustive]
pub enum Message {
    /// First relay frame advertising the compiled platform.
    Hello {
        /// Linux image architecture.
        architecture: Architecture,
    },
    /// Relay listener is bound; does not prove bootstrap readiness.
    Ready,
    /// The function issued its first valid GET /next.
    BootstrapReady,
    /// One host-owned invocation; never replay automatically.
    Invoke {
        /// Shared caller/backend attempt UUID.
        attempt: AttemptId,
        /// Absolute wall deadline, checked and converted by the receiver.
        deadline_unix_ms: NonZeroU64,
        /// Requested ARN, preserving an invoked alias.
        function_arn: FunctionArn,
        /// Runtime trace header.
        trace_id: TraceId,
        /// Raw event bytes.
        payload: Payload,
    },
    /// Function completion for one delivered attempt.
    InvocationResult {
        /// Completed attempt UUID.
        attempt: AttemptId,
        /// /response versus /error.
        outcome: Outcome,
        /// Raw result or error bytes.
        payload: Payload,
    },
    /// Fatal initialization failure before event delivery.
    InitError {
        /// Raw initialization error document.
        payload: Payload,
    },
    /// Invalidate an attempt; the host still terminates its function.
    Cancel {
        /// Invalidated attempt UUID.
        attempt: AttemptId,
    },
    /// Close the environment's sole control session.
    Close,
    /// Cancellation or closure acknowledgement, scoped by operation.
    Ack(Acknowledgement),
}
