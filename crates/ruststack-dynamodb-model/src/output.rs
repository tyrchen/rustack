//! DynamoDB output types for the 12 MVP operations.
//!
//! All output structs use `PascalCase` JSON field naming to match the DynamoDB
//! wire protocol (`awsJson1_0`). Optional fields are omitted when `None`,
//! empty `HashMap`s and `Vec`s are omitted to produce minimal JSON responses.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{
    attribute_value::AttributeValue,
    types::{
        ConsumedCapacity, ItemCollectionMetrics, ItemResponse, KeysAndAttributes, TableDescription,
        Tag, TimeToLiveDescription, TimeToLiveSpecification, WriteRequest,
    },
};

// ---------------------------------------------------------------------------
// Table management
// ---------------------------------------------------------------------------

/// Output for the `CreateTable` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CreateTableOutput {
    /// The properties of the newly created table.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_description: Option<TableDescription>,
}

/// Output for the `DeleteTable` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteTableOutput {
    /// The properties of the table that was deleted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_description: Option<TableDescription>,
}

/// Output for the `DescribeTable` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeTableOutput {
    /// The properties of the table.
    #[serde(rename = "Table", skip_serializing_if = "Option::is_none")]
    pub table: Option<TableDescription>,
}

/// Output for the `UpdateTable` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UpdateTableOutput {
    /// The properties of the updated table.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_description: Option<TableDescription>,
}

/// Output for the `ListTables` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListTablesOutput {
    /// The names of the tables associated with the current account and region.
    #[serde(default)]
    pub table_names: Vec<String>,

    /// The name of the last table in the current page of results. Use this
    /// value as `ExclusiveStartTableName` in a subsequent request to continue.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_evaluated_table_name: Option<String>,
}

// ---------------------------------------------------------------------------
// Item CRUD
// ---------------------------------------------------------------------------

/// Output for the `PutItem` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutItemOutput {
    /// The attribute values as they appeared before the `PutItem` operation
    /// (only returned when `ReturnValues` is specified).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub attributes: HashMap<String, AttributeValue>,

    /// The capacity units consumed by the operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consumed_capacity: Option<ConsumedCapacity>,

    /// Information about item collections modified by the operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_collection_metrics: Option<ItemCollectionMetrics>,
}

/// Output for the `GetItem` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetItemOutput {
    /// A map of attribute names to `AttributeValue` objects for the retrieved
    /// item. Returns `None` if the item does not exist.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<HashMap<String, AttributeValue>>,

    /// The capacity units consumed by the operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consumed_capacity: Option<ConsumedCapacity>,
}

/// Output for the `UpdateItem` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UpdateItemOutput {
    /// The attribute values as they appeared before or after the update
    /// (depending on the `ReturnValues` setting).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub attributes: HashMap<String, AttributeValue>,

    /// The capacity units consumed by the operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consumed_capacity: Option<ConsumedCapacity>,

    /// Information about item collections modified by the operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_collection_metrics: Option<ItemCollectionMetrics>,
}

/// Output for the `DeleteItem` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteItemOutput {
    /// The attribute values as they appeared before the deletion (only
    /// returned when `ReturnValues` is `ALL_OLD`).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub attributes: HashMap<String, AttributeValue>,

    /// The capacity units consumed by the operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consumed_capacity: Option<ConsumedCapacity>,

    /// Information about item collections modified by the operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_collection_metrics: Option<ItemCollectionMetrics>,
}

// ---------------------------------------------------------------------------
// Query & Scan
// ---------------------------------------------------------------------------

/// Output for the `Query` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueryOutput {
    /// An array of item attributes that match the query conditions.
    /// `None` when `Select=COUNT` (omitted from JSON), `Some(vec)` otherwise
    /// (always serialized, even when empty).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<HashMap<String, AttributeValue>>>,

    /// The number of items in the response.
    pub count: i32,

    /// The number of items evaluated before the filter expression was applied.
    pub scanned_count: i32,

    /// The primary key of the item where the query operation stopped. Use this
    /// value as `ExclusiveStartKey` in a subsequent query to continue.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub last_evaluated_key: HashMap<String, AttributeValue>,

    /// The capacity units consumed by the operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consumed_capacity: Option<ConsumedCapacity>,
}

/// Output for the `Scan` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ScanOutput {
    /// An array of item attributes that match the scan conditions.
    /// `None` when `Select=COUNT` (omitted from JSON), `Some(vec)` otherwise
    /// (always serialized, even when empty).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<HashMap<String, AttributeValue>>>,

    /// The number of items in the response.
    pub count: i32,

    /// The number of items evaluated before the filter expression was applied.
    pub scanned_count: i32,

    /// The primary key of the item where the scan operation stopped. Use this
    /// value as `ExclusiveStartKey` in a subsequent scan to continue.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub last_evaluated_key: HashMap<String, AttributeValue>,

    /// The capacity units consumed by the operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consumed_capacity: Option<ConsumedCapacity>,
}

// ---------------------------------------------------------------------------
// Batch operations
// ---------------------------------------------------------------------------

/// Output for the `BatchGetItem` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct BatchGetItemOutput {
    /// A map of table names to the items retrieved from each table.
    #[serde(default)]
    pub responses: HashMap<String, Vec<HashMap<String, AttributeValue>>>,

    /// A map of tables and their respective keys that were not processed. Use
    /// these values as `RequestItems` in a subsequent `BatchGetItem` call.
    #[serde(default)]
    pub unprocessed_keys: HashMap<String, KeysAndAttributes>,

    /// The capacity units consumed by the operation for each table.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub consumed_capacity: Vec<ConsumedCapacity>,
}

/// Output for the `BatchWriteItem` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct BatchWriteItemOutput {
    /// A map of tables and their respective `WriteRequest` objects that were
    /// not processed. Use these values as `RequestItems` in a subsequent
    /// `BatchWriteItem` call.
    #[serde(default)]
    pub unprocessed_items: HashMap<String, Vec<WriteRequest>>,

    /// A map of tables to item collection metrics for the tables that were
    /// affected by the operation.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub item_collection_metrics: HashMap<String, Vec<ItemCollectionMetrics>>,

    /// The capacity units consumed by the operation for each table.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub consumed_capacity: Vec<ConsumedCapacity>,
}

// ---------------------------------------------------------------------------
// Tagging
// ---------------------------------------------------------------------------

/// Output for the `TagResource` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TagResourceOutput {}

/// Output for the `UntagResource` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UntagResourceOutput {}

/// Output for the `ListTagsOfResource` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListTagsOfResourceOutput {
    /// The tags associated with the resource.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<Tag>>,
    /// A pagination token for subsequent requests.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

// ---------------------------------------------------------------------------
// Time to Live
// ---------------------------------------------------------------------------

/// Output for the `UpdateTimeToLive` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UpdateTimeToLiveOutput {
    /// The TTL specification that was applied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_to_live_specification: Option<TimeToLiveSpecification>,
}

/// Output for the `DescribeTimeToLive` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeTimeToLiveOutput {
    /// The current TTL description for the table.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_to_live_description: Option<TimeToLiveDescription>,
}

// ---------------------------------------------------------------------------
// Transactions
// ---------------------------------------------------------------------------

/// Output for the `TransactWriteItems` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TransactWriteItemsOutput {
    /// The capacity units consumed by the operation for each table.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub consumed_capacity: Vec<ConsumedCapacity>,
    /// Item collection metrics for the affected tables.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub item_collection_metrics: HashMap<String, Vec<ItemCollectionMetrics>>,
}

/// Output for the `TransactGetItems` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TransactGetItemsOutput {
    /// The capacity units consumed by the operation for each table.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub consumed_capacity: Vec<ConsumedCapacity>,
    /// The items retrieved, in the same order as the input.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub responses: Option<Vec<ItemResponse>>,
}

// ---------------------------------------------------------------------------
// Describe operations
// ---------------------------------------------------------------------------

/// Output for the `DescribeLimits` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeLimitsOutput {
    /// Maximum read capacity units per account.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_max_read_capacity_units: Option<i64>,
    /// Maximum write capacity units per account.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_max_write_capacity_units: Option<i64>,
    /// Maximum read capacity units per table.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_max_read_capacity_units: Option<i64>,
    /// Maximum write capacity units per table.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_max_write_capacity_units: Option<i64>,
}

/// Output for the `DescribeEndpoints` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeEndpointsOutput {
    /// The list of endpoints.
    pub endpoints: Vec<Endpoint>,
}

/// A DynamoDB endpoint descriptor.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Endpoint {
    /// The endpoint address.
    pub address: String,
    /// The cache period in minutes.
    pub cache_period_in_minutes: i64,
}
