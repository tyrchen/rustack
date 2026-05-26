//! In-memory storage engine for DynamoDB tables.
//!
//! Provides B-Tree indexed storage with concurrent partition-level access via
//! [`DashMap`]. Each partition key maps to a [`BTreeMap`] of sort keys to items,
//! enabling efficient range queries and ordered scans within a partition.
//!
//! # Architecture
//!
//! ```text
//! DashMap<PartitionKey, BTreeMap<SortableAttributeValue, StoredItem>>
//! ```
//!
//! - Partition-level concurrency: different partitions can be read/written concurrently without
//!   contention.
//! - Sort key ordering: within each partition, items are stored in a `BTreeMap` keyed by
//!   [`SortableAttributeValue`], which implements [`Ord`] following DynamoDB comparison rules.
//! - For tables without a sort key, a sentinel value is used as the single BTreeMap key per
//!   partition.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashMap},
    hash::{DefaultHasher, Hash, Hasher},
    ops::Bound,
    sync::atomic::{AtomicU64, Ordering as AtomicOrdering},
};

use dashmap::DashMap;
use rustack_dynamodb_model::{AttributeValue, types::ScalarAttributeType};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::debug;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur during storage operations.
#[derive(Debug, Error)]
pub enum StorageError {
    /// A required key attribute was not found in the item.
    #[error("missing required key attribute: {attr}")]
    MissingKeyAttribute {
        /// The name of the missing attribute.
        attr: String,
    },
    /// A key attribute has the wrong type.
    #[error("key attribute '{attr}' has wrong type: expected {expected}, got {actual}")]
    InvalidKeyType {
        /// The name of the attribute.
        attr: String,
        /// The expected type descriptor.
        expected: String,
        /// The actual type descriptor.
        actual: String,
    },
}

// ---------------------------------------------------------------------------
// Key types
// ---------------------------------------------------------------------------

/// Parsed key schema for a table, containing the partition key definition
/// and an optional sort key definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeySchema {
    /// Partition (HASH) key name and type.
    pub partition_key: KeyAttribute,
    /// Optional sort (RANGE) key name and type.
    pub sort_key: Option<KeyAttribute>,
}

/// A single key attribute definition with its name and scalar type.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyAttribute {
    /// The attribute name.
    pub name: String,
    /// The scalar type (S, N, or B).
    pub attr_type: ScalarAttributeType,
}

/// A primary key consisting of a partition key and an optional sort key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PrimaryKey {
    /// The partition (HASH) key value.
    pub partition_key: AttributeValue,
    /// The optional sort (RANGE) key value.
    pub sort_key: Option<SortableAttributeValue>,
}

// ---------------------------------------------------------------------------
// SortableAttributeValue
// ---------------------------------------------------------------------------

/// A wrapper around key-eligible [`AttributeValue`] variants that implements
/// [`Ord`] for use as `BTreeMap` keys.
///
/// Ordering rules follow DynamoDB semantics:
/// - **Strings (S)**: UTF-8 byte ordering.
/// - **Numbers (N)**: Numeric ordering (parsed as `f64`). Full 38-digit precision would require
///   `bigdecimal`, but `f64` is sufficient for local development use cases.
/// - **Binary (B)**: Byte-by-byte unsigned ordering.
/// - **Sentinel**: A special value used when the table has no sort key. It always compares equal to
///   itself.
#[derive(Debug, Clone)]
pub enum SortableAttributeValue {
    /// String sort key.
    S(String),
    /// Number sort key (stored as the original string representation).
    N(String),
    /// Binary sort key.
    B(bytes::Bytes),
    /// Sentinel value for tables without a sort key.
    Sentinel,
}

impl SortableAttributeValue {
    /// Converts this sortable value back into an [`AttributeValue`].
    ///
    /// Returns `None` for the `Sentinel` variant.
    #[must_use]
    pub fn to_attribute_value(&self) -> Option<AttributeValue> {
        match self {
            Self::S(s) => Some(AttributeValue::S(s.clone())),
            Self::N(n) => Some(AttributeValue::N(n.clone())),
            Self::B(b) => Some(AttributeValue::B(b.clone())),
            Self::Sentinel => None,
        }
    }

    /// Creates a `SortableAttributeValue` from an `AttributeValue`.
    ///
    /// # Errors
    ///
    /// Returns `StorageError::InvalidKeyType` if the attribute value is not
    /// a valid sort key type (S, N, or B).
    pub fn from_attribute_value(
        attr_name: &str,
        value: &AttributeValue,
    ) -> Result<Self, StorageError> {
        match value {
            AttributeValue::S(s) => Ok(Self::S(s.clone())),
            AttributeValue::N(n) => Ok(Self::N(n.clone())),
            AttributeValue::B(b) => Ok(Self::B(b.clone())),
            other => Err(StorageError::InvalidKeyType {
                attr: attr_name.to_owned(),
                expected: "S, N, or B".to_owned(),
                actual: other.type_descriptor().to_owned(),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Arbitrary-precision decimal number comparison
// ---------------------------------------------------------------------------

/// A normalized representation of a decimal number for comparison.
///
/// The number is represented as `sign * 0.{digits} * 10^exponent`, where
/// `digits` is a string of significant digits with no leading or trailing zeros.
/// Zero is represented as `(false, "", 0)`.
#[derive(Debug)]
struct NormalizedNumber {
    /// `true` if the number is negative.
    negative: bool,
    /// The significant digits (no leading/trailing zeros). Empty for zero.
    digits: Vec<u8>,
    /// The base-10 exponent such that the value is `0.{digits} * 10^exponent`.
    exponent: i64,
}

/// Parse a DynamoDB number string into a normalized form for comparison.
///
/// Handles integers, decimals, and scientific notation (e.g., "1.5e10", "-3.14", "0.001").
fn normalize_number(s: &str) -> NormalizedNumber {
    let s = s.trim();
    let (negative, s) = if let Some(rest) = s.strip_prefix('-') {
        (true, rest)
    } else if let Some(rest) = s.strip_prefix('+') {
        (false, rest)
    } else {
        (false, s)
    };

    // Split on 'e' or 'E' for scientific notation.
    let (mantissa_str, explicit_exp) = if let Some(pos) = s.find(['e', 'E']) {
        let exp: i64 = s[pos + 1..].parse().unwrap_or(0);
        (&s[..pos], exp)
    } else {
        (s, 0_i64)
    };

    // Split mantissa into integer and fractional parts.
    let (int_part, frac_part) = if let Some(dot_pos) = mantissa_str.find('.') {
        (&mantissa_str[..dot_pos], &mantissa_str[dot_pos + 1..])
    } else {
        (mantissa_str, "")
    };

    // Collect all digits and compute the exponent.
    // The number is `int_part.frac_part * 10^explicit_exp`.
    // We normalize to `0.{all_digits} * 10^exponent`.
    let mut all_digits: Vec<u8> = Vec::new();
    for c in int_part.chars() {
        if c.is_ascii_digit() {
            all_digits.push(c as u8 - b'0');
        }
    }
    // The exponent starts at int_part length (since we treat all digits as
    // fractional: `0.{int_part}{frac_part} * 10^(len(int_part) + explicit_exp)`).
    // Safety: digit string length is bounded by input and always far below i64::MAX.
    #[allow(clippy::cast_possible_wrap)]
    let int_digits_len = all_digits.len() as i64;
    for c in frac_part.chars() {
        if c.is_ascii_digit() {
            all_digits.push(c as u8 - b'0');
        }
    }

    let exponent = int_digits_len + explicit_exp;

    // Strip leading zeros from all_digits, adjusting exponent.
    let leading_zeros = all_digits.iter().take_while(|&&d| d == 0).count();
    let digits: Vec<u8> = all_digits[leading_zeros..].to_vec();
    // Safety: leading_zeros count is bounded by digit string length.
    #[allow(clippy::cast_possible_wrap)]
    let exponent = exponent - leading_zeros as i64;

    // Strip trailing zeros from digits.
    let mut digits = digits;
    while digits.last() == Some(&0) {
        digits.pop();
    }

    // If no significant digits remain, the number is zero.
    if digits.is_empty() {
        return NormalizedNumber {
            negative: false,
            digits: Vec::new(),
            exponent: 0,
        };
    }

    NormalizedNumber {
        negative,
        digits,
        exponent,
    }
}

/// Compare two DynamoDB number strings with arbitrary precision.
fn compare_number_strings(a: &str, b: &str) -> Ordering {
    let na = normalize_number(a);
    let nb = normalize_number(b);

    let a_zero = na.digits.is_empty();
    let b_zero = nb.digits.is_empty();

    // Handle zeros.
    if a_zero && b_zero {
        return Ordering::Equal;
    }
    if a_zero {
        return if nb.negative {
            Ordering::Greater
        } else {
            Ordering::Less
        };
    }
    if b_zero {
        return if na.negative {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }

    // Compare signs.
    match (na.negative, nb.negative) {
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        _ => {}
    }

    // Both same sign. Compare magnitude, then flip if negative.
    let mag_cmp = compare_magnitude(&na, &nb);
    if na.negative {
        mag_cmp.reverse()
    } else {
        mag_cmp
    }
}

/// Compare the magnitude (absolute value) of two normalized numbers.
fn compare_magnitude(a: &NormalizedNumber, b: &NormalizedNumber) -> Ordering {
    // First compare exponents (which represent the order of magnitude).
    match a.exponent.cmp(&b.exponent) {
        Ordering::Less => return Ordering::Less,
        Ordering::Greater => return Ordering::Greater,
        Ordering::Equal => {}
    }

    // Same exponent: compare digit-by-digit.
    let max_len = a.digits.len().max(b.digits.len());
    for i in 0..max_len {
        let da = a.digits.get(i).copied().unwrap_or(0);
        let db = b.digits.get(i).copied().unwrap_or(0);
        match da.cmp(&db) {
            Ordering::Equal => {}
            other => return other,
        }
    }

    Ordering::Equal
}

impl PartialEq for SortableAttributeValue {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for SortableAttributeValue {}

impl PartialOrd for SortableAttributeValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SortableAttributeValue {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::S(a), Self::S(b)) => a.as_bytes().cmp(b.as_bytes()),
            (Self::N(a), Self::N(b)) => compare_number_strings(a, b),
            (Self::B(a), Self::B(b)) => a.as_ref().cmp(b.as_ref()),
            (Self::Sentinel, Self::Sentinel) => Ordering::Equal,
            // Different variant types should not appear in the same BTreeMap,
            // but we define a deterministic ordering just in case.
            (Self::S(_), _) => Ordering::Less,
            (_, Self::S(_)) => Ordering::Greater,
            (Self::N(_), _) => Ordering::Less,
            (_, Self::N(_)) => Ordering::Greater,
            (Self::B(_), _) => Ordering::Less,
            (_, Self::B(_)) => Ordering::Greater,
        }
    }
}

impl std::hash::Hash for SortableAttributeValue {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
        match self {
            Self::S(s) => s.hash(state),
            Self::N(n) => {
                // Hash using the normalized form so that equivalent numbers
                // (e.g., "1e2" and "100") produce the same hash.
                let norm = normalize_number(n);
                norm.negative.hash(state);
                norm.digits.hash(state);
                norm.exponent.hash(state);
            }
            Self::B(b) => b.hash(state),
            Self::Sentinel => {}
        }
    }
}

// ---------------------------------------------------------------------------
// SortKeyCondition
// ---------------------------------------------------------------------------

/// Conditions that can be applied to sort keys during query operations.
#[derive(Debug, Clone)]
pub enum SortKeyCondition {
    /// Sort key equals the given value.
    Eq(SortableAttributeValue),
    /// Sort key is less than the given value.
    Lt(SortableAttributeValue),
    /// Sort key is less than or equal to the given value.
    Le(SortableAttributeValue),
    /// Sort key is greater than the given value.
    Gt(SortableAttributeValue),
    /// Sort key is greater than or equal to the given value.
    Ge(SortableAttributeValue),
    /// Sort key is between the two given values (inclusive).
    Between(SortableAttributeValue, SortableAttributeValue),
    /// Sort key begins with the given string prefix.
    BeginsWithStr(String),
    /// Sort key begins with the given binary prefix.
    BeginsWithBytes(bytes::Bytes),
}

impl SortKeyCondition {
    /// Tests whether the given [`SortableAttributeValue`] satisfies this condition.
    ///
    /// Used for in-memory filtering when querying items by a secondary index
    /// (where items are not stored in a dedicated B-Tree ordered by the index
    /// sort key).
    #[must_use]
    pub fn matches(&self, value: &SortableAttributeValue) -> bool {
        match self {
            Self::Eq(v) => value.cmp(v) == Ordering::Equal,
            Self::Lt(v) => value.cmp(v) == Ordering::Less,
            Self::Le(v) => value.cmp(v) != Ordering::Greater,
            Self::Gt(v) => value.cmp(v) == Ordering::Greater,
            Self::Ge(v) => value.cmp(v) != Ordering::Less,
            Self::Between(low, high) => {
                value.cmp(low) != Ordering::Less && value.cmp(high) != Ordering::Greater
            }
            Self::BeginsWithStr(prefix) => {
                if let SortableAttributeValue::S(s) = value {
                    s.starts_with(prefix.as_str())
                } else {
                    false
                }
            }
            Self::BeginsWithBytes(prefix) => {
                if let SortableAttributeValue::B(b) = value {
                    b.starts_with(prefix.as_ref())
                } else {
                    false
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// StoredItem
// ---------------------------------------------------------------------------

/// An item stored in the table, containing all attributes.
#[derive(Debug, Clone)]
struct StoredItem {
    /// The full set of attributes for this item, including key attributes.
    attributes: HashMap<String, AttributeValue>,
}

// ---------------------------------------------------------------------------
// TableStorage
// ---------------------------------------------------------------------------

/// In-memory storage for a single DynamoDB table.
///
/// Uses [`DashMap`] for concurrent partition-level access and [`BTreeMap`]
/// for sort key ordering within each partition.
#[derive(Debug)]
pub struct TableStorage {
    /// Partition key -> sorted items within that partition.
    data: DashMap<AttributeValue, BTreeMap<SortableAttributeValue, StoredItem>>,
    /// The key schema for this table.
    key_schema: KeySchema,
    /// Total number of items in the table.
    item_count: AtomicU64,
    /// Total size of all items in bytes (approximate, following DynamoDB rules).
    total_size: AtomicU64,
}

impl TableStorage {
    /// Creates a new empty `TableStorage` with the given key schema.
    #[must_use]
    pub fn new(key_schema: KeySchema) -> Self {
        Self {
            data: DashMap::new(),
            key_schema,
            item_count: AtomicU64::new(0),
            total_size: AtomicU64::new(0),
        }
    }

    /// Returns a reference to the key schema.
    #[must_use]
    pub fn key_schema(&self) -> &KeySchema {
        &self.key_schema
    }

    /// Returns the current item count.
    #[must_use]
    pub fn item_count(&self) -> u64 {
        self.item_count.load(AtomicOrdering::Relaxed)
    }

    /// Returns the total size of all items in bytes.
    #[must_use]
    pub fn total_size_bytes(&self) -> u64 {
        self.total_size.load(AtomicOrdering::Relaxed)
    }

    /// Return all items in deterministic scan order for snapshot export.
    #[must_use]
    pub fn snapshot_items(&self) -> Vec<HashMap<String, AttributeValue>> {
        self.scan(None, None, None, None).0
    }

    /// Inserts or replaces an item in the table.
    ///
    /// Returns the previously stored item if one existed with the same key.
    ///
    /// # Errors
    ///
    /// Returns `StorageError::MissingKeyAttribute` if a required key attribute
    /// is missing from the item, or `StorageError::InvalidKeyType` if a key
    /// attribute has the wrong type.
    pub fn put_item(
        &self,
        item: HashMap<String, AttributeValue>,
    ) -> Result<Option<HashMap<String, AttributeValue>>, StorageError> {
        let primary_key = extract_primary_key(&self.key_schema, &item)?;
        let sort_key = primary_key
            .sort_key
            .unwrap_or(SortableAttributeValue::Sentinel);
        let new_size = calculate_item_size(&item);

        let stored = StoredItem { attributes: item };

        let old_item = {
            let mut partition = self.data.entry(primary_key.partition_key).or_default();
            partition.insert(sort_key, stored)
        };

        if let Some(old) = old_item {
            let old_size = calculate_item_size(&old.attributes);
            // Update total size: add new, subtract old.
            self.total_size.fetch_add(new_size, AtomicOrdering::Relaxed);
            self.total_size.fetch_sub(old_size, AtomicOrdering::Relaxed);
            debug!(old_size, new_size, "replaced existing item");
            Ok(Some(old.attributes))
        } else {
            self.item_count.fetch_add(1, AtomicOrdering::Relaxed);
            self.total_size.fetch_add(new_size, AtomicOrdering::Relaxed);
            debug!(new_size, "inserted new item");
            Ok(None)
        }
    }

    /// Retrieves an item by its primary key.
    ///
    /// Returns `None` if the item does not exist.
    #[must_use]
    pub fn get_item(&self, key: &PrimaryKey) -> Option<HashMap<String, AttributeValue>> {
        let sort_key = key
            .sort_key
            .as_ref()
            .map_or(&SortableAttributeValue::Sentinel, |sk| sk);

        self.data
            .get(&key.partition_key)
            .and_then(|partition| partition.get(sort_key).map(|item| item.attributes.clone()))
    }

    /// Deletes an item by its primary key.
    ///
    /// Returns the deleted item if it existed.
    pub fn delete_item(&self, key: &PrimaryKey) -> Option<HashMap<String, AttributeValue>> {
        let sort_key = key
            .sort_key
            .as_ref()
            .map_or(&SortableAttributeValue::Sentinel, |sk| sk);

        let removed = {
            let mut partition = self.data.get_mut(&key.partition_key)?;
            // If the partition is now empty, we can clean it up later.
            // For now, leave it to avoid holding the DashMap write lock
            // longer than necessary.
            partition.remove(sort_key)?
        };

        let size = calculate_item_size(&removed.attributes);
        self.item_count.fetch_sub(1, AtomicOrdering::Relaxed);
        self.total_size.fetch_sub(size, AtomicOrdering::Relaxed);
        debug!(size, "deleted item");

        Some(removed.attributes)
    }

    /// Queries items in a single partition with optional sort key conditions.
    ///
    /// Returns a tuple of (items, last_evaluated_key). The last evaluated key
    /// is `Some` when the result was truncated by the count limit or the 1 MB
    /// response size cap, indicating that more items are available for pagination.
    #[must_use]
    pub fn query(
        &self,
        partition_key: &AttributeValue,
        sort_condition: Option<&SortKeyCondition>,
        scan_forward: bool,
        limit: Option<usize>,
        exclusive_start_key: Option<&SortableAttributeValue>,
    ) -> (Vec<HashMap<String, AttributeValue>>, Option<PrimaryKey>) {
        /// DynamoDB caps a single Query response at 1 MB.
        const MAX_RESPONSE_BYTES: u64 = 1_048_576;

        let Some(partition) = self.data.get(partition_key) else {
            return (Vec::new(), None);
        };

        let all_items = collect_matching_items(
            &partition,
            sort_condition,
            scan_forward,
            None, // collect all matching items first
            exclusive_start_key,
        );

        // Apply the 1 MB size cap and optional count limit.
        let effective_limit = limit.unwrap_or(usize::MAX);
        let mut selected: Vec<&StoredItem> = Vec::new();
        let mut cumulative_size: u64 = 0;
        let mut size_limited = false;

        for item in all_items.iter().take(effective_limit) {
            let item_size = calculate_item_size(&item.attributes);
            // Always include at least one item even if it exceeds 1 MB on its
            // own, matching DynamoDB behaviour.
            if !selected.is_empty() && cumulative_size + item_size > MAX_RESPONSE_BYTES {
                size_limited = true;
                break;
            }
            cumulative_size += item_size;
            selected.push(item);
        }

        let count_limited =
            !size_limited && selected.len() == effective_limit && effective_limit < all_items.len();
        let has_more = size_limited || count_limited;

        let last_key = if has_more {
            build_last_evaluated_key(
                &selected,
                Some(selected.len()),
                partition_key,
                &self.key_schema,
            )
        } else {
            None
        };

        let result: Vec<_> = selected
            .into_iter()
            .map(|stored| stored.attributes.clone())
            .collect();

        (result, last_key)
    }

    /// Scans all items in the table.
    ///
    /// Returns a tuple of (items, last_evaluated_key). The last evaluated key
    /// is `Some` when the result was truncated by the limit or by the 1 MB
    /// response size cap.
    ///
    /// When `segment` and `total_segments` are provided, only items whose
    /// partition key hashes into the given segment are included (parallel scan).
    #[must_use]
    pub fn scan(
        &self,
        limit: Option<usize>,
        exclusive_start_key: Option<&PrimaryKey>,
        segment: Option<u32>,
        total_segments: Option<u32>,
    ) -> (Vec<HashMap<String, AttributeValue>>, Option<PrimaryKey>) {
        /// DynamoDB caps a single Scan/Query response at 1 MB.
        const MAX_RESPONSE_BYTES: u64 = 1_048_576;

        // Collect all items sorted deterministically by partition key then sort key.
        let mut all_items: Vec<(AttributeValue, SortableAttributeValue, &StoredItem)> = Vec::new();

        // We need to hold all partition refs alive while iterating.
        // Collect partition keys first, then iterate in sorted order.
        let mut partition_entries: Vec<_> = self.data.iter().collect();
        // Sort partitions deterministically by partition key display string.
        partition_entries.sort_by(|a, b| format!("{}", a.key()).cmp(&format!("{}", b.key())));

        // For parallel scan, filter partitions by segment assignment.
        let parallel_filter = segment.zip(total_segments);

        for entry in &partition_entries {
            // When parallel scan is active, skip partitions not assigned to this segment.
            if let Some((seg, total)) = parallel_filter {
                let pk_hash = partition_key_segment(entry.key(), total);
                if pk_hash != seg {
                    continue;
                }
            }
            for (sk, item) in entry.value() {
                all_items.push((entry.key().clone(), sk.clone(), item));
            }
        }

        // Apply exclusive_start_key: skip items up to and including the start key.
        let start_idx = if let Some(start_key) = exclusive_start_key {
            let start_sk = start_key
                .sort_key
                .as_ref()
                .map_or(&SortableAttributeValue::Sentinel, |sk| sk);

            all_items
                .iter()
                .position(|(pk, sk, _)| *pk == start_key.partition_key && *sk == *start_sk)
                .map_or(0, |idx| idx + 1)
        } else {
            0
        };

        let effective_limit = limit.unwrap_or(usize::MAX);

        // Select items respecting both the count limit and the 1 MB size cap.
        let mut selected: Vec<&(AttributeValue, SortableAttributeValue, &StoredItem)> = Vec::new();
        let mut cumulative_size: u64 = 0;
        let mut size_limited = false;

        for entry in all_items.iter().skip(start_idx).take(effective_limit) {
            let item_size = calculate_item_size(&entry.2.attributes);
            // Always include at least one item even if it exceeds 1 MB on its
            // own, matching DynamoDB behaviour.
            if !selected.is_empty() && cumulative_size + item_size > MAX_RESPONSE_BYTES {
                size_limited = true;
                break;
            }
            cumulative_size += item_size;
            selected.push(entry);
        }

        let count_limited = !size_limited
            && selected.len() == effective_limit
            && start_idx + effective_limit < all_items.len();
        let has_more = size_limited || count_limited;

        let last_key = if has_more {
            selected.last().map(|(pk, sk, _)| {
                PrimaryKey {
                    partition_key: pk.clone(),
                    sort_key: sk.to_attribute_value().map(|av| {
                        // This conversion is safe because we know the sort key
                        // came from a valid SortableAttributeValue.
                        SortableAttributeValue::from_attribute_value(
                            &self
                                .key_schema
                                .sort_key
                                .as_ref()
                                .map_or_else(String::new, |k| k.name.clone()),
                            &av,
                        )
                        .unwrap_or(SortableAttributeValue::Sentinel)
                    }),
                }
            })
        } else {
            None
        };

        let result: Vec<_> = selected
            .into_iter()
            .map(|(_, _, item)| item.attributes.clone())
            .collect();

        (result, last_key)
    }
}

// ---------------------------------------------------------------------------
// Parallel scan helpers
// ---------------------------------------------------------------------------

/// Compute which segment a partition key belongs to by hashing the key's
/// display representation and taking the modulus with the total segment count.
pub fn partition_key_segment(pk: &AttributeValue, total_segments: u32) -> u32 {
    let mut hasher = DefaultHasher::new();
    // Use the Display representation for a stable, deterministic hash.
    format!("{pk}").hash(&mut hasher);
    let h = hasher.finish();
    #[allow(clippy::cast_possible_truncation)]
    {
        (h % u64::from(total_segments)) as u32
    }
}

// ---------------------------------------------------------------------------
// Query helpers
// ---------------------------------------------------------------------------

/// Collects items from a partition that match the sort key condition.
fn collect_matching_items<'a>(
    partition: &'a BTreeMap<SortableAttributeValue, StoredItem>,
    sort_condition: Option<&SortKeyCondition>,
    scan_forward: bool,
    limit: Option<usize>,
    exclusive_start_key: Option<&SortableAttributeValue>,
) -> Vec<&'a StoredItem> {
    let effective_limit = limit.unwrap_or(usize::MAX);

    match sort_condition {
        None => collect_all_in_order(
            partition,
            scan_forward,
            effective_limit,
            exclusive_start_key,
        ),
        Some(condition) => collect_with_condition(
            partition,
            condition,
            scan_forward,
            effective_limit,
            exclusive_start_key,
        ),
    }
}

/// Collects all items in a partition, respecting direction and limit.
fn collect_all_in_order<'a>(
    partition: &'a BTreeMap<SortableAttributeValue, StoredItem>,
    scan_forward: bool,
    limit: usize,
    exclusive_start_key: Option<&SortableAttributeValue>,
) -> Vec<&'a StoredItem> {
    if scan_forward {
        let iter: Box<dyn Iterator<Item = (_, _)>> = match exclusive_start_key {
            Some(start) => {
                Box::new(partition.range((Bound::Excluded(start.clone()), Bound::Unbounded)))
            }
            None => Box::new(partition.iter()),
        };
        iter.take(limit).map(|(_, item)| item).collect()
    } else {
        let iter: Box<dyn Iterator<Item = (_, _)>> = match exclusive_start_key {
            Some(start) => Box::new(
                partition
                    .range((Bound::Unbounded, Bound::Excluded(start.clone())))
                    .rev(),
            ),
            None => Box::new(partition.iter().rev()),
        };
        iter.take(limit).map(|(_, item)| item).collect()
    }
}

/// Collects items matching a sort key condition.
fn collect_with_condition<'a>(
    partition: &'a BTreeMap<SortableAttributeValue, StoredItem>,
    condition: &SortKeyCondition,
    scan_forward: bool,
    limit: usize,
    exclusive_start_key: Option<&SortableAttributeValue>,
) -> Vec<&'a StoredItem> {
    match condition {
        SortKeyCondition::Eq(val) => {
            // Exact match - at most one item.
            partition.get(val).into_iter().collect()
        }
        SortKeyCondition::Lt(val) => collect_range(
            partition,
            Bound::Unbounded,
            Bound::Excluded(val.clone()),
            scan_forward,
            limit,
            exclusive_start_key,
        ),
        SortKeyCondition::Le(val) => collect_range(
            partition,
            Bound::Unbounded,
            Bound::Included(val.clone()),
            scan_forward,
            limit,
            exclusive_start_key,
        ),
        SortKeyCondition::Gt(val) => collect_range(
            partition,
            Bound::Excluded(val.clone()),
            Bound::Unbounded,
            scan_forward,
            limit,
            exclusive_start_key,
        ),
        SortKeyCondition::Ge(val) => collect_range(
            partition,
            Bound::Included(val.clone()),
            Bound::Unbounded,
            scan_forward,
            limit,
            exclusive_start_key,
        ),
        SortKeyCondition::Between(low, high) => collect_range(
            partition,
            Bound::Included(low.clone()),
            Bound::Included(high.clone()),
            scan_forward,
            limit,
            exclusive_start_key,
        ),
        SortKeyCondition::BeginsWithStr(prefix) => {
            collect_begins_with_str(partition, prefix, scan_forward, limit, exclusive_start_key)
        }
        SortKeyCondition::BeginsWithBytes(prefix) => {
            collect_begins_with_bytes(partition, prefix, scan_forward, limit, exclusive_start_key)
        }
    }
}

/// Collects items within a BTreeMap range, respecting direction and pagination.
fn collect_range<'a>(
    partition: &'a BTreeMap<SortableAttributeValue, StoredItem>,
    lower: Bound<SortableAttributeValue>,
    upper: Bound<SortableAttributeValue>,
    scan_forward: bool,
    limit: usize,
    exclusive_start_key: Option<&SortableAttributeValue>,
) -> Vec<&'a StoredItem> {
    if scan_forward {
        let effective_lower = match exclusive_start_key {
            Some(start) => {
                // The exclusive start key takes precedence if it's within bounds.
                match &lower {
                    Bound::Unbounded => Bound::Excluded(start.clone()),
                    Bound::Included(l) | Bound::Excluded(l) => {
                        if start >= l {
                            Bound::Excluded(start.clone())
                        } else {
                            lower
                        }
                    }
                }
            }
            None => lower,
        };
        partition
            .range((effective_lower, upper))
            .take(limit)
            .map(|(_, item)| item)
            .collect()
    } else {
        let effective_upper = match exclusive_start_key {
            Some(start) => match &upper {
                Bound::Unbounded => Bound::Excluded(start.clone()),
                Bound::Included(u) | Bound::Excluded(u) => {
                    if start <= u {
                        Bound::Excluded(start.clone())
                    } else {
                        upper
                    }
                }
            },
            None => upper,
        };
        partition
            .range((lower, effective_upper))
            .rev()
            .take(limit)
            .map(|(_, item)| item)
            .collect()
    }
}

/// Collects items whose string sort key begins with the given prefix.
fn collect_begins_with_str<'a>(
    partition: &'a BTreeMap<SortableAttributeValue, StoredItem>,
    prefix: &str,
    scan_forward: bool,
    limit: usize,
    exclusive_start_key: Option<&SortableAttributeValue>,
) -> Vec<&'a StoredItem> {
    // Use range starting from the prefix to the prefix with last byte incremented.
    let start = SortableAttributeValue::S(prefix.to_owned());

    // Compute the upper bound: prefix with the last byte incremented.
    let upper = compute_str_prefix_upper_bound(prefix);

    let lower_bound = match exclusive_start_key {
        Some(esk) if scan_forward && *esk >= start => Bound::Excluded(esk.clone()),
        _ => Bound::Included(start),
    };

    let upper_bound = match &upper {
        Some(ub) => match exclusive_start_key {
            Some(esk) if !scan_forward && *esk <= *ub => Bound::Excluded(esk.clone()),
            _ => Bound::Excluded(ub.clone()),
        },
        None => match exclusive_start_key {
            Some(esk) if !scan_forward => Bound::Excluded(esk.clone()),
            _ => Bound::Unbounded,
        },
    };

    if scan_forward {
        partition
            .range((lower_bound, upper_bound))
            .filter(|(k, _)| matches!(k, SortableAttributeValue::S(s) if s.starts_with(prefix)))
            .take(limit)
            .map(|(_, item)| item)
            .collect()
    } else {
        partition
            .range((lower_bound, upper_bound))
            .rev()
            .filter(|(k, _)| matches!(k, SortableAttributeValue::S(s) if s.starts_with(prefix)))
            .take(limit)
            .map(|(_, item)| item)
            .collect()
    }
}

/// Collects items whose binary sort key begins with the given byte prefix.
fn collect_begins_with_bytes<'a>(
    partition: &'a BTreeMap<SortableAttributeValue, StoredItem>,
    prefix: &[u8],
    scan_forward: bool,
    limit: usize,
    exclusive_start_key: Option<&SortableAttributeValue>,
) -> Vec<&'a StoredItem> {
    let start = SortableAttributeValue::B(bytes::Bytes::copy_from_slice(prefix));

    // Compute the upper bound: prefix with the last byte incremented.
    let upper = compute_bytes_prefix_upper_bound(prefix);

    let lower_bound = match exclusive_start_key {
        Some(esk) if scan_forward && *esk >= start => Bound::Excluded(esk.clone()),
        _ => Bound::Included(start),
    };

    let upper_bound = match &upper {
        Some(ub) => match exclusive_start_key {
            Some(esk) if !scan_forward && *esk <= *ub => Bound::Excluded(esk.clone()),
            _ => Bound::Excluded(ub.clone()),
        },
        None => match exclusive_start_key {
            Some(esk) if !scan_forward => Bound::Excluded(esk.clone()),
            _ => Bound::Unbounded,
        },
    };

    if scan_forward {
        partition
            .range((lower_bound, upper_bound))
            .filter(|(k, _)| matches!(k, SortableAttributeValue::B(b) if b.starts_with(prefix)))
            .take(limit)
            .map(|(_, item)| item)
            .collect()
    } else {
        partition
            .range((lower_bound, upper_bound))
            .rev()
            .filter(|(k, _)| matches!(k, SortableAttributeValue::B(b) if b.starts_with(prefix)))
            .take(limit)
            .map(|(_, item)| item)
            .collect()
    }
}

/// Computes the exclusive upper bound for a string prefix scan.
///
/// Increments the last byte of the prefix string. Returns `None` if the
/// prefix is empty or all bytes are `0xFF`.
fn compute_str_prefix_upper_bound(prefix: &str) -> Option<SortableAttributeValue> {
    let mut bytes = prefix.as_bytes().to_vec();
    // Pop trailing 0xFF bytes since they cannot be incremented.
    while bytes.last() == Some(&0xFF) {
        bytes.pop();
    }
    if bytes.is_empty() {
        return None;
    }
    if let Some(last) = bytes.last_mut() {
        *last += 1;
    }
    // The resulting bytes may not be valid UTF-8 after incrementing,
    // but since we use byte ordering for strings, we create a string
    // from lossy conversion. This is acceptable because it is only
    // used as a range bound for the BTreeMap.
    Some(SortableAttributeValue::S(
        String::from_utf8_lossy(&bytes).into_owned(),
    ))
}

/// Computes the exclusive upper bound for a binary prefix scan.
///
/// Increments the last byte of the prefix. Returns `None` if the
/// prefix is empty or all bytes are `0xFF`.
fn compute_bytes_prefix_upper_bound(prefix: &[u8]) -> Option<SortableAttributeValue> {
    let mut bytes = prefix.to_vec();
    // Pop trailing 0xFF bytes since they cannot be incremented.
    while bytes.last() == Some(&0xFF) {
        bytes.pop();
    }
    if bytes.is_empty() {
        return None;
    }
    if let Some(last) = bytes.last_mut() {
        *last += 1;
    }
    Some(SortableAttributeValue::B(bytes::Bytes::from(bytes)))
}

/// Builds the last evaluated key for pagination when a limit was hit.
fn build_last_evaluated_key(
    items: &[&StoredItem],
    limit: Option<usize>,
    partition_key: &AttributeValue,
    key_schema: &KeySchema,
) -> Option<PrimaryKey> {
    let effective_limit = limit?;

    if items.len() < effective_limit {
        return None;
    }

    let last_item = items.last()?;
    let sort_key = key_schema.sort_key.as_ref().and_then(|sk_def| {
        last_item
            .attributes
            .get(&sk_def.name)
            .and_then(|av| SortableAttributeValue::from_attribute_value(&sk_def.name, av).ok())
    });

    Some(PrimaryKey {
        partition_key: partition_key.clone(),
        sort_key,
    })
}

// ---------------------------------------------------------------------------
// Key extraction
// ---------------------------------------------------------------------------

/// Extracts the primary key from an item according to the key schema.
///
/// # Errors
///
/// Returns `StorageError::MissingKeyAttribute` if a required key attribute
/// is missing, or `StorageError::InvalidKeyType` if a key attribute has
/// the wrong type.
#[allow(clippy::implicit_hasher)]
pub fn extract_primary_key(
    key_schema: &KeySchema,
    item: &HashMap<String, AttributeValue>,
) -> Result<PrimaryKey, StorageError> {
    let pk_value = item.get(&key_schema.partition_key.name).ok_or_else(|| {
        StorageError::MissingKeyAttribute {
            attr: key_schema.partition_key.name.clone(),
        }
    })?;

    validate_key_type(
        &key_schema.partition_key.name,
        &key_schema.partition_key.attr_type,
        pk_value,
    )?;

    let sort_key = match &key_schema.sort_key {
        Some(sk_def) => {
            let sk_value =
                item.get(&sk_def.name)
                    .ok_or_else(|| StorageError::MissingKeyAttribute {
                        attr: sk_def.name.clone(),
                    })?;

            validate_key_type(&sk_def.name, &sk_def.attr_type, sk_value)?;

            Some(SortableAttributeValue::from_attribute_value(
                &sk_def.name,
                sk_value,
            )?)
        }
        None => None,
    };

    Ok(PrimaryKey {
        partition_key: pk_value.clone(),
        sort_key,
    })
}

/// Validates that an attribute value matches the expected scalar type.
fn validate_key_type(
    attr_name: &str,
    expected: &ScalarAttributeType,
    value: &AttributeValue,
) -> Result<(), StorageError> {
    if matches!(
        (expected, value),
        (ScalarAttributeType::S, AttributeValue::S(_))
            | (ScalarAttributeType::N, AttributeValue::N(_))
            | (ScalarAttributeType::B, AttributeValue::B(_))
    ) {
        Ok(())
    } else {
        Err(StorageError::InvalidKeyType {
            attr: attr_name.to_owned(),
            expected: expected.as_str().to_owned(),
            actual: value.type_descriptor().to_owned(),
        })
    }
}

// ---------------------------------------------------------------------------
// Item size calculation
// ---------------------------------------------------------------------------

/// Calculates the size of a DynamoDB item in bytes, following DynamoDB's
/// item size calculation rules.
///
/// Each attribute contributes: `len(attribute_name) + size(attribute_value)`.
///
/// Value sizes:
/// - **S**: length of the UTF-8 string
/// - **N**: `(len + 1) / 2 + 1` (compact decimal encoding)
/// - **B**: byte length
/// - **Bool / Null**: 1 byte
/// - **SS / NS / BS**: sum of element sizes
/// - **L**: `3 + sum(1 + element_size)` for each element
/// - **M**: `3 + sum(key_len + 1 + value_size)` for each entry
#[must_use]
#[allow(clippy::implicit_hasher)]
pub fn calculate_item_size(item: &HashMap<String, AttributeValue>) -> u64 {
    item.iter()
        .map(|(name, value)| name.len() as u64 + calculate_value_size(value))
        .sum()
}

/// Calculates the size of a single attribute value.
#[must_use]
fn calculate_value_size(value: &AttributeValue) -> u64 {
    match value {
        AttributeValue::S(s) => s.len() as u64,
        AttributeValue::N(n) => (n.len().div_ceil(2) + 1) as u64,
        AttributeValue::B(b) => b.len() as u64,
        AttributeValue::Bool(_) | AttributeValue::Null(_) => 1,
        AttributeValue::Ss(v) => v.iter().map(|s| s.len() as u64).sum(),
        AttributeValue::Ns(v) => v.iter().map(|n| (n.len().div_ceil(2) + 1) as u64).sum(),
        AttributeValue::Bs(v) => v.iter().map(|b| b.len() as u64).sum(),
        AttributeValue::L(list) => {
            3 + list
                .iter()
                .map(|elem| 1 + calculate_value_size(elem))
                .sum::<u64>()
        }
        AttributeValue::M(map) => {
            3 + map
                .iter()
                .map(|(k, v)| k.len() as u64 + 1 + calculate_value_size(v))
                .sum::<u64>()
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn string_key_schema() -> KeySchema {
        KeySchema {
            partition_key: KeyAttribute {
                name: "pk".to_owned(),
                attr_type: ScalarAttributeType::S,
            },
            sort_key: None,
        }
    }

    fn composite_key_schema() -> KeySchema {
        KeySchema {
            partition_key: KeyAttribute {
                name: "pk".to_owned(),
                attr_type: ScalarAttributeType::S,
            },
            sort_key: Some(KeyAttribute {
                name: "sk".to_owned(),
                attr_type: ScalarAttributeType::S,
            }),
        }
    }

    fn numeric_sort_key_schema() -> KeySchema {
        KeySchema {
            partition_key: KeyAttribute {
                name: "pk".to_owned(),
                attr_type: ScalarAttributeType::S,
            },
            sort_key: Some(KeyAttribute {
                name: "sk".to_owned(),
                attr_type: ScalarAttributeType::N,
            }),
        }
    }

    fn make_item(pairs: &[(&str, AttributeValue)]) -> HashMap<String, AttributeValue> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect()
    }

    #[test]
    fn test_should_put_and_get_item() {
        let storage = TableStorage::new(string_key_schema());
        let item = make_item(&[
            ("pk", AttributeValue::S("user1".to_owned())),
            ("name", AttributeValue::S("Alice".to_owned())),
        ]);

        let result = storage.put_item(item.clone());
        assert!(result.is_ok());
        assert!(result.as_ref().ok().and_then(|r| r.as_ref()).is_none());

        let key = PrimaryKey {
            partition_key: AttributeValue::S("user1".to_owned()),
            sort_key: None,
        };
        let retrieved = storage.get_item(&key);
        assert!(retrieved.is_some());
        assert_eq!(
            retrieved.as_ref().and_then(|i| i.get("name")),
            item.get("name")
        );
        assert_eq!(storage.item_count(), 1);
    }

    #[test]
    fn test_should_delete_item() {
        let storage = TableStorage::new(string_key_schema());
        let item = make_item(&[
            ("pk", AttributeValue::S("user1".to_owned())),
            ("name", AttributeValue::S("Alice".to_owned())),
        ]);

        storage.put_item(item.clone()).ok();

        let key = PrimaryKey {
            partition_key: AttributeValue::S("user1".to_owned()),
            sort_key: None,
        };
        let deleted = storage.delete_item(&key);
        assert!(deleted.is_some());
        assert_eq!(
            deleted.as_ref().and_then(|i| i.get("name")),
            item.get("name")
        );

        // Item should no longer exist.
        assert!(storage.get_item(&key).is_none());
        assert_eq!(storage.item_count(), 0);
    }

    #[test]
    fn test_should_replace_existing_item() {
        let storage = TableStorage::new(string_key_schema());
        let item1 = make_item(&[
            ("pk", AttributeValue::S("user1".to_owned())),
            ("name", AttributeValue::S("Alice".to_owned())),
        ]);
        let item2 = make_item(&[
            ("pk", AttributeValue::S("user1".to_owned())),
            ("name", AttributeValue::S("Bob".to_owned())),
        ]);

        storage.put_item(item1.clone()).ok();
        let old = storage.put_item(item2).ok().flatten();

        assert!(old.is_some());
        assert_eq!(
            old.as_ref().and_then(|i| i.get("name")),
            Some(&AttributeValue::S("Alice".to_owned()))
        );
        assert_eq!(storage.item_count(), 1);
    }

    #[test]
    fn test_should_query_with_sort_key_range() {
        let storage = TableStorage::new(numeric_sort_key_schema());

        for i in 1..=10 {
            let item = make_item(&[
                ("pk", AttributeValue::S("partition1".to_owned())),
                ("sk", AttributeValue::N(i.to_string())),
                ("data", AttributeValue::S(format!("item{i}"))),
            ]);
            storage.put_item(item).ok();
        }

        // Between 3 and 7 inclusive.
        let condition = SortKeyCondition::Between(
            SortableAttributeValue::N("3".to_owned()),
            SortableAttributeValue::N("7".to_owned()),
        );
        let pk = AttributeValue::S("partition1".to_owned());
        let (items, _) = storage.query(&pk, Some(&condition), true, None, None);
        assert_eq!(items.len(), 5);

        // Gt 8.
        let condition = SortKeyCondition::Gt(SortableAttributeValue::N("8".to_owned()));
        let (items, _) = storage.query(&pk, Some(&condition), true, None, None);
        assert_eq!(items.len(), 2);

        // Lt 3.
        let condition = SortKeyCondition::Lt(SortableAttributeValue::N("3".to_owned()));
        let (items, _) = storage.query(&pk, Some(&condition), true, None, None);
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn test_should_query_begins_with() {
        let storage = TableStorage::new(composite_key_schema());

        let prefixes = ["user#1", "user#2", "user#10", "order#1", "order#2"];
        for prefix in &prefixes {
            let item = make_item(&[
                ("pk", AttributeValue::S("partition1".to_owned())),
                ("sk", AttributeValue::S((*prefix).to_owned())),
            ]);
            storage.put_item(item).ok();
        }

        let pk = AttributeValue::S("partition1".to_owned());
        let condition = SortKeyCondition::BeginsWithStr("user#".to_owned());
        let (items, _) = storage.query(&pk, Some(&condition), true, None, None);
        assert_eq!(items.len(), 3);

        let condition = SortKeyCondition::BeginsWithStr("order#".to_owned());
        let (items, _) = storage.query(&pk, Some(&condition), true, None, None);
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn test_should_scan_all_items() {
        let storage = TableStorage::new(string_key_schema());

        for i in 1..=5 {
            let item = make_item(&[
                ("pk", AttributeValue::S(format!("user{i}"))),
                ("name", AttributeValue::S(format!("Name{i}"))),
            ]);
            storage.put_item(item).ok();
        }

        let (items, last_key) = storage.scan(None, None, None, None);
        assert_eq!(items.len(), 5);
        assert!(last_key.is_none());
    }

    #[test]
    fn test_should_scan_with_limit() {
        let storage = TableStorage::new(string_key_schema());

        for i in 1..=5 {
            let item = make_item(&[
                ("pk", AttributeValue::S(format!("user{i}"))),
                ("name", AttributeValue::S(format!("Name{i}"))),
            ]);
            storage.put_item(item).ok();
        }

        let (items, last_key) = storage.scan(Some(3), None, None, None);
        assert_eq!(items.len(), 3);
        assert!(last_key.is_some());

        // Continue scanning from the last key.
        let (items2, last_key2) = storage.scan(Some(3), last_key.as_ref(), None, None);
        assert_eq!(items2.len(), 2);
        assert!(last_key2.is_none());
    }

    #[test]
    fn test_should_error_on_missing_key() {
        let storage = TableStorage::new(string_key_schema());
        let item = make_item(&[("name", AttributeValue::S("Alice".to_owned()))]);

        let result = storage.put_item(item);
        assert!(result.is_err());
        let err = result.err();
        assert!(matches!(
            err,
            Some(StorageError::MissingKeyAttribute { ref attr }) if attr == "pk"
        ));
    }

    #[test]
    fn test_should_error_on_invalid_key_type() {
        let storage = TableStorage::new(string_key_schema());
        // Provide a number where a string is expected.
        let item = make_item(&[("pk", AttributeValue::N("123".to_owned()))]);

        let result = storage.put_item(item);
        assert!(result.is_err());
        assert!(matches!(
            result.err(),
            Some(StorageError::InvalidKeyType { ref attr, ref expected, ref actual })
                if attr == "pk" && expected == "S" && actual == "N"
        ));
    }

    #[test]
    fn test_should_calculate_item_size() {
        // Simple item with string and number.
        let item = make_item(&[
            ("pk", AttributeValue::S("hello".to_owned())), // name=2 + value=5 = 7
            ("age", AttributeValue::N("25".to_owned())),   // name=3 + value=(2+1)/2+1=2 = 5
        ]);
        let size = calculate_item_size(&item);
        assert_eq!(size, 12);

        // Item with bool and null.
        let item = make_item(&[
            ("a", AttributeValue::Bool(true)), // name=1 + value=1 = 2
            ("b", AttributeValue::Null(true)), // name=1 + value=1 = 2
        ]);
        let size = calculate_item_size(&item);
        assert_eq!(size, 4);

        // Item with a list.
        let item = make_item(&[
            (
                "l",
                AttributeValue::L(vec![
                    AttributeValue::S("ab".to_owned()), // 1 + 2 = 3
                    AttributeValue::N("1".to_owned()),  // 1 + (1+1)/2+1 = 1+2 = 3
                ]),
            ),
            // name=1 + value = 3 + 3 + 3 = 9 => total 10
        ]);
        let size = calculate_item_size(&item);
        assert_eq!(size, 10);

        // Item with a map.
        let mut inner = HashMap::new();
        inner.insert("x".to_owned(), AttributeValue::S("y".to_owned()));
        let item = make_item(&[
            ("m", AttributeValue::M(inner)),
            // name=1 + value = 3 + (1 + 1 + 1) = 6 => total 7
        ]);
        let size = calculate_item_size(&item);
        assert_eq!(size, 7);
    }

    #[test]
    fn test_should_query_reverse_order() {
        let storage = TableStorage::new(numeric_sort_key_schema());

        for i in 1..=5 {
            let item = make_item(&[
                ("pk", AttributeValue::S("p1".to_owned())),
                ("sk", AttributeValue::N(i.to_string())),
                ("val", AttributeValue::S(format!("v{i}"))),
            ]);
            storage.put_item(item).ok();
        }

        let pk = AttributeValue::S("p1".to_owned());
        let (items, _) = storage.query(&pk, None, false, None, None);
        assert_eq!(items.len(), 5);

        // First item in reverse should be the one with sk=5.
        let first_sk = items[0].get("sk");
        assert_eq!(first_sk, Some(&AttributeValue::N("5".to_owned())));
    }

    #[test]
    fn test_should_query_with_limit_and_pagination() {
        let storage = TableStorage::new(numeric_sort_key_schema());

        for i in 1..=10 {
            let item = make_item(&[
                ("pk", AttributeValue::S("p1".to_owned())),
                ("sk", AttributeValue::N(i.to_string())),
            ]);
            storage.put_item(item).ok();
        }

        let pk = AttributeValue::S("p1".to_owned());

        // First page.
        let (items, last_key) = storage.query(&pk, None, true, Some(3), None);
        assert_eq!(items.len(), 3);
        assert!(last_key.is_some());

        // Second page using exclusive start key.
        let start_sk = last_key.as_ref().and_then(|k| k.sort_key.as_ref());
        let (items2, _) = storage.query(&pk, None, true, Some(3), start_sk);
        assert_eq!(items2.len(), 3);

        // Verify no overlap between pages.
        let first_page_sks: Vec<_> = items
            .iter()
            .filter_map(|i| i.get("sk").and_then(|v| v.as_n()))
            .collect();
        let second_page_sks: Vec<_> = items2
            .iter()
            .filter_map(|i| i.get("sk").and_then(|v| v.as_n()))
            .collect();
        for sk in &second_page_sks {
            assert!(!first_page_sks.contains(sk));
        }
    }

    #[test]
    fn test_should_handle_composite_key_put_get_delete() {
        let storage = TableStorage::new(composite_key_schema());
        let item = make_item(&[
            ("pk", AttributeValue::S("user1".to_owned())),
            ("sk", AttributeValue::S("profile".to_owned())),
            ("name", AttributeValue::S("Alice".to_owned())),
        ]);

        storage.put_item(item.clone()).ok();

        let key = PrimaryKey {
            partition_key: AttributeValue::S("user1".to_owned()),
            sort_key: Some(SortableAttributeValue::S("profile".to_owned())),
        };
        let retrieved = storage.get_item(&key);
        assert!(retrieved.is_some());

        let deleted = storage.delete_item(&key);
        assert!(deleted.is_some());
        assert!(storage.get_item(&key).is_none());
    }

    #[test]
    fn test_should_track_total_size() {
        let storage = TableStorage::new(string_key_schema());
        let item = make_item(&[("pk", AttributeValue::S("a".to_owned()))]);
        let expected_size = calculate_item_size(&item);

        storage.put_item(item).ok();
        assert_eq!(storage.total_size_bytes(), expected_size);

        let key = PrimaryKey {
            partition_key: AttributeValue::S("a".to_owned()),
            sort_key: None,
        };
        storage.delete_item(&key);
        assert_eq!(storage.total_size_bytes(), 0);
    }

    #[test]
    fn test_should_sort_numbers_numerically() {
        let a = SortableAttributeValue::N("2".to_owned());
        let b = SortableAttributeValue::N("10".to_owned());
        // Numeric ordering: 2 < 10.
        assert!(a < b);

        let c = SortableAttributeValue::N("-5".to_owned());
        let d = SortableAttributeValue::N("3".to_owned());
        assert!(c < d);
    }

    #[test]
    fn test_should_sort_strings_by_bytes() {
        let a = SortableAttributeValue::S("abc".to_owned());
        let b = SortableAttributeValue::S("abe".to_owned());
        assert!(a < b);

        let c = SortableAttributeValue::S("a".to_owned());
        let d = SortableAttributeValue::S("aa".to_owned());
        assert!(c < d);
    }

    #[test]
    fn test_should_sort_binary_by_bytes() {
        let a = SortableAttributeValue::B(bytes::Bytes::from_static(&[0x00, 0x01]));
        let b = SortableAttributeValue::B(bytes::Bytes::from_static(&[0x00, 0x02]));
        assert!(a < b);
    }

    #[test]
    fn test_should_query_empty_partition() {
        let storage = TableStorage::new(string_key_schema());
        let pk = AttributeValue::S("nonexistent".to_owned());
        let (items, last_key) = storage.query(&pk, None, true, None, None);
        assert!(items.is_empty());
        assert!(last_key.is_none());
    }

    #[test]
    fn test_should_query_eq_condition() {
        let storage = TableStorage::new(composite_key_schema());

        for suffix in &["a", "b", "c"] {
            let item = make_item(&[
                ("pk", AttributeValue::S("p1".to_owned())),
                ("sk", AttributeValue::S((*suffix).to_owned())),
            ]);
            storage.put_item(item).ok();
        }

        let pk = AttributeValue::S("p1".to_owned());
        let condition = SortKeyCondition::Eq(SortableAttributeValue::S("b".to_owned()));
        let (items, _) = storage.query(&pk, Some(&condition), true, None, None);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].get("sk"), Some(&AttributeValue::S("b".to_owned())));
    }
}
