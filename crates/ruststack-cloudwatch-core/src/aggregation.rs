//! Statistical aggregation engine for CloudWatch metrics.
//!
//! Computes Sum, Average, Minimum, Maximum, `SampleCount` over
//! period-aligned time buckets.

use std::collections::BTreeMap;

use ruststack_cloudwatch_model::types::{StandardUnit, Statistic};

use crate::metric_store::DataPoint;

/// Result of aggregating a period bucket.
#[derive(Debug, Clone)]
pub struct AggregatedDatapoint {
    /// Bucket start timestamp in epoch seconds.
    pub timestamp: i64,
    /// Sum of all values in the bucket.
    pub sum: Option<f64>,
    /// Average value in the bucket.
    pub average: Option<f64>,
    /// Minimum value in the bucket.
    pub minimum: Option<f64>,
    /// Maximum value in the bucket.
    pub maximum: Option<f64>,
    /// Number of samples in the bucket.
    pub sample_count: Option<f64>,
    /// Unit for this data point.
    pub unit: Option<StandardUnit>,
}

/// Aggregate data points into period-aligned buckets and compute statistics.
///
/// # Arguments
/// * `data_points` - Raw data points within the requested time range
/// * `start_time` - Start of the query range (epoch seconds)
/// * `end_time` - End of the query range (epoch seconds)
/// * `period` - Aggregation period in seconds (>= 60, multiple of 60)
/// * `statistics` - Which statistics to compute
#[must_use]
pub fn aggregate_statistics(
    data_points: &BTreeMap<i64, Vec<DataPoint>>,
    start_time: i64,
    end_time: i64,
    period: i64,
    statistics: &[Statistic],
) -> Vec<AggregatedDatapoint> {
    let start_ms = start_time * 1000;
    let end_ms = end_time * 1000;
    let period_ms = period * 1000;

    // Collect data points in range.
    let points_in_range: Vec<&DataPoint> = data_points
        .range(start_ms..end_ms)
        .flat_map(|(_, points)| points.iter())
        .collect();

    if points_in_range.is_empty() {
        return Vec::new();
    }

    // Group points into period-aligned buckets.
    let mut buckets: BTreeMap<i64, Vec<&DataPoint>> = BTreeMap::new();
    for point in &points_in_range {
        let bucket_start = (point.timestamp_ms / period_ms) * period_ms;
        buckets.entry(bucket_start).or_default().push(point);
    }

    // Compute statistics for each bucket.
    let mut results = Vec::with_capacity(buckets.len());
    for (bucket_start, bucket_points) in &buckets {
        let aggregated = compute_bucket_statistics(*bucket_start / 1000, bucket_points, statistics);
        results.push(aggregated);
    }

    results
}

/// Compute statistics for a single period bucket.
fn compute_bucket_statistics(
    timestamp: i64,
    points: &[&DataPoint],
    statistics: &[Statistic],
) -> AggregatedDatapoint {
    let mut sum = 0.0_f64;
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    let mut sample_count = 0.0_f64;

    for point in points {
        if let Some(ss) = &point.statistic_values {
            sum += ss.sum;
            if ss.minimum < min {
                min = ss.minimum;
            }
            if ss.maximum > max {
                max = ss.maximum;
            }
            sample_count += ss.sample_count;
        } else if !point.values.is_empty() {
            for (i, val) in point.values.iter().enumerate() {
                let count = point.counts.get(i).copied().unwrap_or(1.0);
                sum += val * count;
                if *val < min {
                    min = *val;
                }
                if *val > max {
                    max = *val;
                }
                sample_count += count;
            }
        } else if let Some(val) = point.value {
            sum += val;
            if val < min {
                min = val;
            }
            if val > max {
                max = val;
            }
            sample_count += 1.0;
        }
    }

    if sample_count == 0.0 {
        return AggregatedDatapoint {
            timestamp,
            sum: None,
            average: None,
            minimum: None,
            maximum: None,
            sample_count: None,
            unit: points.first().and_then(|p| p.unit.clone()),
        };
    }

    let average = sum / sample_count;

    AggregatedDatapoint {
        timestamp,
        sum: if statistics.contains(&Statistic::Sum) {
            Some(sum)
        } else {
            None
        },
        average: if statistics.contains(&Statistic::Average) {
            Some(average)
        } else {
            None
        },
        minimum: if statistics.contains(&Statistic::Minimum) {
            Some(min)
        } else {
            None
        },
        maximum: if statistics.contains(&Statistic::Maximum) {
            Some(max)
        } else {
            None
        },
        sample_count: if statistics.contains(&Statistic::SampleCount) {
            Some(sample_count)
        } else {
            None
        },
        unit: points.first().and_then(|p| p.unit.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metric_store::StatisticSet;

    fn make_point(ts_ms: i64, value: f64) -> DataPoint {
        DataPoint {
            timestamp_ms: ts_ms,
            value: Some(value),
            statistic_values: None,
            values: vec![],
            counts: vec![],
            unit: None,
        }
    }

    fn make_statistic_set_point(ts_ms: i64, sum: f64, min: f64, max: f64, count: f64) -> DataPoint {
        DataPoint {
            timestamp_ms: ts_ms,
            value: None,
            statistic_values: Some(StatisticSet {
                sample_count: count,
                sum,
                minimum: min,
                maximum: max,
            }),
            values: vec![],
            counts: vec![],
            unit: None,
        }
    }

    #[test]
    fn test_should_aggregate_simple_values() {
        let mut data = BTreeMap::new();
        data.insert(60_000, vec![make_point(60_000, 10.0)]);
        data.insert(120_000, vec![make_point(120_000, 20.0)]);
        data.insert(180_000, vec![make_point(180_000, 30.0)]);

        let stats = vec![Statistic::Sum, Statistic::Average, Statistic::SampleCount];
        let result = aggregate_statistics(&data, 0, 300, 300, &stats);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].sum, Some(60.0));
        assert_eq!(result[0].average, Some(20.0));
        assert_eq!(result[0].sample_count, Some(3.0));
    }

    #[test]
    fn test_should_aggregate_statistic_values() {
        let mut data = BTreeMap::new();
        data.insert(
            60_000,
            vec![make_statistic_set_point(60_000, 100.0, 5.0, 50.0, 10.0)],
        );
        data.insert(
            120_000,
            vec![make_statistic_set_point(120_000, 200.0, 2.0, 80.0, 20.0)],
        );

        let stats = vec![
            Statistic::Sum,
            Statistic::Minimum,
            Statistic::Maximum,
            Statistic::SampleCount,
        ];
        let result = aggregate_statistics(&data, 0, 300, 300, &stats);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].sum, Some(300.0));
        assert_eq!(result[0].minimum, Some(2.0));
        assert_eq!(result[0].maximum, Some(80.0));
        assert_eq!(result[0].sample_count, Some(30.0));
    }

    #[test]
    fn test_should_return_empty_for_no_data() {
        let data = BTreeMap::new();
        let stats = vec![Statistic::Sum];
        let result = aggregate_statistics(&data, 0, 300, 300, &stats);
        assert!(result.is_empty());
    }

    #[test]
    fn test_should_create_multiple_buckets() {
        let mut data = BTreeMap::new();
        data.insert(60_000, vec![make_point(60_000, 10.0)]);
        data.insert(360_000, vec![make_point(360_000, 20.0)]);

        let stats = vec![Statistic::Sum];
        let result = aggregate_statistics(&data, 0, 600, 300, &stats);

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].sum, Some(10.0));
        assert_eq!(result[1].sum, Some(20.0));
    }

    #[test]
    fn test_should_aggregate_values_with_counts() {
        let mut data = BTreeMap::new();
        data.insert(
            60_000,
            vec![DataPoint {
                timestamp_ms: 60_000,
                value: None,
                statistic_values: None,
                values: vec![10.0, 20.0, 30.0],
                counts: vec![2.0, 3.0, 5.0],
                unit: None,
            }],
        );

        let stats = vec![Statistic::Sum, Statistic::SampleCount, Statistic::Average];
        let result = aggregate_statistics(&data, 0, 300, 300, &stats);

        assert_eq!(result.len(), 1);
        // Sum = 10*2 + 20*3 + 30*5 = 20 + 60 + 150 = 230
        assert_eq!(result[0].sum, Some(230.0));
        // Count = 2 + 3 + 5 = 10
        assert_eq!(result[0].sample_count, Some(10.0));
        // Average = 230/10 = 23
        assert_eq!(result[0].average, Some(23.0));
    }
}
