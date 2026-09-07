//! Source-level HTTP byte budgets and progress/absolute deadlines.
//!
//! Limits apply before frames reach parsers or collectors, independent of Content-Length.

use std::{
    error::Error,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use bytes::{Buf, Bytes};
use http_body::{Body, Frame, SizeHint};
use http_body_util::BodyExt;
use tokio::time::{Instant, Sleep, sleep_until};

/// Default maximum aggregate control-plane request size.
pub const CONTROL_BODY_LIMIT: u64 = 16 * 1024 * 1024;
/// Maximum Lambda code-upload JSON envelope.
pub const LAMBDA_CODE_BODY_LIMIT: u64 = 96 * 1024 * 1024;
/// Default maximum upstream response size.
pub const UPSTREAM_BODY_LIMIT: u64 = 64 * 1024 * 1024;
/// Maximum S3 object stream size (not an aggregate allocation budget).
pub const S3_OBJECT_BODY_LIMIT: u64 = 5 * 1024 * 1024 * 1024;

/// Validated byte and time budget for one HTTP body.
#[derive(Debug, Clone, Copy)]
pub struct BodyBudget {
    max_bytes: u64,
    total: Duration,
    idle: Duration,
}

impl BodyBudget {
    /// Build a nonzero, bounded body budget.
    ///
    /// # Errors
    /// Rejects zero values, sizes above 5 GiB, or deadlines above one hour.
    pub fn new(max_bytes: u64, total: Duration, idle: Duration) -> Result<Self, BodyReadError> {
        if max_bytes == 0
            || max_bytes > S3_OBJECT_BODY_LIMIT
            || total.is_zero()
            || idle.is_zero()
            || total > Duration::from_hours(1)
            || idle > total
        {
            return Err(BodyReadError::InvalidBudget);
        }
        Ok(Self {
            max_bytes,
            total,
            idle,
        })
    }

    /// Default control-plane limits: 16 MiB, 30 seconds total, 5 seconds idle.
    #[must_use]
    pub fn control() -> Self {
        let settings = crate::settings::budgets();
        Self {
            max_bytes: settings.control_body_bytes,
            total: Duration::from_secs(settings.body_total_seconds),
            idle: Duration::from_secs(settings.body_idle_seconds),
        }
    }

    /// Code-upload limits: 96 MiB with the control-plane deadlines.
    #[must_use]
    pub fn lambda_code() -> Self {
        Self {
            max_bytes: crate::settings::budgets().lambda_code_body_bytes,
            ..Self::control()
        }
    }

    /// Configured streaming object budget, with independent decoded-byte enforcement.
    #[must_use]
    pub fn s3_object() -> Self {
        let settings = crate::settings::budgets();
        Self {
            max_bytes: settings.s3_object_body_bytes,
            total: Duration::from_secs(settings.s3_body_total_seconds),
            idle: Duration::from_secs(settings.body_idle_seconds),
        }
    }

    /// Encoded S3 input budget including bounded chunk/trailer overhead.
    #[must_use]
    pub fn s3_encoded() -> Self {
        let budget = Self::s3_object();
        Self {
            max_bytes: budget
                .max_bytes
                .saturating_mul(2)
                .saturating_add(CONTROL_BODY_LIMIT),
            ..budget
        }
    }

    /// Tighten the byte limit without relaxing either deadline.
    #[must_use]
    pub fn capped(self, max_bytes: std::num::NonZeroU64) -> Self {
        Self {
            max_bytes: self.max_bytes.min(max_bytes.get()),
            ..self
        }
    }

    /// Maximum accepted data bytes.
    #[must_use]
    pub const fn max_bytes(self) -> u64 {
        self.max_bytes
    }
}

/// Terminal body read failures; no partial body is returned.
#[derive(Debug, thiserror::Error)]
pub enum BodyReadError {
    /// Invalid operator budget configuration.
    #[error("invalid HTTP body budget")]
    InvalidBudget,
    /// Actual data bytes exceed the budget.
    #[error("HTTP body exceeds byte budget")]
    TooLarge,
    /// Absolute body deadline elapsed.
    #[error("HTTP body total deadline exceeded")]
    Deadline,
    /// No data progress within the idle budget.
    #[error("HTTP body idle deadline exceeded")]
    Idle,
    /// Source transport failed.
    #[error("HTTP body transport failed: {0}")]
    Transport(#[source] Box<dyn Error + Send + Sync>),
    /// Bounded allocation could not be reserved.
    #[error("HTTP body allocation failed")]
    Allocation(#[source] std::collections::TryReserveError),
}

impl BodyReadError {
    /// HTTP status for a protocol adapter's native error envelope.
    #[must_use]
    pub const fn status_code(&self) -> http::StatusCode {
        match self {
            Self::TooLarge => http::StatusCode::PAYLOAD_TOO_LARGE,
            Self::Idle | Self::Deadline => http::StatusCode::REQUEST_TIMEOUT,
            Self::Transport(_) => http::StatusCode::BAD_REQUEST,
            Self::InvalidBudget | Self::Allocation(_) => http::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// Body adapter checking every frame before passing ownership to its consumer.
#[derive(Debug)]
pub struct BudgetedBody<B> {
    inner: Pin<Box<B>>,
    remaining: u64,
    total: Pin<Box<Sleep>>,
    idle: Pin<Box<Sleep>>,
    idle_duration: Duration,
    finished: bool,
}

impl<B> BudgetedBody<B> {
    /// Wrap a source. Timers start immediately and are not reset by empty frames.
    pub fn new(body: B, budget: BodyBudget) -> Self {
        let now = Instant::now();
        Self {
            inner: Box::pin(body),
            remaining: budget.max_bytes,
            total: Box::pin(sleep_until(now + budget.total)),
            idle: Box::pin(sleep_until(now + budget.idle)),
            idle_duration: budget.idle,
            finished: false,
        }
    }
}

impl<B> Body for BudgetedBody<B>
where
    B: Body,
    B::Error: Error + Send + Sync + 'static,
{
    type Data = B::Data;
    type Error = BodyReadError;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        if this.finished {
            return Poll::Ready(None);
        }
        let timeout = if this.total.as_mut().poll(cx).is_ready() {
            Some(BodyReadError::Deadline)
        } else if this.idle.as_mut().poll(cx).is_ready() {
            Some(BodyReadError::Idle)
        } else {
            None
        };
        if let Some(error) = timeout {
            this.finished = true;
            return Poll::Ready(Some(Err(error)));
        }
        match this.inner.as_mut().poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    let size = data.remaining() as u64;
                    let Some(remaining) = this.remaining.checked_sub(size) else {
                        this.finished = true;
                        return Poll::Ready(Some(Err(BodyReadError::TooLarge)));
                    };
                    this.remaining = remaining;
                    if size != 0 {
                        this.idle
                            .as_mut()
                            .reset(Instant::now() + this.idle_duration);
                    }
                }
                Poll::Ready(Some(Ok(frame)))
            }
            Poll::Ready(Some(Err(error))) => {
                this.finished = true;
                Poll::Ready(Some(Err(BodyReadError::Transport(Box::new(error)))))
            }
            Poll::Ready(None) => {
                this.finished = true;
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn is_end_stream(&self) -> bool {
        self.finished
    }

    fn size_hint(&self) -> SizeHint {
        let mut hint = SizeHint::new();
        hint.set_upper(self.remaining);
        hint
    }
}

/// Collect only after enforcing the budget on each source frame.
///
/// # Errors
/// Returns overflow, timeout or transport errors, discarding partial bytes.
pub async fn collect_body<B>(body: B, budget: BodyBudget) -> Result<Bytes, BodyReadError>
where
    B: Body<Data = Bytes>,
    B::Error: Error + Send + Sync + 'static,
{
    let mut body = BudgetedBody::new(body, budget);
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        if let Ok(data) = frame?.into_data() {
            append_bounded(&mut bytes, &data, budget.max_bytes)?;
        }
    }
    Ok(Bytes::from(bytes))
}

/// Append bytes without letting geometric buffer growth exceed the byte budget.
/// # Errors
/// Rejects excess bytes before copying or returns a bounded allocation failure.
pub fn append_bounded(buffer: &mut Vec<u8>, data: &[u8], limit: u64) -> Result<(), BodyReadError> {
    let needed = buffer
        .len()
        .checked_add(data.len())
        .ok_or(BodyReadError::TooLarge)?;
    let limit = usize::try_from(limit).map_err(|_| BodyReadError::InvalidBudget)?;
    if needed > limit {
        return Err(BodyReadError::TooLarge);
    }
    if needed > buffer.capacity() {
        let target = buffer.capacity().saturating_mul(2).max(needed).min(limit);
        buffer
            .try_reserve_exact(target.saturating_sub(buffer.len()))
            .map_err(BodyReadError::Allocation)?;
    }
    buffer.extend_from_slice(data);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use http_body_util::Full;

    use super::*;

    #[tokio::test]
    async fn test_should_limit_actual_frames_before_collection() {
        let budget = BodyBudget::new(3, Duration::from_secs(1), Duration::from_secs(1)).unwrap();
        assert_eq!(
            collect_body(Full::new(Bytes::from_static(b"abc")), budget)
                .await
                .unwrap(),
            "abc"
        );
        assert!(matches!(
            collect_body(Full::new(Bytes::from_static(b"abcd")), budget).await,
            Err(BodyReadError::TooLarge)
        ));
    }

    #[derive(Debug)]
    struct PendingBody;
    impl Body for PendingBody {
        type Data = Bytes;
        type Error = Infallible;
        fn poll_frame(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
        ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
            Poll::Pending
        }
    }

    #[tokio::test]
    async fn test_should_expire_pending_body_without_eof() {
        let budget = BodyBudget::new(3, Duration::from_secs(1), Duration::from_millis(5)).unwrap();
        assert!(matches!(
            collect_body(PendingBody, budget).await,
            Err(BodyReadError::Idle)
        ));
        let budget =
            BodyBudget::new(3, Duration::from_millis(5), Duration::from_millis(5)).unwrap();
        assert!(matches!(
            collect_body(PendingBody, budget).await,
            Err(BodyReadError::Deadline)
        ));
    }

    #[test]
    fn test_should_keep_aggregate_capacity_inside_budget() {
        let mut buffer = Vec::new();
        for _ in 0..15 {
            append_bounded(&mut buffer, &[0; 100], 1500).unwrap();
            assert!(buffer.capacity() <= 1500);
        }
        let capacity = buffer.capacity();
        assert!(matches!(
            append_bounded(&mut buffer, &[0], 1500),
            Err(BodyReadError::TooLarge)
        ));
        assert_eq!(buffer.capacity(), capacity);
    }

    #[derive(Debug)]
    struct DripBody {
        timer: Pin<Box<Sleep>>,
    }
    impl Body for DripBody {
        type Data = Bytes;
        type Error = Infallible;
        fn poll_frame(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
            let this = self.get_mut();
            if this.timer.as_mut().poll(cx).is_pending() {
                return Poll::Pending;
            }
            this.timer
                .as_mut()
                .reset(Instant::now() + Duration::from_millis(2));
            Poll::Ready(Some(Ok(Frame::data(Bytes::from_static(b"x")))))
        }
    }

    #[tokio::test]
    async fn test_should_not_reset_total_deadline_on_progress() {
        let budget =
            BodyBudget::new(1000, Duration::from_millis(20), Duration::from_millis(10)).unwrap();
        let body = DripBody {
            timer: Box::pin(sleep_until(Instant::now())),
        };
        assert!(matches!(
            collect_body(body, budget).await,
            Err(BodyReadError::Deadline)
        ));
    }

    #[test]
    fn test_should_reject_invalid_budgets() {
        assert!(BodyBudget::new(0, Duration::from_secs(1), Duration::from_secs(1)).is_err());
        assert!(BodyBudget::new(1, Duration::ZERO, Duration::ZERO).is_err());
        assert!(BodyBudget::new(1, Duration::from_secs(1), Duration::from_secs(2)).is_err());
    }
}
