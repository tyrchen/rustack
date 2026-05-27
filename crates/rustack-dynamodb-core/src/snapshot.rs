//! Snapshot support for DynamoDB tables and items.

use std::collections::HashMap;

use rustack_dynamodb_model::{
    AttributeValue,
    types::{
        AttributeDefinition, BillingMode, GlobalSecondaryIndex, KeySchemaElement,
        LocalSecondaryIndex, PointInTimeRecoverySpecification, ProvisionedThroughput,
        SSESpecification, StreamSpecification, TableStatus, Tag, TimeToLiveSpecification,
    },
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    provider::RustackDynamoDB,
    state::DynamoDBTable,
    storage::{KeySchema, StorageError, TableStorage},
};

/// Errors raised while exporting or importing DynamoDB snapshots.
#[derive(Debug, Error)]
pub enum DynamoDBSnapshotError {
    /// Stored item no longer satisfies its table key schema.
    #[error("failed to restore DynamoDB item in table {table}: {source}")]
    RestoreItem {
        /// Table name.
        table: String,
        /// Source error.
        #[source]
        source: StorageError,
    },
    /// Table insertion failed while rebuilding state.
    #[error("failed to restore DynamoDB table {table}: {source}")]
    RestoreTable {
        /// Table name.
        table: String,
        /// Source error.
        #[source]
        source: Box<rustack_dynamodb_model::error::DynamoDBError>,
    },
}

/// Serializable DynamoDB service snapshot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DynamoDBSnapshot {
    /// Table snapshots.
    pub tables: Vec<DynamoDBTableSnapshot>,
}

/// Serializable DynamoDB table snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DynamoDBTableSnapshot {
    /// Table name.
    pub name: String,
    /// Table status.
    pub status: TableStatus,
    /// Key schema elements.
    pub key_schema_elements: Vec<KeySchemaElement>,
    /// Parsed key schema used by storage.
    pub key_schema: KeySchema,
    /// Attribute definitions.
    pub attribute_definitions: Vec<AttributeDefinition>,
    /// Billing mode.
    pub billing_mode: BillingMode,
    /// Provisioned throughput.
    pub provisioned_throughput: Option<ProvisionedThroughput>,
    /// Global secondary index definitions.
    pub gsi_definitions: Vec<GlobalSecondaryIndex>,
    /// Local secondary index definitions.
    pub lsi_definitions: Vec<LocalSecondaryIndex>,
    /// Stream specification.
    pub stream_specification: Option<StreamSpecification>,
    /// SSE specification.
    pub sse_specification: Option<SSESpecification>,
    /// Tags.
    pub tags: Vec<Tag>,
    /// TTL specification.
    pub ttl: Option<TimeToLiveSpecification>,
    /// PITR specification.
    pub point_in_time_recovery: PointInTimeRecoverySpecification,
    /// Table ARN.
    pub arn: String,
    /// Stable table ID.
    pub table_id: String,
    /// Creation timestamp.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Stored items.
    pub items: Vec<HashMap<String, AttributeValue>>,
}

impl RustackDynamoDB {
    /// Export DynamoDB tables and items into a snapshot.
    #[must_use]
    pub fn export_snapshot(&self) -> DynamoDBSnapshot {
        let tables = self
            .state
            .snapshot_tables()
            .into_iter()
            .map(|table| DynamoDBTableSnapshot {
                name: table.name.clone(),
                status: table.status.clone(),
                key_schema_elements: table.key_schema_elements.clone(),
                key_schema: table.key_schema.clone(),
                attribute_definitions: table.attribute_definitions.clone(),
                billing_mode: table.billing_mode.clone(),
                provisioned_throughput: table.provisioned_throughput.clone(),
                gsi_definitions: table.gsi_definitions.clone(),
                lsi_definitions: table.lsi_definitions.clone(),
                stream_specification: table.stream_specification.clone(),
                sse_specification: table.sse_specification.clone(),
                tags: table.tags.read().clone(),
                ttl: table.ttl.read().clone(),
                point_in_time_recovery: table.point_in_time_recovery.read().clone(),
                arn: table.arn.clone(),
                table_id: table.table_id.clone(),
                created_at: table.created_at,
                items: table.storage.snapshot_items(),
            })
            .collect();

        DynamoDBSnapshot { tables }
    }

    /// Import DynamoDB tables and items from a snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error if a stored item cannot be restored into its table.
    pub fn import_snapshot(&self, snapshot: DynamoDBSnapshot) -> Result<(), DynamoDBSnapshotError> {
        self.reset();

        for table_snapshot in snapshot.tables {
            let table_name = table_snapshot.name.clone();
            let storage = TableStorage::new(table_snapshot.key_schema.clone());
            for item in table_snapshot.items {
                storage
                    .put_item(item)
                    .map_err(|source| DynamoDBSnapshotError::RestoreItem {
                        table: table_name.clone(),
                        source,
                    })?;
            }

            let table = DynamoDBTable {
                name: table_snapshot.name.clone(),
                status: table_snapshot.status,
                key_schema_elements: table_snapshot.key_schema_elements,
                key_schema: table_snapshot.key_schema,
                attribute_definitions: table_snapshot.attribute_definitions,
                billing_mode: table_snapshot.billing_mode,
                provisioned_throughput: table_snapshot.provisioned_throughput,
                gsi_definitions: table_snapshot.gsi_definitions,
                lsi_definitions: table_snapshot.lsi_definitions,
                stream_specification: table_snapshot.stream_specification,
                sse_specification: table_snapshot.sse_specification,
                tags: parking_lot::RwLock::new(table_snapshot.tags),
                ttl: parking_lot::RwLock::new(table_snapshot.ttl),
                point_in_time_recovery: parking_lot::RwLock::new(
                    table_snapshot.point_in_time_recovery,
                ),
                arn: table_snapshot.arn,
                table_id: table_snapshot.table_id,
                created_at: table_snapshot.created_at,
                storage,
            };

            self.state.create_table(table).map_err(|source| {
                DynamoDBSnapshotError::RestoreTable {
                    table: table_name,
                    source: Box::new(source),
                }
            })?;
        }

        Ok(())
    }
}
