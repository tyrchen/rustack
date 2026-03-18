//! CloudWatch Logs handler implementation bridging HTTP to business logic.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;

use ruststack_logs_http::body::LogsResponseBody;
use ruststack_logs_http::dispatch::LogsHandler;
use ruststack_logs_http::response::json_response;
use ruststack_logs_model::error::LogsError;
use ruststack_logs_model::operations::LogsOperation;

use crate::provider::RustStackLogs;

/// Handler that bridges the HTTP layer to the CloudWatch Logs provider.
#[derive(Debug)]
pub struct RustStackLogsHandler {
    provider: Arc<RustStackLogs>,
}

impl RustStackLogsHandler {
    /// Create a new handler wrapping a provider.
    #[must_use]
    pub fn new(provider: Arc<RustStackLogs>) -> Self {
        Self { provider }
    }
}

impl LogsHandler for RustStackLogsHandler {
    fn handle_operation(
        &self,
        op: LogsOperation,
        body: Bytes,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<LogsResponseBody>, LogsError>> + Send>>
    {
        let provider = Arc::clone(&self.provider);
        Box::pin(async move { dispatch(&provider, op, &body) })
    }
}

/// Dispatch a CloudWatch Logs operation to the appropriate provider method.
#[allow(clippy::too_many_lines)] // Match arms are simple one-liners; splitting would reduce clarity.
fn dispatch(
    provider: &RustStackLogs,
    op: LogsOperation,
    body: &[u8],
) -> Result<http::Response<LogsResponseBody>, LogsError> {
    let request_id = uuid::Uuid::new_v4().to_string();

    match op {
        // Phase 0: Log Group Management
        LogsOperation::CreateLogGroup => {
            let input = deserialize(body)?;
            let output = provider.handle_create_log_group(&input)?;
            serialize(&output, &request_id)
        }
        LogsOperation::DeleteLogGroup => {
            let input = deserialize(body)?;
            let output = provider.handle_delete_log_group(&input)?;
            serialize(&output, &request_id)
        }
        LogsOperation::DescribeLogGroups => {
            let input = deserialize(body)?;
            let output = provider.handle_describe_log_groups(&input)?;
            serialize(&output, &request_id)
        }

        // Phase 0: Log Stream Management
        LogsOperation::CreateLogStream => {
            let input = deserialize(body)?;
            let output = provider.handle_create_log_stream(&input)?;
            serialize(&output, &request_id)
        }
        LogsOperation::DeleteLogStream => {
            let input = deserialize(body)?;
            let output = provider.handle_delete_log_stream(&input)?;
            serialize(&output, &request_id)
        }
        LogsOperation::DescribeLogStreams => {
            let input = deserialize(body)?;
            let output = provider.handle_describe_log_streams(&input)?;
            serialize(&output, &request_id)
        }

        // Phase 0: Log Events
        LogsOperation::PutLogEvents => {
            let input = deserialize(body)?;
            let output = provider.handle_put_log_events(&input)?;
            serialize(&output, &request_id)
        }
        LogsOperation::GetLogEvents => {
            let input = deserialize(body)?;
            let output = provider.handle_get_log_events(&input)?;
            serialize(&output, &request_id)
        }
        LogsOperation::FilterLogEvents => {
            let input = deserialize(body)?;
            let output = provider.handle_filter_log_events(&input)?;
            serialize(&output, &request_id)
        }

        // Phase 1: Retention Policy
        LogsOperation::PutRetentionPolicy => {
            let input = deserialize(body)?;
            let output = provider.handle_put_retention_policy(&input)?;
            serialize(&output, &request_id)
        }
        LogsOperation::DeleteRetentionPolicy => {
            let input = deserialize(body)?;
            let output = provider.handle_delete_retention_policy(&input)?;
            serialize(&output, &request_id)
        }

        // Phase 1: Tagging (legacy)
        LogsOperation::TagLogGroup => {
            let input = deserialize(body)?;
            let output = provider.handle_tag_log_group(&input)?;
            serialize(&output, &request_id)
        }
        LogsOperation::UntagLogGroup => {
            let input = deserialize(body)?;
            let output = provider.handle_untag_log_group(&input)?;
            serialize(&output, &request_id)
        }
        LogsOperation::ListTagsLogGroup => {
            let input = deserialize(body)?;
            let output = provider.handle_list_tags_log_group(&input)?;
            serialize(&output, &request_id)
        }

        // Phase 1: Tagging (ARN-based)
        LogsOperation::TagResource => {
            let input = deserialize(body)?;
            let output = provider.handle_tag_resource(&input)?;
            serialize(&output, &request_id)
        }
        LogsOperation::UntagResource => {
            let input = deserialize(body)?;
            let output = provider.handle_untag_resource(&input)?;
            serialize(&output, &request_id)
        }
        LogsOperation::ListTagsForResource => {
            let input = deserialize(body)?;
            let output = provider.handle_list_tags_for_resource(&input)?;
            serialize(&output, &request_id)
        }

        // Phase 1: Resource Policies
        LogsOperation::PutResourcePolicy => {
            let input = deserialize(body)?;
            let output = provider.handle_put_resource_policy(&input)?;
            serialize(&output, &request_id)
        }
        LogsOperation::DeleteResourcePolicy => {
            let input = deserialize(body)?;
            let output = provider.handle_delete_resource_policy(&input)?;
            serialize(&output, &request_id)
        }
        LogsOperation::DescribeResourcePolicies => {
            let input = deserialize(body)?;
            let output = provider.handle_describe_resource_policies(&input)?;
            serialize(&output, &request_id)
        }

        // Phase 2: Metric Filters
        LogsOperation::PutMetricFilter => {
            let input = deserialize(body)?;
            let output = provider.handle_put_metric_filter(&input)?;
            serialize(&output, &request_id)
        }
        LogsOperation::DeleteMetricFilter => {
            let input = deserialize(body)?;
            let output = provider.handle_delete_metric_filter(&input)?;
            serialize(&output, &request_id)
        }
        LogsOperation::DescribeMetricFilters => {
            let input = deserialize(body)?;
            let output = provider.handle_describe_metric_filters(&input)?;
            serialize(&output, &request_id)
        }
        LogsOperation::TestMetricFilter => {
            let input = deserialize(body)?;
            let output = provider.handle_test_metric_filter(&input)?;
            serialize(&output, &request_id)
        }

        // Phase 2: Subscription Filters
        LogsOperation::PutSubscriptionFilter => {
            let input = deserialize(body)?;
            let output = provider.handle_put_subscription_filter(&input)?;
            serialize(&output, &request_id)
        }
        LogsOperation::DeleteSubscriptionFilter => {
            let input = deserialize(body)?;
            let output = provider.handle_delete_subscription_filter(&input)?;
            serialize(&output, &request_id)
        }
        LogsOperation::DescribeSubscriptionFilters => {
            let input = deserialize(body)?;
            let output = provider.handle_describe_subscription_filters(&input)?;
            serialize(&output, &request_id)
        }

        // Phase 3: Destinations
        LogsOperation::PutDestination => {
            let input = deserialize(body)?;
            let output = provider.handle_put_destination(&input)?;
            serialize(&output, &request_id)
        }
        LogsOperation::PutDestinationPolicy => {
            let input = deserialize(body)?;
            let output = provider.handle_put_destination_policy(&input)?;
            serialize(&output, &request_id)
        }
        LogsOperation::DeleteDestination => {
            let input = deserialize(body)?;
            let output = provider.handle_delete_destination(&input)?;
            serialize(&output, &request_id)
        }
        LogsOperation::DescribeDestinations => {
            let input = deserialize(body)?;
            let output = provider.handle_describe_destinations(&input)?;
            serialize(&output, &request_id)
        }

        // Phase 3: Query Operations (stubs)
        LogsOperation::StartQuery => {
            let input = deserialize(body)?;
            let output = provider.handle_start_query(&input)?;
            serialize(&output, &request_id)
        }
        LogsOperation::StopQuery => {
            let input = deserialize(body)?;
            let output = provider.handle_stop_query(&input)?;
            serialize(&output, &request_id)
        }
        LogsOperation::GetQueryResults => {
            let input = deserialize(body)?;
            let output = provider.handle_get_query_results(&input)?;
            serialize(&output, &request_id)
        }
        LogsOperation::DescribeQueries => {
            let input = deserialize(body)?;
            let output = provider.handle_describe_queries(&input)?;
            serialize(&output, &request_id)
        }

        // Phase 3: Query Definitions
        LogsOperation::PutQueryDefinition => {
            let input = deserialize(body)?;
            let output = provider.handle_put_query_definition(&input)?;
            serialize(&output, &request_id)
        }
        LogsOperation::DeleteQueryDefinition => {
            let input = deserialize(body)?;
            let output = provider.handle_delete_query_definition(&input)?;
            serialize(&output, &request_id)
        }
        LogsOperation::DescribeQueryDefinitions => {
            let input = deserialize(body)?;
            let output = provider.handle_describe_query_definitions(&input)?;
            serialize(&output, &request_id)
        }

        // Phase 3: KMS Key Association
        LogsOperation::AssociateKmsKey => {
            let input = deserialize(body)?;
            let output = provider.handle_associate_kms_key(&input)?;
            serialize(&output, &request_id)
        }
        LogsOperation::DisassociateKmsKey => {
            let input = deserialize(body)?;
            let output = provider.handle_disassociate_kms_key(&input)?;
            serialize(&output, &request_id)
        }

        // Not implemented operations
        LogsOperation::CreateExportTask
        | LogsOperation::CancelExportTask
        | LogsOperation::DescribeExportTasks => Err(LogsError::not_implemented(op.as_str())),
    }
}

/// Deserialize a JSON request body into the input type.
fn deserialize<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, LogsError> {
    serde_json::from_slice(body).map_err(|e| {
        let msg = e.to_string();
        if msg.contains("missing field") || msg.contains("unknown variant") {
            LogsError::validation(format!("1 validation error detected: {msg}"))
        } else {
            LogsError::validation(format!("Failed to deserialize request body: {e}"))
        }
    })
}

/// Serialize an output type into a JSON HTTP response.
fn serialize<T: serde::Serialize>(
    output: &T,
    request_id: &str,
) -> Result<http::Response<LogsResponseBody>, LogsError> {
    let json = serde_json::to_vec(output)
        .map_err(|e| LogsError::internal_error(format!("Failed to serialize response: {e}")))?;
    Ok(json_response(json, request_id))
}
