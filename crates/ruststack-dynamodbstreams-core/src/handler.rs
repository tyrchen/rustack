//! DynamoDB Streams handler implementation bridging HTTP to business logic.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;

use ruststack_dynamodbstreams_http::body::DynamoDBStreamsResponseBody;
use ruststack_dynamodbstreams_http::dispatch::DynamoDBStreamsHandler;
use ruststack_dynamodbstreams_http::response::json_response;
use ruststack_dynamodbstreams_model::error::DynamoDBStreamsError;
use ruststack_dynamodbstreams_model::operations::DynamoDBStreamsOperation;

use crate::provider::RustStackDynamoDBStreams;

/// Handler that bridges the HTTP layer to the DynamoDB Streams provider.
#[derive(Debug)]
pub struct RustStackDynamoDBStreamsHandler {
    provider: Arc<RustStackDynamoDBStreams>,
}

impl RustStackDynamoDBStreamsHandler {
    /// Create a new handler wrapping a provider.
    #[must_use]
    pub fn new(provider: Arc<RustStackDynamoDBStreams>) -> Self {
        Self { provider }
    }
}

impl DynamoDBStreamsHandler for RustStackDynamoDBStreamsHandler {
    fn handle_operation(
        &self,
        op: DynamoDBStreamsOperation,
        body: Bytes,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        http::Response<DynamoDBStreamsResponseBody>,
                        DynamoDBStreamsError,
                    >,
                > + Send,
        >,
    > {
        let provider = Arc::clone(&self.provider);
        Box::pin(async move { dispatch(provider.as_ref(), op, &body) })
    }
}

/// Dispatch a DynamoDB Streams operation to the appropriate handler method.
fn dispatch(
    provider: &RustStackDynamoDBStreams,
    op: DynamoDBStreamsOperation,
    body: &[u8],
) -> Result<http::Response<DynamoDBStreamsResponseBody>, DynamoDBStreamsError> {
    let request_id = uuid::Uuid::new_v4().to_string();

    match op {
        DynamoDBStreamsOperation::DescribeStream => {
            let input = deserialize(body)?;
            let output = provider.handle_describe_stream(input)?;
            serialize(&output, &request_id)
        }
        DynamoDBStreamsOperation::GetShardIterator => {
            let input = deserialize(body)?;
            let output = provider.handle_get_shard_iterator(input)?;
            serialize(&output, &request_id)
        }
        DynamoDBStreamsOperation::GetRecords => {
            let input = deserialize(body)?;
            let output = provider.handle_get_records(input)?;
            serialize(&output, &request_id)
        }
        DynamoDBStreamsOperation::ListStreams => {
            let input = deserialize(body)?;
            let output = provider.handle_list_streams(input)?;
            serialize(&output, &request_id)
        }
    }
}

/// Deserialize a JSON request body into the input type.
fn deserialize<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, DynamoDBStreamsError> {
    serde_json::from_slice(body).map_err(|e| {
        let msg = e.to_string();
        if msg.contains("missing field") || msg.contains("unknown variant") {
            DynamoDBStreamsError::validation(format!("1 validation error detected: {msg}"))
        } else {
            DynamoDBStreamsError::serialization_exception(format!(
                "Failed to deserialize request body: {e}"
            ))
        }
    })
}

/// Serialize an output type into a JSON HTTP response.
fn serialize<T: serde::Serialize>(
    output: &T,
    request_id: &str,
) -> Result<http::Response<DynamoDBStreamsResponseBody>, DynamoDBStreamsError> {
    let json = serde_json::to_vec(output).map_err(|e| {
        DynamoDBStreamsError::internal_error(format!("Failed to serialize response: {e}"))
    })?;
    Ok(json_response(json, request_id))
}
