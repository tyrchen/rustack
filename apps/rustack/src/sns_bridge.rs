//! Bridge between SNS and SQS for fan-out delivery.
//!
//! Implements the [`SqsPublisher`] trait from `rustack-sns-core` by wrapping
//! the actual SQS provider. This bridge lives in the server binary to avoid
//! a direct dependency from `rustack-sns-core` to `rustack-sqs-core`.

use std::sync::Arc;

use async_trait::async_trait;
use rustack_sns_core::{
    config::SnsConfig,
    publisher::{DeliveryError, SqsPublisher},
};
use rustack_sqs_core::provider::RustackSqs;
use rustack_sqs_model::input::SendMessageInput;

/// Production SQS publisher that delegates to the SQS provider.
#[derive(Debug)]
pub struct RustackSqsPublisher {
    sqs: Arc<RustackSqs>,
    account_id: String,
    host: String,
    port: u16,
}

impl RustackSqsPublisher {
    /// Create a new publisher wrapping the given SQS provider.
    pub fn new(sqs: Arc<RustackSqs>, config: SnsConfig) -> Self {
        Self {
            sqs,
            account_id: config.account_id,
            host: config.host,
            port: config.port,
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
            // Fallback: use the ARN as-is (shouldn't happen with valid ARNs).
            format!(
                "http://{}:{}/{}/{}",
                self.host, self.port, self.account_id, queue_arn
            )
        }
    }
}

#[async_trait]
impl SqsPublisher for RustackSqsPublisher {
    async fn send_message(
        &self,
        queue_arn: &str,
        message_body: &str,
        message_group_id: Option<&str>,
        message_deduplication_id: Option<&str>,
    ) -> Result<(), DeliveryError> {
        let queue_url = self.arn_to_queue_url(queue_arn);
        let input = SendMessageInput {
            queue_url,
            message_body: message_body.to_string(),
            message_group_id: message_group_id.map(String::from),
            message_deduplication_id: message_deduplication_id.map(String::from),
            ..SendMessageInput::default()
        };
        self.sqs
            .send_message(input)
            .await
            .map_err(|e| DeliveryError::SqsDeliveryFailed {
                queue_arn: queue_arn.to_string(),
                reason: e.to_string(),
            })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rustack_sqs_core::config::SqsConfig;

    use super::*;

    #[test]
    fn test_should_convert_arn_to_queue_url() {
        let publisher = RustackSqsPublisher {
            sqs: Arc::new(RustackSqs::new(SqsConfig::default())),
            account_id: "000000000000".to_string(),
            host: "localhost".to_string(),
            port: 4566,
        };

        let url = publisher.arn_to_queue_url("arn:aws:sqs:us-east-1:000000000000:my-queue");
        assert_eq!(url, "http://localhost:4566/000000000000/my-queue");
    }
}
