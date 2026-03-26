//! DynamoDB input types for the 12 MVP operations.
//!
//! All input structs use `PascalCase` JSON field naming to match the DynamoDB
//! wire protocol (`awsJson1_0`). Optional fields are omitted when `None`,
//! empty `HashMap`s and `Vec`s are omitted to produce minimal JSON payloads.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{
    attribute_value::AttributeValue,
    types::{
        AttributeDefinition, AttributeValueUpdate, BillingMode, Condition, ConditionalOperator,
        ExpectedAttributeValue, GlobalSecondaryIndex, KeySchemaElement, KeysAndAttributes,
        LocalSecondaryIndex, ProvisionedThroughput, ReturnConsumedCapacity,
        ReturnItemCollectionMetrics, ReturnValue, SSESpecification, Select, StreamSpecification,
        Tag, TimeToLiveSpecification, TransactGetItem, TransactWriteItem, WriteRequest,
    },
};

// ---------------------------------------------------------------------------
// Table management
// ---------------------------------------------------------------------------

/// Input for the `CreateTable` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CreateTableInput {
    /// The name of the table to create.
    pub table_name: String,

    /// The key schema for the table (partition key and optional sort key).
    pub key_schema: Vec<KeySchemaElement>,

    /// The attribute definitions for the key schema and index key attributes.
    #[serde(default)]
    pub attribute_definitions: Vec<AttributeDefinition>,

    /// The billing mode for the table (`PROVISIONED` or `PAY_PER_REQUEST`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_mode: Option<BillingMode>,

    /// The provisioned throughput settings (required when billing mode is `PROVISIONED`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provisioned_throughput: Option<ProvisionedThroughput>,

    /// Global secondary indexes to create on the table.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub global_secondary_indexes: Vec<GlobalSecondaryIndex>,

    /// Local secondary indexes to create on the table.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub local_secondary_indexes: Vec<LocalSecondaryIndex>,

    /// The stream specification for the table.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_specification: Option<StreamSpecification>,

    /// The server-side encryption specification.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sse_specification: Option<SSESpecification>,

    /// Tags to associate with the table.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Tag>,
}

/// Input for the `DeleteTable` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteTableInput {
    /// The name of the table to delete.
    pub table_name: String,
}

/// Input for the `UpdateTable` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UpdateTableInput {
    /// The name of the table to update.
    pub table_name: String,

    /// The new billing mode for the table.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_mode: Option<BillingMode>,

    /// The new provisioned throughput settings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provisioned_throughput: Option<ProvisionedThroughput>,

    /// New attribute definitions (for GSI changes).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attribute_definitions: Vec<AttributeDefinition>,
}

/// Input for the `DescribeTable` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeTableInput {
    /// The name of the table to describe.
    pub table_name: String,
}

/// Input for the `ListTables` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListTablesInput {
    /// The name of the table that starts the list. Use the value returned in
    /// `LastEvaluatedTableName` from a previous request to continue pagination.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclusive_start_table_name: Option<String>,

    /// The maximum number of table names to return (1--100).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
}

// ---------------------------------------------------------------------------
// Item CRUD
// ---------------------------------------------------------------------------

/// Input for the `PutItem` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PutItemInput {
    /// The name of the table to put the item into.
    pub table_name: String,

    /// A map of attribute name to attribute value, representing the item.
    pub item: HashMap<String, AttributeValue>,

    /// A condition that must be satisfied for the put to succeed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition_expression: Option<String>,

    /// Substitution tokens for attribute names in an expression.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub expression_attribute_names: HashMap<String, String>,

    /// Substitution tokens for attribute values in an expression.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub expression_attribute_values: HashMap<String, AttributeValue>,

    /// Determines the attributes to return after the operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_values: Option<ReturnValue>,

    /// Determines the level of detail about provisioned throughput consumption.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_consumed_capacity: Option<ReturnConsumedCapacity>,

    /// Determines whether item collection metrics are returned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_item_collection_metrics: Option<ReturnItemCollectionMetrics>,

    /// Legacy: expected conditions for conditional writes.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub expected: HashMap<String, ExpectedAttributeValue>,

    /// Legacy: logical operator for combining multiple expected conditions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conditional_operator: Option<ConditionalOperator>,

    /// Determines whether to return the item attributes on a failed condition check.
    /// Valid values: `NONE`, `ALL_OLD`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_values_on_condition_check_failure: Option<String>,
}

/// Input for the `GetItem` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetItemInput {
    /// The name of the table containing the item.
    pub table_name: String,

    /// A map of attribute names to `AttributeValue` objects representing the
    /// primary key of the item to retrieve.
    pub key: HashMap<String, AttributeValue>,

    /// If `true`, a strongly consistent read is used; otherwise, an eventually
    /// consistent read is used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consistent_read: Option<bool>,

    /// A string that identifies the attributes to retrieve from the table.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projection_expression: Option<String>,

    /// Substitution tokens for attribute names in an expression.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub expression_attribute_names: HashMap<String, String>,

    /// Determines the level of detail about provisioned throughput consumption.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_consumed_capacity: Option<ReturnConsumedCapacity>,

    /// Legacy: attribute names to retrieve (use `projection_expression` instead).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributes_to_get: Option<Vec<String>>,
}

/// Input for the `UpdateItem` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UpdateItemInput {
    /// The name of the table containing the item to update.
    pub table_name: String,

    /// The primary key of the item to be updated.
    pub key: HashMap<String, AttributeValue>,

    /// An expression that defines one or more attributes to be updated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_expression: Option<String>,

    /// A condition that must be satisfied for the update to succeed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition_expression: Option<String>,

    /// Substitution tokens for attribute names in an expression.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub expression_attribute_names: HashMap<String, String>,

    /// Substitution tokens for attribute values in an expression.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub expression_attribute_values: HashMap<String, AttributeValue>,

    /// Determines the attributes to return after the operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_values: Option<ReturnValue>,

    /// Determines the level of detail about provisioned throughput consumption.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_consumed_capacity: Option<ReturnConsumedCapacity>,

    /// Determines whether item collection metrics are returned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_item_collection_metrics: Option<ReturnItemCollectionMetrics>,

    /// Legacy: attribute updates map (use `update_expression` instead).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub attribute_updates: HashMap<String, AttributeValueUpdate>,

    /// Legacy: expected conditions for conditional writes.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub expected: HashMap<String, ExpectedAttributeValue>,

    /// Legacy: logical operator for combining multiple expected conditions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conditional_operator: Option<ConditionalOperator>,

    /// Determines whether to return the item attributes on a failed condition check.
    /// Valid values: `NONE`, `ALL_OLD`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_values_on_condition_check_failure: Option<String>,
}

/// Input for the `DeleteItem` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeleteItemInput {
    /// The name of the table from which to delete the item.
    pub table_name: String,

    /// A map of attribute names to `AttributeValue` objects representing the
    /// primary key of the item to delete.
    pub key: HashMap<String, AttributeValue>,

    /// A condition that must be satisfied for the deletion to succeed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition_expression: Option<String>,

    /// Substitution tokens for attribute names in an expression.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub expression_attribute_names: HashMap<String, String>,

    /// Substitution tokens for attribute values in an expression.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub expression_attribute_values: HashMap<String, AttributeValue>,

    /// Determines the attributes to return after the operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_values: Option<ReturnValue>,

    /// Determines the level of detail about provisioned throughput consumption.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_consumed_capacity: Option<ReturnConsumedCapacity>,

    /// Determines whether item collection metrics are returned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_item_collection_metrics: Option<ReturnItemCollectionMetrics>,

    /// Legacy: expected conditions for conditional writes.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub expected: HashMap<String, ExpectedAttributeValue>,

    /// Legacy: logical operator for combining multiple expected conditions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conditional_operator: Option<ConditionalOperator>,

    /// Determines whether to return the item attributes on a failed condition check.
    /// Valid values: `NONE`, `ALL_OLD`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_values_on_condition_check_failure: Option<String>,
}

// ---------------------------------------------------------------------------
// Query & Scan
// ---------------------------------------------------------------------------

/// Input for the `Query` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueryInput {
    /// The name of the table to query.
    pub table_name: String,

    /// The name of a secondary index to query.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_name: Option<String>,

    /// The condition that specifies the key values for items to be retrieved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_condition_expression: Option<String>,

    /// A string that contains conditions for filtering the query results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter_expression: Option<String>,

    /// A string that identifies the attributes to retrieve from the table.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projection_expression: Option<String>,

    /// Substitution tokens for attribute names in an expression.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub expression_attribute_names: HashMap<String, String>,

    /// Substitution tokens for attribute values in an expression.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub expression_attribute_values: HashMap<String, AttributeValue>,

    /// Specifies the order of index traversal. `true` (default) for ascending,
    /// `false` for descending.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scan_index_forward: Option<bool>,

    /// The maximum number of items to evaluate (not necessarily the number of
    /// matching items).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,

    /// The primary key of the first item that this operation will evaluate.
    /// Used for pagination.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub exclusive_start_key: HashMap<String, AttributeValue>,

    /// The attributes to be returned in the result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub select: Option<Select>,

    /// If `true`, a strongly consistent read is used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consistent_read: Option<bool>,

    /// Determines the level of detail about provisioned throughput consumption.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_consumed_capacity: Option<ReturnConsumedCapacity>,

    /// Legacy: key conditions (use `key_condition_expression` instead).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub key_conditions: HashMap<String, Condition>,

    /// Legacy: query filter (use `filter_expression` instead).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub query_filter: HashMap<String, Condition>,

    /// Legacy: attribute names to retrieve (use `projection_expression` instead).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributes_to_get: Option<Vec<String>>,

    /// Legacy: logical operator for combining multiple query filter conditions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conditional_operator: Option<ConditionalOperator>,
}

/// Input for the `Scan` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ScanInput {
    /// The name of the table to scan.
    pub table_name: String,

    /// The name of a secondary index to scan.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_name: Option<String>,

    /// A string that contains conditions for filtering the scan results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter_expression: Option<String>,

    /// A string that identifies the attributes to retrieve from the table.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projection_expression: Option<String>,

    /// Substitution tokens for attribute names in an expression.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub expression_attribute_names: HashMap<String, String>,

    /// Substitution tokens for attribute values in an expression.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub expression_attribute_values: HashMap<String, AttributeValue>,

    /// The maximum number of items to evaluate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,

    /// The primary key of the first item that this operation will evaluate.
    /// Used for pagination.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub exclusive_start_key: HashMap<String, AttributeValue>,

    /// For a parallel `Scan` request, identifies an individual segment to be
    /// scanned by an application worker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment: Option<i32>,

    /// For a parallel `Scan` request, the total number of segments into which
    /// the table is divided.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_segments: Option<i32>,

    /// The attributes to be returned in the result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub select: Option<Select>,

    /// If `true`, a strongly consistent read is used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consistent_read: Option<bool>,

    /// Determines the level of detail about provisioned throughput consumption.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_consumed_capacity: Option<ReturnConsumedCapacity>,

    /// Legacy: scan filter (use `filter_expression` instead).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub scan_filter: HashMap<String, Condition>,

    /// Legacy: attribute names to retrieve (use `projection_expression` instead).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributes_to_get: Option<Vec<String>>,

    /// Legacy: logical operator for combining multiple scan filter conditions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conditional_operator: Option<ConditionalOperator>,
}

// ---------------------------------------------------------------------------
// Batch operations
// ---------------------------------------------------------------------------

/// Input for the `BatchGetItem` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct BatchGetItemInput {
    /// A map of one or more table names to the corresponding keys and
    /// projection expressions to retrieve.
    pub request_items: HashMap<String, KeysAndAttributes>,

    /// Determines the level of detail about provisioned throughput consumption.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_consumed_capacity: Option<ReturnConsumedCapacity>,
}

/// Input for the `BatchWriteItem` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct BatchWriteItemInput {
    /// A map of one or more table names to a list of `WriteRequest` objects
    /// (put or delete operations).
    pub request_items: HashMap<String, Vec<WriteRequest>>,

    /// Determines the level of detail about provisioned throughput consumption.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_consumed_capacity: Option<ReturnConsumedCapacity>,

    /// Determines whether item collection metrics are returned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_item_collection_metrics: Option<ReturnItemCollectionMetrics>,
}

// ---------------------------------------------------------------------------
// Tagging
// ---------------------------------------------------------------------------

/// Input for the `TagResource` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TagResourceInput {
    /// The ARN of the resource to tag.
    pub resource_arn: String,
    /// The tags to add to the resource.
    pub tags: Vec<Tag>,
}

/// Input for the `UntagResource` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UntagResourceInput {
    /// The ARN of the resource to untag.
    pub resource_arn: String,
    /// The tag keys to remove.
    pub tag_keys: Vec<String>,
}

/// Input for the `ListTagsOfResource` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListTagsOfResourceInput {
    /// The ARN of the resource.
    pub resource_arn: String,
    /// An optional pagination token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

// ---------------------------------------------------------------------------
// Time to Live
// ---------------------------------------------------------------------------

/// Input for the `UpdateTimeToLive` operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UpdateTimeToLiveInput {
    /// The name of the table.
    pub table_name: String,
    /// The TTL specification to apply.
    pub time_to_live_specification: TimeToLiveSpecification,
}

/// Input for the `DescribeTimeToLive` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeTimeToLiveInput {
    /// The name of the table.
    pub table_name: String,
}

// ---------------------------------------------------------------------------
// Transactions
// ---------------------------------------------------------------------------

/// Input for the `TransactWriteItems` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TransactWriteItemsInput {
    /// The list of write actions to perform atomically.
    pub transact_items: Vec<TransactWriteItem>,
    /// Determines the level of detail about provisioned throughput consumption.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_consumed_capacity: Option<ReturnConsumedCapacity>,
    /// Determines whether item collection metrics are returned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_item_collection_metrics: Option<ReturnItemCollectionMetrics>,
    /// An idempotency token for the transaction.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_request_token: Option<String>,
}

/// Input for the `TransactGetItems` operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TransactGetItemsInput {
    /// The list of get actions to perform.
    pub transact_items: Vec<TransactGetItem>,
    /// Determines the level of detail about provisioned throughput consumption.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_consumed_capacity: Option<ReturnConsumedCapacity>,
}

// ---------------------------------------------------------------------------
// Describe operations
// ---------------------------------------------------------------------------

/// Input for the `DescribeLimits` operation (empty).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeLimitsInput {}

/// Input for the `DescribeEndpoints` operation (empty).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DescribeEndpointsInput {}
