//! DynamoDB Streams request router.
//!
//! DynamoDB Streams uses the `awsJson1_0` protocol where all requests are
//! `POST /` with the operation specified in the `X-Amz-Target` header:
//!
//! ```text
//! X-Amz-Target: DynamoDBStreams_20120810.DescribeStream
//! ```

use ruststack_dynamodbstreams_model::error::DynamoDBStreamsError;
use ruststack_dynamodbstreams_model::operations::DynamoDBStreamsOperation;

/// The expected prefix for the `X-Amz-Target` header value.
const TARGET_PREFIX: &str = "DynamoDBStreams_20120810.";

/// Resolve a DynamoDB Streams operation from an HTTP request.
///
/// Extracts the operation from the `X-Amz-Target` header, validates the
/// format, and maps it to a [`DynamoDBStreamsOperation`] enum variant.
pub fn resolve_operation(
    headers: &http::HeaderMap,
) -> Result<DynamoDBStreamsOperation, DynamoDBStreamsError> {
    let target = headers
        .get("x-amz-target")
        .ok_or_else(DynamoDBStreamsError::missing_action)?;

    let target_str = target
        .to_str()
        .map_err(|_| DynamoDBStreamsError::missing_action())?;

    let operation_name = target_str
        .strip_prefix(TARGET_PREFIX)
        .ok_or_else(|| DynamoDBStreamsError::unknown_operation(target_str))?;

    DynamoDBStreamsOperation::from_name(operation_name)
        .ok_or_else(|| DynamoDBStreamsError::unknown_operation(target_str))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with_target(target: &str) -> http::HeaderMap {
        let mut map = http::HeaderMap::new();
        map.insert("x-amz-target", http::HeaderValue::from_str(target).unwrap());
        map
    }

    #[test]
    fn test_should_resolve_describe_stream() {
        let headers = headers_with_target("DynamoDBStreams_20120810.DescribeStream");
        let op = resolve_operation(&headers).unwrap();
        assert_eq!(op, DynamoDBStreamsOperation::DescribeStream);
    }

    #[test]
    fn test_should_resolve_all_operations() {
        let ops = [
            (
                "DynamoDBStreams_20120810.DescribeStream",
                DynamoDBStreamsOperation::DescribeStream,
            ),
            (
                "DynamoDBStreams_20120810.GetShardIterator",
                DynamoDBStreamsOperation::GetShardIterator,
            ),
            (
                "DynamoDBStreams_20120810.GetRecords",
                DynamoDBStreamsOperation::GetRecords,
            ),
            (
                "DynamoDBStreams_20120810.ListStreams",
                DynamoDBStreamsOperation::ListStreams,
            ),
        ];
        for (target, expected) in ops {
            let headers = headers_with_target(target);
            let op = resolve_operation(&headers).unwrap();
            assert_eq!(op, expected, "failed for target: {target}");
        }
    }

    #[test]
    fn test_should_error_on_missing_target() {
        let headers = http::HeaderMap::new();
        let err = resolve_operation(&headers).unwrap_err();
        assert_eq!(
            err.code,
            ruststack_dynamodbstreams_model::error::DynamoDBStreamsErrorCode::MissingAction,
        );
    }

    #[test]
    fn test_should_error_on_wrong_prefix() {
        let headers = headers_with_target("DynamoDB_20120810.CreateTable");
        let err = resolve_operation(&headers).unwrap_err();
        assert_eq!(
            err.code,
            ruststack_dynamodbstreams_model::error::DynamoDBStreamsErrorCode::InvalidAction,
        );
    }

    #[test]
    fn test_should_error_on_unknown_operation() {
        let headers = headers_with_target("DynamoDBStreams_20120810.NonExistent");
        let err = resolve_operation(&headers).unwrap_err();
        assert_eq!(
            err.code,
            ruststack_dynamodbstreams_model::error::DynamoDBStreamsErrorCode::InvalidAction,
        );
    }
}
