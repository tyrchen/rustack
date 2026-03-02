//! Queue message storage for both standard and FIFO queues.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::time::Instant;

use crate::message::{InFlightMessage, QueueMessage};

/// Standard queue storage with best-effort ordering and at-least-once delivery.
#[derive(Debug, Default)]
pub struct StandardQueueStorage {
    /// Available messages in approximate FIFO order.
    pub available: VecDeque<QueueMessage>,
    /// Messages not yet available (per-message or queue-level delay).
    pub delayed: Vec<QueueMessage>,
    /// Messages currently being processed by consumers.
    pub in_flight: HashMap<String, InFlightMessage>,
    /// Messages that exceeded the DLQ `maxReceiveCount` threshold.
    /// Stored here until actual DLQ routing is implemented.
    pub dead_letters: Vec<QueueMessage>,
}

impl StandardQueueStorage {
    /// Promote delayed messages that have become available.
    pub fn promote_delayed(&mut self) -> bool {
        let now = Instant::now();
        let mut promoted = false;

        let old_delayed = std::mem::take(&mut self.delayed);
        for msg in old_delayed {
            if msg.available_at <= now {
                self.available.push_back(msg);
                promoted = true;
            } else {
                self.delayed.push(msg);
            }
        }

        promoted
    }

    /// Return expired in-flight messages to the available queue.
    pub fn return_expired_inflight(&mut self) -> bool {
        let now = Instant::now();
        let expired: Vec<String> = self
            .in_flight
            .iter()
            .filter(|(_, ifm)| ifm.visible_at <= now)
            .map(|(handle, _)| handle.clone())
            .collect();

        let had_expired = !expired.is_empty();
        for handle in expired {
            if let Some(ifm) = self.in_flight.remove(&handle) {
                self.available.push_back(ifm.message);
            }
        }
        had_expired
    }

    /// Get approximate message counts: (available, in_flight, delayed).
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn counts(&self) -> (u32, u32, u32) {
        (
            self.available.len() as u32,
            self.in_flight.len() as u32,
            self.delayed.len() as u32,
        )
    }

    /// Purge all messages.
    pub fn purge(&mut self) {
        self.available.clear();
        self.delayed.clear();
        self.in_flight.clear();
        self.dead_letters.clear();
    }
}

/// Cached information for a deduplicated message.
#[derive(Debug, Clone)]
pub struct DedupCacheEntry {
    /// When this dedup entry expires.
    pub expiry: Instant,
    /// The original message ID (returned on duplicate sends).
    pub message_id: String,
    /// The original sequence number (returned on duplicate sends).
    pub sequence_number: String,
}

/// FIFO queue storage with strict ordering, message group blocking, and deduplication.
#[derive(Debug)]
pub struct FifoQueueStorage {
    /// Per-group message queues (strict FIFO per group).
    groups: BTreeMap<String, VecDeque<QueueMessage>>,
    /// Groups with in-flight messages (blocked from delivery).
    blocked_groups: HashSet<String>,
    /// Messages currently being processed by consumers.
    in_flight: HashMap<String, FifoInFlightMessage>,
    /// Deduplication cache: effective_dedup_key -> original message info.
    dedup_cache: HashMap<String, DedupCacheEntry>,
    /// Monotonically increasing sequence number.
    next_sequence: AtomicU64,
}

/// In-flight message for FIFO queues, tracking the message group.
#[derive(Debug)]
pub struct FifoInFlightMessage {
    /// The underlying message.
    pub message: QueueMessage,
    /// When visibility timeout expires.
    pub visible_at: Instant,
    /// The message group this message belongs to.
    pub group_id: String,
}

/// Deduplication window (5 minutes).
const DEDUP_WINDOW: std::time::Duration = std::time::Duration::from_mins(5);

impl Default for FifoQueueStorage {
    fn default() -> Self {
        Self {
            groups: BTreeMap::new(),
            blocked_groups: HashSet::new(),
            in_flight: HashMap::new(),
            dedup_cache: HashMap::new(),
            next_sequence: AtomicU64::new(1),
        }
    }
}

/// Result of an enqueue attempt on a FIFO queue.
#[derive(Debug, Clone)]
pub enum EnqueueResult {
    /// Message was enqueued with a new sequence number.
    Enqueued {
        /// The message ID.
        message_id: String,
        /// The assigned sequence number.
        sequence_number: String,
    },
    /// Message was a duplicate; returns the original message's info.
    Deduplicated {
        /// The original message ID.
        message_id: String,
        /// The original sequence number.
        sequence_number: String,
    },
}

impl FifoQueueStorage {
    /// Attempt to enqueue a message with deduplication.
    ///
    /// The `effective_dedup_key` should already incorporate the dedup scope
    /// (e.g., prefixed with group ID when scope is `messageGroup`).
    pub fn enqueue(&mut self, mut msg: QueueMessage, effective_dedup_key: &str) -> EnqueueResult {
        // Check dedup cache.
        if let Some(entry) = self.dedup_cache.get(effective_dedup_key) {
            if Instant::now() < entry.expiry {
                // Duplicate within window: return original message info.
                return EnqueueResult::Deduplicated {
                    message_id: entry.message_id.clone(),
                    sequence_number: entry.sequence_number.clone(),
                };
            }
        }

        // Assign sequence number.
        let seq = self.next_sequence.fetch_add(1, Ordering::Relaxed);
        let seq_str = format!("{seq:020}");
        msg.sequence_number = Some(seq_str.clone());

        let message_id = msg.message_id.clone();

        // Add to dedup cache with original message info.
        self.dedup_cache.insert(
            effective_dedup_key.to_owned(),
            DedupCacheEntry {
                expiry: Instant::now() + DEDUP_WINDOW,
                message_id: message_id.clone(),
                sequence_number: seq_str.clone(),
            },
        );

        // Enqueue to the appropriate group.
        let group_id = msg.message_group_id.clone().unwrap_or_default();
        self.groups.entry(group_id).or_default().push_back(msg);

        EnqueueResult::Enqueued {
            message_id,
            sequence_number: seq_str,
        }
    }

    /// Try to receive up to `max` messages from unblocked groups.
    pub fn receive(&mut self, max: usize) -> Vec<(QueueMessage, String)> {
        let mut result = Vec::new();

        // Iterate through groups in order, skipping blocked ones.
        let group_keys: Vec<String> = self.groups.keys().cloned().collect();
        for group_id in group_keys {
            if result.len() >= max {
                break;
            }
            if self.blocked_groups.contains(&group_id) {
                continue;
            }

            if let Some(queue) = self.groups.get_mut(&group_id) {
                if let Some(msg) = queue.pop_front() {
                    // Block this group until the message is deleted or visibility expires.
                    self.blocked_groups.insert(group_id.clone());
                    result.push((msg, group_id));
                }
            }
        }

        // Clean up empty groups.
        self.groups.retain(|_, q| !q.is_empty());

        result
    }

    /// Record a message as in-flight.
    pub fn mark_in_flight(
        &mut self,
        receipt_handle: String,
        message: QueueMessage,
        group_id: String,
        visible_at: Instant,
    ) {
        self.in_flight.insert(
            receipt_handle,
            FifoInFlightMessage {
                message,
                visible_at,
                group_id,
            },
        );
    }

    /// Delete an in-flight message (unblocks the group).
    pub fn delete_message(&mut self, receipt_handle: &str) -> bool {
        if let Some(ifm) = self.in_flight.remove(receipt_handle) {
            // Unblock the group only if no other in-flight messages for this group exist.
            let group_still_has_inflight =
                self.in_flight.values().any(|m| m.group_id == ifm.group_id);
            if !group_still_has_inflight {
                self.blocked_groups.remove(&ifm.group_id);
            }
            true
        } else {
            false
        }
    }

    /// Change visibility timeout for an in-flight message.
    pub fn change_visibility(&mut self, receipt_handle: &str, visible_at: Instant) -> bool {
        if visible_at <= Instant::now() {
            // Make immediately available again.
            if let Some(ifm) = self.in_flight.remove(receipt_handle) {
                let group_id = ifm.group_id.clone();
                self.groups
                    .entry(group_id.clone())
                    .or_default()
                    .push_front(ifm.message);
                // Unblock group if no other in-flight messages.
                let group_still_has_inflight =
                    self.in_flight.values().any(|m| m.group_id == group_id);
                if !group_still_has_inflight {
                    self.blocked_groups.remove(&group_id);
                }
                return true;
            }
            false
        } else if let Some(ifm) = self.in_flight.get_mut(receipt_handle) {
            ifm.visible_at = visible_at;
            true
        } else {
            false
        }
    }

    /// Return expired in-flight messages to their groups (unblocks groups).
    pub fn return_expired_inflight(&mut self) -> bool {
        let now = Instant::now();
        let expired: Vec<String> = self
            .in_flight
            .iter()
            .filter(|(_, ifm)| ifm.visible_at <= now)
            .map(|(handle, _)| handle.clone())
            .collect();

        let had_expired = !expired.is_empty();
        for handle in expired {
            if let Some(ifm) = self.in_flight.remove(&handle) {
                let group_id = ifm.group_id.clone();
                // Return to front of group queue (maintains FIFO).
                self.groups
                    .entry(group_id.clone())
                    .or_default()
                    .push_front(ifm.message);
                // Unblock group if no other in-flight messages.
                let group_still_has_inflight =
                    self.in_flight.values().any(|m| m.group_id == group_id);
                if !group_still_has_inflight {
                    self.blocked_groups.remove(&group_id);
                }
            }
        }
        had_expired
    }

    /// Clean expired dedup cache entries.
    pub fn clean_dedup_cache(&mut self) {
        let now = Instant::now();
        self.dedup_cache.retain(|_, entry| entry.expiry > now);
    }

    /// Get approximate message counts: (available, in_flight, delayed=0).
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn counts(&self) -> (u32, u32, u32) {
        let available: usize = self.groups.values().map(VecDeque::len).sum();
        (available as u32, self.in_flight.len() as u32, 0)
    }

    /// Purge all messages.
    pub fn purge(&mut self) {
        self.groups.clear();
        self.blocked_groups.clear();
        self.in_flight.clear();
    }
}
