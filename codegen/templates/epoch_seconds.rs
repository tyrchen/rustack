//! Generated JSON epoch-second adapters with exact decimal millisecond precision.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as DeError, ser::Error as SerError};
use serde_json::value::RawValue;

const INVALID: &str = "invalid epoch-second timestamp";

/// Serialize a required timestamp as a numeric JSON epoch value.
///
/// # Errors
/// Rejects leap seconds, invalid numeric representation or serializer failures.
pub fn serialize<S: Serializer>(value: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error> {
    if value.timestamp_subsec_nanos() >= 1_000_000_000 { return Err(S::Error::custom(INVALID)); }
    let nanos = i128::from(value.timestamp()).checked_mul(1_000_000_000)
        .and_then(|seconds| seconds.checked_add(i128::from(value.timestamp_subsec_nanos())))
        .ok_or_else(|| S::Error::custom(INVALID))?;
    let millis = nanos / 1_000_000;
    let sign = if millis < 0 { "-" } else { "" };
    let magnitude = millis.unsigned_abs();
    let seconds = magnitude / 1000;
    let fraction = magnitude % 1000;
    let number = if fraction == 0 { format!("{sign}{seconds}") } else { format!("{sign}{seconds}.{fraction:03}") };
    RawValue::from_string(number).map_err(|_| S::Error::custom(INVALID))?.serialize(serializer)
}

/// Deserialize a required JSON number, truncating submillisecond precision.
///
/// # Errors
/// Rejects nonnumeric, oversized, out-of-range or invalid epoch values.
pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<DateTime<Utc>, D::Error> {
    let value = Box::<RawValue>::deserialize(deserializer)?;
    parse(value.get()).ok_or_else(|| D::Error::custom(INVALID))
}

fn parse(raw: &str) -> Option<DateTime<Utc>> {
    if raw.len() > 128 { return None; }
    let (negative, raw) = raw.strip_prefix('-').map_or((false, raw), |rest| (true, rest));
    let (mantissa, exponent) = match raw.split_once(['e', 'E']) {
        Some((value, exponent)) => {
            let (negative, digits) = exponent.strip_prefix('-').map_or_else(
                || (false, exponent.strip_prefix('+').unwrap_or(exponent)), |digits| (true, digits));
            if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) { return None; }
            let magnitude = digits.bytes().fold(0_i32, |value, digit|
                value.saturating_mul(10).saturating_add(i32::from(digit - b'0')).min(1024));
            (value, if negative { -magnitude } else { magnitude })
        },
        None => (raw, 0),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if whole.is_empty() || !whole.bytes().chain(fraction.bytes()).all(|byte| byte.is_ascii_digit()) { return None; }
    let digits: Vec<_> = whole.bytes().chain(fraction.bytes()).collect();
    if digits.iter().all(|digit| *digit == b'0') { return DateTime::from_timestamp_millis(0); }
    let keep = i32::try_from(whole.len()).ok()?.checked_add(exponent)?.checked_add(3)?;
    if keep <= 0 { return DateTime::from_timestamp_millis(0); }
    // Leading zeroes must not make a small representable exponent look huge.
    let leading = digits.iter().take_while(|digit| **digit == b'0').count();
    let significant = keep.checked_sub(i32::try_from(leading).ok()?)?;
    if significant <= 0 { return DateTime::from_timestamp_millis(0); }
    if significant > 19 { return None; }
    let mut millis = 0_i128;
    let count = usize::try_from(significant).ok()?;
    for offset in 0..count {
        let digit = digits.get(leading.checked_add(offset)?).copied().unwrap_or(b'0');
        millis = millis.checked_mul(10)?.checked_add(i128::from(digit.checked_sub(b'0')?))?;
    }
    let signed = if negative { millis.checked_neg()? } else { millis };
    DateTime::from_timestamp_millis(i64::try_from(signed).ok()?)
}

/// Optional JSON timestamp adapter; missing members also need serde default.
pub mod option {
    use std::borrow::Borrow;
    use super::{DateTime, Deserialize, Deserializer, DeError, RawValue, Serializer, Utc, INVALID};

    /// Serialize a timestamp or JSON null.
    ///
    /// # Errors
    /// Returns the required adapter's validation/serialization errors.
    pub fn serialize<S: Serializer, V: Borrow<Option<DateTime<Utc>>>>(value: V, serializer: S) -> Result<S::Ok, S::Error> {
        match value.borrow() { Some(value) => super::serialize(value, serializer), None => serializer.serialize_none() }
    }
    /// Deserialize a timestamp or JSON null.
    ///
    /// # Errors
    /// Rejects invalid nonnull values with the required adapter's rules.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error> {
        Option::<Box<RawValue>>::deserialize(deserializer)?.map(|value| super::parse(value.get()).ok_or_else(|| D::Error::custom(INVALID))).transpose()
    }
}
