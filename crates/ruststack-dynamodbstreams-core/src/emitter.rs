//! DynamoDB Streams implementation of the `StreamEmitter` trait.
//!
//! Receives change events from DynamoDB core and appends them to the
//! appropriate table stream in the `StreamStore`.

use std::sync::Arc;

use ruststack_dynamodb_core::stream::{ChangeEvent, StreamEmitter, StreamLifecycle};

use crate::storage::StreamStore;

/// DynamoDB Streams implementation of the `StreamEmitter` trait.
///
/// Receives change events from DynamoDB core and appends them to the
/// appropriate table stream in the `StreamStore`.
#[derive(Debug)]
pub struct DynamoDBStreamEmitter {
    store: Arc<StreamStore>,
    region: String,
}

impl DynamoDBStreamEmitter {
    /// Create a new emitter backed by the given stream store.
    #[must_use]
    pub fn new(store: Arc<StreamStore>, region: String) -> Self {
        Self { store, region }
    }
}

impl StreamEmitter for DynamoDBStreamEmitter {
    fn emit(&self, event: ChangeEvent) {
        self.store.append_change_event(&event, &self.region);
    }
}

/// DynamoDB Streams lifecycle manager.
///
/// Manages the lifecycle of streams alongside DynamoDB tables (create, disable,
/// delete).
#[derive(Debug)]
pub struct DynamoDBStreamLifecycleManager {
    store: Arc<StreamStore>,
    region: String,
    account_id: String,
}

impl DynamoDBStreamLifecycleManager {
    /// Create a new lifecycle manager.
    #[must_use]
    pub fn new(store: Arc<StreamStore>, region: String, account_id: String) -> Self {
        Self {
            store,
            region,
            account_id,
        }
    }
}

impl StreamLifecycle for DynamoDBStreamLifecycleManager {
    fn on_stream_enabled(
        &self,
        table_name: &str,
        table_arn: &str,
        key_schema: Vec<ruststack_dynamodb_model::types::KeySchemaElement>,
        stream_view_type: ruststack_dynamodb_model::types::StreamViewType,
    ) -> String {
        self.store.create_stream(
            table_name,
            table_arn,
            key_schema,
            stream_view_type,
            &self.region,
            &self.account_id,
        )
    }

    fn on_stream_disabled(&self, table_name: &str) {
        self.store.disable_stream(table_name);
    }

    fn on_table_deleted(&self, table_name: &str) {
        self.store.disable_stream(table_name);
        self.store.remove_stream(table_name);
    }

    fn get_stream_arn(&self, table_name: &str) -> Option<String> {
        self.store.get_stream_arn(table_name)
    }

    fn get_stream_label(&self, table_name: &str) -> Option<String> {
        self.store.get_stream_label(table_name)
    }
}
