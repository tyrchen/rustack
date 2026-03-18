//! CloudWatch Logs HTTP response body type.

use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use http_body_util::Full;

/// Response body for CloudWatch Logs HTTP responses.
///
/// All CloudWatch Logs responses are either JSON (buffered) or empty.
#[derive(Debug, Default)]
pub enum LogsResponseBody {
    /// A fully buffered response body (JSON payload).
    Buffered(Full<Bytes>),
    /// An empty body.
    #[default]
    Empty,
}

impl LogsResponseBody {
    /// Create a response body from raw bytes.
    #[must_use]
    pub fn from_bytes(data: impl Into<Bytes>) -> Self {
        Self::Buffered(Full::new(data.into()))
    }

    /// Create an empty response body.
    #[must_use]
    pub fn empty() -> Self {
        Self::Empty
    }

    /// Create a response body from a JSON-serialized value.
    #[must_use]
    pub fn from_json(json: Vec<u8>) -> Self {
        Self::Buffered(Full::new(Bytes::from(json)))
    }
}

impl http_body::Body for LogsResponseBody {
    type Data = Bytes;
    type Error = std::io::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        match self.get_mut() {
            Self::Buffered(full) => Pin::new(full)
                .poll_frame(cx)
                .map_err(|never| match never {}),
            Self::Empty => Poll::Ready(None),
        }
    }

    fn is_end_stream(&self) -> bool {
        match self {
            Self::Buffered(full) => full.is_end_stream(),
            Self::Empty => true,
        }
    }

    fn size_hint(&self) -> http_body::SizeHint {
        match self {
            Self::Buffered(full) => full.size_hint(),
            Self::Empty => http_body::SizeHint::with_exact(0),
        }
    }
}
