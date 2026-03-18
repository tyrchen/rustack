//! Target delivery abstraction for EventBridge.
//!
//! The `TargetDelivery` trait defines how matched events are delivered to
//! targets. Implementations may deliver to SQS, SNS, Lambda, or other
//! services. The `NoopTargetDelivery` implementation logs and discards events.

use async_trait::async_trait;

/// Errors that can occur during target delivery.
#[derive(Debug, thiserror::Error)]
pub enum DeliveryError {
    /// The target ARN is malformed or unsupported.
    #[error("Invalid target ARN: {0}")]
    InvalidArn(String),
    /// Delivery to the target failed.
    #[error("Target delivery failed: {0}")]
    TargetError(String),
}

/// Trait for delivering matched events to targets.
///
/// This trait uses `async-trait` because it requires object safety for
/// dynamic dispatch via `Arc<dyn TargetDelivery>`.
#[async_trait]
pub trait TargetDelivery: Send + Sync + std::fmt::Debug + 'static {
    /// Deliver an event (as a JSON string) to the specified target ARN.
    async fn deliver(&self, target_arn: &str, event_json: &str) -> Result<(), DeliveryError>;
}

/// A no-op delivery implementation that logs events but does not deliver them.
#[derive(Debug)]
pub struct NoopTargetDelivery;

#[async_trait]
impl TargetDelivery for NoopTargetDelivery {
    async fn deliver(&self, target_arn: &str, _event_json: &str) -> Result<(), DeliveryError> {
        tracing::debug!(target_arn = %target_arn, "NoopTargetDelivery: event not delivered");
        Ok(())
    }
}
