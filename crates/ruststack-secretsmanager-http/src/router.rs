//! Secrets Manager request router.
//!
//! Secrets Manager uses the `awsJson1_1` protocol where all requests are `POST /`
//! with the operation specified in the `X-Amz-Target` header:
//!
//! ```text
//! X-Amz-Target: secretsmanager.CreateSecret
//! ```

use ruststack_secretsmanager_model::error::SecretsManagerError;
use ruststack_secretsmanager_model::operations::SecretsManagerOperation;

/// The expected prefix for the `X-Amz-Target` header value.
const TARGET_PREFIX: &str = "secretsmanager.";

/// Resolve a Secrets Manager operation from an HTTP request.
///
/// Extracts the operation from the `X-Amz-Target` header, validates the
/// format, and maps it to a [`SecretsManagerOperation`] enum variant.
pub fn resolve_operation(
    headers: &http::HeaderMap,
) -> Result<SecretsManagerOperation, SecretsManagerError> {
    let target = headers
        .get("x-amz-target")
        .ok_or_else(SecretsManagerError::missing_action)?;

    let target_str = target
        .to_str()
        .map_err(|_| SecretsManagerError::missing_action())?;

    let operation_name = target_str
        .strip_prefix(TARGET_PREFIX)
        .ok_or_else(|| SecretsManagerError::unknown_operation(target_str))?;

    SecretsManagerOperation::from_name(operation_name)
        .ok_or_else(|| SecretsManagerError::unknown_operation(target_str))
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
    fn test_should_resolve_create_secret() {
        let headers = headers_with_target("secretsmanager.CreateSecret");
        let op = resolve_operation(&headers).expect("should resolve");
        assert_eq!(op, SecretsManagerOperation::CreateSecret);
    }

    #[test]
    fn test_should_resolve_all_operations() {
        let ops = [
            (
                "secretsmanager.CreateSecret",
                SecretsManagerOperation::CreateSecret,
            ),
            (
                "secretsmanager.DescribeSecret",
                SecretsManagerOperation::DescribeSecret,
            ),
            (
                "secretsmanager.GetSecretValue",
                SecretsManagerOperation::GetSecretValue,
            ),
            (
                "secretsmanager.PutSecretValue",
                SecretsManagerOperation::PutSecretValue,
            ),
            (
                "secretsmanager.UpdateSecret",
                SecretsManagerOperation::UpdateSecret,
            ),
            (
                "secretsmanager.DeleteSecret",
                SecretsManagerOperation::DeleteSecret,
            ),
            (
                "secretsmanager.RestoreSecret",
                SecretsManagerOperation::RestoreSecret,
            ),
            (
                "secretsmanager.ListSecrets",
                SecretsManagerOperation::ListSecrets,
            ),
            (
                "secretsmanager.ListSecretVersionIds",
                SecretsManagerOperation::ListSecretVersionIds,
            ),
            (
                "secretsmanager.GetRandomPassword",
                SecretsManagerOperation::GetRandomPassword,
            ),
            (
                "secretsmanager.TagResource",
                SecretsManagerOperation::TagResource,
            ),
            (
                "secretsmanager.UntagResource",
                SecretsManagerOperation::UntagResource,
            ),
            (
                "secretsmanager.UpdateSecretVersionStage",
                SecretsManagerOperation::UpdateSecretVersionStage,
            ),
            (
                "secretsmanager.RotateSecret",
                SecretsManagerOperation::RotateSecret,
            ),
            (
                "secretsmanager.CancelRotateSecret",
                SecretsManagerOperation::CancelRotateSecret,
            ),
            (
                "secretsmanager.BatchGetSecretValue",
                SecretsManagerOperation::BatchGetSecretValue,
            ),
            (
                "secretsmanager.GetResourcePolicy",
                SecretsManagerOperation::GetResourcePolicy,
            ),
            (
                "secretsmanager.PutResourcePolicy",
                SecretsManagerOperation::PutResourcePolicy,
            ),
            (
                "secretsmanager.DeleteResourcePolicy",
                SecretsManagerOperation::DeleteResourcePolicy,
            ),
            (
                "secretsmanager.ValidateResourcePolicy",
                SecretsManagerOperation::ValidateResourcePolicy,
            ),
            (
                "secretsmanager.ReplicateSecretToRegions",
                SecretsManagerOperation::ReplicateSecretToRegions,
            ),
            (
                "secretsmanager.RemoveRegionsFromReplication",
                SecretsManagerOperation::RemoveRegionsFromReplication,
            ),
            (
                "secretsmanager.StopReplicationToReplica",
                SecretsManagerOperation::StopReplicationToReplica,
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
            ruststack_secretsmanager_model::error::SecretsManagerErrorCode::MissingAction
        );
    }

    #[test]
    fn test_should_error_on_wrong_prefix() {
        let headers = headers_with_target("WrongService.CreateSecret");
        let err = resolve_operation(&headers).unwrap_err();
        assert_eq!(
            err.code,
            ruststack_secretsmanager_model::error::SecretsManagerErrorCode::InvalidAction,
        );
    }

    #[test]
    fn test_should_error_on_unknown_operation() {
        let headers = headers_with_target("secretsmanager.NonExistent");
        let err = resolve_operation(&headers).unwrap_err();
        assert_eq!(
            err.code,
            ruststack_secretsmanager_model::error::SecretsManagerErrorCode::InvalidAction,
        );
    }
}
