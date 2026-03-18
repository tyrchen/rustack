//! EventBridge request router.
//!
//! EventBridge uses the `awsJson1_1` protocol where all requests are `POST /`
//! with the operation specified in the `X-Amz-Target` header:
//!
//! ```text
//! X-Amz-Target: AWSEvents.PutEvents
//! ```

use ruststack_events_model::error::EventsError;
use ruststack_events_model::operations::EventsOperation;

/// The expected prefix for the `X-Amz-Target` header value.
const TARGET_PREFIX: &str = "AWSEvents.";

/// Resolve an EventBridge operation from an HTTP request.
///
/// Extracts the operation from the `X-Amz-Target` header, validates the
/// format, and maps it to an [`EventsOperation`] enum variant.
pub fn resolve_operation(headers: &http::HeaderMap) -> Result<EventsOperation, EventsError> {
    let target = headers
        .get("x-amz-target")
        .ok_or_else(EventsError::missing_action)?;

    let target_str = target.to_str().map_err(|_| EventsError::missing_action())?;

    let operation_name = target_str
        .strip_prefix(TARGET_PREFIX)
        .ok_or_else(|| EventsError::unknown_operation(target_str))?;

    EventsOperation::from_name(operation_name)
        .ok_or_else(|| EventsError::unknown_operation(target_str))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with_target(target: &str) -> http::HeaderMap {
        let mut map = http::HeaderMap::new();
        map.insert(
            "x-amz-target",
            http::HeaderValue::from_str(target).expect("valid header value"),
        );
        map
    }

    #[test]
    fn test_should_resolve_put_events() {
        let headers = headers_with_target("AWSEvents.PutEvents");
        let op = resolve_operation(&headers).expect("should resolve");
        assert_eq!(op, EventsOperation::PutEvents);
    }

    #[test]
    fn test_should_resolve_all_operations() {
        let ops = [
            ("AWSEvents.CreateEventBus", EventsOperation::CreateEventBus),
            ("AWSEvents.DeleteEventBus", EventsOperation::DeleteEventBus),
            (
                "AWSEvents.DescribeEventBus",
                EventsOperation::DescribeEventBus,
            ),
            ("AWSEvents.ListEventBuses", EventsOperation::ListEventBuses),
            ("AWSEvents.PutRule", EventsOperation::PutRule),
            ("AWSEvents.DeleteRule", EventsOperation::DeleteRule),
            ("AWSEvents.DescribeRule", EventsOperation::DescribeRule),
            ("AWSEvents.ListRules", EventsOperation::ListRules),
            ("AWSEvents.EnableRule", EventsOperation::EnableRule),
            ("AWSEvents.DisableRule", EventsOperation::DisableRule),
            ("AWSEvents.PutTargets", EventsOperation::PutTargets),
            ("AWSEvents.RemoveTargets", EventsOperation::RemoveTargets),
            (
                "AWSEvents.ListTargetsByRule",
                EventsOperation::ListTargetsByRule,
            ),
            ("AWSEvents.PutEvents", EventsOperation::PutEvents),
            (
                "AWSEvents.TestEventPattern",
                EventsOperation::TestEventPattern,
            ),
            ("AWSEvents.TagResource", EventsOperation::TagResource),
            ("AWSEvents.UntagResource", EventsOperation::UntagResource),
            (
                "AWSEvents.ListTagsForResource",
                EventsOperation::ListTagsForResource,
            ),
            ("AWSEvents.PutPermission", EventsOperation::PutPermission),
            (
                "AWSEvents.RemovePermission",
                EventsOperation::RemovePermission,
            ),
            (
                "AWSEvents.ListRuleNamesByTarget",
                EventsOperation::ListRuleNamesByTarget,
            ),
            ("AWSEvents.UpdateEventBus", EventsOperation::UpdateEventBus),
        ];
        for (target, expected) in ops {
            let headers = headers_with_target(target);
            let op = resolve_operation(&headers).expect("should resolve");
            assert_eq!(op, expected, "failed for target: {target}");
        }
    }

    #[test]
    fn test_should_error_on_missing_target() {
        let headers = http::HeaderMap::new();
        let err = resolve_operation(&headers).unwrap_err();
        assert_eq!(
            err.code,
            ruststack_events_model::error::EventsErrorCode::MissingAction
        );
    }

    #[test]
    fn test_should_error_on_wrong_prefix() {
        let headers = headers_with_target("WrongService.PutEvents");
        let err = resolve_operation(&headers).unwrap_err();
        assert_eq!(
            err.code,
            ruststack_events_model::error::EventsErrorCode::InvalidAction,
        );
    }

    #[test]
    fn test_should_error_on_unknown_operation() {
        let headers = headers_with_target("AWSEvents.NonExistent");
        let err = resolve_operation(&headers).unwrap_err();
        assert_eq!(
            err.code,
            ruststack_events_model::error::EventsErrorCode::InvalidAction,
        );
    }
}
