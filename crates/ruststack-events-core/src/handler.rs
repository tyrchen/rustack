//! EventBridge handler implementation bridging HTTP to business logic.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;

use ruststack_events_http::body::EventsResponseBody;
use ruststack_events_http::dispatch::EventsHandler;
use ruststack_events_http::response::json_response;
use ruststack_events_model::error::EventsError;
use ruststack_events_model::operations::EventsOperation;

use crate::provider::RustStackEvents;

/// Handler that bridges the HTTP layer to the EventBridge provider.
#[derive(Debug)]
pub struct RustStackEventsHandler {
    provider: Arc<RustStackEvents>,
}

impl RustStackEventsHandler {
    /// Create a new handler wrapping a provider.
    #[must_use]
    pub fn new(provider: Arc<RustStackEvents>) -> Self {
        Self { provider }
    }
}

impl EventsHandler for RustStackEventsHandler {
    fn handle_operation(
        &self,
        op: EventsOperation,
        body: Bytes,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<EventsResponseBody>, EventsError>> + Send>>
    {
        let provider = Arc::clone(&self.provider);
        Box::pin(async move { dispatch(&provider, op, &body) })
    }
}

/// Dispatch an EventBridge operation to the appropriate provider method.
#[allow(clippy::too_many_lines)] // Match arms are simple one-liners; splitting would reduce clarity.
fn dispatch(
    provider: &RustStackEvents,
    op: EventsOperation,
    body: &[u8],
) -> Result<http::Response<EventsResponseBody>, EventsError> {
    let request_id = uuid::Uuid::new_v4().to_string();

    if !op.is_implemented() {
        return Err(EventsError::not_implemented(op.as_str()));
    }

    match op {
        // Phase 0: Event Bus Management
        EventsOperation::CreateEventBus => {
            let input = deserialize(body)?;
            let output = provider.handle_create_event_bus(&input)?;
            serialize(&output, &request_id)
        }
        EventsOperation::DeleteEventBus => {
            let input = deserialize(body)?;
            let output = provider.handle_delete_event_bus(&input)?;
            serialize(&output, &request_id)
        }
        EventsOperation::DescribeEventBus => {
            let input = deserialize(body)?;
            let output = provider.handle_describe_event_bus(&input)?;
            serialize(&output, &request_id)
        }
        EventsOperation::ListEventBuses => {
            let input = deserialize(body)?;
            let output = provider.handle_list_event_buses(&input)?;
            serialize(&output, &request_id)
        }

        // Phase 0: Rule Management
        EventsOperation::PutRule => {
            let input = deserialize(body)?;
            let output = provider.handle_put_rule(input)?;
            serialize(&output, &request_id)
        }
        EventsOperation::DeleteRule => {
            let input = deserialize(body)?;
            let output = provider.handle_delete_rule(&input)?;
            serialize(&output, &request_id)
        }
        EventsOperation::DescribeRule => {
            let input = deserialize(body)?;
            let output = provider.handle_describe_rule(&input)?;
            serialize(&output, &request_id)
        }
        EventsOperation::ListRules => {
            let input = deserialize(body)?;
            let output = provider.handle_list_rules(&input)?;
            serialize(&output, &request_id)
        }
        EventsOperation::EnableRule => {
            let input = deserialize(body)?;
            let output = provider.handle_enable_rule(&input)?;
            serialize(&output, &request_id)
        }
        EventsOperation::DisableRule => {
            let input = deserialize(body)?;
            let output = provider.handle_disable_rule(&input)?;
            serialize(&output, &request_id)
        }

        // Phase 0: Target Management
        EventsOperation::PutTargets => {
            let input = deserialize(body)?;
            let output = provider.handle_put_targets(input)?;
            serialize(&output, &request_id)
        }
        EventsOperation::RemoveTargets => {
            let input = deserialize(body)?;
            let output = provider.handle_remove_targets(&input)?;
            serialize(&output, &request_id)
        }
        EventsOperation::ListTargetsByRule => {
            let input = deserialize(body)?;
            let output = provider.handle_list_targets_by_rule(&input)?;
            serialize(&output, &request_id)
        }

        // Phase 0: Event Operations
        EventsOperation::PutEvents => {
            let input = deserialize(body)?;
            let output = provider.handle_put_events(&input)?;
            serialize(&output, &request_id)
        }
        EventsOperation::TestEventPattern => {
            let input = deserialize(body)?;
            let output = provider.handle_test_event_pattern(&input)?;
            serialize(&output, &request_id)
        }

        // Phase 1: Tagging
        EventsOperation::TagResource => {
            let input = deserialize(body)?;
            let output = provider.handle_tag_resource(&input)?;
            serialize(&output, &request_id)
        }
        EventsOperation::UntagResource => {
            let input = deserialize(body)?;
            let output = provider.handle_untag_resource(&input)?;
            serialize(&output, &request_id)
        }
        EventsOperation::ListTagsForResource => {
            let input = deserialize(body)?;
            let output = provider.handle_list_tags_for_resource(&input)?;
            serialize(&output, &request_id)
        }

        // Phase 1: Permissions
        EventsOperation::PutPermission => {
            let input = deserialize(body)?;
            let output = provider.handle_put_permission(&input)?;
            serialize(&output, &request_id)
        }
        EventsOperation::RemovePermission => {
            let input = deserialize(body)?;
            let output = provider.handle_remove_permission(&input)?;
            serialize(&output, &request_id)
        }

        // Phase 1: Reverse Lookup
        EventsOperation::ListRuleNamesByTarget => {
            let input = deserialize(body)?;
            let output = provider.handle_list_rule_names_by_target(&input)?;
            serialize(&output, &request_id)
        }

        // Phase 2: Update
        EventsOperation::UpdateEventBus => {
            let input = deserialize(body)?;
            let output = provider.handle_update_event_bus(&input)?;
            serialize(&output, &request_id)
        }

        // Phase 3: Stubs - return empty JSON object
        EventsOperation::CreateArchive
        | EventsOperation::DeleteArchive
        | EventsOperation::DescribeArchive
        | EventsOperation::ListArchives
        | EventsOperation::UpdateArchive
        | EventsOperation::StartReplay
        | EventsOperation::CancelReplay
        | EventsOperation::DescribeReplay
        | EventsOperation::ListReplays
        | EventsOperation::CreateApiDestination
        | EventsOperation::DeleteApiDestination
        | EventsOperation::DescribeApiDestination
        | EventsOperation::ListApiDestinations
        | EventsOperation::UpdateApiDestination
        | EventsOperation::CreateConnection
        | EventsOperation::DeleteConnection
        | EventsOperation::DescribeConnection
        | EventsOperation::ListConnections
        | EventsOperation::UpdateConnection
        | EventsOperation::DeauthorizeConnection
        | EventsOperation::CreateEndpoint
        | EventsOperation::DeleteEndpoint
        | EventsOperation::DescribeEndpoint
        | EventsOperation::ListEndpoints
        | EventsOperation::UpdateEndpoint
        | EventsOperation::ActivateEventSource
        | EventsOperation::CreatePartnerEventSource
        | EventsOperation::DeactivateEventSource
        | EventsOperation::DeletePartnerEventSource
        | EventsOperation::DescribeEventSource
        | EventsOperation::DescribePartnerEventSource
        | EventsOperation::ListEventSources
        | EventsOperation::ListPartnerEventSourceAccounts
        | EventsOperation::ListPartnerEventSources
        | EventsOperation::PutPartnerEvents => {
            let output = ruststack_events_model::output::GenericOutput {
                value: serde_json::json!({}),
            };
            serialize(&output, &request_id)
        }
    }
}

/// Deserialize a JSON request body into the input type.
fn deserialize<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, EventsError> {
    serde_json::from_slice(body).map_err(|e| {
        let msg = e.to_string();
        if msg.contains("missing field") || msg.contains("unknown variant") {
            EventsError::validation(format!("1 validation error detected: {msg}"))
        } else {
            EventsError::validation(format!("Failed to deserialize request body: {e}"))
        }
    })
}

/// Serialize an output type into a JSON HTTP response.
fn serialize<T: serde::Serialize>(
    output: &T,
    request_id: &str,
) -> Result<http::Response<EventsResponseBody>, EventsError> {
    let json = serde_json::to_vec(output)
        .map_err(|e| EventsError::internal_error(format!("Failed to serialize response: {e}")))?;
    Ok(json_response(json, request_id))
}
