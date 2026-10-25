//! Bounded control framing for the isolated Docker feasibility experiment.
//!
//! This tool verifies transport feasibility with trusted fixture peers. It is not
//! the production Runtime API or a hardened external CBOR deserializer.
#![forbid(unsafe_code)]

use std::{
    fmt,
    io::{self, Cursor},
    num::TryFromIntError,
};

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Maximum frame in this feasibility experiment.
pub const MAX_FRAME: usize = 6 * 1024 * 1024 + 64 * 1024;

/// Framing failures without payload-bearing diagnostics.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum FrameError {
    /// Encoding the envelope failed.
    #[error("control envelope encoding failed")]
    Encode(#[from] ciborium::ser::Error<io::Error>),
    /// Decoding the envelope failed.
    #[error("control envelope decoding failed")]
    Decode(#[from] ciborium::de::Error<io::Error>),
    /// Reading or writing the transport failed.
    #[error("control transport IO failed")]
    Io(#[from] io::Error),
    /// A length does not fit the target integer type.
    #[error("control length conversion failed")]
    Length(#[from] TryFromIntError),
    /// A frame exceeds the allocation budget.
    #[error("oversized control frame")]
    TooLarge,
    /// More than one CBOR item was present.
    #[error("trailing control data")]
    TrailingData,
    /// The control version is unsupported.
    #[error("unsupported control protocol")]
    UnsupportedVersion,
}

/// Fixed message schema for experiment-owned peers only.
#[derive(Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Frame {
    /// Protocol version.
    pub version: u32,
    /// Environment identity.
    pub environment: String,
    /// Message variant.
    pub kind: String,
    /// Invocation identity, empty outside invocation messages.
    pub request_id: String,
    /// Raw event/result bytes, omitted from Debug.
    pub payload: Bytes,
}

impl Frame {
    /// Build a control message from trusted fixture metadata.
    ///
    /// ```
    /// use bytes::Bytes;
    /// use rustack_workflows_spikes::Frame;
    /// let frame = Frame::new("fixture", "ready", "", Bytes::new());
    /// assert_eq!(frame.version, 1);
    /// ```
    #[must_use]
    pub fn new(environment: &str, kind: &str, request_id: &str, payload: Bytes) -> Self {
        Self {
            version: 1,
            environment: environment.into(),
            kind: kind.into(),
            request_id: request_id.into(),
            payload,
        }
    }
}

/// Encode a complete length-prefixed frame.
///
/// # Errors
/// Returns an encoding or length error, or rejects an oversized frame.
///
/// ```
/// use bytes::Bytes;
/// use rustack_workflows_spikes::{Frame, encode};
/// # fn main() -> anyhow::Result<()> {
/// let bytes = encode(&Frame::new("fixture", "ready", "", Bytes::new()))?;
/// assert!(bytes.len() > 4);
/// # Ok(()) }
/// ```
pub fn encode(frame: &Frame) -> Result<Vec<u8>, FrameError> {
    let mut body = Vec::new();
    ciborium::into_writer(frame, &mut body)?;
    if body.len() > MAX_FRAME {
        return Err(FrameError::TooLarge);
    }
    let length = u32::try_from(body.len())?.to_be_bytes();
    let capacity = body.len().checked_add(4).ok_or(FrameError::TooLarge)?;
    let mut out = Vec::with_capacity(capacity);
    out.extend_from_slice(&length);
    out.extend_from_slice(&body);
    Ok(out)
}

/// Decode one already bounded message body from a trusted fixture peer.
///
/// # Errors
/// Rejects oversized, malformed, trailing or unsupported-version data.
///
/// ```
/// use rustack_workflows_spikes::decode;
/// assert!(decode(&[]).is_err());
/// ```
pub fn decode(body: &[u8]) -> Result<Frame, FrameError> {
    if body.len() > MAX_FRAME {
        return Err(FrameError::TooLarge);
    }
    let mut cursor = Cursor::new(body);
    let frame: Frame = ciborium::from_reader(&mut cursor)?;
    if usize::try_from(cursor.position())? != body.len() {
        return Err(FrameError::TrailingData);
    }
    if frame.version != 1 {
        return Err(FrameError::UnsupportedVersion);
    }
    Ok(frame)
}

/// Read a bounded frame from a dedicated reader task.
///
/// # Errors
/// Propagates transport, decoding and length failures.
///
/// ```no_run
/// # async fn example() -> anyhow::Result<()> {
/// let mut stdin = tokio::io::stdin();
/// let _frame = rustack_workflows_spikes::read_frame(&mut stdin).await?;
/// # Ok(()) }
/// ```
pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Frame, FrameError> {
    let length = usize::try_from(reader.read_u32().await?)?;
    if length > MAX_FRAME {
        return Err(FrameError::TooLarge);
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).await?;
    decode(&body)
}

/// Write and flush a complete frame.
///
/// # Errors
/// Propagates encoding and transport failures.
///
/// ```no_run
/// # async fn example(frame: rustack_workflows_spikes::Frame) -> anyhow::Result<()> {
/// let mut stdout = tokio::io::stdout();
/// rustack_workflows_spikes::write_frame(&mut stdout, &frame).await?;
/// # Ok(()) }
/// ```
pub async fn write_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    frame: &Frame,
) -> Result<(), FrameError> {
    writer.write_all(&encode(frame)?).await?;
    writer.flush().await?;
    Ok(())
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Frame")
            .field("version", &self.version)
            .field("environment", &self.environment)
            .field("kind", &self.kind)
            .field("request_id", &self.request_id)
            .field("payload_bytes", &self.payload.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;

    use anyhow::Result;
    use bytes::Bytes;
    use tokio::io::AsyncWriteExt;

    use super::{Frame, FrameError, MAX_FRAME, decode, encode, read_frame};

    #[tokio::test]
    async fn test_should_round_trip_fragmented_binary_frame() -> Result<()> {
        let payload = Bytes::from((0_u8..=255).collect::<Vec<_>>());
        let frame = Frame::new(
            "test-environment",
            "invoke",
            "test-attempt",
            payload.clone(),
        );
        let encoded = encode(&frame)?;
        let (mut writer, mut reader) = tokio::io::duplex(16);
        let task = tokio::spawn(async move {
            for chunk in encoded.chunks(3) {
                writer.write_all(chunk).await?;
            }
            Ok::<(), std::io::Error>(())
        });
        let decoded = read_frame(&mut reader).await?;
        task.await??;
        assert_eq!(decoded.payload, payload);
        Ok(())
    }

    #[test]
    fn test_should_reject_trailing_cbor_data() -> Result<()> {
        let mut body = Vec::new();
        ciborium::into_writer(&Frame::new("env", "ready", "", Bytes::new()), &mut body)?;
        body.push(0);
        assert!(decode(&body).is_err());
        Ok(())
    }

    #[tokio::test]
    async fn test_should_reject_oversized_prefix_before_body_read() -> Result<()> {
        let prefix = u32::try_from(MAX_FRAME + 1)?.to_be_bytes();
        let mut reader = prefix.as_slice();
        assert!(read_frame(&mut reader).await.is_err());
        Ok(())
    }

    #[test]
    fn test_should_redact_payload_in_debug() {
        let debug = format!(
            "{:?}",
            Frame::new(
                "env",
                "invoke",
                "attempt",
                Bytes::from_static(b"secret-marker")
            )
        );
        assert!(!debug.contains("secret-marker"));
        assert!(debug.contains("payload_bytes"));
    }
    #[tokio::test]
    async fn test_should_detect_clean_eof() {
        let mut input = b"".as_slice();
        assert!(
            matches!(read_frame(&mut input).await, Err(FrameError::Io(error)) if error.kind() == ErrorKind::UnexpectedEof)
        );
    }

    #[tokio::test]
    async fn test_should_detect_truncated_prefix_eof() {
        let mut input = b"\0\0\0".as_slice();
        assert!(
            matches!(read_frame(&mut input).await, Err(FrameError::Io(error)) if error.kind() == ErrorKind::UnexpectedEof)
        );
    }

    #[tokio::test]
    async fn test_should_detect_truncated_body_eof() {
        let mut input = b"\0\0\0\x02\xa0".as_slice();
        assert!(
            matches!(read_frame(&mut input).await, Err(FrameError::Io(error)) if error.kind() == ErrorKind::UnexpectedEof)
        );
    }
}
