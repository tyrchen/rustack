//! Kinesis request router.
//!
//! Kinesis uses the `awsJson1_1` protocol where all requests are `POST /`
//! with the operation specified in the `X-Amz-Target` header:
//!
//! ```text
//! X-Amz-Target: Kinesis_20131202.PutRecord
//! ```

use ruststack_kinesis_model::error::KinesisError;
use ruststack_kinesis_model::operations::KinesisOperation;

/// The expected prefix for the `X-Amz-Target` header value.
const TARGET_PREFIX: &str = "Kinesis_20131202.";

/// Resolve a Kinesis operation from an HTTP request.
///
/// Extracts the operation from the `X-Amz-Target` header, validates the
/// format, and maps it to a [`KinesisOperation`] enum variant.
pub fn resolve_operation(headers: &http::HeaderMap) -> Result<KinesisOperation, KinesisError> {
    let target = headers
        .get("x-amz-target")
        .ok_or_else(KinesisError::missing_action)?;

    let target_str = target
        .to_str()
        .map_err(|_| KinesisError::missing_action())?;

    let operation_name = target_str
        .strip_prefix(TARGET_PREFIX)
        .ok_or_else(|| KinesisError::unknown_operation(target_str))?;

    KinesisOperation::from_name(operation_name)
        .ok_or_else(|| KinesisError::unknown_operation(target_str))
}
