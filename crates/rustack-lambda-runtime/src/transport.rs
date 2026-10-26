//! Retained, bounded stream workers shared by both ends of relay control.

use std::time::Duration;

use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite},
    runtime::Handle,
    sync::{mpsc, oneshot, watch},
    task::{JoinHandle, JoinSet},
    time::{Instant, timeout_at},
};
use tracing::instrument::WithSubscriber;

use crate::{
    protocol::{Acknowledgement, Frame, Message, read_frame, write_frame},
    relay::RelayError,
};

const MAILBOX_CAPACITY: usize = 2;

#[derive(Debug, Clone, Copy)]
pub(crate) enum InputEnd {
    Close,
    CloseAcknowledgement,
}

impl InputEnd {
    fn matches(self, message: &Message) -> bool {
        matches!(
            (self, message),
            (Self::Close, Message::Close)
                | (
                    Self::CloseAcknowledgement,
                    Message::Ack(Acknowledgement::Close)
                )
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Settings {
    pub(crate) silence: Duration,
    pub(crate) io_budget: Duration,
    pub(crate) input_end: InputEnd,
}

#[derive(Debug)]
struct Outbound {
    frame: Frame,
    deadline: Instant,
    written: oneshot::Sender<bool>,
}

#[derive(Debug)]
enum WorkerExit {
    InputClose,
    OutputClosed,
}

#[derive(Debug)]
struct OwnerHandle {
    stop: watch::Sender<bool>,
    task: Option<JoinHandle<Result<(), RelayError>>>,
    settlement: Option<Result<(), RelayError>>,
}

#[derive(Debug)]
pub(crate) struct Transport {
    input: mpsc::Receiver<Frame>,
    output: mpsc::Sender<Outbound>,
    status: watch::Receiver<Option<Result<(), RelayError>>>,
    owner: OwnerHandle,
}

impl Transport {
    pub(crate) fn start<R, W>(reader: R, writer: W, settings: Settings) -> Result<Self, RelayError>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let runtime = Handle::try_current().map_err(|_| RelayError::Worker)?;
        let (stop, stopped) = watch::channel(false);
        let (input, frames) = mpsc::channel(MAILBOX_CAPACITY);
        let (output, queued) = mpsc::channel(MAILBOX_CAPACITY);
        let (status, observed) = watch::channel(None);
        let mut workers = JoinSet::new();
        workers.spawn(read_input(reader, input, settings).with_current_subscriber());
        workers.spawn(write_output(writer, queued).with_current_subscriber());
        let task = runtime.spawn(
            Owner {
                workers,
                stop: stopped,
                status,
            }
            .run()
            .with_current_subscriber(),
        );
        Ok(Self {
            input: frames,
            output,
            status: observed,
            owner: OwnerHandle {
                stop,
                task: Some(task),
                settlement: None,
            },
        })
    }

    pub(crate) fn finished(&self) -> bool {
        *self.owner.stop.borrow()
            || self.status.borrow().is_some()
            || self.status.has_changed().is_err()
    }

    pub(crate) fn stop(&self) {
        self.owner.stop.send_replace(true);
    }

    pub(crate) async fn recv(&mut self) -> Result<Frame, RelayError> {
        loop {
            if let Some(result) = self.status.borrow().as_ref() {
                return Err(result.clone().err().unwrap_or(RelayError::Eof));
            }
            tokio::select! {
                biased;
                changed = self.status.changed() => {
                    changed.map_err(|_| RelayError::Worker)?;
                }
                frame = self.input.recv() => match frame {
                    Some(frame) => return Ok(frame),
                    // The reader drops its sender before the supervisor joins
                    // it. Await that settlement so EOF cannot mask its actual
                    // protocol/deadline failure.
                    None => { self.status.changed().await.map_err(|_| RelayError::Worker)?; }
                },
            }
        }
    }

    pub(crate) async fn send(&self, frame: Frame, deadline: Instant) -> Result<(), RelayError> {
        if self.finished() {
            return Err(RelayError::Output);
        }
        let (written, receipt) = oneshot::channel();
        timeout_at(
            deadline,
            self.output.send(Outbound {
                frame,
                deadline,
                written,
            }),
        )
        .await
        .map_err(|_| RelayError::Deadline)?
        .map_err(|_| RelayError::Output)?;
        match timeout_at(deadline, receipt).await {
            Ok(Ok(true)) if Instant::now() < deadline => Ok(()),
            Ok(Ok(true)) => Err(RelayError::Deadline),
            _ => Err(RelayError::Output),
        }
    }

    pub(crate) async fn shutdown(&mut self) -> Result<(), RelayError> {
        self.stop();
        if let Some(result) = &self.owner.settlement {
            return result.clone();
        }
        let result = match self.owner.task.as_mut() {
            Some(task) => task
                .await
                .map_err(|_| RelayError::Worker)
                .and_then(|result| result),
            None => Ok(()),
        };
        self.owner.task.take();
        self.owner.settlement = Some(result.clone());
        result
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Debug)]
struct Owner {
    workers: JoinSet<Result<WorkerExit, RelayError>>,
    stop: watch::Receiver<bool>,
    status: watch::Sender<Option<Result<(), RelayError>>>,
}

impl Owner {
    async fn run(mut self) -> Result<(), RelayError> {
        let mut outcome = loop {
            tokio::select! {
                biased;
                () = async { let _ = self.stop.wait_for(|stop| *stop).await; } => break Ok(()),
                worker = self.workers.join_next() => match worker {
                    Some(Ok(Ok(WorkerExit::InputClose))) => {},
                    Some(Ok(Err(error))) => break Err(error),
                    _ => break Err(RelayError::Worker),
                }
            }
        };
        self.workers.abort_all();
        while let Some(result) = self.workers.join_next().await {
            let result = match result {
                Ok(Ok(WorkerExit::InputClose)) => Ok(()),
                Ok(Err(error)) => Err(error),
                Err(error) if error.is_cancelled() => Ok(()),
                Ok(Ok(WorkerExit::OutputClosed)) | Err(_) => Err(RelayError::Worker),
            };
            outcome = outcome.and(result);
        }
        self.status.send_replace(Some(outcome.clone()));
        outcome
    }
}

async fn read_input<R: AsyncRead + Unpin>(
    mut reader: R,
    input: mpsc::Sender<Frame>,
    settings: Settings,
) -> Result<WorkerExit, RelayError> {
    loop {
        let mut first = [0_u8];
        let count = timeout_at(after(settings.silence)?, reader.read(&mut first))
            .await
            .map_err(|_| RelayError::Deadline)??;
        if count == 0 {
            return Err(RelayError::Eof);
        }
        let mut prefixed = first.as_slice().chain(&mut reader);
        let frame = read_frame(&mut prefixed, after(settings.io_budget)?)
            .await?
            .ok_or(RelayError::Eof)?;
        let close = settings.input_end.matches(frame.message());
        timeout_at(after(settings.io_budget)?, input.send(frame))
            .await
            .map_err(|_| RelayError::Deadline)?
            .map_err(|_| RelayError::Worker)?;
        if close {
            return Ok(WorkerExit::InputClose);
        }
    }
}

async fn write_output<W: AsyncWrite + Unpin>(
    mut writer: W,
    mut output: mpsc::Receiver<Outbound>,
) -> Result<WorkerExit, RelayError> {
    while let Some(frame) = output.recv().await {
        let result = write_frame(&mut writer, &frame.frame, frame.deadline).await;
        let _ = frame.written.send(result.is_ok());
        result?;
    }
    Ok(WorkerExit::OutputClosed)
}

pub(crate) fn after(duration: Duration) -> Result<Instant, RelayError> {
    Instant::now()
        .checked_add(duration)
        .ok_or(RelayError::Configuration)
}
