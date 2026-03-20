//! CloudWatch Metrics integration tests against a running `RustStack` server.
//!
//! These tests cover metrics, alarms, dashboards, composite alarms, tags,
//! insight rules, anomaly detectors, and metric streams.
//!
//! Run with: `cargo test -p ruststack-integration -- cloudwatch --ignored`

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use aws_sdk_cloudwatch as cloudwatch;
    use aws_sdk_cloudwatch::types::{
        AlarmType, ComparisonOperator, Dimension, MetricDatum, StandardUnit, StateValue, Statistic,
        StatisticSet, Tag,
    };

    use crate::cloudwatch_client;

    /// Generate a unique name fragment for test isolation.
    fn uid() -> String {
        uuid::Uuid::new_v4().to_string()[..8].to_owned()
    }

    // -----------------------------------------------------------------------
    // Metric data: put and get statistics
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_put_and_get_metric_statistics() {
        let client = cloudwatch_client();
        let namespace = format!("TestApp/{}", uid());
        let metric_name = "RequestCount";

        let now = aws_sdk_cloudwatch::primitives::DateTime::from_secs(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                .cast_signed(),
        );

        let dim = Dimension::builder()
            .name("Environment")
            .value("Production")
            .build();

        // Put first data point: value 42
        client
            .put_metric_data()
            .namespace(&namespace)
            .metric_data(
                MetricDatum::builder()
                    .metric_name(metric_name)
                    .value(42.0)
                    .unit(StandardUnit::Count)
                    .timestamp(now)
                    .dimensions(dim.clone())
                    .build(),
            )
            .send()
            .await
            .unwrap();

        // Put second data point: value 58
        client
            .put_metric_data()
            .namespace(&namespace)
            .metric_data(
                MetricDatum::builder()
                    .metric_name(metric_name)
                    .value(58.0)
                    .unit(StandardUnit::Count)
                    .timestamp(now)
                    .dimensions(dim.clone())
                    .build(),
            )
            .send()
            .await
            .unwrap();

        // Query statistics over a window that includes our data points.
        let start = aws_sdk_cloudwatch::primitives::DateTime::from_secs(now.secs() - 3600);
        let end = aws_sdk_cloudwatch::primitives::DateTime::from_secs(now.secs() + 3600);

        let stats = client
            .get_metric_statistics()
            .namespace(&namespace)
            .metric_name(metric_name)
            .dimensions(dim)
            .start_time(start)
            .end_time(end)
            .period(3600)
            .statistics(Statistic::Sum)
            .statistics(Statistic::Average)
            .statistics(Statistic::SampleCount)
            .send()
            .await
            .unwrap();

        let datapoints = stats.datapoints();
        assert!(!datapoints.is_empty(), "should have at least one datapoint");

        let dp = &datapoints[0];
        assert_eq!(dp.sum(), Some(100.0), "sum should be 42+58=100");
        assert_eq!(dp.average(), Some(50.0), "average should be 50");
        assert_eq!(dp.sample_count(), Some(2.0), "sample count should be 2");
    }

    // -----------------------------------------------------------------------
    // List metrics with filters
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_list_metrics_with_filters() {
        let client = cloudwatch_client();
        let ns1 = format!("TestApp1/{}", uid());
        let ns2 = format!("TestApp2/{}", uid());

        // Put metric in namespace 1.
        client
            .put_metric_data()
            .namespace(&ns1)
            .metric_data(
                MetricDatum::builder()
                    .metric_name("Metric1")
                    .value(1.0)
                    .build(),
            )
            .send()
            .await
            .unwrap();

        // Put metric in namespace 2.
        client
            .put_metric_data()
            .namespace(&ns2)
            .metric_data(
                MetricDatum::builder()
                    .metric_name("Metric2")
                    .value(2.0)
                    .build(),
            )
            .send()
            .await
            .unwrap();

        // List with namespace filter -> expect 1 metric.
        let filtered = client.list_metrics().namespace(&ns1).send().await.unwrap();
        assert_eq!(
            filtered.metrics().len(),
            1,
            "filtered list should have exactly 1 metric"
        );
        assert_eq!(
            filtered.metrics()[0].metric_name(),
            Some("Metric1"),
            "filtered metric should be Metric1"
        );

        // List without filter -> expect at least 2.
        let all = client.list_metrics().send().await.unwrap();
        assert!(
            all.metrics().len() >= 2,
            "unfiltered list should have at least 2 metrics, got {}",
            all.metrics().len()
        );
    }

    // -----------------------------------------------------------------------
    // Metric alarm lifecycle
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_manage_metric_alarms() {
        let client = cloudwatch_client();
        let alarm_name = format!("HighCPU-{}", uid());
        let namespace = "AWS/EC2";

        // Create alarm.
        client
            .put_metric_alarm()
            .alarm_name(&alarm_name)
            .namespace(namespace)
            .metric_name("CPUUtilization")
            .comparison_operator(ComparisonOperator::GreaterThanThreshold)
            .threshold(80.0)
            .period(300)
            .evaluation_periods(3)
            .statistic(Statistic::Average)
            .send()
            .await
            .unwrap();

        // Describe alarms -> verify alarm exists.
        let alarms = client
            .describe_alarms()
            .alarm_names(&alarm_name)
            .send()
            .await
            .unwrap();
        let metric_alarms = alarms.metric_alarms();
        assert_eq!(metric_alarms.len(), 1, "should find exactly 1 alarm");
        let alarm = &metric_alarms[0];
        assert_eq!(alarm.alarm_name(), Some(alarm_name.as_str()));
        assert_eq!(alarm.namespace(), Some(namespace));
        assert_eq!(alarm.metric_name(), Some("CPUUtilization"));
        assert_eq!(alarm.threshold(), Some(80.0));
        assert_eq!(alarm.period(), Some(300));
        assert_eq!(alarm.evaluation_periods(), Some(3));

        // Set alarm state to ALARM.
        client
            .set_alarm_state()
            .alarm_name(&alarm_name)
            .state_value(StateValue::Alarm)
            .state_reason("testing")
            .send()
            .await
            .unwrap();

        // Describe alarms with state filter ALARM -> should include our alarm.
        let alarm_state = client
            .describe_alarms()
            .state_value(StateValue::Alarm)
            .send()
            .await
            .unwrap();
        let in_alarm: Vec<_> = alarm_state
            .metric_alarms()
            .iter()
            .filter(|a| a.alarm_name() == Some(alarm_name.as_str()))
            .collect();
        assert_eq!(
            in_alarm.len(),
            1,
            "alarm should appear in ALARM state filter"
        );

        // Describe alarm history -> verify state change recorded.
        let history = client
            .describe_alarm_history()
            .alarm_name(&alarm_name)
            .send()
            .await
            .unwrap();
        assert!(
            !history.alarm_history_items().is_empty(),
            "alarm history should contain at least one entry"
        );

        // Delete alarm.
        client
            .delete_alarms()
            .alarm_names(&alarm_name)
            .send()
            .await
            .unwrap();

        // Verify deletion.
        let after_delete = client
            .describe_alarms()
            .alarm_names(&alarm_name)
            .send()
            .await
            .unwrap();
        assert!(
            after_delete.metric_alarms().is_empty(),
            "alarm should be gone after deletion"
        );
    }

    // -----------------------------------------------------------------------
    // Alarm actions: enable / disable
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_manage_alarm_actions() {
        let client = cloudwatch_client();
        let alarm_name = format!("ActionAlarm-{}", uid());

        // Create alarm with actions.
        client
            .put_metric_alarm()
            .alarm_name(&alarm_name)
            .namespace("TestNS")
            .metric_name("TestMetric")
            .comparison_operator(ComparisonOperator::GreaterThanThreshold)
            .threshold(50.0)
            .period(60)
            .evaluation_periods(1)
            .statistic(Statistic::Average)
            .alarm_actions("arn:aws:sns:us-east-1:000000000000:my-topic")
            .ok_actions("arn:aws:sns:us-east-1:000000000000:my-ok-topic")
            .send()
            .await
            .unwrap();

        // Enable alarm actions.
        client
            .enable_alarm_actions()
            .alarm_names(&alarm_name)
            .send()
            .await
            .unwrap();

        // Verify actions enabled.
        let desc = client
            .describe_alarms()
            .alarm_names(&alarm_name)
            .send()
            .await
            .unwrap();
        assert_eq!(
            desc.metric_alarms()[0].actions_enabled(),
            Some(true),
            "actions should be enabled"
        );

        // Disable alarm actions.
        client
            .disable_alarm_actions()
            .alarm_names(&alarm_name)
            .send()
            .await
            .unwrap();

        // Verify actions disabled.
        let desc = client
            .describe_alarms()
            .alarm_names(&alarm_name)
            .send()
            .await
            .unwrap();
        assert_eq!(
            desc.metric_alarms()[0].actions_enabled(),
            Some(false),
            "actions should be disabled"
        );

        // Cleanup.
        client
            .delete_alarms()
            .alarm_names(&alarm_name)
            .send()
            .await
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // Describe alarms for metric
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_describe_alarms_for_metric() {
        let client = cloudwatch_client();
        let id = uid();
        let alarm1 = format!("alarm-1-{id}");
        let alarm2 = format!("alarm-2-{id}");
        let namespace = format!("NS1/{id}");

        // Create alarm-1 for metric M1.
        client
            .put_metric_alarm()
            .alarm_name(&alarm1)
            .namespace(&namespace)
            .metric_name("M1")
            .comparison_operator(ComparisonOperator::GreaterThanThreshold)
            .threshold(10.0)
            .period(60)
            .evaluation_periods(1)
            .statistic(Statistic::Average)
            .send()
            .await
            .unwrap();

        // Create alarm-2 for metric M2.
        client
            .put_metric_alarm()
            .alarm_name(&alarm2)
            .namespace(&namespace)
            .metric_name("M2")
            .comparison_operator(ComparisonOperator::GreaterThanThreshold)
            .threshold(20.0)
            .period(60)
            .evaluation_periods(1)
            .statistic(Statistic::Average)
            .send()
            .await
            .unwrap();

        // Describe alarms for metric M1 -> should only include alarm-1.
        let result = client
            .describe_alarms_for_metric()
            .namespace(&namespace)
            .metric_name("M1")
            .send()
            .await
            .unwrap();

        let alarm_names: Vec<_> = result
            .metric_alarms()
            .iter()
            .filter_map(|a| a.alarm_name())
            .collect();
        assert!(
            alarm_names.contains(&alarm1.as_str()),
            "alarm-1 should be returned for M1"
        );
        assert!(
            !alarm_names.contains(&alarm2.as_str()),
            "alarm-2 should NOT be returned for M1"
        );

        // Cleanup.
        client
            .delete_alarms()
            .alarm_names(&alarm1)
            .alarm_names(&alarm2)
            .send()
            .await
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // Tags: tag, list, untag
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_manage_tags() {
        let client = cloudwatch_client();
        let alarm_name = format!("TagAlarm-{}", uid());

        // Create alarm to get a resource ARN.
        client
            .put_metric_alarm()
            .alarm_name(&alarm_name)
            .namespace("TagNS")
            .metric_name("TagMetric")
            .comparison_operator(ComparisonOperator::GreaterThanThreshold)
            .threshold(1.0)
            .period(60)
            .evaluation_periods(1)
            .statistic(Statistic::Average)
            .send()
            .await
            .unwrap();

        // Get the alarm ARN.
        let desc = client
            .describe_alarms()
            .alarm_names(&alarm_name)
            .send()
            .await
            .unwrap();
        let alarm_arn = desc.metric_alarms()[0].alarm_arn().unwrap().to_string();

        // Tag with two tags.
        client
            .tag_resource()
            .resource_arn(&alarm_arn)
            .tags(Tag::builder().key("Env").value("Prod").build())
            .tags(Tag::builder().key("Team").value("Platform").build())
            .send()
            .await
            .unwrap();

        // List tags -> verify 2 tags.
        let tags_resp = client
            .list_tags_for_resource()
            .resource_arn(&alarm_arn)
            .send()
            .await
            .unwrap();
        let tag_map: HashMap<_, _> = tags_resp
            .tags()
            .iter()
            .map(|t| {
                (
                    t.key().unwrap_or_default().to_string(),
                    t.value().unwrap_or_default().to_string(),
                )
            })
            .collect();
        assert_eq!(tag_map.len(), 2, "should have 2 tags");
        assert_eq!(tag_map.get("Env").unwrap(), "Prod");
        assert_eq!(tag_map.get("Team").unwrap(), "Platform");

        // Untag "Team".
        client
            .untag_resource()
            .resource_arn(&alarm_arn)
            .tag_keys("Team")
            .send()
            .await
            .unwrap();

        // List tags -> verify 1 tag (Env only).
        let tags_resp = client
            .list_tags_for_resource()
            .resource_arn(&alarm_arn)
            .send()
            .await
            .unwrap();
        let tag_map: HashMap<_, _> = tags_resp
            .tags()
            .iter()
            .map(|t| {
                (
                    t.key().unwrap_or_default().to_string(),
                    t.value().unwrap_or_default().to_string(),
                )
            })
            .collect();
        assert_eq!(tag_map.len(), 1, "should have 1 tag after untag");
        assert!(tag_map.contains_key("Env"), "Env tag should remain");
        assert!(!tag_map.contains_key("Team"), "Team tag should be removed");

        // Cleanup.
        client
            .delete_alarms()
            .alarm_names(&alarm_name)
            .send()
            .await
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // Dashboards
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_manage_dashboards() {
        let client = cloudwatch_client();
        let dash_name = format!("TestDash-{}", uid());
        let body_v1 = r#"{"widgets":[]}"#;
        let body_v2 = r#"{"widgets":[{"type":"text","properties":{"markdown":"hello"}}]}"#;

        // Create dashboard.
        client
            .put_dashboard()
            .dashboard_name(&dash_name)
            .dashboard_body(body_v1)
            .send()
            .await
            .unwrap();

        // Get dashboard -> verify body.
        let get = client
            .get_dashboard()
            .dashboard_name(&dash_name)
            .send()
            .await
            .unwrap();
        assert_eq!(
            get.dashboard_body().unwrap(),
            body_v1,
            "dashboard body should match"
        );
        assert_eq!(
            get.dashboard_name().unwrap(),
            dash_name.as_str(),
            "dashboard name should match"
        );

        // List dashboards -> includes ours.
        let list = client.list_dashboards().send().await.unwrap();
        let names: Vec<_> = list
            .dashboard_entries()
            .iter()
            .filter_map(|d| d.dashboard_name())
            .collect();
        assert!(
            names.contains(&dash_name.as_str()),
            "dashboard should be in list"
        );

        // Update dashboard.
        client
            .put_dashboard()
            .dashboard_name(&dash_name)
            .dashboard_body(body_v2)
            .send()
            .await
            .unwrap();

        // Verify update.
        let get2 = client
            .get_dashboard()
            .dashboard_name(&dash_name)
            .send()
            .await
            .unwrap();
        assert_eq!(
            get2.dashboard_body().unwrap(),
            body_v2,
            "dashboard body should be updated"
        );

        // Delete dashboard.
        client
            .delete_dashboards()
            .dashboard_names(&dash_name)
            .send()
            .await
            .unwrap();

        // Get after delete -> should fail.
        let err = client
            .get_dashboard()
            .dashboard_name(&dash_name)
            .send()
            .await;
        assert!(err.is_err(), "get_dashboard should fail after deletion");
    }

    // -----------------------------------------------------------------------
    // Composite alarms
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_manage_composite_alarms() {
        let client = cloudwatch_client();
        let id = uid();
        let cpu_alarm = format!("cpu-alarm-{id}");
        let composite_name = format!("composite-{id}");

        // Create underlying metric alarm.
        client
            .put_metric_alarm()
            .alarm_name(&cpu_alarm)
            .namespace("AWS/EC2")
            .metric_name("CPUUtilization")
            .comparison_operator(ComparisonOperator::GreaterThanThreshold)
            .threshold(90.0)
            .period(300)
            .evaluation_periods(1)
            .statistic(Statistic::Average)
            .send()
            .await
            .unwrap();

        // Create composite alarm referencing the metric alarm.
        let alarm_rule = format!("ALARM(\"{cpu_alarm}\")");
        client
            .put_composite_alarm()
            .alarm_name(&composite_name)
            .alarm_rule(&alarm_rule)
            .send()
            .await
            .unwrap();

        // Describe alarms with alarm_types CompositeAlarm -> includes ours.
        let desc = client
            .describe_alarms()
            .alarm_types(AlarmType::CompositeAlarm)
            .send()
            .await
            .unwrap();
        let composite_names: Vec<_> = desc
            .composite_alarms()
            .iter()
            .filter_map(|a| a.alarm_name())
            .collect();
        assert!(
            composite_names.contains(&composite_name.as_str()),
            "composite alarm should be in describe results"
        );

        // Cleanup.
        client
            .delete_alarms()
            .alarm_names(&composite_name)
            .alarm_names(&cpu_alarm)
            .send()
            .await
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // GetMetricData
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_get_metric_data() {
        let client = cloudwatch_client();
        let namespace = format!("TestData/{}", uid());
        let metric_name = "Latency";

        let now = aws_sdk_cloudwatch::primitives::DateTime::from_secs(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                .cast_signed(),
        );

        // Put metric data.
        client
            .put_metric_data()
            .namespace(&namespace)
            .metric_data(
                MetricDatum::builder()
                    .metric_name(metric_name)
                    .value(123.0)
                    .unit(StandardUnit::Milliseconds)
                    .timestamp(now)
                    .build(),
            )
            .send()
            .await
            .unwrap();

        let start = aws_sdk_cloudwatch::primitives::DateTime::from_secs(now.secs() - 3600);
        let end = aws_sdk_cloudwatch::primitives::DateTime::from_secs(now.secs() + 3600);

        // Build a MetricDataQuery.
        let metric_stat = cloudwatch::types::MetricStat::builder()
            .metric(
                cloudwatch::types::Metric::builder()
                    .namespace(&namespace)
                    .metric_name(metric_name)
                    .build(),
            )
            .period(3600)
            .stat("Sum")
            .build();

        let query = cloudwatch::types::MetricDataQuery::builder()
            .id("q1")
            .metric_stat(metric_stat)
            .build();

        let result = client
            .get_metric_data()
            .metric_data_queries(query)
            .start_time(start)
            .end_time(end)
            .send()
            .await
            .unwrap();

        let results = result.metric_data_results();
        assert!(
            !results.is_empty(),
            "GetMetricData should return at least one result"
        );

        let r = &results[0];
        assert_eq!(r.id(), Some("q1"), "result ID should match query ID");
        assert!(
            !r.values().is_empty(),
            "result should contain at least one value"
        );
    }

    // -----------------------------------------------------------------------
    // StatisticValues (pre-aggregated data)
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_handle_statistic_values() {
        let client = cloudwatch_client();
        let namespace = format!("TestStatValues/{}", uid());
        let metric_name = "PreAggregated";

        let now = aws_sdk_cloudwatch::primitives::DateTime::from_secs(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                .cast_signed(),
        );

        // Put metric data with StatisticValues (pre-aggregated).
        let stat_values = StatisticSet::builder()
            .sum(100.0)
            .minimum(5.0)
            .maximum(50.0)
            .sample_count(10.0)
            .build();

        client
            .put_metric_data()
            .namespace(&namespace)
            .metric_data(
                MetricDatum::builder()
                    .metric_name(metric_name)
                    .statistic_values(stat_values)
                    .unit(StandardUnit::Count)
                    .timestamp(now)
                    .build(),
            )
            .send()
            .await
            .unwrap();

        // Get statistics.
        let start = aws_sdk_cloudwatch::primitives::DateTime::from_secs(now.secs() - 3600);
        let end = aws_sdk_cloudwatch::primitives::DateTime::from_secs(now.secs() + 3600);

        let stats = client
            .get_metric_statistics()
            .namespace(&namespace)
            .metric_name(metric_name)
            .start_time(start)
            .end_time(end)
            .period(3600)
            .statistics(Statistic::Sum)
            .statistics(Statistic::Minimum)
            .statistics(Statistic::Maximum)
            .statistics(Statistic::SampleCount)
            .send()
            .await
            .unwrap();

        let datapoints = stats.datapoints();
        assert!(!datapoints.is_empty(), "should have at least one datapoint");

        let dp = &datapoints[0];
        assert_eq!(dp.sum(), Some(100.0), "sum should be 100");
        assert_eq!(dp.minimum(), Some(5.0), "minimum should be 5");
        assert_eq!(dp.maximum(), Some(50.0), "maximum should be 50");
        assert_eq!(dp.sample_count(), Some(10.0), "sample count should be 10");
    }

    // -----------------------------------------------------------------------
    // Insight rules
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_manage_insight_rules() {
        let client = cloudwatch_client();
        let rule_name = format!("InsightRule-{}", uid());
        let rule_definition = r#"{
            "Schema": {"Name": "CloudWatchLogRule", "Version": 1},
            "LogGroupNames": ["/test/logs"],
            "Contribution": {
                "Filters": [],
                "Keys": ["$.ip"]
            },
            "AggregateOn": "Count"
        }"#;

        // Put insight rule.
        client
            .put_insight_rule()
            .rule_name(&rule_name)
            .rule_definition(rule_definition)
            .rule_state("ENABLED")
            .send()
            .await
            .unwrap();

        // Describe insight rules -> verify present.
        let desc = client.describe_insight_rules().send().await.unwrap();
        let rule_names: Vec<_> = desc
            .insight_rules()
            .iter()
            .filter_map(|r| r.name.as_deref())
            .collect();
        assert!(
            rule_names.contains(&rule_name.as_str()),
            "insight rule should be in describe results"
        );

        // Delete insight rules.
        client
            .delete_insight_rules()
            .rule_names(&rule_name)
            .send()
            .await
            .unwrap();

        // Verify deletion.
        let desc2 = client.describe_insight_rules().send().await.unwrap();
        let rule_names2: Vec<_> = desc2
            .insight_rules()
            .iter()
            .filter_map(|r| r.name.as_deref())
            .collect();
        assert!(
            !rule_names2.contains(&rule_name.as_str()),
            "insight rule should be gone after deletion"
        );
    }

    // -----------------------------------------------------------------------
    // Anomaly detectors
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_manage_anomaly_detectors() {
        let client = cloudwatch_client();
        let namespace = format!("AnomalyNS/{}", uid());
        let metric_name = "AnomalyMetric";

        // Put anomaly detector using single-metric configuration.
        let single_metric = cloudwatch::types::SingleMetricAnomalyDetector::builder()
            .namespace(&namespace)
            .metric_name(metric_name)
            .stat("Average")
            .build();

        client
            .put_anomaly_detector()
            .single_metric_anomaly_detector(single_metric)
            .send()
            .await
            .unwrap();

        // Describe anomaly detectors -> verify present.
        let desc = client
            .describe_anomaly_detectors()
            .namespace(&namespace)
            .send()
            .await
            .unwrap();
        assert!(
            !desc.anomaly_detectors().is_empty(),
            "should have at least one anomaly detector"
        );

        let detector = &desc.anomaly_detectors()[0];
        let single = detector.single_metric_anomaly_detector().unwrap();
        assert_eq!(
            single.namespace(),
            Some(namespace.as_str()),
            "namespace should match"
        );
        assert_eq!(
            single.metric_name(),
            Some(metric_name),
            "metric name should match"
        );

        // Delete anomaly detector.
        let delete_single = cloudwatch::types::SingleMetricAnomalyDetector::builder()
            .namespace(&namespace)
            .metric_name(metric_name)
            .stat("Average")
            .build();

        client
            .delete_anomaly_detector()
            .single_metric_anomaly_detector(delete_single)
            .send()
            .await
            .unwrap();

        // Verify deletion.
        let desc2 = client
            .describe_anomaly_detectors()
            .namespace(&namespace)
            .send()
            .await
            .unwrap();
        assert!(
            desc2.anomaly_detectors().is_empty(),
            "anomaly detector should be gone after deletion"
        );
    }

    // -----------------------------------------------------------------------
    // Metric streams
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_manage_metric_streams() {
        let client = cloudwatch_client();
        let stream_name = format!("TestStream-{}", uid());
        let firehose_arn = "arn:aws:firehose:us-east-1:000000000000:deliverystream/test-stream";
        let role_arn = "arn:aws:iam::000000000000:role/test-role";

        // Put metric stream.
        client
            .put_metric_stream()
            .name(&stream_name)
            .firehose_arn(firehose_arn)
            .role_arn(role_arn)
            .output_format(cloudwatch::types::MetricStreamOutputFormat::Json)
            .send()
            .await
            .unwrap();

        // Get metric stream -> verify.
        let get = client
            .get_metric_stream()
            .name(&stream_name)
            .send()
            .await
            .unwrap();
        assert_eq!(
            get.name(),
            Some(stream_name.as_str()),
            "stream name should match"
        );
        assert_eq!(
            get.firehose_arn(),
            Some(firehose_arn),
            "firehose ARN should match"
        );

        // List metric streams -> includes ours.
        let list = client.list_metric_streams().send().await.unwrap();
        let stream_names: Vec<_> = list.entries().iter().filter_map(|e| e.name()).collect();
        assert!(
            stream_names.contains(&stream_name.as_str()),
            "metric stream should be in list"
        );

        // Delete metric stream.
        client
            .delete_metric_stream()
            .name(&stream_name)
            .send()
            .await
            .unwrap();

        // Verify deletion.
        let get_err = client.get_metric_stream().name(&stream_name).send().await;
        assert!(
            get_err.is_err(),
            "get_metric_stream should fail after deletion"
        );
    }
}
