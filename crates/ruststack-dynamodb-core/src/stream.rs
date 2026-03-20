//! Stream emitter trait and change event types for DynamoDB Streams integration.
//!
//! DynamoDB core calls [`StreamEmitter::emit`] after each successful write
//! operation to capture change data for DynamoDB Streams. The trait is defined
//! here (in `ruststack-dynamodb-core`) and implemented in
//! `ruststack-dynamodbstreams-core` to follow the dependency inversion
//! principle.

use std::collections::HashMap;

use ruststack_dynamodb_model::AttributeValue;

/// Event name for a stream change record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeEventName {
    /// A new item was inserted.
    Insert,
    /// An existing item was modified.
    Modify,
    /// An item was removed.
    Remove,
}

impl ChangeEventName {
    /// Returns the DynamoDB Streams wire-format string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Insert => "INSERT",
            Self::Modify => "MODIFY",
            Self::Remove => "REMOVE",
        }
    }
}

/// A change event emitted by DynamoDB core after a successful write.
///
/// Contains all the information needed to produce a DynamoDB Streams record.
/// The `StreamViewType` filtering is applied by the consumer (Streams core),
/// not the emitter. DynamoDB core always provides both old and new images when
/// available; the Streams consumer strips fields based on the table's
/// `StreamViewType` configuration.
#[derive(Debug, Clone)]
pub struct ChangeEvent {
    /// The table name.
    pub table_name: String,
    /// The event type.
    pub event_name: ChangeEventName,
    /// The primary key attributes of the affected item.
    pub keys: HashMap<String, AttributeValue>,
    /// The item as it appeared before the write (None for INSERT).
    pub old_image: Option<HashMap<String, AttributeValue>>,
    /// The item as it appeared after the write (None for REMOVE).
    pub new_image: Option<HashMap<String, AttributeValue>>,
    /// Approximate size of the affected item in bytes.
    pub size_bytes: u64,
}

/// Trait for emitting DynamoDB change events to a stream consumer.
///
/// DynamoDB core calls `emit` after each successful write operation.
/// The implementation is provided by `ruststack-dynamodbstreams-core` and
/// wired in by the server binary.
///
/// This trait is defined in `ruststack-dynamodb-core` to avoid a dependency
/// from DynamoDB core on the Streams crate (dependency inversion).
pub trait StreamEmitter: Send + Sync + 'static {
    /// Emit a change event for a successful write operation.
    ///
    /// This method must not block. If the stream is disabled for the table,
    /// the implementation should silently discard the event.
    fn emit(&self, event: ChangeEvent);
}

/// A no-op emitter that discards all events.
///
/// Used when DynamoDB Streams is not enabled (feature gate off or no
/// stream configured for the table).
#[derive(Debug)]
pub struct NoopStreamEmitter;

impl StreamEmitter for NoopStreamEmitter {
    fn emit(&self, _event: ChangeEvent) {
        // Intentionally empty.
    }
}

/// A lifecycle manager for DynamoDB Streams.
///
/// Observes DynamoDB table creation/update/deletion and manages
/// corresponding streams in the `StreamStore`.
pub trait StreamLifecycle: Send + Sync + 'static {
    /// Called after a successful `CreateTable` or `UpdateTable` that enables streaming.
    /// Returns the stream ARN.
    fn on_stream_enabled(
        &self,
        table_name: &str,
        table_arn: &str,
        key_schema: Vec<ruststack_dynamodb_model::types::KeySchemaElement>,
        stream_view_type: ruststack_dynamodb_model::types::StreamViewType,
    ) -> String;

    /// Called after `UpdateTable` that disables streaming.
    fn on_stream_disabled(&self, table_name: &str);

    /// Called after `DeleteTable`.
    fn on_table_deleted(&self, table_name: &str);

    /// Get the stream ARN for a table, if one exists.
    fn get_stream_arn(&self, table_name: &str) -> Option<String>;

    /// Get the stream label for a table, if one exists.
    fn get_stream_label(&self, table_name: &str) -> Option<String>;
}

/// A no-op lifecycle manager that does nothing.
#[derive(Debug)]
pub struct NoopStreamLifecycle;

impl StreamLifecycle for NoopStreamLifecycle {
    fn on_stream_enabled(
        &self,
        _table_name: &str,
        _table_arn: &str,
        _key_schema: Vec<ruststack_dynamodb_model::types::KeySchemaElement>,
        _stream_view_type: ruststack_dynamodb_model::types::StreamViewType,
    ) -> String {
        String::new()
    }

    fn on_stream_disabled(&self, _table_name: &str) {}
    fn on_table_deleted(&self, _table_name: &str) {}

    fn get_stream_arn(&self, _table_name: &str) -> Option<String> {
        None
    }

    fn get_stream_label(&self, _table_name: &str) -> Option<String> {
        None
    }
}
