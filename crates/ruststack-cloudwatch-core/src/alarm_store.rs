//! Alarm configuration and state management.

use dashmap::DashMap;
use ruststack_cloudwatch_model::types::{
    AlarmHistoryItem, CompositeAlarm, Dimension, MetricAlarm, Tag,
};

/// Alarm store holding all alarm configurations and current state.
#[derive(Debug)]
pub struct AlarmStore {
    /// Metric alarms keyed by alarm name.
    metric_alarms: DashMap<String, MetricAlarm>,
    /// Composite alarms keyed by alarm name.
    composite_alarms: DashMap<String, CompositeAlarm>,
    /// Alarm history entries keyed by alarm name.
    history: DashMap<String, Vec<AlarmHistoryItem>>,
    /// Tags keyed by resource ARN.
    tags: DashMap<String, Vec<Tag>>,
}

impl AlarmStore {
    /// Create a new alarm store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            metric_alarms: DashMap::new(),
            composite_alarms: DashMap::new(),
            history: DashMap::new(),
            tags: DashMap::new(),
        }
    }

    /// Store a metric alarm (insert or update).
    pub fn put_metric_alarm(&self, name: &str, alarm: MetricAlarm) {
        self.metric_alarms.insert(name.to_owned(), alarm);
    }

    /// Get a metric alarm by name.
    #[must_use]
    pub fn get_metric_alarm(
        &self,
        name: &str,
    ) -> Option<dashmap::mapref::one::Ref<'_, String, MetricAlarm>> {
        self.metric_alarms.get(name)
    }

    /// Get a mutable reference to a metric alarm by name.
    #[must_use]
    pub fn get_metric_alarm_mut(
        &self,
        name: &str,
    ) -> Option<dashmap::mapref::one::RefMut<'_, String, MetricAlarm>> {
        self.metric_alarms.get_mut(name)
    }

    /// Delete a metric alarm by name.
    #[must_use]
    pub fn delete_metric_alarm(&self, name: &str) -> bool {
        self.metric_alarms.remove(name).is_some()
    }

    /// List all metric alarms.
    #[must_use]
    pub fn list_metric_alarms(&self) -> Vec<MetricAlarm> {
        self.metric_alarms
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Store a composite alarm.
    pub fn put_composite_alarm(&self, name: &str, alarm: CompositeAlarm) {
        self.composite_alarms.insert(name.to_owned(), alarm);
    }

    /// Get a composite alarm by name.
    #[must_use]
    pub fn get_composite_alarm(
        &self,
        name: &str,
    ) -> Option<dashmap::mapref::one::Ref<'_, String, CompositeAlarm>> {
        self.composite_alarms.get(name)
    }

    /// Get a mutable composite alarm reference.
    #[must_use]
    pub fn get_composite_alarm_mut(
        &self,
        name: &str,
    ) -> Option<dashmap::mapref::one::RefMut<'_, String, CompositeAlarm>> {
        self.composite_alarms.get_mut(name)
    }

    /// Delete a composite alarm by name.
    #[must_use]
    pub fn delete_composite_alarm(&self, name: &str) -> bool {
        self.composite_alarms.remove(name).is_some()
    }

    /// List all composite alarms.
    #[must_use]
    pub fn list_composite_alarms(&self) -> Vec<CompositeAlarm> {
        self.composite_alarms
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Check if an alarm exists (metric or composite).
    #[must_use]
    pub fn alarm_exists(&self, name: &str) -> bool {
        self.metric_alarms.contains_key(name) || self.composite_alarms.contains_key(name)
    }

    /// Get alarm ARN (metric or composite).
    #[must_use]
    pub fn get_alarm_arn(&self, name: &str) -> Option<String> {
        if let Some(a) = self.metric_alarms.get(name) {
            return a.alarm_arn.clone();
        }
        if let Some(a) = self.composite_alarms.get(name) {
            return a.alarm_arn.clone();
        }
        None
    }

    /// Record an alarm history entry.
    pub fn record_history(&self, alarm_name: &str, item: AlarmHistoryItem) {
        self.history
            .entry(alarm_name.to_owned())
            .or_default()
            .push(item);
    }

    /// Get alarm history entries.
    #[must_use]
    pub fn get_history(&self, alarm_name: Option<&str>) -> Vec<AlarmHistoryItem> {
        match alarm_name {
            Some(name) => self
                .history
                .get(name)
                .map(|v| v.value().clone())
                .unwrap_or_default(),
            None => self
                .history
                .iter()
                .flat_map(|entry| entry.value().clone())
                .collect(),
        }
    }

    /// Set tags for a resource ARN (merge: update existing, add new).
    pub fn set_tags(&self, arn: &str, new_tags: Vec<Tag>) {
        let mut entry = self.tags.entry(arn.to_owned()).or_default();
        let existing = entry.value_mut();
        for tag in new_tags {
            if let Some(pos) = existing.iter().position(|t| t.key == tag.key) {
                existing[pos] = tag;
            } else {
                existing.push(tag);
            }
        }
    }

    /// Remove tags by key for a resource ARN.
    pub fn remove_tags(&self, arn: &str, keys: &[String]) {
        if let Some(mut entry) = self.tags.get_mut(arn) {
            entry.value_mut().retain(|t| !keys.contains(&t.key));
        }
    }

    /// Get tags for a resource ARN.
    #[must_use]
    pub fn get_tags(&self, arn: &str) -> Vec<Tag> {
        self.tags
            .get(arn)
            .map(|v| v.value().clone())
            .unwrap_or_default()
    }

    /// Find metric alarms monitoring a specific metric.
    #[must_use]
    pub fn find_alarms_for_metric(
        &self,
        namespace: &str,
        metric_name: &str,
        dimensions: Option<&[Dimension]>,
        period: Option<i32>,
        statistic: Option<&str>,
    ) -> Vec<MetricAlarm> {
        self.metric_alarms
            .iter()
            .filter(|entry| {
                let a = entry.value();
                let ns_match = a.namespace.as_deref() == Some(namespace);
                let mn_match = a.metric_name.as_deref() == Some(metric_name);
                let dim_match = dimensions.is_none_or(|filter_dims| a.dimensions == *filter_dims);
                let period_match = period.is_none_or(|p| a.period == Some(p));
                let stat_match = statistic
                    .is_none_or(|s| a.statistic.as_ref().is_some_and(|as_| as_.as_str() == s));
                ns_match && mn_match && dim_match && period_match && stat_match
            })
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Enable or disable alarm actions for a list of alarm names.
    pub fn set_actions_enabled(&self, alarm_names: &[String], enabled: bool) {
        for name in alarm_names {
            if let Some(mut alarm) = self.metric_alarms.get_mut(name) {
                alarm.actions_enabled = Some(enabled);
            }
            if let Some(mut alarm) = self.composite_alarms.get_mut(name) {
                alarm.actions_enabled = Some(enabled);
            }
        }
    }
}

impl Default for AlarmStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use ruststack_cloudwatch_model::types::StateValue;

    fn make_alarm(name: &str) -> MetricAlarm {
        MetricAlarm {
            alarm_name: Some(name.to_owned()),
            alarm_arn: Some(format!(
                "arn:aws:cloudwatch:us-east-1:000000000000:alarm:{name}"
            )),
            namespace: Some("TestNs".to_owned()),
            metric_name: Some("TestMetric".to_owned()),
            state_value: Some(StateValue::InsufficientData),
            state_reason: Some("Unchecked: Initial alarm creation".to_owned()),
            actions_enabled: Some(true),
            ..Default::default()
        }
    }

    #[test]
    fn test_should_store_and_retrieve_alarm() {
        let store = AlarmStore::new();
        store.put_metric_alarm("test-alarm", make_alarm("test-alarm"));
        assert!(store.get_metric_alarm("test-alarm").is_some());
        assert!(store.get_metric_alarm("missing").is_none());
    }

    #[test]
    fn test_should_delete_alarm() {
        let store = AlarmStore::new();
        store.put_metric_alarm("test-alarm", make_alarm("test-alarm"));
        assert!(store.delete_metric_alarm("test-alarm"));
        assert!(store.get_metric_alarm("test-alarm").is_none());
    }

    #[test]
    fn test_should_manage_tags() {
        let store = AlarmStore::new();
        let arn = "arn:aws:cloudwatch:us-east-1:000000000000:alarm:test";
        store.set_tags(
            arn,
            vec![Tag {
                key: "Env".to_owned(),
                value: "Prod".to_owned(),
            }],
        );
        let tags = store.get_tags(arn);
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].key, "Env");
    }
}
