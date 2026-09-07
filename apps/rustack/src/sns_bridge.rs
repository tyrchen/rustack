//! Application-owned SNS → SQS bridge; no core-to-core dependency.
use std::sync::Arc;

use async_trait::async_trait;
use rustack_sns_core::publisher::{DeliveryError, SqsPublisher};
use rustack_sqs_core::provider::RustackSqs;
use rustack_sqs_model::input::SendMessageInput;

/// Production publisher using the SQS provider's authoritative ARN/URL scope.
#[derive(Debug)]
pub struct RustackSqsPublisher {
    sqs: Arc<RustackSqs>,
}
impl RustackSqsPublisher {
    /// Connect to an enabled SQS provider.
    pub fn new(sqs: Arc<RustackSqs>) -> Self {
        Self { sqs }
    }
}
#[async_trait]
impl SqsPublisher for RustackSqsPublisher {
    fn validate(&self, arn: &str) -> Result<(), DeliveryError> {
        self.sqs
            .queue_url_for_arn(arn)
            .map(|_| ())
            .map_err(|error| DeliveryError::Unsupported(error.to_string()))
    }
    async fn send_message(
        &self,
        queue_arn: &str,
        body: &str,
        group: Option<&str>,
        dedup: Option<&str>,
    ) -> Result<(), DeliveryError> {
        self.validate(queue_arn)?;
        let queue_url = self
            .sqs
            .queue_url_for_arn(queue_arn)
            .map_err(|error| DeliveryError::Unsupported(error.to_string()))?;
        self.sqs
            .send_message(SendMessageInput {
                queue_url,
                message_body: body.to_owned(),
                message_group_id: group.map(str::to_owned),
                message_deduplication_id: dedup.map(str::to_owned),
                ..Default::default()
            })
            .await
            .map_err(|error| DeliveryError::SqsDeliveryFailed {
                queue_arn: queue_arn.to_owned(),
                reason: error.to_string(),
            })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rustack_sns_core::{config::SnsConfig, provider::RustackSns};
    use rustack_sqs_core::config::SqsConfig;
    use rustack_sqs_model::input::{CreateQueueInput, ReceiveMessageInput};

    use super::*;

    #[tokio::test]
    async fn test_should_deliver_sns_fifo_to_real_queue_with_identity() {
        let sqs = Arc::new(RustackSqs::new(SqsConfig::default()));
        let queue = sqs
            .create_queue(
                serde_json::from_value::<CreateQueueInput>(serde_json::json!({
                    "QueueName":"sns.fifo","Attributes":{"FifoQueue":"true"}
                }))
                .unwrap(),
            )
            .await
            .unwrap()
            .queue_url
            .unwrap();
        let sns = RustackSns::new(
            SnsConfig::default(),
            Arc::new(RustackSqsPublisher::new(sqs.clone())),
        );
        let topic = sns
            .create_topic(
                serde_json::from_value(
                    serde_json::json!({"Name":"topic.fifo","Attributes":{"FifoTopic":"true"}}),
                )
                .unwrap(),
            )
            .unwrap()
            .topic_arn;
        sns.subscribe(serde_json::from_value(serde_json::json!({"TopicArn":topic,"Protocol":"sqs","Endpoint":"arn:aws:sqs:us-east-1:000000000000:sns.fifo"})).unwrap()).unwrap();
        sns.publish(serde_json::from_value(serde_json::json!({"TopicArn":topic,"Message":"payload","MessageGroupId":"g","MessageDeduplicationId":"d"})).unwrap()).await.unwrap();
        sns.quiesce().await;
        let messages = sqs
            .receive_message(ReceiveMessageInput {
                queue_url: queue,
                message_system_attribute_names: vec!["All".into()],
                ..Default::default()
            })
            .await
            .unwrap()
            .messages;
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0]
                .attributes
                .get("MessageGroupId")
                .map(String::as_str),
            Some("g")
        );
        assert_eq!(
            messages[0]
                .attributes
                .get("MessageDeduplicationId")
                .map(String::as_str),
            Some("d")
        );
        assert_eq!(sns.delivery_stats().delivered, 1);
        sqs.shutdown_all().await;
    }
}
