//! SSM request router.
//!
//! SSM uses the `awsJson1_1` protocol where all requests are `POST /`
//! with the operation specified in the `X-Amz-Target` header:
//!
//! ```text
//! X-Amz-Target: AmazonSSM.PutParameter
//! ```

use ruststack_ssm_model::error::SsmError;
use ruststack_ssm_model::operations::SsmOperation;

/// The expected prefix for the `X-Amz-Target` header value.
const TARGET_PREFIX: &str = "AmazonSSM.";

/// Resolve an SSM operation from an HTTP request.
///
/// Extracts the operation from the `X-Amz-Target` header, validates the
/// format, and maps it to an [`SsmOperation`] enum variant.
pub fn resolve_operation(headers: &http::HeaderMap) -> Result<SsmOperation, SsmError> {
    let target = headers
        .get("x-amz-target")
        .ok_or_else(SsmError::missing_action)?;

    let target_str = target.to_str().map_err(|_| SsmError::missing_action())?;

    let operation_name = target_str
        .strip_prefix(TARGET_PREFIX)
        .ok_or_else(|| SsmError::unknown_operation(target_str))?;

    SsmOperation::from_name(operation_name).ok_or_else(|| SsmError::unknown_operation(target_str))
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
    fn test_should_resolve_put_parameter() {
        let headers = headers_with_target("AmazonSSM.PutParameter");
        let op = resolve_operation(&headers).expect("should resolve");
        assert_eq!(op, SsmOperation::PutParameter);
    }

    #[test]
    fn test_should_resolve_all_operations() {
        let ops = [
            ("AmazonSSM.PutParameter", SsmOperation::PutParameter),
            ("AmazonSSM.GetParameter", SsmOperation::GetParameter),
            ("AmazonSSM.GetParameters", SsmOperation::GetParameters),
            (
                "AmazonSSM.GetParametersByPath",
                SsmOperation::GetParametersByPath,
            ),
            ("AmazonSSM.DeleteParameter", SsmOperation::DeleteParameter),
            ("AmazonSSM.DeleteParameters", SsmOperation::DeleteParameters),
            (
                "AmazonSSM.DescribeParameters",
                SsmOperation::DescribeParameters,
            ),
            (
                "AmazonSSM.GetParameterHistory",
                SsmOperation::GetParameterHistory,
            ),
            (
                "AmazonSSM.AddTagsToResource",
                SsmOperation::AddTagsToResource,
            ),
            (
                "AmazonSSM.RemoveTagsFromResource",
                SsmOperation::RemoveTagsFromResource,
            ),
            (
                "AmazonSSM.ListTagsForResource",
                SsmOperation::ListTagsForResource,
            ),
            (
                "AmazonSSM.LabelParameterVersion",
                SsmOperation::LabelParameterVersion,
            ),
            (
                "AmazonSSM.UnlabelParameterVersion",
                SsmOperation::UnlabelParameterVersion,
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
            ruststack_ssm_model::error::SsmErrorCode::MissingAction
        );
    }

    #[test]
    fn test_should_error_on_wrong_prefix() {
        let headers = headers_with_target("WrongService.PutParameter");
        let err = resolve_operation(&headers).unwrap_err();
        assert_eq!(
            err.code,
            ruststack_ssm_model::error::SsmErrorCode::ValidationException,
        );
    }

    #[test]
    fn test_should_error_on_unknown_operation() {
        let headers = headers_with_target("AmazonSSM.NonExistent");
        let err = resolve_operation(&headers).unwrap_err();
        assert_eq!(
            err.code,
            ruststack_ssm_model::error::SsmErrorCode::ValidationException,
        );
    }
}
