//! Queue actor: per-queue message lifecycle management.
//!
//! Each queue runs as an independent actor that owns all its state and
//! communicates via a `tokio::sync::mpsc` channel. The actor supports both
//! standard and FIFO queue types.

use std::{
    collections::HashMap,
    future::{Future, poll_fn},
    pin::Pin,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
    task::Poll,
    time::Duration,
};

use dashmap::DashMap;
use rustack_sqs_model::{
    error::SqsError,
    input::{ReceiveMessageInput, SendMessageInput},
    output::{ReceiveMessageOutput, SendMessageOutput},
    types::Message,
};
use tokio::{
    sync::{mpsc, oneshot, watch},
    time::Instant,
};

use super::{
    attributes::QueueAttributes,
    storage::{DedupKey, EnqueueResult, FifoQueueStorage, StandardQueueStorage},
};
use crate::message::{
    InFlightMessage, QueueMessage, generate_receipt_handle, md5_of_body, md5_of_message_attributes,
    now_epoch_millis,
};

/// Registry routing shared with the queue manager, never held across await.
type QueueRoutes = Weak<DashMap<String, Arc<QueueHandle>>>;

#[derive(Debug, Default)]
struct RedriveManager {
    routes: QueueRoutes,
    pending: Vec<PendingTransfer>,
    quiescing: bool,
}

#[derive(Debug)]
struct PendingTransfer {
    message: QueueMessage,
    reply: oneshot::Receiver<Result<(), SqsError>>,
    completion: Option<Result<(), SqsError>>,
}

impl RedriveManager {
    fn handoff(&mut self, message: &QueueMessage, target_arn: &str, source_arn: &str) -> bool {
        if self.quiescing || self.pending.len() >= 128 || source_arn == target_arn {
            return false;
        }
        let Some(routes) = self.routes.upgrade() else {
            return false;
        };
        let Some(name) = target_arn.rsplit(':').next() else {
            return false;
        };
        let Some(target) = routes.get(name).map(|entry| Arc::clone(entry.value())) else {
            tracing::warn!("DLQ target unavailable; retaining source message");
            return false;
        };
        if target.metadata.arn != target_arn || target.shutdown.load(Ordering::Acquire) {
            return false;
        }
        let (reply, receiver) = oneshot::channel();
        if target
            .sender
            .try_send(QueueCommand::Transfer {
                message: message.clone(),
                source_arn: source_arn.to_owned(),
                reply,
            })
            .is_err()
        {
            tracing::warn!("DLQ target channel unavailable/full; retaining source message");
            return false;
        }
        self.pending.push(PendingTransfer {
            message: message.clone(),
            reply: receiver,
            completion: None,
        });
        true
    }
}

/// Register each acknowledgment with the actor's waker; no timer polling or
/// actor-to-actor await blocks ordinary queue commands.
async fn wait_for_transfer(pending: &mut [PendingTransfer]) {
    poll_fn(|context| {
        for transfer in &mut *pending {
            if transfer.completion.is_some() {
                return Poll::Ready(());
            }
            if let Poll::Ready(result) = Pin::new(&mut transfer.reply).poll(context) {
                transfer.completion = Some(result.unwrap_or_else(|_| {
                    Err(SqsError::internal_error(
                        "DLQ actor closed before acknowledgment",
                    ))
                }));
                return Poll::Ready(());
            }
        }
        Poll::Pending
    })
    .await;
}

/// Commands sent to a queue actor via its channel.
pub enum QueueCommand {
    /// Test-only actor supervision fault injection.
    #[cfg(test)]
    CrashActor,
    /// Transfer a dead-letter message without blocking the source actor.
    Transfer {
        /// Original message retained by the source until acknowledgment.
        message: QueueMessage,
        /// Exact source queue ARN.
        source_arn: String,
        /// Confirms successful enqueue or rejection before mutation.
        reply: oneshot::Sender<Result<(), SqsError>>,
    },
    /// Stop initiating redrives and acknowledge after pending transfers settle.
    Quiesce {
        /// Completion acknowledgment.
        reply: oneshot::Sender<()>,
    },
    /// Send a message to the queue.
    SendMessage {
        /// The send message input.
        input: SendMessageInput,
        /// Reply channel for the result.
        reply: oneshot::Sender<Result<SendMessageOutput, SqsError>>,
    },
    /// Receive messages from the queue.
    ReceiveMessage {
        /// The receive message input.
        input: ReceiveMessageInput,
        /// Reply channel for the result.
        reply: oneshot::Sender<Result<ReceiveMessageOutput, SqsError>>,
    },
    /// Delete a message by receipt handle.
    DeleteMessage {
        /// Receipt handle of the message to delete.
        receipt_handle: String,
        /// Reply channel for the result.
        reply: oneshot::Sender<Result<(), SqsError>>,
    },
    /// Change visibility timeout of a message.
    ChangeVisibility {
        /// Receipt handle.
        receipt_handle: String,
        /// New visibility timeout in seconds.
        visibility_timeout: i32,
        /// Reply channel for the result.
        reply: oneshot::Sender<Result<(), SqsError>>,
    },
    /// Get queue attributes.
    GetAttributes {
        /// Attribute names to retrieve.
        attribute_names: Vec<String>,
        /// Reply channel.
        reply: oneshot::Sender<HashMap<String, String>>,
    },
    /// Set queue attributes.
    SetAttributes {
        /// Attributes to set.
        attributes: HashMap<String, String>,
        /// Reply channel.
        reply: oneshot::Sender<Result<(), SqsError>>,
    },
    /// Purge all messages.
    Purge {
        /// Reply channel.
        reply: oneshot::Sender<Result<(), SqsError>>,
    },
    /// Get tags.
    GetTags {
        /// Reply channel.
        reply: oneshot::Sender<HashMap<String, String>>,
    },
    /// Set tags.
    SetTags {
        /// Tags to add/update.
        tags: HashMap<String, String>,
        /// Reply channel.
        reply: oneshot::Sender<()>,
    },
    /// Remove tags.
    RemoveTags {
        /// Tag keys to remove.
        keys: Vec<String>,
        /// Reply channel.
        reply: oneshot::Sender<()>,
    },
    /// Shutdown the actor.
    Shutdown,
}

impl std::fmt::Debug for QueueCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transfer { .. } => write!(f, "Transfer"),
            Self::Quiesce { .. } => write!(f, "Quiesce"),
            Self::SendMessage { .. } => write!(f, "SendMessage"),
            Self::ReceiveMessage { .. } => write!(f, "ReceiveMessage"),
            Self::DeleteMessage { .. } => write!(f, "DeleteMessage"),
            Self::ChangeVisibility { .. } => write!(f, "ChangeVisibility"),
            Self::GetAttributes { .. } => write!(f, "GetAttributes"),
            Self::SetAttributes { .. } => write!(f, "SetAttributes"),
            Self::Purge { .. } => write!(f, "Purge"),
            Self::GetTags { .. } => write!(f, "GetTags"),
            Self::SetTags { .. } => write!(f, "SetTags"),
            Self::RemoveTags { .. } => write!(f, "RemoveTags"),
            #[cfg(test)]
            Self::CrashActor => write!(f, "CrashActor"),
            Self::Shutdown => write!(f, "Shutdown"),
        }
    }
}

/// Dispatch-enum over Standard and FIFO storage.
enum QueueStorage {
    /// Standard queue storage.
    Standard(StandardQueueStorage),
    /// FIFO queue storage.
    Fifo(FifoQueueStorage),
}

impl std::fmt::Debug for QueueStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Standard(_) => write!(f, "Standard"),
            Self::Fifo(_) => write!(f, "Fifo"),
        }
    }
}

/// Per-queue actor that owns all message state.
pub struct QueueActor {
    /// Queue name.
    name: String,
    /// Queue ARN.
    arn: String,
    /// Whether this is a FIFO queue.
    is_fifo: bool,
    /// Queue attributes.
    attributes: QueueAttributes,
    /// Queue storage (Standard or FIFO).
    storage: QueueStorage,
    /// Command channel receiver.
    commands: mpsc::Receiver<QueueCommand>,
    /// Tags.
    tags: HashMap<String, String>,
    /// Creation timestamp (epoch seconds).
    created_at: u64,
    /// Last modified timestamp (epoch seconds).
    last_modified_at: u64,
    /// Last purge timestamp.
    last_purge_at: Option<Instant>,
    /// Account ID (for sender ID).
    account_id: String,
    /// Pending long-poll receivers.
    pending_long_polls: Vec<PendingLongPoll>,
    redrive: RedriveManager,
    quiesce_reply: Option<oneshot::Sender<()>>,
    stopping: bool,
}

/// A pending long-poll request waiting for messages.
struct PendingLongPoll {
    /// Reply channel.
    reply: oneshot::Sender<Result<ReceiveMessageOutput, SqsError>>,
    /// Maximum messages to return.
    max_messages: i32,
    /// Per-request visibility timeout in seconds.
    visibility_timeout: i32,
    /// When the poll times out.
    deadline: Instant,
    /// System attribute names requested.
    attribute_names: Vec<String>,
    /// User attribute names requested.
    message_attribute_names: Vec<String>,
}

impl std::fmt::Debug for QueueActor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueueActor")
            .field("name", &self.name)
            .field("is_fifo", &self.is_fifo)
            .finish_non_exhaustive()
    }
}

impl QueueActor {
    /// Create a new queue actor.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: String,
        arn: String,
        is_fifo: bool,
        attributes: QueueAttributes,
        commands: mpsc::Receiver<QueueCommand>,
        tags: HashMap<String, String>,
        account_id: String,
        created_at: u64,
    ) -> Self {
        let storage = if is_fifo {
            QueueStorage::Fifo(FifoQueueStorage::default())
        } else {
            QueueStorage::Standard(StandardQueueStorage::default())
        };

        Self {
            name,
            arn,
            is_fifo,
            attributes,
            storage,
            commands,
            tags,
            created_at,
            last_modified_at: created_at,
            last_purge_at: None,
            account_id,
            pending_long_polls: Vec::new(),
            redrive: RedriveManager::default(),
            quiesce_reply: None,
            stopping: false,
        }
    }

    /// Connect this actor to the manager's queue routing registry.
    pub(crate) fn with_routes(mut self, routes: QueueRoutes) -> Self {
        self.redrive.routes = routes;
        self
    }

    /// Run the actor event loop.
    pub async fn run(mut self) {
        let mut cleanup_interval = tokio::time::interval(Duration::from_secs(1));
        let mut commands_open = true;
        loop {
            self.settle_transfers();
            if self.redrive.pending.is_empty() {
                if let Some(reply) = self.quiesce_reply.take() {
                    let _ = reply.send(());
                }
                if self.stopping {
                    break;
                }
            }
            // Compute the earliest long-poll deadline so we can expire it precisely
            // instead of waiting for the next 1-second cleanup tick.
            let next_poll_deadline = self
                .pending_long_polls
                .iter()
                .map(|p| p.deadline)
                .min()
                .unwrap_or_else(|| Instant::now() + Duration::from_hours(24));

            tokio::select! {
                command = self.commands.recv(), if commands_open => {
                    let Some(cmd) = command else {
                        commands_open = false;
                        self.stopping = true;
                        self.redrive.quiescing = true;
                        continue;
                    };
                    let should_fulfill_long_polls = match cmd {
                        QueueCommand::Shutdown => {
                            self.stopping = true;
                            self.redrive.quiescing = true;
                            false
                        },
                        cmd => self.handle_command(cmd),
                    };
                    if should_fulfill_long_polls && !self.pending_long_polls.is_empty() {
                        self.fulfill_pending_long_polls();
                    }
                }
                () = wait_for_transfer(&mut self.redrive.pending), if !self.redrive.pending.is_empty() => {}
                _ = cleanup_interval.tick() => {
                    self.periodic_cleanup();
                }
                () = tokio::time::sleep_until(next_poll_deadline), if !self.pending_long_polls.is_empty() => {
                    self.expire_long_polls();
                }
            }
        }
        tracing::debug!(queue = %self.name, "queue actor shutting down");
    }

    /// Handle a single command.
    #[allow(clippy::too_many_lines)]
    fn handle_command(&mut self, cmd: QueueCommand) -> bool {
        match cmd {
            #[cfg(test)]
            QueueCommand::CrashActor => panic!("injected queue actor crash"),
            QueueCommand::Transfer {
                message,
                source_arn,
                reply,
            } => {
                if reply.is_closed() {
                    return false;
                }
                let result = self.accept_transfer(message, &source_arn);
                let accepted = result.is_ok();
                let _ = reply.send(result);
                accepted
            }
            QueueCommand::Quiesce { reply } => {
                self.redrive.quiescing = true;
                if self
                    .quiesce_reply
                    .as_ref()
                    .is_none_or(oneshot::Sender::is_closed)
                {
                    self.quiesce_reply = Some(reply);
                }
                false
            }
            QueueCommand::SendMessage { input, reply } => {
                if reply.is_closed() {
                    return false;
                }
                let result = self.handle_send_message(input);
                let should_fulfill_long_polls = matches!(&result, Ok((_, true)));
                let _ = reply.send(result.map(|(output, _)| output));
                should_fulfill_long_polls
            }
            QueueCommand::ReceiveMessage { input, reply } => {
                self.handle_receive_message(input, reply);
                false
            }
            QueueCommand::DeleteMessage {
                receipt_handle,
                reply,
            } => {
                let should_fulfill_long_polls = self.handle_delete_message(&receipt_handle);
                let _ = reply.send(Ok(()));
                should_fulfill_long_polls
            }
            QueueCommand::ChangeVisibility {
                receipt_handle,
                visibility_timeout,
                reply,
            } => {
                let result = self.handle_change_visibility(&receipt_handle, visibility_timeout);
                let should_fulfill_long_polls = matches!(&result, Ok(true));
                let _ = reply.send(result.map(|_| ()));
                should_fulfill_long_polls
            }
            QueueCommand::GetAttributes {
                attribute_names,
                reply,
            } => {
                let mut counts = match &self.storage {
                    QueueStorage::Standard(s) => s.counts(),
                    QueueStorage::Fifo(s) => s.counts(),
                };
                counts.1 = counts
                    .1
                    .saturating_add(u32::try_from(self.redrive.pending.len()).unwrap_or(u32::MAX));
                let attrs = self.attributes.to_map(
                    &attribute_names,
                    self.is_fifo,
                    &self.arn,
                    self.created_at,
                    self.last_modified_at,
                    counts,
                );
                let _ = reply.send(attrs);
                false
            }
            QueueCommand::SetAttributes { attributes, reply } => {
                let result = self.attributes.update_from_map(&attributes, self.is_fifo);
                if result.is_ok() {
                    self.last_modified_at = crate::message::now_epoch_seconds();
                }
                let _ = reply.send(result);
                false
            }
            QueueCommand::Purge { reply } => {
                let result = self.handle_purge();
                let _ = reply.send(result);
                false
            }
            QueueCommand::GetTags { reply } => {
                let _ = reply.send(self.tags.clone());
                false
            }
            QueueCommand::SetTags { tags, reply } => {
                self.tags.extend(tags);
                let _ = reply.send(());
                false
            }
            QueueCommand::RemoveTags { keys, reply } => {
                for key in &keys {
                    self.tags.remove(key);
                }
                let _ = reply.send(());
                false
            }
            QueueCommand::Shutdown => {
                // Handled in the event loop.
                false
            }
        }
    }

    fn accept_transfer(
        &mut self,
        mut message: QueueMessage,
        source_arn: &str,
    ) -> Result<(), SqsError> {
        if self.stopping || self.is_fifo != message.message_group_id.is_some() {
            return Err(SqsError::invalid_parameter_value(
                "DLQ unavailable or queue type mismatch",
            ));
        }
        if message.body.len() > usize::try_from(self.attributes.maximum_message_size).unwrap_or(0) {
            return Err(SqsError::invalid_parameter_value(
                "Message exceeds DLQ maximum size",
            ));
        }
        let source_scope = source_arn.rsplit_once(':').map(|(scope, _)| scope);
        let target_scope = self.arn.rsplit_once(':').map(|(scope, _)| scope);
        if source_scope != target_scope {
            return Err(SqsError::invalid_parameter_value(
                "DLQ must be in the same account and region",
            ));
        }
        if let Some(policy) = &self.attributes.redrive_allow_policy {
            let policy: serde_json::Value = serde_json::from_str(policy)
                .map_err(|_| SqsError::invalid_parameter_value("Invalid RedriveAllowPolicy"))?;
            let allowed = match policy
                .get("redrivePermission")
                .and_then(serde_json::Value::as_str)
            {
                Some("allowAll") => true,
                Some("byQueue") => policy
                    .get("sourceQueueArns")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|arns| arns.iter().any(|arn| arn.as_str() == Some(source_arn))),
                _ => false,
            };
            if !allowed {
                return Err(SqsError::invalid_parameter_value(
                    "DLQ redrive permission denied",
                ));
            }
        }
        message.dead_letter_queue_source_arn = Some(source_arn.to_owned());
        message.approximate_receive_count = 0;
        message.approximate_first_receive_timestamp = None;
        message.available_at = Instant::now();
        message.delay_seconds = 0;
        match &mut self.storage {
            QueueStorage::Standard(storage) => storage.available.push_back(message),
            QueueStorage::Fifo(storage) => {
                message.sent_timestamp = now_epoch_millis();
                message.message_deduplication_id = Some(message.message_id.clone());
                let key = DedupKey::Redrive {
                    source: source_arn.to_owned(),
                    message: message.message_id.clone(),
                };
                storage.enqueue(message, &key);
            }
        }
        Ok(())
    }

    fn settle_transfers(&mut self) {
        let pending = std::mem::take(&mut self.redrive.pending);
        for mut transfer in pending {
            let result = if let Some(completed) = transfer.completion.take() {
                completed
            } else {
                match transfer.reply.try_recv() {
                    Ok(result) => result,
                    Err(oneshot::error::TryRecvError::Empty) => {
                        self.redrive.pending.push(transfer);
                        continue;
                    }
                    Err(oneshot::error::TryRecvError::Closed) => Err(SqsError::internal_error(
                        "DLQ actor closed before acknowledgment",
                    )),
                }
            };
            let group = transfer
                .message
                .message_group_id
                .clone()
                .unwrap_or_default();
            let restore = if result.is_err() {
                tracing::warn!("DLQ delivery failed; restoring source message");
                Some(transfer.message)
            } else {
                None
            };
            match &mut self.storage {
                QueueStorage::Standard(storage) => {
                    if let Some(message) = restore {
                        storage.available.push_front(message);
                    }
                }
                QueueStorage::Fifo(storage) => storage.finish_redrive(&group, restore),
            }
        }
    }

    /// Handle `SendMessage`.
    #[allow(clippy::cast_sign_loss)]
    fn handle_send_message(
        &mut self,
        input: SendMessageInput,
    ) -> Result<(SendMessageOutput, bool), SqsError> {
        // Validate message body.
        if input.message_body.is_empty() {
            return Err(SqsError::invalid_parameter_value(
                "The request must contain the parameter MessageBody.",
            ));
        }
        let body_bytes = input.message_body.len();
        if body_bytes > self.attributes.maximum_message_size as usize {
            return Err(SqsError::invalid_parameter_value(format!(
                "One or more parameters are invalid. Reason: Message must be shorter than {} \
                 bytes.",
                self.attributes.maximum_message_size
            )));
        }

        // Validate message attributes count.
        if input.message_attributes.len() > 10 {
            return Err(SqsError::invalid_parameter_value(
                "Number of message attributes [{}] exceeds the allowed maximum [10].",
            ));
        }

        if self.is_fifo {
            self.handle_send_message_fifo(input)
        } else {
            self.handle_send_message_standard(input)
        }
    }

    /// Send a message to a standard queue.
    #[allow(clippy::cast_sign_loss)]
    fn handle_send_message_standard(
        &mut self,
        input: SendMessageInput,
    ) -> Result<(SendMessageOutput, bool), SqsError> {
        // Reject FIFO-only fields on standard queues.
        if input.message_group_id.is_some() {
            return Err(SqsError::invalid_parameter_value(
                "Value for parameter MessageGroupId is invalid. Reason: The request includes a \
                 parameter that is not valid for this queue type.",
            ));
        }
        if input.message_deduplication_id.is_some() {
            return Err(SqsError::invalid_parameter_value(
                "Value for parameter MessageDeduplicationId is invalid. Reason: The request \
                 includes a parameter that is not valid for this queue type.",
            ));
        }

        let QueueStorage::Standard(ref mut storage) = self.storage else {
            return Err(SqsError::internal_error("Storage type mismatch"));
        };

        let message_id = uuid::Uuid::new_v4().to_string();
        let body_md5 = md5_of_body(&input.message_body);
        let attr_md5 = md5_of_message_attributes(&input.message_attributes);

        let delay_seconds = input.delay_seconds.unwrap_or(self.attributes.delay_seconds);
        let available_at = if delay_seconds > 0 {
            Instant::now() + Duration::from_secs(delay_seconds as u64)
        } else {
            Instant::now()
        };

        let msg = QueueMessage {
            dead_letter_queue_source_arn: None,
            message_id: message_id.clone(),
            body: input.message_body,
            md5_of_body: body_md5.clone(),
            message_attributes: input.message_attributes,
            md5_of_message_attributes: attr_md5.clone(),
            sender_id: self.account_id.clone(),
            sent_timestamp: now_epoch_millis(),
            approximate_receive_count: 0,
            approximate_first_receive_timestamp: None,
            sequence_number: None,
            message_group_id: None,
            message_deduplication_id: None,
            available_at,
            delay_seconds,
        };

        if delay_seconds > 0 {
            storage.delayed.push(msg);
        } else {
            storage.available.push_back(msg);
        }

        let should_fulfill_long_polls = delay_seconds <= 0;

        Ok((
            SendMessageOutput {
                message_id: Some(message_id),
                md5_of_message_body: Some(body_md5),
                md5_of_message_attributes: attr_md5,
                md5_of_message_system_attributes: None,
                sequence_number: None,
            },
            should_fulfill_long_polls,
        ))
    }

    /// Send a message to a FIFO queue with deduplication and sequencing.
    fn handle_send_message_fifo(
        &mut self,
        input: SendMessageInput,
    ) -> Result<(SendMessageOutput, bool), SqsError> {
        let QueueStorage::Fifo(ref mut storage) = self.storage else {
            return Err(SqsError::internal_error("Storage type mismatch"));
        };

        // FIFO queues do not support per-message delay.
        if input.delay_seconds.is_some_and(|d| d > 0) {
            return Err(SqsError::invalid_parameter_value(
                "Value 0 for parameter DelaySeconds is invalid. Reason: The request includes a \
                 parameter that is not valid for this queue type.",
            ));
        }

        // FIFO queues require MessageGroupId.
        let group_id = input.message_group_id.clone().ok_or_else(|| {
            SqsError::missing_parameter("The request must contain the parameter MessageGroupId.")
        })?;

        // Resolve deduplication ID.
        let dedup_id = if let Some(ref id) = input.message_deduplication_id {
            id.clone()
        } else if self.attributes.content_based_deduplication {
            // SHA-256 of the body.
            use sha2::{Digest, Sha256};
            let hash = Sha256::digest(input.message_body.as_bytes());
            hex::encode(hash)
        } else {
            return Err(SqsError::invalid_parameter_value(
                "The queue should either have ContentBasedDeduplication enabled or \
                 MessageDeduplicationId provided explicitly.",
            ));
        };

        // Build the effective dedup key based on DeduplicationScope.
        // "queue" scope: global dedup across all groups (default).
        // "messageGroup" scope: dedup only within the same group.
        let message_deduplication_id = Some(dedup_id.clone());
        let effective_dedup_key = if self.attributes.deduplication_scope == "messageGroup" {
            DedupKey::Group {
                group: group_id.clone(),
                id: dedup_id,
            }
        } else {
            DedupKey::Queue(dedup_id)
        };

        let message_id = uuid::Uuid::new_v4().to_string();
        let body_md5 = md5_of_body(&input.message_body);
        let attr_md5 = md5_of_message_attributes(&input.message_attributes);

        let msg = QueueMessage {
            dead_letter_queue_source_arn: None,
            message_id: message_id.clone(),
            body: input.message_body,
            md5_of_body: body_md5.clone(),
            message_attributes: input.message_attributes,
            md5_of_message_attributes: attr_md5.clone(),
            sender_id: self.account_id.clone(),
            sent_timestamp: now_epoch_millis(),
            approximate_receive_count: 0,
            approximate_first_receive_timestamp: None,
            sequence_number: None,
            message_group_id: Some(group_id),
            message_deduplication_id,
            available_at: Instant::now(),
            delay_seconds: 0,
        };

        let enqueue_result = storage.enqueue(msg, &effective_dedup_key);

        // On dedup, return the original message's ID and sequence number per AWS spec.
        match enqueue_result {
            EnqueueResult::Enqueued {
                message_id: mid,
                sequence_number,
            } => Ok((
                SendMessageOutput {
                    message_id: Some(mid),
                    md5_of_message_body: Some(body_md5),
                    md5_of_message_attributes: attr_md5,
                    md5_of_message_system_attributes: None,
                    sequence_number: Some(sequence_number),
                },
                true,
            )),
            EnqueueResult::Deduplicated {
                message_id: mid,
                sequence_number,
            } => Ok((
                SendMessageOutput {
                    message_id: Some(mid),
                    md5_of_message_body: Some(body_md5),
                    md5_of_message_attributes: attr_md5,
                    md5_of_message_system_attributes: None,
                    sequence_number: Some(sequence_number),
                },
                false,
            )),
        }
    }

    /// Handle `ReceiveMessage`.
    #[allow(clippy::cast_sign_loss)]
    fn handle_receive_message(
        &mut self,
        input: ReceiveMessageInput,
        reply: oneshot::Sender<Result<ReceiveMessageOutput, SqsError>>,
    ) {
        let max_messages = input.max_number_of_messages.unwrap_or(1).clamp(1, 10);
        let wait_time = input
            .wait_time_seconds
            .unwrap_or(self.attributes.receive_message_wait_time_seconds);
        let visibility_timeout = input
            .visibility_timeout
            .unwrap_or(self.attributes.visibility_timeout);

        let messages = self.try_receive(
            max_messages,
            visibility_timeout,
            &input.attribute_names,
            &input.message_system_attribute_names,
            &input.message_attribute_names,
        );

        if !messages.is_empty() || wait_time <= 0 {
            let _ = reply.send(Ok(ReceiveMessageOutput { messages }));
            return;
        }

        // Long poll: store the pending reply.
        self.pending_long_polls.push(PendingLongPoll {
            reply,
            max_messages,
            visibility_timeout,
            deadline: Instant::now() + Duration::from_secs(wait_time as u64),
            attribute_names: merge_attribute_names(
                &input.attribute_names,
                &input.message_system_attribute_names,
            ),
            message_attribute_names: input.message_attribute_names,
        });
    }

    /// Try to receive messages immediately from the queue.
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    fn try_receive(
        &mut self,
        max_messages: i32,
        visibility_timeout: i32,
        attribute_names: &[String],
        system_attribute_names: &[String],
        message_attribute_names: &[String],
    ) -> Vec<Message> {
        // Visibility is evaluated at receive time, not only on the periodic timer.
        match &mut self.storage {
            QueueStorage::Standard(storage) => {
                storage.return_expired_inflight();
                storage.promote_delayed();
            }
            QueueStorage::Fifo(storage) => {
                storage.return_expired_inflight();
            }
        }
        let merged_sys_attrs = merge_attribute_names(attribute_names, system_attribute_names);
        let vis_timeout = Duration::from_secs(visibility_timeout as u64);
        let mut redrive = RedriveRequest {
            manager: &mut self.redrive,
            attributes: &self.attributes,
            source_arn: &self.arn,
        };
        match &mut self.storage {
            QueueStorage::Standard(storage) => try_receive_standard(
                storage,
                max_messages as usize,
                vis_timeout,
                &merged_sys_attrs,
                message_attribute_names,
                &mut redrive,
            ),
            QueueStorage::Fifo(storage) => try_receive_fifo(
                storage,
                max_messages as usize,
                vis_timeout,
                &merged_sys_attrs,
                message_attribute_names,
                &mut redrive,
            ),
        }
    }

    /// Handle `DeleteMessage`.
    fn handle_delete_message(&mut self, receipt_handle: &str) -> bool {
        match &mut self.storage {
            QueueStorage::Standard(storage) => {
                storage.in_flight.remove(receipt_handle);
                false
            }
            QueueStorage::Fifo(storage) => storage.delete_message(receipt_handle),
        }
    }

    /// Handle `ChangeMessageVisibility`.
    #[allow(clippy::cast_sign_loss)]
    fn handle_change_visibility(
        &mut self,
        receipt_handle: &str,
        visibility_timeout: i32,
    ) -> Result<bool, SqsError> {
        match &mut self.storage {
            QueueStorage::Standard(storage) => {
                if visibility_timeout == 0 {
                    if let Some(ifm) = storage.in_flight.remove(receipt_handle) {
                        storage.available.push_back(ifm.message);
                        Ok(true)
                    } else {
                        Err(SqsError::new(
                            rustack_sqs_model::error::SqsErrorCode::MessageNotInflight,
                            "Message does not exist or is not available for visibility timeout \
                             change.",
                        ))
                    }
                } else if let Some(ifm) = storage.in_flight.get_mut(receipt_handle) {
                    ifm.visible_at =
                        Instant::now() + Duration::from_secs(visibility_timeout as u64);
                    Ok(false)
                } else {
                    Err(SqsError::new(
                        rustack_sqs_model::error::SqsErrorCode::MessageNotInflight,
                        "Message does not exist or is not available for visibility timeout change.",
                    ))
                }
            }
            QueueStorage::Fifo(storage) => {
                let visible_at = if visibility_timeout == 0 {
                    Instant::now() // Immediately visible
                } else {
                    Instant::now() + Duration::from_secs(visibility_timeout as u64)
                };
                if storage.change_visibility(receipt_handle, visible_at) {
                    Ok(visibility_timeout == 0)
                } else {
                    Err(SqsError::new(
                        rustack_sqs_model::error::SqsErrorCode::MessageNotInflight,
                        "Message does not exist or is not available for visibility timeout change.",
                    ))
                }
            }
        }
    }

    /// Handle `PurgeQueue`.
    fn handle_purge(&mut self) -> Result<(), SqsError> {
        if let Some(last_purge) = self.last_purge_at {
            if last_purge.elapsed() < Duration::from_mins(1) {
                return Err(SqsError::purge_queue_in_progress());
            }
        }
        self.redrive.pending.clear();
        match &mut self.storage {
            QueueStorage::Standard(s) => s.purge(),
            QueueStorage::Fifo(s) => s.purge(),
        }
        self.last_purge_at = Some(Instant::now());
        Ok(())
    }

    /// Periodic cleanup: expired visibility, delayed message promotion, dedup cache.
    fn periodic_cleanup(&mut self) {
        let changed = match &mut self.storage {
            QueueStorage::Standard(storage) => {
                let returned = storage.return_expired_inflight();
                let promoted = storage.promote_delayed();
                returned || promoted
            }
            QueueStorage::Fifo(storage) => {
                let returned = storage.return_expired_inflight();
                storage.clean_dedup_cache();
                returned
            }
        };

        if changed && !self.pending_long_polls.is_empty() {
            self.fulfill_pending_long_polls();
        }

        self.expire_long_polls();
    }

    /// Fulfill pending long-poll requests that now have messages.
    fn fulfill_pending_long_polls(&mut self) {
        let polls = std::mem::take(&mut self.pending_long_polls);
        let mut remaining = Vec::new();

        for poll in polls {
            if poll.reply.is_closed() {
                continue;
            }

            let messages = self.try_receive(
                poll.max_messages,
                poll.visibility_timeout,
                &poll.attribute_names,
                &[],
                &poll.message_attribute_names,
            );

            if messages.is_empty() {
                remaining.push(poll);
            } else {
                let _ = poll.reply.send(Ok(ReceiveMessageOutput { messages }));
            }
        }

        self.pending_long_polls = remaining;
    }

    /// Expire long polls that have exceeded their deadline.
    fn expire_long_polls(&mut self) {
        let now = Instant::now();
        let polls = std::mem::take(&mut self.pending_long_polls);
        let mut remaining = Vec::new();

        for poll in polls {
            if poll.reply.is_closed() {
                continue;
            }
            if now >= poll.deadline {
                let _ = poll.reply.send(Ok(ReceiveMessageOutput {
                    messages: Vec::new(),
                }));
            } else {
                remaining.push(poll);
            }
        }

        self.pending_long_polls = remaining;
    }
}

// ---------------------------------------------------------------------------
// Standard queue receive helper
// ---------------------------------------------------------------------------

struct RedriveRequest<'a> {
    manager: &'a mut RedriveManager,
    attributes: &'a QueueAttributes,
    source_arn: &'a str,
}

impl RedriveRequest<'_> {
    fn target(&self, message: &QueueMessage) -> Option<String> {
        self.attributes
            .redrive_policy
            .as_ref()
            .filter(|policy| {
                message.approximate_receive_count
                    >= u32::try_from(policy.max_receive_count).unwrap_or(u32::MAX)
            })
            .map(|policy| policy.dead_letter_target_arn.clone())
    }
}

/// Receive messages from a standard queue.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn try_receive_standard(
    storage: &mut StandardQueueStorage,
    max: usize,
    vis_timeout: Duration,
    sys_attrs: &[String],
    msg_attrs: &[String],
    redrive: &mut RedriveRequest<'_>,
) -> Vec<Message> {
    let mut result = Vec::new();

    let scan_budget = storage.available.len().min(128);
    for _ in 0..scan_budget {
        if result.len() >= max {
            break;
        }
        match storage.available.pop_front() {
            Some(mut msg) => {
                if let Some(target) = redrive.target(&msg) {
                    if !redrive.manager.handoff(&msg, &target, redrive.source_arn) {
                        // Standard queues have no group ordering constraint: retain
                        // the poison message without starving healthy messages.
                        storage.available.push_back(msg);
                    }
                    continue;
                }
                msg.approximate_receive_count = msg.approximate_receive_count.saturating_add(1);
                if msg.approximate_first_receive_timestamp.is_none() {
                    msg.approximate_first_receive_timestamp = Some(now_epoch_millis());
                }

                let receipt_handle = generate_receipt_handle(&msg.message_id);
                let message = build_message(&msg, &receipt_handle, sys_attrs, msg_attrs);

                storage.in_flight.insert(
                    receipt_handle,
                    InFlightMessage {
                        message: msg,
                        receipt_handle: message.receipt_handle.clone().unwrap_or_default(),
                        visible_at: Instant::now() + vis_timeout,
                    },
                );

                result.push(message);
            }
            None => break,
        }
    }
    result
}

// ---------------------------------------------------------------------------
// FIFO queue receive helper
// ---------------------------------------------------------------------------

/// Receive messages from a FIFO queue.
fn try_receive_fifo(
    storage: &mut FifoQueueStorage,
    max: usize,
    vis_timeout: Duration,
    sys_attrs: &[String],
    msg_attrs: &[String],
    redrive: &mut RedriveRequest<'_>,
) -> Vec<Message> {
    let received = storage.receive(max);
    let mut result = Vec::new();

    for (mut msg, group_id) in received {
        if let Some(target) = redrive.target(&msg) {
            if !redrive.manager.handoff(&msg, &target, redrive.source_arn) {
                storage.finish_redrive(&group_id, Some(msg));
            }
            continue;
        }
        msg.approximate_receive_count = msg.approximate_receive_count.saturating_add(1);
        if msg.approximate_first_receive_timestamp.is_none() {
            msg.approximate_first_receive_timestamp = Some(now_epoch_millis());
        }

        let receipt_handle = generate_receipt_handle(&msg.message_id);
        let message = build_message(&msg, &receipt_handle, sys_attrs, msg_attrs);

        storage.mark_in_flight(receipt_handle, msg, group_id, Instant::now() + vis_timeout);

        result.push(message);
    }

    result
}

// ---------------------------------------------------------------------------
// QueueHandle
// ---------------------------------------------------------------------------

/// Handle to a running queue actor.
pub struct QueueHandle {
    /// Channel to send commands to the queue actor.
    pub sender: mpsc::Sender<QueueCommand>,
    /// Queue metadata (read-only after creation).
    pub metadata: QueueMetadata,
    /// Supervisor-observed actor completion, including panic status.
    pub completion: watch::Receiver<Option<bool>>,
    /// Shutdown flag.
    pub shutdown: Arc<AtomicBool>,
}

impl std::fmt::Debug for QueueHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueueHandle")
            .field("metadata", &self.metadata)
            .finish_non_exhaustive()
    }
}

impl QueueHandle {
    /// Send a message.
    pub async fn send_message(
        &self,
        input: SendMessageInput,
    ) -> Result<SendMessageOutput, SqsError> {
        let (tx, rx) = oneshot::channel();
        self.sender
            .send(QueueCommand::SendMessage { input, reply: tx })
            .await
            .map_err(|_| SqsError::internal_error("Queue actor is not running"))?;
        rx.await
            .map_err(|_| SqsError::internal_error("Queue actor dropped reply channel"))?
    }

    /// Receive messages.
    pub async fn receive_message(
        &self,
        input: ReceiveMessageInput,
    ) -> Result<ReceiveMessageOutput, SqsError> {
        let (tx, rx) = oneshot::channel();
        self.sender
            .send(QueueCommand::ReceiveMessage { input, reply: tx })
            .await
            .map_err(|_| SqsError::internal_error("Queue actor is not running"))?;
        rx.await
            .map_err(|_| SqsError::internal_error("Queue actor dropped reply channel"))?
    }

    /// Delete a message.
    pub async fn delete_message(&self, receipt_handle: String) -> Result<(), SqsError> {
        let (tx, rx) = oneshot::channel();
        self.sender
            .send(QueueCommand::DeleteMessage {
                receipt_handle,
                reply: tx,
            })
            .await
            .map_err(|_| SqsError::internal_error("Queue actor is not running"))?;
        rx.await
            .map_err(|_| SqsError::internal_error("Queue actor dropped reply channel"))?
    }

    /// Change message visibility.
    pub async fn change_visibility(
        &self,
        receipt_handle: String,
        visibility_timeout: i32,
    ) -> Result<(), SqsError> {
        let (tx, rx) = oneshot::channel();
        self.sender
            .send(QueueCommand::ChangeVisibility {
                receipt_handle,
                visibility_timeout,
                reply: tx,
            })
            .await
            .map_err(|_| SqsError::internal_error("Queue actor is not running"))?;
        rx.await
            .map_err(|_| SqsError::internal_error("Queue actor dropped reply channel"))?
    }

    /// Get queue attributes.
    pub async fn get_attributes(
        &self,
        attribute_names: Vec<String>,
    ) -> Result<HashMap<String, String>, SqsError> {
        let (tx, rx) = oneshot::channel();
        self.sender
            .send(QueueCommand::GetAttributes {
                attribute_names,
                reply: tx,
            })
            .await
            .map_err(|_| SqsError::internal_error("Queue actor is not running"))?;
        rx.await
            .map_err(|_| SqsError::internal_error("Queue actor dropped reply channel"))
    }

    /// Set queue attributes.
    pub async fn set_attributes(
        &self,
        attributes: HashMap<String, String>,
    ) -> Result<(), SqsError> {
        let (tx, rx) = oneshot::channel();
        self.sender
            .send(QueueCommand::SetAttributes {
                attributes,
                reply: tx,
            })
            .await
            .map_err(|_| SqsError::internal_error("Queue actor is not running"))?;
        rx.await
            .map_err(|_| SqsError::internal_error("Queue actor dropped reply channel"))?
    }

    /// Purge the queue.
    pub async fn purge(&self) -> Result<(), SqsError> {
        let (tx, rx) = oneshot::channel();
        self.sender
            .send(QueueCommand::Purge { reply: tx })
            .await
            .map_err(|_| SqsError::internal_error("Queue actor is not running"))?;
        rx.await
            .map_err(|_| SqsError::internal_error("Queue actor dropped reply channel"))?
    }

    /// Get tags.
    pub async fn get_tags(&self) -> Result<HashMap<String, String>, SqsError> {
        let (tx, rx) = oneshot::channel();
        self.sender
            .send(QueueCommand::GetTags { reply: tx })
            .await
            .map_err(|_| SqsError::internal_error("Queue actor is not running"))?;
        rx.await
            .map_err(|_| SqsError::internal_error("Queue actor dropped reply channel"))
    }

    /// Set tags.
    pub async fn set_tags(&self, tags: HashMap<String, String>) -> Result<(), SqsError> {
        let (tx, rx) = oneshot::channel();
        self.sender
            .send(QueueCommand::SetTags { tags, reply: tx })
            .await
            .map_err(|_| SqsError::internal_error("Queue actor is not running"))?;
        let _ = rx.await;
        Ok(())
    }

    /// Remove tags.
    pub async fn remove_tags(&self, keys: Vec<String>) -> Result<(), SqsError> {
        let (tx, rx) = oneshot::channel();
        self.sender
            .send(QueueCommand::RemoveTags { keys, reply: tx })
            .await
            .map_err(|_| SqsError::internal_error("Queue actor is not running"))?;
        let _ = rx.await;
        Ok(())
    }

    /// Stop initiating redrive and wait for all accepted handoffs without clearing messages.
    ///
    /// # Errors
    /// Returns an error if the actor is closed or a simultaneous quiesce is in progress.
    pub async fn quiesce(&self) -> Result<(), SqsError> {
        let (reply, receiver) = oneshot::channel();
        self.sender
            .send(QueueCommand::Quiesce { reply })
            .await
            .map_err(|_| SqsError::internal_error("Queue actor is not running"))?;
        receiver
            .await
            .map_err(|_| SqsError::internal_error("Queue quiesce did not complete"))
    }

    /// Shutdown the queue actor.
    pub async fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let _ = self.sender.send(QueueCommand::Shutdown).await;
        let mut completion = self.completion.clone();
        loop {
            if let Some(success) = *completion.borrow_and_update() {
                if !success {
                    tracing::error!("SQS actor shutdown observed a panic");
                }
                return;
            }
            if completion.changed().await.is_err() {
                return;
            }
        }
    }
}

/// Queue metadata (read-only after creation).
#[derive(Debug, Clone)]
pub struct QueueMetadata {
    /// Queue name.
    pub name: String,
    /// Queue URL.
    pub url: String,
    /// Queue ARN.
    pub arn: String,
    /// Whether this is a FIFO queue.
    pub is_fifo: bool,
    /// Creation timestamp (epoch seconds).
    pub created_at: u64,
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Build a `Message` response from internal queue message and receipt handle.
fn build_message(
    msg: &QueueMessage,
    receipt_handle: &str,
    system_attr_names: &[String],
    message_attr_names: &[String],
) -> Message {
    let want_all_sys = system_attr_names.iter().any(|n| n == "All");
    let want_sys = |name: &str| want_all_sys || system_attr_names.iter().any(|n| n == name);

    let mut attributes = HashMap::new();
    if want_sys("DeadLetterQueueSourceArn") {
        if let Some(arn) = &msg.dead_letter_queue_source_arn {
            attributes.insert("DeadLetterQueueSourceArn".to_owned(), arn.clone());
        }
    }
    if want_sys("SenderId") {
        attributes.insert("SenderId".to_owned(), msg.sender_id.clone());
    }
    if want_sys("SentTimestamp") {
        attributes.insert("SentTimestamp".to_owned(), msg.sent_timestamp.to_string());
    }
    if want_sys("ApproximateReceiveCount") {
        attributes.insert(
            "ApproximateReceiveCount".to_owned(),
            msg.approximate_receive_count.to_string(),
        );
    }
    if want_sys("ApproximateFirstReceiveTimestamp") {
        if let Some(ts) = msg.approximate_first_receive_timestamp {
            attributes.insert(
                "ApproximateFirstReceiveTimestamp".to_owned(),
                ts.to_string(),
            );
        }
    }
    if want_sys("MessageGroupId") {
        if let Some(ref gid) = msg.message_group_id {
            attributes.insert("MessageGroupId".to_owned(), gid.clone());
        }
    }
    if want_sys("MessageDeduplicationId") {
        if let Some(ref did) = msg.message_deduplication_id {
            attributes.insert("MessageDeduplicationId".to_owned(), did.clone());
        }
    }
    if want_sys("SequenceNumber") {
        if let Some(ref sn) = msg.sequence_number {
            attributes.insert("SequenceNumber".to_owned(), sn.clone());
        }
    }

    // Filter user message attributes.
    // Per AWS spec: if no MessageAttributeNames are specified, no user attributes are returned.
    // Only return all when explicitly requested with "All" or ".*".
    let want_all_msg = message_attr_names.iter().any(|n| n == "All" || n == ".*");
    let filtered_attrs = if message_attr_names.is_empty() {
        HashMap::new()
    } else if want_all_msg {
        msg.message_attributes.clone()
    } else {
        msg.message_attributes
            .iter()
            .filter(|(k, _)| message_attr_names.iter().any(|n| n == *k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    };

    Message {
        message_id: Some(msg.message_id.clone()),
        receipt_handle: Some(receipt_handle.to_owned()),
        body: Some(msg.body.clone()),
        md5_of_body: Some(msg.md5_of_body.clone()),
        md5_of_message_attributes: msg.md5_of_message_attributes.clone(),
        message_attributes: filtered_attrs,
        attributes,
    }
}

/// Merge the deprecated `AttributeNames` with the newer `MessageSystemAttributeNames`.
fn merge_attribute_names(old: &[String], new: &[String]) -> Vec<String> {
    let mut merged = old.to_vec();
    for name in new {
        if !merged.contains(name) {
            merged.push(name.clone());
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rustack_sqs_model::input::{ReceiveMessageInput, SendMessageInput};
    use tokio::sync::{mpsc, oneshot};

    use super::{QueueActor, QueueCommand};
    use crate::queue::attributes::QueueAttributes;

    fn test_actor() -> QueueActor {
        let (_sender, receiver) = mpsc::channel(8);
        QueueActor::new(
            "test-queue".to_owned(),
            "arn:aws:sqs:us-east-1:000000000000:test-queue".to_owned(),
            false,
            QueueAttributes::default(),
            receiver,
            HashMap::new(),
            "000000000000".to_owned(),
            0,
        )
    }

    fn receive_input(wait_time_seconds: i32) -> ReceiveMessageInput {
        ReceiveMessageInput {
            queue_url: "http://localhost:4566/000000000000/test-queue".to_owned(),
            wait_time_seconds: Some(wait_time_seconds),
            ..ReceiveMessageInput::default()
        }
    }

    fn send_input(delay_seconds: Option<i32>) -> SendMessageInput {
        SendMessageInput {
            queue_url: "http://localhost:4566/000000000000/test-queue".to_owned(),
            message_body: "test body".to_owned(),
            delay_seconds,
            ..SendMessageInput::default()
        }
    }

    #[test]
    fn receive_long_poll_registration_does_not_request_fulfill() {
        let mut actor = test_actor();
        let (reply, mut response) = oneshot::channel();

        let should_fulfill = actor.handle_command(QueueCommand::ReceiveMessage {
            input: receive_input(20),
            reply,
        });

        assert!(!should_fulfill);
        assert_eq!(actor.pending_long_polls.len(), 1);
        assert!(response.try_recv().is_err());
    }

    #[test]
    fn standard_send_wakeup_decision_tracks_immediate_visibility() {
        let mut actor = test_actor();
        let (reply, mut response) = oneshot::channel();

        let should_fulfill = actor.handle_command(QueueCommand::SendMessage {
            input: send_input(None),
            reply,
        });

        assert!(should_fulfill);
        assert!(
            response
                .try_recv()
                .expect("send command should reply")
                .is_ok()
        );

        let (reply, mut response) = oneshot::channel();
        let should_fulfill = actor.handle_command(QueueCommand::SendMessage {
            input: send_input(Some(5)),
            reply,
        });

        assert!(!should_fulfill);
        assert!(
            response
                .try_recv()
                .expect("delayed send command should reply")
                .is_ok()
        );
    }

    #[test]
    fn immediate_send_fulfills_existing_long_poll() {
        let mut actor = test_actor();
        let (receive_reply, mut receive_response) = oneshot::channel();

        let should_fulfill = actor.handle_command(QueueCommand::ReceiveMessage {
            input: receive_input(20),
            reply: receive_reply,
        });

        assert!(!should_fulfill);
        assert_eq!(actor.pending_long_polls.len(), 1);

        let (send_reply, mut send_response) = oneshot::channel();
        let should_fulfill = actor.handle_command(QueueCommand::SendMessage {
            input: send_input(None),
            reply: send_reply,
        });
        assert!(should_fulfill);
        assert!(
            send_response
                .try_recv()
                .expect("send command should reply")
                .is_ok()
        );

        actor.fulfill_pending_long_polls();

        assert!(actor.pending_long_polls.is_empty());
        let output = receive_response
            .try_recv()
            .expect("long poll should receive a response")
            .expect("long poll should succeed");
        assert_eq!(output.messages.len(), 1);
        assert_eq!(output.messages[0].body.as_deref(), Some("test body"));
    }
}
