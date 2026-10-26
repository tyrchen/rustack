//! Fully encoded response bytes with redacted diagnostics.

use std::{
    convert::Infallible,
    fmt,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use http_body::{Body, Frame, SizeHint};
use http_body_util::Full;

/// Bounded buffered response; Debug does not print its workflow data.
#[non_exhaustive]
pub struct StepFunctionsBody {
    inner: Full<Bytes>,
}
impl fmt::Debug for StepFunctionsBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StepFunctionsBody(<redacted>)")
    }
}
impl StepFunctionsBody {
    pub(crate) fn new(bytes: Bytes) -> Self {
        Self {
            inner: Full::new(bytes),
        }
    }
}
impl Body for StepFunctionsBody {
    type Data = Bytes;
    type Error = Infallible;
    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        Pin::new(&mut self.get_mut().inner).poll_frame(cx)
    }
    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }
}

/// Ordinary JSON requests have no trailer semantics; never silently discard them.
pub(crate) struct RequestBody<B> {
    inner: Pin<Box<B>>,
    finished: bool,
}
impl<B> RequestBody<B> {
    pub(crate) fn new(inner: B) -> Self {
        Self {
            inner: Box::pin(inner),
            finished: false,
        }
    }
}
impl<B> fmt::Debug for RequestBody<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RequestBody(<redacted>)")
    }
}
#[derive(thiserror::Error)]
pub(crate) enum InputError {
    #[error("request trailer frames are unsupported")]
    Trailers,
    #[error("request source failed")]
    Source(#[source] Box<dyn std::error::Error + Send + Sync>),
}
impl fmt::Debug for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl<B> Body for RequestBody<B>
where
    B: Body<Data = Bytes>,
    B::Error: std::error::Error + Send + Sync + 'static,
{
    type Data = Bytes;
    type Error = InputError;
    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, InputError>>> {
        let this = self.get_mut();
        if this.finished {
            return Poll::Ready(None);
        }
        match this.inner.as_mut().poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) if !frame.is_data() => {
                this.finished = true;
                Poll::Ready(Some(Err(InputError::Trailers)))
            }
            Poll::Ready(Some(Err(error))) => {
                this.finished = true;
                Poll::Ready(Some(Err(InputError::Source(Box::new(error)))))
            }
            Poll::Ready(None) => {
                this.finished = true;
                Poll::Ready(None)
            }
            Poll::Ready(Some(Ok(frame))) => Poll::Ready(Some(Ok(frame))),
            Poll::Pending => Poll::Pending,
        }
    }
    fn is_end_stream(&self) -> bool {
        self.finished || self.inner.is_end_stream()
    }
    fn size_hint(&self) -> SizeHint {
        if self.finished {
            SizeHint::with_exact(0)
        } else {
            self.inner.size_hint()
        }
    }
}
