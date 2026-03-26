//! DynamoDB operation enum.

use std::fmt;

/// All supported DynamoDB operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DynamoDBOperation {
    // Table management
    /// Create a new table.
    CreateTable,
    /// Delete a table.
    DeleteTable,
    /// Update a table's settings.
    UpdateTable,
    /// Describe a table.
    DescribeTable,
    /// List all tables.
    ListTables,

    // Item CRUD
    /// Put (insert or replace) an item.
    PutItem,
    /// Get an item by primary key.
    GetItem,
    /// Update an item.
    UpdateItem,
    /// Delete an item by primary key.
    DeleteItem,

    // Query & Scan
    /// Query items by key condition.
    Query,
    /// Scan all items in a table.
    Scan,

    // Batch operations
    /// Batch get items from multiple tables.
    BatchGetItem,
    /// Batch write (put/delete) items to multiple tables.
    BatchWriteItem,

    // Tagging
    /// Add tags to a resource.
    TagResource,
    /// Remove tags from a resource.
    UntagResource,
    /// List tags for a resource.
    ListTagsOfResource,

    // Time to Live
    /// Describe the TTL settings for a table.
    DescribeTimeToLive,
    /// Update the TTL settings for a table.
    UpdateTimeToLive,

    // Transactions
    /// Get items atomically across tables.
    TransactGetItems,
    /// Write items atomically across tables.
    TransactWriteItems,

    // Describe
    /// Describe account limits for DynamoDB.
    DescribeLimits,
    /// Describe regional endpoints for DynamoDB.
    DescribeEndpoints,
}

impl DynamoDBOperation {
    /// Returns the AWS operation name string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CreateTable => "CreateTable",
            Self::DeleteTable => "DeleteTable",
            Self::UpdateTable => "UpdateTable",
            Self::DescribeTable => "DescribeTable",
            Self::ListTables => "ListTables",
            Self::PutItem => "PutItem",
            Self::GetItem => "GetItem",
            Self::UpdateItem => "UpdateItem",
            Self::DeleteItem => "DeleteItem",
            Self::Query => "Query",
            Self::Scan => "Scan",
            Self::BatchGetItem => "BatchGetItem",
            Self::BatchWriteItem => "BatchWriteItem",
            Self::TagResource => "TagResource",
            Self::UntagResource => "UntagResource",
            Self::ListTagsOfResource => "ListTagsOfResource",
            Self::DescribeTimeToLive => "DescribeTimeToLive",
            Self::UpdateTimeToLive => "UpdateTimeToLive",
            Self::TransactGetItems => "TransactGetItems",
            Self::TransactWriteItems => "TransactWriteItems",
            Self::DescribeLimits => "DescribeLimits",
            Self::DescribeEndpoints => "DescribeEndpoints",
        }
    }

    /// Parse an operation name string into a `DynamoDBOperation`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "CreateTable" => Some(Self::CreateTable),
            "DeleteTable" => Some(Self::DeleteTable),
            "UpdateTable" => Some(Self::UpdateTable),
            "DescribeTable" => Some(Self::DescribeTable),
            "ListTables" => Some(Self::ListTables),
            "PutItem" => Some(Self::PutItem),
            "GetItem" => Some(Self::GetItem),
            "UpdateItem" => Some(Self::UpdateItem),
            "DeleteItem" => Some(Self::DeleteItem),
            "Query" => Some(Self::Query),
            "Scan" => Some(Self::Scan),
            "BatchGetItem" => Some(Self::BatchGetItem),
            "BatchWriteItem" => Some(Self::BatchWriteItem),
            "TagResource" => Some(Self::TagResource),
            "UntagResource" => Some(Self::UntagResource),
            "ListTagsOfResource" => Some(Self::ListTagsOfResource),
            "DescribeTimeToLive" => Some(Self::DescribeTimeToLive),
            "UpdateTimeToLive" => Some(Self::UpdateTimeToLive),
            "TransactGetItems" => Some(Self::TransactGetItems),
            "TransactWriteItems" => Some(Self::TransactWriteItems),
            "DescribeLimits" => Some(Self::DescribeLimits),
            "DescribeEndpoints" => Some(Self::DescribeEndpoints),
            _ => None,
        }
    }
}

impl fmt::Display for DynamoDBOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
