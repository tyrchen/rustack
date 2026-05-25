//! SQS integration tests against a running `Rustack` server.
//!
//! These tests cover queue lifecycle, message operations, batch operations,
//! FIFO queues, tags, and error handling.

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::{create_test_queue, delete_test_queue, sqs_client, test_queue_name};

    // ---------------------------------------------------------------------------
    // Queue Management
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_and_delete_queue() {
        let client = sqs_client();
        let (name, url) = create_test_queue(&client, "lifecycle").await;

        // Verify queue URL contains the name.
        assert!(url.contains(&name));

        // Get queue URL by name.
        let got_url = client
            .get_queue_url()
            .queue_name(&name)
            .send()
            .await
            .unwrap()
            .queue_url()
            .unwrap()
            .to_string();
        assert_eq!(got_url, url);

        delete_test_queue(&client, &url).await;

        // Verify queue is gone.
        let err = client.get_queue_url().queue_name(&name).send().await;
        assert!(err.is_err());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_list_queues() {
        let client = sqs_client();
        let prefix = &uuid::Uuid::new_v4().to_string()[..8];
        let name1 = format!("list-{prefix}-a");
        let name2 = format!("list-{prefix}-b");

        let url1 = client
            .create_queue()
            .queue_name(&name1)
            .send()
            .await
            .unwrap()
            .queue_url()
            .unwrap()
            .to_string();
        let url2 = client
            .create_queue()
            .queue_name(&name2)
            .send()
            .await
            .unwrap()
            .queue_url()
            .unwrap()
            .to_string();

        let result = client
            .list_queues()
            .queue_name_prefix(format!("list-{prefix}"))
            .send()
            .await
            .unwrap();

        let queue_urls = result.queue_urls();
        assert!(
            queue_urls.len() >= 2,
            "expected at least 2 queues, got {}",
            queue_urls.len()
        );
        assert!(queue_urls.contains(&url1));
        assert!(queue_urls.contains(&url2));

        delete_test_queue(&client, &url1).await;
        delete_test_queue(&client, &url2).await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_queue_idempotently() {
        let client = sqs_client();
        let name = test_queue_name("idempotent");

        let url1 = client
            .create_queue()
            .queue_name(&name)
            .send()
            .await
            .unwrap()
            .queue_url()
            .unwrap()
            .to_string();

        // Same name, no attributes = idempotent.
        let url2 = client
            .create_queue()
            .queue_name(&name)
            .send()
            .await
            .unwrap()
            .queue_url()
            .unwrap()
            .to_string();

        assert_eq!(url1, url2);

        delete_test_queue(&client, &url1).await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_get_and_set_queue_attributes() {
        let client = sqs_client();
        let (_, url) = create_test_queue(&client, "attrs").await;

        // Set custom visibility timeout.
        client
            .set_queue_attributes()
            .queue_url(&url)
            .attributes(
                aws_sdk_sqs::types::QueueAttributeName::VisibilityTimeout,
                "60",
            )
            .send()
            .await
            .unwrap();

        // Get and verify.
        let attrs = client
            .get_queue_attributes()
            .queue_url(&url)
            .attribute_names(aws_sdk_sqs::types::QueueAttributeName::VisibilityTimeout)
            .send()
            .await
            .unwrap();

        let map = attrs.attributes().unwrap();
        assert_eq!(
            map.get(&aws_sdk_sqs::types::QueueAttributeName::VisibilityTimeout)
                .unwrap(),
            "60"
        );

        delete_test_queue(&client, &url).await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_purge_queue() {
        let client = sqs_client();
        let (_, url) = create_test_queue(&client, "purge").await;

        // Send messages.
        for i in 0..5 {
            client
                .send_message()
                .queue_url(&url)
                .message_body(format!("msg-{i}"))
                .send()
                .await
                .unwrap();
        }

        // Purge.
        client.purge_queue().queue_url(&url).send().await.unwrap();

        // Verify empty.
        let result = client
            .receive_message()
            .queue_url(&url)
            .max_number_of_messages(10)
            .wait_time_seconds(0)
            .send()
            .await
            .unwrap();

        assert!(result.messages().is_empty());

        delete_test_queue(&client, &url).await;
    }

    // ---------------------------------------------------------------------------
    // Message Operations
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_send_and_receive_message() {
        let client = sqs_client();
        let (_, url) = create_test_queue(&client, "send-recv").await;

        let send_result = client
            .send_message()
            .queue_url(&url)
            .message_body("hello world")
            .send()
            .await
            .unwrap();

        assert!(send_result.message_id().is_some());
        assert!(send_result.md5_of_message_body().is_some());

        let recv_result = client
            .receive_message()
            .queue_url(&url)
            .max_number_of_messages(1)
            .wait_time_seconds(0)
            .send()
            .await
            .unwrap();

        let messages = recv_result.messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].body().unwrap(), "hello world");
        assert!(messages[0].receipt_handle().is_some());
        assert_eq!(
            messages[0].message_id().unwrap(),
            send_result.message_id().unwrap()
        );

        delete_test_queue(&client, &url).await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_delete_message() {
        let client = sqs_client();
        let (_, url) = create_test_queue(&client, "delete-msg").await;

        client
            .send_message()
            .queue_url(&url)
            .message_body("to-delete")
            .send()
            .await
            .unwrap();

        let recv = client
            .receive_message()
            .queue_url(&url)
            .max_number_of_messages(1)
            .wait_time_seconds(0)
            .send()
            .await
            .unwrap();

        let receipt_handle = recv.messages()[0].receipt_handle().unwrap();

        // Delete the message.
        client
            .delete_message()
            .queue_url(&url)
            .receipt_handle(receipt_handle)
            .send()
            .await
            .unwrap();

        // Verify gone (with short poll).
        let recv2 = client
            .receive_message()
            .queue_url(&url)
            .max_number_of_messages(1)
            .wait_time_seconds(0)
            .send()
            .await
            .unwrap();

        assert!(recv2.messages().is_empty());

        delete_test_queue(&client, &url).await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_send_message_with_attributes() {
        let client = sqs_client();
        let (_, url) = create_test_queue(&client, "msg-attrs").await;

        let attr_value = aws_sdk_sqs::types::MessageAttributeValue::builder()
            .data_type("String")
            .string_value("test-value")
            .build()
            .unwrap();

        client
            .send_message()
            .queue_url(&url)
            .message_body("with attributes")
            .message_attributes("myAttr", attr_value)
            .send()
            .await
            .unwrap();

        let recv = client
            .receive_message()
            .queue_url(&url)
            .message_attribute_names("All")
            .wait_time_seconds(0)
            .send()
            .await
            .unwrap();

        let msg = &recv.messages()[0];
        let attrs = msg.message_attributes().unwrap();
        assert!(attrs.contains_key("myAttr"));
        assert_eq!(attrs["myAttr"].string_value().unwrap(), "test-value");

        delete_test_queue(&client, &url).await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_send_message_with_delay() {
        let client = sqs_client();
        let (_, url) = create_test_queue(&client, "delay").await;

        client
            .send_message()
            .queue_url(&url)
            .message_body("delayed message")
            .delay_seconds(2)
            .send()
            .await
            .unwrap();

        // Immediately should be empty.
        let recv = client
            .receive_message()
            .queue_url(&url)
            .max_number_of_messages(1)
            .wait_time_seconds(0)
            .send()
            .await
            .unwrap();
        assert!(recv.messages().is_empty());

        // Wait for delay.
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        let recv2 = client
            .receive_message()
            .queue_url(&url)
            .max_number_of_messages(1)
            .wait_time_seconds(0)
            .send()
            .await
            .unwrap();
        assert_eq!(recv2.messages().len(), 1);
        assert_eq!(recv2.messages()[0].body().unwrap(), "delayed message");

        delete_test_queue(&client, &url).await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_receive_system_attributes() {
        let client = sqs_client();
        let (_, url) = create_test_queue(&client, "sys-attrs").await;

        client
            .send_message()
            .queue_url(&url)
            .message_body("system attrs test")
            .send()
            .await
            .unwrap();

        let recv = client
            .receive_message()
            .queue_url(&url)
            .message_system_attribute_names(aws_sdk_sqs::types::MessageSystemAttributeName::All)
            .wait_time_seconds(0)
            .send()
            .await
            .unwrap();

        let msg = &recv.messages()[0];
        let attrs = msg.attributes().unwrap();

        // Should have system attributes.
        assert!(
            attrs.contains_key(&aws_sdk_sqs::types::MessageSystemAttributeName::SentTimestamp),
            "missing SentTimestamp"
        );
        assert!(
            attrs.contains_key(
                &aws_sdk_sqs::types::MessageSystemAttributeName::ApproximateReceiveCount
            ),
            "missing ApproximateReceiveCount"
        );

        delete_test_queue(&client, &url).await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_change_message_visibility() {
        let client = sqs_client();
        let (_, url) = create_test_queue(&client, "vis-change").await;

        client
            .send_message()
            .queue_url(&url)
            .message_body("visibility test")
            .send()
            .await
            .unwrap();

        // Receive with long visibility timeout.
        let recv = client
            .receive_message()
            .queue_url(&url)
            .visibility_timeout(30)
            .wait_time_seconds(0)
            .send()
            .await
            .unwrap();

        let receipt_handle = recv.messages()[0].receipt_handle().unwrap();

        // Change visibility to 0 to make immediately available.
        client
            .change_message_visibility()
            .queue_url(&url)
            .receipt_handle(receipt_handle)
            .visibility_timeout(0)
            .send()
            .await
            .unwrap();

        // Should be immediately available.
        let recv2 = client
            .receive_message()
            .queue_url(&url)
            .max_number_of_messages(1)
            .wait_time_seconds(0)
            .send()
            .await
            .unwrap();

        assert_eq!(recv2.messages().len(), 1);

        delete_test_queue(&client, &url).await;
    }

    // ---------------------------------------------------------------------------
    // Batch Operations
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_send_message_batch() {
        let client = sqs_client();
        let (_, url) = create_test_queue(&client, "batch-send").await;

        let entries: Vec<_> = (0..5)
            .map(|i| {
                aws_sdk_sqs::types::SendMessageBatchRequestEntry::builder()
                    .id(format!("msg-{i}"))
                    .message_body(format!("body-{i}"))
                    .build()
                    .unwrap()
            })
            .collect();

        let result = client
            .send_message_batch()
            .queue_url(&url)
            .set_entries(Some(entries))
            .send()
            .await
            .unwrap();

        assert_eq!(result.successful().len(), 5);
        assert!(result.failed().is_empty());

        // Receive all messages.
        let recv = client
            .receive_message()
            .queue_url(&url)
            .max_number_of_messages(10)
            .wait_time_seconds(0)
            .send()
            .await
            .unwrap();

        assert_eq!(recv.messages().len(), 5);

        delete_test_queue(&client, &url).await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_delete_message_batch() {
        let client = sqs_client();
        let (_, url) = create_test_queue(&client, "batch-del").await;

        // Send messages.
        for i in 0..3 {
            client
                .send_message()
                .queue_url(&url)
                .message_body(format!("batch-del-{i}"))
                .send()
                .await
                .unwrap();
        }

        // Receive all.
        let recv = client
            .receive_message()
            .queue_url(&url)
            .max_number_of_messages(10)
            .wait_time_seconds(0)
            .send()
            .await
            .unwrap();

        // Build batch delete entries.
        let entries: Vec<_> = recv
            .messages()
            .iter()
            .enumerate()
            .map(|(i, msg)| {
                aws_sdk_sqs::types::DeleteMessageBatchRequestEntry::builder()
                    .id(format!("del-{i}"))
                    .receipt_handle(msg.receipt_handle().unwrap())
                    .build()
                    .unwrap()
            })
            .collect();

        let result = client
            .delete_message_batch()
            .queue_url(&url)
            .set_entries(Some(entries))
            .send()
            .await
            .unwrap();

        assert_eq!(result.successful().len(), 3);

        // Verify empty.
        let recv2 = client
            .receive_message()
            .queue_url(&url)
            .max_number_of_messages(10)
            .wait_time_seconds(0)
            .send()
            .await
            .unwrap();
        assert!(recv2.messages().is_empty());

        delete_test_queue(&client, &url).await;
    }

    // ---------------------------------------------------------------------------
    // FIFO Queues
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_fifo_queue() {
        let client = sqs_client();
        let id = &uuid::Uuid::new_v4().to_string()[..8];
        let name = format!("test-fifo-{id}.fifo");

        let url = client
            .create_queue()
            .queue_name(&name)
            .attributes(aws_sdk_sqs::types::QueueAttributeName::FifoQueue, "true")
            .send()
            .await
            .unwrap()
            .queue_url()
            .unwrap()
            .to_string();

        assert!(url.contains(".fifo"));

        // Verify FifoQueue attribute.
        let attrs = client
            .get_queue_attributes()
            .queue_url(&url)
            .attribute_names(aws_sdk_sqs::types::QueueAttributeName::FifoQueue)
            .send()
            .await
            .unwrap();

        let map = attrs.attributes().unwrap();
        assert_eq!(
            map.get(&aws_sdk_sqs::types::QueueAttributeName::FifoQueue)
                .unwrap(),
            "true"
        );

        delete_test_queue(&client, &url).await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_send_and_receive_fifo_messages() {
        let client = sqs_client();
        let id = &uuid::Uuid::new_v4().to_string()[..8];
        let name = format!("test-fifo-msg-{id}.fifo");

        let url = client
            .create_queue()
            .queue_name(&name)
            .attributes(aws_sdk_sqs::types::QueueAttributeName::FifoQueue, "true")
            .attributes(
                aws_sdk_sqs::types::QueueAttributeName::ContentBasedDeduplication,
                "true",
            )
            .send()
            .await
            .unwrap()
            .queue_url()
            .unwrap()
            .to_string();

        // Send messages in order.
        for i in 0..3 {
            let result = client
                .send_message()
                .queue_url(&url)
                .message_body(format!("fifo-msg-{i}"))
                .message_group_id("group-a")
                .send()
                .await
                .unwrap();
            assert!(result.sequence_number().is_some());
        }

        // Receive and verify FIFO ordering.
        let recv = client
            .receive_message()
            .queue_url(&url)
            .max_number_of_messages(10)
            .wait_time_seconds(0)
            .send()
            .await
            .unwrap();

        // FIFO delivers one message per group at a time.
        assert_eq!(recv.messages().len(), 1);
        assert_eq!(recv.messages()[0].body().unwrap(), "fifo-msg-0");

        // Delete and receive next.
        client
            .delete_message()
            .queue_url(&url)
            .receipt_handle(recv.messages()[0].receipt_handle().unwrap())
            .send()
            .await
            .unwrap();

        let recv2 = client
            .receive_message()
            .queue_url(&url)
            .max_number_of_messages(10)
            .wait_time_seconds(0)
            .send()
            .await
            .unwrap();

        assert_eq!(recv2.messages().len(), 1);
        assert_eq!(recv2.messages()[0].body().unwrap(), "fifo-msg-1");

        delete_test_queue(&client, &url).await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_deduplicate_fifo_messages() {
        let client = sqs_client();
        let id = &uuid::Uuid::new_v4().to_string()[..8];
        let name = format!("test-fifo-dedup-{id}.fifo");

        let url = client
            .create_queue()
            .queue_name(&name)
            .attributes(aws_sdk_sqs::types::QueueAttributeName::FifoQueue, "true")
            .send()
            .await
            .unwrap()
            .queue_url()
            .unwrap()
            .to_string();

        // Send same dedup ID twice.
        let result1 = client
            .send_message()
            .queue_url(&url)
            .message_body("dedup-test")
            .message_group_id("group-a")
            .message_deduplication_id("dedup-1")
            .send()
            .await
            .unwrap();

        let result2 = client
            .send_message()
            .queue_url(&url)
            .message_body("dedup-test-different-body")
            .message_group_id("group-a")
            .message_deduplication_id("dedup-1")
            .send()
            .await
            .unwrap();

        // Both should return the same message ID (original).
        assert_eq!(result1.message_id().unwrap(), result2.message_id().unwrap());

        // Only one message should be in the queue.
        let recv = client
            .receive_message()
            .queue_url(&url)
            .max_number_of_messages(10)
            .wait_time_seconds(0)
            .send()
            .await
            .unwrap();

        assert_eq!(recv.messages().len(), 1);
        assert_eq!(recv.messages()[0].body().unwrap(), "dedup-test");

        delete_test_queue(&client, &url).await;
    }

    // ---------------------------------------------------------------------------
    // Tags
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_manage_queue_tags() {
        let client = sqs_client();
        let (_, url) = create_test_queue(&client, "tags").await;

        // Tag the queue.
        let mut tags = HashMap::new();
        tags.insert("env".to_owned(), "test".to_owned());
        tags.insert("project".to_owned(), "rustack".to_owned());

        client
            .tag_queue()
            .queue_url(&url)
            .set_tags(Some(tags))
            .send()
            .await
            .unwrap();

        // List tags.
        let result = client
            .list_queue_tags()
            .queue_url(&url)
            .send()
            .await
            .unwrap();

        let tags = result.tags().unwrap();
        assert_eq!(tags.get("env").unwrap(), "test");
        assert_eq!(tags.get("project").unwrap(), "rustack");

        // Untag.
        client
            .untag_queue()
            .queue_url(&url)
            .tag_keys("env")
            .send()
            .await
            .unwrap();

        let result2 = client
            .list_queue_tags()
            .queue_url(&url)
            .send()
            .await
            .unwrap();

        let tags2 = result2.tags().unwrap();
        assert!(!tags2.contains_key("env"));
        assert!(tags2.contains_key("project"));

        delete_test_queue(&client, &url).await;
    }

    // ---------------------------------------------------------------------------
    // Error Handling
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_error_on_nonexistent_queue() {
        let client = sqs_client();

        let err = client
            .get_queue_url()
            .queue_name("nonexistent-queue-12345")
            .send()
            .await;

        assert!(err.is_err());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_error_on_invalid_queue_name() {
        let client = sqs_client();

        // Queue name with invalid characters.
        let err = client
            .create_queue()
            .queue_name("invalid queue name!")
            .send()
            .await;

        assert!(err.is_err());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_reject_fifo_fields_on_standard_queue() {
        let client = sqs_client();
        let (_, url) = create_test_queue(&client, "no-fifo").await;

        // Sending with MessageGroupId on a standard queue should fail.
        let err = client
            .send_message()
            .queue_url(&url)
            .message_body("test")
            .message_group_id("group-a")
            .send()
            .await;

        assert!(err.is_err());

        delete_test_queue(&client, &url).await;
    }

    // ---------------------------------------------------------------------------
    // Bug: Long-poll does not wake up when a message is sent mid-poll
    // ---------------------------------------------------------------------------
    //
    // ReceiveMessage with WaitTimeSeconds>0 should return immediately when a
    // message is sent to the queue while the long-poll is in progress.
    //
    // Root cause: the actor event loop uses `tokio::select!` with both
    // `commands.recv()` and `message_notify.notified()` as arms. When a
    // SendMessage command arrives via `commands.recv()`, it calls
    // `notify_waiters()` inside `handle_command`. But the `notified()` future
    // lost the select race and is not being polled, so the notification is lost.
    // The pending long-poll is only fulfilled when it expires (WaitTimeSeconds
    // later), not when the message becomes available.

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_long_poll_wakes_on_new_message() {
        let client = sqs_client();
        let (_, url) = create_test_queue(&client, "longpoll-wake").await;

        let recv_client = sqs_client();
        let recv_url = url.clone();

        // Start a long-poll in a background task (20s wait time).
        let recv_handle = tokio::spawn(async move {
            let start = std::time::Instant::now();
            let result = recv_client
                .receive_message()
                .queue_url(&recv_url)
                .max_number_of_messages(1)
                .wait_time_seconds(20)
                .send()
                .await
                .unwrap();
            (result, start.elapsed())
        });

        // Give the long-poll a moment to register.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Send a message while the long-poll is in progress.
        client
            .send_message()
            .queue_url(&url)
            .message_body("wake up!")
            .send()
            .await
            .unwrap();

        // The long-poll should return almost immediately (well under 20s).
        let (result, elapsed) = recv_handle.await.unwrap();

        assert_eq!(result.messages().len(), 1);
        assert_eq!(result.messages()[0].body().unwrap(), "wake up!");
        assert!(
            elapsed.as_secs() < 5,
            "long-poll took {elapsed:?} — should have woken up immediately, \
             not waited for the full WaitTimeSeconds"
        );

        delete_test_queue(&client, &url).await;
    }

    // ---------------------------------------------------------------------------
    // Bug: DashMap guard held across .await causes deadlock under concurrency
    // ---------------------------------------------------------------------------
    //
    // The SQS provider's `get_queue()` returns a `DashMap::Ref` (read guard).
    // All operation methods (send_message, receive_message, etc.) hold this
    // guard while `.await`-ing the actor's response via a oneshot channel.
    //
    // Under concurrent load — especially when CreateQueue and DeleteQueue run
    // alongside message operations — the DashMap shard locks contend and the
    // runtime deadlocks: a task holding a read guard awaits on the actor, while
    // another task needs a write guard on the same shard to complete a delete,
    // but cannot acquire it because the read guard is held across the await.
    //
    // This test reproduces the issue by hammering create/delete/send/receive
    // concurrently on many queues.

    /// Concurrent queue lifecycle operations stress test.
    ///
    /// Multiple tasks simultaneously create, use (idempotent re-create which
    /// triggers `get_attributes` while holding a `DashMap` read guard), send,
    /// receive via long-poll, and delete queues.  Under the original code the
    /// `DashMap::Ref` returned by `get_queue()` is held across `.await` points
    /// which can deadlock the tokio runtime when shard locks contend.
    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_concurrent_queue_operations_do_not_deadlock() {
        let client = std::sync::Arc::new(sqs_client());
        let mut handles = Vec::new();

        // Spawn 50 concurrent tasks.  Each task creates a queue, then
        // re-creates it (idempotent path that calls get_attributes while
        // holding the DashMap guard), sends messages, receives with a short
        // long-poll, and deletes.  The mix of create+delete across tasks
        // maximises DashMap shard contention.
        for i in 0..50 {
            let c = client.clone();
            handles.push(tokio::spawn(async move {
                let name = format!(
                    "test-deadlock-{}-{}",
                    i,
                    &uuid::Uuid::new_v4().to_string()[..8]
                );

                // First create.
                let url = c
                    .create_queue()
                    .queue_name(&name)
                    .send()
                    .await
                    .unwrap()
                    .queue_url()
                    .unwrap()
                    .to_string();

                // Idempotent re-create — this takes the code path that calls
                // get_attributes while holding the DashMap read guard.
                let url2 = c
                    .create_queue()
                    .queue_name(&name)
                    .send()
                    .await
                    .unwrap()
                    .queue_url()
                    .unwrap()
                    .to_string();
                assert_eq!(url, url2);

                // Send messages.
                for j in 0..5 {
                    c.send_message()
                        .queue_url(&url)
                        .message_body(format!("msg-{i}-{j}"))
                        .send()
                        .await
                        .unwrap();
                }

                // Receive with a short long-poll so we also exercise the
                // long-poll path concurrently.
                let recv = c
                    .receive_message()
                    .queue_url(&url)
                    .max_number_of_messages(10)
                    .wait_time_seconds(1)
                    .send()
                    .await
                    .unwrap();

                assert!(
                    !recv.messages().is_empty(),
                    "queue {name} should have messages"
                );

                // Delete the queue — takes a write lock on the DashMap shard.
                c.delete_queue().queue_url(&url).send().await.unwrap();
            }));
        }

        // All tasks must complete within 30 seconds. A deadlock will cause
        // this to time out.
        let deadline = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            futures::future::join_all(handles),
        )
        .await
        .expect("deadlock: concurrent queue operations did not complete within 30s");

        for result in deadline {
            result.expect("task panicked");
        }
    }

    // ---------------------------------------------------------------------------
    // Bug: ChangeMessageVisibility doesn't wake long-poll waiters
    // ---------------------------------------------------------------------------
    //
    // When a consumer calls ChangeMessageVisibility with a short timeout (e.g.
    // 1 second) to retry a message quickly, the message becomes available after
    // the timeout expires but any pending ReceiveMessage long-poll is not woken
    // up. The periodic_cleanup (1s interval) moves expired in-flight messages
    // back to the available queue and calls notify_waiters(), but the
    // notification is lost because the notified() future in the actor's
    // tokio::select! loop is not being polled at that moment.

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_change_visibility_wakes_long_poll() {
        let client = sqs_client();
        let (_, url) = create_test_queue(&client, "vis-wake").await;

        // Send a message.
        client
            .send_message()
            .queue_url(&url)
            .message_body("retry me")
            .send()
            .await
            .unwrap();

        // Receive it with a long visibility timeout.
        let recv = client
            .receive_message()
            .queue_url(&url)
            .visibility_timeout(60)
            .wait_time_seconds(0)
            .send()
            .await
            .unwrap();
        assert_eq!(recv.messages().len(), 1);
        let receipt_handle = recv.messages()[0].receipt_handle().unwrap().to_owned();

        // Change visibility to 1 second — simulates a consumer requesting
        // a quick retry after a transient error.
        client
            .change_message_visibility()
            .queue_url(&url)
            .receipt_handle(&receipt_handle)
            .visibility_timeout(1)
            .send()
            .await
            .unwrap();

        // Start a long-poll that should pick up the message after the 1s
        // visibility timeout expires.
        let start = std::time::Instant::now();
        let recv2 = client
            .receive_message()
            .queue_url(&url)
            .max_number_of_messages(1)
            .wait_time_seconds(10)
            .send()
            .await
            .unwrap();

        let elapsed = start.elapsed();

        assert_eq!(
            recv2.messages().len(),
            1,
            "expected message to be redelivered after visibility timeout"
        );
        assert_eq!(recv2.messages()[0].body().unwrap(), "retry me");
        assert!(
            elapsed.as_secs() < 5,
            "long-poll took {elapsed:?} — should have returned within ~2s \
             (1s visibility + 1s cleanup interval), not waited for the full \
             WaitTimeSeconds"
        );

        delete_test_queue(&client, &url).await;
    }
}
