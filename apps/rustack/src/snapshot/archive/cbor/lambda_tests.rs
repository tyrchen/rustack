//! Raw Lambda path and metadata barriers must precede typed allocation.

use std::cell::Cell;

use ciborium::Value;
use rstest::rstest;
use rustack_lambda_core::{artifact::Architecture, provider::DockerArchiveProfile};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::IgnoredAny,
    ser::{SerializeMap, SerializeSeq},
};

use super::*;

thread_local! { static VISITS: Cell<usize> = const { Cell::new(0) }; }
#[derive(Debug)]
struct Sentinel;
impl<'de> Deserialize<'de> for Sentinel {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        VISITS.set(VISITS.get().saturating_add(1));
        IgnoredAny::deserialize(decoder)?;
        Ok(Self)
    }
}
fn managed() -> DockerArchiveProfile {
    DockerArchiveProfile::ProvidedZip(Architecture::Arm64)
}
fn map(values: Vec<(&str, Value)>) -> Value {
    Value::Map(
        values
            .into_iter()
            .map(|(key, value)| (Value::Text(key.into()), value))
            .collect(),
    )
}
fn body() -> Value {
    Value::Text("A".repeat(128 * 1024 + 4))
}
fn version() -> Value {
    map(vec![("zipBytesBase64", body())])
}
fn row(published: bool) -> Value {
    if published {
        map(vec![(
            "versions",
            Value::Map(vec![(Value::Integer(1_u64.into()), version())]),
        )])
    } else {
        map(vec![("latest", version())])
    }
}
fn rows(value: Value) -> Value {
    map(vec![(
        "functions",
        map(vec![("functions", Value::Array(vec![value]))]),
    )])
}
fn rejected(value: &Value, profile: DockerArchiveProfile, expected: Failure) {
    let mut bytes = Vec::new();
    ciborium::into_writer(value, &mut bytes).unwrap();
    VISITS.set(0);
    assert_eq!(
        decode_lambda::<Sentinel>(&bytes, profile).err(),
        Some(expected)
    );
    assert_eq!(VISITS.get(), 0);
}

struct Indefinite<'a>(&'a Value);
impl Serialize for Indefinite<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            Value::Map(values) => {
                let mut map = serializer.serialize_map(None)?;
                for (key, value) in values {
                    map.serialize_entry(key, &Self(value))?;
                }
                map.end()
            }
            Value::Array(values) => {
                let mut sequence = serializer.serialize_seq(None)?;
                for value in values {
                    sequence.serialize_element(&Self(value))?;
                }
                sequence.end()
            }
            value => value.serialize(serializer),
        }
    }
}
#[rstest]
#[case(false, false)]
#[case(false, true)]
#[case(true, false)]
#[case(true, true)]
fn test_should_allow_only_known_latest_and_unsigned_version_body_paths(
    #[case] published: bool,
    #[case] indefinite: bool,
) {
    let value = rows(row(published));
    let bytes = if indefinite {
        encode(&Indefinite(&value)).unwrap()
    } else {
        encode(&value).unwrap()
    };
    let decoded: Value = decode_lambda(&bytes, managed()).unwrap();
    assert_eq!(decoded, value);
    assert!(matches!(
        decode_lambda::<Sentinel>(&bytes, DockerArchiveProfile::ImageOnly),
        Err(Failure::Scalar)
    ));
    assert_eq!(
        encode_lambda(&value, managed()).unwrap(),
        encode(&value).unwrap()
    );
}

#[rstest]
#[case(0)]
#[case(1)]
#[case(2)]
#[case(3)]
#[case(4)]
#[case(5)]
#[case(6)]
#[case(7)]
fn test_should_reject_misplaced_and_unknown_lookalikes_before_deserialization(
    #[case] location: usize,
) {
    let value = match location {
        0 => map(vec![("unknown", rows(row(false)))]),
        1 => map(vec![("functions", map(vec![("unknown", version())]))]),
        2 => rows(map(vec![("policy", map(vec![("latest", version())]))])),
        3 => rows(map(vec![("latest", map(vec![("unknown", version())]))])),
        4 => map(vec![("layers", map(vec![("latest", version())]))]),
        5 => rows(map(vec![(
            "versions",
            Value::Map(vec![(Value::Text("1".into()), version())]),
        )])),
        6 => rows(map(vec![(
            "versions",
            Value::Map(vec![(Value::Integer((-1_i64).into()), version())]),
        )])),
        _ => map(vec![(
            "functions",
            map(vec![("functions", map(vec![("latest", version())]))]),
        )]),
    };
    rejected(&value, managed(), Failure::Scalar);
}
#[test]
fn test_should_reject_duplicate_keys_even_when_both_values_are_authorized_zip_slots() {
    rejected(
        &rows(map(vec![("latest", version()), ("latest", version())])),
        managed(),
        Failure::Duplicate,
    );
}
#[rstest]
#[case(false)]
#[case(true)]
fn test_should_bound_function_and_revision_counts_before_typed_allocation(#[case] revisions: bool) {
    let value = if revisions {
        let versions = (1_u64..=1024)
            .map(|n| (Value::Integer(n.into()), Value::Null))
            .collect();
        rows(map(vec![
            ("latest", Value::Null),
            ("versions", Value::Map(versions)),
        ]))
    } else {
        map(vec![(
            "functions",
            map(vec![("functions", Value::Array(vec![Value::Null; 257]))]),
        )])
    };
    for bytes in [
        encode(&value).unwrap(),
        encode(&Indefinite(&value)).unwrap(),
    ] {
        VISITS.set(0);
        assert!(matches!(
            decode_lambda::<Sentinel>(&bytes, managed()),
            Err(Failure::Collection)
        ));
        assert_eq!(VISITS.get(), 0);
    }
}
#[test]
fn test_should_charge_headers_and_keys_but_exempt_only_exact_body_contents() {
    let bytes = encode(&rows(row(false))).unwrap();
    let metadata = bytes.len().checked_sub(128 * 1024 + 4).unwrap();
    for (allowed, expected) in [
        (metadata, Ok(())),
        (metadata.checked_sub(1).unwrap(), Err(Failure::Bytes)),
    ] {
        let mut budget = LambdaBudget::new(managed());
        budget.metadata = allowed;
        let mut scan = Scan {
            input: &bytes,
            offset: 0,
            remaining: LIMITS.nodes,
            limits: LIMITS,
            lambda: Some(budget),
        };
        assert_eq!(scan.item(0, LambdaPath::Root), expected);
    }
}
#[test]
fn test_should_preserve_pair_and_array_member_budget_semantics() {
    let exact = encode(&vec![(); 65_536]).unwrap();
    assert!(decode_lambda::<IgnoredAny>(&exact, managed()).is_ok());
    rejected(
        &Value::Array(vec![Value::Null; 65_537]),
        managed(),
        Failure::Collection,
    );
}
