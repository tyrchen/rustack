//! Bridge between EventBridge and SQS for target delivery.
//!
//! Implements the [`TargetDelivery`] trait from `rustack-events-core` by wrapping
//! the actual SQS provider. This bridge lives in the server binary to avoid
//! a direct dependency from `rustack-events-core` to `rustack-sqs-core`.

use std::sync::Arc;

use async_trait::async_trait;
use rustack_events_core::delivery::{DeliveryError, TargetDelivery};
use rustack_sqs_core::provider::RustackSqs;
use rustack_sqs_model::input::SendMessageInput;

/// Production target delivery that routes events to SQS queues.
#[derive(Debug)]
pub struct LocalTargetDelivery {
    sqs: Arc<RustackSqs>,
    account_id: String,
    host: String,
    port: u16,
}

impl LocalTargetDelivery {
    /// Create a new delivery bridge wrapping the given SQS provider.
    pub fn new(sqs: Arc<RustackSqs>, account_id: String, host: String, port: u16) -> Self {
        Self {
            sqs,
            account_id,
            host,
            port,
        }
    }

    /// Convert an SQS queue ARN to a queue URL.
    ///
    /// ARN format: `arn:aws:sqs:{region}:{account}:{queue_name}`
    /// URL format: `http://{host}:{port}/{account}/{queue_name}`
    fn arn_to_queue_url(&self, queue_arn: &str) -> String {
        let parts: Vec<&str> = queue_arn.split(':').collect();
        if parts.len() >= 6 {
            let account = parts[4];
            let queue_name = parts[5];
            format!("http://{}:{}/{account}/{queue_name}", self.host, self.port)
        } else {
            format!(
                "http://{}:{}/{}/{}",
                self.host, self.port, self.account_id, queue_arn
            )
        }
    }
}

#[async_trait]
impl TargetDelivery for LocalTargetDelivery {
    async fn deliver(&self, target_arn: &str, event_json: &str) -> Result<(), DeliveryError> {
        if target_arn.contains(":sqs:") {
            let queue_url = self.arn_to_queue_url(target_arn);
            let input = SendMessageInput {
                queue_url,
                message_body: event_json.to_string(),
                ..SendMessageInput::default()
            };
            self.sqs
                .send_message(input)
                .await
                .map_err(|e| DeliveryError::TargetError(e.to_string()))?;
            Ok(())
        } else {
            tracing::debug!(
                target_arn = %target_arn,
                "unsupported target type, event not delivered"
            );
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use rustack_sqs_core::config::SqsConfig;

    use super::*;

    #[test]
    fn test_should_convert_arn_to_queue_url() {
        let delivery = LocalTargetDelivery {
            sqs: Arc::new(RustackSqs::new(SqsConfig::default())),
            account_id: "000000000000".to_string(),
            host: "localhost".to_string(),
            port: 4566,
        };

        let url = delivery.arn_to_queue_url("arn:aws:sqs:us-east-1:000000000000:my-queue");
        assert_eq!(url, "http://localhost:4566/000000000000/my-queue");
    }
}
