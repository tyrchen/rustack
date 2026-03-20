//! Time-series metric storage engine.

use std::collections::BTreeMap;

use dashmap::DashMap;
use ruststack_cloudwatch_model::types::{Dimension, StandardUnit};

/// Unique identifier for a metric series.
///
/// Dimensions are sorted by name for consistent lookup regardless of insertion order.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct MetricKey {
    /// CloudWatch namespace (e.g., `"AWS/EC2"`, `"MyApp"`).
    pub namespace: String,
    /// Metric name (e.g., `"CPUUtilization"`, `"RequestCount"`).
    pub metric_name: String,
    /// Sorted dimensions.
    pub dimensions: Vec<Dimension>,
}

impl MetricKey {
    /// Create a new `MetricKey` with dimensions normalized (sorted by name).
    #[must_use]
    pub fn new(namespace: String, metric_name: String, mut dimensions: Vec<Dimension>) -> Self {
        dimensions.sort_by(|a, b| a.name.cmp(&b.name));
        Self {
            namespace,
            metric_name,
            dimensions,
        }
    }
}

/// A single data point within a metric series.
#[derive(Debug, Clone)]
pub struct DataPoint {
    /// Timestamp in epoch milliseconds.
    pub timestamp_ms: i64,
    /// Simple scalar value.
    pub value: Option<f64>,
    /// Pre-aggregated statistics.
    pub statistic_values: Option<StatisticSet>,
    /// Array of values (high-cardinality data).
    pub values: Vec<f64>,
    /// Array of counts corresponding to values.
    pub counts: Vec<f64>,
    /// Unit for this data point.
    pub unit: Option<StandardUnit>,
}

/// Pre-aggregated statistic set from `PutMetricData` `StatisticValues`.
#[derive(Debug, Clone)]
pub struct StatisticSet {
    /// Number of samples.
    pub sample_count: f64,
    /// Sum of values.
    pub sum: f64,
    /// Minimum value.
    pub minimum: f64,
    /// Maximum value.
    pub maximum: f64,
}

/// Time-series data for a single metric.
#[derive(Debug, Default)]
pub struct MetricSeries {
    /// Data points keyed by timestamp (epoch ms) for efficient range queries.
    pub data_points: BTreeMap<i64, Vec<DataPoint>>,
    /// Unit for this metric series (set by first `PutMetricData` call).
    pub unit: Option<StandardUnit>,
}

/// Top-level metric store.
#[derive(Debug)]
pub struct MetricStore {
    /// All metric series keyed by `MetricKey`.
    series: DashMap<MetricKey, MetricSeries>,
    /// Maximum data points per metric series.
    max_points_per_series: usize,
}

impl MetricStore {
    /// Create a new metric store.
    #[must_use]
    pub fn new(max_points_per_series: usize) -> Self {
        Self {
            series: DashMap::new(),
            max_points_per_series,
        }
    }

    /// Insert a data point into the store.
    pub fn insert(&self, key: MetricKey, data_point: DataPoint) {
        let mut entry = self.series.entry(key).or_default();
        let series = entry.value_mut();

        if series.unit.is_none() {
            series.unit.clone_from(&data_point.unit);
        }

        series
            .data_points
            .entry(data_point.timestamp_ms)
            .or_default()
            .push(data_point);

        // Enforce max points limit: remove oldest if over limit.
        let total: usize = series.data_points.values().map(Vec::len).sum();
        if total > self.max_points_per_series {
            let excess = total - self.max_points_per_series;
            let mut removed = 0;
            let keys_to_check: Vec<i64> = series.data_points.keys().copied().collect();
            for ts in keys_to_check {
                if removed >= excess {
                    break;
                }
                if let Some(points) = series.data_points.get_mut(&ts) {
                    let drain_count = (excess - removed).min(points.len());
                    points.drain(..drain_count);
                    removed += drain_count;
                    if points.is_empty() {
                        series.data_points.remove(&ts);
                    }
                }
            }
        }
    }

    /// Get a reference to a metric series by key.
    #[must_use]
    pub fn get(
        &self,
        key: &MetricKey,
    ) -> Option<dashmap::mapref::one::Ref<'_, MetricKey, MetricSeries>> {
        self.series.get(key)
    }

    /// Iterate all metric keys in the store.
    #[must_use]
    pub fn keys(&self) -> Vec<MetricKey> {
        self.series
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Get the number of distinct metric series.
    #[must_use]
    pub fn len(&self) -> usize {
        self.series.len()
    }

    /// Check if the store is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.series.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key(namespace: &str, name: &str) -> MetricKey {
        MetricKey::new(namespace.to_owned(), name.to_owned(), vec![])
    }

    fn test_data_point(ts: i64, value: f64) -> DataPoint {
        DataPoint {
            timestamp_ms: ts,
            value: Some(value),
            statistic_values: None,
            values: vec![],
            counts: vec![],
            unit: None,
        }
    }

    #[test]
    fn test_should_insert_and_retrieve_data_point() {
        let store = MetricStore::new(100_000);
        let key = test_key("MyApp", "RequestCount");
        store.insert(key.clone(), test_data_point(1000, 42.0));

        let series = store.get(&key).unwrap();
        assert_eq!(series.data_points.len(), 1);
        assert_eq!(series.data_points[&1000][0].value, Some(42.0));
    }

    #[test]
    fn test_should_normalize_dimension_order() {
        let key1 = MetricKey::new(
            "ns".to_owned(),
            "m".to_owned(),
            vec![
                Dimension {
                    name: "B".to_owned(),
                    value: "2".to_owned(),
                },
                Dimension {
                    name: "A".to_owned(),
                    value: "1".to_owned(),
                },
            ],
        );
        let key2 = MetricKey::new(
            "ns".to_owned(),
            "m".to_owned(),
            vec![
                Dimension {
                    name: "A".to_owned(),
                    value: "1".to_owned(),
                },
                Dimension {
                    name: "B".to_owned(),
                    value: "2".to_owned(),
                },
            ],
        );
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_should_enforce_max_points() {
        let store = MetricStore::new(5);
        let key = test_key("ns", "m");
        for i in 0..10 {
            #[allow(clippy::cast_precision_loss)]
            store.insert(key.clone(), test_data_point(i, i as f64));
        }
        let series = store.get(&key).unwrap();
        let total: usize = series.data_points.values().map(Vec::len).sum();
        assert!(total <= 5);
    }
}
