//! Integration tests for DynamoDB Streams service.
//!
//! Tests the full pipeline: DynamoDB writes -> stream records -> GetRecords.
//! Requires a running RustStack server at `localhost:4566`.

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use aws_sdk_dynamodb as dynamodb;
    use aws_sdk_dynamodb::types::{
        AttributeDefinition, AttributeValue, BillingMode, KeySchemaElement, KeyType, PutRequest,
        ScalarAttributeType, StreamSpecification, StreamViewType, WriteRequest,
    };
    use aws_sdk_dynamodbstreams as streams;

    use crate::{dynamodb_client, dynamodbstreams_client, test_table_name};

    /// Helper: create a table with streams enabled and return (table_name, stream_arn).
    async fn create_table_with_stream(
        ddb: &dynamodb::Client,
        view_type: StreamViewType,
    ) -> (String, String) {
        let name = test_table_name("streams");

        let result = ddb
            .create_table()
            .table_name(&name)
            .attribute_definitions(
                AttributeDefinition::builder()
                    .attribute_name("pk")
                    .attribute_type(ScalarAttributeType::S)
                    .build()
                    .unwrap(),
            )
            .key_schema(
                KeySchemaElement::builder()
                    .attribute_name("pk")
                    .key_type(KeyType::Hash)
                    .build()
                    .unwrap(),
            )
            .billing_mode(BillingMode::PayPerRequest)
            .stream_specification(
                StreamSpecification::builder()
                    .stream_enabled(true)
                    .stream_view_type(view_type)
                    .build()
                    .unwrap(),
            )
            .send()
            .await
            .expect("create table with stream should succeed");

        let desc = result.table_description().unwrap();
        let stream_arn = desc
            .latest_stream_arn()
            .expect("table should have a stream ARN")
            .to_string();

        (name, stream_arn)
    }

    /// Helper: get a TRIM_HORIZON shard iterator for the first shard.
    async fn get_trim_horizon_iterator(client: &streams::Client, stream_arn: &str) -> String {
        let desc = client
            .describe_stream()
            .stream_arn(stream_arn)
            .send()
            .await
            .expect("describe stream should succeed");

        let stream_desc = desc.stream_description().unwrap();
        let shard = &stream_desc.shards()[0];
        let shard_id = shard.shard_id().unwrap();

        let iter_resp = client
            .get_shard_iterator()
            .stream_arn(stream_arn)
            .shard_id(shard_id)
            .shard_iterator_type(streams::types::ShardIteratorType::TrimHorizon)
            .send()
            .await
            .expect("get shard iterator should succeed");

        iter_resp.shard_iterator().unwrap().to_string()
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_list_streams_for_table_with_stream() {
        let ddb = dynamodb_client();
        let streams_client = dynamodbstreams_client();

        let (table_name, _stream_arn) =
            create_table_with_stream(&ddb, StreamViewType::NewAndOldImages).await;

        let list = streams_client
            .list_streams()
            .table_name(&table_name)
            .send()
            .await
            .expect("list streams should succeed");

        assert!(
            !list.streams().is_empty(),
            "should have at least one stream"
        );
        assert_eq!(list.streams()[0].table_name().unwrap(), table_name,);

        // Cleanup
        ddb.delete_table().table_name(&table_name).send().await.ok();
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_describe_stream() {
        let ddb = dynamodb_client();
        let streams_client = dynamodbstreams_client();

        let (table_name, stream_arn) =
            create_table_with_stream(&ddb, StreamViewType::NewAndOldImages).await;

        let desc = streams_client
            .describe_stream()
            .stream_arn(&stream_arn)
            .send()
            .await
            .expect("describe stream should succeed");

        let stream_desc = desc.stream_description().unwrap();
        assert_eq!(stream_desc.stream_arn().unwrap(), stream_arn);
        assert_eq!(stream_desc.table_name().unwrap(), table_name);
        assert!(
            !stream_desc.shards().is_empty(),
            "should have at least one shard"
        );

        // Cleanup
        ddb.delete_table().table_name(&table_name).send().await.ok();
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_capture_put_item_as_insert_event() {
        let ddb = dynamodb_client();
        let streams_client = dynamodbstreams_client();

        let (table_name, stream_arn) =
            create_table_with_stream(&ddb, StreamViewType::NewAndOldImages).await;

        // Put an item
        ddb.put_item()
            .table_name(&table_name)
            .item("pk", AttributeValue::S("item1".to_string()))
            .item("data", AttributeValue::S("hello".to_string()))
            .send()
            .await
            .expect("put item should succeed");

        // Get records
        let iterator = get_trim_horizon_iterator(&streams_client, &stream_arn).await;

        let records_resp = streams_client
            .get_records()
            .shard_iterator(&iterator)
            .send()
            .await
            .expect("get records should succeed");

        let records = records_resp.records();
        assert_eq!(records.len(), 1, "should have exactly 1 record");

        let record = &records[0];
        assert_eq!(
            record.event_name(),
            Some(&streams::types::OperationType::Insert),
        );

        let dynamodb_record = record.dynamodb().unwrap();
        assert!(
            dynamodb_record.keys().is_some_and(|k| !k.is_empty()),
            "should have keys"
        );
        assert!(
            dynamodb_record.new_image().is_some_and(|i| !i.is_empty()),
            "should have new image for INSERT"
        );
        assert!(
            dynamodb_record.old_image().is_none_or(HashMap::is_empty),
            "should not have old image for INSERT"
        );

        // Cleanup
        ddb.delete_table().table_name(&table_name).send().await.ok();
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_capture_update_item_as_modify_event() {
        let ddb = dynamodb_client();
        let streams_client = dynamodbstreams_client();

        let (table_name, stream_arn) =
            create_table_with_stream(&ddb, StreamViewType::NewAndOldImages).await;

        // Create item
        ddb.put_item()
            .table_name(&table_name)
            .item("pk", AttributeValue::S("item1".to_string()))
            .item("data", AttributeValue::S("v1".to_string()))
            .send()
            .await
            .unwrap();

        // Update item
        ddb.update_item()
            .table_name(&table_name)
            .key("pk", AttributeValue::S("item1".to_string()))
            .update_expression("SET #d = :v")
            .expression_attribute_names("#d", "data")
            .expression_attribute_values(":v", AttributeValue::S("v2".to_string()))
            .send()
            .await
            .unwrap();

        // Get records
        let iterator = get_trim_horizon_iterator(&streams_client, &stream_arn).await;

        let records_resp = streams_client
            .get_records()
            .shard_iterator(&iterator)
            .send()
            .await
            .unwrap();

        let records = records_resp.records();
        assert!(
            records.len() >= 2,
            "should have at least 2 records (INSERT + MODIFY)"
        );

        // First record should be INSERT
        assert_eq!(
            records[0].event_name(),
            Some(&streams::types::OperationType::Insert),
        );

        // Second record should be MODIFY
        assert_eq!(
            records[1].event_name(),
            Some(&streams::types::OperationType::Modify),
        );

        // MODIFY should have both old and new images
        let modify_record = records[1].dynamodb().unwrap();
        assert!(
            modify_record.new_image().is_some_and(|i| !i.is_empty()),
            "MODIFY should have new image"
        );
        assert!(
            modify_record.old_image().is_some_and(|i| !i.is_empty()),
            "MODIFY should have old image"
        );

        // Cleanup
        ddb.delete_table().table_name(&table_name).send().await.ok();
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_capture_delete_item_as_remove_event() {
        let ddb = dynamodb_client();
        let streams_client = dynamodbstreams_client();

        let (table_name, stream_arn) =
            create_table_with_stream(&ddb, StreamViewType::NewAndOldImages).await;

        // Create then delete an item
        ddb.put_item()
            .table_name(&table_name)
            .item("pk", AttributeValue::S("item1".to_string()))
            .item("data", AttributeValue::S("hello".to_string()))
            .send()
            .await
            .unwrap();

        ddb.delete_item()
            .table_name(&table_name)
            .key("pk", AttributeValue::S("item1".to_string()))
            .send()
            .await
            .unwrap();

        // Get records
        let iterator = get_trim_horizon_iterator(&streams_client, &stream_arn).await;

        let records_resp = streams_client
            .get_records()
            .shard_iterator(&iterator)
            .send()
            .await
            .unwrap();

        let records = records_resp.records();
        assert!(records.len() >= 2, "should have at least 2 records");

        // Last record should be REMOVE
        let last = records.last().unwrap();
        assert_eq!(
            last.event_name(),
            Some(&streams::types::OperationType::Remove),
        );

        let remove_record = last.dynamodb().unwrap();
        assert!(
            remove_record.old_image().is_some_and(|i| !i.is_empty()),
            "REMOVE should have old image"
        );
        assert!(
            remove_record.new_image().is_none_or(HashMap::is_empty),
            "REMOVE should not have new image"
        );

        // Cleanup
        ddb.delete_table().table_name(&table_name).send().await.ok();
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_filter_images_by_stream_view_type_keys_only() {
        let ddb = dynamodb_client();
        let streams_client = dynamodbstreams_client();

        let (table_name, stream_arn) =
            create_table_with_stream(&ddb, StreamViewType::KeysOnly).await;

        ddb.put_item()
            .table_name(&table_name)
            .item("pk", AttributeValue::S("item1".to_string()))
            .item("data", AttributeValue::S("hello".to_string()))
            .send()
            .await
            .unwrap();

        let iterator = get_trim_horizon_iterator(&streams_client, &stream_arn).await;

        let records_resp = streams_client
            .get_records()
            .shard_iterator(&iterator)
            .send()
            .await
            .unwrap();

        let records = records_resp.records();
        assert_eq!(records.len(), 1);

        let dynamodb_record = records[0].dynamodb().unwrap();
        assert!(
            dynamodb_record.keys().is_some_and(|k| !k.is_empty()),
            "KEYS_ONLY should still have keys"
        );
        assert!(
            dynamodb_record.new_image().is_none_or(HashMap::is_empty),
            "KEYS_ONLY should not have new image"
        );
        assert!(
            dynamodb_record.old_image().is_none_or(HashMap::is_empty),
            "KEYS_ONLY should not have old image"
        );

        // Cleanup
        ddb.delete_table().table_name(&table_name).send().await.ok();
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_filter_images_by_stream_view_type_new_image() {
        let ddb = dynamodb_client();
        let streams_client = dynamodbstreams_client();

        let (table_name, stream_arn) =
            create_table_with_stream(&ddb, StreamViewType::NewImage).await;

        // Insert then update
        ddb.put_item()
            .table_name(&table_name)
            .item("pk", AttributeValue::S("item1".to_string()))
            .item("data", AttributeValue::S("v1".to_string()))
            .send()
            .await
            .unwrap();

        ddb.put_item()
            .table_name(&table_name)
            .item("pk", AttributeValue::S("item1".to_string()))
            .item("data", AttributeValue::S("v2".to_string()))
            .send()
            .await
            .unwrap();

        let iterator = get_trim_horizon_iterator(&streams_client, &stream_arn).await;

        let records_resp = streams_client
            .get_records()
            .shard_iterator(&iterator)
            .send()
            .await
            .unwrap();

        let records = records_resp.records();
        assert_eq!(records.len(), 2);

        // Both should have new image, neither should have old image
        for record in records {
            let dynamodb_record = record.dynamodb().unwrap();
            assert!(
                dynamodb_record.new_image().is_some_and(|i| !i.is_empty()),
                "NEW_IMAGE should have new image"
            );
            assert!(
                dynamodb_record.old_image().is_none_or(HashMap::is_empty),
                "NEW_IMAGE should not have old image"
            );
        }

        // Cleanup
        ddb.delete_table().table_name(&table_name).send().await.ok();
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_capture_batch_write_item_events() {
        let ddb = dynamodb_client();
        let streams_client = dynamodbstreams_client();

        let (table_name, stream_arn) =
            create_table_with_stream(&ddb, StreamViewType::NewAndOldImages).await;

        // BatchWriteItem with 3 puts
        let requests: Vec<WriteRequest> = (1..=3)
            .map(|i| {
                WriteRequest::builder()
                    .put_request(
                        PutRequest::builder()
                            .item("pk", AttributeValue::S(format!("batch-{i}")))
                            .item("data", AttributeValue::S(format!("value-{i}")))
                            .build()
                            .unwrap(),
                    )
                    .build()
            })
            .collect();

        ddb.batch_write_item()
            .request_items(&table_name, requests)
            .send()
            .await
            .unwrap();

        let iterator = get_trim_horizon_iterator(&streams_client, &stream_arn).await;

        let records_resp = streams_client
            .get_records()
            .shard_iterator(&iterator)
            .send()
            .await
            .unwrap();

        let records = records_resp.records();
        assert_eq!(records.len(), 3, "should have 3 records from batch write");

        for record in records {
            assert_eq!(
                record.event_name(),
                Some(&streams::types::OperationType::Insert),
            );
        }

        // Cleanup
        ddb.delete_table().table_name(&table_name).send().await.ok();
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_get_shard_iterator_latest_skips_existing_records() {
        let ddb = dynamodb_client();
        let streams_client = dynamodbstreams_client();

        let (table_name, stream_arn) =
            create_table_with_stream(&ddb, StreamViewType::NewAndOldImages).await;

        // Put record 1 before getting LATEST iterator
        ddb.put_item()
            .table_name(&table_name)
            .item("pk", AttributeValue::S("before-latest".to_string()))
            .send()
            .await
            .unwrap();

        // Get LATEST iterator (should skip record 1)
        let desc = streams_client
            .describe_stream()
            .stream_arn(&stream_arn)
            .send()
            .await
            .unwrap();

        let shard_id = desc.stream_description().unwrap().shards()[0]
            .shard_id()
            .unwrap();

        let iter_resp = streams_client
            .get_shard_iterator()
            .stream_arn(&stream_arn)
            .shard_id(shard_id)
            .shard_iterator_type(streams::types::ShardIteratorType::Latest)
            .send()
            .await
            .unwrap();

        let latest_iterator = iter_resp.shard_iterator().unwrap();

        // Put record 2 after getting LATEST iterator
        ddb.put_item()
            .table_name(&table_name)
            .item("pk", AttributeValue::S("after-latest".to_string()))
            .send()
            .await
            .unwrap();

        // Get records with LATEST iterator -> should only get record 2
        let records_resp = streams_client
            .get_records()
            .shard_iterator(latest_iterator)
            .send()
            .await
            .unwrap();

        let records = records_resp.records();
        assert_eq!(
            records.len(),
            1,
            "LATEST should only return records after iterator creation"
        );

        // Cleanup
        ddb.delete_table().table_name(&table_name).send().await.ok();
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_return_empty_records_when_no_new_data() {
        let ddb = dynamodb_client();
        let streams_client = dynamodbstreams_client();

        let (table_name, stream_arn) =
            create_table_with_stream(&ddb, StreamViewType::NewAndOldImages).await;

        ddb.put_item()
            .table_name(&table_name)
            .item("pk", AttributeValue::S("item1".to_string()))
            .send()
            .await
            .unwrap();

        let iterator = get_trim_horizon_iterator(&streams_client, &stream_arn).await;

        // First read: consume all records
        let resp1 = streams_client
            .get_records()
            .shard_iterator(&iterator)
            .send()
            .await
            .unwrap();

        assert_eq!(resp1.records().len(), 1);

        // Second read with next iterator: should be empty
        let next_iter = resp1.next_shard_iterator().unwrap();
        let resp2 = streams_client
            .get_records()
            .shard_iterator(next_iter)
            .send()
            .await
            .unwrap();

        assert!(
            resp2.records().is_empty(),
            "should be empty when no new data"
        );
        assert!(
            resp2.next_shard_iterator().is_some(),
            "should still provide next iterator for open shard"
        );

        // Cleanup
        ddb.delete_table().table_name(&table_name).send().await.ok();
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_describe_table_include_stream_arn() {
        let ddb = dynamodb_client();

        let (table_name, stream_arn) =
            create_table_with_stream(&ddb, StreamViewType::NewAndOldImages).await;

        let desc = ddb
            .describe_table()
            .table_name(&table_name)
            .send()
            .await
            .unwrap();

        let table_desc = desc.table().unwrap();
        assert_eq!(table_desc.latest_stream_arn().unwrap(), stream_arn,);
        assert!(
            table_desc.latest_stream_label().is_some(),
            "should have stream label"
        );

        // Cleanup
        ddb.delete_table().table_name(&table_name).send().await.ok();
    }
}
