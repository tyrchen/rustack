//! Integration tests for CloudWatch Logs service.
//!
//! These tests require a running RustStack server at `localhost:4566`.
//! Run with: `cargo test -p ruststack-integration -- logs --ignored`

#[cfg(test)]
mod tests {
    use aws_sdk_cloudwatchlogs::types::{InputLogEvent, MetricTransformation};

    use crate::logs_client;

    // -----------------------------------------------------------------------
    // Log group lifecycle
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_logs_should_create_and_delete_log_group() {
        let client = logs_client();
        let group_name = format!(
            "/test/group-{}",
            uuid::Uuid::new_v4().to_string()[..8].to_owned()
        );

        // Create
        client
            .create_log_group()
            .log_group_name(&group_name)
            .send()
            .await
            .unwrap();

        // Describe - should be present
        let describe = client
            .describe_log_groups()
            .log_group_name_prefix(&group_name)
            .send()
            .await
            .unwrap();
        assert!(
            describe
                .log_groups()
                .iter()
                .any(|g| g.log_group_name() == Some(group_name.as_str())),
            "log group should exist after creation"
        );

        // Delete
        client
            .delete_log_group()
            .log_group_name(&group_name)
            .send()
            .await
            .unwrap();

        // Describe - should be gone
        let describe = client
            .describe_log_groups()
            .log_group_name_prefix(&group_name)
            .send()
            .await
            .unwrap();
        assert!(
            !describe
                .log_groups()
                .iter()
                .any(|g| g.log_group_name() == Some(group_name.as_str())),
            "log group should not exist after deletion"
        );
    }

    // -----------------------------------------------------------------------
    // Log stream lifecycle
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_logs_should_create_and_delete_log_stream() {
        let client = logs_client();
        let group_name = format!(
            "/test/stream-group-{}",
            uuid::Uuid::new_v4().to_string()[..8].to_owned()
        );
        let stream_name = format!(
            "stream-{}",
            uuid::Uuid::new_v4().to_string()[..8].to_owned()
        );

        // Create group
        client
            .create_log_group()
            .log_group_name(&group_name)
            .send()
            .await
            .unwrap();

        // Create stream
        client
            .create_log_stream()
            .log_group_name(&group_name)
            .log_stream_name(&stream_name)
            .send()
            .await
            .unwrap();

        // Describe streams - should be present
        let describe = client
            .describe_log_streams()
            .log_group_name(&group_name)
            .log_stream_name_prefix(&stream_name)
            .send()
            .await
            .unwrap();
        assert!(
            describe
                .log_streams()
                .iter()
                .any(|s| s.log_stream_name() == Some(stream_name.as_str())),
            "log stream should exist after creation"
        );

        // Delete stream
        client
            .delete_log_stream()
            .log_group_name(&group_name)
            .log_stream_name(&stream_name)
            .send()
            .await
            .unwrap();

        // Cleanup
        client
            .delete_log_group()
            .log_group_name(&group_name)
            .send()
            .await
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // Put and get log events
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_logs_should_put_and_get_log_events() {
        let client = logs_client();
        let group_name = format!(
            "/test/put-get-{}",
            uuid::Uuid::new_v4().to_string()[..8].to_owned()
        );
        let stream_name = format!(
            "stream-{}",
            uuid::Uuid::new_v4().to_string()[..8].to_owned()
        );

        // Create group and stream
        client
            .create_log_group()
            .log_group_name(&group_name)
            .send()
            .await
            .unwrap();
        client
            .create_log_stream()
            .log_group_name(&group_name)
            .log_stream_name(&stream_name)
            .send()
            .await
            .unwrap();

        // Put 3 log events
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
            .try_into()
            .unwrap();

        let events = vec![
            InputLogEvent::builder()
                .timestamp(now)
                .message("message 1")
                .build()
                .unwrap(),
            InputLogEvent::builder()
                .timestamp(now + 1)
                .message("message 2")
                .build()
                .unwrap(),
            InputLogEvent::builder()
                .timestamp(now + 2)
                .message("message 3")
                .build()
                .unwrap(),
        ];

        client
            .put_log_events()
            .log_group_name(&group_name)
            .log_stream_name(&stream_name)
            .set_log_events(Some(events))
            .send()
            .await
            .unwrap();

        // Get log events
        let get = client
            .get_log_events()
            .log_group_name(&group_name)
            .log_stream_name(&stream_name)
            .start_from_head(true)
            .send()
            .await
            .unwrap();

        let returned_events = get.events();
        assert_eq!(returned_events.len(), 3, "should have 3 log events");

        // Verify messages and order
        assert_eq!(returned_events[0].message(), Some("message 1"));
        assert_eq!(returned_events[1].message(), Some("message 2"));
        assert_eq!(returned_events[2].message(), Some("message 3"));

        // Cleanup
        client
            .delete_log_group()
            .log_group_name(&group_name)
            .send()
            .await
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // Filter log events
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_logs_should_filter_log_events() {
        let client = logs_client();
        let group_name = format!(
            "/test/filter-{}",
            uuid::Uuid::new_v4().to_string()[..8].to_owned()
        );
        let stream1 = format!(
            "stream-a-{}",
            uuid::Uuid::new_v4().to_string()[..8].to_owned()
        );
        let stream2 = format!(
            "stream-b-{}",
            uuid::Uuid::new_v4().to_string()[..8].to_owned()
        );

        // Create group and 2 streams
        client
            .create_log_group()
            .log_group_name(&group_name)
            .send()
            .await
            .unwrap();
        client
            .create_log_stream()
            .log_group_name(&group_name)
            .log_stream_name(&stream1)
            .send()
            .await
            .unwrap();
        client
            .create_log_stream()
            .log_group_name(&group_name)
            .log_stream_name(&stream2)
            .send()
            .await
            .unwrap();

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
            .try_into()
            .unwrap();

        // Put events to stream 1
        client
            .put_log_events()
            .log_group_name(&group_name)
            .log_stream_name(&stream1)
            .log_events(
                InputLogEvent::builder()
                    .timestamp(now)
                    .message("ERROR something went wrong")
                    .build()
                    .unwrap(),
            )
            .log_events(
                InputLogEvent::builder()
                    .timestamp(now + 1)
                    .message("INFO all good")
                    .build()
                    .unwrap(),
            )
            .send()
            .await
            .unwrap();

        // Put events to stream 2
        client
            .put_log_events()
            .log_group_name(&group_name)
            .log_stream_name(&stream2)
            .log_events(
                InputLogEvent::builder()
                    .timestamp(now + 2)
                    .message("ERROR another failure")
                    .build()
                    .unwrap(),
            )
            .send()
            .await
            .unwrap();

        // Filter across both streams for "ERROR"
        let filter = client
            .filter_log_events()
            .log_group_name(&group_name)
            .filter_pattern("ERROR")
            .send()
            .await
            .unwrap();

        let filtered = filter.events();
        assert!(
            filtered.len() >= 2,
            "should find at least 2 ERROR events, found {}",
            filtered.len()
        );
        for event in filtered {
            assert!(
                event.message().unwrap_or_default().contains("ERROR"),
                "filtered event should contain ERROR"
            );
        }

        // Cleanup
        client
            .delete_log_group()
            .log_group_name(&group_name)
            .send()
            .await
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // Retention policy
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_logs_should_put_retention_policy() {
        let client = logs_client();
        let group_name = format!(
            "/test/retention-{}",
            uuid::Uuid::new_v4().to_string()[..8].to_owned()
        );

        // Create group
        client
            .create_log_group()
            .log_group_name(&group_name)
            .send()
            .await
            .unwrap();

        // Put retention policy - 7 days
        client
            .put_retention_policy()
            .log_group_name(&group_name)
            .retention_in_days(7)
            .send()
            .await
            .unwrap();

        // Describe - verify retention
        let describe = client
            .describe_log_groups()
            .log_group_name_prefix(&group_name)
            .send()
            .await
            .unwrap();
        let group = describe
            .log_groups()
            .iter()
            .find(|g| g.log_group_name() == Some(group_name.as_str()))
            .expect("log group should exist");
        assert_eq!(
            group.retention_in_days(),
            Some(7),
            "retention should be 7 days"
        );

        // Cleanup
        client
            .delete_log_group()
            .log_group_name(&group_name)
            .send()
            .await
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // Tags
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_logs_should_tag_log_group() {
        let client = logs_client();
        let group_name = format!(
            "/test/tags-{}",
            uuid::Uuid::new_v4().to_string()[..8].to_owned()
        );

        // Create group
        client
            .create_log_group()
            .log_group_name(&group_name)
            .send()
            .await
            .unwrap();

        // Get the log group ARN for tag operations
        let describe = client
            .describe_log_groups()
            .log_group_name_prefix(&group_name)
            .send()
            .await
            .unwrap();
        let arn = describe.log_groups()[0].arn().unwrap().to_string();

        // Tag using TagResource (modern API)
        client
            .tag_resource()
            .resource_arn(&arn)
            .tags("env", "test")
            .tags("team", "platform")
            .send()
            .await
            .unwrap();

        // List tags - verify present
        let tags_resp = client
            .list_tags_for_resource()
            .resource_arn(&arn)
            .send()
            .await
            .unwrap();
        let tags = tags_resp.tags().unwrap();
        assert_eq!(tags.get("env").map(String::as_str), Some("test"));
        assert_eq!(tags.get("team").map(String::as_str), Some("platform"));

        // Untag
        client
            .untag_resource()
            .resource_arn(&arn)
            .tag_keys("team")
            .send()
            .await
            .unwrap();

        // List tags - verify removed
        let tags_resp = client
            .list_tags_for_resource()
            .resource_arn(&arn)
            .send()
            .await
            .unwrap();
        let tags = tags_resp.tags().unwrap();
        assert_eq!(tags.get("env").map(String::as_str), Some("test"));
        assert!(!tags.contains_key("team"), "team tag should be removed");

        // Cleanup
        client
            .delete_log_group()
            .log_group_name(&group_name)
            .send()
            .await
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // Resource policy CRUD
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_logs_should_crud_resource_policy() {
        let client = logs_client();
        let policy_name = format!(
            "test-policy-{}",
            uuid::Uuid::new_v4().to_string()[..8].to_owned()
        );
        let policy_document = r#"{"Version":"2012-10-17","Statement":[{"Sid":"AllowLogs","Effect":"Allow","Principal":{"Service":"es.amazonaws.com"},"Action":["logs:PutLogEvents","logs:CreateLogStream"],"Resource":"*"}]}"#;

        // Put resource policy
        client
            .put_resource_policy()
            .policy_name(&policy_name)
            .policy_document(policy_document)
            .send()
            .await
            .unwrap();

        // Describe - verify present
        let describe = client.describe_resource_policies().send().await.unwrap();
        assert!(
            describe
                .resource_policies()
                .iter()
                .any(|p| p.policy_name() == Some(policy_name.as_str())),
            "resource policy should exist"
        );

        // Delete
        client
            .delete_resource_policy()
            .policy_name(&policy_name)
            .send()
            .await
            .unwrap();

        // Describe - verify gone
        let describe = client.describe_resource_policies().send().await.unwrap();
        assert!(
            !describe
                .resource_policies()
                .iter()
                .any(|p| p.policy_name() == Some(policy_name.as_str())),
            "resource policy should not exist after deletion"
        );
    }

    // -----------------------------------------------------------------------
    // Metric filter CRUD
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_logs_should_crud_metric_filter() {
        let client = logs_client();
        let group_name = format!(
            "/test/metric-filter-{}",
            uuid::Uuid::new_v4().to_string()[..8].to_owned()
        );
        let filter_name = format!(
            "filter-{}",
            uuid::Uuid::new_v4().to_string()[..8].to_owned()
        );

        // Create group
        client
            .create_log_group()
            .log_group_name(&group_name)
            .send()
            .await
            .unwrap();

        // Put metric filter
        let transformation = MetricTransformation::builder()
            .metric_name("ErrorCount")
            .metric_namespace("TestNamespace")
            .metric_value("1")
            .build()
            .unwrap();

        client
            .put_metric_filter()
            .log_group_name(&group_name)
            .filter_name(&filter_name)
            .filter_pattern("ERROR")
            .metric_transformations(transformation)
            .send()
            .await
            .unwrap();

        // Describe - verify present
        let describe = client
            .describe_metric_filters()
            .log_group_name(&group_name)
            .filter_name_prefix(&filter_name)
            .send()
            .await
            .unwrap();
        assert!(
            describe
                .metric_filters()
                .iter()
                .any(|f| f.filter_name() == Some(filter_name.as_str())),
            "metric filter should exist"
        );

        // Delete metric filter
        client
            .delete_metric_filter()
            .log_group_name(&group_name)
            .filter_name(&filter_name)
            .send()
            .await
            .unwrap();

        // Describe - verify gone
        let describe = client
            .describe_metric_filters()
            .log_group_name(&group_name)
            .filter_name_prefix(&filter_name)
            .send()
            .await
            .unwrap();
        assert!(
            !describe
                .metric_filters()
                .iter()
                .any(|f| f.filter_name() == Some(filter_name.as_str())),
            "metric filter should not exist after deletion"
        );

        // Cleanup
        client
            .delete_log_group()
            .log_group_name(&group_name)
            .send()
            .await
            .unwrap();
    }
}
