//! Hash key types for Kinesis shard routing.

use md5::{Digest, Md5};

/// A 128-bit hash key used for shard routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HashKey(pub u128);

impl HashKey {
    /// The minimum possible hash key (0).
    pub const MIN: Self = Self(0);

    /// The maximum possible hash key (2^128 - 1).
    pub const MAX: Self = Self(u128::MAX);

    /// Compute the hash key from a partition key using MD5.
    #[must_use]
    pub fn from_partition_key(partition_key: &str) -> Self {
        let mut hasher = Md5::new();
        hasher.update(partition_key.as_bytes());
        let result = hasher.finalize();
        Self(u128::from_be_bytes(result.into()))
    }

    /// Parse a hash key from a decimal string representation.
    pub fn from_decimal_str(s: &str) -> Result<Self, std::num::ParseIntError> {
        s.parse::<u128>().map(Self)
    }

    /// Convert to a decimal string representation.
    #[must_use]
    pub fn to_decimal_string(self) -> String {
        self.0.to_string()
    }
}

impl std::fmt::Display for HashKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A range of hash keys [start, end] (inclusive on both ends).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HashKeyRange {
    /// The starting hash key (inclusive).
    pub start: HashKey,
    /// The ending hash key (inclusive).
    pub end: HashKey,
}

impl HashKeyRange {
    /// Check if this range contains the given hash key.
    #[must_use]
    pub fn contains(&self, key: HashKey) -> bool {
        key >= self.start && key <= self.end
    }

    /// Divide the full hash key space [0, 2^128 - 1] into `n` equal ranges.
    ///
    /// Returns `n` non-overlapping ranges that together cover the entire key space.
    #[must_use]
    pub fn divide_evenly(n: u32) -> Vec<Self> {
        if n == 0 {
            return Vec::new();
        }
        if n == 1 {
            return vec![Self {
                start: HashKey::MIN,
                end: HashKey::MAX,
            }];
        }

        let n_u128 = u128::from(n);
        let mut ranges = Vec::with_capacity(n as usize);

        for i in 0..n {
            let i_u128 = u128::from(i);
            let start = if i == 0 {
                0
            } else {
                // (MAX / n) * i + i  (distribute remainder)
                (u128::MAX / n_u128) * i_u128 + i_u128
            };
            let end = if i == n - 1 {
                u128::MAX
            } else {
                let next_i = i_u128 + 1;
                (u128::MAX / n_u128) * next_i + next_i - 1
            };
            ranges.push(Self {
                start: HashKey(start),
                end: HashKey(end),
            });
        }

        ranges
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_hash_partition_key_deterministically() {
        let key1 = HashKey::from_partition_key("test-key");
        let key2 = HashKey::from_partition_key("test-key");
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_should_produce_different_hashes_for_different_keys() {
        let key1 = HashKey::from_partition_key("key-a");
        let key2 = HashKey::from_partition_key("key-b");
        assert_ne!(key1, key2);
    }

    #[test]
    fn test_should_parse_decimal_string() {
        let key = HashKey::from_decimal_str("12345").unwrap();
        assert_eq!(key.0, 12345);
        assert_eq!(key.to_decimal_string(), "12345");
    }

    #[test]
    fn test_should_parse_max_value() {
        let max_str = u128::MAX.to_string();
        let key = HashKey::from_decimal_str(&max_str).unwrap();
        assert_eq!(key, HashKey::MAX);
    }

    #[test]
    fn test_should_contain_key_in_range() {
        let range = HashKeyRange {
            start: HashKey(10),
            end: HashKey(20),
        };
        assert!(range.contains(HashKey(10)));
        assert!(range.contains(HashKey(15)));
        assert!(range.contains(HashKey(20)));
        assert!(!range.contains(HashKey(9)));
        assert!(!range.contains(HashKey(21)));
    }

    #[test]
    fn test_should_divide_evenly_single() {
        let ranges = HashKeyRange::divide_evenly(1);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].start, HashKey::MIN);
        assert_eq!(ranges[0].end, HashKey::MAX);
    }

    #[test]
    fn test_should_divide_evenly_multiple() {
        let ranges = HashKeyRange::divide_evenly(4);
        assert_eq!(ranges.len(), 4);
        // First range starts at 0
        assert_eq!(ranges[0].start, HashKey::MIN);
        // Last range ends at MAX
        assert_eq!(ranges[3].end, HashKey::MAX);
        // Ranges are contiguous
        for i in 0..3 {
            assert_eq!(ranges[i].end.0 + 1, ranges[i + 1].start.0);
        }
    }

    #[test]
    fn test_should_divide_evenly_zero() {
        let ranges = HashKeyRange::divide_evenly(0);
        assert!(ranges.is_empty());
    }

    #[test]
    fn test_should_cover_full_space_with_any_partition_key() {
        let ranges = HashKeyRange::divide_evenly(4);
        // Any hash key should fall in exactly one range
        let key = HashKey::from_partition_key("arbitrary-key");
        let matches: Vec<_> = ranges.iter().filter(|r| r.contains(key)).collect();
        assert_eq!(matches.len(), 1);
    }
}
