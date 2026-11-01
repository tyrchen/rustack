//! Absolute DTO bounds; domain validity is established by restore preparation.

use std::{collections::BTreeMap, fmt, marker::PhantomData};

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{
    Deserialize, Deserializer,
    de::{Error, IgnoredAny, MapAccess, SeqAccess, Visitor},
};

pub(super) struct Text<const MAX: usize>(pub(super) String);
impl<const MAX: usize> fmt::Debug for Text<MAX> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SnapshotText(<redacted>)")
    }
}
impl<'de, const MAX: usize> Deserialize<'de> for Text<MAX> {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        text::<D, MAX>(decoder).map(Self)
    }
}
pub(super) fn text<'de, D: Deserializer<'de>, const MAX: usize>(
    decoder: D,
) -> Result<String, D::Error> {
    let value = String::deserialize(decoder)?;
    if value.len() > MAX {
        return Err(D::Error::custom("snapshot text limit exceeded"));
    }
    Ok(value)
}
pub(super) fn optional_text<'de, D: Deserializer<'de>, const MAX: usize>(
    decoder: D,
) -> Result<Option<String>, D::Error> {
    Option::<Text<MAX>>::deserialize(decoder).map(|value| value.map(|value| value.0))
}
pub(super) fn time<'de, D: Deserializer<'de>>(decoder: D) -> Result<DateTime<Utc>, D::Error> {
    instant::<D::Error>(&Text::<64>::deserialize(decoder)?.0)
}
fn instant<E: Error>(text: &str) -> Result<DateTime<Utc>, E> {
    let value: DateTime<Utc> = text
        .parse()
        .map_err(|_| E::custom("invalid snapshot instant"))?;
    if value.timestamp_subsec_nanos() >= 1_000_000_000
        || value.to_rfc3339_opts(SecondsFormat::AutoSi, true) != text
    {
        return Err(E::custom("noncanonical snapshot instant"));
    }
    Ok(value)
}
pub(super) fn optional_time<'de, D: Deserializer<'de>>(
    decoder: D,
) -> Result<Option<DateTime<Utc>>, D::Error> {
    Option::<Text<64>>::deserialize(decoder)?
        .map(|text| instant::<D::Error>(&text.0))
        .transpose()
}
#[derive(Debug)]
struct Rows<T, const MAX: usize>(PhantomData<T>);
impl<'de, T: Deserialize<'de>, const MAX: usize> Visitor<'de> for Rows<T, MAX> {
    type Value = Vec<T>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("bounded snapshot rows")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        if sequence.size_hint().is_some_and(|count| count > MAX) {
            return Err(A::Error::custom("snapshot row limit exceeded"));
        }
        let mut rows = Vec::new();
        rows.try_reserve_exact(sequence.size_hint().unwrap_or(0).min(1024))
            .map_err(|_| A::Error::custom("snapshot allocation failed"))?;
        loop {
            if rows.len() == MAX {
                if sequence.next_element::<IgnoredAny>()?.is_some() {
                    return Err(A::Error::custom("snapshot row limit exceeded"));
                }
                return Ok(rows);
            }
            let Some(row) = sequence.next_element()? else {
                return Ok(rows);
            };
            rows.try_reserve(1)
                .map_err(|_| A::Error::custom("snapshot allocation failed"))?;
            rows.push(row);
        }
    }
}
pub(super) fn rows<'de, D: Deserializer<'de>, T: Deserialize<'de>, const MAX: usize>(
    decoder: D,
) -> Result<Vec<T>, D::Error> {
    decoder.deserialize_seq(Rows::<T, MAX>(PhantomData))
}
#[derive(Debug)]
struct Tags;
impl<'de> Visitor<'de> for Tags {
    type Value = BTreeMap<String, String>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("bounded snapshot tags")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        if map.size_hint().is_some_and(|count| count > 50) {
            return Err(A::Error::custom("snapshot tag limit exceeded"));
        }
        let mut tags = BTreeMap::new();
        while let Some(key) = map.next_key::<Text<128>>()? {
            if tags.len() >= 50 {
                return Err(A::Error::custom("snapshot tag limit exceeded"));
            }
            let value = map.next_value::<Text<256>>()?;
            if tags.insert(key.0, value.0).is_some() {
                return Err(A::Error::custom("duplicate snapshot tag"));
            }
        }
        Ok(tags)
    }
}
pub(super) fn tags<'de, D: Deserializer<'de>>(
    decoder: D,
) -> Result<BTreeMap<String, String>, D::Error> {
    decoder.deserialize_map(Tags)
}
