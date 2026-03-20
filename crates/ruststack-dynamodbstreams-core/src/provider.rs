//! DynamoDB Streams provider implementing all 4 operations.

use std::collections::HashMap;
use std::sync::Arc;

use ruststack_dynamodb_model::AttributeValue;
use ruststack_dynamodbstreams_model::error::DynamoDBStreamsError;
use ruststack_dynamodbstreams_model::input::{
    DescribeStreamInput, GetRecordsInput, GetShardIteratorInput, ListStreamsInput,
};
use ruststack_dynamodbstreams_model::output::{
    DescribeStreamOutput, GetRecordsOutput, GetShardIteratorOutput, ListStreamsOutput,
};
use ruststack_dynamodbstreams_model::types::{
    self as streams_types, OperationType, Record, SequenceNumberRange, Shard, Stream,
    StreamDescription, StreamRecord, StreamStatus,
};

use crate::config::DynamoDBStreamsConfig;
use crate::iterator::{decode_iterator, encode_iterator};
use crate::storage::{StreamChangeRecord, StreamStore};

/// Main DynamoDB Streams provider implementing all 4 operations.
#[derive(Debug)]
pub struct RustStackDynamoDBStreams {
    /// Stream storage.
    pub store: Arc<StreamStore>,
    /// Configuration.
    pub config: Arc<DynamoDBStreamsConfig>,
}

impl RustStackDynamoDBStreams {
    /// Create a new DynamoDB Streams provider.
    #[must_use]
    pub fn new(store: Arc<StreamStore>, config: DynamoDBStreamsConfig) -> Self {
        Self {
            store,
            config: Arc::new(config),
        }
    }

    /// Handle `DescribeStream`.
    ///
    /// # Errors
    ///
    /// Returns `ResourceNotFoundException` if the stream ARN is not found.
    #[allow(clippy::needless_pass_by_value)]
    pub fn handle_describe_stream(
        &self,
        input: DescribeStreamInput,
    ) -> Result<DescribeStreamOutput, DynamoDBStreamsError> {
        let stream_arn = &input.stream_arn;

        let stream = self.store.get_stream_by_arn(stream_arn).ok_or_else(|| {
            DynamoDBStreamsError::resource_not_found(format!(
                "Requested resource not found: Stream: {stream_arn} not found"
            ))
        })?;

        let shard = stream.shard.read();

        let shard_desc = Shard {
            shard_id: Some(shard.shard_id.clone()),
            parent_shard_id: shard.parent_shard_id.clone(),
            sequence_number_range: Some(SequenceNumberRange {
                starting_sequence_number: shard.starting_sequence_number.clone(),
                ending_sequence_number: shard.ending_sequence_number.clone(),
            }),
        };

        // If ExclusiveStartShardId matches the only shard, return empty list.
        let shards = if input
            .exclusive_start_shard_id
            .as_ref()
            .is_some_and(|id| *id == shard.shard_id)
        {
            vec![]
        } else {
            vec![shard_desc]
        };

        let description = StreamDescription {
            stream_arn: Some(stream.stream_arn.clone()),
            stream_label: Some(stream.stream_label.clone()),
            stream_status: Some(stream.stream_status.clone()),
            stream_view_type: Some(stream.stream_view_type.clone()),
            table_name: Some(stream.table_name.clone()),
            key_schema: stream.key_schema.clone(),
            shards,
            creation_request_date_time: None,
            last_evaluated_shard_id: None,
        };

        Ok(DescribeStreamOutput {
            stream_description: Some(description),
        })
    }

    /// Handle `ListStreams`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request is invalid.
    #[allow(clippy::needless_pass_by_value)]
    pub fn handle_list_streams(
        &self,
        input: ListStreamsInput,
    ) -> Result<ListStreamsOutput, DynamoDBStreamsError> {
        let mut streams = self.store.list_streams(input.table_name.as_deref());

        // Sort by stream ARN for deterministic pagination.
        streams.sort_by(|a, b| a.stream_arn.cmp(&b.stream_arn));

        // Apply ExclusiveStartStreamArn.
        if let Some(ref start_arn) = input.exclusive_start_stream_arn {
            if let Some(pos) = streams.iter().position(|s| s.stream_arn == *start_arn) {
                streams = streams.split_off(pos + 1);
            }
        }

        #[allow(clippy::cast_sign_loss)]
        let limit = input.limit.map_or(100, |l| l.clamp(1, 100) as usize);
        let has_more = streams.len() > limit;
        streams.truncate(limit);

        let last_arn = if has_more {
            streams.last().map(|s| s.stream_arn.clone())
        } else {
            None
        };

        let stream_items: Vec<Stream> = streams
            .into_iter()
            .map(|s| Stream {
                stream_arn: Some(s.stream_arn),
                table_name: Some(s.table_name),
                stream_label: Some(s.stream_label),
            })
            .collect();

        Ok(ListStreamsOutput {
            streams: stream_items,
            last_evaluated_stream_arn: last_arn,
        })
    }

    /// Handle `GetShardIterator`.
    ///
    /// # Errors
    ///
    /// Returns `ResourceNotFoundException` if the stream or shard is not found,
    /// or `ValidationException` if required parameters are missing.
    #[allow(clippy::needless_pass_by_value)]
    pub fn handle_get_shard_iterator(
        &self,
        input: GetShardIteratorInput,
    ) -> Result<GetShardIteratorOutput, DynamoDBStreamsError> {
        let stream_arn = &input.stream_arn;
        let shard_id = &input.shard_id;
        let iter_type = &input.shard_iterator_type;

        let stream = self.store.get_stream_by_arn(stream_arn).ok_or_else(|| {
            DynamoDBStreamsError::resource_not_found(format!(
                "Requested resource not found: Stream: {stream_arn} not found"
            ))
        })?;

        let shard = stream.shard.read();
        if shard.shard_id != *shard_id {
            return Err(DynamoDBStreamsError::resource_not_found(format!(
                "Requested resource not found: Shard: {shard_id} in Stream: {stream_arn} not found"
            )));
        }

        let position = match iter_type {
            streams_types::ShardIteratorType::TrimHorizon => 0u64,
            streams_types::ShardIteratorType::Latest => shard.records.len() as u64,
            streams_types::ShardIteratorType::AtSequenceNumber => {
                let seq = input.sequence_number.as_deref().ok_or_else(|| {
                    DynamoDBStreamsError::validation(
                        "SequenceNumber is required for AT_SEQUENCE_NUMBER",
                    )
                })?;
                find_sequence_position(&shard.records, seq)?
            }
            streams_types::ShardIteratorType::AfterSequenceNumber => {
                let seq = input.sequence_number.as_deref().ok_or_else(|| {
                    DynamoDBStreamsError::validation(
                        "SequenceNumber is required for AFTER_SEQUENCE_NUMBER",
                    )
                })?;
                find_sequence_position(&shard.records, seq)? + 1
            }
        };

        let token = encode_iterator(stream_arn, shard_id, position);

        Ok(GetShardIteratorOutput {
            shard_iterator: Some(token),
        })
    }

    /// Handle `GetRecords`.
    ///
    /// # Errors
    ///
    /// Returns `ExpiredIteratorException` if the shard iterator is invalid or expired.
    #[allow(clippy::needless_pass_by_value)]
    pub fn handle_get_records(
        &self,
        input: GetRecordsInput,
    ) -> Result<GetRecordsOutput, DynamoDBStreamsError> {
        let token = &input.shard_iterator;

        let (stream_arn, shard_id, position) = decode_iterator(token)?;

        let stream = self.store.get_stream_by_arn(stream_arn).ok_or_else(|| {
            DynamoDBStreamsError::expired_iterator("The shard iterator is expired or invalid.")
        })?;

        let shard = stream.shard.read();
        if shard.shard_id != shard_id {
            return Err(DynamoDBStreamsError::expired_iterator(
                "The shard iterator is expired or invalid.",
            ));
        }

        #[allow(clippy::cast_sign_loss)]
        let limit = input.limit.map_or(1000, |l| (l as usize).clamp(1, 1000));
        #[allow(clippy::cast_possible_truncation)]
        let start = position as usize;
        let end = (start + limit).min(shard.records.len());

        let records: Vec<Record> = shard
            .records
            .range(start..end)
            .map(record_to_output)
            .collect();

        let next_position = end as u64;

        // If shard is closed and we've read all records, no next iterator.
        let next_iterator = if stream.stream_status == StreamStatus::Disabled
            && next_position >= shard.records.len() as u64
        {
            None
        } else {
            Some(encode_iterator(
                &stream.stream_arn,
                &shard.shard_id,
                next_position,
            ))
        };

        Ok(GetRecordsOutput {
            records,
            next_shard_iterator: next_iterator,
        })
    }
}

/// Find the 0-based position of a record with the given sequence number.
fn find_sequence_position(
    records: &std::collections::VecDeque<StreamChangeRecord>,
    sequence_number: &str,
) -> Result<u64, DynamoDBStreamsError> {
    records
        .iter()
        .position(|r| r.dynamodb.sequence_number.as_deref() == Some(sequence_number))
        .map(|p| p as u64)
        .ok_or_else(|| {
            DynamoDBStreamsError::trimmed_data_access(
                "The requested sequence number is beyond the trim horizon.",
            )
        })
}

/// Convert an internal `StreamChangeRecord` to the API output `Record` type.
fn record_to_output(record: &StreamChangeRecord) -> Record {
    // Convert internal AttributeValue (DynamoDB model) to Streams model AttributeValue.
    let keys = convert_attribute_map(&record.dynamodb.keys);
    let new_image = record
        .dynamodb
        .new_image
        .as_ref()
        .map(convert_attribute_map);
    let old_image = record
        .dynamodb
        .old_image
        .as_ref()
        .map(convert_attribute_map);

    Record {
        event_id: Some(record.event_id.clone()),
        event_name: Some(match record.event_name.as_str() {
            "MODIFY" => OperationType::Modify,
            "REMOVE" => OperationType::Remove,
            _ => OperationType::Insert,
        }),
        event_version: Some(record.event_version.clone()),
        event_source: Some(record.event_source.clone()),
        aws_region: Some(record.aws_region.clone()),
        dynamodb: Some(StreamRecord {
            keys,
            new_image: new_image.unwrap_or_default(),
            old_image: old_image.unwrap_or_default(),
            sequence_number: record.dynamodb.sequence_number.clone(),
            size_bytes: Some(record.dynamodb.size_bytes.cast_signed()),
            stream_view_type: Some(record.dynamodb.stream_view_type.clone()),
            approximate_creation_date_time: Some(record.dynamodb.approximate_creation_date_time),
        }),
        user_identity: None,
    }
}

/// Convert DynamoDB model `AttributeValue` map to Streams model `AttributeValue` map.
fn convert_attribute_map(
    map: &HashMap<String, AttributeValue>,
) -> HashMap<String, streams_types::AttributeValue> {
    map.iter()
        .map(|(k, v)| (k.clone(), convert_attribute_value(v)))
        .collect()
}

/// Convert a single DynamoDB model `AttributeValue` to Streams model `AttributeValue`.
fn convert_attribute_value(val: &AttributeValue) -> streams_types::AttributeValue {
    match val {
        AttributeValue::S(s) => streams_types::AttributeValue {
            s: Some(s.clone()),
            ..Default::default()
        },
        AttributeValue::N(n) => streams_types::AttributeValue {
            n: Some(n.clone()),
            ..Default::default()
        },
        AttributeValue::B(b) => streams_types::AttributeValue {
            b: Some(b.clone()),
            ..Default::default()
        },
        AttributeValue::Ss(ss) => streams_types::AttributeValue {
            ss: Some(ss.clone()),
            ..Default::default()
        },
        AttributeValue::Ns(ns) => streams_types::AttributeValue {
            ns: Some(ns.clone()),
            ..Default::default()
        },
        AttributeValue::Bs(bs) => streams_types::AttributeValue {
            bs: Some(bs.clone()),
            ..Default::default()
        },
        AttributeValue::Bool(b) => streams_types::AttributeValue {
            bool: Some(*b),
            ..Default::default()
        },
        AttributeValue::Null(n) => streams_types::AttributeValue {
            null: Some(*n),
            ..Default::default()
        },
        AttributeValue::L(l) => streams_types::AttributeValue {
            l: Some(l.iter().map(convert_attribute_value).collect()),
            ..Default::default()
        },
        AttributeValue::M(m) => streams_types::AttributeValue {
            m: Some(convert_attribute_map(m)),
            ..Default::default()
        },
    }
}
