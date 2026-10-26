//! Window revocation and mutation consumption receipts; neither Drop claims IO joins.

use std::future::pending;

use rustack_lambda_runtime::protocol::AttemptId;
use tokio::{
    sync::watch,
    time::{Instant, sleep_until},
};
use uuid::Uuid;

use super::DockerError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct WindowKey {
    id: Uuid,
    attempt: AttemptId,
}
impl WindowKey {
    pub(super) fn new(attempt: AttemptId) -> Self {
        Self {
            id: Uuid::new_v4(),
            attempt,
        }
    }
}

/// Opaque ownership of one invocation's observed log window; consume with finish.
#[derive(Debug)]
#[non_exhaustive]
pub struct LogWindow {
    pub(super) key: WindowKey,
    revoke: watch::Sender<bool>,
    complete: bool,
}
impl LogWindow {
    pub(super) fn new(attempt: AttemptId) -> (Self, ActiveWindow) {
        let key = WindowKey::new(attempt);
        let (revoke, revoked) = watch::channel(false);
        (
            Self {
                key,
                revoke,
                complete: false,
            },
            ActiveWindow { key, revoked },
        )
    }
    pub(super) fn complete(&mut self) {
        self.complete = true;
    }
}
impl Drop for LogWindow {
    fn drop(&mut self) {
        if !self.complete {
            self.revoke.send_replace(true);
        }
    }
}
pub(super) struct ActiveWindow {
    pub(super) key: WindowKey,
    pub(super) revoked: watch::Receiver<bool>,
}

#[derive(Clone, Copy)]
enum ReceiptState {
    Pending,
    Consumed(Instant),
    Revoked,
}
pub(super) struct ReceiptGuard {
    sender: watch::Sender<ReceiptState>,
    complete: bool,
}
#[derive(Clone)]
pub(super) struct PendingReceipt {
    receiver: watch::Receiver<ReceiptState>,
    deadline: Instant,
}
impl ReceiptGuard {
    pub(super) fn new(deadline: Instant) -> (Self, PendingReceipt) {
        let (sender, receiver) = watch::channel(ReceiptState::Pending);
        (
            Self {
                sender,
                complete: false,
            },
            PendingReceipt { receiver, deadline },
        )
    }
    pub(super) fn consume(&mut self, deadline: Instant) -> Result<(), DockerError> {
        let consumed = Instant::now();
        if consumed >= deadline {
            return Err(DockerError::Deadline);
        }
        self.sender.send_replace(ReceiptState::Consumed(consumed));
        self.complete = true;
        Ok(())
    }
}
impl Drop for ReceiptGuard {
    fn drop(&mut self) {
        if !self.complete {
            self.sender.send_replace(ReceiptState::Revoked);
        }
    }
}
impl PendingReceipt {
    pub(super) fn consumed(&self) -> Result<bool, DockerError> {
        match *self.receiver.borrow() {
            ReceiptState::Consumed(at) if at < self.deadline => Ok(true),
            ReceiptState::Consumed(_) => Err(DockerError::Deadline),
            ReceiptState::Revoked => Err(DockerError::Protocol),
            ReceiptState::Pending if Instant::now() >= self.deadline => Err(DockerError::Deadline),
            ReceiptState::Pending if self.receiver.has_changed().is_err() => {
                Err(DockerError::Protocol)
            }
            ReceiptState::Pending => Ok(false),
        }
    }
}
pub(super) async fn changed(
    mut receipt: Option<PendingReceipt>,
    mut revocation: Option<watch::Receiver<bool>>,
) {
    tokio::select! { biased;
        ()=async {match &mut revocation { Some(receiver)=>{let _=receiver.wait_for(|revoked|*revoked).await;},None=>pending().await,}}=>{},
        ()=async {match &mut receipt {Some(receipt)=>tokio::select! {biased;_=receipt.receiver.changed()=>{},()=sleep_until(receipt.deadline)=>{}},None=>pending().await,}}=>{},
    }
}
