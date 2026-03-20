//! CloudWatch request router.
//!
//! CloudWatch uses the `awsQuery` protocol where requests are `POST /` with
//! `Content-Type: application/x-www-form-urlencoded`. The operation is
//! specified by the `Action=<OperationName>` form parameter.

use ruststack_cloudwatch_model::error::CloudWatchError;
use ruststack_cloudwatch_model::operations::CloudWatchOperation;

/// Resolve a CloudWatch operation from parsed form parameters.
///
/// Looks for the `Action` parameter in the form body and maps it to
/// the corresponding [`CloudWatchOperation`] variant.
pub fn resolve_operation(
    params: &[(String, String)],
) -> Result<CloudWatchOperation, CloudWatchError> {
    let action = params
        .iter()
        .find(|(k, _)| k == "Action")
        .map(|(_, v)| v.as_str())
        .ok_or_else(CloudWatchError::missing_action)?;

    CloudWatchOperation::from_name(action).ok_or_else(|| CloudWatchError::unknown_operation(action))
}

#[cfg(test)]
mod tests {
    use ruststack_cloudwatch_model::error::CloudWatchErrorCode;

    use super::*;

    fn params_with_action(action: &str) -> Vec<(String, String)> {
        vec![("Action".to_owned(), action.to_owned())]
    }

    #[test]
    fn test_should_resolve_put_metric_data() {
        let params = params_with_action("PutMetricData");
        let op = resolve_operation(&params).unwrap();
        assert_eq!(op, CloudWatchOperation::PutMetricData);
    }

    #[test]
    fn test_should_resolve_describe_alarms() {
        let params = params_with_action("DescribeAlarms");
        let op = resolve_operation(&params).unwrap();
        assert_eq!(op, CloudWatchOperation::DescribeAlarms);
    }

    #[test]
    fn test_should_error_on_missing_action() {
        let params: Vec<(String, String)> = vec![];
        let err = resolve_operation(&params).unwrap_err();
        assert_eq!(err.code, CloudWatchErrorCode::MissingAction);
    }

    #[test]
    fn test_should_error_on_unknown_operation() {
        let params = params_with_action("NonExistentOperation");
        let err = resolve_operation(&params).unwrap_err();
        assert_eq!(err.code, CloudWatchErrorCode::InvalidAction);
    }
}
