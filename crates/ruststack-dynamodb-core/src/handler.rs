//! DynamoDB handler implementation bridging HTTP to business logic.

use std::{future::Future, pin::Pin, sync::Arc};

use bytes::Bytes;
use ruststack_dynamodb_http::{
    body::DynamoDBResponseBody, dispatch::DynamoDBHandler, response::json_response,
};
use ruststack_dynamodb_model::{error::DynamoDBError, operations::DynamoDBOperation};

use crate::provider::RustStackDynamoDB;

/// Handler that bridges the HTTP layer to the DynamoDB provider.
#[derive(Debug)]
pub struct RustStackDynamoDBHandler {
    provider: Arc<RustStackDynamoDB>,
}

impl RustStackDynamoDBHandler {
    /// Create a new handler wrapping a provider.
    #[must_use]
    pub fn new(provider: Arc<RustStackDynamoDB>) -> Self {
        Self { provider }
    }
}

impl DynamoDBHandler for RustStackDynamoDBHandler {
    fn handle_operation(
        &self,
        op: DynamoDBOperation,
        body: Bytes,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<http::Response<DynamoDBResponseBody>, DynamoDBError>> + Send,
        >,
    > {
        let provider = Arc::clone(&self.provider);
        Box::pin(async move { dispatch(provider.as_ref(), op, &body) })
    }
}

/// Dispatch a DynamoDB operation to the appropriate handler method.
#[allow(clippy::too_many_lines)]
fn dispatch(
    provider: &RustStackDynamoDB,
    op: DynamoDBOperation,
    body: &[u8],
) -> Result<http::Response<DynamoDBResponseBody>, DynamoDBError> {
    // Generate a request ID for responses.
    let request_id = uuid::Uuid::new_v4().to_string();

    match op {
        DynamoDBOperation::CreateTable => {
            let input = deserialize(body)?;
            let output = provider.handle_create_table(input)?;
            serialize(&output, &request_id)
        }
        DynamoDBOperation::DeleteTable => {
            let input = deserialize(body)?;
            let output = provider.handle_delete_table(input)?;
            serialize(&output, &request_id)
        }
        DynamoDBOperation::UpdateTable => {
            let input = deserialize(body)?;
            let output = provider.handle_update_table(input)?;
            serialize(&output, &request_id)
        }
        DynamoDBOperation::DescribeTable => {
            let input = deserialize(body)?;
            let output = provider.handle_describe_table(input)?;
            serialize(&output, &request_id)
        }
        DynamoDBOperation::ListTables => {
            let input = deserialize(body)?;
            let output = provider.handle_list_tables(input)?;
            serialize(&output, &request_id)
        }
        DynamoDBOperation::PutItem => {
            let input = deserialize(body)?;
            let output = provider.handle_put_item(input)?;
            serialize(&output, &request_id)
        }
        DynamoDBOperation::GetItem => {
            let input = deserialize(body)?;
            let output = provider.handle_get_item(input)?;
            serialize(&output, &request_id)
        }
        DynamoDBOperation::UpdateItem => {
            let input = deserialize(body)?;
            let output = provider.handle_update_item(input)?;
            serialize(&output, &request_id)
        }
        DynamoDBOperation::DeleteItem => {
            let input = deserialize(body)?;
            let output = provider.handle_delete_item(input)?;
            serialize(&output, &request_id)
        }
        DynamoDBOperation::Query => {
            let input = deserialize(body)?;
            let output = provider.handle_query(input)?;
            serialize(&output, &request_id)
        }
        DynamoDBOperation::Scan => {
            let input = deserialize(body)?;
            let output = provider.handle_scan(input)?;
            serialize(&output, &request_id)
        }
        DynamoDBOperation::BatchGetItem => {
            let input = deserialize(body)?;
            let output = provider.handle_batch_get_item(input)?;
            serialize(&output, &request_id)
        }
        DynamoDBOperation::BatchWriteItem => {
            let input = deserialize(body)?;
            let output = provider.handle_batch_write_item(input)?;
            serialize(&output, &request_id)
        }
        DynamoDBOperation::TagResource => {
            let input = deserialize(body)?;
            let output = provider.handle_tag_resource(input)?;
            serialize(&output, &request_id)
        }
        DynamoDBOperation::UntagResource => {
            let input = deserialize(body)?;
            let output = provider.handle_untag_resource(input)?;
            serialize(&output, &request_id)
        }
        DynamoDBOperation::ListTagsOfResource => {
            let input = deserialize(body)?;
            let output = provider.handle_list_tags_of_resource(input)?;
            serialize(&output, &request_id)
        }
        DynamoDBOperation::DescribeTimeToLive => {
            let input = deserialize(body)?;
            let output = provider.handle_describe_time_to_live(input)?;
            serialize(&output, &request_id)
        }
        DynamoDBOperation::UpdateTimeToLive => {
            let input = deserialize(body)?;
            let output = provider.handle_update_time_to_live(input)?;
            serialize(&output, &request_id)
        }
        DynamoDBOperation::TransactGetItems => {
            let input = deserialize(body)?;
            let output = provider.handle_transact_get_items(input)?;
            serialize(&output, &request_id)
        }
        DynamoDBOperation::TransactWriteItems => {
            let input = deserialize(body)?;
            let output = provider.handle_transact_write_items(input)?;
            serialize(&output, &request_id)
        }
        DynamoDBOperation::DescribeLimits => {
            let input = deserialize(body)?;
            let output = provider.handle_describe_limits(input)?;
            serialize(&output, &request_id)
        }
        DynamoDBOperation::DescribeEndpoints => {
            let input = deserialize(body)?;
            let output = provider.handle_describe_endpoints(input)?;
            serialize(&output, &request_id)
        }
    }
}

/// Deserialize a JSON request body into the input type.
///
/// Serde errors for missing required fields (e.g., `missing field 'AttributeName'`)
/// are mapped to `ValidationException` to match DynamoDB behaviour, while other
/// deserialization errors remain `SerializationException`.
fn deserialize<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, DynamoDBError> {
    serde_json::from_slice(body).map_err(|e| {
        let msg = e.to_string();
        if msg.contains("missing field") || msg.contains("unknown variant") {
            DynamoDBError::validation(format!("1 validation error detected: {msg}"))
        } else {
            DynamoDBError::serialization_exception(format!(
                "Failed to deserialize request body: {e}"
            ))
        }
    })
}

/// Serialize an output type into a JSON HTTP response.
fn serialize<T: serde::Serialize>(
    output: &T,
    request_id: &str,
) -> Result<http::Response<DynamoDBResponseBody>, DynamoDBError> {
    let json = serde_json::to_vec(output)
        .map_err(|e| DynamoDBError::internal_error(format!("Failed to serialize response: {e}")))?;
    Ok(json_response(json, request_id))
}
