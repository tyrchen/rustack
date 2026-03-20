//! Metric stream metadata storage (no actual streaming).

use dashmap::DashMap;

/// A stored metric stream configuration.
#[derive(Debug, Clone)]
pub struct MetricStreamRecord {
    /// Stream name.
    pub name: String,
    /// Stream ARN.
    pub arn: String,
    /// Firehose ARN.
    pub firehose_arn: String,
    /// IAM role ARN.
    pub role_arn: String,
    /// Output format.
    pub output_format: String,
    /// Include/exclude filters (serialized).
    pub include_filters: Vec<(String, Vec<String>)>,
    /// Exclude filters.
    pub exclude_filters: Vec<(String, Vec<String>)>,
    /// Stream state (always "running" for local dev).
    pub state: String,
    /// Creation date (epoch seconds).
    pub creation_date: f64,
    /// Last update date (epoch seconds).
    pub last_update_date: f64,
    /// Include linked accounts metrics.
    pub include_linked_accounts_metrics: bool,
    /// Statistics configurations.
    pub statistics_configurations: Vec<String>,
}

/// Metric stream store.
#[derive(Debug, Default)]
pub struct MetricStreamStore {
    streams: DashMap<String, MetricStreamRecord>,
}

impl MetricStreamStore {
    /// Create a new metric stream store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            streams: DashMap::new(),
        }
    }

    /// Store a metric stream.
    #[must_use]
    pub fn put(&self, record: MetricStreamRecord) -> String {
        let arn = record.arn.clone();
        self.streams.insert(record.name.clone(), record);
        arn
    }

    /// Get a metric stream by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<MetricStreamRecord> {
        self.streams.get(name).map(|r| r.value().clone())
    }

    /// Delete a metric stream by name.
    #[must_use]
    pub fn delete(&self, name: &str) -> bool {
        self.streams.remove(name).is_some()
    }

    /// List all metric streams.
    #[must_use]
    pub fn list(&self) -> Vec<MetricStreamRecord> {
        self.streams.iter().map(|e| e.value().clone()).collect()
    }
}
