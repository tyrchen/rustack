//! Before-copy projection controls, including the flattened live revision shape.

use std::{cell::Cell, collections::BTreeMap};

use rstest::rstest;
use serde::{Serialize, Serializer, ser::Error as _};
use serde_json::{Value, json};

use super::Metadata;

#[derive(Debug, Serialize)]
struct Root<V> {
    functions: Rows<V>,
}
#[derive(Debug, Serialize)]
struct Rows<V> {
    functions: Vec<Function<V>>,
}
#[derive(Debug, Serialize)]
struct Function<V> {
    latest: V,
    versions: BTreeMap<u64, V>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Plain<V> {
    zip_bytes_base64: V,
    note: &'static str,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Flat<V> {
    zip_bytes_base64: V,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}
#[derive(Debug)]
struct Body<'a>(&'a Cell<usize>);
impl Serialize for Body<'_> {
    fn serialize<S: Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
        self.0.set(self.0.get().saturating_add(1));
        Err(S::Error::custom("ZIP body formatter was invoked"))
    }
}
fn project<V: Serialize>(latest: V, published: V) -> Value {
    let source = Root {
        functions: Rows {
            functions: vec![Function {
                latest,
                versions: [(1, published)].into_iter().collect(),
            }],
        },
    };
    serde_json::to_value(Metadata::new(&source)).unwrap()
}
#[rstest]
#[case::plain(false)]
#[case::flattened(true)]
fn test_should_never_invoke_original_body_serializers_in_known_struct_and_flattened_slots(
    #[case] flat: bool,
) {
    let calls = Cell::new(0);
    let output = if flat {
        let make = || Flat {
            zip_bytes_base64: Body(&calls),
            extra: [("note".into(), json!("retained"))].into_iter().collect(),
        };
        project(make(), make())
    } else {
        let make = || Plain {
            zip_bytes_base64: Body(&calls),
            note: "retained",
        };
        project(make(), make())
    };
    assert_eq!(calls.get(), 0);
    for row in [
        &output["functions"]["functions"][0]["latest"],
        &output["functions"]["functions"][0]["versions"]["1"],
    ] {
        assert!(row["zipBytesBase64"].is_null());
        assert_eq!(row["note"], "retained");
    }
}
#[rstest]
#[case(json!({"unknown":{"functions":{"functions":[{"latest":{"zipBytesBase64":"retained"}}]}}}))]
#[case(json!({"layers":{"latest":{"zipBytesBase64":"retained"}}}))]
#[case(json!({"functions":{"functions":[{"policy":{"latest":{"zipBytesBase64":"retained"}}}]}}))]
#[case(json!({"functions":{"functions":[{"versions":{"1":{"zipBytesBase64":"retained"}}}]}}))]
#[case(json!({"functions":{"functions":{"latest":{"zipBytesBase64":"retained"}}}}))]
#[case(json!({"functions":{"functions":[{"latest":[{"zipBytesBase64":"retained"}]}]}}))]
fn test_should_keep_nonmatching_paths_and_text_version_key_lookalikes(#[case] source: Value) {
    let encoded = serde_json::to_vec(&Metadata::new(&source)).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&encoded).unwrap(), source);
}
#[test]
fn test_should_bound_map_key_formatting_before_value_serialization() {
    let source = json!({"x".repeat(7 * 1024):{"zipBytesBase64":"retained"}});
    let error = serde_json::to_vec(&Metadata::new(&source)).unwrap_err();
    assert!(error.to_string().contains("invalid archive metadata key"));
    assert!(!format!("{:?}", Metadata::new(&source)).contains("retained"));
}
