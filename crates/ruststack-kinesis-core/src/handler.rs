//! Kinesis handler implementation bridging HTTP to business logic.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;

use ruststack_kinesis_http::body::KinesisResponseBody;
use ruststack_kinesis_http::dispatch::KinesisHandler;
use ruststack_kinesis_http::response::json_response;
use ruststack_kinesis_model::error::{KinesisError, KinesisErrorCode};
use ruststack_kinesis_model::operations::KinesisOperation;

use crate::provider::RustStackKinesis;

/// Handler that bridges the HTTP layer to the Kinesis provider.
#[derive(Debug)]
pub struct RustStackKinesisHandler {
    provider: Arc<RustStackKinesis>,
}

impl RustStackKinesisHandler {
    /// Create a new handler wrapping a provider.
    #[must_use]
    pub fn new(provider: Arc<RustStackKinesis>) -> Self {
        Self { provider }
    }
}

impl KinesisHandler for RustStackKinesisHandler {
    fn handle_operation(
        &self,
        op: KinesisOperation,
        body: Bytes,
    ) -> Pin<
        Box<dyn Future<Output = Result<http::Response<KinesisResponseBody>, KinesisError>> + Send>,
    > {
        let provider = Arc::clone(&self.provider);
        Box::pin(async move { dispatch(&provider, op, &body).await })
    }
}

/// Dispatch a Kinesis operation to the appropriate handler method.
#[allow(clippy::too_many_lines)]
async fn dispatch(
    provider: &RustStackKinesis,
    op: KinesisOperation,
    body: &[u8],
) -> Result<http::Response<KinesisResponseBody>, KinesisError> {
    let request_id = uuid::Uuid::new_v4().to_string();

    match op {
        // Phase 0 - Core stream operations
        KinesisOperation::CreateStream => {
            let input = deserialize(body)?;
            provider.create_stream(input)?;
            serialize(&serde_json::json!({}), &request_id)
        }
        KinesisOperation::DeleteStream => {
            let input = deserialize(body)?;
            provider.delete_stream(input).await?;
            serialize(&serde_json::json!({}), &request_id)
        }
        KinesisOperation::DescribeStream => {
            let input = deserialize(body)?;
            let output = provider.describe_stream(input)?;
            serialize(&output, &request_id)
        }
        KinesisOperation::DescribeStreamSummary => {
            let input = deserialize(body)?;
            let output = provider.describe_stream_summary(input)?;
            serialize(&output, &request_id)
        }
        KinesisOperation::ListStreams => {
            let input = deserialize(body)?;
            let output = provider.list_streams(input)?;
            serialize(&output, &request_id)
        }
        KinesisOperation::PutRecord => {
            let input = deserialize(body)?;
            let output = provider.put_record(input).await?;
            serialize(&output, &request_id)
        }
        KinesisOperation::PutRecords => {
            let input = deserialize(body)?;
            let output = provider.put_records(input).await?;
            serialize(&output, &request_id)
        }
        KinesisOperation::GetRecords => {
            let input = deserialize(body)?;
            let output = provider.get_records(input).await?;
            serialize(&output, &request_id)
        }
        KinesisOperation::GetShardIterator => {
            let input = deserialize(body)?;
            let output = provider.get_shard_iterator(input).await?;
            serialize(&output, &request_id)
        }
        KinesisOperation::ListShards => {
            let input = deserialize(body)?;
            let output = provider.list_shards(input)?;
            serialize(&output, &request_id)
        }
        KinesisOperation::UpdateShardCount => {
            let input = deserialize(body)?;
            let output = provider.update_shard_count(input)?;
            serialize(&output, &request_id)
        }

        // Phase 1 - Tags
        KinesisOperation::AddTagsToStream => {
            let input = deserialize(body)?;
            provider.add_tags_to_stream(input)?;
            serialize(&serde_json::json!({}), &request_id)
        }
        KinesisOperation::RemoveTagsFromStream => {
            let input = deserialize(body)?;
            provider.remove_tags_from_stream(input)?;
            serialize(&serde_json::json!({}), &request_id)
        }
        KinesisOperation::ListTagsForStream => {
            let input = deserialize(body)?;
            let output = provider.list_tags_for_stream(input)?;
            serialize(&output, &request_id)
        }

        // Phase 1 - Retention
        KinesisOperation::IncreaseStreamRetentionPeriod => {
            let input = deserialize(body)?;
            provider.increase_stream_retention_period(input)?;
            serialize(&serde_json::json!({}), &request_id)
        }
        KinesisOperation::DecreaseStreamRetentionPeriod => {
            let input = deserialize(body)?;
            provider.decrease_stream_retention_period(input)?;
            serialize(&serde_json::json!({}), &request_id)
        }

        // Phase 1 - Split/Merge
        KinesisOperation::SplitShard => {
            let input = deserialize(body)?;
            provider.split_shard(input).await?;
            serialize(&serde_json::json!({}), &request_id)
        }
        KinesisOperation::MergeShards => {
            let input = deserialize(body)?;
            provider.merge_shards(input).await?;
            serialize(&serde_json::json!({}), &request_id)
        }

        // Phase 1 - Encryption
        KinesisOperation::StartStreamEncryption => {
            let input = deserialize(body)?;
            provider.start_stream_encryption(input)?;
            serialize(&serde_json::json!({}), &request_id)
        }
        KinesisOperation::StopStreamEncryption => {
            let input = deserialize(body)?;
            provider.stop_stream_encryption(input)?;
            serialize(&serde_json::json!({}), &request_id)
        }

        // Phase 1 - Limits
        KinesisOperation::DescribeLimits => {
            let input = deserialize(body)?;
            let output = provider.describe_limits(input)?;
            serialize(&output, &request_id)
        }

        // Phase 2 - Enhanced fan-out consumers
        KinesisOperation::RegisterStreamConsumer => {
            let input = deserialize(body)?;
            let output = provider.register_stream_consumer(input)?;
            serialize(&output, &request_id)
        }
        KinesisOperation::DeregisterStreamConsumer => {
            let input = deserialize(body)?;
            provider.deregister_stream_consumer(input)?;
            serialize(&serde_json::json!({}), &request_id)
        }
        KinesisOperation::DescribeStreamConsumer => {
            let input = deserialize(body)?;
            let output = provider.describe_stream_consumer(input)?;
            serialize(&output, &request_id)
        }
        KinesisOperation::ListStreamConsumers => {
            let input = deserialize(body)?;
            let output = provider.list_stream_consumers(input)?;
            serialize(&output, &request_id)
        }
        KinesisOperation::SubscribeToShard => {
            let input = deserialize(body)?;
            provider.subscribe_to_shard(input)?;
            // Unreachable since subscribe_to_shard always errors
            serialize(&serde_json::json!({}), &request_id)
        }

        // Phase 3 - Resource policies
        KinesisOperation::GetResourcePolicy => {
            let input = deserialize(body)?;
            let output = provider.get_resource_policy(input)?;
            serialize(&output, &request_id)
        }
        KinesisOperation::PutResourcePolicy => {
            let input = deserialize(body)?;
            provider.put_resource_policy(input)?;
            serialize(&serde_json::json!({}), &request_id)
        }
        KinesisOperation::DeleteResourcePolicy => {
            let input = deserialize(body)?;
            provider.delete_resource_policy(input)?;
            serialize(&serde_json::json!({}), &request_id)
        }
    }
}

/// Deserialize a JSON request body into the input type.
fn deserialize<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, KinesisError> {
    serde_json::from_slice(body).map_err(|e| {
        let msg = e.to_string();
        if msg.contains("missing field") || msg.contains("unknown variant") {
            KinesisError::with_message(
                KinesisErrorCode::InvalidArgumentException,
                format!("1 validation error detected: {msg}"),
            )
        } else {
            KinesisError::with_message(
                KinesisErrorCode::InternalFailureException,
                format!("Failed to deserialize request body: {e}"),
            )
        }
    })
}

/// Serialize an output type into a JSON HTTP response.
fn serialize<T: serde::Serialize>(
    output: &T,
    request_id: &str,
) -> Result<http::Response<KinesisResponseBody>, KinesisError> {
    let json = serde_json::to_vec(output)
        .map_err(|e| KinesisError::internal_error(format!("Failed to serialize response: {e}")))?;
    Ok(json_response(json, request_id))
}
