//! Application-owned EventBridge → SQS delivery bridge.
use std::sync::Arc;

use async_trait::async_trait;
use rustack_events_core::delivery::{DeliveryError, Target, TargetDelivery};
use rustack_sqs_core::provider::RustackSqs;
use rustack_sqs_model::input::SendMessageInput;

/// Actual in-process SQS target delivery, preserving all FIFO parameters.
#[derive(Debug)]
pub struct LocalTargetDelivery {
    sqs: Arc<RustackSqs>,
}
impl LocalTargetDelivery {
    /// Connect the bridge to an enabled SQS provider.
    pub fn new(sqs: Arc<RustackSqs>) -> Self {
        Self { sqs }
    }
    fn arn_to_queue_url(&self, arn: &str) -> Result<String, DeliveryError> {
        self.sqs
            .queue_url_for_arn(arn)
            .map_err(|error| DeliveryError::InvalidArn(error.to_string()))
    }
}
#[async_trait]
impl TargetDelivery for LocalTargetDelivery {
    fn validate(&self, target: &Target) -> Result<(), DeliveryError> {
        if let ["arn", _, service, _, _, _] = target.arn.split(':').collect::<Vec<_>>().as_slice() {
            if *service != "sqs" {
                return Err(DeliveryError::Unsupported(
                    "Only SQS targets are executable".into(),
                ));
            }
        }
        self.arn_to_queue_url(&target.arn)?;
        let fifo = std::path::Path::new(&target.arn)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("fifo"));
        if fifo && target.sqs_parameters.is_none() {
            return Err(DeliveryError::Unsupported(
                "FIFO target requires SqsParameters.MessageGroupId".into(),
            ));
        }
        if !fifo && target.sqs_parameters.is_some() {
            return Err(DeliveryError::Unsupported(
                "SqsParameters requires a FIFO queue".into(),
            ));
        }
        Ok(())
    }
    async fn deliver(&self, target: &Target, event_json: &str) -> Result<(), DeliveryError> {
        self.validate(target)?;
        self.sqs
            .send_message(SendMessageInput {
                queue_url: self.arn_to_queue_url(&target.arn)?,
                message_body: event_json.to_owned(),
                message_group_id: target
                    .sqs_parameters
                    .as_ref()
                    .map(|parameters| parameters.message_group_id.clone()),
                ..Default::default()
            })
            .await
            .map_err(|error| DeliveryError::TargetError(error.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rustack_events_core::{config::EventsConfig, provider::RustackEvents};
    use rustack_sqs_core::config::SqsConfig;
    use rustack_sqs_model::input::{CreateQueueInput, ReceiveMessageInput};

    use super::*;

    #[tokio::test]
    async fn test_should_roundtrip_sqs_parameters_and_deliver_fifo_group() {
        let sqs = Arc::new(RustackSqs::new(SqsConfig::default()));
        let queue = sqs.create_queue(serde_json::from_value::<CreateQueueInput>(serde_json::json!({
            "QueueName":"events.fifo", "Attributes":{"FifoQueue":"true","ContentBasedDeduplication":"true"}
        })).unwrap()).await.unwrap().queue_url.unwrap();
        let bridge = Arc::new(LocalTargetDelivery::new(sqs.clone()));
        let events = RustackEvents::new(EventsConfig::default(), bridge);
        events
            .handle_put_rule(
                serde_json::from_value(
                    serde_json::json!({"Name":"rule", "EventPattern":"{\"source\":[\"test\"]}"}),
                )
                .unwrap(),
            )
            .unwrap();
        let target = serde_json::json!({"Id":"fifo","Arn":"arn:aws:sqs:us-east-1:000000000000:events.fifo", "SqsParameters":{"MessageGroupId":"g"}});
        let result = events
            .handle_put_targets(
                serde_json::from_value(
                    serde_json::json!({"Rule":"rule","Targets":[target.clone()]}),
                )
                .unwrap(),
            )
            .unwrap();
        assert_eq!(result.failed_entry_count, 0);
        let listed = events
            .handle_list_targets_by_rule(
                &serde_json::from_value(serde_json::json!({"Rule":"rule"})).unwrap(),
            )
            .unwrap();
        assert_eq!(serde_json::to_value(&listed.targets[0]).unwrap(), target);
        let result = events.handle_put_events(&serde_json::from_value(serde_json::json!({"Entries":[{"Source":"test","DetailType":"test","Detail":"{}"}]})).unwrap()).unwrap();
        assert_eq!(result.failed_entry_count, 0);
        events.quiesce().await.unwrap();
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
        assert_eq!(events.delivery_stats().delivered, 1);
        sqs.shutdown_all().await;
    }
}
