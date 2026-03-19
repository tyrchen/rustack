//! In-memory record storage for a single shard.

use std::time::Duration;

use bytes::Bytes;
use chrono::{DateTime, Utc};

use super::sequence::SequenceNumber;

/// A record stored in the shard record log.
#[derive(Debug, Clone)]
pub struct StoredRecord {
    /// The sequence number assigned to this record.
    pub sequence_number: SequenceNumber,
    /// The raw data payload.
    pub data: Bytes,
    /// The partition key used for routing.
    pub partition_key: String,
    /// The explicit hash key, if provided.
    pub explicit_hash_key: Option<String>,
    /// When the record was approximately received.
    pub approximate_arrival_timestamp: DateTime<Utc>,
    /// The encryption type.
    pub encryption_type: Option<String>,
}

/// An in-memory log of records for a single shard.
#[derive(Debug)]
pub struct ShardRecordLog {
    records: Vec<StoredRecord>,
    trim_offset: usize,
    retention_period: Duration,
}

impl ShardRecordLog {
    /// Create a new record log with the given retention period.
    #[must_use]
    pub fn new(retention_period: Duration) -> Self {
        Self {
            records: Vec::new(),
            trim_offset: 0,
            retention_period,
        }
    }

    /// Append a record to the log.
    pub fn append(&mut self, record: StoredRecord) {
        self.records.push(record);
    }

    /// Get records starting from a logical position with a limit.
    ///
    /// `position` is a logical index into the record log (accounting for trim offset).
    /// Returns the records and the next logical position.
    #[must_use]
    pub fn get_records(&self, position: usize, limit: usize) -> (Vec<&StoredRecord>, usize) {
        let start = position.saturating_sub(self.trim_offset);
        if start >= self.records.len() {
            return (Vec::new(), self.trim_offset + self.records.len());
        }
        let end = (start + limit).min(self.records.len());
        let records: Vec<&StoredRecord> = self.records[start..end].iter().collect();
        let next_position = self.trim_offset + end;
        (records, next_position)
    }

    /// Find the logical position at or after the given sequence number.
    #[must_use]
    pub fn position_at_sequence_number(&self, seq: SequenceNumber) -> usize {
        for (i, record) in self.records.iter().enumerate() {
            if record.sequence_number >= seq {
                return self.trim_offset + i;
            }
        }
        self.trim_offset + self.records.len()
    }

    /// Find the logical position after the given sequence number.
    #[must_use]
    pub fn position_after_sequence_number(&self, seq: SequenceNumber) -> usize {
        for (i, record) in self.records.iter().enumerate() {
            if record.sequence_number > seq {
                return self.trim_offset + i;
            }
        }
        self.trim_offset + self.records.len()
    }

    /// Find the logical position at or after the given timestamp.
    #[must_use]
    pub fn position_at_timestamp(&self, ts: DateTime<Utc>) -> usize {
        for (i, record) in self.records.iter().enumerate() {
            if record.approximate_arrival_timestamp >= ts {
                return self.trim_offset + i;
            }
        }
        self.trim_offset + self.records.len()
    }

    /// Return the trim horizon position (the earliest available record).
    #[must_use]
    pub fn trim_horizon_position(&self) -> usize {
        self.trim_offset
    }

    /// Return the latest position (past the last record).
    #[must_use]
    pub fn latest_position(&self) -> usize {
        self.trim_offset + self.records.len()
    }

    /// Remove records older than the retention period.
    pub fn trim_expired(&mut self) {
        let cutoff = Utc::now() - self.retention_period;
        let mut remove_count = 0;
        for record in &self.records {
            if record.approximate_arrival_timestamp < cutoff {
                remove_count += 1;
            } else {
                break;
            }
        }
        if remove_count > 0 {
            self.records.drain(..remove_count);
            self.trim_offset += remove_count;
        }
    }

    /// Return the number of records currently in the log.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Check if the log is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Return the last sequence number in the log, or `None` if empty.
    #[must_use]
    pub fn last_sequence_number(&self) -> Option<String> {
        self.records
            .last()
            .map(|r| r.sequence_number.to_padded_string())
    }

    /// Update the retention period.
    pub fn set_retention_period(&mut self, period: Duration) {
        self.retention_period = period;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_record(seq: u128, partition_key: &str) -> StoredRecord {
        StoredRecord {
            sequence_number: SequenceNumber(seq),
            data: Bytes::from("test-data"),
            partition_key: partition_key.to_owned(),
            explicit_hash_key: None,
            approximate_arrival_timestamp: Utc::now(),
            encryption_type: None,
        }
    }

    #[test]
    fn test_should_append_and_retrieve_records() {
        let mut log = ShardRecordLog::new(Duration::from_hours(24));
        log.append(make_record(1, "pk1"));
        log.append(make_record(2, "pk2"));
        log.append(make_record(3, "pk3"));

        let (records, next_pos) = log.get_records(0, 10);
        assert_eq!(records.len(), 3);
        assert_eq!(next_pos, 3);
    }

    #[test]
    fn test_should_respect_limit() {
        let mut log = ShardRecordLog::new(Duration::from_hours(24));
        log.append(make_record(1, "pk1"));
        log.append(make_record(2, "pk2"));
        log.append(make_record(3, "pk3"));

        let (records, next_pos) = log.get_records(0, 2);
        assert_eq!(records.len(), 2);
        assert_eq!(next_pos, 2);
    }

    #[test]
    fn test_should_find_position_at_sequence_number() {
        let mut log = ShardRecordLog::new(Duration::from_hours(24));
        log.append(make_record(10, "pk1"));
        log.append(make_record(20, "pk2"));
        log.append(make_record(30, "pk3"));

        assert_eq!(log.position_at_sequence_number(SequenceNumber(20)), 1);
        assert_eq!(log.position_after_sequence_number(SequenceNumber(20)), 2);
    }

    #[test]
    fn test_should_return_trim_horizon_and_latest() {
        let mut log = ShardRecordLog::new(Duration::from_hours(24));
        assert_eq!(log.trim_horizon_position(), 0);
        assert_eq!(log.latest_position(), 0);

        log.append(make_record(1, "pk1"));
        log.append(make_record(2, "pk2"));

        assert_eq!(log.trim_horizon_position(), 0);
        assert_eq!(log.latest_position(), 2);
    }

    #[test]
    fn test_should_report_length() {
        let mut log = ShardRecordLog::new(Duration::from_hours(24));
        assert!(log.is_empty());
        assert_eq!(log.len(), 0);
        log.append(make_record(1, "pk1"));
        assert!(!log.is_empty());
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn test_should_return_empty_for_out_of_range_position() {
        let mut log = ShardRecordLog::new(Duration::from_hours(24));
        log.append(make_record(1, "pk1"));

        let (records, _) = log.get_records(100, 10);
        assert!(records.is_empty());
    }
}
