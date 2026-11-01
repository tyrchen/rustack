//! Strict streaming non-TTY Docker output records; no full-record allocation.

use std::{fmt, time::Duration};

use bytes::Bytes;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    time::{Instant, timeout_at},
};

use super::DockerError;

/// Maximum outer Docker record, distinct from the inner CBOR frame allowance.
pub const MAX_DOCKER_RECORD: usize = 8 * 1024 * 1024;
const CHUNK_BYTES: usize = 16 * 1024;

/// Docker's validated non-TTY output channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OutputChannel {
    /// Standard output; relay output is binary control data.
    Stdout,
    /// Standard error; function logs are kept separately from relay control.
    Stderr,
}

/// At most16 KiB from one validated Docker record; Debug redacts content.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OutputChunk {
    /// Output channel advertised by the validated record header.
    pub channel: OutputChannel,
    /// Raw bytes; no implicit UTF-8 conversion or line splitting.
    pub bytes: Bytes,
}
impl fmt::Debug for OutputChunk {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OutputChunk")
            .field("channel", &self.channel)
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

#[derive(Debug)]
struct Record {
    channel: OutputChannel,
    remaining: usize,
    deadline: Instant,
}

/// One retained parser with a fixed deadline for each partial record.
#[non_exhaustive]
pub struct DockerOutputReader<R> {
    reader: R,
    record: Option<Record>,
    io_budget: Duration,
    poisoned: bool,
    closed: bool,
}
impl<R> fmt::Debug for DockerOutputReader<R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DockerOutputReader")
            .field("record", &self.record)
            .field("poisoned", &self.poisoned)
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

impl<R: AsyncRead + Unpin> DockerOutputReader<R> {
    /// Own the reader with a nonzero partial-record budget≤60s.
    ///
    /// # Errors
    /// Rejects a zero/excessive IO budget.
    ///
    /// ```
    /// let reader = rustack_lambda_core::docker::DockerOutputReader::new(
    ///     b"".as_slice(), std::time::Duration::from_secs(5))?;
    /// # Ok::<(), rustack_lambda_core::docker::DockerError>(())
    /// ```
    pub fn new(reader: R, io_budget: Duration) -> Result<Self, DockerError> {
        if io_budget.is_zero() || io_budget > Duration::from_secs(60) {
            return Err(DockerError::Configuration);
        }
        Ok(Self {
            reader,
            record: None,
            io_budget,
            poisoned: false,
            closed: false,
        })
    }

    /// Read one bounded chunk. Cancelling this future invalidates the reader.
    ///
    /// Boundary EOF returns None; a partial header/payload fails. The caller's
    /// absolute deadline may tighten, but never extend, a partial record lease.
    ///
    /// # Errors
    /// Returns framing, size, deadline, IO or interrupted-reader errors.
    ///
    /// ```no_run
    /// # async fn example<R: tokio::io::AsyncRead + Unpin>(reader: &mut rustack_lambda_core::docker::DockerOutputReader<R>) {
    /// let chunk = reader.next(tokio::time::Instant::now() + std::time::Duration::from_secs(30)).await;
    /// # }
    /// ```
    pub async fn next(&mut self, deadline: Instant) -> Result<Option<OutputChunk>, DockerError> {
        if self.poisoned {
            return Err(DockerError::Poisoned);
        }
        if self.closed {
            return Ok(None);
        }
        self.poisoned = true;
        let result = self.read(deadline).await;
        if result.is_ok() {
            self.poisoned = false;
        }
        result
    }

    async fn read(&mut self, deadline: Instant) -> Result<Option<OutputChunk>, DockerError> {
        let mut empty = 0_u8;
        loop {
            if Instant::now() >= deadline {
                return Err(DockerError::Deadline);
            }
            if self.record.is_none() {
                let mut header = [0_u8; 8];
                let first = header.get_mut(..1).ok_or(DockerError::Protocol)?;
                let count = timeout_at(deadline, self.reader.read(first))
                    .await
                    .map_err(|_| DockerError::Deadline)??;
                if Instant::now() >= deadline {
                    return Err(DockerError::Deadline);
                }
                if count == 0 {
                    self.closed = true;
                    return Ok(None);
                }
                let record_deadline = Instant::now()
                    .checked_add(self.io_budget)
                    .ok_or(DockerError::Configuration)?
                    .min(deadline);
                timeout_at(
                    record_deadline,
                    self.reader
                        .read_exact(header.get_mut(1..).ok_or(DockerError::Protocol)?),
                )
                .await
                .map_err(|_| DockerError::Deadline)??;
                if Instant::now() >= record_deadline {
                    return Err(DockerError::Deadline);
                }
                let channel = match header.first() {
                    Some(1) => OutputChannel::Stdout,
                    Some(2) => OutputChannel::Stderr,
                    _ => return Err(DockerError::Protocol),
                };
                if header.get(1..4) != Some([0_u8; 3].as_slice()) {
                    return Err(DockerError::Protocol);
                }
                let length: [u8; 4] = header
                    .get(4..)
                    .ok_or(DockerError::Protocol)?
                    .try_into()
                    .map_err(|_| DockerError::Protocol)?;
                let remaining = usize::try_from(u32::from_be_bytes(length))
                    .map_err(|_| DockerError::TooLarge)?;
                if remaining > MAX_DOCKER_RECORD {
                    return Err(DockerError::TooLarge);
                }
                if remaining == 0 {
                    empty = empty.saturating_add(1);
                    if empty == 64 {
                        tokio::task::yield_now().await;
                        empty = 0;
                    }
                    continue;
                }
                self.record = Some(Record {
                    channel,
                    remaining,
                    deadline: record_deadline,
                });
            }
            let record = self.record.as_mut().ok_or(DockerError::Protocol)?;
            let until = record.deadline.min(deadline);
            if Instant::now() >= until {
                return Err(DockerError::Deadline);
            }
            let size = record.remaining.min(CHUNK_BYTES);
            let mut output = Vec::new();
            output
                .try_reserve_exact(size)
                .map_err(|_| DockerError::Allocation)?;
            output.resize(size, 0);
            timeout_at(until, self.reader.read_exact(&mut output))
                .await
                .map_err(|_| DockerError::Deadline)??;
            if Instant::now() >= until {
                return Err(DockerError::Deadline);
            }
            record.remaining = record
                .remaining
                .checked_sub(size)
                .ok_or(DockerError::Protocol)?;
            let channel = record.channel;
            if record.remaining == 0 {
                self.record = None;
            }
            return Ok(Some(OutputChunk {
                channel,
                bytes: Bytes::from(output),
            }));
        }
    }
}
