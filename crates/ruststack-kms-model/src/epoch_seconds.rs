//! Epoch seconds serde helpers for timestamp fields.
//!
//! AWS KMS transmits timestamps as floating-point epoch seconds in JSON
//! (e.g., `1710766800.123`). The `chrono::DateTime<Utc>` default serde
//! would produce ISO 8601 strings, which the AWS SDK cannot parse.

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Deserializer, Serializer};

/// Serde helpers for `Option<DateTime<Utc>>` as epoch seconds.
pub mod option {
    use super::{DateTime, Deserialize, Deserializer, Serializer, TimeZone, Utc};

    /// Serialize `Option<DateTime<Utc>>` as epoch seconds (f64) or null.
    pub fn serialize<S: Serializer>(dt: &Option<DateTime<Utc>>, s: S) -> Result<S::Ok, S::Error> {
        match dt {
            Some(d) => {
                #[allow(clippy::cast_precision_loss)]
                let secs = d.timestamp() as f64 + f64::from(d.timestamp_subsec_millis()) / 1000.0;
                s.serialize_f64(secs)
            }
            None => s.serialize_none(),
        }
    }

    /// Deserialize epoch seconds (f64) or null into `Option<DateTime<Utc>>`.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<DateTime<Utc>>, D::Error> {
        let opt: Option<f64> = Option::deserialize(d)?;
        match opt {
            Some(secs) => {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let whole = secs.trunc() as i64;
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let nanos = (secs.fract() * 1_000_000_000.0) as u32;
                Utc.timestamp_opt(whole, nanos)
                    .single()
                    .ok_or_else(|| serde::de::Error::custom("invalid epoch timestamp"))
                    .map(Some)
            }
            None => Ok(None),
        }
    }
}
