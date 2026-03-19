//! Auto-generated from AWS Kinesis Smithy model. DO NOT EDIT.

/// All supported Kinesis operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KinesisOperation {
    /// The CreateStream operation.
    CreateStream,
    /// The DeleteStream operation.
    DeleteStream,
    /// The DescribeStream operation.
    DescribeStream,
    /// The DescribeStreamSummary operation.
    DescribeStreamSummary,
    /// The ListStreams operation.
    ListStreams,
    /// The UpdateShardCount operation.
    UpdateShardCount,
    /// The PutRecord operation.
    PutRecord,
    /// The PutRecords operation.
    PutRecords,
    /// The GetRecords operation.
    GetRecords,
    /// The GetShardIterator operation.
    GetShardIterator,
    /// The ListShards operation.
    ListShards,
    /// The AddTagsToStream operation.
    AddTagsToStream,
    /// The RemoveTagsFromStream operation.
    RemoveTagsFromStream,
    /// The ListTagsForStream operation.
    ListTagsForStream,
    /// The IncreaseStreamRetentionPeriod operation.
    IncreaseStreamRetentionPeriod,
    /// The DecreaseStreamRetentionPeriod operation.
    DecreaseStreamRetentionPeriod,
    /// The MergeShards operation.
    MergeShards,
    /// The SplitShard operation.
    SplitShard,
    /// The StartStreamEncryption operation.
    StartStreamEncryption,
    /// The StopStreamEncryption operation.
    StopStreamEncryption,
    /// The DescribeLimits operation.
    DescribeLimits,
    /// The RegisterStreamConsumer operation.
    RegisterStreamConsumer,
    /// The DeregisterStreamConsumer operation.
    DeregisterStreamConsumer,
    /// The DescribeStreamConsumer operation.
    DescribeStreamConsumer,
    /// The ListStreamConsumers operation.
    ListStreamConsumers,
    /// The SubscribeToShard operation.
    SubscribeToShard,
    /// The GetResourcePolicy operation.
    GetResourcePolicy,
    /// The PutResourcePolicy operation.
    PutResourcePolicy,
    /// The DeleteResourcePolicy operation.
    DeleteResourcePolicy,
}

impl KinesisOperation {
    /// Returns the AWS operation name string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CreateStream => "CreateStream",
            Self::DeleteStream => "DeleteStream",
            Self::DescribeStream => "DescribeStream",
            Self::DescribeStreamSummary => "DescribeStreamSummary",
            Self::ListStreams => "ListStreams",
            Self::UpdateShardCount => "UpdateShardCount",
            Self::PutRecord => "PutRecord",
            Self::PutRecords => "PutRecords",
            Self::GetRecords => "GetRecords",
            Self::GetShardIterator => "GetShardIterator",
            Self::ListShards => "ListShards",
            Self::AddTagsToStream => "AddTagsToStream",
            Self::RemoveTagsFromStream => "RemoveTagsFromStream",
            Self::ListTagsForStream => "ListTagsForStream",
            Self::IncreaseStreamRetentionPeriod => "IncreaseStreamRetentionPeriod",
            Self::DecreaseStreamRetentionPeriod => "DecreaseStreamRetentionPeriod",
            Self::MergeShards => "MergeShards",
            Self::SplitShard => "SplitShard",
            Self::StartStreamEncryption => "StartStreamEncryption",
            Self::StopStreamEncryption => "StopStreamEncryption",
            Self::DescribeLimits => "DescribeLimits",
            Self::RegisterStreamConsumer => "RegisterStreamConsumer",
            Self::DeregisterStreamConsumer => "DeregisterStreamConsumer",
            Self::DescribeStreamConsumer => "DescribeStreamConsumer",
            Self::ListStreamConsumers => "ListStreamConsumers",
            Self::SubscribeToShard => "SubscribeToShard",
            Self::GetResourcePolicy => "GetResourcePolicy",
            Self::PutResourcePolicy => "PutResourcePolicy",
            Self::DeleteResourcePolicy => "DeleteResourcePolicy",
        }
    }

    /// Parse an operation name string into an KinesisOperation.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "CreateStream" => Some(Self::CreateStream),
            "DeleteStream" => Some(Self::DeleteStream),
            "DescribeStream" => Some(Self::DescribeStream),
            "DescribeStreamSummary" => Some(Self::DescribeStreamSummary),
            "ListStreams" => Some(Self::ListStreams),
            "UpdateShardCount" => Some(Self::UpdateShardCount),
            "PutRecord" => Some(Self::PutRecord),
            "PutRecords" => Some(Self::PutRecords),
            "GetRecords" => Some(Self::GetRecords),
            "GetShardIterator" => Some(Self::GetShardIterator),
            "ListShards" => Some(Self::ListShards),
            "AddTagsToStream" => Some(Self::AddTagsToStream),
            "RemoveTagsFromStream" => Some(Self::RemoveTagsFromStream),
            "ListTagsForStream" => Some(Self::ListTagsForStream),
            "IncreaseStreamRetentionPeriod" => Some(Self::IncreaseStreamRetentionPeriod),
            "DecreaseStreamRetentionPeriod" => Some(Self::DecreaseStreamRetentionPeriod),
            "MergeShards" => Some(Self::MergeShards),
            "SplitShard" => Some(Self::SplitShard),
            "StartStreamEncryption" => Some(Self::StartStreamEncryption),
            "StopStreamEncryption" => Some(Self::StopStreamEncryption),
            "DescribeLimits" => Some(Self::DescribeLimits),
            "RegisterStreamConsumer" => Some(Self::RegisterStreamConsumer),
            "DeregisterStreamConsumer" => Some(Self::DeregisterStreamConsumer),
            "DescribeStreamConsumer" => Some(Self::DescribeStreamConsumer),
            "ListStreamConsumers" => Some(Self::ListStreamConsumers),
            "SubscribeToShard" => Some(Self::SubscribeToShard),
            "GetResourcePolicy" => Some(Self::GetResourcePolicy),
            "PutResourcePolicy" => Some(Self::PutResourcePolicy),
            "DeleteResourcePolicy" => Some(Self::DeleteResourcePolicy),
            _ => None,
        }
    }
}

impl std::fmt::Display for KinesisOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
