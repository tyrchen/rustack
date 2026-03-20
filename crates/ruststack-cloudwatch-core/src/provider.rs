//! CloudWatch Metrics business logic provider.
//!
//! Implements all 31 CloudWatch operations: metric data ingestion/retrieval,
//! alarm management, dashboard CRUD, insight rules, anomaly detectors,
//! metric streams, and tagging.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use tracing::instrument;

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
use ruststack_cloudwatch_model::output::{
    DeleteAnomalyDetectorOutput, DeleteDashboardsOutput, DeleteInsightRulesOutput,
    DeleteMetricStreamOutput, DescribeAlarmHistoryOutput, DescribeAlarmsForMetricOutput,
    DescribeAlarmsOutput, DescribeAnomalyDetectorsOutput, DescribeInsightRulesOutput,
    GetDashboardOutput, GetMetricDataOutput, GetMetricStatisticsOutput, GetMetricStreamOutput,
    ListDashboardsOutput, ListMetricStreamsOutput, ListMetricsOutput, ListTagsForResourceOutput,
    PutAnomalyDetectorOutput, PutDashboardOutput, PutInsightRuleOutput,
    PutManagedInsightRulesOutput, PutMetricStreamOutput, TagResourceOutput, UntagResourceOutput,
};
use ruststack_cloudwatch_model::types::{
    AlarmHistoryItem, AlarmType, AnomalyDetector, CompositeAlarm, DashboardEntry, Datapoint,
    HistoryItemType, InsightRule, Metric, MetricAlarm, MetricDataResult, MetricStreamEntry,
    MetricStreamOutputFormat, StateValue, Statistic, StatusCode,
};

use crate::aggregation::aggregate_statistics;
use crate::alarm_store::AlarmStore;
use crate::anomaly_store::AnomalyStore;
use crate::config::CloudWatchConfig;
use crate::dashboard_store::{DashboardRecord, DashboardStore};
use crate::dimensions::{dimensions_match, normalize_dimensions};
use crate::insight_store::InsightStore;
use crate::metric_store::{DataPoint, MetricKey, MetricStore, StatisticSet};
use crate::metric_stream_store::{MetricStreamRecord, MetricStreamStore};
use crate::validation::{
    validate_alarm_name, validate_dashboard_body, validate_dashboard_name, validate_dimensions,
    validate_metric_name, validate_namespace,
};

/// CloudWatch service provider implementing all 31 operations.
///
/// Holds all in-memory stores and dispatches each API call
/// to the appropriate store with validation, ARN generation,
/// and history recording.
#[derive(Debug)]
pub struct RustStackCloudWatch {
    metric_store: Arc<MetricStore>,
    alarm_store: Arc<AlarmStore>,
    dashboard_store: Arc<DashboardStore>,
    insight_store: Arc<InsightStore>,
    anomaly_store: Arc<AnomalyStore>,
    metric_stream_store: Arc<MetricStreamStore>,
    config: Arc<CloudWatchConfig>,
}

#[allow(clippy::needless_pass_by_value)] // Input structs are passed by value following AWS SDK conventions
impl RustStackCloudWatch {
    /// Create a new `RustStackCloudWatch` provider with the given configuration.
    ///
    /// All stores are initialized empty and ready to accept operations.
    #[must_use]
    pub fn new(config: Arc<CloudWatchConfig>) -> Self {
        let max_points = config.max_points_per_series;
        Self {
            metric_store: Arc::new(MetricStore::new(max_points)),
            alarm_store: Arc::new(AlarmStore::new()),
            dashboard_store: Arc::new(DashboardStore::new()),
            insight_store: Arc::new(InsightStore::new()),
            anomaly_store: Arc::new(AnomalyStore::new()),
            metric_stream_store: Arc::new(MetricStreamStore::new()),
            config,
        }
    }

    // ── Metric Data ──────────────────────────────────────────────────

    /// Publish metric data points to CloudWatch.
    #[instrument(skip(self, input))]
    #[allow(clippy::too_many_lines)]
    pub fn put_metric_data(&self, input: PutMetricDataInput) -> Result<(), CloudWatchError> {
        validate_namespace(&input.namespace)?;

        for datum in &input.metric_data {
            validate_metric_name(&datum.metric_name)?;
            validate_dimensions(&datum.dimensions)?;
        }

        let now_ms = Utc::now().timestamp_millis();

        for datum in input.metric_data {
            let dims = normalize_dimensions(datum.dimensions);
            let key = MetricKey::new(input.namespace.clone(), datum.metric_name, dims);

            let timestamp_ms = datum.timestamp.map_or(now_ms, |t| t.timestamp_millis());

            let stat_values = datum.statistic_values.map(|sv| StatisticSet {
                sample_count: sv.sample_count,
                sum: sv.sum,
                minimum: sv.minimum,
                maximum: sv.maximum,
            });

            let dp = DataPoint {
                timestamp_ms,
                value: datum.value,
                statistic_values: stat_values,
                values: datum.values,
                counts: datum.counts,
                unit: datum.unit,
            };

            self.metric_store.insert(key, dp);
        }

        Ok(())
    }

    /// Retrieve metric statistics for a single metric.
    #[instrument(skip(self, input))]
    pub fn get_metric_statistics(
        &self,
        input: GetMetricStatisticsInput,
    ) -> Result<GetMetricStatisticsOutput, CloudWatchError> {
        validate_namespace(&input.namespace)?;
        validate_metric_name(&input.metric_name)?;
        validate_dimensions(&input.dimensions)?;

        if input.statistics.is_empty() {
            return Err(CloudWatchError::with_message(
                CloudWatchErrorCode::MissingRequiredParameterException,
                "Statistics must not be empty.",
            ));
        }

        let dims = normalize_dimensions(input.dimensions);
        let key = MetricKey::new(input.namespace, input.metric_name.clone(), dims);

        let datapoints = if let Some(series) = self.metric_store.get(&key) {
            let start_secs = input.start_time.timestamp();
            let end_secs = input.end_time.timestamp();
            let period = i64::from(input.period);

            let aggregated = aggregate_statistics(
                &series.data_points,
                start_secs,
                end_secs,
                period,
                &input.statistics,
            );

            aggregated
                .into_iter()
                .map(|a| Datapoint {
                    timestamp: Some(
                        chrono::DateTime::from_timestamp(a.timestamp, 0).unwrap_or_default(),
                    ),
                    sum: a.sum,
                    average: a.average,
                    minimum: a.minimum,
                    maximum: a.maximum,
                    sample_count: a.sample_count,
                    unit: a.unit,
                    extended_statistics: HashMap::default(),
                })
                .collect()
        } else {
            Vec::new()
        };

        Ok(GetMetricStatisticsOutput {
            label: Some(input.metric_name),
            datapoints,
        })
    }

    /// Retrieve metric data for one or more metrics using `MetricStat` queries.
    ///
    /// Expression-based queries are not supported and will return an error.
    #[instrument(skip(self, input))]
    #[allow(clippy::too_many_lines)]
    pub fn get_metric_data(
        &self,
        input: GetMetricDataInput,
    ) -> Result<GetMetricDataOutput, CloudWatchError> {
        let mut results = Vec::with_capacity(input.metric_data_queries.len());

        for query in &input.metric_data_queries {
            if query.expression.is_some() {
                return Err(CloudWatchError::with_message(
                    CloudWatchErrorCode::InvalidParameterValueException,
                    "Expression-based queries are not supported. Use MetricStat instead.",
                ));
            }

            let should_return = query.return_data.unwrap_or(true);
            if !should_return {
                continue;
            }

            let Some(metric_stat) = &query.metric_stat else {
                results.push(MetricDataResult {
                    id: Some(query.id.clone()),
                    label: query.label.clone(),
                    timestamps: Vec::new(),
                    values: Vec::new(),
                    status_code: Some(StatusCode::Complete),
                    messages: Vec::new(),
                });
                continue;
            };

            let namespace = metric_stat.metric.namespace.as_deref().unwrap_or_default();
            let metric_name = metric_stat
                .metric
                .metric_name
                .as_deref()
                .unwrap_or_default();
            let dims = normalize_dimensions(metric_stat.metric.dimensions.clone());

            let stat: Statistic = Statistic::from(metric_stat.stat.as_str());
            let key = MetricKey::new(namespace.to_owned(), metric_name.to_owned(), dims);

            let (timestamps, values) = if let Some(series) = self.metric_store.get(&key) {
                let start_secs = input.start_time.timestamp();
                let end_secs = input.end_time.timestamp();
                let period = i64::from(metric_stat.period);

                let aggregated = aggregate_statistics(
                    &series.data_points,
                    start_secs,
                    end_secs,
                    period,
                    std::slice::from_ref(&stat),
                );

                let mut ts_vec = Vec::with_capacity(aggregated.len());
                let mut val_vec = Vec::with_capacity(aggregated.len());

                for a in aggregated {
                    if let Some(dt) = chrono::DateTime::from_timestamp(a.timestamp, 0) {
                        ts_vec.push(dt);
                        let val = match stat {
                            Statistic::Sum => a.sum.unwrap_or(0.0),
                            Statistic::Average => a.average.unwrap_or(0.0),
                            Statistic::Minimum => a.minimum.unwrap_or(0.0),
                            Statistic::Maximum => a.maximum.unwrap_or(0.0),
                            Statistic::SampleCount => a.sample_count.unwrap_or(0.0),
                        };
                        val_vec.push(val);
                    }
                }

                (ts_vec, val_vec)
            } else {
                (Vec::new(), Vec::new())
            };

            results.push(MetricDataResult {
                id: Some(query.id.clone()),
                label: query.label.clone().or_else(|| Some(metric_name.to_owned())),
                timestamps,
                values,
                status_code: Some(StatusCode::Complete),
                messages: Vec::new(),
            });
        }

        Ok(GetMetricDataOutput {
            metric_data_results: results,
            next_token: None,
            messages: Vec::new(),
        })
    }

    /// List all metrics in the store, with optional namespace/name/dimension filters.
    #[instrument(skip(self, input))]
    pub fn list_metrics(
        &self,
        input: ListMetricsInput,
    ) -> Result<ListMetricsOutput, CloudWatchError> {
        let three_hours_ms = 3 * 60 * 60 * 1000_i64;
        let now_ms = Utc::now().timestamp_millis();

        let dim_filters: Vec<(String, Option<String>)> = input
            .dimensions
            .into_iter()
            .map(|df| (df.name, df.value))
            .collect();

        let keys = self.metric_store.keys();

        let metrics: Vec<Metric> = keys
            .into_iter()
            .filter(|key| {
                if let Some(ref ns) = input.namespace {
                    if key.namespace != *ns {
                        return false;
                    }
                }
                if let Some(ref mn) = input.metric_name {
                    if key.metric_name != *mn {
                        return false;
                    }
                }
                if !dim_filters.is_empty() && !dimensions_match(&key.dimensions, &dim_filters) {
                    return false;
                }
                // If recently_active filter is set, check last data point timestamp.
                if input.recently_active.is_some() {
                    if let Some(series) = self.metric_store.get(key) {
                        let latest = series.data_points.keys().next_back().copied().unwrap_or(0);
                        if now_ms - latest > three_hours_ms {
                            return false;
                        }
                    } else {
                        return false;
                    }
                }
                true
            })
            .map(|key| Metric {
                namespace: Some(key.namespace),
                metric_name: Some(key.metric_name),
                dimensions: key.dimensions,
            })
            .collect();

        Ok(ListMetricsOutput {
            metrics,
            next_token: None,
            owning_accounts: Vec::new(),
        })
    }

    // ── Metric Alarms ────────────────────────────────────────────────

    /// Create or update a metric alarm.
    #[instrument(skip(self, input))]
    #[allow(clippy::too_many_lines)]
    pub fn put_metric_alarm(&self, input: PutMetricAlarmInput) -> Result<(), CloudWatchError> {
        validate_alarm_name(&input.alarm_name)?;

        let now = Utc::now();
        let arn = format!(
            "arn:aws:cloudwatch:{}:{}:alarm:{}",
            self.config.default_region, self.config.account_id, input.alarm_name
        );

        let is_update = self
            .alarm_store
            .get_metric_alarm(&input.alarm_name)
            .is_some();

        let alarm = MetricAlarm {
            alarm_name: Some(input.alarm_name.clone()),
            alarm_arn: Some(arn.clone()),
            alarm_description: input.alarm_description,
            namespace: input.namespace,
            metric_name: input.metric_name,
            dimensions: input.dimensions,
            statistic: input.statistic,
            extended_statistic: input.extended_statistic,
            period: input.period,
            evaluation_periods: Some(input.evaluation_periods),
            datapoints_to_alarm: input.datapoints_to_alarm,
            comparison_operator: Some(input.comparison_operator),
            threshold: input.threshold,
            threshold_metric_id: input.threshold_metric_id,
            treat_missing_data: input.treat_missing_data,
            alarm_actions: input.alarm_actions,
            ok_actions: input.ok_actions,
            insufficient_data_actions: input.insufficient_data_actions,
            actions_enabled: Some(input.actions_enabled.unwrap_or(true)),
            state_value: Some(StateValue::InsufficientData),
            state_reason: Some("Unchecked: Initial alarm creation".to_owned()),
            state_reason_data: None,
            state_updated_timestamp: Some(now),
            state_transitioned_timestamp: Some(now),
            alarm_configuration_updated_timestamp: Some(now),
            unit: input.unit,
            metrics: input.metrics,
            evaluate_low_sample_count_percentile: input.evaluate_low_sample_count_percentile,
            evaluation_state: None,
        };

        self.alarm_store.put_metric_alarm(&input.alarm_name, alarm);

        // Record history.
        let summary = if is_update {
            "Alarm updated"
        } else {
            "Alarm created"
        };

        self.alarm_store.record_history(
            &input.alarm_name,
            AlarmHistoryItem {
                alarm_name: Some(input.alarm_name.clone()),
                alarm_type: Some(AlarmType::MetricAlarm),
                history_item_type: Some(HistoryItemType::ConfigurationUpdate),
                history_summary: Some(summary.to_owned()),
                history_data: None,
                timestamp: Some(now),
                ..Default::default()
            },
        );

        // Apply tags if provided.
        if !input.tags.is_empty() {
            self.alarm_store.set_tags(&arn, input.tags);
        }

        Ok(())
    }

    /// Describe alarms with optional name/prefix/state/type filters.
    #[instrument(skip(self, input))]
    #[allow(clippy::too_many_lines)]
    pub fn describe_alarms(
        &self,
        input: DescribeAlarmsInput,
    ) -> Result<DescribeAlarmsOutput, CloudWatchError> {
        #[allow(clippy::cast_sign_loss)]
        let max_records = input.max_records.unwrap_or(100).clamp(1, 100) as usize;
        let include_metric =
            input.alarm_types.is_empty() || input.alarm_types.contains(&AlarmType::MetricAlarm);
        let include_composite =
            input.alarm_types.is_empty() || input.alarm_types.contains(&AlarmType::CompositeAlarm);

        let mut metric_alarms = Vec::new();
        let mut composite_alarms = Vec::new();

        if include_metric {
            let all = self.alarm_store.list_metric_alarms();
            for alarm in all {
                if !input.alarm_names.is_empty() {
                    if let Some(ref name) = alarm.alarm_name {
                        if !input.alarm_names.contains(name) {
                            continue;
                        }
                    } else {
                        continue;
                    }
                }
                if let Some(ref prefix) = input.alarm_name_prefix {
                    if !alarm
                        .alarm_name
                        .as_deref()
                        .unwrap_or_default()
                        .starts_with(prefix.as_str())
                    {
                        continue;
                    }
                }
                if let Some(ref sv) = input.state_value {
                    if alarm.state_value.as_ref() != Some(sv) {
                        continue;
                    }
                }
                if let Some(ref ap) = input.action_prefix {
                    let has_matching_action = alarm
                        .alarm_actions
                        .iter()
                        .any(|a| a.starts_with(ap.as_str()));
                    if !has_matching_action {
                        continue;
                    }
                }
                metric_alarms.push(alarm);
                if metric_alarms.len() >= max_records {
                    break;
                }
            }
        }

        if include_composite {
            let all = self.alarm_store.list_composite_alarms();
            for alarm in all {
                if !input.alarm_names.is_empty() {
                    if let Some(ref name) = alarm.alarm_name {
                        if !input.alarm_names.contains(name) {
                            continue;
                        }
                    } else {
                        continue;
                    }
                }
                if let Some(ref prefix) = input.alarm_name_prefix {
                    if !alarm
                        .alarm_name
                        .as_deref()
                        .unwrap_or_default()
                        .starts_with(prefix.as_str())
                    {
                        continue;
                    }
                }
                if let Some(ref sv) = input.state_value {
                    if alarm.state_value.as_ref() != Some(sv) {
                        continue;
                    }
                }
                composite_alarms.push(alarm);
                if composite_alarms.len() >= max_records {
                    break;
                }
            }
        }

        Ok(DescribeAlarmsOutput {
            metric_alarms,
            composite_alarms,
            next_token: None,
        })
    }

    /// Describe metric alarms configured for a specific metric.
    #[instrument(skip(self, input))]
    pub fn describe_alarms_for_metric(
        &self,
        input: DescribeAlarmsForMetricInput,
    ) -> Result<DescribeAlarmsForMetricOutput, CloudWatchError> {
        validate_namespace(&input.namespace)?;
        validate_metric_name(&input.metric_name)?;

        let stat_str = input.statistic.as_ref().map(|s| s.as_str().to_owned());

        let alarms = self.alarm_store.find_alarms_for_metric(
            &input.namespace,
            &input.metric_name,
            if input.dimensions.is_empty() {
                None
            } else {
                Some(&input.dimensions)
            },
            input.period,
            stat_str.as_deref(),
        );

        Ok(DescribeAlarmsForMetricOutput {
            metric_alarms: alarms,
        })
    }

    /// Delete one or more alarms (metric or composite).
    #[instrument(skip(self, input))]
    pub fn delete_alarms(&self, input: DeleteAlarmsInput) -> Result<(), CloudWatchError> {
        for name in &input.alarm_names {
            let found = self.alarm_store.delete_metric_alarm(name)
                || self.alarm_store.delete_composite_alarm(name);
            if !found {
                return Err(CloudWatchError::with_message(
                    CloudWatchErrorCode::ResourceNotFound,
                    format!("Alarm {name} does not exist."),
                ));
            }
        }
        Ok(())
    }

    /// Temporarily set the state of an alarm for testing purposes.
    #[instrument(skip(self, input))]
    pub fn set_alarm_state(&self, input: SetAlarmStateInput) -> Result<(), CloudWatchError> {
        validate_alarm_name(&input.alarm_name)?;

        let now = Utc::now();

        // Try metric alarm first.
        if let Some(mut alarm) = self.alarm_store.get_metric_alarm_mut(&input.alarm_name) {
            alarm.state_value = Some(input.state_value.clone());
            alarm.state_reason = Some(input.state_reason.clone());
            alarm.state_reason_data.clone_from(&input.state_reason_data);
            alarm.state_updated_timestamp = Some(now);
            alarm.state_transitioned_timestamp = Some(now);
            drop(alarm);

            self.alarm_store.record_history(
                &input.alarm_name,
                AlarmHistoryItem {
                    alarm_name: Some(input.alarm_name.clone()),
                    alarm_type: Some(AlarmType::MetricAlarm),
                    history_item_type: Some(HistoryItemType::StateUpdate),
                    history_summary: Some(input.state_reason),
                    history_data: input.state_reason_data,
                    timestamp: Some(now),
                    ..Default::default()
                },
            );
            return Ok(());
        }

        // Try composite alarm.
        if let Some(mut alarm) = self.alarm_store.get_composite_alarm_mut(&input.alarm_name) {
            alarm.state_value = Some(input.state_value.clone());
            alarm.state_reason = Some(input.state_reason.clone());
            alarm.state_reason_data.clone_from(&input.state_reason_data);
            alarm.state_updated_timestamp = Some(now);
            alarm.state_transitioned_timestamp = Some(now);
            drop(alarm);

            self.alarm_store.record_history(
                &input.alarm_name,
                AlarmHistoryItem {
                    alarm_name: Some(input.alarm_name.clone()),
                    alarm_type: Some(AlarmType::CompositeAlarm),
                    history_item_type: Some(HistoryItemType::StateUpdate),
                    history_summary: Some(input.state_reason),
                    history_data: input.state_reason_data,
                    timestamp: Some(now),
                    ..Default::default()
                },
            );
            return Ok(());
        }

        Err(CloudWatchError::with_message(
            CloudWatchErrorCode::ResourceNotFound,
            format!("Alarm {} does not exist.", input.alarm_name),
        ))
    }

    /// Enable actions for the specified alarms.
    #[instrument(skip(self, input))]
    pub fn enable_alarm_actions(
        &self,
        input: EnableAlarmActionsInput,
    ) -> Result<(), CloudWatchError> {
        self.alarm_store
            .set_actions_enabled(&input.alarm_names, true);
        Ok(())
    }

    /// Disable actions for the specified alarms.
    #[instrument(skip(self, input))]
    pub fn disable_alarm_actions(
        &self,
        input: DisableAlarmActionsInput,
    ) -> Result<(), CloudWatchError> {
        self.alarm_store
            .set_actions_enabled(&input.alarm_names, false);
        Ok(())
    }

    /// Retrieve alarm history entries with optional filters.
    #[instrument(skip(self, input))]
    pub fn describe_alarm_history(
        &self,
        input: DescribeAlarmHistoryInput,
    ) -> Result<DescribeAlarmHistoryOutput, CloudWatchError> {
        #[allow(clippy::cast_sign_loss)]
        let max_records = input.max_records.unwrap_or(100).clamp(1, 100) as usize;

        let mut items = self.alarm_store.get_history(input.alarm_name.as_deref());

        // Filter by history item type.
        if let Some(ref hit) = input.history_item_type {
            items.retain(|item| item.history_item_type.as_ref() == Some(hit));
        }

        // Filter by date range.
        if let Some(ref start) = input.start_date {
            items.retain(|item| item.timestamp.as_ref().is_some_and(|ts| ts >= start));
        }
        if let Some(ref end) = input.end_date {
            items.retain(|item| item.timestamp.as_ref().is_some_and(|ts| ts <= end));
        }

        items.truncate(max_records);

        Ok(DescribeAlarmHistoryOutput {
            alarm_history_items: items,
            next_token: None,
        })
    }

    // ── Composite Alarms ─────────────────────────────────────────────

    /// Create or update a composite alarm.
    #[instrument(skip(self, input))]
    pub fn put_composite_alarm(
        &self,
        input: PutCompositeAlarmInput,
    ) -> Result<(), CloudWatchError> {
        validate_alarm_name(&input.alarm_name)?;

        let now = Utc::now();
        let arn = format!(
            "arn:aws:cloudwatch:{}:{}:alarm:{}",
            self.config.default_region, self.config.account_id, input.alarm_name
        );

        let alarm = CompositeAlarm {
            alarm_name: Some(input.alarm_name.clone()),
            alarm_arn: Some(arn.clone()),
            alarm_description: input.alarm_description,
            alarm_rule: Some(input.alarm_rule),
            alarm_actions: input.alarm_actions,
            ok_actions: input.ok_actions,
            insufficient_data_actions: input.insufficient_data_actions,
            actions_enabled: Some(input.actions_enabled.unwrap_or(true)),
            state_value: Some(StateValue::InsufficientData),
            state_reason: Some("Unchecked: Initial alarm creation".to_owned()),
            state_reason_data: None,
            state_updated_timestamp: Some(now),
            state_transitioned_timestamp: Some(now),
            alarm_configuration_updated_timestamp: Some(now),
            actions_suppressor: input.actions_suppressor,
            actions_suppressor_wait_period: input.actions_suppressor_wait_period,
            actions_suppressor_extension_period: input.actions_suppressor_extension_period,
            actions_suppressed_by: None,
            actions_suppressed_reason: None,
        };

        self.alarm_store
            .put_composite_alarm(&input.alarm_name, alarm);

        self.alarm_store.record_history(
            &input.alarm_name,
            AlarmHistoryItem {
                alarm_name: Some(input.alarm_name.clone()),
                alarm_type: Some(AlarmType::CompositeAlarm),
                history_item_type: Some(HistoryItemType::ConfigurationUpdate),
                history_summary: Some("Composite alarm created or updated".to_owned()),
                history_data: None,
                timestamp: Some(now),
                ..Default::default()
            },
        );

        if !input.tags.is_empty() {
            self.alarm_store.set_tags(&arn, input.tags);
        }

        Ok(())
    }

    // ── Tagging ──────────────────────────────────────────────────────

    /// Add or update tags on a CloudWatch resource.
    #[instrument(skip(self, input))]
    pub fn tag_resource(
        &self,
        input: TagResourceInput,
    ) -> Result<TagResourceOutput, CloudWatchError> {
        self.alarm_store.set_tags(&input.resource_arn, input.tags);
        Ok(TagResourceOutput {})
    }

    /// Remove tags from a CloudWatch resource.
    #[instrument(skip(self, input))]
    pub fn untag_resource(
        &self,
        input: UntagResourceInput,
    ) -> Result<UntagResourceOutput, CloudWatchError> {
        self.alarm_store
            .remove_tags(&input.resource_arn, &input.tag_keys);
        Ok(UntagResourceOutput {})
    }

    /// List tags for a CloudWatch resource.
    #[instrument(skip(self, input))]
    pub fn list_tags_for_resource(
        &self,
        input: ListTagsForResourceInput,
    ) -> Result<ListTagsForResourceOutput, CloudWatchError> {
        let tags = self.alarm_store.get_tags(&input.resource_arn);
        Ok(ListTagsForResourceOutput { tags })
    }

    // ── Dashboards ───────────────────────────────────────────────────

    /// Create or update a dashboard.
    #[instrument(skip(self, input))]
    pub fn put_dashboard(
        &self,
        input: PutDashboardInput,
    ) -> Result<PutDashboardOutput, CloudWatchError> {
        validate_dashboard_name(&input.dashboard_name)?;
        validate_dashboard_body(&input.dashboard_body)?;

        #[allow(clippy::cast_precision_loss)]
        let now = Utc::now().timestamp() as f64;
        let arn = format!(
            "arn:aws:cloudwatch::{}:dashboard/{}",
            self.config.account_id, input.dashboard_name
        );
        #[allow(clippy::cast_possible_wrap)]
        let size = input.dashboard_body.len() as i64;

        self.dashboard_store.put(DashboardRecord {
            dashboard_name: input.dashboard_name,
            dashboard_arn: arn,
            dashboard_body: input.dashboard_body,
            last_modified: now,
            size,
        });

        Ok(PutDashboardOutput {
            dashboard_validation_messages: Vec::new(),
        })
    }

    /// Get a dashboard by name.
    #[instrument(skip(self, input))]
    pub fn get_dashboard(
        &self,
        input: GetDashboardInput,
    ) -> Result<GetDashboardOutput, CloudWatchError> {
        let record = self
            .dashboard_store
            .get(&input.dashboard_name)
            .ok_or_else(|| {
                CloudWatchError::with_message(
                    CloudWatchErrorCode::DashboardNotFoundError,
                    format!("Dashboard {} does not exist.", input.dashboard_name),
                )
            })?;

        Ok(GetDashboardOutput {
            dashboard_arn: Some(record.dashboard_arn),
            dashboard_body: Some(record.dashboard_body),
            dashboard_name: Some(record.dashboard_name),
        })
    }

    /// Delete one or more dashboards by name.
    #[instrument(skip(self, input))]
    pub fn delete_dashboards(
        &self,
        input: DeleteDashboardsInput,
    ) -> Result<DeleteDashboardsOutput, CloudWatchError> {
        if input.dashboard_names.is_empty() {
            return Err(CloudWatchError::with_message(
                CloudWatchErrorCode::InvalidParameterValueException,
                "DashboardNames must not be empty.",
            ));
        }

        // Verify all dashboards exist before deleting.
        for name in &input.dashboard_names {
            if self.dashboard_store.get(name).is_none() {
                return Err(CloudWatchError::with_message(
                    CloudWatchErrorCode::DashboardNotFoundError,
                    format!("Dashboard {name} does not exist."),
                ));
            }
        }

        let _ = self.dashboard_store.delete(&input.dashboard_names);

        Ok(DeleteDashboardsOutput {})
    }

    /// List dashboards with optional name prefix filter.
    #[instrument(skip(self, input))]
    pub fn list_dashboards(
        &self,
        input: ListDashboardsInput,
    ) -> Result<ListDashboardsOutput, CloudWatchError> {
        let records = self
            .dashboard_store
            .list(input.dashboard_name_prefix.as_deref());

        let entries: Vec<DashboardEntry> = records
            .into_iter()
            .map(|r| {
                #[allow(clippy::cast_possible_truncation)]
                let last_modified = chrono::DateTime::from_timestamp(r.last_modified as i64, 0);
                DashboardEntry {
                    dashboard_name: Some(r.dashboard_name),
                    dashboard_arn: Some(r.dashboard_arn),
                    last_modified,
                    size: Some(r.size),
                }
            })
            .collect();

        Ok(ListDashboardsOutput {
            dashboard_entries: entries,
            next_token: None,
        })
    }

    // ── Insight Rules ────────────────────────────────────────────────

    /// Create or update an insight rule.
    #[instrument(skip(self, input))]
    pub fn put_insight_rule(
        &self,
        input: PutInsightRuleInput,
    ) -> Result<PutInsightRuleOutput, CloudWatchError> {
        let rule = InsightRule {
            name: input.rule_name,
            definition: input.rule_definition,
            schema: "CloudWatchLogRule/1".to_owned(),
            state: input.rule_state.unwrap_or_else(|| "ENABLED".to_owned()),
            managed_rule: None,
            apply_on_transformed_logs: input.apply_on_transformed_logs,
        };

        self.insight_store.put(rule);

        Ok(PutInsightRuleOutput {})
    }

    /// Delete insight rules by name.
    #[instrument(skip(self, input))]
    pub fn delete_insight_rules(
        &self,
        input: DeleteInsightRulesInput,
    ) -> Result<DeleteInsightRulesOutput, CloudWatchError> {
        self.insight_store.delete(&input.rule_names);
        Ok(DeleteInsightRulesOutput {
            failures: Vec::new(),
        })
    }

    /// Describe all insight rules.
    #[instrument(skip(self, input))]
    pub fn describe_insight_rules(
        &self,
        input: DescribeInsightRulesInput,
    ) -> Result<DescribeInsightRulesOutput, CloudWatchError> {
        let mut rules = self.insight_store.list();

        if let Some(max) = input.max_results {
            #[allow(clippy::cast_sign_loss)]
            rules.truncate(max as usize);
        }

        Ok(DescribeInsightRulesOutput {
            insight_rules: rules,
            next_token: None,
        })
    }

    // ── Anomaly Detectors ────────────────────────────────────────────

    /// Create or update an anomaly detector.
    #[instrument(skip(self, input))]
    pub fn put_anomaly_detector(
        &self,
        input: PutAnomalyDetectorInput,
    ) -> Result<PutAnomalyDetectorOutput, CloudWatchError> {
        let detector = AnomalyDetector {
            namespace: input.namespace,
            metric_name: input.metric_name,
            dimensions: input.dimensions,
            stat: input.stat,
            configuration: input.configuration,
            single_metric_anomaly_detector: input.single_metric_anomaly_detector,
            metric_math_anomaly_detector: input.metric_math_anomaly_detector,
            metric_characteristics: input.metric_characteristics,
            state_value: None,
        };

        self.anomaly_store.put(detector);

        Ok(PutAnomalyDetectorOutput {})
    }

    /// Describe anomaly detectors with optional namespace filter.
    #[instrument(skip(self, input))]
    pub fn describe_anomaly_detectors(
        &self,
        input: DescribeAnomalyDetectorsInput,
    ) -> Result<DescribeAnomalyDetectorsOutput, CloudWatchError> {
        let mut detectors = self.anomaly_store.list();

        if let Some(ref ns) = input.namespace {
            detectors.retain(|d| {
                // Check both top-level and single_metric_anomaly_detector namespace.
                d.namespace.as_deref() == Some(ns.as_str())
                    || d.single_metric_anomaly_detector
                        .as_ref()
                        .and_then(|s| s.namespace.as_deref())
                        == Some(ns.as_str())
            });
        }
        if let Some(ref mn) = input.metric_name {
            detectors.retain(|d| {
                d.metric_name.as_deref() == Some(mn.as_str())
                    || d.single_metric_anomaly_detector
                        .as_ref()
                        .and_then(|s| s.metric_name.as_deref())
                        == Some(mn.as_str())
            });
        }

        if let Some(max) = input.max_results {
            #[allow(clippy::cast_sign_loss)]
            detectors.truncate(max as usize);
        }

        Ok(DescribeAnomalyDetectorsOutput {
            anomaly_detectors: detectors,
            next_token: None,
        })
    }

    /// Delete an anomaly detector.
    #[instrument(skip(self, input))]
    pub fn delete_anomaly_detector(
        &self,
        input: DeleteAnomalyDetectorInput,
    ) -> Result<DeleteAnomalyDetectorOutput, CloudWatchError> {
        let detector = AnomalyDetector {
            namespace: input.namespace,
            metric_name: input.metric_name,
            dimensions: input.dimensions,
            stat: input.stat,
            single_metric_anomaly_detector: input.single_metric_anomaly_detector,
            metric_math_anomaly_detector: input.metric_math_anomaly_detector,
            configuration: None,
            metric_characteristics: None,
            state_value: None,
        };

        let deleted = self.anomaly_store.delete(&detector);
        if !deleted {
            return Err(CloudWatchError::with_message(
                CloudWatchErrorCode::ResourceNotFoundException,
                "The specified anomaly detector does not exist.",
            ));
        }

        Ok(DeleteAnomalyDetectorOutput {})
    }

    // ── Managed Insight Rules ────────────────────────────────────────

    /// Create managed insight rules (stores them as regular insight rules).
    #[instrument(skip(self, input))]
    pub fn put_managed_insight_rules(
        &self,
        input: PutManagedInsightRulesInput,
    ) -> Result<PutManagedInsightRulesOutput, CloudWatchError> {
        let failures = Vec::new();

        for managed_rule in input.managed_rules {
            let rule = InsightRule {
                name: managed_rule.template_name.clone(),
                definition: format!("managed-rule:{}", managed_rule.resource_arn),
                schema: "CloudWatchLogRule/1".to_owned(),
                state: "ENABLED".to_owned(),
                managed_rule: Some(true),
                apply_on_transformed_logs: None,
            };

            self.insight_store.put(rule);

            if !managed_rule.tags.is_empty() {
                let arn = format!(
                    "arn:aws:cloudwatch:{}:{}:insight-rule/{}",
                    self.config.default_region, self.config.account_id, managed_rule.template_name,
                );
                self.alarm_store.set_tags(&arn, managed_rule.tags);
            }
        }

        Ok(PutManagedInsightRulesOutput { failures })
    }

    // ── Metric Streams ───────────────────────────────────────────────

    /// Create or update a metric stream.
    #[instrument(skip(self, input))]
    pub fn put_metric_stream(
        &self,
        input: PutMetricStreamInput,
    ) -> Result<PutMetricStreamOutput, CloudWatchError> {
        #[allow(clippy::cast_precision_loss)]
        let now = Utc::now().timestamp() as f64;
        let arn = format!(
            "arn:aws:cloudwatch:{}:{}:metric-stream/{}",
            self.config.default_region, self.config.account_id, input.name
        );

        let existing = self.metric_stream_store.get(&input.name);
        let creation_date = existing.as_ref().map_or(now, |e| e.creation_date);

        let include_filters = input
            .include_filters
            .into_iter()
            .map(|f| (f.namespace.unwrap_or_default(), f.metric_names))
            .collect();

        let exclude_filters = input
            .exclude_filters
            .into_iter()
            .map(|f| (f.namespace.unwrap_or_default(), f.metric_names))
            .collect();

        let statistics_configurations: Vec<String> = input
            .statistics_configurations
            .iter()
            .map(|sc| serde_json::to_string(sc).unwrap_or_default())
            .collect();

        let record = MetricStreamRecord {
            name: input.name,
            arn: arn.clone(),
            firehose_arn: input.firehose_arn,
            role_arn: input.role_arn,
            output_format: input.output_format.as_str().to_owned(),
            include_filters,
            exclude_filters,
            state: "running".to_owned(),
            creation_date,
            last_update_date: now,
            include_linked_accounts_metrics: input.include_linked_accounts_metrics.unwrap_or(false),
            statistics_configurations,
        };

        let returned_arn = self.metric_stream_store.put(record);

        if !input.tags.is_empty() {
            self.alarm_store.set_tags(&returned_arn, input.tags);
        }

        Ok(PutMetricStreamOutput {
            arn: Some(returned_arn),
        })
    }

    /// Delete a metric stream by name.
    #[instrument(skip(self, input))]
    pub fn delete_metric_stream(
        &self,
        input: DeleteMetricStreamInput,
    ) -> Result<DeleteMetricStreamOutput, CloudWatchError> {
        // AWS does not error if the stream doesn't exist.
        let _ = self.metric_stream_store.delete(&input.name);
        Ok(DeleteMetricStreamOutput {})
    }

    /// List all metric streams.
    #[instrument(skip(self, input))]
    pub fn list_metric_streams(
        &self,
        input: ListMetricStreamsInput,
    ) -> Result<ListMetricStreamsOutput, CloudWatchError> {
        let mut records = self.metric_stream_store.list();

        if let Some(max) = input.max_results {
            #[allow(clippy::cast_sign_loss)]
            records.truncate(max as usize);
        }

        let entries: Vec<MetricStreamEntry> = records
            .into_iter()
            .map(|r| {
                #[allow(clippy::cast_possible_truncation)]
                let creation_date = chrono::DateTime::from_timestamp(r.creation_date as i64, 0);
                #[allow(clippy::cast_possible_truncation)]
                let last_update_date =
                    chrono::DateTime::from_timestamp(r.last_update_date as i64, 0);
                MetricStreamEntry {
                    name: Some(r.name),
                    arn: Some(r.arn),
                    firehose_arn: Some(r.firehose_arn),
                    creation_date,
                    last_update_date,
                    output_format: Some(MetricStreamOutputFormat::from(r.output_format.as_str())),
                    state: Some(r.state),
                }
            })
            .collect();

        Ok(ListMetricStreamsOutput {
            entries,
            next_token: None,
        })
    }

    /// Get details for a metric stream by name.
    #[instrument(skip(self, input))]
    pub fn get_metric_stream(
        &self,
        input: GetMetricStreamInput,
    ) -> Result<GetMetricStreamOutput, CloudWatchError> {
        let record = self.metric_stream_store.get(&input.name).ok_or_else(|| {
            CloudWatchError::with_message(
                CloudWatchErrorCode::ResourceNotFoundException,
                format!("Metric stream {} does not exist.", input.name),
            )
        })?;

        let include_filters = record
            .include_filters
            .into_iter()
            .map(
                |(ns, metric_names)| ruststack_cloudwatch_model::types::MetricStreamFilter {
                    namespace: Some(ns),
                    metric_names,
                },
            )
            .collect();

        let exclude_filters = record
            .exclude_filters
            .into_iter()
            .map(
                |(ns, metric_names)| ruststack_cloudwatch_model::types::MetricStreamFilter {
                    namespace: Some(ns),
                    metric_names,
                },
            )
            .collect();

        let statistics_configurations = record
            .statistics_configurations
            .iter()
            .filter_map(|s| serde_json::from_str(s).ok())
            .collect();

        #[allow(clippy::cast_possible_truncation)]
        let creation_date = chrono::DateTime::from_timestamp(record.creation_date as i64, 0);
        #[allow(clippy::cast_possible_truncation)]
        let last_update_date = chrono::DateTime::from_timestamp(record.last_update_date as i64, 0);

        Ok(GetMetricStreamOutput {
            name: Some(record.name),
            arn: Some(record.arn),
            firehose_arn: Some(record.firehose_arn),
            role_arn: Some(record.role_arn),
            output_format: Some(MetricStreamOutputFormat::from(
                record.output_format.as_str(),
            )),
            creation_date,
            last_update_date,
            state: Some(record.state),
            include_filters,
            exclude_filters,
            include_linked_accounts_metrics: Some(record.include_linked_accounts_metrics),
            statistics_configurations,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruststack_cloudwatch_model::input::{
        DeleteAlarmsInput, DescribeAlarmsInput, PutMetricAlarmInput, PutMetricDataInput,
        SetAlarmStateInput,
    };
    use ruststack_cloudwatch_model::types::{ComparisonOperator, MetricDatum, StateValue};

    fn make_provider() -> RustStackCloudWatch {
        RustStackCloudWatch::new(Arc::new(CloudWatchConfig::default()))
    }

    #[test]
    fn test_should_put_and_list_metrics() {
        let provider = make_provider();
        let input = PutMetricDataInput {
            namespace: "TestNs".to_owned(),
            metric_data: vec![MetricDatum {
                metric_name: "CPU".to_owned(),
                value: Some(42.0),
                ..Default::default()
            }],
            entity_metric_data: Vec::new(),
            strict_entity_validation: None,
        };

        provider.put_metric_data(input).ok();

        let list = provider
            .list_metrics(ListMetricsInput {
                namespace: Some("TestNs".to_owned()),
                ..Default::default()
            })
            .ok();

        assert!(list.is_some());
        assert_eq!(list.as_ref().map_or(0, |o| o.metrics.len()), 1);
    }

    #[test]
    fn test_should_put_and_describe_alarm() {
        let provider = make_provider();

        provider
            .put_metric_alarm(PutMetricAlarmInput {
                alarm_name: "test-alarm".to_owned(),
                namespace: Some("TestNs".to_owned()),
                metric_name: Some("CPU".to_owned()),
                comparison_operator: ComparisonOperator::GreaterThanThreshold,
                evaluation_periods: 1,
                threshold: Some(80.0),
                ..Default::default()
            })
            .ok();

        let result = provider
            .describe_alarms(DescribeAlarmsInput {
                alarm_names: vec!["test-alarm".to_owned()],
                ..Default::default()
            })
            .ok();

        assert!(result.is_some());
        assert_eq!(result.as_ref().map_or(0, |o| o.metric_alarms.len()), 1);
    }

    #[test]
    fn test_should_set_alarm_state() {
        let provider = make_provider();

        provider
            .put_metric_alarm(PutMetricAlarmInput {
                alarm_name: "state-test".to_owned(),
                comparison_operator: ComparisonOperator::GreaterThanThreshold,
                evaluation_periods: 1,
                ..Default::default()
            })
            .ok();

        let result = provider.set_alarm_state(SetAlarmStateInput {
            alarm_name: "state-test".to_owned(),
            state_value: StateValue::Alarm,
            state_reason: "Testing".to_owned(),
            state_reason_data: None,
        });

        assert!(result.is_ok());
    }

    #[test]
    fn test_should_delete_alarm() {
        let provider = make_provider();

        provider
            .put_metric_alarm(PutMetricAlarmInput {
                alarm_name: "del-alarm".to_owned(),
                comparison_operator: ComparisonOperator::GreaterThanThreshold,
                evaluation_periods: 1,
                ..Default::default()
            })
            .ok();

        let del = provider.delete_alarms(DeleteAlarmsInput {
            alarm_names: vec!["del-alarm".to_owned()],
        });
        assert!(del.is_ok());

        let del2 = provider.delete_alarms(DeleteAlarmsInput {
            alarm_names: vec!["del-alarm".to_owned()],
        });
        assert!(del2.is_err());
    }

    #[test]
    fn test_should_put_and_get_dashboard() {
        let provider = make_provider();

        provider
            .put_dashboard(PutDashboardInput {
                dashboard_name: "test-dash".to_owned(),
                dashboard_body: r#"{"widgets":[]}"#.to_owned(),
            })
            .ok();

        let result = provider.get_dashboard(GetDashboardInput {
            dashboard_name: "test-dash".to_owned(),
        });

        assert!(result.is_ok());
        let output = result.ok();
        assert_eq!(
            output.as_ref().and_then(|o| o.dashboard_name.as_deref()),
            Some("test-dash")
        );
    }
}
