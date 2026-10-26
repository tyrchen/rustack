//! Length-prefixed, bounded CBOR for one environment's attach connection.
//!
//! A partial read/write failure invalidates the connection. Callers own the IO
//! task and must retire that transport rather than retrying a partially read
//! frame or replaying an invocation.

mod preflight;
#[cfg(test)]
mod tests;
mod types;

use std::{io, num::TryFromIntError};

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    time::{Instant, timeout_at},
};
pub use types::{
    Acknowledgement, Architecture, AttemptId, EnvironmentId, FunctionArn, HandlerTimeout, Message,
    Outcome, Payload, TraceId,
};

/// Maximum event or result payload, in bytes.
pub const MAX_PAYLOAD_BYTES: usize = 6 * 1024 * 1024;
/// Maximum CBOR envelope metadata, excluding raw payload bytes.
pub const MAX_METADATA_BYTES: usize = 64 * 1024;
/// Maximum frame body, excluding its four-byte prefix.
pub const MAX_FRAME_BYTES: usize = MAX_PAYLOAD_BYTES + MAX_METADATA_BYTES;
/// Host/relay contract version; older relays cannot enforce delivery-based timeouts.
pub const PROTOCOL_VERSION: u32 = 2;

/// Protocol failures contain classes and IO sources, never invalid peer values.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ProtocolError {
    /// A declared/allotted length exceeds a protocol budget.
    #[error("control protocol size budget exceeded")]
    TooLarge,
    /// The byte structure is invalid, unsupported, duplicated or truncated.
    #[error("invalid control CBOR structure")]
    InvalidStructure,
    /// The valid CBOR does not match the typed message schema.
    #[error("invalid control message schema")]
    InvalidSchema,
    /// An incompatible host/relay version was supplied.
    #[error("unsupported control protocol version")]
    UnsupportedVersion,
    /// A validated primitive has invalid syntax or range.
    #[error("invalid control metadata")]
    InvalidMetadata,
    /// IO failed, including truncated transport input.
    #[error("control transport IO failed")]
    Io(#[from] io::Error),
    /// An absolute IO deadline expired; the transport must be retired.
    #[error("control transport deadline exceeded")]
    Deadline,
}

impl From<TryFromIntError> for ProtocolError {
    fn from(_: TryFromIntError) -> Self {
        Self::TooLarge
    }
}

/// A validated, environment-scoped control envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "RawFrame")]
#[non_exhaustive]
pub struct Frame {
    version: u32,
    environment: EnvironmentId,
    message: Message,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawFrame {
    version: u32,
    environment: EnvironmentId,
    message: Message,
}

impl TryFrom<RawFrame> for Frame {
    type Error = ProtocolError;

    fn try_from(raw: RawFrame) -> Result<Self, Self::Error> {
        if raw.version != PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedVersion);
        }
        Ok(Self::new(raw.environment, raw.message))
    }
}

impl Frame {
    /// Wrap a typed message in the current protocol version.
    ///
    /// ```
    /// use rustack_lambda_runtime::protocol::{EnvironmentId, Frame, Message};
    /// let frame = Frame::new(EnvironmentId::default(), Message::Ready);
    /// assert_eq!(frame.message(), &Message::Ready);
    /// ```
    #[must_use]
    pub fn new(environment: EnvironmentId, message: Message) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            environment,
            message,
        }
    }

    /// Identity that must match the connection's owned environment.
    ///
    /// ```
    /// # use rustack_lambda_runtime::protocol::{EnvironmentId, Frame, Message};
    /// let id = EnvironmentId::default();
    /// assert_eq!(Frame::new(id, Message::Close).environment(), id);
    /// ```
    #[must_use]
    pub fn environment(&self) -> EnvironmentId {
        self.environment
    }

    /// Borrow the typed message without cloning its payload.
    ///
    /// ```
    /// # use rustack_lambda_runtime::protocol::{EnvironmentId, Frame, Message};
    /// let frame = Frame::new(EnvironmentId::default(), Message::Close);
    /// assert!(matches!(frame.message(), Message::Close));
    /// ```
    #[must_use]
    pub fn message(&self) -> &Message {
        &self.message
    }

    /// Move the message into the owning session or transport actor.
    ///
    /// ```
    /// # use rustack_lambda_runtime::protocol::{EnvironmentId, Frame, Message};
    /// let frame = Frame::new(EnvironmentId::default(), Message::Close);
    /// assert_eq!(frame.into_message(), Message::Close);
    /// ```
    #[must_use]
    pub fn into_message(self) -> Message {
        self.message
    }
}

/// Encode one complete frame, including the big-endian length prefix.
///
/// # Errors
/// Rejects excessive metadata or an encoding/size failure.
///
/// ```
/// # use rustack_lambda_runtime::protocol::{EnvironmentId, Frame, Message, encode};
/// let bytes = encode(&Frame::new(EnvironmentId::default(), Message::Ready))?;
/// assert!(bytes.len() > 4);
/// # Ok::<(), rustack_lambda_runtime::protocol::ProtocolError>(())
/// ```
pub fn encode(frame: &Frame) -> Result<Bytes, ProtocolError> {
    let mut output = vec![0; 4];
    ciborium::into_writer(frame, &mut output).map_err(|_| ProtocolError::InvalidSchema)?;
    let body = output.get(4..).ok_or(ProtocolError::InvalidStructure)?;
    preflight::validate(body)?;
    let length = u32::try_from(body.len())?.to_be_bytes();
    output
        .get_mut(..4)
        .ok_or(ProtocolError::InvalidStructure)?
        .copy_from_slice(&length);
    Ok(Bytes::from(output))
}

/// Decode one bounded frame body, without its length prefix.
///
/// # Errors
/// Rejects malformed CBOR, unsupported schema/version or invalid metadata.
/// No untrusted text is included in the returned error.
///
/// ```
/// use rustack_lambda_runtime::protocol::decode;
/// assert!(decode(&[]).is_err());
/// ```
pub fn decode(body: &[u8]) -> Result<Frame, ProtocolError> {
    preflight::validate(body)?;
    let raw: RawFrame = ciborium::from_reader(body).map_err(|_| ProtocolError::InvalidSchema)?;
    Frame::try_from(raw)
}

/// Read a frame under one absolute deadline; None denotes clean boundary EOF.
///
/// # Errors
/// Returns IO/truncation, deadline or protocol validation errors. A failed or
/// cancelled read invalidates the connection, including any consumed prefix.
///
/// ```no_run
/// # async fn read(mut input: impl tokio::io::AsyncRead + Unpin) -> Result<(), rustack_lambda_runtime::protocol::ProtocolError> {
/// let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
/// let frame = rustack_lambda_runtime::protocol::read_frame(&mut input, deadline).await?;
/// # Ok(()) }
/// ```
pub async fn read_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
    deadline: Instant,
) -> Result<Option<Frame>, ProtocolError> {
    if Instant::now() >= deadline {
        return Err(ProtocolError::Deadline);
    }
    let frame = timeout_at(deadline, async {
        let mut prefix = [0; 4];
        let count = reader.read(&mut prefix).await?;
        if count == 0 {
            return Ok(None);
        }
        reader
            .read_exact(
                prefix
                    .get_mut(count..)
                    .ok_or(ProtocolError::InvalidStructure)?,
            )
            .await?;
        let length = usize::try_from(u32::from_be_bytes(prefix))?;
        if length > MAX_FRAME_BYTES {
            return Err(ProtocolError::TooLarge);
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).await?;
        decode(&body).map(Some)
    })
    .await
    .map_err(|_| ProtocolError::Deadline)??;
    if Instant::now() >= deadline {
        return Err(ProtocolError::Deadline);
    }
    Ok(frame)
}

/// Encode, write and flush a frame under one absolute deadline.
///
/// # Errors
/// Returns encoding, IO or deadline failure. Partial writes require retiring
/// the connection; retrying a business invocation is never implicit.
///
/// ```no_run
/// # async fn write(mut output: impl tokio::io::AsyncWrite + Unpin, frame: rustack_lambda_runtime::protocol::Frame) -> Result<(), rustack_lambda_runtime::protocol::ProtocolError> {
/// let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
/// rustack_lambda_runtime::protocol::write_frame(&mut output, &frame, deadline).await?;
/// # Ok(()) }
/// ```
pub async fn write_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    frame: &Frame,
    deadline: Instant,
) -> Result<(), ProtocolError> {
    if Instant::now() >= deadline {
        return Err(ProtocolError::Deadline);
    }
    let encoded = encode(frame)?;
    if Instant::now() >= deadline {
        return Err(ProtocolError::Deadline);
    }
    timeout_at(deadline, async {
        writer.write_all(&encoded).await?;
        writer.flush().await
    })
    .await
    .map_err(|_| ProtocolError::Deadline)??;
    if Instant::now() >= deadline {
        return Err(ProtocolError::Deadline);
    }
    Ok(())
}
