//! KMS request router.
//!
//! KMS uses the `awsJson1_1` protocol where all requests are `POST /`
//! with the operation specified in the `X-Amz-Target` header:
//!
//! ```text
//! X-Amz-Target: TrentService.CreateKey
//! ```

use ruststack_kms_model::error::KmsError;
use ruststack_kms_model::operations::KmsOperation;

/// The expected prefix for the `X-Amz-Target` header value.
const TARGET_PREFIX: &str = "TrentService.";

/// Resolve a KMS operation from an HTTP request.
///
/// Extracts the operation from the `X-Amz-Target` header, validates the
/// format, and maps it to a [`KmsOperation`] enum variant.
pub fn resolve_operation(headers: &http::HeaderMap) -> Result<KmsOperation, KmsError> {
    let target = headers
        .get("x-amz-target")
        .ok_or_else(KmsError::missing_action)?;

    let target_str = target.to_str().map_err(|_| KmsError::missing_action())?;

    let operation_name = target_str
        .strip_prefix(TARGET_PREFIX)
        .ok_or_else(|| KmsError::unknown_operation(target_str))?;

    KmsOperation::from_name(operation_name).ok_or_else(|| KmsError::unknown_operation(target_str))
}
