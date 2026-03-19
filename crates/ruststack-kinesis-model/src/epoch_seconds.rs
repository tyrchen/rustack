//! Epoch seconds serde helpers for timestamp fields.
//!
//! AWS Kinesis transmits timestamps as floating-point epoch seconds in JSON
//! (e.g., `1710766800.123`). The `chrono::DateTime<Utc>` default serde
//! would produce ISO 8601 strings, which the AWS SDK cannot parse.

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserializer, Serializer};

/// Serialize `DateTime<Utc>` as epoch seconds (f64).
pub fn serialize<S: Serializer>(dt: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
    #[allow(clippy::cast_precision_loss)]
    let secs = dt.timestamp() as f64 + f64::from(dt.timestamp_subsec_millis()) / 1000.0;
    s.serialize_f64(secs)
}

/// Deserialize epoch seconds (f64) or ISO 8601 string into `DateTime<Utc>`.
pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<DateTime<Utc>, D::Error> {
    use serde::de;

    struct EpochOrIso;

    impl de::Visitor<'_> for EpochOrIso {
        type Value = DateTime<Utc>;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("epoch seconds (number) or ISO 8601 string")
        }

        fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
            epoch_to_datetime(v)
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
            Utc.timestamp_opt(v, 0)
                .single()
                .ok_or_else(|| E::custom("invalid epoch timestamp"))
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
            #[allow(clippy::cast_possible_wrap)]
            let secs = v as i64;
            Utc.timestamp_opt(secs, 0)
                .single()
                .ok_or_else(|| E::custom("invalid epoch timestamp"))
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            v.parse::<DateTime<Utc>>()
                .map_err(|e| E::custom(format!("invalid ISO 8601 timestamp: {e}")))
        }
    }

    d.deserialize_any(EpochOrIso)
}

/// Serde helpers for `Option<DateTime<Utc>>` as epoch seconds.
pub mod option {
    use super::{DateTime, Deserializer, Serializer, TimeZone, Utc, epoch_to_datetime};
    use serde::de;

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

    /// Deserialize epoch seconds (f64), ISO 8601 string, or null into `Option<DateTime<Utc>>`.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<DateTime<Utc>>, D::Error> {
        struct OptEpochOrIso;

        impl<'de> de::Visitor<'de> for OptEpochOrIso {
            type Value = Option<DateTime<Utc>>;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("epoch seconds (number), ISO 8601 string, or null")
            }

            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(None)
            }

            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(None)
            }

            fn visit_some<D2: Deserializer<'de>>(
                self,
                deserializer: D2,
            ) -> Result<Self::Value, D2::Error> {
                super::deserialize(deserializer).map(Some)
            }

            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
                epoch_to_datetime(v).map(Some)
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Utc.timestamp_opt(v, 0)
                    .single()
                    .ok_or_else(|| E::custom("invalid epoch timestamp"))
                    .map(Some)
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                #[allow(clippy::cast_possible_wrap)]
                let secs = v as i64;
                Utc.timestamp_opt(secs, 0)
                    .single()
                    .ok_or_else(|| E::custom("invalid epoch timestamp"))
                    .map(Some)
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                v.parse::<DateTime<Utc>>()
                    .map(Some)
                    .map_err(|e| E::custom(format!("invalid ISO 8601 timestamp: {e}")))
            }
        }

        // Try option-aware deserialization first, fall back to direct visitor
        d.deserialize_any(OptEpochOrIso)
    }
}

fn epoch_to_datetime<E: serde::de::Error>(secs: f64) -> Result<DateTime<Utc>, E> {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let whole = secs.trunc() as i64;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let nanos = (secs.fract() * 1_000_000_000.0) as u32;
    Utc.timestamp_opt(whole, nanos)
        .single()
        .ok_or_else(|| E::custom("invalid epoch timestamp"))
}
