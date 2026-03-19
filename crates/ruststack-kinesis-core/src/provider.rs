//! Kinesis service provider implementing all operations.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use ruststack_kinesis_model::error::{KinesisError, KinesisErrorCode};
use ruststack_kinesis_model::input::{
    AddTagsToStreamInput, CreateStreamInput, DecreaseStreamRetentionPeriodInput,
    DeleteResourcePolicyInput, DeleteStreamInput, DeregisterStreamConsumerInput,
    DescribeLimitsInput, DescribeStreamConsumerInput, DescribeStreamInput,
    DescribeStreamSummaryInput, GetRecordsInput, GetResourcePolicyInput, GetShardIteratorInput,
    IncreaseStreamRetentionPeriodInput, ListShardsInput, ListStreamConsumersInput,
    ListStreamsInput, ListTagsForStreamInput, MergeShardsInput, PutRecordInput, PutRecordsInput,
    PutResourcePolicyInput, RegisterStreamConsumerInput, RemoveTagsFromStreamInput,
    SplitShardInput, StartStreamEncryptionInput, StopStreamEncryptionInput, SubscribeToShardInput,
    UpdateShardCountInput,
};
use ruststack_kinesis_model::output::{
    DescribeLimitsOutput, DescribeStreamConsumerOutput, DescribeStreamOutput,
    DescribeStreamSummaryOutput, GetRecordsOutput, GetResourcePolicyOutput, GetShardIteratorOutput,
    ListShardsOutput, ListStreamConsumersOutput, ListStreamsOutput, ListTagsForStreamOutput,
    PutRecordOutput, PutRecordsOutput, RegisterStreamConsumerOutput, UpdateShardCountOutput,
};
use ruststack_kinesis_model::types::{
    Consumer, ConsumerDescription, ConsumerStatus, EncryptionType, EnhancedMetrics,
    PutRecordsResultEntry, Record, SequenceNumberRange, Shard, ShardFilterType, ShardIteratorType,
    StreamDescription, StreamDescriptionSummary, StreamMode, StreamModeDetails, StreamStatus,
    StreamSummary, Tag,
};

use crate::config::KinesisConfig;
use crate::shard::actor::{IteratorRequest, ShardCommand, ShardHandle, ShardInfo};
use crate::shard::hash::{HashKey, HashKeyRange};
use crate::shard::iterator::ShardIteratorToken;

/// Internal consumer metadata.
#[derive(Debug, Clone)]
struct ConsumerState {
    name: String,
    arn: String,
    creation_timestamp: DateTime<Utc>,
    status: ConsumerStatus,
}

/// Lightweight snapshot of a shard handle for use after dropping the DashMap guard.
#[derive(Debug)]
struct ShardSenderSnapshot {
    info: ShardInfo,
    sender: mpsc::Sender<ShardCommand>,
}

/// Internal stream state.
#[derive(Debug)]
pub struct StreamState {
    name: String,
    arn: String,
    status: StreamStatus,
    mode: StreamMode,
    retention_period: Duration,
    creation_timestamp: DateTime<Utc>,
    /// All shards (including closed ones for history).
    shards: Vec<ShardInfo>,
    /// Active (open) shard handles.
    active_shards: Vec<ShardHandle>,
    /// Handles for closed shards (still accessible for reads).
    closed_shard_handles: Vec<ShardHandle>,
    tags: HashMap<String, String>,
    encryption_type: Option<EncryptionType>,
    key_id: Option<String>,
    consumers: HashMap<String, ConsumerState>,
    resource_policy: Option<String>,
    next_shard_index: u32,
}

/// The Kinesis service provider.
#[derive(Debug)]
pub struct RustStackKinesis {
    streams: DashMap<String, StreamState>,
    config: Arc<KinesisConfig>,
}

/// Convert an i32 to usize, clamping negatives to 0.
fn i32_to_usize(v: i32) -> usize {
    v.max(0).cast_unsigned() as usize
}

/// Safely truncate a usize to i32 (clamped to `i32::MAX`).
fn usize_to_i32(v: usize) -> i32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    let result = v.min(i32::MAX as usize) as i32;
    result
}

/// Safely truncate u64 to i32 (clamped to `i32::MAX`).
fn u64_to_i32(v: u64) -> i32 {
    #[allow(clippy::cast_possible_truncation)]
    let result = v.min(u64::from(i32::MAX.cast_unsigned())) as i32;
    result
}

// Many public methods take input types by value for API consistency with the
// handler layer, even when not all fields are consumed. This is intentional.
#[allow(clippy::needless_pass_by_value)]
impl RustStackKinesis {
    /// Create a new Kinesis provider with the given configuration.
    #[must_use]
    pub fn new(config: KinesisConfig) -> Self {
        Self {
            streams: DashMap::new(),
            config: Arc::new(config),
        }
    }

    // ── Helpers ──

    fn resolve_stream_name(
        stream_name: Option<&str>,
        stream_arn: Option<&str>,
    ) -> Result<String, KinesisError> {
        if let Some(name) = stream_name {
            return Ok(name.to_owned());
        }
        if let Some(arn) = stream_arn {
            // ARN format: arn:aws:kinesis:{region}:{account_id}:stream/{name}
            if let Some(name) = arn.rsplit('/').next() {
                return Ok(name.to_owned());
            }
            return Err(KinesisError::with_message(
                KinesisErrorCode::InvalidArgumentException,
                format!("Invalid stream ARN: {arn}"),
            ));
        }
        Err(KinesisError::with_message(
            KinesisErrorCode::InvalidArgumentException,
            "Either StreamName or StreamARN must be provided",
        ))
    }

    fn get_stream(
        &self,
        name: &str,
    ) -> Result<dashmap::mapref::one::Ref<'_, String, StreamState>, KinesisError> {
        self.streams.get(name).ok_or_else(|| {
            KinesisError::with_message(
                KinesisErrorCode::ResourceNotFoundException,
                format!(
                    "Stream {} under account {} not found.",
                    name, self.config.default_account_id
                ),
            )
        })
    }

    fn get_stream_mut(
        &self,
        name: &str,
    ) -> Result<dashmap::mapref::one::RefMut<'_, String, StreamState>, KinesisError> {
        self.streams.get_mut(name).ok_or_else(|| {
            KinesisError::with_message(
                KinesisErrorCode::ResourceNotFoundException,
                format!(
                    "Stream {} under account {} not found.",
                    name, self.config.default_account_id
                ),
            )
        })
    }

    /// Find a shard handle by ID in both active and closed shard handles.
    fn find_shard_handle_any<'a>(
        active: &'a [ShardHandle],
        closed: &'a [ShardHandle],
        shard_id: &str,
    ) -> Option<&'a ShardHandle> {
        active
            .iter()
            .chain(closed.iter())
            .find(|h| h.info.shard_id == shard_id)
    }

    fn validate_stream_name(name: &str) -> Result<(), KinesisError> {
        if name.is_empty() || name.len() > 128 {
            return Err(KinesisError::with_message(
                KinesisErrorCode::InvalidArgumentException,
                "Stream name must be between 1 and 128 characters",
            ));
        }
        Ok(())
    }

    fn route_to_shard(
        active_shards: &[ShardHandle],
        partition_key: &str,
        explicit_hash_key: Option<&str>,
    ) -> Result<usize, KinesisError> {
        if active_shards.is_empty() {
            return Err(KinesisError::with_message(
                KinesisErrorCode::ResourceNotFoundException,
                "No active shards available",
            ));
        }

        let hash_key = if let Some(ehk) = explicit_hash_key {
            HashKey::from_decimal_str(ehk).map_err(|_| {
                KinesisError::with_message(
                    KinesisErrorCode::InvalidArgumentException,
                    format!("Invalid ExplicitHashKey: {ehk}"),
                )
            })?
        } else {
            HashKey::from_partition_key(partition_key)
        };

        for (i, handle) in active_shards.iter().enumerate() {
            let range = HashKeyRange {
                start: HashKey::from_decimal_str(&handle.info.hash_key_range.starting_hash_key)
                    .unwrap_or(HashKey::MIN),
                end: HashKey::from_decimal_str(&handle.info.hash_key_range.ending_hash_key)
                    .unwrap_or(HashKey::MAX),
            };
            if range.contains(hash_key) {
                return Ok(i);
            }
        }

        // Fallback: use first shard (should not happen with proper hash ranges)
        Ok(0)
    }

    fn format_shard_id(index: u32) -> String {
        format!("shardId-{index:012}")
    }

    fn stream_arn(&self, name: &str) -> String {
        format!(
            "arn:aws:kinesis:{}:{}:stream/{}",
            self.config.default_region, self.config.default_account_id, name
        )
    }

    fn consumer_arn(&self, stream_name: &str, consumer_name: &str, timestamp: i64) -> String {
        format!(
            "arn:aws:kinesis:{}:{}:stream/{}/consumer/{}:{}",
            self.config.default_region,
            self.config.default_account_id,
            stream_name,
            consumer_name,
            timestamp
        )
    }

    fn shard_info_to_model(info: &ShardInfo) -> Shard {
        Shard {
            shard_id: info.shard_id.clone(),
            hash_key_range: info.hash_key_range.clone(),
            sequence_number_range: SequenceNumberRange {
                starting_sequence_number: info.starting_sequence_number.clone(),
                ending_sequence_number: info.ending_sequence_number.clone(),
            },
            parent_shard_id: info.parent_shard_id.clone(),
            adjacent_parent_shard_id: info.adjacent_parent_shard_id.clone(),
        }
    }

    fn parse_hash_range_from_info(info: &ShardInfo) -> HashKeyRange {
        let start = HashKey::from_decimal_str(&info.hash_key_range.starting_hash_key)
            .unwrap_or(HashKey::MIN);
        let end =
            HashKey::from_decimal_str(&info.hash_key_range.ending_hash_key).unwrap_or(HashKey::MAX);
        HashKeyRange { start, end }
    }

    /// Spawn a child shard and register it in the stream state.
    fn spawn_child_shard(
        stream: &mut StreamState,
        range: HashKeyRange,
        creation_millis: u64,
        parent_shard_id: Option<String>,
        adjacent_parent_shard_id: Option<String>,
    ) -> ShardInfo {
        let child_idx = stream.next_shard_index;
        stream.next_shard_index += 1;
        let child_id = Self::format_shard_id(child_idx);
        #[allow(clippy::cast_possible_truncation)]
        let shard_index_u16 = (child_idx & 0xFFFF) as u16;
        let handle = ShardHandle::spawn(
            child_id,
            shard_index_u16,
            range,
            stream.retention_period,
            creation_millis,
            parent_shard_id,
            adjacent_parent_shard_id,
        );
        let info = handle.info.clone();
        stream.shards.push(info.clone());
        stream.active_shards.push(handle);
        info
    }

    /// Close a shard actor by sending a Close command.
    /// Returns the actual ending sequence number from the shard actor.
    async fn close_shard(handle: &ShardHandle) -> Option<String> {
        let (tx, rx) = oneshot::channel();
        let _ = handle
            .sender
            .send(ShardCommand::Close {
                ending_sequence_number: None,
                reply: tx,
            })
            .await;
        rx.await.ok().flatten()
    }

    /// Mark a shard as closed in the shard history with the actual ending sequence number.
    fn mark_shard_closed(
        stream: &mut StreamState,
        shard_id: &str,
        ending_sequence_number: Option<String>,
    ) {
        if let Some(shard_info) = stream.shards.iter_mut().find(|s| s.shard_id == shard_id) {
            // Use the actual ending sequence number from the shard actor,
            // or the starting sequence number if no records were written.
            shard_info.ending_sequence_number = Some(
                ending_sequence_number
                    .unwrap_or_else(|| shard_info.starting_sequence_number.clone()),
            );
        }
    }

    /// Extract stream name from an ARN string.
    fn stream_name_from_arn(arn: &str) -> Result<String, KinesisError> {
        arn.rsplit('/').next().map(str::to_owned).ok_or_else(|| {
            KinesisError::with_message(
                KinesisErrorCode::InvalidArgumentException,
                format!("Invalid ARN: {arn}"),
            )
        })
    }

    /// Parse consumer name from a consumer ARN.
    /// Format: `arn:aws:kinesis:{region}:{account}:stream/{stream}/consumer/{name}:{ts}`
    fn parse_consumer_arn(arn: &str) -> Result<(&str, &str), KinesisError> {
        // Split: ["arn:aws:kinesis:...:stream", "{stream}", "consumer", "{name}:{ts}"]
        let parts: Vec<&str> = arn.split('/').collect();
        if parts.len() < 4 {
            return Err(KinesisError::with_message(
                KinesisErrorCode::InvalidArgumentException,
                format!("Invalid consumer ARN: {arn}"),
            ));
        }
        let stream_name = parts[1];
        // parts[3] is "{name}:{timestamp}" - extract just the name
        let consumer_name = parts[3].split(':').next().unwrap_or(parts[3]);
        Ok((stream_name, consumer_name))
    }

    // ── Phase 0: Core stream operations ──

    /// Create a new Kinesis stream.
    pub fn create_stream(&self, input: CreateStreamInput) -> Result<(), KinesisError> {
        Self::validate_stream_name(&input.stream_name)?;

        if self.streams.contains_key(&input.stream_name) {
            return Err(KinesisError::with_message(
                KinesisErrorCode::ResourceInUseException,
                format!("Stream {} already exists", input.stream_name),
            ));
        }

        let shard_count = input
            .shard_count
            .map_or(self.config.default_shard_count, |c| {
                c.max(0).cast_unsigned()
            });
        let retention_hours = self.config.default_retention_hours;
        let retention_period = Duration::from_secs(u64::from(retention_hours) * 3600);
        let now = Utc::now();
        let creation_millis = now_epoch_millis();
        let arn = self.stream_arn(&input.stream_name);

        let hash_ranges = HashKeyRange::divide_evenly(shard_count);
        let mut shards = Vec::with_capacity(shard_count as usize);
        let mut active_shards = Vec::with_capacity(shard_count as usize);

        for (i, range) in hash_ranges.into_iter().enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            let shard_idx = i as u32;
            let shard_id = Self::format_shard_id(shard_idx);
            #[allow(clippy::cast_possible_truncation)]
            let shard_index_u16 = (shard_idx & 0xFFFF) as u16;
            let handle = ShardHandle::spawn(
                shard_id,
                shard_index_u16,
                range,
                retention_period,
                creation_millis,
                None,
                None,
            );
            shards.push(handle.info.clone());
            active_shards.push(handle);
        }

        let mode = input
            .stream_mode_details
            .as_ref()
            .map_or(StreamMode::Provisioned, |d| d.stream_mode.clone());

        let state = StreamState {
            name: input.stream_name.clone(),
            arn,
            status: StreamStatus::Active,
            mode,
            retention_period,
            creation_timestamp: now,
            shards,
            active_shards,
            closed_shard_handles: Vec::new(),
            tags: input.tags,
            encryption_type: None,
            key_id: None,
            consumers: HashMap::new(),
            resource_policy: None,
            next_shard_index: shard_count,
        };

        self.streams.insert(input.stream_name, state);
        Ok(())
    }

    /// Delete a stream and shut down all shard actors.
    pub async fn delete_stream(&self, input: DeleteStreamInput) -> Result<(), KinesisError> {
        let name =
            Self::resolve_stream_name(input.stream_name.as_deref(), input.stream_arn.as_deref())?;

        let (_, mut state) = self.streams.remove(&name).ok_or_else(|| {
            KinesisError::with_message(
                KinesisErrorCode::ResourceNotFoundException,
                format!(
                    "Stream {} under account {} not found.",
                    name, self.config.default_account_id
                ),
            )
        })?;

        // Shut down all shard actors (active and closed)
        let all_handles = state
            .active_shards
            .iter()
            .chain(state.closed_shard_handles.iter());
        for handle in all_handles {
            let (tx, rx) = oneshot::channel();
            let _ = handle
                .sender
                .send(ShardCommand::Shutdown { reply: tx })
                .await;
            let _ = rx.await;
        }

        // Wait for all actor tasks to finish
        for handle in state.active_shards.drain(..) {
            let _ = handle.task.await;
        }
        for handle in state.closed_shard_handles.drain(..) {
            let _ = handle.task.await;
        }

        tracing::info!(stream = %name, "stream deleted");
        Ok(())
    }

    /// Describe a stream.
    pub fn describe_stream(
        &self,
        input: DescribeStreamInput,
    ) -> Result<DescribeStreamOutput, KinesisError> {
        let name =
            Self::resolve_stream_name(input.stream_name.as_deref(), input.stream_arn.as_deref())?;
        let stream = self.get_stream(&name)?;

        let all_shards: Vec<Shard> = stream
            .shards
            .iter()
            .map(Self::shard_info_to_model)
            .collect();

        // Handle pagination
        let start_idx = if let Some(ref start_shard_id) = input.exclusive_start_shard_id {
            all_shards
                .iter()
                .position(|s| s.shard_id == *start_shard_id)
                .map_or(0, |pos| pos + 1)
        } else {
            0
        };
        let limit = i32_to_usize(input.limit.unwrap_or(100));
        let end_idx = (start_idx + limit).min(all_shards.len());
        let page_shards: Vec<Shard> = all_shards[start_idx..end_idx].to_vec();
        let has_more = end_idx < all_shards.len();

        let retention_hours = stream.retention_period.as_secs() / 3600;

        let description = StreamDescription {
            stream_name: stream.name.clone(),
            stream_arn: stream.arn.clone(),
            stream_status: stream.status.clone(),
            stream_mode_details: Some(StreamModeDetails {
                stream_mode: stream.mode.clone(),
            }),
            stream_creation_timestamp: stream.creation_timestamp,
            retention_period_hours: u64_to_i32(retention_hours),
            shards: page_shards,
            has_more_shards: has_more,
            enhanced_monitoring: vec![EnhancedMetrics {
                shard_level_metrics: Vec::new(),
            }],
            encryption_type: stream.encryption_type.clone(),
            key_id: stream.key_id.clone(),
        };

        Ok(DescribeStreamOutput {
            stream_description: description,
        })
    }

    /// Describe a stream summary.
    pub fn describe_stream_summary(
        &self,
        input: DescribeStreamSummaryInput,
    ) -> Result<DescribeStreamSummaryOutput, KinesisError> {
        let name =
            Self::resolve_stream_name(input.stream_name.as_deref(), input.stream_arn.as_deref())?;
        let stream = self.get_stream(&name)?;
        let retention_hours = stream.retention_period.as_secs() / 3600;

        let summary = StreamDescriptionSummary {
            stream_name: stream.name.clone(),
            stream_arn: stream.arn.clone(),
            stream_status: stream.status.clone(),
            stream_mode_details: Some(StreamModeDetails {
                stream_mode: stream.mode.clone(),
            }),
            stream_creation_timestamp: stream.creation_timestamp,
            retention_period_hours: u64_to_i32(retention_hours),
            open_shard_count: usize_to_i32(stream.active_shards.len()),
            consumer_count: Some(usize_to_i32(stream.consumers.len())),
            enhanced_monitoring: vec![EnhancedMetrics {
                shard_level_metrics: Vec::new(),
            }],
            encryption_type: stream.encryption_type.clone(),
            key_id: stream.key_id.clone(),
            stream_id: None,
            max_record_size_in_ki_b: None,
            warm_throughput: None,
        };

        Ok(DescribeStreamSummaryOutput {
            stream_description_summary: summary,
        })
    }

    /// List streams with pagination.
    pub fn list_streams(&self, input: ListStreamsInput) -> Result<ListStreamsOutput, KinesisError> {
        let limit = i32_to_usize(input.limit.unwrap_or(100));

        let mut names: Vec<String> = self
            .streams
            .iter()
            .map(|entry| entry.key().clone())
            .collect();
        names.sort();

        let start_idx = if let Some(ref start_name) = input.exclusive_start_stream_name {
            names
                .iter()
                .position(|n| n > start_name)
                .unwrap_or(names.len())
        } else {
            0
        };

        let end_idx = (start_idx + limit).min(names.len());
        let page_names: Vec<String> = names[start_idx..end_idx].to_vec();
        let has_more = end_idx < names.len();

        let summaries: Vec<StreamSummary> = page_names
            .iter()
            .filter_map(|name| {
                self.streams.get(name).map(|s| StreamSummary {
                    stream_name: s.name.clone(),
                    stream_arn: s.arn.clone(),
                    stream_status: s.status.clone(),
                    stream_mode_details: Some(StreamModeDetails {
                        stream_mode: s.mode.clone(),
                    }),
                    stream_creation_timestamp: Some(s.creation_timestamp),
                })
            })
            .collect();

        Ok(ListStreamsOutput {
            stream_names: page_names,
            stream_summaries: summaries,
            has_more_streams: has_more,
            next_token: None,
        })
    }

    /// Validate a partition key (1-256 characters).
    fn validate_partition_key(partition_key: &str) -> Result<(), KinesisError> {
        if partition_key.is_empty() || partition_key.len() > 256 {
            return Err(KinesisError::with_message(
                KinesisErrorCode::InvalidArgumentException,
                "Partition key must be between 1 and 256 characters",
            ));
        }
        Ok(())
    }

    /// Maximum record data size: 1 MiB.
    const MAX_RECORD_SIZE: usize = 1_048_576;

    /// Maximum number of records per PutRecords request.
    const MAX_PUT_RECORDS_ENTRIES: usize = 500;

    /// Maximum total data size per PutRecords request: 5 MiB.
    const MAX_PUT_RECORDS_TOTAL_SIZE: usize = 5 * 1_048_576;

    /// Validate record data size (max 1 MiB).
    fn validate_data_size(data: &[u8]) -> Result<(), KinesisError> {
        if data.len() > Self::MAX_RECORD_SIZE {
            return Err(KinesisError::with_message(
                KinesisErrorCode::InvalidArgumentException,
                format!(
                    "Record data size {} exceeds maximum of {} bytes",
                    data.len(),
                    Self::MAX_RECORD_SIZE,
                ),
            ));
        }
        Ok(())
    }

    /// Put a single record to a stream.
    pub async fn put_record(&self, input: PutRecordInput) -> Result<PutRecordOutput, KinesisError> {
        let name =
            Self::resolve_stream_name(input.stream_name.as_deref(), input.stream_arn.as_deref())?;

        Self::validate_partition_key(&input.partition_key)?;
        Self::validate_data_size(&input.data)?;

        // Extract the sender and encryption type from the DashMap guard, then drop
        // the guard before awaiting to avoid holding it across an .await point.
        let (sender, encryption_type) = {
            let stream = self.get_stream(&name)?;
            let shard_idx = Self::route_to_shard(
                &stream.active_shards,
                &input.partition_key,
                input.explicit_hash_key.as_deref(),
            )?;
            let sender = stream.active_shards[shard_idx].sender.clone();
            let encryption_type = stream.encryption_type.clone();
            (sender, encryption_type)
        };

        let (tx, rx) = oneshot::channel();
        sender
            .send(ShardCommand::PutRecord {
                data: input.data,
                partition_key: input.partition_key,
                explicit_hash_key: input.explicit_hash_key,
                reply: tx,
            })
            .await
            .map_err(|_| KinesisError::internal_error("Failed to send command to shard actor"))?;

        let (seq_number, shard_id) = rx
            .await
            .map_err(|_| KinesisError::internal_error("Shard actor did not respond"))?
            .map_err(KinesisError::internal_error)?;

        Ok(PutRecordOutput {
            sequence_number: seq_number,
            shard_id,
            encryption_type,
        })
    }

    /// Put multiple records to a stream.
    #[allow(clippy::too_many_lines)]
    pub async fn put_records(
        &self,
        input: PutRecordsInput,
    ) -> Result<PutRecordsOutput, KinesisError> {
        let name =
            Self::resolve_stream_name(input.stream_name.as_deref(), input.stream_arn.as_deref())?;

        if input.records.len() > Self::MAX_PUT_RECORDS_ENTRIES {
            return Err(KinesisError::with_message(
                KinesisErrorCode::InvalidArgumentException,
                format!(
                    "PutRecords supports a maximum of {} records per request",
                    Self::MAX_PUT_RECORDS_ENTRIES,
                ),
            ));
        }

        // Validate each entry's partition key and data size, and total payload.
        let mut total_size = 0usize;
        for entry in &input.records {
            Self::validate_partition_key(&entry.partition_key)?;
            Self::validate_data_size(&entry.data)?;
            total_size += entry.data.len() + entry.partition_key.len();
        }
        if total_size > Self::MAX_PUT_RECORDS_TOTAL_SIZE {
            return Err(KinesisError::with_message(
                KinesisErrorCode::InvalidArgumentException,
                format!(
                    "Total request payload size {} exceeds maximum of {} bytes",
                    total_size,
                    Self::MAX_PUT_RECORDS_TOTAL_SIZE,
                ),
            ));
        }

        // Clone active shard handles (senders) and encryption type, then drop the
        // DashMap guard before awaiting to avoid holding it across .await points.
        let (active_shards_snapshot, encryption_type) = {
            let stream = self.get_stream(&name)?;
            let snapshot: Vec<ShardSenderSnapshot> = stream
                .active_shards
                .iter()
                .map(|h| ShardSenderSnapshot {
                    info: h.info.clone(),
                    sender: h.sender.clone(),
                })
                .collect();
            let encryption_type = stream.encryption_type.clone();
            (snapshot, encryption_type)
        };

        let mut results = Vec::with_capacity(input.records.len());
        let mut failed_count = 0i32;

        for entry in &input.records {
            let result = Self::put_single_record_via_snapshot(&active_shards_snapshot, entry).await;
            match result {
                Ok(entry_result) => results.push(entry_result),
                Err(entry_result) => {
                    failed_count += 1;
                    results.push(entry_result);
                }
            }
        }

        Ok(PutRecordsOutput {
            records: results,
            failed_record_count: Some(failed_count),
            encryption_type,
        })
    }

    /// Route a record to a shard using snapshot data.
    fn route_to_shard_snapshot(
        active_shards: &[ShardSenderSnapshot],
        partition_key: &str,
        explicit_hash_key: Option<&str>,
    ) -> Result<usize, KinesisError> {
        if active_shards.is_empty() {
            return Err(KinesisError::with_message(
                KinesisErrorCode::ResourceNotFoundException,
                "No active shards available",
            ));
        }

        let hash_key = if let Some(ehk) = explicit_hash_key {
            HashKey::from_decimal_str(ehk).map_err(|_| {
                KinesisError::with_message(
                    KinesisErrorCode::InvalidArgumentException,
                    format!("Invalid ExplicitHashKey: {ehk}"),
                )
            })?
        } else {
            HashKey::from_partition_key(partition_key)
        };

        for (i, handle) in active_shards.iter().enumerate() {
            let range = HashKeyRange {
                start: HashKey::from_decimal_str(&handle.info.hash_key_range.starting_hash_key)
                    .unwrap_or(HashKey::MIN),
                end: HashKey::from_decimal_str(&handle.info.hash_key_range.ending_hash_key)
                    .unwrap_or(HashKey::MAX),
            };
            if range.contains(hash_key) {
                return Ok(i);
            }
        }

        Ok(0)
    }

    /// Put a single record entry for batch operations using shard snapshots.
    /// Returns `Ok(entry)` on success, `Err(entry)` on failure (with error details in entry).
    async fn put_single_record_via_snapshot(
        active_shards: &[ShardSenderSnapshot],
        entry: &ruststack_kinesis_model::types::PutRecordsRequestEntry,
    ) -> Result<PutRecordsResultEntry, PutRecordsResultEntry> {
        let idx = Self::route_to_shard_snapshot(
            active_shards,
            &entry.partition_key,
            entry.explicit_hash_key.as_deref(),
        )
        .map_err(|e| PutRecordsResultEntry {
            error_code: Some("InternalFailure".to_owned()),
            error_message: Some(e.message),
            ..PutRecordsResultEntry::default()
        })?;

        let (tx, rx) = oneshot::channel();
        active_shards[idx]
            .sender
            .send(ShardCommand::PutRecord {
                data: entry.data.clone(),
                partition_key: entry.partition_key.clone(),
                explicit_hash_key: entry.explicit_hash_key.clone(),
                reply: tx,
            })
            .await
            .map_err(|_| PutRecordsResultEntry {
                error_code: Some("InternalFailure".to_owned()),
                error_message: Some("Failed to send to shard actor".to_owned()),
                ..PutRecordsResultEntry::default()
            })?;

        let result = rx.await.map_err(|_| PutRecordsResultEntry {
            error_code: Some("InternalFailure".to_owned()),
            error_message: Some("Shard actor did not respond".to_owned()),
            ..PutRecordsResultEntry::default()
        })?;

        match result {
            Ok((seq, shard_id)) => Ok(PutRecordsResultEntry {
                sequence_number: Some(seq),
                shard_id: Some(shard_id),
                error_code: None,
                error_message: None,
            }),
            Err(e) => Err(PutRecordsResultEntry {
                error_code: Some("InternalFailure".to_owned()),
                error_message: Some(e),
                ..PutRecordsResultEntry::default()
            }),
        }
    }

    /// Get records from a shard iterator.
    pub async fn get_records(
        &self,
        input: GetRecordsInput,
    ) -> Result<GetRecordsOutput, KinesisError> {
        let token = ShardIteratorToken::decode(&input.shard_iterator).map_err(|e| {
            KinesisError::with_message(
                KinesisErrorCode::InvalidArgumentException,
                format!("Invalid shard iterator: {e}"),
            )
        })?;

        let limit = i32_to_usize(input.limit.unwrap_or(10000).min(10000));

        // Clone the sender from the shard handle, then drop the DashMap guard
        // before awaiting to avoid holding it across an .await point.
        // Search both active and closed shards.
        let sender = {
            let stream = self.get_stream(&token.stream_name)?;
            let shard_handle = Self::find_shard_handle_any(
                &stream.active_shards,
                &stream.closed_shard_handles,
                &token.shard_id,
            )
            .ok_or_else(|| {
                KinesisError::with_message(
                    KinesisErrorCode::ResourceNotFoundException,
                    format!(
                        "Shard {} not found in stream {}",
                        token.shard_id, token.stream_name
                    ),
                )
            })?;
            shard_handle.sender.clone()
        };

        let (tx, rx) = oneshot::channel();
        sender
            .send(ShardCommand::GetRecords {
                position: token.position,
                limit,
                reply: tx,
            })
            .await
            .map_err(|_| KinesisError::internal_error("Failed to send to shard actor"))?;

        let (records, next_pos, millis_behind) = rx
            .await
            .map_err(|_| KinesisError::internal_error("Shard actor did not respond"))?;

        // Build next iterator token
        let next_token = ShardIteratorToken {
            stream_name: token.stream_name,
            shard_id: token.shard_id,
            position: next_pos,
            nonce: Uuid::new_v4().to_string(),
        };

        let model_records: Vec<Record> = records
            .into_iter()
            .map(|r| Record {
                sequence_number: r.sequence_number.to_padded_string(),
                data: r.data,
                partition_key: r.partition_key,
                approximate_arrival_timestamp: Some(r.approximate_arrival_timestamp),
                encryption_type: r.encryption_type.map(|_| EncryptionType::None),
            })
            .collect();

        Ok(GetRecordsOutput {
            records: model_records,
            next_shard_iterator: Some(next_token.encode()),
            millis_behind_latest: Some(millis_behind),
            child_shards: Vec::new(),
        })
    }

    /// Get a shard iterator.
    pub async fn get_shard_iterator(
        &self,
        input: GetShardIteratorInput,
    ) -> Result<GetShardIteratorOutput, KinesisError> {
        let name =
            Self::resolve_stream_name(input.stream_name.as_deref(), input.stream_arn.as_deref())?;

        let iter_request = match input.shard_iterator_type {
            ShardIteratorType::TrimHorizon => IteratorRequest::TrimHorizon,
            ShardIteratorType::Latest => IteratorRequest::Latest,
            ShardIteratorType::AtSequenceNumber => {
                let seq = input.starting_sequence_number.ok_or_else(|| {
                    KinesisError::with_message(
                        KinesisErrorCode::InvalidArgumentException,
                        "StartingSequenceNumber is required for AT_SEQUENCE_NUMBER",
                    )
                })?;
                IteratorRequest::AtSequenceNumber(seq)
            }
            ShardIteratorType::AfterSequenceNumber => {
                let seq = input.starting_sequence_number.ok_or_else(|| {
                    KinesisError::with_message(
                        KinesisErrorCode::InvalidArgumentException,
                        "StartingSequenceNumber is required for AFTER_SEQUENCE_NUMBER",
                    )
                })?;
                IteratorRequest::AfterSequenceNumber(seq)
            }
            ShardIteratorType::AtTimestamp => {
                let ts = input.timestamp.ok_or_else(|| {
                    KinesisError::with_message(
                        KinesisErrorCode::InvalidArgumentException,
                        "Timestamp is required for AT_TIMESTAMP",
                    )
                })?;
                IteratorRequest::AtTimestamp(ts)
            }
        };

        // Clone the sender from the shard handle, then drop the DashMap guard
        // before awaiting. Search both active and closed shards.
        let sender = {
            let stream = self.get_stream(&name)?;
            let shard_handle = Self::find_shard_handle_any(
                &stream.active_shards,
                &stream.closed_shard_handles,
                &input.shard_id,
            )
            .ok_or_else(|| {
                KinesisError::with_message(
                    KinesisErrorCode::ResourceNotFoundException,
                    format!("Shard {} not found in stream {}", input.shard_id, name),
                )
            })?;
            shard_handle.sender.clone()
        };

        let (tx, rx) = oneshot::channel();
        sender
            .send(ShardCommand::GetShardIterator {
                iter_type: iter_request,
                reply: tx,
            })
            .await
            .map_err(|_| KinesisError::internal_error("Failed to send to shard actor"))?;

        let position = rx
            .await
            .map_err(|_| KinesisError::internal_error("Shard actor did not respond"))?
            .map_err(|e| {
                KinesisError::with_message(KinesisErrorCode::InvalidArgumentException, e)
            })?;

        let token = ShardIteratorToken {
            stream_name: name,
            shard_id: input.shard_id,
            position,
            nonce: Uuid::new_v4().to_string(),
        };

        Ok(GetShardIteratorOutput {
            shard_iterator: Some(token.encode()),
        })
    }

    /// List shards in a stream.
    pub fn list_shards(&self, input: ListShardsInput) -> Result<ListShardsOutput, KinesisError> {
        let name =
            Self::resolve_stream_name(input.stream_name.as_deref(), input.stream_arn.as_deref())?;
        let stream = self.get_stream(&name)?;
        let max_results = i32_to_usize(input.max_results.unwrap_or(100));

        let mut all_shards: Vec<Shard> = stream
            .shards
            .iter()
            .map(Self::shard_info_to_model)
            .collect();

        // Apply shard filter
        if let Some(ref filter) = input.shard_filter {
            match filter.r#type {
                ShardFilterType::AfterShardId => {
                    if let Some(ref shard_id) = filter.shard_id {
                        if let Some(pos) = all_shards.iter().position(|s| &s.shard_id == shard_id) {
                            all_shards = all_shards[pos + 1..].to_vec();
                        }
                    }
                }
                ShardFilterType::AtLatest => {
                    // Return only active (open) shards
                    all_shards.retain(|s| s.sequence_number_range.ending_sequence_number.is_none());
                }
                ShardFilterType::AtTrimHorizon
                | ShardFilterType::FromTrimHorizon
                | ShardFilterType::AtTimestamp
                | ShardFilterType::FromTimestamp => {
                    // Return all shards
                }
            }
        }

        // When next_token is provided, treat it as the exclusive start shard ID.
        let effective_start_shard_id = input
            .next_token
            .as_ref()
            .or(input.exclusive_start_shard_id.as_ref());

        let start_idx = if let Some(start_id) = effective_start_shard_id {
            all_shards
                .iter()
                .position(|s| s.shard_id == *start_id)
                .map_or(0, |pos| pos + 1)
        } else {
            0
        };

        let end_idx = (start_idx + max_results).min(all_shards.len());
        let page: Vec<Shard> = all_shards[start_idx..end_idx].to_vec();
        let next_token = if end_idx < all_shards.len() {
            page.last().map(|s| s.shard_id.clone())
        } else {
            None
        };

        Ok(ListShardsOutput {
            shards: page,
            next_token,
        })
    }

    /// Update shard count (placeholder - returns current count).
    pub fn update_shard_count(
        &self,
        input: UpdateShardCountInput,
    ) -> Result<UpdateShardCountOutput, KinesisError> {
        let name =
            Self::resolve_stream_name(input.stream_name.as_deref(), input.stream_arn.as_deref())?;
        let stream = self.get_stream(&name)?;
        let current_count = usize_to_i32(stream.active_shards.len());

        Ok(UpdateShardCountOutput {
            current_shard_count: Some(current_count),
            target_shard_count: Some(input.target_shard_count),
            stream_name: Some(stream.name.clone()),
            stream_arn: Some(stream.arn.clone()),
        })
    }

    // ── Phase 1: Tags, retention, split/merge, encryption ──

    /// Add tags to a stream.
    pub fn add_tags_to_stream(&self, input: AddTagsToStreamInput) -> Result<(), KinesisError> {
        let name =
            Self::resolve_stream_name(input.stream_name.as_deref(), input.stream_arn.as_deref())?;
        let mut stream = self.get_stream_mut(&name)?;

        let new_count = stream.tags.len() + input.tags.len();
        if new_count > 50 {
            return Err(KinesisError::with_message(
                KinesisErrorCode::InvalidArgumentException,
                format!(
                    "Failed to add tags: would exceed maximum of 50 tags (current: {}, adding: {})",
                    stream.tags.len(),
                    input.tags.len()
                ),
            ));
        }

        stream.tags.extend(input.tags);
        Ok(())
    }

    /// Remove tags from a stream.
    pub fn remove_tags_from_stream(
        &self,
        input: RemoveTagsFromStreamInput,
    ) -> Result<(), KinesisError> {
        let name =
            Self::resolve_stream_name(input.stream_name.as_deref(), input.stream_arn.as_deref())?;
        let mut stream = self.get_stream_mut(&name)?;

        for key in &input.tag_keys {
            stream.tags.remove(key);
        }
        Ok(())
    }

    /// List tags for a stream.
    pub fn list_tags_for_stream(
        &self,
        input: ListTagsForStreamInput,
    ) -> Result<ListTagsForStreamOutput, KinesisError> {
        let name =
            Self::resolve_stream_name(input.stream_name.as_deref(), input.stream_arn.as_deref())?;
        let stream = self.get_stream(&name)?;
        let limit = i32_to_usize(input.limit.unwrap_or(50));

        let mut tag_keys: Vec<&String> = stream.tags.keys().collect();
        tag_keys.sort();

        let start_idx = if let Some(ref start_key) = input.exclusive_start_tag_key {
            tag_keys
                .iter()
                .position(|k| *k > start_key)
                .unwrap_or(tag_keys.len())
        } else {
            0
        };

        let end_idx = (start_idx + limit).min(tag_keys.len());
        let page_tags: Vec<Tag> = tag_keys[start_idx..end_idx]
            .iter()
            .map(|k| Tag {
                key: (*k).clone(),
                value: stream.tags.get(*k).cloned(),
            })
            .collect();

        let has_more = end_idx < tag_keys.len();

        Ok(ListTagsForStreamOutput {
            tags: page_tags,
            has_more_tags: has_more,
        })
    }

    /// Increase stream retention period.
    pub fn increase_stream_retention_period(
        &self,
        input: IncreaseStreamRetentionPeriodInput,
    ) -> Result<(), KinesisError> {
        let name =
            Self::resolve_stream_name(input.stream_name.as_deref(), input.stream_arn.as_deref())?;
        let mut stream = self.get_stream_mut(&name)?;
        let current_hours = u64_to_i32(stream.retention_period.as_secs() / 3600);

        if input.retention_period_hours < current_hours {
            return Err(KinesisError::with_message(
                KinesisErrorCode::InvalidArgumentException,
                format!(
                    "Requested retention period {} hours is less than current {} hours",
                    input.retention_period_hours, current_hours
                ),
            ));
        }

        if input.retention_period_hours > 8760 {
            return Err(KinesisError::with_message(
                KinesisErrorCode::InvalidArgumentException,
                "Retention period cannot exceed 8760 hours (365 days)",
            ));
        }

        stream.retention_period =
            Duration::from_secs(i32_to_usize(input.retention_period_hours) as u64 * 3600);
        Ok(())
    }

    /// Decrease stream retention period.
    pub fn decrease_stream_retention_period(
        &self,
        input: DecreaseStreamRetentionPeriodInput,
    ) -> Result<(), KinesisError> {
        let name =
            Self::resolve_stream_name(input.stream_name.as_deref(), input.stream_arn.as_deref())?;
        let mut stream = self.get_stream_mut(&name)?;
        let current_hours = u64_to_i32(stream.retention_period.as_secs() / 3600);

        if input.retention_period_hours > current_hours {
            return Err(KinesisError::with_message(
                KinesisErrorCode::InvalidArgumentException,
                format!(
                    "Requested retention period {} hours is greater than current {} hours",
                    input.retention_period_hours, current_hours
                ),
            ));
        }

        if input.retention_period_hours < 24 {
            return Err(KinesisError::with_message(
                KinesisErrorCode::InvalidArgumentException,
                "Retention period cannot be less than 24 hours",
            ));
        }

        stream.retention_period =
            Duration::from_secs(i32_to_usize(input.retention_period_hours) as u64 * 3600);
        Ok(())
    }

    /// Split a shard into two child shards.
    pub async fn split_shard(&self, input: SplitShardInput) -> Result<(), KinesisError> {
        let name =
            Self::resolve_stream_name(input.stream_name.as_deref(), input.stream_arn.as_deref())?;

        let new_hash = HashKey::from_decimal_str(&input.new_starting_hash_key).map_err(|_| {
            KinesisError::with_message(
                KinesisErrorCode::InvalidArgumentException,
                format!(
                    "Invalid NewStartingHashKey: {}",
                    input.new_starting_hash_key
                ),
            )
        })?;

        let mut stream = self.get_stream_mut(&name)?;

        // Find the parent shard
        let parent_idx = stream
            .active_shards
            .iter()
            .position(|h| h.info.shard_id == input.shard_to_split)
            .ok_or_else(|| {
                KinesisError::with_message(
                    KinesisErrorCode::ResourceNotFoundException,
                    format!("Shard {} not found", input.shard_to_split),
                )
            })?;

        let parent_range = Self::parse_hash_range_from_info(&stream.active_shards[parent_idx].info);

        if new_hash <= parent_range.start || new_hash > parent_range.end {
            return Err(KinesisError::with_message(
                KinesisErrorCode::InvalidArgumentException,
                "NewStartingHashKey is not within the hash key range of the shard to split",
            ));
        }

        // Close the parent shard
        let parent_handle = stream.active_shards.remove(parent_idx);
        let ending_seq = Self::close_shard(&parent_handle).await;

        let parent_shard_id = parent_handle.info.shard_id.clone();
        Self::mark_shard_closed(&mut stream, &parent_shard_id, ending_seq);

        // Move the closed handle to closed_shard_handles for continued read access
        stream.closed_shard_handles.push(parent_handle);

        let creation_millis = now_epoch_millis();

        // Use checked_sub for the first child's ending hash key
        let child1_end = HashKey(new_hash.0.checked_sub(1).unwrap_or(parent_range.start.0));
        let child1_range = HashKeyRange {
            start: parent_range.start,
            end: child1_end,
        };
        Self::spawn_child_shard(
            &mut stream,
            child1_range,
            creation_millis,
            Some(parent_shard_id.clone()),
            None,
        );

        let child2_range = HashKeyRange {
            start: new_hash,
            end: parent_range.end,
        };
        Self::spawn_child_shard(
            &mut stream,
            child2_range,
            creation_millis,
            Some(parent_shard_id),
            None,
        );

        Ok(())
    }

    /// Merge two adjacent shards into one.
    pub async fn merge_shards(&self, input: MergeShardsInput) -> Result<(), KinesisError> {
        let name =
            Self::resolve_stream_name(input.stream_name.as_deref(), input.stream_arn.as_deref())?;
        let mut stream = self.get_stream_mut(&name)?;

        let shard1_idx = stream
            .active_shards
            .iter()
            .position(|h| h.info.shard_id == input.shard_to_merge)
            .ok_or_else(|| {
                KinesisError::with_message(
                    KinesisErrorCode::ResourceNotFoundException,
                    format!("Shard {} not found", input.shard_to_merge),
                )
            })?;

        let shard2_idx = stream
            .active_shards
            .iter()
            .position(|h| h.info.shard_id == input.adjacent_shard_to_merge)
            .ok_or_else(|| {
                KinesisError::with_message(
                    KinesisErrorCode::ResourceNotFoundException,
                    format!("Shard {} not found", input.adjacent_shard_to_merge),
                )
            })?;

        let range1 = Self::parse_hash_range_from_info(&stream.active_shards[shard1_idx].info);
        let range2 = Self::parse_hash_range_from_info(&stream.active_shards[shard2_idx].info);

        // Validate that the shards are adjacent
        let adjacent = (range1.end.0.checked_add(1) == Some(range2.start.0))
            || (range2.end.0.checked_add(1) == Some(range1.start.0));
        if !adjacent {
            return Err(KinesisError::with_message(
                KinesisErrorCode::InvalidArgumentException,
                format!(
                    "Shards {} and {} are not adjacent",
                    input.shard_to_merge, input.adjacent_shard_to_merge
                ),
            ));
        }

        let merged_range = HashKeyRange {
            start: range1.start.min(range2.start),
            end: range1.end.max(range2.end),
        };

        // Remove higher index first to preserve the other index
        let (first_idx, second_idx) = if shard1_idx > shard2_idx {
            (shard1_idx, shard2_idx)
        } else {
            (shard2_idx, shard1_idx)
        };

        let handle_first = stream.active_shards.remove(first_idx);
        let handle_second = stream.active_shards.remove(second_idx);

        let ending_seq1 = Self::close_shard(&handle_first).await;
        let ending_seq2 = Self::close_shard(&handle_second).await;

        let shard1_id = handle_first.info.shard_id.clone();
        let shard2_id = handle_second.info.shard_id.clone();

        Self::mark_shard_closed(&mut stream, &shard1_id, ending_seq1);
        Self::mark_shard_closed(&mut stream, &shard2_id, ending_seq2);

        // Move closed handles for continued read access
        stream.closed_shard_handles.push(handle_first);
        stream.closed_shard_handles.push(handle_second);

        let (parent, adjacent_parent) = if range1.start < range2.start {
            (shard1_id, shard2_id)
        } else {
            (shard2_id, shard1_id)
        };

        let creation_millis = now_epoch_millis();
        Self::spawn_child_shard(
            &mut stream,
            merged_range,
            creation_millis,
            Some(parent),
            Some(adjacent_parent),
        );

        Ok(())
    }

    /// Start stream encryption (metadata only).
    pub fn start_stream_encryption(
        &self,
        input: StartStreamEncryptionInput,
    ) -> Result<(), KinesisError> {
        let name =
            Self::resolve_stream_name(input.stream_name.as_deref(), input.stream_arn.as_deref())?;
        let mut stream = self.get_stream_mut(&name)?;
        stream.encryption_type = Some(input.encryption_type);
        stream.key_id = Some(input.key_id);
        Ok(())
    }

    /// Stop stream encryption.
    pub fn stop_stream_encryption(
        &self,
        input: StopStreamEncryptionInput,
    ) -> Result<(), KinesisError> {
        let name =
            Self::resolve_stream_name(input.stream_name.as_deref(), input.stream_arn.as_deref())?;
        let mut stream = self.get_stream_mut(&name)?;
        stream.encryption_type = Some(EncryptionType::None);
        stream.key_id = None;
        Ok(())
    }

    /// Describe limits (static defaults).
    pub fn describe_limits(
        &self,
        _input: DescribeLimitsInput,
    ) -> Result<DescribeLimitsOutput, KinesisError> {
        let open_shard_count: i32 = self
            .streams
            .iter()
            .map(|s| usize_to_i32(s.active_shards.len()))
            .sum();

        Ok(DescribeLimitsOutput {
            shard_limit: 500,
            open_shard_count,
            on_demand_stream_count: 0,
            on_demand_stream_count_limit: 50,
        })
    }

    // ── Phase 2: Enhanced fan-out consumers ──

    /// Register a stream consumer.
    pub fn register_stream_consumer(
        &self,
        input: RegisterStreamConsumerInput,
    ) -> Result<RegisterStreamConsumerOutput, KinesisError> {
        let stream_name = Self::stream_name_from_arn(&input.stream_arn)?;
        let mut stream = self.get_stream_mut(&stream_name)?;

        if stream.consumers.contains_key(&input.consumer_name) {
            return Err(KinesisError::with_message(
                KinesisErrorCode::ResourceInUseException,
                format!("Consumer {} already exists", input.consumer_name),
            ));
        }

        let now = Utc::now();
        let consumer_arn = self.consumer_arn(&stream_name, &input.consumer_name, now.timestamp());

        let state = ConsumerState {
            name: input.consumer_name.clone(),
            arn: consumer_arn.clone(),
            creation_timestamp: now,
            status: ConsumerStatus::Active,
        };

        stream.consumers.insert(input.consumer_name.clone(), state);

        Ok(RegisterStreamConsumerOutput {
            consumer: Consumer {
                consumer_name: input.consumer_name,
                consumer_arn,
                consumer_status: ConsumerStatus::Active,
                consumer_creation_timestamp: now,
            },
        })
    }

    /// Deregister a stream consumer.
    pub fn deregister_stream_consumer(
        &self,
        input: DeregisterStreamConsumerInput,
    ) -> Result<(), KinesisError> {
        if let Some(ref consumer_arn) = input.consumer_arn {
            let (stream_name, consumer_name) = Self::parse_consumer_arn(consumer_arn)?;
            let mut stream = self.get_stream_mut(stream_name)?;
            stream.consumers.remove(consumer_name).ok_or_else(|| {
                KinesisError::with_message(
                    KinesisErrorCode::ResourceNotFoundException,
                    format!("Consumer {consumer_name} not found"),
                )
            })?;
        } else {
            let stream_arn = input.stream_arn.as_deref().ok_or_else(|| {
                KinesisError::with_message(
                    KinesisErrorCode::InvalidArgumentException,
                    "Either ConsumerARN or StreamARN+ConsumerName must be provided",
                )
            })?;
            let consumer_name = input.consumer_name.as_deref().ok_or_else(|| {
                KinesisError::with_message(
                    KinesisErrorCode::InvalidArgumentException,
                    "ConsumerName is required when using StreamARN",
                )
            })?;
            let stream_name = Self::stream_name_from_arn(stream_arn)?;
            let mut stream = self.get_stream_mut(&stream_name)?;
            stream.consumers.remove(consumer_name).ok_or_else(|| {
                KinesisError::with_message(
                    KinesisErrorCode::ResourceNotFoundException,
                    format!("Consumer {consumer_name} not found"),
                )
            })?;
        }

        Ok(())
    }

    /// Describe a stream consumer.
    pub fn describe_stream_consumer(
        &self,
        input: DescribeStreamConsumerInput,
    ) -> Result<DescribeStreamConsumerOutput, KinesisError> {
        let (stream_name, consumer_name) = if let Some(ref consumer_arn) = input.consumer_arn {
            let (sn, cn) = Self::parse_consumer_arn(consumer_arn)?;
            (sn.to_owned(), cn.to_owned())
        } else {
            let stream_arn = input.stream_arn.as_deref().ok_or_else(|| {
                KinesisError::with_message(
                    KinesisErrorCode::InvalidArgumentException,
                    "Either ConsumerARN or StreamARN+ConsumerName must be provided",
                )
            })?;
            let cn = input.consumer_name.as_deref().ok_or_else(|| {
                KinesisError::with_message(
                    KinesisErrorCode::InvalidArgumentException,
                    "ConsumerName is required when using StreamARN",
                )
            })?;
            let sn = Self::stream_name_from_arn(stream_arn)?;
            (sn, cn.to_owned())
        };

        let stream = self.get_stream(&stream_name)?;
        let state = stream.consumers.get(&consumer_name).ok_or_else(|| {
            KinesisError::with_message(
                KinesisErrorCode::ResourceNotFoundException,
                format!("Consumer {consumer_name} not found"),
            )
        })?;

        Ok(DescribeStreamConsumerOutput {
            consumer_description: ConsumerDescription {
                consumer_name: state.name.clone(),
                consumer_arn: state.arn.clone(),
                consumer_status: state.status.clone(),
                consumer_creation_timestamp: state.creation_timestamp,
                stream_arn: stream.arn.clone(),
            },
        })
    }

    /// List stream consumers.
    pub fn list_stream_consumers(
        &self,
        input: ListStreamConsumersInput,
    ) -> Result<ListStreamConsumersOutput, KinesisError> {
        let stream_name = Self::stream_name_from_arn(&input.stream_arn)?;
        let stream = self.get_stream(&stream_name)?;
        let max_results = i32_to_usize(input.max_results.unwrap_or(100));

        let mut consumer_names: Vec<&String> = stream.consumers.keys().collect();
        consumer_names.sort();

        let start_idx = if let Some(ref token) = input.next_token {
            consumer_names
                .iter()
                .position(|n| *n > token)
                .unwrap_or(consumer_names.len())
        } else {
            0
        };

        let end_idx = (start_idx + max_results).min(consumer_names.len());
        let consumers: Vec<Consumer> = consumer_names[start_idx..end_idx]
            .iter()
            .filter_map(|name| {
                stream.consumers.get(*name).map(|state| Consumer {
                    consumer_name: state.name.clone(),
                    consumer_arn: state.arn.clone(),
                    consumer_status: state.status.clone(),
                    consumer_creation_timestamp: state.creation_timestamp,
                })
            })
            .collect();

        let next_token = if end_idx < consumer_names.len() {
            consumer_names.get(end_idx).map(|n| (*n).clone())
        } else {
            None
        };

        Ok(ListStreamConsumersOutput {
            consumers,
            next_token,
        })
    }

    /// Subscribe to shard (not implemented - returns error).
    pub fn subscribe_to_shard(&self, _input: SubscribeToShardInput) -> Result<(), KinesisError> {
        Err(KinesisError::not_implemented("SubscribeToShard"))
    }

    // ── Phase 3: Resource policies ──

    /// Get resource policy.
    pub fn get_resource_policy(
        &self,
        input: GetResourcePolicyInput,
    ) -> Result<GetResourcePolicyOutput, KinesisError> {
        let stream_name = Self::stream_name_from_arn(&input.resource_arn)?;
        let stream = self.get_stream(&stream_name)?;
        let policy = stream.resource_policy.clone().unwrap_or_default();
        Ok(GetResourcePolicyOutput { policy })
    }

    /// Put resource policy.
    pub fn put_resource_policy(&self, input: PutResourcePolicyInput) -> Result<(), KinesisError> {
        let stream_name = Self::stream_name_from_arn(&input.resource_arn)?;
        let mut stream = self.get_stream_mut(&stream_name)?;
        stream.resource_policy = Some(input.policy);
        Ok(())
    }

    /// Delete resource policy.
    pub fn delete_resource_policy(
        &self,
        input: DeleteResourcePolicyInput,
    ) -> Result<(), KinesisError> {
        let stream_name = Self::stream_name_from_arn(&input.resource_arn)?;
        let mut stream = self.get_stream_mut(&stream_name)?;
        stream.resource_policy = None;
        Ok(())
    }
}

/// Get current time as epoch milliseconds.
fn now_epoch_millis() -> u64 {
    Utc::now().timestamp_millis().cast_unsigned()
}
