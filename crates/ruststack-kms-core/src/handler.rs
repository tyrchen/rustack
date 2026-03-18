//! KMS handler implementation bridging HTTP to business logic.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;

use ruststack_kms_http::body::KmsResponseBody;
use ruststack_kms_http::dispatch::KmsHandler;
use ruststack_kms_http::response::json_response;
use ruststack_kms_model::error::KmsError;
use ruststack_kms_model::operations::KmsOperation;

use crate::provider::RustStackKms;

/// Handler that bridges the HTTP layer to the KMS provider.
#[derive(Debug)]
pub struct RustStackKmsHandler {
    provider: Arc<RustStackKms>,
}

impl RustStackKmsHandler {
    /// Create a new handler wrapping a provider.
    #[must_use]
    pub fn new(provider: Arc<RustStackKms>) -> Self {
        Self { provider }
    }
}

impl KmsHandler for RustStackKmsHandler {
    fn handle_operation(
        &self,
        op: KmsOperation,
        body: Bytes,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<KmsResponseBody>, KmsError>> + Send>>
    {
        let provider = Arc::clone(&self.provider);
        Box::pin(async move { dispatch(provider.as_ref(), op, &body) })
    }
}

/// Dispatch a KMS operation to the appropriate handler method.
#[allow(clippy::too_many_lines)]
fn dispatch(
    provider: &RustStackKms,
    op: KmsOperation,
    body: &[u8],
) -> Result<http::Response<KmsResponseBody>, KmsError> {
    let request_id = uuid::Uuid::new_v4().to_string();

    match op {
        // Phase 0 - Key management
        KmsOperation::CreateKey => {
            let input = deserialize(body)?;
            let output = provider.create_key(input)?;
            serialize(&output, &request_id)
        }
        KmsOperation::DescribeKey => {
            let input = deserialize(body)?;
            let output = provider.describe_key(&input)?;
            serialize(&output, &request_id)
        }
        KmsOperation::ListKeys => {
            let input = deserialize(body)?;
            let output = provider.list_keys(&input)?;
            serialize(&output, &request_id)
        }
        KmsOperation::EnableKey => {
            let input = deserialize(body)?;
            provider.enable_key(&input)?;
            serialize(&serde_json::json!({}), &request_id)
        }
        KmsOperation::DisableKey => {
            let input = deserialize(body)?;
            provider.disable_key(&input)?;
            serialize(&serde_json::json!({}), &request_id)
        }
        KmsOperation::ScheduleKeyDeletion => {
            let input = deserialize(body)?;
            let output = provider.schedule_key_deletion(&input)?;
            serialize(&output, &request_id)
        }
        KmsOperation::CancelKeyDeletion => {
            let input = deserialize(body)?;
            let output = provider.cancel_key_deletion(&input)?;
            serialize(&output, &request_id)
        }
        KmsOperation::UpdateKeyDescription => {
            let input = deserialize(body)?;
            provider.update_key_description(&input)?;
            serialize(&serde_json::json!({}), &request_id)
        }
        // Phase 0 - Cryptographic operations
        KmsOperation::Encrypt => {
            let input = deserialize(body)?;
            let output = provider.encrypt(&input)?;
            serialize(&output, &request_id)
        }
        KmsOperation::Decrypt => {
            let input = deserialize(body)?;
            let output = provider.decrypt(&input)?;
            serialize(&output, &request_id)
        }
        KmsOperation::ReEncrypt => {
            let input = deserialize(body)?;
            let output = provider.re_encrypt(&input)?;
            serialize(&output, &request_id)
        }
        KmsOperation::GenerateDataKey => {
            let input = deserialize(body)?;
            let output = provider.generate_data_key(&input)?;
            serialize(&output, &request_id)
        }
        KmsOperation::GenerateDataKeyWithoutPlaintext => {
            let input = deserialize(body)?;
            let output = provider.generate_data_key_without_plaintext(&input)?;
            serialize(&output, &request_id)
        }
        KmsOperation::GenerateDataKeyPair => {
            let input = deserialize(body)?;
            let output = provider.generate_data_key_pair(&input)?;
            serialize(&output, &request_id)
        }
        KmsOperation::GenerateDataKeyPairWithoutPlaintext => {
            let input = deserialize(body)?;
            let output = provider.generate_data_key_pair_without_plaintext(&input)?;
            serialize(&output, &request_id)
        }
        KmsOperation::Sign => {
            let input = deserialize(body)?;
            let output = provider.sign(&input)?;
            serialize(&output, &request_id)
        }
        KmsOperation::Verify => {
            let input = deserialize(body)?;
            let output = provider.verify(&input)?;
            serialize(&output, &request_id)
        }
        KmsOperation::GetPublicKey => {
            let input = deserialize(body)?;
            let output = provider.get_public_key(&input)?;
            serialize(&output, &request_id)
        }
        KmsOperation::GenerateMac => {
            let input = deserialize(body)?;
            let output = provider.generate_mac(&input)?;
            serialize(&output, &request_id)
        }
        KmsOperation::VerifyMac => {
            let input = deserialize(body)?;
            let output = provider.verify_mac(&input)?;
            serialize(&output, &request_id)
        }
        KmsOperation::GenerateRandom => {
            let input = deserialize(body)?;
            let output = provider.generate_random(&input)?;
            serialize(&output, &request_id)
        }
        // Phase 1 - Aliases
        KmsOperation::CreateAlias => {
            let input = deserialize(body)?;
            provider.create_alias(&input)?;
            serialize(&serde_json::json!({}), &request_id)
        }
        KmsOperation::DeleteAlias => {
            let input = deserialize(body)?;
            provider.delete_alias(&input)?;
            serialize(&serde_json::json!({}), &request_id)
        }
        KmsOperation::ListAliases => {
            let input = deserialize(body)?;
            let output = provider.list_aliases(&input)?;
            serialize(&output, &request_id)
        }
        KmsOperation::UpdateAlias => {
            let input = deserialize(body)?;
            provider.update_alias(&input)?;
            serialize(&serde_json::json!({}), &request_id)
        }
        // Phase 1 - Tags
        KmsOperation::TagResource => {
            let input = deserialize(body)?;
            provider.tag_resource(&input)?;
            serialize(&serde_json::json!({}), &request_id)
        }
        KmsOperation::UntagResource => {
            let input = deserialize(body)?;
            provider.untag_resource(&input)?;
            serialize(&serde_json::json!({}), &request_id)
        }
        KmsOperation::ListResourceTags => {
            let input = deserialize(body)?;
            let output = provider.list_resource_tags(&input)?;
            serialize(&output, &request_id)
        }
        // Phase 1 - Key policies
        KmsOperation::GetKeyPolicy => {
            let input = deserialize(body)?;
            let output = provider.get_key_policy(&input)?;
            serialize(&output, &request_id)
        }
        KmsOperation::PutKeyPolicy => {
            let input = deserialize(body)?;
            provider.put_key_policy(&input)?;
            serialize(&serde_json::json!({}), &request_id)
        }
        KmsOperation::ListKeyPolicies => {
            let input = deserialize(body)?;
            let output = provider.list_key_policies(&input)?;
            serialize(&output, &request_id)
        }
        // Phase 2 - Grants
        KmsOperation::CreateGrant => {
            let input = deserialize(body)?;
            let output = provider.create_grant(&input)?;
            serialize(&output, &request_id)
        }
        KmsOperation::ListGrants => {
            let input = deserialize(body)?;
            let output = provider.list_grants(&input)?;
            serialize(&output, &request_id)
        }
        KmsOperation::RetireGrant => {
            let input = deserialize(body)?;
            provider.retire_grant(&input)?;
            serialize(&serde_json::json!({}), &request_id)
        }
        KmsOperation::RevokeGrant => {
            let input = deserialize(body)?;
            provider.revoke_grant(&input)?;
            serialize(&serde_json::json!({}), &request_id)
        }
        KmsOperation::ListRetirableGrants => {
            let input = deserialize(body)?;
            let output = provider.list_retirable_grants(&input)?;
            serialize(&output, &request_id)
        }
        // Phase 2 - Key rotation
        KmsOperation::EnableKeyRotation => {
            let input = deserialize(body)?;
            provider.enable_key_rotation(&input)?;
            serialize(&serde_json::json!({}), &request_id)
        }
        KmsOperation::DisableKeyRotation => {
            let input = deserialize(body)?;
            provider.disable_key_rotation(&input)?;
            serialize(&serde_json::json!({}), &request_id)
        }
        KmsOperation::GetKeyRotationStatus => {
            let input = deserialize(body)?;
            let output = provider.get_key_rotation_status(&input)?;
            serialize(&output, &request_id)
        }
    }
}

/// Deserialize a JSON request body into the input type.
fn deserialize<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, KmsError> {
    serde_json::from_slice(body).map_err(|e| {
        let msg = e.to_string();
        if msg.contains("missing field") || msg.contains("unknown variant") {
            KmsError::with_message(
                ruststack_kms_model::error::KmsErrorCode::KMSInternalException,
                format!("1 validation error detected: {msg}"),
            )
        } else {
            KmsError::with_message(
                ruststack_kms_model::error::KmsErrorCode::KMSInternalException,
                format!("Failed to deserialize request body: {e}"),
            )
        }
    })
}

/// Serialize an output type into a JSON HTTP response.
fn serialize<T: serde::Serialize>(
    output: &T,
    request_id: &str,
) -> Result<http::Response<KmsResponseBody>, KmsError> {
    let json = serde_json::to_vec(output)
        .map_err(|e| KmsError::internal_error(format!("Failed to serialize response: {e}")))?;
    Ok(json_response(json, request_id))
}
