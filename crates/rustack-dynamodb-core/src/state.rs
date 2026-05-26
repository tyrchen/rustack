//! DynamoDB service state management.

use std::sync::Arc;

use dashmap::DashMap;
use rustack_dynamodb_model::{
    error::DynamoDBError,
    types::{
        AttributeDefinition, BillingMode, BillingModeSummary, GlobalSecondaryIndex,
        GlobalSecondaryIndexDescription, GlobalSecondaryIndexWarmThroughputDescription,
        IndexStatus, KeySchemaElement, LocalSecondaryIndex, LocalSecondaryIndexDescription,
        PointInTimeRecoverySpecification, ProvisionedThroughput, ProvisionedThroughputDescription,
        SSEDescription, SSESpecification, SseStatus, SseType, StreamSpecification,
        TableDescription, TableStatus, TableWarmThroughputDescription, Tag,
        TimeToLiveSpecification,
    },
};

use crate::storage::{KeySchema, TableStorage};

/// Top-level DynamoDB service state.
#[derive(Debug)]
pub struct DynamoDBServiceState {
    /// All tables keyed by name.
    tables: DashMap<String, Arc<DynamoDBTable>>,
}

impl DynamoDBServiceState {
    /// Create a new empty service state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tables: DashMap::new(),
        }
    }

    /// Get a table by name.
    #[must_use]
    pub fn get_table(&self, name: &str) -> Option<Arc<DynamoDBTable>> {
        self.tables.get(name).map(|r| Arc::clone(r.value()))
    }

    /// Get a table or return `ResourceNotFoundException`.
    pub fn require_table(&self, name: &str) -> Result<Arc<DynamoDBTable>, DynamoDBError> {
        self.get_table(name).ok_or_else(|| {
            DynamoDBError::resource_not_found(format!(
                "Requested resource not found: Table: {name} not found"
            ))
        })
    }

    /// Insert a new table. Returns error if table already exists.
    pub fn create_table(&self, table: DynamoDBTable) -> Result<Arc<DynamoDBTable>, DynamoDBError> {
        let name = table.name.clone();
        let table = Arc::new(table);
        // Use entry API to atomically check + insert.
        match self.tables.entry(name) {
            dashmap::mapref::entry::Entry::Occupied(e) => Err(DynamoDBError::resource_in_use(
                format!("Table already exists: {}", e.key()),
            )),
            dashmap::mapref::entry::Entry::Vacant(e) => {
                let table = Arc::clone(&table);
                e.insert(table.clone());
                Ok(table)
            }
        }
    }

    /// Remove a table by name. Returns the removed table.
    pub fn delete_table(&self, name: &str) -> Result<Arc<DynamoDBTable>, DynamoDBError> {
        self.tables.remove(name).map(|(_, t)| t).ok_or_else(|| {
            DynamoDBError::resource_not_found(format!(
                "Requested resource not found: Table: {name} not found"
            ))
        })
    }

    /// List all table names (sorted).
    #[must_use]
    pub fn list_table_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.tables.iter().map(|r| r.key().clone()).collect();
        names.sort();
        names
    }

    /// Reset all state (remove all tables).
    pub fn reset(&self) {
        self.tables.clear();
    }
}

impl Default for DynamoDBServiceState {
    fn default() -> Self {
        Self::new()
    }
}

/// A single DynamoDB table with metadata and storage.
#[derive(Debug)]
pub struct DynamoDBTable {
    /// Table name.
    pub name: String,
    /// Table status.
    pub status: TableStatus,
    /// Key schema elements.
    pub key_schema_elements: Vec<KeySchemaElement>,
    /// Parsed key schema for storage operations.
    pub key_schema: KeySchema,
    /// Attribute definitions.
    pub attribute_definitions: Vec<AttributeDefinition>,
    /// Billing mode.
    pub billing_mode: BillingMode,
    /// Provisioned throughput (accepted but not enforced).
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
    pub tags: parking_lot::RwLock<Vec<Tag>>,
    /// Time-to-Live specification.
    pub ttl: parking_lot::RwLock<Option<TimeToLiveSpecification>>,
    /// Point-in-time recovery specification.
    pub point_in_time_recovery: parking_lot::RwLock<PointInTimeRecoverySpecification>,
    /// Table ARN.
    pub arn: String,
    /// Stable table ID (UUID v4), assigned at creation time.
    pub table_id: String,
    /// Creation timestamp.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Item storage engine.
    pub storage: TableStorage,
}

impl DynamoDBTable {
    /// Build a `TableDescription` from this table's metadata.
    #[must_use]
    pub fn to_description(&self) -> TableDescription {
        #[allow(clippy::cast_precision_loss)] // Acceptable: DynamoDB returns epoch seconds as f64
        let creation_time = self.created_at.timestamp_millis() as f64 / 1000.0;
        TableDescription {
            table_name: Some(self.name.clone()),
            table_status: Some(self.status.clone()),
            key_schema: self.key_schema_elements.clone(),
            attribute_definitions: self.attribute_definitions.clone(),
            table_arn: Some(self.arn.clone()),
            table_id: Some(self.table_id.clone()),
            creation_date_time: Some(creation_time),
            item_count: Some(i64::try_from(self.storage.item_count()).unwrap_or(i64::MAX)),
            table_size_bytes: Some(
                i64::try_from(self.storage.total_size_bytes()).unwrap_or(i64::MAX),
            ),
            billing_mode_summary: Some(BillingModeSummary {
                billing_mode: Some(self.billing_mode.clone()),
                last_update_to_pay_per_request_date_time: Some(creation_time),
            }),
            provisioned_throughput: Some(self.provisioned_throughput_description()),
            warm_throughput: Some(default_table_warm_throughput()),
            global_secondary_indexes: self
                .gsi_definitions
                .iter()
                .map(|gsi| GlobalSecondaryIndexDescription {
                    index_name: Some(gsi.index_name.clone()),
                    key_schema: gsi.key_schema.clone(),
                    projection: Some(gsi.projection.clone()),
                    index_status: Some(IndexStatus::Active),
                    provisioned_throughput: gsi.provisioned_throughput.as_ref().map(|pt| {
                        ProvisionedThroughputDescription {
                            read_capacity_units: pt.read_capacity_units,
                            write_capacity_units: pt.write_capacity_units,
                            ..Default::default()
                        }
                    }),
                    index_size_bytes: Some(0),
                    warm_throughput: Some(default_gsi_warm_throughput()),
                    item_count: Some(0),
                    index_arn: Some(format!("{}/index/{}", self.arn, gsi.index_name)),
                    ..Default::default()
                })
                .collect(),
            local_secondary_indexes: self
                .lsi_definitions
                .iter()
                .map(|lsi| LocalSecondaryIndexDescription {
                    index_name: Some(lsi.index_name.clone()),
                    key_schema: lsi.key_schema.clone(),
                    projection: Some(lsi.projection.clone()),
                    index_size_bytes: Some(0),
                    item_count: Some(0),
                    index_arn: Some(format!("{}/index/{}", self.arn, lsi.index_name)),
                })
                .collect(),
            stream_specification: self.stream_specification.clone(),
            sse_description: self.sse_specification.as_ref().map(|_| SSEDescription {
                status: Some(SseStatus::Enabled),
                sse_type: Some(SseType::Aes256),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// Build a stripped `TableDescription` for the `DeleteTable` response.
    ///
    /// Per DynamoDB specification, `DeleteTable` does not include `KeySchema`,
    /// `AttributeDefinitions`, `CreationDateTime`, `GlobalSecondaryIndexes`,
    /// or `LocalSecondaryIndexes` in its response.
    #[must_use]
    pub fn to_delete_description(&self) -> TableDescription {
        #[allow(clippy::cast_precision_loss)]
        let creation_time = self.created_at.timestamp_millis() as f64 / 1000.0;
        TableDescription {
            table_name: Some(self.name.clone()),
            table_status: Some(TableStatus::Deleting),
            table_arn: Some(self.arn.clone()),
            table_id: Some(self.table_id.clone()),
            item_count: Some(i64::try_from(self.storage.item_count()).unwrap_or(i64::MAX)),
            table_size_bytes: Some(
                i64::try_from(self.storage.total_size_bytes()).unwrap_or(i64::MAX),
            ),
            billing_mode_summary: Some(BillingModeSummary {
                billing_mode: Some(self.billing_mode.clone()),
                last_update_to_pay_per_request_date_time: Some(creation_time),
            }),
            provisioned_throughput: Some(self.provisioned_throughput_description()),
            warm_throughput: Some(default_table_warm_throughput()),
            ..Default::default()
        }
    }

    /// Build the `ProvisionedThroughputDescription` for this table.
    fn provisioned_throughput_description(&self) -> ProvisionedThroughputDescription {
        self.provisioned_throughput.as_ref().map_or_else(
            || ProvisionedThroughputDescription {
                read_capacity_units: 0,
                write_capacity_units: 0,
                number_of_decreases_today: Some(0),
                ..Default::default()
            },
            |pt| ProvisionedThroughputDescription {
                read_capacity_units: pt.read_capacity_units,
                write_capacity_units: pt.write_capacity_units,
                number_of_decreases_today: Some(0),
                ..Default::default()
            },
        )
    }
}

fn default_table_warm_throughput() -> TableWarmThroughputDescription {
    TableWarmThroughputDescription {
        read_units_per_second: Some(12_000),
        write_units_per_second: Some(4_000),
        status: Some(TableStatus::Active),
    }
}

fn default_gsi_warm_throughput() -> GlobalSecondaryIndexWarmThroughputDescription {
    GlobalSecondaryIndexWarmThroughputDescription {
        read_units_per_second: Some(12_000),
        write_units_per_second: Some(4_000),
        status: Some(IndexStatus::Active),
    }
}
