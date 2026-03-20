//! CloudWatch handler implementation bridging HTTP to business logic.
//!
//! Parses form-urlencoded request bodies, dispatches to the provider,
//! and serializes XML responses following the awsQuery protocol.
//!
//! Covers all 31 CloudWatch operations: metrics, alarms, dashboards,
//! insight rules, anomaly detectors, metric streams, and tagging.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;

use ruststack_cloudwatch_http::body::CloudWatchResponseBody;
use ruststack_cloudwatch_http::dispatch::CloudWatchHandler;
use ruststack_cloudwatch_http::request::{
    get_optional_bool, get_optional_f64, get_optional_i32, get_optional_param, get_required_param,
    parse_dimension_filters, parse_dimensions, parse_form_params, parse_string_list,
    parse_struct_list, parse_tag_list,
};
use ruststack_cloudwatch_http::response::{XmlWriter, xml_response};
use ruststack_cloudwatch_model::error::{CloudWatchError, CloudWatchErrorCode};
use ruststack_cloudwatch_model::input::{
    DeleteAlarmsInput, DeleteAnomalyDetectorInput, DeleteDashboardsInput, DeleteInsightRulesInput,
    DeleteMetricStreamInput, DescribeAlarmHistoryInput, DescribeAlarmsForMetricInput,
    DescribeAlarmsInput, DescribeAnomalyDetectorsInput, DescribeInsightRulesInput,
    DisableAlarmActionsInput, EnableAlarmActionsInput, GetDashboardInput, GetMetricDataInput,
    GetMetricStatisticsInput, GetMetricStreamInput, ListDashboardsInput, ListMetricStreamsInput,
    ListMetricsInput, ListTagsForResourceInput, PutAnomalyDetectorInput, PutCompositeAlarmInput,
    PutDashboardInput, PutInsightRuleInput, PutManagedInsightRulesInput, PutMetricAlarmInput,
    PutMetricDataInput, PutMetricStreamInput, SetAlarmStateInput, TagResourceInput,
    UntagResourceInput,
};
use ruststack_cloudwatch_model::operations::CloudWatchOperation;
use ruststack_cloudwatch_model::types::{
    AlarmType, AnomalyDetectorType, ComparisonOperator, CompositeAlarm, Dimension, DimensionFilter,
    HistoryItemType, LabelOptions, ManagedRule, MetricAlarm, MetricCharacteristics,
    MetricDataQuery, MetricDatum, MetricMathAnomalyDetector, MetricStat, MetricStreamFilter,
    MetricStreamOutputFormat, MetricStreamStatisticsConfiguration, MetricStreamStatisticsMetric,
    RecentlyActive, ScanBy, SingleMetricAnomalyDetector, StandardUnit, StateValue, Statistic,
    StatisticSet, Tag,
};

use crate::provider::RustStackCloudWatch;

/// Handler that bridges the HTTP layer to the CloudWatch provider.
#[derive(Debug)]
pub struct RustStackCloudWatchHandler {
    provider: Arc<RustStackCloudWatch>,
}

impl RustStackCloudWatchHandler {
    /// Create a new handler wrapping a provider.
    #[must_use]
    pub fn new(provider: Arc<RustStackCloudWatch>) -> Self {
        Self { provider }
    }
}

impl CloudWatchHandler for RustStackCloudWatchHandler {
    fn handle_operation(
        &self,
        op: CloudWatchOperation,
        body: Bytes,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<http::Response<CloudWatchResponseBody>, CloudWatchError>>
                + Send,
        >,
    > {
        let provider = Arc::clone(&self.provider);
        Box::pin(async move { dispatch(provider.as_ref(), op, &body) })
    }
}

/// Dispatch a CloudWatch operation to the appropriate handler method.
#[allow(clippy::too_many_lines)]
fn dispatch(
    provider: &RustStackCloudWatch,
    op: CloudWatchOperation,
    body: &[u8],
) -> Result<http::Response<CloudWatchResponseBody>, CloudWatchError> {
    let params = parse_form_params(body);
    let request_id = uuid::Uuid::new_v4().to_string();

    match op {
        // ---- Metric Data ----
        CloudWatchOperation::PutMetricData => {
            let input = deserialize_put_metric_data(&params)?;
            provider.put_metric_data(input)?;
            let mut w = XmlWriter::new();
            w.start_response("PutMetricData");
            w.start_result("PutMetricData");
            w.end_element("PutMetricDataResult");
            w.write_response_metadata(&request_id);
            w.end_element("PutMetricDataResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        CloudWatchOperation::GetMetricStatistics => {
            let input = deserialize_get_metric_statistics(&params)?;
            let output = provider.get_metric_statistics(input)?;
            let mut w = XmlWriter::new();
            w.start_response("GetMetricStatistics");
            w.start_result("GetMetricStatistics");
            w.write_optional_element("Label", output.label.as_deref());
            w.raw("<Datapoints>");
            for dp in &output.datapoints {
                w.raw("<member>");
                if let Some(ts) = &dp.timestamp {
                    w.write_element("Timestamp", &ts.format("%Y-%m-%dT%H:%M:%SZ").to_string());
                }
                w.write_optional_f64_element("Average", dp.average);
                w.write_optional_f64_element("Sum", dp.sum);
                w.write_optional_f64_element("Minimum", dp.minimum);
                w.write_optional_f64_element("Maximum", dp.maximum);
                w.write_optional_f64_element("SampleCount", dp.sample_count);
                if let Some(unit) = &dp.unit {
                    w.write_element("Unit", unit.as_str());
                }
                if !dp.extended_statistics.is_empty() {
                    w.raw("<ExtendedStatistics>");
                    let mut keys: Vec<&String> = dp.extended_statistics.keys().collect();
                    keys.sort();
                    for k in keys {
                        if let Some(v) = dp.extended_statistics.get(k) {
                            w.raw("<entry>");
                            w.write_element("key", k);
                            w.write_f64_element("value", *v);
                            w.raw("</entry>");
                        }
                    }
                    w.raw("</ExtendedStatistics>");
                }
                w.raw("</member>");
            }
            w.raw("</Datapoints>");
            w.end_element("GetMetricStatisticsResult");
            w.write_response_metadata(&request_id);
            w.end_element("GetMetricStatisticsResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        CloudWatchOperation::GetMetricData => {
            let input = deserialize_get_metric_data(&params)?;
            let output = provider.get_metric_data(input)?;
            let mut w = XmlWriter::new();
            w.start_response("GetMetricData");
            w.start_result("GetMetricData");
            w.raw("<MetricDataResults>");
            for result in &output.metric_data_results {
                w.raw("<member>");
                w.write_optional_element("Id", result.id.as_deref());
                w.write_optional_element("Label", result.label.as_deref());
                if let Some(sc) = &result.status_code {
                    w.write_element("StatusCode", sc.as_str());
                }
                if !result.timestamps.is_empty() {
                    w.raw("<Timestamps>");
                    for ts in &result.timestamps {
                        w.write_element("member", &ts.format("%Y-%m-%dT%H:%M:%SZ").to_string());
                    }
                    w.raw("</Timestamps>");
                }
                if !result.values.is_empty() {
                    w.raw("<Values>");
                    for v in &result.values {
                        w.write_element("member", &v.to_string());
                    }
                    w.raw("</Values>");
                }
                if !result.messages.is_empty() {
                    w.raw("<Messages>");
                    for msg in &result.messages {
                        w.raw("<member>");
                        w.write_optional_element("Code", msg.code.as_deref());
                        w.write_optional_element("Value", msg.value.as_deref());
                        w.raw("</member>");
                    }
                    w.raw("</Messages>");
                }
                w.raw("</member>");
            }
            w.raw("</MetricDataResults>");
            if !output.messages.is_empty() {
                w.raw("<Messages>");
                for msg in &output.messages {
                    w.raw("<member>");
                    w.write_optional_element("Code", msg.code.as_deref());
                    w.write_optional_element("Value", msg.value.as_deref());
                    w.raw("</member>");
                }
                w.raw("</Messages>");
            }
            w.write_optional_element("NextToken", output.next_token.as_deref());
            w.end_element("GetMetricDataResult");
            w.write_response_metadata(&request_id);
            w.end_element("GetMetricDataResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        CloudWatchOperation::ListMetrics => {
            let input = deserialize_list_metrics(&params);
            let output = provider.list_metrics(input)?;
            let mut w = XmlWriter::new();
            w.start_response("ListMetrics");
            w.start_result("ListMetrics");
            w.raw("<Metrics>");
            for metric in &output.metrics {
                w.raw("<member>");
                w.write_optional_element("MetricName", metric.metric_name.as_deref());
                w.write_optional_element("Namespace", metric.namespace.as_deref());
                if !metric.dimensions.is_empty() {
                    write_dimensions_xml(&mut w, &metric.dimensions);
                }
                w.raw("</member>");
            }
            w.raw("</Metrics>");
            if !output.owning_accounts.is_empty() {
                w.raw("<OwningAccounts>");
                for acct in &output.owning_accounts {
                    w.write_element("member", acct);
                }
                w.raw("</OwningAccounts>");
            }
            w.write_optional_element("NextToken", output.next_token.as_deref());
            w.end_element("ListMetricsResult");
            w.write_response_metadata(&request_id);
            w.end_element("ListMetricsResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        // ---- Alarms ----
        CloudWatchOperation::PutMetricAlarm => {
            let input = deserialize_put_metric_alarm(&params)?;
            provider.put_metric_alarm(input)?;
            let mut w = XmlWriter::new();
            w.start_response("PutMetricAlarm");
            w.start_result("PutMetricAlarm");
            w.end_element("PutMetricAlarmResult");
            w.write_response_metadata(&request_id);
            w.end_element("PutMetricAlarmResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        CloudWatchOperation::DescribeAlarms => {
            let input = deserialize_describe_alarms(&params)?;
            let output = provider.describe_alarms(input)?;
            let mut w = XmlWriter::new();
            w.start_response("DescribeAlarms");
            w.start_result("DescribeAlarms");
            w.raw("<MetricAlarms>");
            for alarm in &output.metric_alarms {
                w.raw("<member>");
                write_metric_alarm_xml(&mut w, alarm);
                w.raw("</member>");
            }
            w.raw("</MetricAlarms>");
            w.raw("<CompositeAlarms>");
            for alarm in &output.composite_alarms {
                w.raw("<member>");
                write_composite_alarm_xml(&mut w, alarm);
                w.raw("</member>");
            }
            w.raw("</CompositeAlarms>");
            w.write_optional_element("NextToken", output.next_token.as_deref());
            w.end_element("DescribeAlarmsResult");
            w.write_response_metadata(&request_id);
            w.end_element("DescribeAlarmsResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        CloudWatchOperation::DescribeAlarmsForMetric => {
            let input = deserialize_describe_alarms_for_metric(&params)?;
            let output = provider.describe_alarms_for_metric(input)?;
            let mut w = XmlWriter::new();
            w.start_response("DescribeAlarmsForMetric");
            w.start_result("DescribeAlarmsForMetric");
            w.raw("<MetricAlarms>");
            for alarm in &output.metric_alarms {
                w.raw("<member>");
                write_metric_alarm_xml(&mut w, alarm);
                w.raw("</member>");
            }
            w.raw("</MetricAlarms>");
            w.end_element("DescribeAlarmsForMetricResult");
            w.write_response_metadata(&request_id);
            w.end_element("DescribeAlarmsForMetricResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        CloudWatchOperation::DeleteAlarms => {
            let input = deserialize_delete_alarms(&params);
            provider.delete_alarms(input)?;
            let mut w = XmlWriter::new();
            w.start_response("DeleteAlarms");
            w.start_result("DeleteAlarms");
            w.end_element("DeleteAlarmsResult");
            w.write_response_metadata(&request_id);
            w.end_element("DeleteAlarmsResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        CloudWatchOperation::SetAlarmState => {
            let input = deserialize_set_alarm_state(&params)?;
            provider.set_alarm_state(input)?;
            let mut w = XmlWriter::new();
            w.start_response("SetAlarmState");
            w.start_result("SetAlarmState");
            w.end_element("SetAlarmStateResult");
            w.write_response_metadata(&request_id);
            w.end_element("SetAlarmStateResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        CloudWatchOperation::EnableAlarmActions => {
            let input = deserialize_enable_alarm_actions(&params);
            provider.enable_alarm_actions(input)?;
            let mut w = XmlWriter::new();
            w.start_response("EnableAlarmActions");
            w.start_result("EnableAlarmActions");
            w.end_element("EnableAlarmActionsResult");
            w.write_response_metadata(&request_id);
            w.end_element("EnableAlarmActionsResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        CloudWatchOperation::DisableAlarmActions => {
            let input = deserialize_disable_alarm_actions(&params);
            provider.disable_alarm_actions(input)?;
            let mut w = XmlWriter::new();
            w.start_response("DisableAlarmActions");
            w.start_result("DisableAlarmActions");
            w.end_element("DisableAlarmActionsResult");
            w.write_response_metadata(&request_id);
            w.end_element("DisableAlarmActionsResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        CloudWatchOperation::DescribeAlarmHistory => {
            let input = deserialize_describe_alarm_history(&params)?;
            let output = provider.describe_alarm_history(input)?;
            let mut w = XmlWriter::new();
            w.start_response("DescribeAlarmHistory");
            w.start_result("DescribeAlarmHistory");
            w.raw("<AlarmHistoryItems>");
            for item in &output.alarm_history_items {
                w.raw("<member>");
                w.write_optional_element("AlarmName", item.alarm_name.as_deref());
                if let Some(at) = &item.alarm_type {
                    w.write_element("AlarmType", at.as_str());
                }
                if let Some(ts) = &item.timestamp {
                    w.write_element("Timestamp", &ts.format("%Y-%m-%dT%H:%M:%SZ").to_string());
                }
                if let Some(hit) = &item.history_item_type {
                    w.write_element("HistoryItemType", hit.as_str());
                }
                w.write_optional_element("HistorySummary", item.history_summary.as_deref());
                w.write_optional_element("HistoryData", item.history_data.as_deref());
                w.raw("</member>");
            }
            w.raw("</AlarmHistoryItems>");
            w.write_optional_element("NextToken", output.next_token.as_deref());
            w.end_element("DescribeAlarmHistoryResult");
            w.write_response_metadata(&request_id);
            w.end_element("DescribeAlarmHistoryResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        // ---- Composite Alarms ----
        CloudWatchOperation::PutCompositeAlarm => {
            let input = deserialize_put_composite_alarm(&params)?;
            provider.put_composite_alarm(input)?;
            let mut w = XmlWriter::new();
            w.start_response("PutCompositeAlarm");
            w.start_result("PutCompositeAlarm");
            w.end_element("PutCompositeAlarmResult");
            w.write_response_metadata(&request_id);
            w.end_element("PutCompositeAlarmResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        // ---- Tagging ----
        CloudWatchOperation::TagResource => {
            let input = deserialize_tag_resource(&params)?;
            let _output = provider.tag_resource(input)?;
            let mut w = XmlWriter::new();
            w.start_response("TagResource");
            w.start_result("TagResource");
            w.end_element("TagResourceResult");
            w.write_response_metadata(&request_id);
            w.end_element("TagResourceResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        CloudWatchOperation::UntagResource => {
            let input = deserialize_untag_resource(&params)?;
            let _output = provider.untag_resource(input)?;
            let mut w = XmlWriter::new();
            w.start_response("UntagResource");
            w.start_result("UntagResource");
            w.end_element("UntagResourceResult");
            w.write_response_metadata(&request_id);
            w.end_element("UntagResourceResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        CloudWatchOperation::ListTagsForResource => {
            let input = deserialize_list_tags_for_resource(&params)?;
            let output = provider.list_tags_for_resource(input)?;
            let mut w = XmlWriter::new();
            w.start_response("ListTagsForResource");
            w.start_result("ListTagsForResource");
            w.raw("<Tags>");
            for tag in &output.tags {
                w.raw("<member>");
                w.write_element("Key", &tag.key);
                w.write_element("Value", &tag.value);
                w.raw("</member>");
            }
            w.raw("</Tags>");
            w.end_element("ListTagsForResourceResult");
            w.write_response_metadata(&request_id);
            w.end_element("ListTagsForResourceResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        // ---- Dashboards ----
        CloudWatchOperation::PutDashboard => {
            let input = deserialize_put_dashboard(&params)?;
            let output = provider.put_dashboard(input)?;
            let mut w = XmlWriter::new();
            w.start_response("PutDashboard");
            w.start_result("PutDashboard");
            if !output.dashboard_validation_messages.is_empty() {
                w.raw("<DashboardValidationMessages>");
                for msg in &output.dashboard_validation_messages {
                    w.raw("<member>");
                    w.write_optional_element("DataPath", msg.data_path.as_deref());
                    w.write_optional_element("Message", msg.message.as_deref());
                    w.raw("</member>");
                }
                w.raw("</DashboardValidationMessages>");
            }
            w.end_element("PutDashboardResult");
            w.write_response_metadata(&request_id);
            w.end_element("PutDashboardResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        CloudWatchOperation::GetDashboard => {
            let input = deserialize_get_dashboard(&params)?;
            let output = provider.get_dashboard(input)?;
            let mut w = XmlWriter::new();
            w.start_response("GetDashboard");
            w.start_result("GetDashboard");
            w.write_optional_element("DashboardArn", output.dashboard_arn.as_deref());
            w.write_optional_element("DashboardBody", output.dashboard_body.as_deref());
            w.write_optional_element("DashboardName", output.dashboard_name.as_deref());
            w.end_element("GetDashboardResult");
            w.write_response_metadata(&request_id);
            w.end_element("GetDashboardResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        CloudWatchOperation::DeleteDashboards => {
            let input = deserialize_delete_dashboards(&params);
            let _output = provider.delete_dashboards(input)?;
            let mut w = XmlWriter::new();
            w.start_response("DeleteDashboards");
            w.start_result("DeleteDashboards");
            w.end_element("DeleteDashboardsResult");
            w.write_response_metadata(&request_id);
            w.end_element("DeleteDashboardsResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        CloudWatchOperation::ListDashboards => {
            let input = deserialize_list_dashboards(&params);
            let output = provider.list_dashboards(input)?;
            let mut w = XmlWriter::new();
            w.start_response("ListDashboards");
            w.start_result("ListDashboards");
            w.raw("<DashboardEntries>");
            for entry in &output.dashboard_entries {
                w.raw("<member>");
                w.write_optional_element("DashboardArn", entry.dashboard_arn.as_deref());
                w.write_optional_element("DashboardName", entry.dashboard_name.as_deref());
                if let Some(lm) = &entry.last_modified {
                    w.write_element("LastModified", &lm.format("%Y-%m-%dT%H:%M:%SZ").to_string());
                }
                if let Some(size) = entry.size {
                    w.write_element("Size", &size.to_string());
                }
                w.raw("</member>");
            }
            w.raw("</DashboardEntries>");
            w.write_optional_element("NextToken", output.next_token.as_deref());
            w.end_element("ListDashboardsResult");
            w.write_response_metadata(&request_id);
            w.end_element("ListDashboardsResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        // ---- Insight Rules ----
        CloudWatchOperation::PutInsightRule => {
            let input = deserialize_put_insight_rule(&params)?;
            let _output = provider.put_insight_rule(input)?;
            let mut w = XmlWriter::new();
            w.start_response("PutInsightRule");
            w.start_result("PutInsightRule");
            w.end_element("PutInsightRuleResult");
            w.write_response_metadata(&request_id);
            w.end_element("PutInsightRuleResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        CloudWatchOperation::DeleteInsightRules => {
            let input = deserialize_delete_insight_rules(&params);
            let output = provider.delete_insight_rules(input)?;
            let mut w = XmlWriter::new();
            w.start_response("DeleteInsightRules");
            w.start_result("DeleteInsightRules");
            write_partial_failures_xml(&mut w, &output.failures);
            w.end_element("DeleteInsightRulesResult");
            w.write_response_metadata(&request_id);
            w.end_element("DeleteInsightRulesResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        CloudWatchOperation::DescribeInsightRules => {
            let input = deserialize_describe_insight_rules(&params)?;
            let output = provider.describe_insight_rules(input)?;
            let mut w = XmlWriter::new();
            w.start_response("DescribeInsightRules");
            w.start_result("DescribeInsightRules");
            w.raw("<InsightRules>");
            for rule in &output.insight_rules {
                w.raw("<member>");
                w.write_element("Name", &rule.name);
                w.write_element("State", &rule.state);
                w.write_element("Schema", &rule.schema);
                w.write_element("Definition", &rule.definition);
                if let Some(managed) = rule.managed_rule {
                    w.write_bool_element("ManagedRule", managed);
                }
                if let Some(apply) = rule.apply_on_transformed_logs {
                    w.write_bool_element("ApplyOnTransformedLogs", apply);
                }
                w.raw("</member>");
            }
            w.raw("</InsightRules>");
            w.write_optional_element("NextToken", output.next_token.as_deref());
            w.end_element("DescribeInsightRulesResult");
            w.write_response_metadata(&request_id);
            w.end_element("DescribeInsightRulesResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        // ---- Anomaly Detectors ----
        CloudWatchOperation::PutAnomalyDetector => {
            let input = deserialize_put_anomaly_detector(&params)?;
            let _output = provider.put_anomaly_detector(input)?;
            let mut w = XmlWriter::new();
            w.start_response("PutAnomalyDetector");
            w.start_result("PutAnomalyDetector");
            w.end_element("PutAnomalyDetectorResult");
            w.write_response_metadata(&request_id);
            w.end_element("PutAnomalyDetectorResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        CloudWatchOperation::DescribeAnomalyDetectors => {
            let input = deserialize_describe_anomaly_detectors(&params)?;
            let output = provider.describe_anomaly_detectors(input)?;
            let mut w = XmlWriter::new();
            w.start_response("DescribeAnomalyDetectors");
            w.start_result("DescribeAnomalyDetectors");
            w.raw("<AnomalyDetectors>");
            for ad in &output.anomaly_detectors {
                w.raw("<member>");
                w.write_optional_element("Namespace", ad.namespace.as_deref());
                w.write_optional_element("MetricName", ad.metric_name.as_deref());
                w.write_optional_element("Stat", ad.stat.as_deref());
                if !ad.dimensions.is_empty() {
                    write_dimensions_xml(&mut w, &ad.dimensions);
                }
                if let Some(sv) = &ad.state_value {
                    w.write_element("StateValue", sv.as_str());
                }
                if let Some(smad) = &ad.single_metric_anomaly_detector {
                    write_single_metric_anomaly_detector_xml(&mut w, smad);
                }
                if let Some(mmad) = &ad.metric_math_anomaly_detector {
                    write_metric_math_anomaly_detector_xml(&mut w, mmad);
                }
                w.raw("</member>");
            }
            w.raw("</AnomalyDetectors>");
            w.write_optional_element("NextToken", output.next_token.as_deref());
            w.end_element("DescribeAnomalyDetectorsResult");
            w.write_response_metadata(&request_id);
            w.end_element("DescribeAnomalyDetectorsResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        CloudWatchOperation::DeleteAnomalyDetector => {
            let input = deserialize_delete_anomaly_detector(&params)?;
            let _output = provider.delete_anomaly_detector(input)?;
            let mut w = XmlWriter::new();
            w.start_response("DeleteAnomalyDetector");
            w.start_result("DeleteAnomalyDetector");
            w.end_element("DeleteAnomalyDetectorResult");
            w.write_response_metadata(&request_id);
            w.end_element("DeleteAnomalyDetectorResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        // ---- Managed Insight Rules ----
        CloudWatchOperation::PutManagedInsightRules => {
            let input = deserialize_put_managed_insight_rules(&params)?;
            let output = provider.put_managed_insight_rules(input)?;
            let mut w = XmlWriter::new();
            w.start_response("PutManagedInsightRules");
            w.start_result("PutManagedInsightRules");
            write_partial_failures_xml(&mut w, &output.failures);
            w.end_element("PutManagedInsightRulesResult");
            w.write_response_metadata(&request_id);
            w.end_element("PutManagedInsightRulesResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        // ---- Metric Streams ----
        CloudWatchOperation::PutMetricStream => {
            let input = deserialize_put_metric_stream(&params)?;
            let output = provider.put_metric_stream(input)?;
            let mut w = XmlWriter::new();
            w.start_response("PutMetricStream");
            w.start_result("PutMetricStream");
            w.write_optional_element("Arn", output.arn.as_deref());
            w.end_element("PutMetricStreamResult");
            w.write_response_metadata(&request_id);
            w.end_element("PutMetricStreamResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        CloudWatchOperation::DeleteMetricStream => {
            let input = deserialize_delete_metric_stream(&params)?;
            let _output = provider.delete_metric_stream(input)?;
            let mut w = XmlWriter::new();
            w.start_response("DeleteMetricStream");
            w.start_result("DeleteMetricStream");
            w.end_element("DeleteMetricStreamResult");
            w.write_response_metadata(&request_id);
            w.end_element("DeleteMetricStreamResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        CloudWatchOperation::ListMetricStreams => {
            let input = deserialize_list_metric_streams(&params)?;
            let output = provider.list_metric_streams(input)?;
            let mut w = XmlWriter::new();
            w.start_response("ListMetricStreams");
            w.start_result("ListMetricStreams");
            w.raw("<Entries>");
            for entry in &output.entries {
                w.raw("<member>");
                w.write_optional_element("Arn", entry.arn.as_deref());
                w.write_optional_element("Name", entry.name.as_deref());
                w.write_optional_element("FirehoseArn", entry.firehose_arn.as_deref());
                w.write_optional_element("State", entry.state.as_deref());
                if let Some(fmt) = &entry.output_format {
                    w.write_element("OutputFormat", fmt.as_str());
                }
                if let Some(cd) = &entry.creation_date {
                    w.write_element("CreationDate", &cd.format("%Y-%m-%dT%H:%M:%SZ").to_string());
                }
                if let Some(lu) = &entry.last_update_date {
                    w.write_element(
                        "LastUpdateDate",
                        &lu.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                    );
                }
                w.raw("</member>");
            }
            w.raw("</Entries>");
            w.write_optional_element("NextToken", output.next_token.as_deref());
            w.end_element("ListMetricStreamsResult");
            w.write_response_metadata(&request_id);
            w.end_element("ListMetricStreamsResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        CloudWatchOperation::GetMetricStream => {
            let input = deserialize_get_metric_stream(&params)?;
            let output = provider.get_metric_stream(input)?;
            let mut w = XmlWriter::new();
            w.start_response("GetMetricStream");
            w.start_result("GetMetricStream");
            w.write_optional_element("Arn", output.arn.as_deref());
            w.write_optional_element("Name", output.name.as_deref());
            w.write_optional_element("FirehoseArn", output.firehose_arn.as_deref());
            w.write_optional_element("RoleArn", output.role_arn.as_deref());
            w.write_optional_element("State", output.state.as_deref());
            if let Some(fmt) = &output.output_format {
                w.write_element("OutputFormat", fmt.as_str());
            }
            if let Some(ila) = output.include_linked_accounts_metrics {
                w.write_bool_element("IncludeLinkedAccountsMetrics", ila);
            }
            if let Some(cd) = &output.creation_date {
                w.write_element("CreationDate", &cd.format("%Y-%m-%dT%H:%M:%SZ").to_string());
            }
            if let Some(lu) = &output.last_update_date {
                w.write_element(
                    "LastUpdateDate",
                    &lu.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                );
            }
            write_metric_stream_filters_xml(&mut w, "IncludeFilters", &output.include_filters);
            write_metric_stream_filters_xml(&mut w, "ExcludeFilters", &output.exclude_filters);
            if !output.statistics_configurations.is_empty() {
                write_statistics_configurations_xml(&mut w, &output.statistics_configurations);
            }
            w.end_element("GetMetricStreamResult");
            w.write_response_metadata(&request_id);
            w.end_element("GetMetricStreamResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }
    }
}

// ---------------------------------------------------------------------------
// XML helpers
// ---------------------------------------------------------------------------

/// Write `<Dimensions>` block with `<member>` elements.
fn write_dimensions_xml(w: &mut XmlWriter, dims: &[Dimension]) {
    w.raw("<Dimensions>");
    for dim in dims {
        w.raw("<member>");
        w.write_element("Name", &dim.name);
        w.write_element("Value", &dim.value);
        w.raw("</member>");
    }
    w.raw("</Dimensions>");
}

/// Write all fields of a `MetricAlarm` as XML elements.
fn write_metric_alarm_xml(w: &mut XmlWriter, alarm: &MetricAlarm) {
    w.write_optional_element("AlarmName", alarm.alarm_name.as_deref());
    w.write_optional_element("AlarmArn", alarm.alarm_arn.as_deref());
    w.write_optional_element("AlarmDescription", alarm.alarm_description.as_deref());
    if let Some(ts) = &alarm.alarm_configuration_updated_timestamp {
        w.write_element(
            "AlarmConfigurationUpdatedTimestamp",
            &ts.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        );
    }
    if let Some(ae) = alarm.actions_enabled {
        w.write_bool_element("ActionsEnabled", ae);
    }
    if !alarm.ok_actions.is_empty() {
        w.raw("<OKActions>");
        for a in &alarm.ok_actions {
            w.write_element("member", a);
        }
        w.raw("</OKActions>");
    }
    if !alarm.alarm_actions.is_empty() {
        w.raw("<AlarmActions>");
        for a in &alarm.alarm_actions {
            w.write_element("member", a);
        }
        w.raw("</AlarmActions>");
    }
    if !alarm.insufficient_data_actions.is_empty() {
        w.raw("<InsufficientDataActions>");
        for a in &alarm.insufficient_data_actions {
            w.write_element("member", a);
        }
        w.raw("</InsufficientDataActions>");
    }
    if let Some(sv) = &alarm.state_value {
        w.write_element("StateValue", sv.as_str());
    }
    w.write_optional_element("StateReason", alarm.state_reason.as_deref());
    w.write_optional_element("StateReasonData", alarm.state_reason_data.as_deref());
    if let Some(ts) = &alarm.state_updated_timestamp {
        w.write_element(
            "StateUpdatedTimestamp",
            &ts.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        );
    }
    if let Some(ts) = &alarm.state_transitioned_timestamp {
        w.write_element(
            "StateTransitionedTimestamp",
            &ts.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        );
    }
    w.write_optional_element("MetricName", alarm.metric_name.as_deref());
    w.write_optional_element("Namespace", alarm.namespace.as_deref());
    if let Some(stat) = &alarm.statistic {
        w.write_element("Statistic", stat.as_str());
    }
    w.write_optional_element("ExtendedStatistic", alarm.extended_statistic.as_deref());
    if !alarm.dimensions.is_empty() {
        write_dimensions_xml(w, &alarm.dimensions);
    }
    if let Some(p) = alarm.period {
        w.write_i32_element("Period", p);
    }
    if let Some(u) = &alarm.unit {
        w.write_element("Unit", u.as_str());
    }
    if let Some(ep) = alarm.evaluation_periods {
        w.write_i32_element("EvaluationPeriods", ep);
    }
    if let Some(dp) = alarm.datapoints_to_alarm {
        w.write_i32_element("DatapointsToAlarm", dp);
    }
    w.write_optional_f64_element("Threshold", alarm.threshold);
    if let Some(co) = &alarm.comparison_operator {
        w.write_element("ComparisonOperator", co.as_str());
    }
    w.write_optional_element("TreatMissingData", alarm.treat_missing_data.as_deref());
    w.write_optional_element(
        "EvaluateLowSampleCountPercentile",
        alarm.evaluate_low_sample_count_percentile.as_deref(),
    );
    w.write_optional_element("ThresholdMetricId", alarm.threshold_metric_id.as_deref());
    if let Some(es) = &alarm.evaluation_state {
        w.write_element("EvaluationState", es.as_str());
    }
    if !alarm.metrics.is_empty() {
        w.raw("<Metrics>");
        for mdq in &alarm.metrics {
            w.raw("<member>");
            write_metric_data_query_xml(w, mdq);
            w.raw("</member>");
        }
        w.raw("</Metrics>");
    }
}

/// Write all fields of a `CompositeAlarm` as XML elements.
fn write_composite_alarm_xml(w: &mut XmlWriter, alarm: &CompositeAlarm) {
    w.write_optional_element("AlarmName", alarm.alarm_name.as_deref());
    w.write_optional_element("AlarmArn", alarm.alarm_arn.as_deref());
    w.write_optional_element("AlarmDescription", alarm.alarm_description.as_deref());
    w.write_optional_element("AlarmRule", alarm.alarm_rule.as_deref());
    if let Some(ts) = &alarm.alarm_configuration_updated_timestamp {
        w.write_element(
            "AlarmConfigurationUpdatedTimestamp",
            &ts.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        );
    }
    if let Some(ae) = alarm.actions_enabled {
        w.write_bool_element("ActionsEnabled", ae);
    }
    if !alarm.ok_actions.is_empty() {
        w.raw("<OKActions>");
        for a in &alarm.ok_actions {
            w.write_element("member", a);
        }
        w.raw("</OKActions>");
    }
    if !alarm.alarm_actions.is_empty() {
        w.raw("<AlarmActions>");
        for a in &alarm.alarm_actions {
            w.write_element("member", a);
        }
        w.raw("</AlarmActions>");
    }
    if !alarm.insufficient_data_actions.is_empty() {
        w.raw("<InsufficientDataActions>");
        for a in &alarm.insufficient_data_actions {
            w.write_element("member", a);
        }
        w.raw("</InsufficientDataActions>");
    }
    if let Some(sv) = &alarm.state_value {
        w.write_element("StateValue", sv.as_str());
    }
    w.write_optional_element("StateReason", alarm.state_reason.as_deref());
    w.write_optional_element("StateReasonData", alarm.state_reason_data.as_deref());
    if let Some(ts) = &alarm.state_updated_timestamp {
        w.write_element(
            "StateUpdatedTimestamp",
            &ts.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        );
    }
    if let Some(ts) = &alarm.state_transitioned_timestamp {
        w.write_element(
            "StateTransitionedTimestamp",
            &ts.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        );
    }
    w.write_optional_element("ActionsSuppressor", alarm.actions_suppressor.as_deref());
    if let Some(v) = alarm.actions_suppressor_wait_period {
        w.write_i32_element("ActionsSuppressorWaitPeriod", v);
    }
    if let Some(v) = alarm.actions_suppressor_extension_period {
        w.write_i32_element("ActionsSuppressorExtensionPeriod", v);
    }
    if let Some(sb) = &alarm.actions_suppressed_by {
        w.write_element("ActionsSuppressedBy", sb.as_str());
    }
    w.write_optional_element(
        "ActionsSuppressedReason",
        alarm.actions_suppressed_reason.as_deref(),
    );
}

/// Write a `MetricDataQuery` as XML.
fn write_metric_data_query_xml(w: &mut XmlWriter, mdq: &MetricDataQuery) {
    w.write_element("Id", &mdq.id);
    w.write_optional_element("Expression", mdq.expression.as_deref());
    w.write_optional_element("Label", mdq.label.as_deref());
    w.write_optional_element("AccountId", mdq.account_id.as_deref());
    if let Some(rd) = mdq.return_data {
        w.write_bool_element("ReturnData", rd);
    }
    if let Some(p) = mdq.period {
        w.write_i32_element("Period", p);
    }
    if let Some(ms) = &mdq.metric_stat {
        w.raw("<MetricStat>");
        w.raw("<Metric>");
        w.write_optional_element("Namespace", ms.metric.namespace.as_deref());
        w.write_optional_element("MetricName", ms.metric.metric_name.as_deref());
        if !ms.metric.dimensions.is_empty() {
            write_dimensions_xml(w, &ms.metric.dimensions);
        }
        w.raw("</Metric>");
        w.write_i32_element("Period", ms.period);
        w.write_element("Stat", &ms.stat);
        if let Some(u) = &ms.unit {
            w.write_element("Unit", u.as_str());
        }
        w.raw("</MetricStat>");
    }
}

/// Write `<Failures>` block from partial failures.
fn write_partial_failures_xml(
    w: &mut XmlWriter,
    failures: &[ruststack_cloudwatch_model::types::PartialFailure],
) {
    if !failures.is_empty() {
        w.raw("<Failures>");
        for f in failures {
            w.raw("<member>");
            w.write_optional_element("ExceptionType", f.exception_type.as_deref());
            w.write_optional_element("FailureCode", f.failure_code.as_deref());
            w.write_optional_element("FailureDescription", f.failure_description.as_deref());
            w.write_optional_element("FailureResource", f.failure_resource.as_deref());
            w.raw("</member>");
        }
        w.raw("</Failures>");
    }
}

/// Write `SingleMetricAnomalyDetector` XML block.
fn write_single_metric_anomaly_detector_xml(w: &mut XmlWriter, smad: &SingleMetricAnomalyDetector) {
    w.raw("<SingleMetricAnomalyDetector>");
    w.write_optional_element("AccountId", smad.account_id.as_deref());
    w.write_optional_element("Namespace", smad.namespace.as_deref());
    w.write_optional_element("MetricName", smad.metric_name.as_deref());
    w.write_optional_element("Stat", smad.stat.as_deref());
    if !smad.dimensions.is_empty() {
        write_dimensions_xml(w, &smad.dimensions);
    }
    w.raw("</SingleMetricAnomalyDetector>");
}

/// Write `MetricMathAnomalyDetector` XML block.
fn write_metric_math_anomaly_detector_xml(w: &mut XmlWriter, mmad: &MetricMathAnomalyDetector) {
    w.raw("<MetricMathAnomalyDetector>");
    if !mmad.metric_data_queries.is_empty() {
        w.raw("<MetricDataQueries>");
        for mdq in &mmad.metric_data_queries {
            w.raw("<member>");
            write_metric_data_query_xml(w, mdq);
            w.raw("</member>");
        }
        w.raw("</MetricDataQueries>");
    }
    w.raw("</MetricMathAnomalyDetector>");
}

/// Write metric stream filters XML block.
fn write_metric_stream_filters_xml(
    w: &mut XmlWriter,
    element_name: &str,
    filters: &[MetricStreamFilter],
) {
    if !filters.is_empty() {
        w.start_element(element_name);
        for f in filters {
            w.raw("<member>");
            w.write_optional_element("Namespace", f.namespace.as_deref());
            if !f.metric_names.is_empty() {
                w.raw("<MetricNames>");
                for mn in &f.metric_names {
                    w.write_element("member", mn);
                }
                w.raw("</MetricNames>");
            }
            w.raw("</member>");
        }
        w.end_element(element_name);
    }
}

/// Write statistics configurations XML block.
fn write_statistics_configurations_xml(
    w: &mut XmlWriter,
    configs: &[MetricStreamStatisticsConfiguration],
) {
    w.raw("<StatisticsConfigurations>");
    for cfg in configs {
        w.raw("<member>");
        if !cfg.additional_statistics.is_empty() {
            w.raw("<AdditionalStatistics>");
            for s in &cfg.additional_statistics {
                w.write_element("member", s);
            }
            w.raw("</AdditionalStatistics>");
        }
        if !cfg.include_metrics.is_empty() {
            w.raw("<IncludeMetrics>");
            for m in &cfg.include_metrics {
                w.raw("<member>");
                w.write_element("Namespace", &m.namespace);
                w.write_element("MetricName", &m.metric_name);
                w.raw("</member>");
            }
            w.raw("</IncludeMetrics>");
        }
        w.raw("</member>");
    }
    w.raw("</StatisticsConfigurations>");
}

// ---------------------------------------------------------------------------
// Timestamp parsing helper
// ---------------------------------------------------------------------------

/// Parse a timestamp string from form params.
///
/// Supports ISO 8601 / RFC 3339 formats commonly sent by AWS clients.
fn parse_timestamp(s: &str) -> Result<chrono::DateTime<chrono::Utc>, CloudWatchError> {
    // Try RFC 3339 first (e.g. 2026-03-19T12:00:00Z or with offset).
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&chrono::Utc));
    }
    // Try without timezone suffix (assume UTC).
    if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return Ok(ndt.and_utc());
    }
    // Try with fractional seconds.
    if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f") {
        return Ok(ndt.and_utc());
    }
    // Try epoch seconds.
    if let Ok(epoch) = s.parse::<f64>() {
        #[allow(clippy::cast_possible_truncation)]
        let secs = epoch as i64;
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let nanos = ((epoch - secs as f64) * 1_000_000_000.0) as u32;
        if let Some(dt) = chrono::DateTime::from_timestamp(secs, nanos) {
            return Ok(dt);
        }
    }
    Err(CloudWatchError::with_message(
        CloudWatchErrorCode::InvalidParameterValueException,
        format!("Invalid timestamp format: {s}"),
    ))
}

/// Get a required timestamp param.
fn get_required_timestamp(
    params: &[(String, String)],
    key: &str,
) -> Result<chrono::DateTime<chrono::Utc>, CloudWatchError> {
    let s = get_required_param(params, key)?;
    parse_timestamp(s)
}

/// Get an optional timestamp param.
fn get_optional_timestamp(
    params: &[(String, String)],
    key: &str,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, CloudWatchError> {
    match get_optional_param(params, key) {
        Some(s) => parse_timestamp(s).map(Some),
        None => Ok(None),
    }
}

/// Get a required i32 param.
fn get_required_i32(params: &[(String, String)], key: &str) -> Result<i32, CloudWatchError> {
    let s = get_required_param(params, key)?;
    s.parse::<i32>().map_err(|_| {
        CloudWatchError::with_message(
            CloudWatchErrorCode::InvalidParameterValueException,
            format!("Invalid integer value for {key}: {s}"),
        )
    })
}

// ---------------------------------------------------------------------------
// Deserializers: form params -> input structs
// ---------------------------------------------------------------------------

fn deserialize_put_metric_data(
    params: &[(String, String)],
) -> Result<PutMetricDataInput, CloudWatchError> {
    let namespace = get_required_param(params, "Namespace")?.to_owned();
    let strict_entity_validation = get_optional_bool(params, "StrictEntityValidation");
    let metric_data = parse_metric_datum_list(params, "MetricData")?;

    Ok(PutMetricDataInput {
        namespace,
        metric_data,
        strict_entity_validation,
        entity_metric_data: Vec::new(),
    })
}

fn parse_metric_datum_list(
    params: &[(String, String)],
    prefix: &str,
) -> Result<Vec<MetricDatum>, CloudWatchError> {
    let struct_list = parse_struct_list(params, prefix);
    let mut data = Vec::with_capacity(struct_list.len());

    for sub_params in &struct_list {
        let metric_name = get_required_param(sub_params, "MetricName")?.to_owned();
        let value = get_optional_f64(sub_params, "Value")?;
        let unit = get_optional_param(sub_params, "Unit").map(StandardUnit::from);
        let timestamp = get_optional_timestamp(sub_params, "Timestamp")?;
        let storage_resolution = get_optional_i32(sub_params, "StorageResolution")?;

        let dims_raw = parse_dimensions(sub_params, "Dimensions");
        let dimensions: Vec<Dimension> = dims_raw
            .into_iter()
            .map(|(n, v)| Dimension { name: n, value: v })
            .collect();

        let statistic_values = parse_statistic_set(sub_params)?;

        let values = parse_f64_list(sub_params, "Values")?;
        let counts = parse_f64_list(sub_params, "Counts")?;

        data.push(MetricDatum {
            metric_name,
            value,
            unit,
            timestamp,
            storage_resolution,
            dimensions,
            statistic_values,
            values,
            counts,
        });
    }

    Ok(data)
}

fn parse_statistic_set(
    params: &[(String, String)],
) -> Result<Option<StatisticSet>, CloudWatchError> {
    let sample_count = get_optional_f64(params, "StatisticValues.SampleCount")?;
    let sum = get_optional_f64(params, "StatisticValues.Sum")?;
    let minimum = get_optional_f64(params, "StatisticValues.Minimum")?;
    let maximum = get_optional_f64(params, "StatisticValues.Maximum")?;

    match (sample_count, sum, minimum, maximum) {
        (Some(sc), Some(s), Some(min), Some(max)) => Ok(Some(StatisticSet {
            sample_count: sc,
            sum: s,
            minimum: min,
            maximum: max,
        })),
        (None, None, None, None) => Ok(None),
        _ => Err(CloudWatchError::with_message(
            CloudWatchErrorCode::MissingRequiredParameterException,
            "StatisticValues requires all four fields: SampleCount, Sum, Minimum, Maximum",
        )),
    }
}

fn parse_f64_list(params: &[(String, String)], prefix: &str) -> Result<Vec<f64>, CloudWatchError> {
    let strings = parse_string_list(params, prefix);
    strings
        .iter()
        .map(|s| {
            s.parse::<f64>().map_err(|_| {
                CloudWatchError::with_message(
                    CloudWatchErrorCode::InvalidParameterValueException,
                    format!("Invalid numeric value '{s}' in {prefix}"),
                )
            })
        })
        .collect()
}

fn deserialize_get_metric_statistics(
    params: &[(String, String)],
) -> Result<GetMetricStatisticsInput, CloudWatchError> {
    let namespace = get_required_param(params, "Namespace")?.to_owned();
    let metric_name = get_required_param(params, "MetricName")?.to_owned();
    let start_time = get_required_timestamp(params, "StartTime")?;
    let end_time = get_required_timestamp(params, "EndTime")?;
    let period = get_required_i32(params, "Period")?;

    let statistics: Vec<Statistic> = parse_string_list(params, "Statistics")
        .iter()
        .map(|s| Statistic::from(s.as_str()))
        .collect();

    let extended_statistics = parse_string_list(params, "ExtendedStatistics");

    let dims_raw = parse_dimensions(params, "Dimensions");
    let dimensions: Vec<Dimension> = dims_raw
        .into_iter()
        .map(|(n, v)| Dimension { name: n, value: v })
        .collect();

    let unit = get_optional_param(params, "Unit").map(StandardUnit::from);

    Ok(GetMetricStatisticsInput {
        namespace,
        metric_name,
        start_time,
        end_time,
        period,
        statistics,
        extended_statistics,
        dimensions,
        unit,
    })
}

fn deserialize_get_metric_data(
    params: &[(String, String)],
) -> Result<GetMetricDataInput, CloudWatchError> {
    let start_time = get_required_timestamp(params, "StartTime")?;
    let end_time = get_required_timestamp(params, "EndTime")?;
    let next_token = get_optional_param(params, "NextToken").map(str::to_owned);
    let max_datapoints = get_optional_i32(params, "MaxDatapoints")?;
    let scan_by = get_optional_param(params, "ScanBy").map(ScanBy::from);

    let label_options =
        get_optional_param(params, "LabelOptions.Timezone").map(|tz| LabelOptions {
            timezone: Some(tz.to_owned()),
        });

    let metric_data_queries = parse_metric_data_queries(params, "MetricDataQueries")?;

    Ok(GetMetricDataInput {
        start_time,
        end_time,
        next_token,
        max_datapoints,
        scan_by,
        label_options,
        metric_data_queries,
    })
}

fn parse_metric_data_queries(
    params: &[(String, String)],
    prefix: &str,
) -> Result<Vec<MetricDataQuery>, CloudWatchError> {
    let struct_list = parse_struct_list(params, prefix);
    let mut queries = Vec::with_capacity(struct_list.len());

    for sub_params in &struct_list {
        let id = get_required_param(sub_params, "Id")?.to_owned();
        let expression = get_optional_param(sub_params, "Expression").map(str::to_owned);
        let label = get_optional_param(sub_params, "Label").map(str::to_owned);
        let account_id = get_optional_param(sub_params, "AccountId").map(str::to_owned);
        let return_data = get_optional_bool(sub_params, "ReturnData");
        let period = get_optional_i32(sub_params, "Period")?;

        let metric_stat = parse_metric_stat(sub_params)?;

        queries.push(MetricDataQuery {
            id,
            expression,
            label,
            account_id,
            return_data,
            period,
            metric_stat,
        });
    }

    Ok(queries)
}

fn parse_metric_stat(params: &[(String, String)]) -> Result<Option<MetricStat>, CloudWatchError> {
    let stat = get_optional_param(params, "MetricStat.Stat");
    let period = get_optional_i32(params, "MetricStat.Period")?;

    match (stat, period) {
        (Some(stat_val), Some(period_val)) => {
            let namespace =
                get_optional_param(params, "MetricStat.Metric.Namespace").map(str::to_owned);
            let metric_name =
                get_optional_param(params, "MetricStat.Metric.MetricName").map(str::to_owned);
            let unit = get_optional_param(params, "MetricStat.Unit").map(StandardUnit::from);

            // Build sub-params for dimensions under MetricStat.Metric.Dimensions
            let dim_prefix = "MetricStat.Metric.Dimensions";
            let dims_raw = parse_dimensions(params, dim_prefix);
            let dimensions: Vec<Dimension> = dims_raw
                .into_iter()
                .map(|(n, v)| Dimension { name: n, value: v })
                .collect();

            Ok(Some(MetricStat {
                metric: ruststack_cloudwatch_model::types::Metric {
                    namespace,
                    metric_name,
                    dimensions,
                },
                period: period_val,
                stat: stat_val.to_owned(),
                unit,
            }))
        }
        _ => Ok(None),
    }
}

fn deserialize_list_metrics(params: &[(String, String)]) -> ListMetricsInput {
    let namespace = get_optional_param(params, "Namespace").map(str::to_owned);
    let metric_name = get_optional_param(params, "MetricName").map(str::to_owned);
    let next_token = get_optional_param(params, "NextToken").map(str::to_owned);
    let include_linked_accounts = get_optional_bool(params, "IncludeLinkedAccounts");
    let owning_account = get_optional_param(params, "OwningAccount").map(str::to_owned);
    let recently_active = get_optional_param(params, "RecentlyActive").map(RecentlyActive::from);

    let dim_filters_raw = parse_dimension_filters(params, "Dimensions");
    let dimensions: Vec<DimensionFilter> = dim_filters_raw
        .into_iter()
        .map(|(n, v)| DimensionFilter { name: n, value: v })
        .collect();

    ListMetricsInput {
        namespace,
        metric_name,
        next_token,
        include_linked_accounts,
        owning_account,
        recently_active,
        dimensions,
    }
}

fn deserialize_put_metric_alarm(
    params: &[(String, String)],
) -> Result<PutMetricAlarmInput, CloudWatchError> {
    let alarm_name = get_required_param(params, "AlarmName")?.to_owned();
    let comparison_operator_str = get_required_param(params, "ComparisonOperator")?;
    let comparison_operator = ComparisonOperator::from(comparison_operator_str);
    let evaluation_periods = get_required_i32(params, "EvaluationPeriods")?;

    let alarm_description = get_optional_param(params, "AlarmDescription").map(str::to_owned);
    let actions_enabled = get_optional_bool(params, "ActionsEnabled");
    let metric_name = get_optional_param(params, "MetricName").map(str::to_owned);
    let namespace = get_optional_param(params, "Namespace").map(str::to_owned);
    let statistic = get_optional_param(params, "Statistic").map(Statistic::from);
    let extended_statistic = get_optional_param(params, "ExtendedStatistic").map(str::to_owned);
    let period = get_optional_i32(params, "Period")?;
    let unit = get_optional_param(params, "Unit").map(StandardUnit::from);
    let datapoints_to_alarm = get_optional_i32(params, "DatapointsToAlarm")?;
    let threshold = get_optional_f64(params, "Threshold")?;
    let threshold_metric_id = get_optional_param(params, "ThresholdMetricId").map(str::to_owned);
    let treat_missing_data = get_optional_param(params, "TreatMissingData").map(str::to_owned);
    let evaluate_low_sample_count_percentile =
        get_optional_param(params, "EvaluateLowSampleCountPercentile").map(str::to_owned);

    let alarm_actions = parse_string_list(params, "AlarmActions");
    let ok_actions = parse_string_list(params, "OKActions");
    let insufficient_data_actions = parse_string_list(params, "InsufficientDataActions");

    let dims_raw = parse_dimensions(params, "Dimensions");
    let dimensions: Vec<Dimension> = dims_raw
        .into_iter()
        .map(|(n, v)| Dimension { name: n, value: v })
        .collect();

    let metrics = parse_metric_data_queries(params, "Metrics")?;

    let tags_raw = parse_tag_list(params, "Tags")?;
    let tags: Vec<Tag> = tags_raw
        .into_iter()
        .map(|(k, v)| Tag { key: k, value: v })
        .collect();

    Ok(PutMetricAlarmInput {
        alarm_name,
        comparison_operator,
        evaluation_periods,
        alarm_description,
        actions_enabled,
        metric_name,
        namespace,
        statistic,
        extended_statistic,
        period,
        unit,
        datapoints_to_alarm,
        threshold,
        threshold_metric_id,
        treat_missing_data,
        evaluate_low_sample_count_percentile,
        alarm_actions,
        ok_actions,
        insufficient_data_actions,
        dimensions,
        metrics,
        tags,
    })
}

fn deserialize_describe_alarms(
    params: &[(String, String)],
) -> Result<DescribeAlarmsInput, CloudWatchError> {
    let alarm_names = parse_string_list(params, "AlarmNames");
    let alarm_name_prefix = get_optional_param(params, "AlarmNamePrefix").map(str::to_owned);
    let alarm_types: Vec<AlarmType> = parse_string_list(params, "AlarmTypes")
        .iter()
        .map(|s| AlarmType::from(s.as_str()))
        .collect();
    let children_of_alarm_name =
        get_optional_param(params, "ChildrenOfAlarmName").map(str::to_owned);
    let parents_of_alarm_name = get_optional_param(params, "ParentsOfAlarmName").map(str::to_owned);
    let state_value = get_optional_param(params, "StateValue").map(StateValue::from);
    let action_prefix = get_optional_param(params, "ActionPrefix").map(str::to_owned);
    let max_records = get_optional_i32(params, "MaxRecords")?;
    let next_token = get_optional_param(params, "NextToken").map(str::to_owned);

    Ok(DescribeAlarmsInput {
        alarm_names,
        alarm_name_prefix,
        alarm_types,
        children_of_alarm_name,
        parents_of_alarm_name,
        state_value,
        action_prefix,
        max_records,
        next_token,
    })
}

fn deserialize_describe_alarms_for_metric(
    params: &[(String, String)],
) -> Result<DescribeAlarmsForMetricInput, CloudWatchError> {
    let metric_name = get_required_param(params, "MetricName")?.to_owned();
    let namespace = get_required_param(params, "Namespace")?.to_owned();
    let statistic = get_optional_param(params, "Statistic").map(Statistic::from);
    let extended_statistic = get_optional_param(params, "ExtendedStatistic").map(str::to_owned);
    let period = get_optional_i32(params, "Period")?;
    let unit = get_optional_param(params, "Unit").map(StandardUnit::from);

    let dims_raw = parse_dimensions(params, "Dimensions");
    let dimensions: Vec<Dimension> = dims_raw
        .into_iter()
        .map(|(n, v)| Dimension { name: n, value: v })
        .collect();

    Ok(DescribeAlarmsForMetricInput {
        metric_name,
        namespace,
        statistic,
        extended_statistic,
        period,
        unit,
        dimensions,
    })
}

fn deserialize_delete_alarms(params: &[(String, String)]) -> DeleteAlarmsInput {
    let alarm_names = parse_string_list(params, "AlarmNames");
    DeleteAlarmsInput { alarm_names }
}

fn deserialize_set_alarm_state(
    params: &[(String, String)],
) -> Result<SetAlarmStateInput, CloudWatchError> {
    let alarm_name = get_required_param(params, "AlarmName")?.to_owned();
    let state_value_str = get_required_param(params, "StateValue")?;
    let state_value = StateValue::from(state_value_str);
    let state_reason = get_required_param(params, "StateReason")?.to_owned();
    let state_reason_data = get_optional_param(params, "StateReasonData").map(str::to_owned);

    Ok(SetAlarmStateInput {
        alarm_name,
        state_value,
        state_reason,
        state_reason_data,
    })
}

fn deserialize_enable_alarm_actions(params: &[(String, String)]) -> EnableAlarmActionsInput {
    let alarm_names = parse_string_list(params, "AlarmNames");
    EnableAlarmActionsInput { alarm_names }
}

fn deserialize_disable_alarm_actions(params: &[(String, String)]) -> DisableAlarmActionsInput {
    let alarm_names = parse_string_list(params, "AlarmNames");
    DisableAlarmActionsInput { alarm_names }
}

fn deserialize_describe_alarm_history(
    params: &[(String, String)],
) -> Result<DescribeAlarmHistoryInput, CloudWatchError> {
    let alarm_name = get_optional_param(params, "AlarmName").map(str::to_owned);
    let alarm_contributor_id = get_optional_param(params, "AlarmContributorId").map(str::to_owned);
    let alarm_types: Vec<AlarmType> = parse_string_list(params, "AlarmTypes")
        .iter()
        .map(|s| AlarmType::from(s.as_str()))
        .collect();
    let history_item_type =
        get_optional_param(params, "HistoryItemType").map(HistoryItemType::from);
    let start_date = get_optional_timestamp(params, "StartDate")?;
    let end_date = get_optional_timestamp(params, "EndDate")?;
    let max_records = get_optional_i32(params, "MaxRecords")?;
    let next_token = get_optional_param(params, "NextToken").map(str::to_owned);
    let scan_by = get_optional_param(params, "ScanBy").map(ScanBy::from);

    Ok(DescribeAlarmHistoryInput {
        alarm_name,
        alarm_contributor_id,
        alarm_types,
        history_item_type,
        start_date,
        end_date,
        max_records,
        next_token,
        scan_by,
    })
}

fn deserialize_put_composite_alarm(
    params: &[(String, String)],
) -> Result<PutCompositeAlarmInput, CloudWatchError> {
    let alarm_name = get_required_param(params, "AlarmName")?.to_owned();
    let alarm_rule = get_required_param(params, "AlarmRule")?.to_owned();
    let alarm_description = get_optional_param(params, "AlarmDescription").map(str::to_owned);
    let actions_enabled = get_optional_bool(params, "ActionsEnabled");
    let actions_suppressor = get_optional_param(params, "ActionsSuppressor").map(str::to_owned);
    let actions_suppressor_wait_period = get_optional_i32(params, "ActionsSuppressorWaitPeriod")?;
    let actions_suppressor_extension_period =
        get_optional_i32(params, "ActionsSuppressorExtensionPeriod")?;

    let alarm_actions = parse_string_list(params, "AlarmActions");
    let ok_actions = parse_string_list(params, "OKActions");
    let insufficient_data_actions = parse_string_list(params, "InsufficientDataActions");

    let tags_raw = parse_tag_list(params, "Tags")?;
    let tags: Vec<Tag> = tags_raw
        .into_iter()
        .map(|(k, v)| Tag { key: k, value: v })
        .collect();

    Ok(PutCompositeAlarmInput {
        alarm_name,
        alarm_rule,
        alarm_description,
        actions_enabled,
        actions_suppressor,
        actions_suppressor_wait_period,
        actions_suppressor_extension_period,
        alarm_actions,
        ok_actions,
        insufficient_data_actions,
        tags,
    })
}

fn deserialize_tag_resource(
    params: &[(String, String)],
) -> Result<TagResourceInput, CloudWatchError> {
    let resource_arn = get_required_param(params, "ResourceARN")?.to_owned();
    let tags_raw = parse_tag_list(params, "Tags")?;
    let tags: Vec<Tag> = tags_raw
        .into_iter()
        .map(|(k, v)| Tag { key: k, value: v })
        .collect();
    Ok(TagResourceInput { resource_arn, tags })
}

fn deserialize_untag_resource(
    params: &[(String, String)],
) -> Result<UntagResourceInput, CloudWatchError> {
    let resource_arn = get_required_param(params, "ResourceARN")?.to_owned();
    let tag_keys = parse_string_list(params, "TagKeys");
    Ok(UntagResourceInput {
        resource_arn,
        tag_keys,
    })
}

fn deserialize_list_tags_for_resource(
    params: &[(String, String)],
) -> Result<ListTagsForResourceInput, CloudWatchError> {
    let resource_arn = get_required_param(params, "ResourceARN")?.to_owned();
    Ok(ListTagsForResourceInput { resource_arn })
}

fn deserialize_put_dashboard(
    params: &[(String, String)],
) -> Result<PutDashboardInput, CloudWatchError> {
    let dashboard_name = get_required_param(params, "DashboardName")?.to_owned();
    let dashboard_body = get_required_param(params, "DashboardBody")?.to_owned();
    Ok(PutDashboardInput {
        dashboard_name,
        dashboard_body,
    })
}

fn deserialize_get_dashboard(
    params: &[(String, String)],
) -> Result<GetDashboardInput, CloudWatchError> {
    let dashboard_name = get_required_param(params, "DashboardName")?.to_owned();
    Ok(GetDashboardInput { dashboard_name })
}

fn deserialize_delete_dashboards(params: &[(String, String)]) -> DeleteDashboardsInput {
    let dashboard_names = parse_string_list(params, "DashboardNames");
    DeleteDashboardsInput { dashboard_names }
}

fn deserialize_list_dashboards(params: &[(String, String)]) -> ListDashboardsInput {
    let dashboard_name_prefix =
        get_optional_param(params, "DashboardNamePrefix").map(str::to_owned);
    let next_token = get_optional_param(params, "NextToken").map(str::to_owned);
    ListDashboardsInput {
        dashboard_name_prefix,
        next_token,
    }
}

fn deserialize_put_insight_rule(
    params: &[(String, String)],
) -> Result<PutInsightRuleInput, CloudWatchError> {
    let rule_name = get_required_param(params, "RuleName")?.to_owned();
    let rule_definition = get_required_param(params, "RuleDefinition")?.to_owned();
    let rule_state = get_optional_param(params, "RuleState").map(str::to_owned);
    let apply_on_transformed_logs = get_optional_bool(params, "ApplyOnTransformedLogs");

    let tags_raw = parse_tag_list(params, "Tags")?;
    let tags: Vec<Tag> = tags_raw
        .into_iter()
        .map(|(k, v)| Tag { key: k, value: v })
        .collect();

    Ok(PutInsightRuleInput {
        rule_name,
        rule_definition,
        rule_state,
        apply_on_transformed_logs,
        tags,
    })
}

fn deserialize_delete_insight_rules(params: &[(String, String)]) -> DeleteInsightRulesInput {
    let rule_names = parse_string_list(params, "RuleNames");
    DeleteInsightRulesInput { rule_names }
}

fn deserialize_describe_insight_rules(
    params: &[(String, String)],
) -> Result<DescribeInsightRulesInput, CloudWatchError> {
    let max_results = get_optional_i32(params, "MaxResults")?;
    let next_token = get_optional_param(params, "NextToken").map(str::to_owned);
    Ok(DescribeInsightRulesInput {
        max_results,
        next_token,
    })
}

fn deserialize_put_anomaly_detector(
    params: &[(String, String)],
) -> Result<PutAnomalyDetectorInput, CloudWatchError> {
    let namespace = get_optional_param(params, "Namespace").map(str::to_owned);
    let metric_name = get_optional_param(params, "MetricName").map(str::to_owned);
    let stat = get_optional_param(params, "Stat").map(str::to_owned);

    let dims_raw = parse_dimensions(params, "Dimensions");
    let dimensions: Vec<Dimension> = dims_raw
        .into_iter()
        .map(|(n, v)| Dimension { name: n, value: v })
        .collect();

    let configuration = parse_anomaly_detector_configuration(params)?;
    let metric_characteristics = get_optional_bool(params, "MetricCharacteristics.PeriodicSpikes")
        .map(|ps| MetricCharacteristics {
            periodic_spikes: Some(ps),
        });

    let single_metric_anomaly_detector = parse_single_metric_anomaly_detector(params);
    let metric_math_anomaly_detector = parse_metric_math_anomaly_detector(params)?;

    Ok(PutAnomalyDetectorInput {
        namespace,
        metric_name,
        stat,
        dimensions,
        configuration,
        metric_characteristics,
        single_metric_anomaly_detector,
        metric_math_anomaly_detector,
    })
}

fn parse_anomaly_detector_configuration(
    params: &[(String, String)],
) -> Result<Option<ruststack_cloudwatch_model::types::AnomalyDetectorConfiguration>, CloudWatchError>
{
    let metric_timezone =
        get_optional_param(params, "Configuration.MetricTimezone").map(str::to_owned);

    // Parse ExcludedTimeRanges
    let range_structs = parse_struct_list(params, "Configuration.ExcludedTimeRanges");
    let mut excluded_time_ranges = Vec::new();
    for sub_params in &range_structs {
        let start_time = get_required_timestamp(sub_params, "StartTime")?;
        let end_time = get_required_timestamp(sub_params, "EndTime")?;
        excluded_time_ranges.push(ruststack_cloudwatch_model::types::Range {
            start_time,
            end_time,
        });
    }

    if metric_timezone.is_none() && excluded_time_ranges.is_empty() {
        return Ok(None);
    }

    Ok(Some(
        ruststack_cloudwatch_model::types::AnomalyDetectorConfiguration {
            metric_timezone,
            excluded_time_ranges,
        },
    ))
}

fn parse_single_metric_anomaly_detector(
    params: &[(String, String)],
) -> Option<SingleMetricAnomalyDetector> {
    let prefix = "SingleMetricAnomalyDetector.";
    let namespace = get_optional_param(params, &format!("{prefix}Namespace")).map(str::to_owned);
    let metric_name = get_optional_param(params, &format!("{prefix}MetricName")).map(str::to_owned);
    let stat = get_optional_param(params, &format!("{prefix}Stat")).map(str::to_owned);
    let account_id = get_optional_param(params, &format!("{prefix}AccountId")).map(str::to_owned);

    let dim_prefix = format!("{prefix}Dimensions");
    let dims_raw = parse_dimensions(params, &dim_prefix);
    let dimensions: Vec<Dimension> = dims_raw
        .into_iter()
        .map(|(n, v)| Dimension { name: n, value: v })
        .collect();

    if namespace.is_none()
        && metric_name.is_none()
        && stat.is_none()
        && account_id.is_none()
        && dimensions.is_empty()
    {
        return None;
    }

    Some(SingleMetricAnomalyDetector {
        namespace,
        metric_name,
        stat,
        account_id,
        dimensions,
    })
}

fn parse_metric_math_anomaly_detector(
    params: &[(String, String)],
) -> Result<Option<MetricMathAnomalyDetector>, CloudWatchError> {
    let queries = parse_metric_data_queries(params, "MetricMathAnomalyDetector.MetricDataQueries")?;

    if queries.is_empty() {
        return Ok(None);
    }

    Ok(Some(MetricMathAnomalyDetector {
        metric_data_queries: queries,
    }))
}

fn deserialize_describe_anomaly_detectors(
    params: &[(String, String)],
) -> Result<DescribeAnomalyDetectorsInput, CloudWatchError> {
    let namespace = get_optional_param(params, "Namespace").map(str::to_owned);
    let metric_name = get_optional_param(params, "MetricName").map(str::to_owned);
    let max_results = get_optional_i32(params, "MaxResults")?;
    let next_token = get_optional_param(params, "NextToken").map(str::to_owned);

    let anomaly_detector_types: Vec<AnomalyDetectorType> =
        parse_string_list(params, "AnomalyDetectorTypes")
            .iter()
            .map(|s| AnomalyDetectorType::from(s.as_str()))
            .collect();

    let dims_raw = parse_dimensions(params, "Dimensions");
    let dimensions: Vec<Dimension> = dims_raw
        .into_iter()
        .map(|(n, v)| Dimension { name: n, value: v })
        .collect();

    Ok(DescribeAnomalyDetectorsInput {
        namespace,
        metric_name,
        max_results,
        next_token,
        anomaly_detector_types,
        dimensions,
    })
}

fn deserialize_delete_anomaly_detector(
    params: &[(String, String)],
) -> Result<DeleteAnomalyDetectorInput, CloudWatchError> {
    let namespace = get_optional_param(params, "Namespace").map(str::to_owned);
    let metric_name = get_optional_param(params, "MetricName").map(str::to_owned);
    let stat = get_optional_param(params, "Stat").map(str::to_owned);

    let dims_raw = parse_dimensions(params, "Dimensions");
    let dimensions: Vec<Dimension> = dims_raw
        .into_iter()
        .map(|(n, v)| Dimension { name: n, value: v })
        .collect();

    let single_metric_anomaly_detector = parse_single_metric_anomaly_detector(params);
    let metric_math_anomaly_detector = parse_metric_math_anomaly_detector(params)?;

    Ok(DeleteAnomalyDetectorInput {
        namespace,
        metric_name,
        stat,
        dimensions,
        single_metric_anomaly_detector,
        metric_math_anomaly_detector,
    })
}

fn deserialize_put_managed_insight_rules(
    params: &[(String, String)],
) -> Result<PutManagedInsightRulesInput, CloudWatchError> {
    let struct_list = parse_struct_list(params, "ManagedRules");
    let mut managed_rules = Vec::with_capacity(struct_list.len());

    for sub_params in &struct_list {
        let template_name = get_required_param(sub_params, "TemplateName")?.to_owned();
        let resource_arn = get_required_param(sub_params, "ResourceARN")?.to_owned();

        let tags_raw = parse_tag_list(sub_params, "Tags")?;
        let tags: Vec<Tag> = tags_raw
            .into_iter()
            .map(|(k, v)| Tag { key: k, value: v })
            .collect();

        managed_rules.push(ManagedRule {
            template_name,
            resource_arn,
            tags,
        });
    }

    Ok(PutManagedInsightRulesInput { managed_rules })
}

fn deserialize_put_metric_stream(
    params: &[(String, String)],
) -> Result<PutMetricStreamInput, CloudWatchError> {
    let name = get_required_param(params, "Name")?.to_owned();
    let firehose_arn = get_required_param(params, "FirehoseArn")?.to_owned();
    let role_arn = get_required_param(params, "RoleArn")?.to_owned();
    let output_format_str = get_required_param(params, "OutputFormat")?;
    let output_format = MetricStreamOutputFormat::from(output_format_str);
    let include_linked_accounts_metrics = get_optional_bool(params, "IncludeLinkedAccountsMetrics");

    let include_filters = parse_metric_stream_filter_list(params, "IncludeFilters");
    let exclude_filters = parse_metric_stream_filter_list(params, "ExcludeFilters");
    let statistics_configurations =
        parse_statistics_configurations(params, "StatisticsConfigurations")?;

    let tags_raw = parse_tag_list(params, "Tags")?;
    let tags: Vec<Tag> = tags_raw
        .into_iter()
        .map(|(k, v)| Tag { key: k, value: v })
        .collect();

    Ok(PutMetricStreamInput {
        name,
        firehose_arn,
        role_arn,
        output_format,
        include_linked_accounts_metrics,
        include_filters,
        exclude_filters,
        statistics_configurations,
        tags,
    })
}

fn parse_metric_stream_filter_list(
    params: &[(String, String)],
    prefix: &str,
) -> Vec<MetricStreamFilter> {
    let struct_list = parse_struct_list(params, prefix);
    let mut filters = Vec::with_capacity(struct_list.len());

    for sub_params in &struct_list {
        let namespace = get_optional_param(sub_params, "Namespace").map(str::to_owned);
        let metric_names = parse_string_list(sub_params, "MetricNames");
        filters.push(MetricStreamFilter {
            namespace,
            metric_names,
        });
    }

    filters
}

fn parse_statistics_configurations(
    params: &[(String, String)],
    prefix: &str,
) -> Result<Vec<MetricStreamStatisticsConfiguration>, CloudWatchError> {
    let struct_list = parse_struct_list(params, prefix);
    let mut configs = Vec::with_capacity(struct_list.len());

    for sub_params in &struct_list {
        let additional_statistics = parse_string_list(sub_params, "AdditionalStatistics");

        let include_metrics_structs = parse_struct_list(sub_params, "IncludeMetrics");
        let mut include_metrics = Vec::with_capacity(include_metrics_structs.len());
        for metric_params in &include_metrics_structs {
            let metric_name = get_required_param(metric_params, "MetricName")?.to_owned();
            let namespace = get_required_param(metric_params, "Namespace")?.to_owned();
            include_metrics.push(MetricStreamStatisticsMetric {
                metric_name,
                namespace,
            });
        }

        configs.push(MetricStreamStatisticsConfiguration {
            additional_statistics,
            include_metrics,
        });
    }

    Ok(configs)
}

fn deserialize_delete_metric_stream(
    params: &[(String, String)],
) -> Result<DeleteMetricStreamInput, CloudWatchError> {
    let name = get_required_param(params, "Name")?.to_owned();
    Ok(DeleteMetricStreamInput { name })
}

fn deserialize_list_metric_streams(
    params: &[(String, String)],
) -> Result<ListMetricStreamsInput, CloudWatchError> {
    let max_results = get_optional_i32(params, "MaxResults")?;
    let next_token = get_optional_param(params, "NextToken").map(str::to_owned);
    Ok(ListMetricStreamsInput {
        max_results,
        next_token,
    })
}

fn deserialize_get_metric_stream(
    params: &[(String, String)],
) -> Result<GetMetricStreamInput, CloudWatchError> {
    let name = get_required_param(params, "Name")?.to_owned();
    Ok(GetMetricStreamInput { name })
}
