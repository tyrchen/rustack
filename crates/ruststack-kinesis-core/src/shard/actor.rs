//! Shard actor implementing the actor-per-shard pattern.

use std::str::FromStr;
use std::time::Duration;

use bytes::Bytes;
use chrono::{DateTime, Utc};
use tokio::sync::{mpsc, oneshot};

use ruststack_kinesis_model::types::HashKeyRange as ModelHashKeyRange;

use super::hash::HashKeyRange;
use super::sequence::{SequenceNumber, SequenceNumberGenerator};
use super::storage::{ShardRecordLog, StoredRecord};

/// Commands sent to a shard actor.
#[derive(Debug)]
pub enum ShardCommand {
    /// Put a single record into the shard.
    PutRecord {
        /// The raw data payload.
        data: Bytes,
        /// The partition key.
        partition_key: String,
        /// The explicit hash key, if provided.
        explicit_hash_key: Option<String>,
        /// Reply channel with (sequence_number, shard_id).
        reply: oneshot::Sender<Result<(String, String), String>>,
    },
    /// Get records starting from a position.
    GetRecords {
        /// Logical position in the record log.
        position: usize,
        /// Maximum number of records to return.
        limit: usize,
        /// Reply channel with (records, next_position, millis_behind_latest).
        reply: oneshot::Sender<(Vec<StoredRecord>, usize, i64)>,
    },
    /// Get a shard iterator starting position.
    GetShardIterator {
        /// The iterator type specification.
        iter_type: IteratorRequest,
        /// Reply channel with the logical position.
        reply: oneshot::Sender<Result<usize, String>>,
    },
    /// Close the shard (no new records accepted).
    /// Replies with the last sequence number in the shard (if any).
    Close {
        /// The ending sequence number to set.
        ending_sequence_number: Option<String>,
        /// Reply channel with the actual ending sequence number.
        reply: oneshot::Sender<Option<String>>,
    },
    /// Shutdown the actor.
    Shutdown {
        /// Reply channel.
        reply: oneshot::Sender<()>,
    },
}

/// Specification for getting a shard iterator.
#[derive(Debug)]
pub enum IteratorRequest {
    /// Start at the oldest record.
    TrimHorizon,
    /// Start at the latest (new records only).
    Latest,
    /// Start at the given sequence number.
    AtSequenceNumber(String),
    /// Start after the given sequence number.
    AfterSequenceNumber(String),
    /// Start at the given timestamp.
    AtTimestamp(DateTime<Utc>),
}

/// The shard actor that owns the record log and processes commands.
#[derive(Debug)]
struct ShardActor {
    shard_id: String,
    record_log: ShardRecordLog,
    seq_gen: SequenceNumberGenerator,
    commands: mpsc::Receiver<ShardCommand>,
    closed: bool,
}

impl ShardActor {
    /// Run the actor event loop.
    async fn run(mut self) {
        let mut cleanup_interval = tokio::time::interval(Duration::from_mins(1));
        // Don't run cleanup immediately on start
        cleanup_interval.tick().await;

        loop {
            tokio::select! {
                cmd = self.commands.recv() => {
                    match cmd {
                        Some(command) => {
                            if self.handle_command(command) {
                                break;
                            }
                        }
                        None => break,
                    }
                }
                _ = cleanup_interval.tick() => {
                    self.record_log.trim_expired();
                }
            }
        }
        tracing::debug!(shard_id = %self.shard_id, "shard actor stopped");
    }

    /// Handle a single command. Returns `true` if the actor should shut down.
    fn handle_command(&mut self, command: ShardCommand) -> bool {
        match command {
            ShardCommand::PutRecord {
                data,
                partition_key,
                explicit_hash_key,
                reply,
            } => {
                if self.closed {
                    let _ = reply.send(Err("Shard is closed".to_owned()));
                    return false;
                }
                let seq = self.seq_gen.next();
                let record = StoredRecord {
                    sequence_number: seq,
                    data,
                    partition_key,
                    explicit_hash_key,
                    approximate_arrival_timestamp: Utc::now(),
                    encryption_type: None,
                };
                self.record_log.append(record);
                let _ = reply.send(Ok((seq.to_padded_string(), self.shard_id.clone())));
            }
            ShardCommand::GetRecords {
                position,
                limit,
                reply,
            } => {
                let (records, next_pos) = self.record_log.get_records(position, limit);
                let millis_behind = if records.is_empty() {
                    0
                } else {
                    let latest_pos = self.record_log.latest_position();
                    if next_pos >= latest_pos {
                        0
                    } else {
                        let now = Utc::now();
                        let last_record = records.last().map(|r| r.approximate_arrival_timestamp);
                        last_record.map_or(0, |ts| (now - ts).num_milliseconds().max(0))
                    }
                };
                let owned_records: Vec<StoredRecord> = records.into_iter().cloned().collect();
                let _ = reply.send((owned_records, next_pos, millis_behind));
            }
            ShardCommand::GetShardIterator { iter_type, reply } => {
                let position = match iter_type {
                    IteratorRequest::TrimHorizon => Ok(self.record_log.trim_horizon_position()),
                    IteratorRequest::Latest => Ok(self.record_log.latest_position()),
                    IteratorRequest::AtSequenceNumber(seq_str) => {
                        match SequenceNumber::from_str(&seq_str) {
                            Ok(seq) => Ok(self.record_log.position_at_sequence_number(seq)),
                            Err(e) => Err(format!("Invalid sequence number: {e}")),
                        }
                    }
                    IteratorRequest::AfterSequenceNumber(seq_str) => {
                        match SequenceNumber::from_str(&seq_str) {
                            Ok(seq) => Ok(self.record_log.position_after_sequence_number(seq)),
                            Err(e) => Err(format!("Invalid sequence number: {e}")),
                        }
                    }
                    IteratorRequest::AtTimestamp(ts) => {
                        Ok(self.record_log.position_at_timestamp(ts))
                    }
                };
                let _ = reply.send(position);
            }
            ShardCommand::Close {
                ending_sequence_number: _,
                reply,
            } => {
                self.closed = true;
                // Return the last sequence number from the record log
                let last_seq = self.record_log.last_sequence_number();
                let _ = reply.send(last_seq);
            }
            ShardCommand::Shutdown { reply } => {
                self.closed = true;
                let _ = reply.send(());
                return true;
            }
        }
        false
    }
}

/// Metadata about a shard for API responses.
#[derive(Debug, Clone)]
pub struct ShardInfo {
    /// The shard identifier.
    pub shard_id: String,
    /// The hash key range as model strings.
    pub hash_key_range: ModelHashKeyRange,
    /// The starting sequence number string.
    pub starting_sequence_number: String,
    /// The ending sequence number string (set when shard is closed).
    pub ending_sequence_number: Option<String>,
    /// The parent shard ID (set for split/merge children).
    pub parent_shard_id: Option<String>,
    /// The adjacent parent shard ID (set for merge children).
    pub adjacent_parent_shard_id: Option<String>,
}

/// A handle to communicate with a shard actor.
#[derive(Debug)]
pub struct ShardHandle {
    /// Shard metadata for API responses.
    pub info: ShardInfo,
    /// Command sender.
    pub sender: mpsc::Sender<ShardCommand>,
    /// The actor's join handle.
    pub task: tokio::task::JoinHandle<()>,
}

impl ShardHandle {
    /// Spawn a new shard actor and return its handle.
    #[must_use]
    pub fn spawn(
        shard_id: String,
        shard_index: u16,
        hash_key_range: HashKeyRange,
        retention_period: Duration,
        creation_epoch_millis: u64,
        parent_shard_id: Option<String>,
        adjacent_parent_shard_id: Option<String>,
    ) -> Self {
        let (tx, rx) = mpsc::channel(256);
        let seq_gen = SequenceNumberGenerator::new(shard_index, creation_epoch_millis);
        let starting_seq = seq_gen.starting_sequence_number().to_padded_string();

        let model_range = ModelHashKeyRange {
            starting_hash_key: hash_key_range.start.to_decimal_string(),
            ending_hash_key: hash_key_range.end.to_decimal_string(),
        };

        let info = ShardInfo {
            shard_id: shard_id.clone(),
            hash_key_range: model_range,
            starting_sequence_number: starting_seq,
            ending_sequence_number: None,
            parent_shard_id,
            adjacent_parent_shard_id,
        };

        let actor = ShardActor {
            shard_id,
            record_log: ShardRecordLog::new(retention_period),
            seq_gen,
            commands: rx,
            closed: false,
        };

        let task = tokio::spawn(actor.run());

        Self {
            info,
            sender: tx,
            task,
        }
    }
}
