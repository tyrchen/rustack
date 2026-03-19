//! Secrets Manager handler implementation bridging HTTP to business logic.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;

use ruststack_secretsmanager_http::body::SecretsManagerResponseBody;
use ruststack_secretsmanager_http::dispatch::SecretsManagerHandler;
use ruststack_secretsmanager_http::response::json_response;
use ruststack_secretsmanager_model::error::SecretsManagerError;
use ruststack_secretsmanager_model::operations::SecretsManagerOperation;

use crate::provider::RustStackSecretsManager;

/// Handler that bridges the HTTP layer to the Secrets Manager provider.
#[derive(Debug)]
pub struct RustStackSecretsManagerHandler {
    provider: Arc<RustStackSecretsManager>,
}

impl RustStackSecretsManagerHandler {
    /// Create a new handler wrapping a provider.
    #[must_use]
    pub fn new(provider: Arc<RustStackSecretsManager>) -> Self {
        Self { provider }
    }
}

impl SecretsManagerHandler for RustStackSecretsManagerHandler {
    fn handle_operation(
        &self,
        op: SecretsManagerOperation,
        body: Bytes,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        http::Response<SecretsManagerResponseBody>,
                        SecretsManagerError,
                    >,
                > + Send,
        >,
    > {
        let provider = Arc::clone(&self.provider);
        Box::pin(async move { dispatch(provider.as_ref(), op, &body) })
    }
}

/// Dispatch a Secrets Manager operation to the appropriate handler method.
#[allow(clippy::too_many_lines)]
fn dispatch(
    provider: &RustStackSecretsManager,
    op: SecretsManagerOperation,
    body: &[u8],
) -> Result<http::Response<SecretsManagerResponseBody>, SecretsManagerError> {
    let request_id = uuid::Uuid::new_v4().to_string();

    match op {
        // Phase 0: Core CRUD
        SecretsManagerOperation::CreateSecret => {
            let input = deserialize(body)?;
            let output = provider.handle_create_secret(input)?;
            serialize(&output, &request_id)
        }
        SecretsManagerOperation::GetSecretValue => {
            let input = deserialize(body)?;
            let output = provider.handle_get_secret_value(&input)?;
            serialize(&output, &request_id)
        }
        SecretsManagerOperation::PutSecretValue => {
            let input = deserialize(body)?;
            let output = provider.handle_put_secret_value(input)?;
            serialize(&output, &request_id)
        }
        SecretsManagerOperation::DescribeSecret => {
            let input = deserialize(body)?;
            let output = provider.handle_describe_secret(&input)?;
            serialize(&output, &request_id)
        }
        SecretsManagerOperation::DeleteSecret => {
            let input = deserialize(body)?;
            let output = provider.handle_delete_secret(&input)?;
            serialize(&output, &request_id)
        }
        SecretsManagerOperation::RestoreSecret => {
            let input = deserialize(body)?;
            let output = provider.handle_restore_secret(&input)?;
            serialize(&output, &request_id)
        }
        SecretsManagerOperation::UpdateSecret => {
            let input = deserialize(body)?;
            let output = provider.handle_update_secret(input)?;
            serialize(&output, &request_id)
        }
        SecretsManagerOperation::ListSecrets => {
            let input = deserialize(body)?;
            let output = provider.handle_list_secrets(&input)?;
            serialize(&output, &request_id)
        }
        SecretsManagerOperation::ListSecretVersionIds => {
            let input = deserialize(body)?;
            let output = provider.handle_list_secret_version_ids(&input)?;
            serialize(&output, &request_id)
        }
        SecretsManagerOperation::GetRandomPassword => {
            let input = deserialize(body)?;
            let output = provider.handle_get_random_password(&input)?;
            serialize(&output, &request_id)
        }

        // Phase 1: Tags, Version Stages, Rotation, Batch
        SecretsManagerOperation::TagResource => {
            let input = deserialize(body)?;
            provider.handle_tag_resource(&input)?;
            serialize(&serde_json::json!({}), &request_id)
        }
        SecretsManagerOperation::UntagResource => {
            let input = deserialize(body)?;
            provider.handle_untag_resource(&input)?;
            serialize(&serde_json::json!({}), &request_id)
        }
        SecretsManagerOperation::UpdateSecretVersionStage => {
            let input = deserialize(body)?;
            let output = provider.handle_update_secret_version_stage(&input)?;
            serialize(&output, &request_id)
        }
        SecretsManagerOperation::RotateSecret => {
            let input = deserialize(body)?;
            let output = provider.handle_rotate_secret(input)?;
            serialize(&output, &request_id)
        }
        SecretsManagerOperation::CancelRotateSecret => {
            let input = deserialize(body)?;
            let output = provider.handle_cancel_rotate_secret(&input)?;
            serialize(&output, &request_id)
        }
        SecretsManagerOperation::BatchGetSecretValue => {
            let input = deserialize(body)?;
            let output = provider.handle_batch_get_secret_value(&input)?;
            serialize(&output, &request_id)
        }

        // Phase 2: Resource Policies
        SecretsManagerOperation::GetResourcePolicy => {
            let input = deserialize(body)?;
            let output = provider.handle_get_resource_policy(&input)?;
            serialize(&output, &request_id)
        }
        SecretsManagerOperation::PutResourcePolicy => {
            let input = deserialize(body)?;
            let output = provider.handle_put_resource_policy(&input)?;
            serialize(&output, &request_id)
        }
        SecretsManagerOperation::DeleteResourcePolicy => {
            let input = deserialize(body)?;
            let output = provider.handle_delete_resource_policy(&input)?;
            serialize(&output, &request_id)
        }
        SecretsManagerOperation::ValidateResourcePolicy => {
            let input = deserialize(body)?;
            let output = provider.handle_validate_resource_policy(&input)?;
            serialize(&output, &request_id)
        }

        // Phase 3: Replication Stubs
        SecretsManagerOperation::ReplicateSecretToRegions => {
            let input = deserialize(body)?;
            let output = provider.handle_replicate_secret_to_regions(&input)?;
            serialize(&output, &request_id)
        }
        SecretsManagerOperation::RemoveRegionsFromReplication => {
            let input = deserialize(body)?;
            let output = provider.handle_remove_regions_from_replication(&input)?;
            serialize(&output, &request_id)
        }
        SecretsManagerOperation::StopReplicationToReplica => {
            let input = deserialize(body)?;
            let output = provider.handle_stop_replication_to_replica(&input)?;
            serialize(&output, &request_id)
        }
    }
}

/// Deserialize a JSON request body into the input type.
fn deserialize<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, SecretsManagerError> {
    serde_json::from_slice(body).map_err(|e| {
        let msg = e.to_string();
        if msg.contains("missing field") || msg.contains("unknown variant") {
            SecretsManagerError::with_message(
                ruststack_secretsmanager_model::error::SecretsManagerErrorCode::InvalidParameterException,
                format!("1 validation error detected: {msg}"),
            )
        } else {
            SecretsManagerError::with_message(
                ruststack_secretsmanager_model::error::SecretsManagerErrorCode::InvalidParameterException,
                format!("Failed to deserialize request body: {e}"),
            )
        }
    })
}

/// Serialize an output type into a JSON HTTP response.
fn serialize<T: serde::Serialize>(
    output: &T,
    request_id: &str,
) -> Result<http::Response<SecretsManagerResponseBody>, SecretsManagerError> {
    let json = serde_json::to_vec(output).map_err(|e| {
        SecretsManagerError::internal_error(format!("Failed to serialize response: {e}"))
    })?;
    Ok(json_response(json, request_id))
}
