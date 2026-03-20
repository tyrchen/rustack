//! Auto-generated from AWS DynamoDB Streams Smithy model. DO NOT EDIT.

/// All supported DynamoDBStreams operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DynamoDBStreamsOperation {
    /// The DescribeStream operation.
    DescribeStream,
    /// The GetShardIterator operation.
    GetShardIterator,
    /// The GetRecords operation.
    GetRecords,
    /// The ListStreams operation.
    ListStreams,
}

impl DynamoDBStreamsOperation {
    /// Returns the AWS operation name string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DescribeStream => "DescribeStream",
            Self::GetShardIterator => "GetShardIterator",
            Self::GetRecords => "GetRecords",
            Self::ListStreams => "ListStreams",
        }
    }

    /// Parse an operation name string into an DynamoDBStreamsOperation.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "DescribeStream" => Some(Self::DescribeStream),
            "GetShardIterator" => Some(Self::GetShardIterator),
            "GetRecords" => Some(Self::GetRecords),
            "ListStreams" => Some(Self::ListStreams),
            _ => None,
        }
    }
}

impl std::fmt::Display for DynamoDBStreamsOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
