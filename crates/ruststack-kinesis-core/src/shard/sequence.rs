//! Sequence number generation for Kinesis records.

use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};

/// A 128-bit sequence number for a Kinesis record.
///
/// Displayed as a 56-digit zero-padded decimal string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SequenceNumber(pub u128);

impl SequenceNumber {
    /// Format as a 56-digit zero-padded decimal string.
    #[must_use]
    pub fn to_padded_string(self) -> String {
        format!("{:056}", self.0)
    }
}

impl FromStr for SequenceNumber {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<u128>().map(Self)
    }
}

impl std::fmt::Display for SequenceNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:056}", self.0)
    }
}

/// Generates monotonically increasing sequence numbers for a shard.
///
/// The sequence number is composed of:
/// - High 64 bits: `(creation_epoch_millis << 16) | shard_index`
/// - Low 64 bits: atomically incrementing counter
#[derive(Debug)]
pub struct SequenceNumberGenerator {
    prefix: u128,
    counter: AtomicU64,
}

impl SequenceNumberGenerator {
    /// Create a new generator for a shard.
    #[must_use]
    pub fn new(shard_index: u16, creation_epoch_millis: u64) -> Self {
        let prefix = u128::from((creation_epoch_millis << 16) | u64::from(shard_index)) << 64;
        Self {
            prefix,
            counter: AtomicU64::new(0),
        }
    }

    /// Generate the next sequence number.
    pub fn next(&self) -> SequenceNumber {
        let count = self.counter.fetch_add(1, Ordering::Relaxed);
        SequenceNumber(self.prefix | u128::from(count))
    }

    /// Return the starting sequence number (counter = 0) for this shard.
    #[must_use]
    pub fn starting_sequence_number(&self) -> SequenceNumber {
        SequenceNumber(self.prefix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_generate_increasing_sequence_numbers() {
        let seq_gen = SequenceNumberGenerator::new(0, 1_000_000);
        let s1 = seq_gen.next();
        let s2 = seq_gen.next();
        let s3 = seq_gen.next();
        assert!(s1 < s2);
        assert!(s2 < s3);
    }

    #[test]
    fn test_should_format_as_56_digit_string() {
        let seq = SequenceNumber(12345);
        let s = seq.to_padded_string();
        assert_eq!(s.len(), 56);
        assert!(s.starts_with('0'));
        assert!(s.ends_with("12345"));
    }

    #[test]
    fn test_should_parse_from_string() {
        let s = "00000000000000000000000000000000000000000000000000012345";
        let seq = SequenceNumber::from_str(s).unwrap();
        assert_eq!(seq.0, 12345);
    }

    #[test]
    fn test_should_have_correct_starting_sequence() {
        let seq_gen = SequenceNumberGenerator::new(1, 500);
        let starting = seq_gen.starting_sequence_number();
        let first = seq_gen.next();
        assert_eq!(starting, first);
    }

    #[test]
    fn test_should_differ_by_shard_index() {
        let seq_gen0 = SequenceNumberGenerator::new(0, 1000);
        let seq_gen1 = SequenceNumberGenerator::new(1, 1000);
        assert_ne!(
            seq_gen0.starting_sequence_number(),
            seq_gen1.starting_sequence_number()
        );
    }
}
