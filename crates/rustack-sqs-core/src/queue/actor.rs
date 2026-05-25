//! Queue actor: per-queue message lifecycle management.
//!
//! Each queue runs as an independent actor that owns all its state and
//! communicates via a `tokio::sync::mpsc` channel. The actor supports both
//! standard and FIFO queue types.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use rustack_sqs_model::{
    error::SqsError,
    input::{ReceiveMessageInput, SendMessageInput},
    output::{ReceiveMessageOutput, SendMessageOutput},
    types::Message,
};
use tokio::{
    sync::{mpsc, oneshot},
    time::Instant,
};

use super::{
    attributes::QueueAttributes,
    storage::{EnqueueResult, FifoQueueStorage, StandardQueueStorage},
};
use crate::message::{
    InFlightMessage, QueueMessage, generate_receipt_handle, md5_of_body, md5_of_message_attributes,
    now_epoch_millis,
};

/// Commands sent to a queue actor via its channel.
pub enum QueueCommand {
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
        }
    }

    /// Run the actor event loop.
    pub async fn run(mut self) {
        let mut cleanup_interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            // Compute the earliest long-poll deadline so we can expire it precisely
            // instead of waiting for the next 1-second cleanup tick.
            let next_poll_deadline = self
                .pending_long_polls
                .iter()
                .map(|p| p.deadline)
                .min()
                .unwrap_or_else(|| Instant::now() + Duration::from_hours(24));

            tokio::select! {
                Some(cmd) = self.commands.recv() => {
                    match cmd {
                        QueueCommand::Shutdown => break,
                        cmd => self.handle_command(cmd),
                    }
                    // After any command that may have enqueued messages,
                    // try to fulfill waiting long-poll receivers directly.
                    if !self.pending_long_polls.is_empty() {
                        self.fulfill_pending_long_polls();
                    }
                }
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
    fn handle_command(&mut self, cmd: QueueCommand) {
        match cmd {
            QueueCommand::SendMessage { input, reply } => {
                let result = self.handle_send_message(input);
                let _ = reply.send(result);
            }
            QueueCommand::ReceiveMessage { input, reply } => {
                self.handle_receive_message(input, reply);
            }
            QueueCommand::DeleteMessage {
                receipt_handle,
                reply,
            } => {
                let result = self.handle_delete_message(&receipt_handle);
                let _ = reply.send(result);
            }
            QueueCommand::ChangeVisibility {
                receipt_handle,
                visibility_timeout,
                reply,
            } => {
                let result = self.handle_change_visibility(&receipt_handle, visibility_timeout);
                let _ = reply.send(result);
            }
            QueueCommand::GetAttributes {
                attribute_names,
                reply,
            } => {
                let counts = match &self.storage {
                    QueueStorage::Standard(s) => s.counts(),
                    QueueStorage::Fifo(s) => s.counts(),
                };
                let attrs = self.attributes.to_map(
                    &attribute_names,
                    self.is_fifo,
                    &self.arn,
                    self.created_at,
                    self.last_modified_at,
                    counts,
                );
                let _ = reply.send(attrs);
            }
            QueueCommand::SetAttributes { attributes, reply } => {
                let result = self.attributes.update_from_map(&attributes, self.is_fifo);
                if result.is_ok() {
                    self.last_modified_at = crate::message::now_epoch_seconds();
                }
                let _ = reply.send(result);
            }
            QueueCommand::Purge { reply } => {
                let result = self.handle_purge();
                let _ = reply.send(result);
            }
            QueueCommand::GetTags { reply } => {
                let _ = reply.send(self.tags.clone());
            }
            QueueCommand::SetTags { tags, reply } => {
                self.tags.extend(tags);
                let _ = reply.send(());
            }
            QueueCommand::RemoveTags { keys, reply } => {
                for key in &keys {
                    self.tags.remove(key);
                }
                let _ = reply.send(());
            }
            QueueCommand::Shutdown => {
                // Handled in the event loop.
            }
        }
    }

    /// Handle `SendMessage`.
    #[allow(clippy::cast_sign_loss)]
    fn handle_send_message(
        &mut self,
        input: SendMessageInput,
    ) -> Result<SendMessageOutput, SqsError> {
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
    ) -> Result<SendMessageOutput, SqsError> {
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

        Ok(SendMessageOutput {
            message_id: Some(message_id),
            md5_of_message_body: Some(body_md5),
            md5_of_message_attributes: attr_md5,
            md5_of_message_system_attributes: None,
            sequence_number: None,
        })
    }

    /// Send a message to a FIFO queue with deduplication and sequencing.
    fn handle_send_message_fifo(
        &mut self,
        input: SendMessageInput,
    ) -> Result<SendMessageOutput, SqsError> {
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
        let effective_dedup_key = if self.attributes.deduplication_scope == "messageGroup" {
            format!("{group_id}:{dedup_id}")
        } else {
            dedup_id
        };

        let message_id = uuid::Uuid::new_v4().to_string();
        let body_md5 = md5_of_body(&input.message_body);
        let attr_md5 = md5_of_message_attributes(&input.message_attributes);

        let msg = QueueMessage {
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
            message_deduplication_id: input.message_deduplication_id,
            available_at: Instant::now(),
            delay_seconds: 0,
        };

        let enqueue_result = storage.enqueue(msg, &effective_dedup_key);

        // On dedup, return the original message's ID and sequence number per AWS spec.
        match enqueue_result {
            EnqueueResult::Enqueued {
                message_id: mid,
                sequence_number,
            }
            | EnqueueResult::Deduplicated {
                message_id: mid,
                sequence_number,
            } => Ok(SendMessageOutput {
                message_id: Some(mid),
                md5_of_message_body: Some(body_md5),
                md5_of_message_attributes: attr_md5,
                md5_of_message_system_attributes: None,
                sequence_number: Some(sequence_number),
            }),
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
        let merged_sys_attrs = merge_attribute_names(attribute_names, system_attribute_names);
        let vis_timeout = Duration::from_secs(visibility_timeout as u64);

        match &mut self.storage {
            QueueStorage::Standard(storage) => try_receive_standard(
                storage,
                max_messages as usize,
                vis_timeout,
                &merged_sys_attrs,
                message_attribute_names,
                &self.attributes,
            ),
            QueueStorage::Fifo(storage) => try_receive_fifo(
                storage,
                max_messages as usize,
                vis_timeout,
                &merged_sys_attrs,
                message_attribute_names,
            ),
        }
    }

    /// Handle `DeleteMessage`.
    #[allow(clippy::unnecessary_wraps)]
    fn handle_delete_message(&mut self, receipt_handle: &str) -> Result<(), SqsError> {
        match &mut self.storage {
            QueueStorage::Standard(storage) => {
                storage.in_flight.remove(receipt_handle);
            }
            QueueStorage::Fifo(storage) => {
                storage.delete_message(receipt_handle);
            }
        }
        // AWS SQS is lenient: delete of non-existent receipt handle succeeds.
        Ok(())
    }

    /// Handle `ChangeMessageVisibility`.
    #[allow(clippy::cast_sign_loss)]
    fn handle_change_visibility(
        &mut self,
        receipt_handle: &str,
        visibility_timeout: i32,
    ) -> Result<(), SqsError> {
        match &mut self.storage {
            QueueStorage::Standard(storage) => {
                if let Some(ifm) = storage.in_flight.get_mut(receipt_handle) {
                    if visibility_timeout == 0 {
                        let ifm = storage.in_flight.remove(receipt_handle).unwrap();
                        storage.available.push_back(ifm.message);

                    } else {
                        ifm.visible_at =
                            Instant::now() + Duration::from_secs(visibility_timeout as u64);
                    }
                    Ok(())
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
                    if visibility_timeout == 0 {

                    }
                    Ok(())
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

/// Receive messages from a standard queue.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn try_receive_standard(
    storage: &mut StandardQueueStorage,
    max: usize,
    vis_timeout: Duration,
    sys_attrs: &[String],
    msg_attrs: &[String],
    queue_attrs: &QueueAttributes,
) -> Vec<Message> {
    let mut result = Vec::new();

    while result.len() < max {
        match storage.available.pop_front() {
            Some(mut msg) => {
                msg.approximate_receive_count += 1;
                if msg.approximate_first_receive_timestamp.is_none() {
                    msg.approximate_first_receive_timestamp = Some(now_epoch_millis());
                }

                // Check DLQ redrive threshold.
                if let Some(ref policy) = queue_attrs.redrive_policy {
                    #[allow(clippy::cast_sign_loss)]
                    if msg.approximate_receive_count > policy.max_receive_count as u32 {
                        // Move to dead_letters storage instead of silently dropping.
                        // Actual DLQ routing (cross-queue send) can be added later.
                        tracing::debug!(
                            message_id = %msg.message_id,
                            receive_count = msg.approximate_receive_count,
                            "message exceeded maxReceiveCount, moved to dead letters"
                        );
                        storage.dead_letters.push(msg);
                        continue;
                    }
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
) -> Vec<Message> {
    let received = storage.receive(max);
    let mut result = Vec::new();

    for (mut msg, group_id) in received {
        msg.approximate_receive_count += 1;
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
    /// Actor task join handle.
    pub task: tokio::task::JoinHandle<()>,
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

    /// Shutdown the queue actor.
    pub async fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let _ = self.sender.send(QueueCommand::Shutdown).await;
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
