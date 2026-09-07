//! End-to-end DLQ and tuple-key regressions using real actors.
use rustack_sqs_model::types::Message;

use super::*;

async fn create(provider: &RustackSqs, name: &str, target: Option<&str>) -> String {
    let mut attributes = HashMap::new();
    if name
        .rsplit_once('.')
        .is_some_and(|(_, suffix)| suffix == "fifo")
    {
        attributes.insert("FifoQueue".into(), "true".into());
        attributes.insert("ContentBasedDeduplication".into(), "true".into());
        attributes.insert("DeduplicationScope".into(), "messageGroup".into());
    }
    if let Some(target) = target {
        attributes.insert("RedrivePolicy".into(), serde_json::json!({
            "deadLetterTargetArn":format!("arn:aws:sqs:us-east-1:000000000000:{target}"), "maxReceiveCount":1
        }).to_string());
    }
    provider
        .create_queue(CreateQueueInput {
            queue_name: name.into(),
            attributes,
            tags: HashMap::new(),
        })
        .await
        .unwrap()
        .queue_url
        .unwrap()
}

async fn receive(provider: &RustackSqs, url: &str) -> Vec<Message> {
    provider
        .receive_message(ReceiveMessageInput {
            queue_url: url.into(),
            max_number_of_messages: Some(10),
            visibility_timeout: Some(0),
            message_system_attribute_names: vec!["All".into()],
            ..Default::default()
        })
        .await
        .unwrap()
        .messages
}

#[tokio::test]
async fn test_should_deliver_standard_and_fifo_dead_letters_to_real_target() {
    for suffix in ["", ".fifo"] {
        let provider = RustackSqs::new(SqsConfig::default());
        let target_name = format!("dead{suffix}");
        let target = create(&provider, &target_name, None).await;
        let source = create(&provider, &format!("source{suffix}"), Some(&target_name)).await;
        provider
            .send_message(SendMessageInput {
                queue_url: source.clone(),
                message_body: "body".into(),
                message_group_id: if suffix.is_empty() {
                    None
                } else {
                    Some("g".into())
                },
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(receive(&provider, &source).await.len(), 1);
        assert!(receive(&provider, &source).await.is_empty());
        provider.quiesce().await.unwrap();
        let messages = receive(&provider, &target).await;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].body.as_deref(), Some("body"));
        assert_eq!(
            messages[0].attributes.get("DeadLetterQueueSourceArn"),
            Some(&format!(
                "arn:aws:sqs:us-east-1:000000000000:source{suffix}"
            ))
        );
        assert!(receive(&provider, &source).await.is_empty());
        provider.shutdown_all().await;
    }
}

#[tokio::test]
async fn test_should_retain_dead_letters_until_missing_target_becomes_available() {
    for suffix in ["", ".fifo"] {
        let provider = RustackSqs::new(SqsConfig::default());
        let target_name = format!("missing{suffix}");
        let source = create(&provider, &format!("source{suffix}"), Some(&target_name)).await;
        provider
            .send_message(SendMessageInput {
                queue_url: source.clone(),
                message_body: "retained".into(),
                message_group_id: if suffix.is_empty() {
                    None
                } else {
                    Some("g".into())
                },
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(receive(&provider, &source).await.len(), 1);
        assert!(receive(&provider, &source).await.is_empty());
        let target = create(&provider, &target_name, None).await;
        assert!(receive(&provider, &source).await.is_empty());
        provider.quiesce().await.unwrap();
        assert_eq!(receive(&provider, &target).await.len(), 1);
        provider.shutdown_all().await;
    }
}

#[tokio::test]
async fn test_should_restore_source_after_target_rejects_transfer() {
    for suffix in ["", ".fifo"] {
        let provider = RustackSqs::new(SqsConfig::default());
        let target_name = format!("denied{suffix}");
        let target = create(&provider, &target_name, None).await;
        provider
            .set_queue_attributes(SetQueueAttributesInput {
                queue_url: target.clone(),
                attributes: HashMap::from([(
                    "RedriveAllowPolicy".into(),
                    "{\"redrivePermission\":\"denyAll\"}".into(),
                )]),
            })
            .await
            .unwrap();
        let source = create(&provider, &format!("source{suffix}"), Some(&target_name)).await;
        provider
            .send_message(SendMessageInput {
                queue_url: source.clone(),
                message_body: "retain".into(),
                message_group_id: if suffix.is_empty() {
                    None
                } else {
                    Some("g".into())
                },
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(receive(&provider, &source).await.len(), 1);
        assert!(receive(&provider, &source).await.is_empty());
        provider.quiesce().await.unwrap();
        let attributes = provider
            .get_queue_attributes(GetQueueAttributesInput {
                queue_url: source,
                attribute_names: vec!["All".into()],
            })
            .await
            .unwrap()
            .attributes;
        assert_eq!(
            attributes
                .get("ApproximateNumberOfMessages")
                .map(String::as_str),
            Some("1")
        );
        assert!(receive(&provider, &target).await.is_empty());
        provider.shutdown_all().await;
    }
}

#[tokio::test]
async fn test_should_retain_source_when_target_command_channel_is_full_or_closed() {
    use crate::queue::actor::QueueCommand;
    for closed in [false, true] {
        let provider = RustackSqs::new(SqsConfig::default());
        let target_url = create(&provider, "target", None).await;
        let source = create(&provider, "source", Some("target")).await;
        let real_target = provider.get_queue(&target_url).unwrap();
        let (sender, receiver) = mpsc::channel(1);
        let (_finished, completion) = tokio::sync::watch::channel(Some(true));
        let _receiver = if closed {
            drop(receiver);
            None
        } else {
            sender.try_send(QueueCommand::Shutdown).unwrap();
            Some(receiver)
        };
        provider.queues.insert(
            "target".into(),
            Arc::new(QueueHandle {
                sender,
                metadata: real_target.metadata.clone(),
                completion,
                shutdown: Arc::new(AtomicBool::new(false)),
            }),
        );
        provider
            .send_message(SendMessageInput {
                queue_url: source.clone(),
                message_body: "retain".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(receive(&provider, &source).await.len(), 1);
        assert!(receive(&provider, &source).await.is_empty());
        provider.queues.insert("target".into(), real_target);
        assert!(receive(&provider, &source).await.is_empty());
        provider.quiesce().await.unwrap();
        assert_eq!(receive(&provider, &target_url).await.len(), 1);
        provider.shutdown_all().await;
    }
}

#[tokio::test]
async fn test_should_not_collide_fifo_group_deduplication_tuples() {
    let provider = RustackSqs::new(SqsConfig::default());
    let url = create(&provider, "tuples.fifo", None).await;
    let mut ids = Vec::new();
    for (group, dedup) in [("a:b", "c"), ("a", "b:c"), ("a:b", "c"), ("other", "c")] {
        ids.push(
            provider
                .send_message(SendMessageInput {
                    queue_url: url.clone(),
                    message_body: format!("{group}/{dedup}"),
                    message_group_id: Some(group.into()),
                    message_deduplication_id: Some(dedup.into()),
                    ..Default::default()
                })
                .await
                .unwrap()
                .message_id,
        );
    }
    assert_eq!(ids[0], ids[2]);
    assert_ne!(ids[0], ids[1]);
    assert_ne!(ids[0], ids[3]);
    assert_eq!(receive(&provider, &url).await.len(), 3);
    provider.shutdown_all().await;
}
