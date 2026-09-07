//! S3 response body types supporting buffered and empty modes.
//!
//! This module provides [`S3ResponseBody`], the HTTP response body type used throughout
//! the S3 HTTP service. It supports two modes:
//!
//! - **Buffered**: For small responses such as XML payloads, error bodies, and raw bytes.
//! - **Empty**: For responses with no body content (e.g., 204 No Content, HEAD responses).
//!
//! Streaming support for large objects (e.g., `GetObject`) can be added in the future
//! by extending this enum with a streaming variant.

use std::{
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use http_body_util::Full;

/// S3 response body supporting buffered and empty modes.
///
/// Implements [`http_body::Body`] so it can be used directly with hyper responses.
#[derive(Debug, Default)]
pub enum S3ResponseBody {
    /// Bounded file-backed object response with byte and time budgets.
    Streaming(rustack_core::http::BudgetedBody<FileBody>),
    /// Buffered body for small responses: XML payloads, error bodies, raw bytes.
    Buffered(Full<Bytes>),
    /// Empty body for 204 responses, DELETE confirmations, HEAD responses, etc.
    #[default]
    Empty,
}

impl S3ResponseBody {
    /// Open a validated immutable object range without reading it into memory.
    /// # Errors
    /// Returns file-open/seek or budget validation failures.
    pub async fn from_staged(
        read: rustack_s3_core::storage::StagedRead,
    ) -> Result<Self, std::io::Error> {
        use tokio::io::AsyncSeekExt as _;
        let mut file = read.upload.open().await?;
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            file.seek(std::io::SeekFrom::Start(read.offset)),
        )
        .await??;
        if read
            .offset
            .checked_add(read.length)
            .is_none_or(|end| end > read.upload.size())
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid stored object range",
            ));
        }
        let budget = rustack_core::http::BodyBudget::s3_object();
        let body = FileBody {
            file,
            remaining: read.length,
            upload: read.upload,
        };
        Ok(Self::Streaming(rustack_core::http::BudgetedBody::new(
            body, budget,
        )))
    }

    /// Create a buffered body from bytes.
    #[must_use]
    pub fn from_bytes(data: impl Into<Bytes>) -> Self {
        Self::Buffered(Full::new(data.into()))
    }

    /// Create an empty body.
    #[must_use]
    pub fn empty() -> Self {
        Self::Empty
    }

    /// Create a buffered body from a UTF-8 string.
    #[must_use]
    pub fn from_string(s: impl Into<String>) -> Self {
        Self::Buffered(Full::new(Bytes::from(s.into())))
    }

    /// Create a buffered body from an XML byte vector.
    #[must_use]
    pub fn from_xml(xml: Vec<u8>) -> Self {
        Self::Buffered(Full::new(Bytes::from(xml)))
    }
}

impl http_body::Body for S3ResponseBody {
    type Data = Bytes;
    type Error = std::io::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        match self.get_mut() {
            Self::Streaming(body) => Pin::new(body).poll_frame(cx).map_err(std::io::Error::other),
            Self::Buffered(full) => Pin::new(full)
                .poll_frame(cx)
                .map_err(|never| match never {}),
            Self::Empty => Poll::Ready(None),
        }
    }

    fn is_end_stream(&self) -> bool {
        match self {
            Self::Streaming(body) => body.is_end_stream(),
            Self::Buffered(full) => full.is_end_stream(),
            Self::Empty => true,
        }
    }

    fn size_hint(&self) -> http_body::SizeHint {
        match self {
            Self::Streaming(body) => body.size_hint(),
            Self::Buffered(full) => full.size_hint(),
            Self::Empty => http_body::SizeHint::with_exact(0),
        }
    }
}

/// Bounded file reader retaining the immutable artifact for the response lifetime.
#[derive(Debug)]
pub struct FileBody {
    file: tokio::fs::File,
    remaining: u64,
    upload: std::sync::Arc<rustack_s3_core::storage::StagedUpload>,
}

impl http_body::Body for FileBody {
    type Data = Bytes;
    type Error = std::io::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Bytes>, std::io::Error>>> {
        use tokio::io::AsyncRead as _;
        let this = self.get_mut();
        if this.remaining == 0 {
            return Poll::Ready(None);
        }
        let length = usize::try_from(this.remaining.min(64 * 1024)).map_err(std::io::Error::other);
        let length = match length {
            Ok(length) => length,
            Err(err) => return Poll::Ready(Some(Err(err))),
        };
        let mut buffer = vec![0; length];
        let mut read = tokio::io::ReadBuf::new(&mut buffer);
        match Pin::new(&mut this.file).poll_read(cx, &mut read) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(error)) => Poll::Ready(Some(Err(error))),
            Poll::Ready(Ok(())) => {
                let size = read.filled().len();
                if size == 0 {
                    return Poll::Ready(Some(Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "truncated stored object",
                    ))));
                }
                this.remaining = this.remaining.saturating_sub(size as u64);
                buffer.truncate(size);
                Poll::Ready(Some(Ok(http_body::Frame::data(Bytes::from(buffer)))))
            }
        }
    }

    fn size_hint(&self) -> http_body::SizeHint {
        // Reading the owner also documents why it must remain alive through the last frame.
        http_body::SizeHint::with_exact(self.remaining.min(self.upload.size()))
    }
}

#[cfg(test)]
mod tests {
    use http_body::Body;

    use super::*;

    #[tokio::test]
    async fn test_should_stream_validated_range_in_bounded_frames() {
        use std::sync::Arc;

        use http_body_util::BodyExt;
        use rustack_s3_core::storage::{StagedRead, UploadWriter};
        let data = vec![42; 200_000];
        let mut writer = UploadWriter::new().await.unwrap();
        writer.write(&data).await.unwrap();
        let upload = Arc::new(writer.finish().await.unwrap());
        let path = upload.path().to_owned();
        let mut body = S3ResponseBody::from_staged(StagedRead {
            upload,
            offset: 10,
            length: 150_000,
        })
        .await
        .unwrap();
        let mut count = 0;
        while let Some(frame) = body.frame().await {
            let data = frame.unwrap().into_data().unwrap();
            assert!(data.len() <= 64 * 1024);
            assert!(data.iter().all(|byte| *byte == 42));
            count += data.len();
        }
        assert_eq!(count, 150_000);
        drop(body);
        assert!(!path.exists());
    }

    #[test]
    fn test_should_report_empty_body_as_end_of_stream() {
        let body = S3ResponseBody::empty();
        assert!(body.is_end_stream());
    }

    #[test]
    fn test_should_have_zero_size_for_empty_body() {
        let body = S3ResponseBody::empty();
        let hint = body.size_hint();
        assert_eq!(hint.exact(), Some(0));
    }

    #[test]
    fn test_should_create_buffered_body_from_bytes() {
        let body = S3ResponseBody::from_bytes(Bytes::from("hello"));
        assert!(!body.is_end_stream());
        let hint = body.size_hint();
        assert_eq!(hint.exact(), Some(5));
    }

    #[test]
    fn test_should_create_buffered_body_from_string() {
        let body = S3ResponseBody::from_string("hello world");
        assert!(!body.is_end_stream());
        let hint = body.size_hint();
        assert_eq!(hint.exact(), Some(11));
    }

    #[test]
    fn test_should_create_buffered_body_from_xml() {
        let xml = b"<Root><Key>value</Key></Root>".to_vec();
        let body = S3ResponseBody::from_xml(xml);
        assert!(!body.is_end_stream());
    }

    #[test]
    fn test_should_default_to_empty() {
        let body = S3ResponseBody::default();
        assert!(body.is_end_stream());
    }
}
