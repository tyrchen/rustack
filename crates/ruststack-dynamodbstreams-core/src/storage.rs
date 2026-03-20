//! DynamoDB Streams storage engine.
//!
//! Manages per-table change logs and serves the 4 Streams API operations.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use dashmap::DashMap;
use parking_lot::RwLock;

use ruststack_dynamodb_core::stream::ChangeEvent;
use ruststack_dynamodb_model::AttributeValue;
use ruststack_dynamodbstreams_model::types::{
    KeySchemaElement, KeyType, StreamStatus, StreamViewType,
};

/// Top-level stream store managing all DynamoDB Streams.
///
/// Keyed by table name. Each table with streams enabled has exactly one
/// `TableStream` entry.
#[derive(Debug)]
pub struct StreamStore {
    /// Active streams keyed by table name.
    streams: DashMap<String, TableStream>,
}

/// A single DynamoDB Stream associated with a table.
///
/// Contains the stream metadata and the change log (shards).
#[derive(Debug)]
pub struct TableStream {
    /// Stream ARN.
    pub stream_arn: String,
    /// Table name this stream belongs to.
    pub table_name: String,
    /// Stream label (ISO 8601 timestamp when the stream was created).
    pub stream_label: String,
    /// What information is captured in stream records.
    pub stream_view_type: StreamViewType,
    /// Stream status.
    pub stream_status: StreamStatus,
    /// Table's key schema (needed for DescribeStream response).
    pub key_schema: Vec<KeySchemaElement>,
    /// Table ARN (for DescribeStream response).
    pub table_arn: String,
    /// The single shard for this stream (MVP: one shard per table).
    pub shard: RwLock<ShardRecord>,
}

/// A shard within a DynamoDB Stream.
///
/// For MVP, each table has exactly one shard. The shard is an append-only
/// log of change records with monotonically increasing sequence numbers.
#[derive(Debug)]
pub struct ShardRecord {
    /// Shard ID in DynamoDB Streams format.
    pub shard_id: String,
    /// Parent shard ID (None for the first shard).
    pub parent_shard_id: Option<String>,
    /// Starting sequence number (first record in this shard).
    pub starting_sequence_number: Option<String>,
    /// Ending sequence number (last record; None if shard is open).
    pub ending_sequence_number: Option<String>,
    /// Change records in chronological order.
    pub records: VecDeque<StreamChangeRecord>,
    /// Next sequence number to assign.
    next_sequence_number: AtomicU64,
}

impl ShardRecord {
    /// Create a new open shard with the given ID.
    #[must_use]
    pub fn new(shard_id: String) -> Self {
        Self {
            shard_id,
            parent_shard_id: None,
            starting_sequence_number: None,
            ending_sequence_number: None,
            records: VecDeque::new(),
            next_sequence_number: AtomicU64::new(1),
        }
    }

    /// Append a change record to this shard.
    ///
    /// Assigns a monotonically increasing sequence number and returns it.
    pub fn append(&mut self, mut record: StreamChangeRecord) -> String {
        let seq = self
            .next_sequence_number
            .fetch_add(1, AtomicOrdering::SeqCst);
        let seq_str = format!("{seq:021}");

        record.dynamodb.sequence_number = Some(seq_str.clone());

        if self.starting_sequence_number.is_none() {
            self.starting_sequence_number = Some(seq_str.clone());
        }

        self.records.push_back(record);
        seq_str
    }

    /// Close this shard, setting the ending sequence number.
    pub fn close(&mut self) {
        if let Some(last) = self.records.back() {
            self.ending_sequence_number = last.dynamodb.sequence_number.clone();
        }
    }
}

/// A single change record in a DynamoDB Stream.
///
/// Matches the `Record` structure in the DynamoDB Streams API response.
#[derive(Debug, Clone)]
pub struct StreamChangeRecord {
    /// Unique identifier for this event.
    pub event_id: String,
    /// Type of change: INSERT, MODIFY, or REMOVE.
    pub event_name: String,
    /// Event version (always "1.1").
    pub event_version: String,
    /// Event source (always "aws:dynamodb").
    pub event_source: String,
    /// AWS region.
    pub aws_region: String,
    /// The DynamoDB-specific portion of the record.
    pub dynamodb: StreamRecordData,
}

/// The `dynamodb` field within a stream record.
///
/// Contains the actual item data (keys, images) and metadata.
#[derive(Debug, Clone)]
pub struct StreamRecordData {
    /// The primary key attributes for the affected item.
    pub keys: HashMap<String, AttributeValue>,
    /// The item as it appeared after the modification (for INSERT/MODIFY).
    pub new_image: Option<HashMap<String, AttributeValue>>,
    /// The item as it appeared before the modification (for MODIFY/REMOVE).
    pub old_image: Option<HashMap<String, AttributeValue>>,
    /// Monotonically increasing sequence number within the shard.
    pub sequence_number: Option<String>,
    /// Approximate size of the stream record in bytes.
    pub size_bytes: u64,
    /// The StreamViewType for this record.
    pub stream_view_type: StreamViewType,
    /// Approximate creation date/time (epoch seconds).
    pub approximate_creation_date_time: f64,
}

/// Summary information about a stream for ListStreams.
#[derive(Debug, Clone)]
pub struct StreamSummary {
    /// Stream ARN.
    pub stream_arn: String,
    /// Table name.
    pub table_name: String,
    /// Stream label.
    pub stream_label: String,
}

impl StreamStore {
    /// Create a new empty stream store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            streams: DashMap::new(),
        }
    }

    /// Create a stream for a table.
    ///
    /// Called when `CreateTable` or `UpdateTable` specifies
    /// `StreamSpecification.StreamEnabled = true`.
    #[must_use]
    #[allow(clippy::needless_pass_by_value)]
    pub fn create_stream(
        &self,
        table_name: &str,
        table_arn: &str,
        key_schema: Vec<ruststack_dynamodb_model::types::KeySchemaElement>,
        stream_view_type: ruststack_dynamodb_model::types::StreamViewType,
        region: &str,
        account_id: &str,
    ) -> String {
        let stream_label = generate_stream_label();
        let arn = stream_arn(region, account_id, table_name, &stream_label);
        let shard_id = generate_shard_id();

        // Convert DynamoDB model KeySchemaElement to Streams model KeySchemaElement.
        let streams_key_schema: Vec<KeySchemaElement> = key_schema
            .iter()
            .map(|k| KeySchemaElement {
                attribute_name: k.attribute_name.clone(),
                key_type: match k.key_type {
                    ruststack_dynamodb_model::types::KeyType::Hash => KeyType::Hash,
                    ruststack_dynamodb_model::types::KeyType::Range => KeyType::Range,
                },
            })
            .collect();

        // Convert DynamoDB model StreamViewType to Streams model StreamViewType.
        let converted_view_type = convert_stream_view_type(&stream_view_type);

        let shard = ShardRecord::new(shard_id);

        let stream = TableStream {
            stream_arn: arn.clone(),
            table_name: table_name.to_string(),
            stream_label,
            stream_view_type: converted_view_type,
            stream_status: StreamStatus::Enabled,
            key_schema: streams_key_schema,
            table_arn: table_arn.to_string(),
            shard: RwLock::new(shard),
        };

        self.streams.insert(table_name.to_string(), stream);
        arn
    }

    /// Disable a stream for a table.
    ///
    /// The stream remains readable but no new records are accepted.
    pub fn disable_stream(&self, table_name: &str) {
        if let Some(mut stream) = self.streams.get_mut(table_name) {
            stream.stream_status = StreamStatus::Disabled;
            stream.shard.write().close();
        }
    }

    /// Remove a stream entirely (after `DeleteTable`).
    pub fn remove_stream(&self, table_name: &str) {
        self.streams.remove(table_name);
    }

    /// Append a change event to a table's stream.
    ///
    /// Silently discards the event if the table has no active stream.
    pub fn append_change_event(&self, event: &ChangeEvent, region: &str) {
        if let Some(stream) = self.streams.get(&event.table_name) {
            if stream.stream_status != StreamStatus::Enabled {
                return;
            }

            let record = create_record(&stream, event, region);
            stream.shard.write().append(record);
        }
    }

    /// Get stream info by ARN.
    #[must_use]
    pub fn get_stream_by_arn(
        &self,
        stream_arn: &str,
    ) -> Option<dashmap::mapref::one::Ref<'_, String, TableStream>> {
        // Find the stream by ARN (iterate and match).
        let table_name = self
            .streams
            .iter()
            .find(|entry| entry.value().stream_arn == stream_arn)
            .map(|entry| entry.key().clone())?;

        self.streams.get(&table_name)
    }

    /// List all streams, optionally filtered by table name.
    #[must_use]
    pub fn list_streams(&self, table_name: Option<&str>) -> Vec<StreamSummary> {
        self.streams
            .iter()
            .filter(|entry| table_name.is_none_or(|tn| entry.value().table_name == tn))
            .map(|entry| {
                let stream = entry.value();
                StreamSummary {
                    stream_arn: stream.stream_arn.clone(),
                    table_name: stream.table_name.clone(),
                    stream_label: stream.stream_label.clone(),
                }
            })
            .collect()
    }

    /// Get the stream ARN for a table, if one exists.
    #[must_use]
    pub fn get_stream_arn(&self, table_name: &str) -> Option<String> {
        self.streams.get(table_name).map(|s| s.stream_arn.clone())
    }

    /// Get the stream label for a table, if one exists.
    #[must_use]
    pub fn get_stream_label(&self, table_name: &str) -> Option<String> {
        self.streams.get(table_name).map(|s| s.stream_label.clone())
    }
}

impl Default for StreamStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert a `ChangeEvent` from DynamoDB core into a `StreamChangeRecord`,
/// applying the `StreamViewType` filter.
fn create_record(stream: &TableStream, event: &ChangeEvent, region: &str) -> StreamChangeRecord {
    let (new_image, old_image) = match stream.stream_view_type {
        StreamViewType::KeysOnly => (None, None),
        StreamViewType::NewImage => (event.new_image.clone(), None),
        StreamViewType::OldImage => (None, event.old_image.clone()),
        StreamViewType::NewAndOldImages => (event.new_image.clone(), event.old_image.clone()),
    };

    #[allow(clippy::cast_precision_loss)]
    let approx_time = chrono::Utc::now().timestamp() as f64;

    StreamChangeRecord {
        event_id: uuid::Uuid::new_v4().to_string(),
        event_name: event.event_name.as_str().to_string(),
        event_version: "1.1".to_string(),
        event_source: "aws:dynamodb".to_string(),
        aws_region: region.to_string(),
        dynamodb: StreamRecordData {
            keys: event.keys.clone(),
            new_image,
            old_image,
            sequence_number: None, // Assigned by ShardRecord::append.
            size_bytes: event.size_bytes,
            stream_view_type: stream.stream_view_type.clone(),
            approximate_creation_date_time: approx_time,
        },
    }
}

/// Construct a DynamoDB Streams ARN.
fn stream_arn(region: &str, account_id: &str, table_name: &str, stream_label: &str) -> String {
    format!("arn:aws:dynamodb:{region}:{account_id}:table/{table_name}/stream/{stream_label}")
}

/// Generate a stream label from the current timestamp.
fn generate_stream_label() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3f")
        .to_string()
}

/// Generate a shard ID in DynamoDB Streams format.
fn generate_shard_id() -> String {
    let id = uuid::Uuid::new_v4().to_string();
    format!("shardId-{id}")
}

/// Convert DynamoDB model `StreamViewType` to Streams model `StreamViewType`.
fn convert_stream_view_type(
    svt: &ruststack_dynamodb_model::types::StreamViewType,
) -> StreamViewType {
    match svt {
        ruststack_dynamodb_model::types::StreamViewType::KeysOnly => StreamViewType::KeysOnly,
        ruststack_dynamodb_model::types::StreamViewType::NewImage => StreamViewType::NewImage,
        ruststack_dynamodb_model::types::StreamViewType::OldImage => StreamViewType::OldImage,
        ruststack_dynamodb_model::types::StreamViewType::NewAndOldImages => {
            StreamViewType::NewAndOldImages
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruststack_dynamodb_core::stream::ChangeEventName;

    #[test]
    fn test_should_create_and_list_streams() {
        let store = StreamStore::new();
        let arn = store.create_stream(
            "TestTable",
            "arn:aws:dynamodb:us-east-1:000000000000:table/TestTable",
            vec![ruststack_dynamodb_model::types::KeySchemaElement {
                attribute_name: "pk".to_string(),
                key_type: ruststack_dynamodb_model::types::KeyType::Hash,
            }],
            ruststack_dynamodb_model::types::StreamViewType::NewAndOldImages,
            "us-east-1",
            "000000000000",
        );

        assert!(arn.contains("TestTable"));
        let streams = store.list_streams(None);
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].table_name, "TestTable");
    }

    #[test]
    fn test_should_append_and_read_records() {
        let store = StreamStore::new();
        let _ = store.create_stream(
            "TestTable",
            "arn:aws:dynamodb:us-east-1:000000000000:table/TestTable",
            vec![],
            ruststack_dynamodb_model::types::StreamViewType::NewAndOldImages,
            "us-east-1",
            "000000000000",
        );

        let event = ChangeEvent {
            table_name: "TestTable".to_string(),
            event_name: ChangeEventName::Insert,
            keys: HashMap::from([("pk".to_string(), AttributeValue::S("val".to_string()))]),
            old_image: None,
            new_image: Some(HashMap::from([(
                "pk".to_string(),
                AttributeValue::S("val".to_string()),
            )])),
            size_bytes: 10,
        };

        store.append_change_event(&event, "us-east-1");

        let stream = store
            .get_stream_by_arn(&store.list_streams(None)[0].stream_arn)
            .unwrap();
        let shard = stream.shard.read();
        assert_eq!(shard.records.len(), 1);
        assert_eq!(shard.records[0].event_name, "INSERT");
        assert_eq!(
            shard.records[0].dynamodb.sequence_number,
            Some("000000000000000000001".to_string()),
        );
    }

    #[test]
    fn test_should_filter_images_keys_only() {
        let store = StreamStore::new();
        let _ = store.create_stream(
            "T",
            "arn:aws:dynamodb:us-east-1:0:table/T",
            vec![],
            ruststack_dynamodb_model::types::StreamViewType::KeysOnly,
            "us-east-1",
            "0",
        );

        let event = ChangeEvent {
            table_name: "T".to_string(),
            event_name: ChangeEventName::Insert,
            keys: HashMap::from([("pk".to_string(), AttributeValue::S("v".to_string()))]),
            old_image: Some(HashMap::new()),
            new_image: Some(HashMap::from([(
                "pk".to_string(),
                AttributeValue::S("v".to_string()),
            )])),
            size_bytes: 5,
        };

        store.append_change_event(&event, "us-east-1");

        let stream = store
            .get_stream_by_arn(&store.list_streams(None)[0].stream_arn)
            .unwrap();
        let shard = stream.shard.read();
        assert!(shard.records[0].dynamodb.new_image.is_none());
        assert!(shard.records[0].dynamodb.old_image.is_none());
    }

    #[test]
    fn test_should_disable_stream() {
        let store = StreamStore::new();
        let _ = store.create_stream(
            "T",
            "arn:aws:dynamodb:us-east-1:0:table/T",
            vec![],
            ruststack_dynamodb_model::types::StreamViewType::NewAndOldImages,
            "us-east-1",
            "0",
        );

        store.disable_stream("T");

        let event = ChangeEvent {
            table_name: "T".to_string(),
            event_name: ChangeEventName::Insert,
            keys: HashMap::new(),
            old_image: None,
            new_image: None,
            size_bytes: 0,
        };

        // Should be silently discarded.
        store.append_change_event(&event, "us-east-1");

        let stream = store
            .get_stream_by_arn(&store.list_streams(None)[0].stream_arn)
            .unwrap();
        let shard = stream.shard.read();
        assert!(shard.records.is_empty());
    }
}
