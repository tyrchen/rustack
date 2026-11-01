//! Independent function output capture; socket reads never wait for a consumer.

mod window;

use std::{collections::VecDeque, fmt, time::Duration};

use bytes::Bytes;
use rustack_lambda_runtime::{diagnostics::runtime_dispatch, protocol::AttemptId};
use tokio::{
    io::AsyncRead,
    runtime::Handle,
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
    time::{Instant, timeout_at},
};
use tracing::instrument::WithSubscriber;
pub use window::LogWindow;
use window::{ActiveWindow, PendingReceipt, ReceiptGuard, WindowKey, changed};

use super::{
    DockerAttachment, DockerError, DockerOutputReader, OutputChannel, OutputChunk, check_deadline,
};

const TAIL: usize = 4096;
const SILENCE: Duration = Duration::from_secs(3600);

/// Bounded binary API output and capture status; Debug never prints user bytes.
#[derive(Clone)]
#[non_exhaustive]
pub struct LogSnapshot {
    /// The final 4 KiB in observed stdout/stderr chunk order, without UTF-8 conversion.
    pub tail: Bytes,
    /// Old stdout bytes overwritten in its independent ring.
    pub stdout_discarded: u64,
    /// Old stderr bytes overwritten in its independent ring.
    pub stderr_discarded: u64,
    /// The attachment reached EOF; container exit remains independently owned.
    pub closed: bool,
    /// A retained framing/IO/deadline failure; joined shutdown preserves it.
    pub failure: Option<DockerError>,
}
impl fmt::Debug for LogSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LogSnapshot")
            .field("bytes", &self.tail.len())
            .field("stdout_discarded", &self.stdout_discarded)
            .field("stderr_discarded", &self.stderr_discarded)
            .field("closed", &self.closed)
            .field("failure", &self.failure)
            .finish()
    }
}

enum Action {
    Snapshot,
    Begin(ActiveWindow, PendingReceipt),
    Finish(WindowKey, PendingReceipt),
}
struct Request {
    action: Action,
    deadline: Instant,
    reply: oneshot::Sender<Result<LogSnapshot, DockerError>>,
}

/// Retained log actor; its environment owns the independent container lifetime.
#[non_exhaustive]
pub struct FunctionLogs {
    requests: mpsc::Sender<Request>,
    stop: watch::Sender<bool>,
    task: Option<JoinHandle<Result<LogSnapshot, DockerError>>>,
    settled: Option<Result<LogSnapshot, DockerError>>,
    window_mutating: bool,
}
impl fmt::Debug for FunctionLogs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FunctionLogs")
            .field("stopping", &*self.stop.borrow())
            .field("settled", &self.settled)
            .field("window_mutation_uncertain", &self.window_mutating)
            .finish_non_exhaustive()
    }
}
impl FunctionLogs {
    /// Start output capture with a4KiB..1MiB ring for each channel.
    ///
    /// # Errors
    /// Rejects invalid ring/record budgets, unavailable runtime or allocation.
    ///
    /// ```no_run
    /// # fn start(attachment: rustack_lambda_core::docker::DockerAttachment) -> Result<(), rustack_lambda_core::docker::DockerError> {
    /// let logs = rustack_lambda_core::docker::FunctionLogs::start(
    ///     attachment, std::time::Duration::from_secs(5), 262_144)?;
    /// # Ok(()) }
    /// ```
    pub fn start(
        attachment: DockerAttachment,
        io_budget: Duration,
        ring_bytes: usize,
    ) -> Result<Self, DockerError> {
        let reader = DockerOutputReader::new(attachment.0, io_budget)?;
        Self::start_reader(reader, ring_bytes)
    }

    pub(super) fn start_reader<R: AsyncRead + Unpin + Send + 'static>(
        reader: DockerOutputReader<R>,
        ring_bytes: usize,
    ) -> Result<Self, DockerError> {
        let runtime = Handle::try_current().map_err(|_| DockerError::Worker)?;
        if !(TAIL..=1024 * 1024).contains(&ring_bytes) {
            return Err(DockerError::Configuration);
        }
        let state = Capture::new(ring_bytes)?;
        let (requests, received) = mpsc::channel(8);
        let (stop, stopped) = watch::channel(false);
        let task = runtime
            .spawn(capture(reader, state, received, stopped).with_subscriber(runtime_dispatch()));
        Ok(Self {
            requests,
            stop,
            task: Some(task),
            settled: None,
            window_mutating: false,
        })
    }

    /// Read a bounded tail/status without interrupting a partial record.
    ///
    /// # Errors
    /// Returns expired/invalid budget, full mailbox, stopped actor or worker failure.
    ///
    /// ```no_run
    /// # async fn snapshot(logs: &rustack_lambda_core::docker::FunctionLogs) {
    /// let snapshot = logs.snapshot(tokio::time::Instant::now() + std::time::Duration::from_secs(5)).await;
    /// # }
    /// ```
    pub async fn snapshot(&self, deadline: Instant) -> Result<LogSnapshot, DockerError> {
        check_deadline(deadline)?;
        if let Some(result) = &self.settled {
            return result.clone();
        }
        self.request(Action::Snapshot, deadline).await
    }

    /// Open an empty invocation window before sending its runtime event.
    ///
    /// # Errors
    /// Returns expired/invalid budget, overload, overlapping/uncertain mutation,
    /// stopped/closed capture or retained IO failure. An accepted mutation's
    /// failure or cancellation forbids further window mutations; retire the owner.
    ///
    /// ```no_run
    /// # async fn begin(logs: &mut rustack_lambda_core::docker::FunctionLogs) {
    /// let window = logs.begin(Default::default(), tokio::time::Instant::now()+std::time::Duration::from_secs(5)).await;
    /// # }
    /// ```
    pub async fn begin(
        &mut self,
        attempt: AttemptId,
        deadline: Instant,
    ) -> Result<LogWindow, DockerError> {
        check_deadline(deadline)?;
        if self.window_mutating {
            return Err(DockerError::Stopped);
        }
        self.window_mutating = true;
        let (window, active) = LogWindow::new(attempt);
        let (mut receipt, pending) = ReceiptGuard::new(deadline);
        self.request(Action::Begin(active, pending), deadline)
            .await?;
        receipt.consume(deadline)?;
        self.window_mutating = false;
        Ok(window)
    }

    /// Close exactly that window and return its bounded observed tail/status.
    ///
    /// # Errors
    /// Returns budget/admission failure, foreign/stale capability, uncertain
    /// mutation or retained capture failure. Caller cancellation requires owner
    /// retirement. Environment diagnostic discard counters remain cumulative.
    ///
    /// ```no_run
    /// # async fn finish(logs: &mut rustack_lambda_core::docker::FunctionLogs, window: rustack_lambda_core::docker::LogWindow) {
    /// let tail = logs.finish(window, tokio::time::Instant::now()+std::time::Duration::from_secs(5)).await;
    /// # }
    /// ```
    pub async fn finish(
        &mut self,
        mut window: LogWindow,
        deadline: Instant,
    ) -> Result<LogSnapshot, DockerError> {
        check_deadline(deadline)?;
        if self.window_mutating {
            return Err(DockerError::Stopped);
        }
        self.window_mutating = true;
        let (mut receipt, pending) = ReceiptGuard::new(deadline);
        let snapshot = self
            .request(Action::Finish(window.key, pending), deadline)
            .await?;
        receipt.consume(deadline)?;
        window.complete();
        self.window_mutating = false;
        Ok(snapshot)
    }

    async fn request(&self, action: Action, deadline: Instant) -> Result<LogSnapshot, DockerError> {
        check_deadline(deadline)?;
        if *self.stop.borrow() {
            return Err(DockerError::Stopped);
        }
        let (reply, response) = oneshot::channel();
        self.requests
            .try_send(Request {
                action,
                deadline,
                reply,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => DockerError::Busy,
                mpsc::error::TrySendError::Closed(_) => DockerError::Worker,
            })?;
        let result = timeout_at(deadline, response)
            .await
            .map_err(|_| DockerError::Deadline)?
            .map_err(|_| DockerError::Worker)?;
        if Instant::now() >= deadline {
            return Err(DockerError::Deadline);
        }
        result
    }

    /// Stop and join socket ownership, preserving failure across cancelled waiters.
    ///
    /// # Errors
    /// Returns a retained parser/IO/deadline failure or failed actor join.
    /// Container cleanup must be awaited separately by the environment owner.
    ///
    /// ```no_run
    /// # async fn stop(logs: &mut rustack_lambda_core::docker::FunctionLogs) {
    /// let final_tail = logs.shutdown().await;
    /// # }
    /// ```
    pub async fn shutdown(&mut self) -> Result<LogSnapshot, DockerError> {
        self.stop.send_replace(true);
        if let Some(result) = &self.settled {
            return result.clone();
        }
        let result = match &mut self.task {
            Some(task) => task
                .await
                .map_err(|_| DockerError::Worker)
                .and_then(|result| result),
            None => Err(DockerError::Worker),
        };
        self.task.take();
        self.settled = Some(result.clone());
        result
    }
}
impl Drop for FunctionLogs {
    fn drop(&mut self) {
        self.stop.send_replace(true);
    }
}

struct Ring {
    bytes: VecDeque<u8>,
    capacity: usize,
    discarded: u64,
}
impl Ring {
    fn new(capacity: usize) -> Result<Self, DockerError> {
        let mut bytes = VecDeque::new();
        bytes
            .try_reserve_exact(capacity)
            .map_err(|_| DockerError::Allocation)?;
        Ok(Self {
            bytes,
            capacity,
            discarded: 0,
        })
    }
    fn push(&mut self, bytes: &[u8]) -> Result<(), DockerError> {
        let skipped = self
            .bytes
            .len()
            .saturating_add(bytes.len())
            .saturating_sub(self.capacity);
        self.discarded = self
            .discarded
            .saturating_add(u64::try_from(skipped).map_err(|_| DockerError::TooLarge)?);
        let remove = skipped.min(self.bytes.len());
        self.bytes.drain(..remove);
        let offset = bytes.len().saturating_sub(self.capacity);
        self.bytes
            .extend(bytes.get(offset..).ok_or(DockerError::Protocol)?);
        Ok(())
    }
}
struct Capture {
    stdout: Ring,
    stderr: Ring,
    tail: Ring,
    window_tail: Ring,
    active: Option<ActiveWindow>,
    receipt: Option<PendingReceipt>,
    closed: bool,
    failure: Option<DockerError>,
}
impl Capture {
    fn new(capacity: usize) -> Result<Self, DockerError> {
        Ok(Self {
            stdout: Ring::new(capacity)?,
            stderr: Ring::new(capacity)?,
            tail: Ring::new(TAIL)?,
            window_tail: Ring::new(TAIL)?,
            active: None,
            receipt: None,
            closed: false,
            failure: None,
        })
    }
    fn push(&mut self, chunk: &OutputChunk) -> Result<(), DockerError> {
        match chunk.channel {
            OutputChannel::Stdout => self.stdout.push(&chunk.bytes)?,
            OutputChannel::Stderr => self.stderr.push(&chunk.bytes)?,
        }
        self.tail.push(&chunk.bytes)?;
        if self.active.is_some() {
            self.window_tail.push(&chunk.bytes)?;
        }
        Ok(())
    }
    fn snapshot(&self) -> LogSnapshot {
        self.snapshot_of(&self.tail)
    }
    fn window_snapshot(&self) -> LogSnapshot {
        self.snapshot_of(&self.window_tail)
    }
    fn snapshot_of(&self, ring: &Ring) -> LogSnapshot {
        LogSnapshot {
            tail: Bytes::from(ring.bytes.iter().copied().collect::<Vec<_>>()),
            stdout_discarded: self.stdout.discarded,
            stderr_discarded: self.stderr.discarded,
            closed: self.closed,
            failure: self.failure.clone(),
        }
    }
    fn fail_window(&mut self, error: DockerError) {
        self.active = None;
        self.receipt = None;
        if self.failure.is_none() {
            self.failure = Some(error);
        }
    }
    fn reconcile(&mut self) -> bool {
        if let Some(receipt) = &self.receipt {
            match receipt.consumed() {
                Ok(true) => {
                    self.receipt = None;
                }
                Ok(false) => {}
                Err(error) => {
                    self.fail_window(error);
                    return true;
                }
            }
        }
        if self
            .active
            .as_ref()
            .is_some_and(|window| *window.revoked.borrow() || window.revoked.has_changed().is_err())
        {
            self.fail_window(DockerError::Protocol);
            true
        } else {
            false
        }
    }
    fn revocation(&self) -> Option<watch::Receiver<bool>> {
        self.active.as_ref().map(|window| window.revoked.clone())
    }
    fn reply(&mut self, request: Request) {
        self.reconcile();
        if request.reply.is_closed() {
            return;
        }
        let mut committed = false;
        let result = if Instant::now() >= request.deadline {
            Err(DockerError::Deadline)
        } else {
            match request.action {
                Action::Snapshot => Ok(self.snapshot()),
                _ if self.failure.is_some() => {
                    Err(self.failure.clone().unwrap_or(DockerError::Worker))
                }
                _ if self.receipt.is_some() => Err(DockerError::Busy),
                Action::Begin(_, _) if self.closed => Err(DockerError::Stopped),
                Action::Begin(_, _) if self.active.is_some() => Err(DockerError::Busy),
                Action::Begin(window, receipt) => {
                    self.window_tail.bytes.clear();
                    self.window_tail.discarded = 0;
                    self.active = Some(window);
                    self.receipt = Some(receipt);
                    committed = true;
                    Ok(self.window_snapshot())
                }
                Action::Finish(key, receipt)
                    if self.active.as_ref().map(|window| window.key) == Some(key) =>
                {
                    self.active = None;
                    self.receipt = Some(receipt);
                    committed = true;
                    Ok(self.window_snapshot())
                }
                Action::Finish(_, _) => Err(DockerError::Protocol),
            }
        };
        if request.reply.send(result).is_err() && committed {
            self.fail_window(DockerError::Protocol);
        }
    }
}

async fn capture<R: AsyncRead + Unpin>(
    reader: DockerOutputReader<R>,
    mut state: Capture,
    mut requests: mpsc::Receiver<Request>,
    mut stop: watch::Receiver<bool>,
) -> Result<LogSnapshot, DockerError> {
    let mut reader = Some(reader);
    let mut quantum = 0_u8;
    loop {
        if *stop.borrow() {
            break;
        }
        if state.reconcile() {
            reader.take();
        }
        if state.closed || state.failure.is_some() {
            let receipt = state.receipt.clone();
            let revocation = state.revocation();
            tokio::select! {
                biased;
                () = super::connection::stopped(&mut stop) => break,
                () = changed(receipt, revocation) => { state.reconcile(); },
                request = requests.recv() => match request { Some(request) => state.reply(request), None => break },
            }
            continue;
        }
        let result = match &mut reader {
            Some(reader) => read_next(reader, &mut state, &mut requests, &mut stop).await,
            None => return Err(DockerError::Worker),
        };
        match result {
            None => break,
            Some(Ok(None)) => state.closed = true,
            Some(Err(error)) => state.failure = Some(error),
            Some(Ok(Some(chunk))) => {
                if let Err(error) = state.push(&chunk) {
                    state.failure = Some(error);
                }
                quantum = quantum.saturating_add(1);
                if quantum == 64 {
                    tokio::task::yield_now().await;
                    quantum = 0;
                }
            }
        }
        if state.closed || state.failure.is_some() {
            reader.take();
        }
    }
    drop(reader);
    requests.close();
    while let Ok(request) = requests.try_recv() {
        if matches!(request.action, Action::Snapshot) {
            state.reply(request);
        } else {
            let _ = request.reply.send(Err(DockerError::Stopped));
        }
    }
    state.reconcile();
    match state.failure.clone() {
        Some(error) => Err(error),
        None => Ok(state.snapshot()),
    }
}

async fn read_next<R: AsyncRead + Unpin>(
    reader: &mut DockerOutputReader<R>,
    state: &mut Capture,
    requests: &mut mpsc::Receiver<Request>,
    stop: &mut watch::Receiver<bool>,
) -> Option<Result<Option<OutputChunk>, DockerError>> {
    // Requests retain this read; only stop or terminal capture failure drops it.
    let next = reader.next(Instant::now() + SILENCE);
    tokio::pin!(next);
    loop {
        if *stop.borrow() {
            return None;
        }
        state.reconcile();
        if let Some(error) = &state.failure {
            return Some(Err(error.clone()));
        }
        if let Ok(request) = requests.try_recv() {
            state.reply(request);
        }
        if let Some(error) = &state.failure {
            return Some(Err(error.clone()));
        }
        let receipt = state.receipt.clone();
        let revocation = state.revocation();
        tokio::select! {
            biased;
            () = super::connection::stopped(stop) => return None,
            () = changed(receipt, revocation) => { state.reconcile(); },
            result = &mut next => return Some(result),
            request = requests.recv() => match request { Some(request) => state.reply(request), None => return None },
        }
    }
}

#[cfg(test)]
#[path = "logs_tests.rs"]
mod tests;
