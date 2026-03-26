//! DynamoDB integration tests against a running RustStack server.

#[cfg(test)]
mod tests {
    use aws_sdk_dynamodb::types::{
        AttributeDefinition, AttributeValue, KeySchemaElement, KeyType, ScalarAttributeType,
    };

    use crate::{dynamodb_client, test_table_name};

    /// Helper: create a simple table with a hash key.
    async fn create_simple_table(client: &aws_sdk_dynamodb::Client, table_name: &str) {
        client
            .create_table()
            .table_name(table_name)
            .key_schema(
                KeySchemaElement::builder()
                    .attribute_name("pk")
                    .key_type(KeyType::Hash)
                    .build()
                    .unwrap(),
            )
            .attribute_definitions(
                AttributeDefinition::builder()
                    .attribute_name("pk")
                    .attribute_type(ScalarAttributeType::S)
                    .build()
                    .unwrap(),
            )
            .billing_mode(aws_sdk_dynamodb::types::BillingMode::PayPerRequest)
            .send()
            .await
            .unwrap_or_else(|e| panic!("failed to create table {table_name}: {e}"));
    }

    /// Helper: create a composite-key table with partition + sort key.
    async fn create_composite_table(client: &aws_sdk_dynamodb::Client, table_name: &str) {
        client
            .create_table()
            .table_name(table_name)
            .key_schema(
                KeySchemaElement::builder()
                    .attribute_name("pk")
                    .key_type(KeyType::Hash)
                    .build()
                    .unwrap(),
            )
            .key_schema(
                KeySchemaElement::builder()
                    .attribute_name("sk")
                    .key_type(KeyType::Range)
                    .build()
                    .unwrap(),
            )
            .attribute_definitions(
                AttributeDefinition::builder()
                    .attribute_name("pk")
                    .attribute_type(ScalarAttributeType::S)
                    .build()
                    .unwrap(),
            )
            .attribute_definitions(
                AttributeDefinition::builder()
                    .attribute_name("sk")
                    .attribute_type(ScalarAttributeType::S)
                    .build()
                    .unwrap(),
            )
            .billing_mode(aws_sdk_dynamodb::types::BillingMode::PayPerRequest)
            .send()
            .await
            .unwrap_or_else(|e| panic!("failed to create table {table_name}: {e}"));
    }

    // -----------------------------------------------------------------------
    // Table Operations
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_and_describe_table() {
        let client = dynamodb_client();
        let table_name = test_table_name("create");

        create_simple_table(&client, &table_name).await;

        let resp = client
            .describe_table()
            .table_name(&table_name)
            .send()
            .await
            .unwrap();

        let desc = resp.table().unwrap();
        assert_eq!(desc.table_name(), Some(table_name.as_str()));
        assert_eq!(desc.key_schema().len(), 1);
        assert_eq!(desc.key_schema()[0].attribute_name(), "pk");

        // Cleanup.
        client
            .delete_table()
            .table_name(&table_name)
            .send()
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_list_tables() {
        let client = dynamodb_client();
        let table_name = test_table_name("list");

        create_simple_table(&client, &table_name).await;

        let resp = client.list_tables().send().await.unwrap();
        let names = resp.table_names();
        assert!(names.contains(&table_name));

        // Cleanup.
        client
            .delete_table()
            .table_name(&table_name)
            .send()
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_delete_table() {
        let client = dynamodb_client();
        let table_name = test_table_name("deltbl");

        create_simple_table(&client, &table_name).await;

        client
            .delete_table()
            .table_name(&table_name)
            .send()
            .await
            .unwrap();

        let err = client.describe_table().table_name(&table_name).send().await;
        assert!(err.is_err());
    }

    // -----------------------------------------------------------------------
    // Item CRUD
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_put_and_get_item() {
        let client = dynamodb_client();
        let table_name = test_table_name("putget");

        create_simple_table(&client, &table_name).await;

        // Put item.
        client
            .put_item()
            .table_name(&table_name)
            .item("pk", AttributeValue::S("user1".to_owned()))
            .item("name", AttributeValue::S("Alice".to_owned()))
            .item("age", AttributeValue::N("30".to_owned()))
            .send()
            .await
            .unwrap();

        // Get item.
        let resp = client
            .get_item()
            .table_name(&table_name)
            .key("pk", AttributeValue::S("user1".to_owned()))
            .send()
            .await
            .unwrap();

        let item = resp.item().unwrap();
        assert_eq!(item.get("name").unwrap().as_s().unwrap(), "Alice");
        assert_eq!(item.get("age").unwrap().as_n().unwrap(), "30");

        // Cleanup.
        client
            .delete_table()
            .table_name(&table_name)
            .send()
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_delete_item() {
        let client = dynamodb_client();
        let table_name = test_table_name("delitem");

        create_simple_table(&client, &table_name).await;

        client
            .put_item()
            .table_name(&table_name)
            .item("pk", AttributeValue::S("user1".to_owned()))
            .item("data", AttributeValue::S("value".to_owned()))
            .send()
            .await
            .unwrap();

        client
            .delete_item()
            .table_name(&table_name)
            .key("pk", AttributeValue::S("user1".to_owned()))
            .send()
            .await
            .unwrap();

        let resp = client
            .get_item()
            .table_name(&table_name)
            .key("pk", AttributeValue::S("user1".to_owned()))
            .send()
            .await
            .unwrap();

        assert!(resp.item().is_none());

        // Cleanup.
        client
            .delete_table()
            .table_name(&table_name)
            .send()
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_update_item_with_set_expression() {
        let client = dynamodb_client();
        let table_name = test_table_name("update");

        create_simple_table(&client, &table_name).await;

        // Create initial item.
        client
            .put_item()
            .table_name(&table_name)
            .item("pk", AttributeValue::S("user1".to_owned()))
            .item("name", AttributeValue::S("Alice".to_owned()))
            .item("age", AttributeValue::N("30".to_owned()))
            .send()
            .await
            .unwrap();

        // Update with SET expression.
        client
            .update_item()
            .table_name(&table_name)
            .key("pk", AttributeValue::S("user1".to_owned()))
            .update_expression("SET #n = :newname, age = :newage")
            .expression_attribute_names("#n", "name")
            .expression_attribute_values(":newname", AttributeValue::S("Bob".to_owned()))
            .expression_attribute_values(":newage", AttributeValue::N("31".to_owned()))
            .send()
            .await
            .unwrap();

        // Verify update.
        let resp = client
            .get_item()
            .table_name(&table_name)
            .key("pk", AttributeValue::S("user1".to_owned()))
            .send()
            .await
            .unwrap();

        let item = resp.item().unwrap();
        assert_eq!(item.get("name").unwrap().as_s().unwrap(), "Bob");
        assert_eq!(item.get("age").unwrap().as_n().unwrap(), "31");

        // Cleanup.
        client
            .delete_table()
            .table_name(&table_name)
            .send()
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_put_item_with_condition_expression() {
        let client = dynamodb_client();
        let table_name = test_table_name("condput");

        create_simple_table(&client, &table_name).await;

        // First put should succeed (attribute_not_exists on new item).
        client
            .put_item()
            .table_name(&table_name)
            .item("pk", AttributeValue::S("user1".to_owned()))
            .item("data", AttributeValue::S("first".to_owned()))
            .condition_expression("attribute_not_exists(pk)")
            .send()
            .await
            .unwrap();

        // Second put with same condition should fail.
        let err = client
            .put_item()
            .table_name(&table_name)
            .item("pk", AttributeValue::S("user1".to_owned()))
            .item("data", AttributeValue::S("second".to_owned()))
            .condition_expression("attribute_not_exists(pk)")
            .send()
            .await;

        assert!(err.is_err());

        // Verify original value was preserved.
        let resp = client
            .get_item()
            .table_name(&table_name)
            .key("pk", AttributeValue::S("user1".to_owned()))
            .send()
            .await
            .unwrap();

        assert_eq!(
            resp.item().unwrap().get("data").unwrap().as_s().unwrap(),
            "first"
        );

        // Cleanup.
        client
            .delete_table()
            .table_name(&table_name)
            .send()
            .await
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // Query & Scan
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_query_by_partition_and_sort_key() {
        let client = dynamodb_client();
        let table_name = test_table_name("query");

        create_composite_table(&client, &table_name).await;

        // Insert multiple items.
        for i in 1..=5 {
            client
                .put_item()
                .table_name(&table_name)
                .item("pk", AttributeValue::S("partition1".to_owned()))
                .item("sk", AttributeValue::S(format!("sort{i:03}")))
                .item("val", AttributeValue::N(i.to_string()))
                .send()
                .await
                .unwrap();
        }

        // Query with key condition on partition + sort key range.
        let resp = client
            .query()
            .table_name(&table_name)
            .key_condition_expression("pk = :pk AND sk BETWEEN :lo AND :hi")
            .expression_attribute_values(":pk", AttributeValue::S("partition1".to_owned()))
            .expression_attribute_values(":lo", AttributeValue::S("sort002".to_owned()))
            .expression_attribute_values(":hi", AttributeValue::S("sort004".to_owned()))
            .send()
            .await
            .unwrap();

        assert_eq!(resp.count(), 3);
        let items = resp.items();
        assert_eq!(items[0].get("sk").unwrap().as_s().unwrap(), "sort002");
        assert_eq!(items[2].get("sk").unwrap().as_s().unwrap(), "sort004");

        // Cleanup.
        client
            .delete_table()
            .table_name(&table_name)
            .send()
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_query_with_filter_expression() {
        let client = dynamodb_client();
        let table_name = test_table_name("qfilter");

        create_composite_table(&client, &table_name).await;

        for i in 1..=5 {
            client
                .put_item()
                .table_name(&table_name)
                .item("pk", AttributeValue::S("p1".to_owned()))
                .item("sk", AttributeValue::S(format!("s{i}")))
                .item("val", AttributeValue::N(i.to_string()))
                .send()
                .await
                .unwrap();
        }

        // Query with filter: val > 3.
        let resp = client
            .query()
            .table_name(&table_name)
            .key_condition_expression("pk = :pk")
            .filter_expression("val > :threshold")
            .expression_attribute_values(":pk", AttributeValue::S("p1".to_owned()))
            .expression_attribute_values(":threshold", AttributeValue::N("3".to_owned()))
            .send()
            .await
            .unwrap();

        // Items 4 and 5 should pass the filter.
        assert_eq!(resp.count(), 2);

        // Cleanup.
        client
            .delete_table()
            .table_name(&table_name)
            .send()
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_scan_all_items() {
        let client = dynamodb_client();
        let table_name = test_table_name("scan");

        create_simple_table(&client, &table_name).await;

        for i in 1..=10 {
            client
                .put_item()
                .table_name(&table_name)
                .item("pk", AttributeValue::S(format!("item{i}")))
                .item("data", AttributeValue::S(format!("value{i}")))
                .send()
                .await
                .unwrap();
        }

        let resp = client.scan().table_name(&table_name).send().await.unwrap();

        assert_eq!(resp.count(), 10);

        // Cleanup.
        client
            .delete_table()
            .table_name(&table_name)
            .send()
            .await
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // Batch Operations
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_batch_write_and_get_items() {
        use aws_sdk_dynamodb::types::{KeysAndAttributes, PutRequest, WriteRequest};

        let client = dynamodb_client();
        let table_name = test_table_name("batch");

        create_simple_table(&client, &table_name).await;

        // Batch write 5 items.
        let write_requests: Vec<WriteRequest> = (1..=5)
            .map(|i| {
                WriteRequest::builder()
                    .put_request(
                        PutRequest::builder()
                            .item("pk", AttributeValue::S(format!("batch{i}")))
                            .item("data", AttributeValue::S(format!("value{i}")))
                            .build()
                            .unwrap(),
                    )
                    .build()
            })
            .collect();

        client
            .batch_write_item()
            .request_items(&table_name, write_requests)
            .send()
            .await
            .unwrap();

        // Batch get items.
        let keys: Vec<_> = (1..=5)
            .map(|i| {
                let mut key = std::collections::HashMap::new();
                key.insert("pk".to_owned(), AttributeValue::S(format!("batch{i}")));
                key
            })
            .collect();

        let resp = client
            .batch_get_item()
            .request_items(
                &table_name,
                KeysAndAttributes::builder()
                    .set_keys(Some(keys))
                    .build()
                    .unwrap(),
            )
            .send()
            .await
            .unwrap();

        let responses = resp.responses().unwrap();
        let items = responses.get(table_name.as_str()).unwrap();
        assert_eq!(items.len(), 5);

        // Cleanup.
        client
            .delete_table()
            .table_name(&table_name)
            .send()
            .await
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // Projection Expression
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_project_specific_attributes() {
        let client = dynamodb_client();
        let table_name = test_table_name("proj");

        create_simple_table(&client, &table_name).await;

        client
            .put_item()
            .table_name(&table_name)
            .item("pk", AttributeValue::S("user1".to_owned()))
            .item("name", AttributeValue::S("Alice".to_owned()))
            .item("age", AttributeValue::N("30".to_owned()))
            .item("email", AttributeValue::S("alice@example.com".to_owned()))
            .send()
            .await
            .unwrap();

        // Get with projection: only name and age.
        let resp = client
            .get_item()
            .table_name(&table_name)
            .key("pk", AttributeValue::S("user1".to_owned()))
            .projection_expression("#n, age")
            .expression_attribute_names("#n", "name")
            .send()
            .await
            .unwrap();

        let item = resp.item().unwrap();
        assert!(item.contains_key("name"));
        assert!(item.contains_key("age"));
        assert!(!item.contains_key("email"));

        // Cleanup.
        client
            .delete_table()
            .table_name(&table_name)
            .send()
            .await
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // Error Cases
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_error_on_describe_nonexistent_table() {
        let client = dynamodb_client();

        let err = client
            .describe_table()
            .table_name("nonexistent-table-xyz")
            .send()
            .await;

        assert!(err.is_err());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_error_on_duplicate_table_creation() {
        let client = dynamodb_client();
        let table_name = test_table_name("dup");

        create_simple_table(&client, &table_name).await;

        let err = client
            .create_table()
            .table_name(&table_name)
            .key_schema(
                KeySchemaElement::builder()
                    .attribute_name("pk")
                    .key_type(KeyType::Hash)
                    .build()
                    .unwrap(),
            )
            .attribute_definitions(
                AttributeDefinition::builder()
                    .attribute_name("pk")
                    .attribute_type(ScalarAttributeType::S)
                    .build()
                    .unwrap(),
            )
            .billing_mode(aws_sdk_dynamodb::types::BillingMode::PayPerRequest)
            .send()
            .await;

        assert!(err.is_err());

        // Cleanup.
        client
            .delete_table()
            .table_name(&table_name)
            .send()
            .await
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // Tagging
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_tag_and_list_tags() {
        use aws_sdk_dynamodb::types::Tag;

        let client = dynamodb_client();
        let table_name = test_table_name("tag");

        create_simple_table(&client, &table_name).await;

        // Get the table ARN for tagging operations.
        let desc = client
            .describe_table()
            .table_name(&table_name)
            .send()
            .await
            .unwrap();
        let table_arn = desc.table().unwrap().table_arn().unwrap().to_owned();

        // Tag the table with 2 tags.
        client
            .tag_resource()
            .resource_arn(&table_arn)
            .tags(Tag::builder().key("env").value("test").build().unwrap())
            .tags(Tag::builder().key("team").value("backend").build().unwrap())
            .send()
            .await
            .unwrap();

        // List tags and verify both are present.
        let resp = client
            .list_tags_of_resource()
            .resource_arn(&table_arn)
            .send()
            .await
            .unwrap();

        let tags = resp.tags();
        assert_eq!(tags.len(), 2);

        let tag_map: std::collections::HashMap<&str, &str> =
            tags.iter().map(|t| (t.key(), t.value())).collect();
        assert_eq!(tag_map.get("env"), Some(&"test"));
        assert_eq!(tag_map.get("team"), Some(&"backend"));

        // Untag one tag.
        client
            .untag_resource()
            .resource_arn(&table_arn)
            .tag_keys("team")
            .send()
            .await
            .unwrap();

        // Verify only one tag remains.
        let resp = client
            .list_tags_of_resource()
            .resource_arn(&table_arn)
            .send()
            .await
            .unwrap();

        let tags = resp.tags();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].key(), "env");
        assert_eq!(tags[0].value(), "test");

        // Cleanup.
        client
            .delete_table()
            .table_name(&table_name)
            .send()
            .await
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // TTL
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_update_and_describe_ttl() {
        use aws_sdk_dynamodb::types::TimeToLiveSpecification;

        let client = dynamodb_client();
        let table_name = test_table_name("ttl");

        create_simple_table(&client, &table_name).await;

        // Enable TTL on the "expires_at" attribute.
        client
            .update_time_to_live()
            .table_name(&table_name)
            .time_to_live_specification(
                TimeToLiveSpecification::builder()
                    .enabled(true)
                    .attribute_name("expires_at")
                    .build()
                    .unwrap(),
            )
            .send()
            .await
            .unwrap();

        // Describe TTL and verify it is enabled.
        let resp = client
            .describe_time_to_live()
            .table_name(&table_name)
            .send()
            .await
            .unwrap();

        let ttl_desc = resp.time_to_live_description().unwrap();
        assert_eq!(ttl_desc.attribute_name(), Some("expires_at"));
        // TTL status should be ENABLED or ENABLING.
        let status = ttl_desc.time_to_live_status().unwrap();
        assert!(
            matches!(
                status,
                aws_sdk_dynamodb::types::TimeToLiveStatus::Enabled
                    | aws_sdk_dynamodb::types::TimeToLiveStatus::Enabling
            ),
            "expected TTL to be ENABLED or ENABLING, got {status:?}"
        );

        // Cleanup.
        client
            .delete_table()
            .table_name(&table_name)
            .send()
            .await
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // Transactions
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_transact_write_and_get_items() {
        use aws_sdk_dynamodb::types::{Get, Put, TransactGetItem, TransactWriteItem};

        let client = dynamodb_client();
        let table_name = test_table_name("txwr");

        create_simple_table(&client, &table_name).await;

        // TransactWriteItems: put 3 items in a single transaction.
        let write_items: Vec<TransactWriteItem> = (1..=3)
            .map(|i| {
                TransactWriteItem::builder()
                    .put(
                        Put::builder()
                            .table_name(&table_name)
                            .item("pk", AttributeValue::S(format!("txn{i}")))
                            .item("data", AttributeValue::S(format!("value{i}")))
                            .build()
                            .unwrap(),
                    )
                    .build()
            })
            .collect();

        client
            .transact_write_items()
            .set_transact_items(Some(write_items))
            .send()
            .await
            .unwrap();

        // TransactGetItems: read all 3 items back.
        let get_items: Vec<TransactGetItem> = (1..=3)
            .map(|i| {
                TransactGetItem::builder()
                    .get(
                        Get::builder()
                            .table_name(&table_name)
                            .key("pk", AttributeValue::S(format!("txn{i}")))
                            .build()
                            .unwrap(),
                    )
                    .build()
            })
            .collect();

        let resp = client
            .transact_get_items()
            .set_transact_items(Some(get_items))
            .send()
            .await
            .unwrap();

        let responses = resp.responses();
        assert_eq!(responses.len(), 3);

        for (i, item_resp) in responses.iter().enumerate() {
            let item = item_resp.item().unwrap();
            assert_eq!(
                item.get("pk").unwrap().as_s().unwrap(),
                &format!("txn{}", i + 1)
            );
            assert_eq!(
                item.get("data").unwrap().as_s().unwrap(),
                &format!("value{}", i + 1)
            );
        }

        // Cleanup.
        client
            .delete_table()
            .table_name(&table_name)
            .send()
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_fail_transact_write_on_condition() {
        use aws_sdk_dynamodb::types::{ConditionCheck, Put, TransactWriteItem};

        let client = dynamodb_client();
        let table_name = test_table_name("txcond");

        create_simple_table(&client, &table_name).await;

        // Put an existing item.
        client
            .put_item()
            .table_name(&table_name)
            .item("pk", AttributeValue::S("existing".to_owned()))
            .item("status", AttributeValue::S("active".to_owned()))
            .send()
            .await
            .unwrap();

        // TransactWriteItems with a ConditionCheck that fails:
        // check that "existing" item has status = "inactive" (it does not).
        let err = client
            .transact_write_items()
            .transact_items(
                TransactWriteItem::builder()
                    .condition_check(
                        ConditionCheck::builder()
                            .table_name(&table_name)
                            .key("pk", AttributeValue::S("existing".to_owned()))
                            .condition_expression("#s = :expected")
                            .expression_attribute_names("#s", "status")
                            .expression_attribute_values(
                                ":expected",
                                AttributeValue::S("inactive".to_owned()),
                            )
                            .build()
                            .unwrap(),
                    )
                    .build(),
            )
            .transact_items(
                TransactWriteItem::builder()
                    .put(
                        Put::builder()
                            .table_name(&table_name)
                            .item("pk", AttributeValue::S("new_item".to_owned()))
                            .item("data", AttributeValue::S("should_not_exist".to_owned()))
                            .build()
                            .unwrap(),
                    )
                    .build(),
            )
            .send()
            .await;

        // The transaction should fail with TransactionCanceledException.
        assert!(err.is_err());
        let err_str = format!("{:?}", err.unwrap_err());
        assert!(
            err_str.contains("TransactionCanceled"),
            "expected TransactionCanceledException, got: {err_str}"
        );

        // Verify the new item was NOT written (transaction is atomic).
        let resp = client
            .get_item()
            .table_name(&table_name)
            .key("pk", AttributeValue::S("new_item".to_owned()))
            .send()
            .await
            .unwrap();
        assert!(resp.item().is_none());

        // Cleanup.
        client
            .delete_table()
            .table_name(&table_name)
            .send()
            .await
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // Describe Limits & Endpoints
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_describe_limits() {
        let client = dynamodb_client();

        let resp = client.describe_limits().send().await.unwrap();

        // DynamoDB default account limits.
        assert_eq!(resp.table_max_read_capacity_units(), Some(40_000));
        assert_eq!(resp.table_max_write_capacity_units(), Some(40_000));
        assert_eq!(resp.account_max_read_capacity_units(), Some(80_000));
        assert_eq!(resp.account_max_write_capacity_units(), Some(80_000));
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_describe_endpoints() {
        let client = dynamodb_client();

        let resp = client.describe_endpoints().send().await.unwrap();

        let endpoints = resp.endpoints();
        assert!(
            !endpoints.is_empty(),
            "expected at least one endpoint from DescribeEndpoints"
        );
    }
}
