//! CloudWatch Logs request router.
//!
//! CloudWatch Logs uses the `awsJson1_1` protocol where all requests are `POST /`
//! with the operation specified in the `X-Amz-Target` header:
//!
//! ```text
//! X-Amz-Target: Logs_20140328.CreateLogGroup
//! ```

use ruststack_logs_model::error::LogsError;
use ruststack_logs_model::operations::LogsOperation;

/// The expected prefix for the `X-Amz-Target` header value.
const TARGET_PREFIX: &str = "Logs_20140328.";

/// Resolve a CloudWatch Logs operation from an HTTP request.
///
/// Extracts the operation from the `X-Amz-Target` header, validates the
/// format, and maps it to a [`LogsOperation`] enum variant.
pub fn resolve_operation(headers: &http::HeaderMap) -> Result<LogsOperation, LogsError> {
    let target = headers
        .get("x-amz-target")
        .ok_or_else(LogsError::missing_action)?;

    let target_str = target.to_str().map_err(|_| LogsError::missing_action())?;

    let operation_name = target_str
        .strip_prefix(TARGET_PREFIX)
        .ok_or_else(|| LogsError::unknown_operation(target_str))?;

    LogsOperation::from_name(operation_name).ok_or_else(|| LogsError::unknown_operation(target_str))
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
    fn test_should_resolve_create_log_group() {
        let headers = headers_with_target("Logs_20140328.CreateLogGroup");
        let op = resolve_operation(&headers).expect("should resolve");
        assert_eq!(op, LogsOperation::CreateLogGroup);
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn test_should_resolve_all_operations() {
        let ops = [
            (
                "Logs_20140328.CreateLogGroup",
                LogsOperation::CreateLogGroup,
            ),
            (
                "Logs_20140328.DeleteLogGroup",
                LogsOperation::DeleteLogGroup,
            ),
            (
                "Logs_20140328.DescribeLogGroups",
                LogsOperation::DescribeLogGroups,
            ),
            (
                "Logs_20140328.CreateLogStream",
                LogsOperation::CreateLogStream,
            ),
            (
                "Logs_20140328.DeleteLogStream",
                LogsOperation::DeleteLogStream,
            ),
            (
                "Logs_20140328.DescribeLogStreams",
                LogsOperation::DescribeLogStreams,
            ),
            ("Logs_20140328.PutLogEvents", LogsOperation::PutLogEvents),
            ("Logs_20140328.GetLogEvents", LogsOperation::GetLogEvents),
            (
                "Logs_20140328.FilterLogEvents",
                LogsOperation::FilterLogEvents,
            ),
            (
                "Logs_20140328.PutRetentionPolicy",
                LogsOperation::PutRetentionPolicy,
            ),
            (
                "Logs_20140328.DeleteRetentionPolicy",
                LogsOperation::DeleteRetentionPolicy,
            ),
            (
                "Logs_20140328.PutMetricFilter",
                LogsOperation::PutMetricFilter,
            ),
            (
                "Logs_20140328.DeleteMetricFilter",
                LogsOperation::DeleteMetricFilter,
            ),
            (
                "Logs_20140328.DescribeMetricFilters",
                LogsOperation::DescribeMetricFilters,
            ),
            (
                "Logs_20140328.PutSubscriptionFilter",
                LogsOperation::PutSubscriptionFilter,
            ),
            (
                "Logs_20140328.DeleteSubscriptionFilter",
                LogsOperation::DeleteSubscriptionFilter,
            ),
            (
                "Logs_20140328.DescribeSubscriptionFilters",
                LogsOperation::DescribeSubscriptionFilters,
            ),
            ("Logs_20140328.TagResource", LogsOperation::TagResource),
            ("Logs_20140328.UntagResource", LogsOperation::UntagResource),
            (
                "Logs_20140328.ListTagsForResource",
                LogsOperation::ListTagsForResource,
            ),
            ("Logs_20140328.TagLogGroup", LogsOperation::TagLogGroup),
            ("Logs_20140328.UntagLogGroup", LogsOperation::UntagLogGroup),
            (
                "Logs_20140328.ListTagsLogGroup",
                LogsOperation::ListTagsLogGroup,
            ),
            (
                "Logs_20140328.PutDestination",
                LogsOperation::PutDestination,
            ),
            (
                "Logs_20140328.PutDestinationPolicy",
                LogsOperation::PutDestinationPolicy,
            ),
            (
                "Logs_20140328.DeleteDestination",
                LogsOperation::DeleteDestination,
            ),
            (
                "Logs_20140328.DescribeDestinations",
                LogsOperation::DescribeDestinations,
            ),
            (
                "Logs_20140328.AssociateKmsKey",
                LogsOperation::AssociateKmsKey,
            ),
            (
                "Logs_20140328.DisassociateKmsKey",
                LogsOperation::DisassociateKmsKey,
            ),
            ("Logs_20140328.StartQuery", LogsOperation::StartQuery),
            ("Logs_20140328.StopQuery", LogsOperation::StopQuery),
            (
                "Logs_20140328.GetQueryResults",
                LogsOperation::GetQueryResults,
            ),
            (
                "Logs_20140328.DescribeQueries",
                LogsOperation::DescribeQueries,
            ),
            (
                "Logs_20140328.PutQueryDefinition",
                LogsOperation::PutQueryDefinition,
            ),
            (
                "Logs_20140328.DeleteQueryDefinition",
                LogsOperation::DeleteQueryDefinition,
            ),
            (
                "Logs_20140328.DescribeQueryDefinitions",
                LogsOperation::DescribeQueryDefinitions,
            ),
            (
                "Logs_20140328.PutResourcePolicy",
                LogsOperation::PutResourcePolicy,
            ),
            (
                "Logs_20140328.DeleteResourcePolicy",
                LogsOperation::DeleteResourcePolicy,
            ),
            (
                "Logs_20140328.DescribeResourcePolicies",
                LogsOperation::DescribeResourcePolicies,
            ),
            (
                "Logs_20140328.TestMetricFilter",
                LogsOperation::TestMetricFilter,
            ),
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
            ruststack_logs_model::error::LogsErrorCode::MissingAction
        );
    }

    #[test]
    fn test_should_error_on_wrong_prefix() {
        let headers = headers_with_target("WrongService.CreateLogGroup");
        let err = resolve_operation(&headers).unwrap_err();
        assert_eq!(
            err.code,
            ruststack_logs_model::error::LogsErrorCode::InvalidAction,
        );
    }

    #[test]
    fn test_should_error_on_unknown_operation() {
        let headers = headers_with_target("Logs_20140328.NonExistent");
        let err = resolve_operation(&headers).unwrap_err();
        assert_eq!(
            err.code,
            ruststack_logs_model::error::LogsErrorCode::InvalidAction,
        );
    }
}
