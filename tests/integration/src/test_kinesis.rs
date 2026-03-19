//! Integration tests for the Kinesis service.
//!
//! Tests cover stream lifecycle, record operations, shard iterators,
//! tags, retention, shard splitting/merging, consumers, encryption,
//! and resource policies.

#[cfg(test)]
mod tests {
    use aws_sdk_kinesis::types::{
        EncryptionType, ShardIteratorType, StreamMode, StreamModeDetails, StreamStatus,
    };

    use crate::{kinesis_client, test_stream_name};

    // -----------------------------------------------------------------------
    // Stream Management
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_and_delete_stream() {
        let client = kinesis_client();
        let name = test_stream_name("create");

        client
            .create_stream()
            .stream_name(&name)
            .shard_count(2)
            .send()
            .await
            .unwrap();

        let desc = client
            .describe_stream()
            .stream_name(&name)
            .send()
            .await
            .unwrap();

        let sd = desc.stream_description().unwrap();
        assert_eq!(sd.stream_name(), &name);
        assert_eq!(sd.stream_status(), &StreamStatus::Active);
        assert_eq!(sd.shards().len(), 2);
        assert!(!sd.has_more_shards());

        client
            .delete_stream()
            .stream_name(&name)
            .send()
            .await
            .unwrap();

        // Verify deleted.
        let err = client.describe_stream().stream_name(&name).send().await;
        assert!(err.is_err());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_on_demand_stream() {
        let client = kinesis_client();
        let name = test_stream_name("ondemand");

        client
            .create_stream()
            .stream_name(&name)
            .stream_mode_details(
                StreamModeDetails::builder()
                    .stream_mode(StreamMode::OnDemand)
                    .build()
                    .unwrap(),
            )
            .send()
            .await
            .unwrap();

        let desc = client
            .describe_stream_summary()
            .stream_name(&name)
            .send()
            .await
            .unwrap();

        let sd = desc.stream_description_summary().unwrap();
        assert_eq!(sd.stream_name(), &name);
        assert!(sd.open_shard_count() > 0);

        client
            .delete_stream()
            .stream_name(&name)
            .send()
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_list_streams() {
        let client = kinesis_client();
        let name1 = test_stream_name("list1");
        let name2 = test_stream_name("list2");

        client
            .create_stream()
            .stream_name(&name1)
            .shard_count(1)
            .send()
            .await
            .unwrap();
        client
            .create_stream()
            .stream_name(&name2)
            .shard_count(1)
            .send()
            .await
            .unwrap();

        let resp = client.list_streams().send().await.unwrap();
        let stream_names = resp.stream_names();
        assert!(stream_names.contains(&name1));
        assert!(stream_names.contains(&name2));

        client
            .delete_stream()
            .stream_name(&name1)
            .send()
            .await
            .unwrap();
        client
            .delete_stream()
            .stream_name(&name2)
            .send()
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_reject_duplicate_stream() {
        let client = kinesis_client();
        let name = test_stream_name("dup");

        client
            .create_stream()
            .stream_name(&name)
            .shard_count(1)
            .send()
            .await
            .unwrap();

        let err = client
            .create_stream()
            .stream_name(&name)
            .shard_count(1)
            .send()
            .await;
        assert!(err.is_err());

        client
            .delete_stream()
            .stream_name(&name)
            .send()
            .await
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // Record Operations
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_put_and_get_record() {
        let client = kinesis_client();
        let name = test_stream_name("putget");

        client
            .create_stream()
            .stream_name(&name)
            .shard_count(1)
            .send()
            .await
            .unwrap();

        let put_resp = client
            .put_record()
            .stream_name(&name)
            .data(aws_sdk_kinesis::primitives::Blob::new("hello world"))
            .partition_key("pk1")
            .send()
            .await
            .unwrap();

        assert!(!put_resp.shard_id().is_empty());
        assert!(!put_resp.sequence_number().is_empty());

        // Get shard iterator.
        let iter_resp = client
            .get_shard_iterator()
            .stream_name(&name)
            .shard_id("shardId-000000000000")
            .shard_iterator_type(ShardIteratorType::TrimHorizon)
            .send()
            .await
            .unwrap();

        let shard_iter = iter_resp.shard_iterator().unwrap();

        // Get records.
        let get_resp = client
            .get_records()
            .shard_iterator(shard_iter)
            .send()
            .await
            .unwrap();

        let records = get_resp.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].data().as_ref(), b"hello world");
        assert_eq!(records[0].partition_key(), "pk1");
        assert!(!records[0].sequence_number().is_empty());

        // Next iterator should return empty.
        let next_iter = get_resp.next_shard_iterator().unwrap();
        let get_resp2 = client
            .get_records()
            .shard_iterator(next_iter)
            .send()
            .await
            .unwrap();
        assert!(get_resp2.records().is_empty());

        client
            .delete_stream()
            .stream_name(&name)
            .send()
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_put_records_batch() {
        let client = kinesis_client();
        let name = test_stream_name("batch");

        client
            .create_stream()
            .stream_name(&name)
            .shard_count(2)
            .send()
            .await
            .unwrap();

        let entries: Vec<_> = (0..10)
            .map(|i| {
                aws_sdk_kinesis::types::PutRecordsRequestEntry::builder()
                    .data(aws_sdk_kinesis::primitives::Blob::new(format!(
                        "record-{i}"
                    )))
                    .partition_key(format!("pk-{i}"))
                    .build()
                    .unwrap()
            })
            .collect();

        let resp = client
            .put_records()
            .stream_name(&name)
            .set_records(Some(entries))
            .send()
            .await
            .unwrap();

        assert_eq!(resp.failed_record_count().unwrap_or(0), 0);
        assert_eq!(resp.records().len(), 10);

        for record in resp.records() {
            assert!(record.shard_id().is_some());
            assert!(record.sequence_number().is_some());
        }

        client
            .delete_stream()
            .stream_name(&name)
            .send()
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_partition_key_routing() {
        let client = kinesis_client();
        let name = test_stream_name("routing");

        client
            .create_stream()
            .stream_name(&name)
            .shard_count(4)
            .send()
            .await
            .unwrap();

        // Same partition key should always go to same shard.
        let r1 = client
            .put_record()
            .stream_name(&name)
            .data(aws_sdk_kinesis::primitives::Blob::new("a"))
            .partition_key("consistent-key")
            .send()
            .await
            .unwrap();

        let r2 = client
            .put_record()
            .stream_name(&name)
            .data(aws_sdk_kinesis::primitives::Blob::new("b"))
            .partition_key("consistent-key")
            .send()
            .await
            .unwrap();

        assert_eq!(r1.shard_id(), r2.shard_id());

        client
            .delete_stream()
            .stream_name(&name)
            .send()
            .await
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // Shard Iterator Types
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_support_all_iterator_types() {
        let client = kinesis_client();
        let name = test_stream_name("itertypes");

        client
            .create_stream()
            .stream_name(&name)
            .shard_count(1)
            .send()
            .await
            .unwrap();

        // Put a record to get a sequence number.
        let put = client
            .put_record()
            .stream_name(&name)
            .data(aws_sdk_kinesis::primitives::Blob::new("test"))
            .partition_key("pk")
            .send()
            .await
            .unwrap();

        let seq_num = put.sequence_number().to_string();
        let shard_id = "shardId-000000000000";

        // TRIM_HORIZON
        let resp = client
            .get_shard_iterator()
            .stream_name(&name)
            .shard_id(shard_id)
            .shard_iterator_type(ShardIteratorType::TrimHorizon)
            .send()
            .await
            .unwrap();
        assert!(resp.shard_iterator().is_some());

        // LATEST
        let resp = client
            .get_shard_iterator()
            .stream_name(&name)
            .shard_id(shard_id)
            .shard_iterator_type(ShardIteratorType::Latest)
            .send()
            .await
            .unwrap();
        assert!(resp.shard_iterator().is_some());

        // AT_SEQUENCE_NUMBER
        let resp = client
            .get_shard_iterator()
            .stream_name(&name)
            .shard_id(shard_id)
            .shard_iterator_type(ShardIteratorType::AtSequenceNumber)
            .starting_sequence_number(&seq_num)
            .send()
            .await
            .unwrap();
        assert!(resp.shard_iterator().is_some());

        // AFTER_SEQUENCE_NUMBER
        let resp = client
            .get_shard_iterator()
            .stream_name(&name)
            .shard_id(shard_id)
            .shard_iterator_type(ShardIteratorType::AfterSequenceNumber)
            .starting_sequence_number(&seq_num)
            .send()
            .await
            .unwrap();
        assert!(resp.shard_iterator().is_some());

        // AT_TIMESTAMP
        let resp = client
            .get_shard_iterator()
            .stream_name(&name)
            .shard_id(shard_id)
            .shard_iterator_type(ShardIteratorType::AtTimestamp)
            .timestamp(aws_sdk_kinesis::primitives::DateTime::from_secs(0))
            .send()
            .await
            .unwrap();
        assert!(resp.shard_iterator().is_some());

        client
            .delete_stream()
            .stream_name(&name)
            .send()
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_get_records_with_latest_then_new() {
        let client = kinesis_client();
        let name = test_stream_name("latest");

        client
            .create_stream()
            .stream_name(&name)
            .shard_count(1)
            .send()
            .await
            .unwrap();

        // Put a record before getting LATEST iterator.
        client
            .put_record()
            .stream_name(&name)
            .data(aws_sdk_kinesis::primitives::Blob::new("before"))
            .partition_key("pk")
            .send()
            .await
            .unwrap();

        // LATEST should not return old records.
        let iter_resp = client
            .get_shard_iterator()
            .stream_name(&name)
            .shard_id("shardId-000000000000")
            .shard_iterator_type(ShardIteratorType::Latest)
            .send()
            .await
            .unwrap();

        let shard_iter = iter_resp.shard_iterator().unwrap();
        let get_resp = client
            .get_records()
            .shard_iterator(shard_iter)
            .send()
            .await
            .unwrap();
        assert!(get_resp.records().is_empty());

        // Put new record after LATEST.
        client
            .put_record()
            .stream_name(&name)
            .data(aws_sdk_kinesis::primitives::Blob::new("after"))
            .partition_key("pk")
            .send()
            .await
            .unwrap();

        // Should get the new record.
        let next_iter = get_resp.next_shard_iterator().unwrap();
        let get_resp2 = client
            .get_records()
            .shard_iterator(next_iter)
            .send()
            .await
            .unwrap();
        assert_eq!(get_resp2.records().len(), 1);
        assert_eq!(get_resp2.records()[0].data().as_ref(), b"after");

        client
            .delete_stream()
            .stream_name(&name)
            .send()
            .await
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // List Shards
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_list_shards() {
        let client = kinesis_client();
        let name = test_stream_name("listshards");

        client
            .create_stream()
            .stream_name(&name)
            .shard_count(4)
            .send()
            .await
            .unwrap();

        let resp = client
            .list_shards()
            .stream_name(&name)
            .send()
            .await
            .unwrap();

        let shards = resp.shards();
        assert_eq!(shards.len(), 4);

        for (i, shard) in shards.iter().enumerate() {
            assert_eq!(shard.shard_id(), format!("shardId-{i:012}"));
            let hkr = shard.hash_key_range().unwrap();
            assert!(!hkr.starting_hash_key().is_empty());
            assert!(!hkr.ending_hash_key().is_empty());
        }

        client
            .delete_stream()
            .stream_name(&name)
            .send()
            .await
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // Tags
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_manage_tags() {
        let client = kinesis_client();
        let name = test_stream_name("tags");

        client
            .create_stream()
            .stream_name(&name)
            .shard_count(1)
            .send()
            .await
            .unwrap();

        // Add tags.
        client
            .add_tags_to_stream()
            .stream_name(&name)
            .tags("env", "test")
            .tags("project", "ruststack")
            .send()
            .await
            .unwrap();

        // List tags.
        let resp = client
            .list_tags_for_stream()
            .stream_name(&name)
            .send()
            .await
            .unwrap();
        let tags = resp.tags();
        assert_eq!(tags.len(), 2);
        assert!(!resp.has_more_tags());

        // Remove tags.
        client
            .remove_tags_from_stream()
            .stream_name(&name)
            .tag_keys("env")
            .send()
            .await
            .unwrap();

        let resp = client
            .list_tags_for_stream()
            .stream_name(&name)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.tags().len(), 1);
        assert_eq!(resp.tags()[0].key(), "project");

        client
            .delete_stream()
            .stream_name(&name)
            .send()
            .await
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // Retention
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_modify_retention_period() {
        let client = kinesis_client();
        let name = test_stream_name("retention");

        client
            .create_stream()
            .stream_name(&name)
            .shard_count(1)
            .send()
            .await
            .unwrap();

        // Increase retention.
        client
            .increase_stream_retention_period()
            .stream_name(&name)
            .retention_period_hours(48)
            .send()
            .await
            .unwrap();

        let desc = client
            .describe_stream()
            .stream_name(&name)
            .send()
            .await
            .unwrap();
        assert_eq!(
            desc.stream_description().unwrap().retention_period_hours(),
            48
        );

        // Decrease retention.
        client
            .decrease_stream_retention_period()
            .stream_name(&name)
            .retention_period_hours(24)
            .send()
            .await
            .unwrap();

        let desc = client
            .describe_stream()
            .stream_name(&name)
            .send()
            .await
            .unwrap();
        assert_eq!(
            desc.stream_description().unwrap().retention_period_hours(),
            24
        );

        client
            .delete_stream()
            .stream_name(&name)
            .send()
            .await
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // Shard Split and Merge
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_split_shard() {
        let client = kinesis_client();
        let name = test_stream_name("split");

        client
            .create_stream()
            .stream_name(&name)
            .shard_count(1)
            .send()
            .await
            .unwrap();

        // Get the shard's hash key range midpoint.
        let desc = client
            .describe_stream()
            .stream_name(&name)
            .send()
            .await
            .unwrap();
        let shard = &desc.stream_description().unwrap().shards()[0];
        let hkr = shard.hash_key_range().unwrap();
        let start: u128 = hkr.starting_hash_key().parse().unwrap();
        let end: u128 = hkr.ending_hash_key().parse().unwrap();
        let mid = start + (end - start) / 2;

        client
            .split_shard()
            .stream_name(&name)
            .shard_to_split("shardId-000000000000")
            .new_starting_hash_key(mid.to_string())
            .send()
            .await
            .unwrap();

        // After split, should have 3 shards (1 closed parent + 2 children).
        let shards_resp = client
            .list_shards()
            .stream_name(&name)
            .send()
            .await
            .unwrap();
        assert_eq!(shards_resp.shards().len(), 3);

        client
            .delete_stream()
            .stream_name(&name)
            .send()
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_merge_shards() {
        let client = kinesis_client();
        let name = test_stream_name("merge");

        client
            .create_stream()
            .stream_name(&name)
            .shard_count(2)
            .send()
            .await
            .unwrap();

        client
            .merge_shards()
            .stream_name(&name)
            .shard_to_merge("shardId-000000000000")
            .adjacent_shard_to_merge("shardId-000000000001")
            .send()
            .await
            .unwrap();

        // After merge, should have 3 shards (2 closed parents + 1 child).
        let shards_resp = client
            .list_shards()
            .stream_name(&name)
            .send()
            .await
            .unwrap();
        assert_eq!(shards_resp.shards().len(), 3);

        client
            .delete_stream()
            .stream_name(&name)
            .send()
            .await
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // Consumers
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_manage_consumers() {
        let client = kinesis_client();
        let name = test_stream_name("consumers");

        client
            .create_stream()
            .stream_name(&name)
            .shard_count(1)
            .send()
            .await
            .unwrap();

        let desc = client
            .describe_stream()
            .stream_name(&name)
            .send()
            .await
            .unwrap();
        let stream_arn = desc.stream_description().unwrap().stream_arn().to_string();

        // Register consumer.
        let reg_resp = client
            .register_stream_consumer()
            .stream_arn(&stream_arn)
            .consumer_name("my-consumer")
            .send()
            .await
            .unwrap();
        let consumer = reg_resp.consumer().unwrap();
        assert_eq!(consumer.consumer_name(), "my-consumer");

        // List consumers.
        let list_resp = client
            .list_stream_consumers()
            .stream_arn(&stream_arn)
            .send()
            .await
            .unwrap();
        assert_eq!(list_resp.consumers().len(), 1);

        // Deregister.
        let consumer_arn = consumer.consumer_arn().to_string();
        client
            .deregister_stream_consumer()
            .consumer_arn(&consumer_arn)
            .stream_arn(&stream_arn)
            .send()
            .await
            .unwrap();

        let list_resp = client
            .list_stream_consumers()
            .stream_arn(&stream_arn)
            .send()
            .await
            .unwrap();
        assert!(list_resp.consumers().is_empty());

        client
            .delete_stream()
            .stream_name(&name)
            .send()
            .await
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // Encryption
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_manage_encryption() {
        let client = kinesis_client();
        let name = test_stream_name("encrypt");

        client
            .create_stream()
            .stream_name(&name)
            .shard_count(1)
            .send()
            .await
            .unwrap();

        // Start encryption.
        client
            .start_stream_encryption()
            .stream_name(&name)
            .encryption_type(EncryptionType::Kms)
            .key_id("alias/aws/kinesis")
            .send()
            .await
            .unwrap();

        let desc = client
            .describe_stream()
            .stream_name(&name)
            .send()
            .await
            .unwrap();
        let sd = desc.stream_description().unwrap();
        assert_eq!(sd.encryption_type(), Some(&EncryptionType::Kms));

        // Stop encryption.
        client
            .stop_stream_encryption()
            .stream_name(&name)
            .encryption_type(EncryptionType::Kms)
            .key_id("alias/aws/kinesis")
            .send()
            .await
            .unwrap();

        client
            .delete_stream()
            .stream_name(&name)
            .send()
            .await
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // Describe Limits
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_describe_limits() {
        let client = kinesis_client();

        let resp = client.describe_limits().send().await.unwrap();
        assert!(resp.shard_limit() > 0);
    }

    // -----------------------------------------------------------------------
    // Error Handling
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_return_not_found_for_nonexistent_stream() {
        let client = kinesis_client();

        let err = client
            .describe_stream()
            .stream_name("nonexistent-stream-xyz")
            .send()
            .await;
        assert!(err.is_err());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_return_error_for_empty_records() {
        let client = kinesis_client();
        let name = test_stream_name("empty");

        client
            .create_stream()
            .stream_name(&name)
            .shard_count(1)
            .send()
            .await
            .unwrap();

        // Get records from empty stream should return empty, not error.
        let iter_resp = client
            .get_shard_iterator()
            .stream_name(&name)
            .shard_id("shardId-000000000000")
            .shard_iterator_type(ShardIteratorType::TrimHorizon)
            .send()
            .await
            .unwrap();

        let get_resp = client
            .get_records()
            .shard_iterator(iter_resp.shard_iterator().unwrap())
            .send()
            .await
            .unwrap();
        assert!(get_resp.records().is_empty());
        assert!(get_resp.next_shard_iterator().is_some());

        client
            .delete_stream()
            .stream_name(&name)
            .send()
            .await
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // Multiple Records and Ordering
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_maintain_record_order_within_shard() {
        let client = kinesis_client();
        let name = test_stream_name("order");

        client
            .create_stream()
            .stream_name(&name)
            .shard_count(1)
            .send()
            .await
            .unwrap();

        // Put 5 records with the same partition key.
        for i in 0..5 {
            client
                .put_record()
                .stream_name(&name)
                .data(aws_sdk_kinesis::primitives::Blob::new(format!("msg-{i}")))
                .partition_key("same-key")
                .send()
                .await
                .unwrap();
        }

        let iter_resp = client
            .get_shard_iterator()
            .stream_name(&name)
            .shard_id("shardId-000000000000")
            .shard_iterator_type(ShardIteratorType::TrimHorizon)
            .send()
            .await
            .unwrap();

        let get_resp = client
            .get_records()
            .shard_iterator(iter_resp.shard_iterator().unwrap())
            .send()
            .await
            .unwrap();

        let records = get_resp.records();
        assert_eq!(records.len(), 5);

        // Verify ordering by sequence number.
        for i in 1..records.len() {
            assert!(records[i].sequence_number() > records[i - 1].sequence_number());
        }

        // Verify data integrity.
        for (i, record) in records.iter().enumerate() {
            assert_eq!(record.data().as_ref(), format!("msg-{i}").as_bytes());
        }

        client
            .delete_stream()
            .stream_name(&name)
            .send()
            .await
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // Describe Stream Summary
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_describe_stream_summary() {
        let client = kinesis_client();
        let name = test_stream_name("summary");

        client
            .create_stream()
            .stream_name(&name)
            .shard_count(3)
            .send()
            .await
            .unwrap();

        let resp = client
            .describe_stream_summary()
            .stream_name(&name)
            .send()
            .await
            .unwrap();

        let summary = resp.stream_description_summary().unwrap();
        assert_eq!(summary.stream_name(), &name);
        assert_eq!(summary.open_shard_count(), 3);
        assert_eq!(summary.retention_period_hours(), 24);
        assert_eq!(summary.stream_status(), &StreamStatus::Active);

        client
            .delete_stream()
            .stream_name(&name)
            .send()
            .await
            .unwrap();
    }
}
