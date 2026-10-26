use std::{error::Error, str::FromStr};

use proptest::prelude::*;
use rstest::rstest;
use rustack_core::json::JsonError;
use serde_json::{Value, json};

use crate::{
    AslError, EvaluationContext, MAX_PAYLOAD_BYTES, MatchKind, Path, Payload, ReferencePath,
    ResultPath,
};

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn test_should_execute_independent_catalog_path_cases() -> TestResult {
    let catalog: Value = serde_json::from_str(include_str!(
        "../../../../tests/workflows/semantic-cases.json"
    ))?;
    let mut count = 0;
    for case in catalog["cases"].as_array().ok_or("missing cases")? {
        if case["kind"] != "path" {
            continue;
        }
        let input = Payload::from_value(&case["input"])?;
        let context =
            EvaluationContext::from_value(&case.get("context").cloned().unwrap_or(json!({})))?;
        let path: Path = case["path"].as_str().ok_or("missing path")?.parse()?;
        let matches = path.select(&input, &context)?;
        if case.get("expectedError").is_some() {
            assert_eq!(matches.kind(), MatchKind::Missing);
            assert!(matches!(matches.to_payload(), Err(AslError::Missing)));
        } else {
            assert_eq!(
                matches.to_payload()?.value(),
                &case["expected"],
                "{}",
                case["id"]
            );
        }
        count += 1;
    }
    assert_eq!(count, 7);
    Ok(())
}

#[rstest]
#[case("$")]
#[case("$$")]
#[case("$.a_b0")]
#[case("$.商品.价格")]
#[case("$['']")]
#[case("$['a\\'b']")]
#[case("$['a\\\\b']")]
#[case("$[0001]")]
#[case("$[:]")]
#[case("$[:2]")]
#[case("$[2:]")]
#[case("$[2:0]")]
#[case("$[*][*]")]
fn test_should_compile_the_explicit_path_subset(#[case] text: &str) -> TestResult {
    let path: Path = text.parse()?;
    assert_eq!(format!("{path:?}"), "Path(<redacted>)");
    Ok(())
}

#[rstest]
#[case("")]
#[case(" ")]
#[case("x")]
#[case("$$$")]
#[case("$.0a")]
#[case("$.a-b")]
#[case("$.a.")]
#[case("$..a")]
#[case("$.*")]
#[case("$[]")]
#[case("$[-1]")]
#[case("$[+1]")]
#[case("$[1,2]")]
#[case("$[1:2:1]")]
#[case("$[ 1]")]
#[case("$[?(@.x)]")]
#[case("$[\"a\"]")]
#[case("$['unterminated]")]
#[case("$['bad\\q']")]
#[case("$['bad\n']")]
#[case("$['a']junk")]
#[case("$[999999999999999999999999999999999]")]
fn test_should_reject_unsupported_or_hostile_path_syntax(#[case] text: &str) {
    assert!(matches!(Path::from_str(text), Err(AslError::InvalidPath)));
}

#[test]
fn test_should_enforce_exact_path_bytes_and_segment_counts() -> TestResult {
    let maximum = format!("$['{}']", "k".repeat(1019));
    assert_eq!(maximum.len(), 1024);
    maximum.parse::<Path>()?;
    assert!(matches!(
        format!("{maximum}x").parse::<Path>(),
        Err(AslError::InvalidPath)
    ));
    format!("${}", ".a".repeat(128)).parse::<Path>()?;
    assert!(matches!(
        format!("${}", ".a".repeat(129)).parse::<Path>(),
        Err(AslError::InvalidPath)
    ));
    Ok(())
}

#[rstest]
#[case("$[*][*]", json!([[1,2],[],[3]]), json!([1,2,3]))]
#[case("$[*].n", json!([{"n":1},{},{"n":null}]), json!([1,null]))]
#[case("$[*]", json!({"z":3,"a":1,"m":2}), json!([1,2,3]))]
#[case("$[1:]", json!([0,1,2]), json!([1,2]))]
#[case("$[:2]", json!([0,1,2]), json!([0,1]))]
#[case("$[1:999]", json!([0,1,2]), json!([1,2]))]
#[case("$[999:]", json!([0,1,2]), json!([]))]
#[case("$[2:1]", json!([0,1,2]), json!([]))]
#[case("$[*]", json!(null), json!([]))]
#[case("$.missing[*]", json!({}), json!([]))]
fn test_should_preserve_deterministic_selection_semantics(
    #[case] text: &str,
    #[case] input: Value,
    #[case] expected: Value,
) -> TestResult {
    let data = Payload::from_value(&input)?;
    let context = EvaluationContext::from(&data);
    let result = text.parse::<Path>()?.select(&data, &context)?;
    assert_eq!(result.kind(), MatchKind::Selection);
    assert!(result.single().is_none());
    assert_eq!(result.to_payload()?.value(), &expected);
    Ok(())
}

#[test]
fn test_should_distinguish_missing_null_context_and_wrong_container() -> TestResult {
    let data = Payload::parse(br#"{"n":null,"a":[],"x":3}"#)?;
    let context = EvaluationContext::from_value(&json!({"n":"context"}))?;
    let null = "$.n".parse::<Path>()?.select(&data, &context)?;
    assert_eq!(null.kind(), MatchKind::Single);
    assert_eq!(null.single(), Some(&Value::Null));
    assert_eq!(
        "$$.n".parse::<ReferencePath>()?.lookup(&data, &context),
        Some(&json!("context"))
    );
    for text in ["$.absent", "$.a[0]", "$.x.n", "$.a.n", "$.n.n", "$[0]"] {
        assert_eq!(
            text.parse::<Path>()?.select(&data, &context)?.kind(),
            MatchKind::Missing
        );
        assert!(
            text.parse::<ReferencePath>()?
                .lookup(&data, &context)
                .is_none()
        );
    }
    Ok(())
}

#[test]
fn test_should_prove_reference_and_result_root_invariants() -> TestResult {
    for path in ["$[*]", "$[0:2]", "$$.x[*]"] {
        assert!(matches!(
            path.parse::<ReferencePath>(),
            Err(AslError::InvalidReference)
        ));
    }
    "$$.x".parse::<ReferencePath>()?;
    assert!(matches!(
        "$$.x".parse::<ResultPath>(),
        Err(AslError::InvalidResultTarget)
    ));
    assert_eq!(
        format!("{:?}", "$.secret".parse::<ReferencePath>()?),
        "ReferencePath(<redacted>)"
    );
    assert_eq!(
        format!("{:?}", "$.secret".parse::<ResultPath>()?),
        "ResultPath(<redacted>)"
    );
    Ok(())
}

#[rstest]
#[case("$", json!({"a":1}), json!(7))]
#[case("$.a.b", json!({}), json!({"a":{"b":7}}))]
#[case("$.a", json!({"a":null}), json!({"a":7}))]
#[case("$[1].a", json!([0,{}]), json!([0,{"a":7}]))]
#[case("$[0]", json!([1]), json!([7]))]
#[case("$['odd.key']", json!({"old":1}), json!({"old":1,"odd.key":7}))]
fn test_should_merge_only_valid_object_and_existing_array_targets(
    #[case] path: &str,
    #[case] input: Value,
    #[case] expected: Value,
) -> TestResult {
    let original = Payload::from_value(&input)?;
    let result = Payload::parse(b"7")?;
    let output = path.parse::<ResultPath>()?.merge(&original, &result)?;
    assert_eq!(output.value(), &expected);
    assert_eq!(original.value(), &input);
    Ok(())
}

#[rstest]
#[case("$.a.b", json!({"a":null}))]
#[case("$.a.b", json!({"a":1}))]
#[case("$.a[0]", json!({}))]
#[case("$[0]", json!([]))]
#[case("$[1]", json!([0]))]
#[case("$.x", json!(null))]
#[case("$.x", json!([]))]
#[case("$[0]", json!({"0":1}))]
fn test_should_reject_invalid_merges_without_mutating_original(
    #[case] path: &str,
    #[case] input: Value,
) -> TestResult {
    let original = Payload::from_value(&input)?;
    let result = Payload::parse(b"7")?;
    assert!(matches!(
        path.parse::<ResultPath>()?.merge(&original, &result),
        Err(AslError::Merge)
    ));
    assert_eq!(original.value(), &input);
    Ok(())
}

#[test]
fn test_should_bound_path_work_and_merged_output() -> TestResult {
    let exact = Payload::from_value(&json!(vec![0; 9999]))?;
    let path: Path = "$[*]".parse()?;
    path.select(&exact, &EvaluationContext::from(&exact))?
        .to_payload()?;
    let excess = Payload::from_value(&json!(vec![0; 10_000]))?;
    assert!(matches!(
        path.select(&excess, &EvaluationContext::from(&excess)),
        Err(AslError::WorkLimit)
    ));
    let large = Payload::from_value(&json!({"large":"v".repeat(150_000)}))?;
    let result = Payload::from_value(&json!("x".repeat(150_000)))?;
    assert!(matches!(
        "$.result".parse::<ResultPath>()?.merge(&large, &result),
        Err(AslError::DataLimit)
    ));
    assert!(large.value().get("result").is_none());
    Ok(())
}

#[test]
fn test_should_bound_selection_materialization_before_cloning_or_encoding_large_data() -> TestResult
{
    let source = Payload::from_value(&json!([[["x".repeat(262_136)]]]))?;
    assert_eq!(source.original_bytes().len(), MAX_PAYLOAD_BYTES);
    let selected = "$[*]"
        .parse::<Path>()?
        .select(&source, &EvaluationContext::from(&source))?
        .to_payload()?;
    assert_eq!(selected.original_bytes().len(), MAX_PAYLOAD_BYTES);
    assert_eq!(selected.value(), source.value());
    let flattened = "$[*][*]"
        .parse::<Path>()?
        .select(&source, &EvaluationContext::from(&source))?
        .to_payload()?;
    assert_eq!(flattened.original_bytes().len(), MAX_PAYLOAD_BYTES - 2);
    Ok(())
}

#[test]
fn test_should_bound_payload_bytes_depth_members_duplicates_and_encoding() -> TestResult {
    let raw = format!("\"{}\"", "x".repeat(MAX_PAYLOAD_BYTES - 2));
    let data = Payload::parse(raw.as_bytes())?;
    assert_eq!(data.original_bytes().len(), MAX_PAYLOAD_BYTES);
    assert!(matches!(
        Payload::parse(format!("{raw} ").as_bytes()),
        Err(AslError::Json(JsonError::Bytes))
    ));
    assert!(matches!(
        Payload::parse(br#"{"a":1,"\u0061":2}"#),
        Err(AslError::Json(JsonError::Duplicate))
    ));
    assert!(Payload::parse(b"").is_err());
    let depth64 = format!("{}0{}", "[".repeat(64), "]".repeat(64));
    Payload::parse(depth64.as_bytes())?;
    assert!(matches!(
        Payload::parse(format!("[{depth64}]").as_bytes()),
        Err(AslError::Json(JsonError::Depth))
    ));
    let mut nested = json!(0);
    for _ in 0..65 {
        nested = json!([nested]);
    }
    assert!(matches!(
        Payload::from_value(&nested),
        Err(AslError::Json(JsonError::Depth))
    ));
    assert!(matches!(
        Payload::from_value(&json!(vec![0; 65_537])),
        Err(AslError::Json(JsonError::Members))
    ));
    assert!(matches!(
        Payload::from_value(&json!("\u{0}".repeat(50_000))),
        Err(AslError::DataLimit)
    ));
    assert!(matches!(
        Payload::from_value(&json!("x".repeat(MAX_PAYLOAD_BYTES + 1))),
        Err(AslError::Json(JsonError::String))
    ));
    assert_eq!(format!("{data:?}"), "Payload(<redacted>)");
    let source = Payload::parse(b"  null\n")?;
    assert_eq!(source.clone().original_bytes(), b"  null\n");
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]
    #[test]
    fn test_should_parse_arbitrary_utf8_without_panics(text in ".{0,1200}") {
        let _ = text.parse::<Path>();
        let _ = text.parse::<ReferencePath>();
        let _ = text.parse::<ResultPath>();
    }

    #[test]
    fn test_should_round_trip_quoted_keys(key in "[^\\p{Cc}]{0,80}", number in any::<i64>()) {
        let escaped = key.replace('\\', "\\\\").replace('\'', "\\'");
        let text = format!("$['{escaped}']");
        let path = text.parse::<ReferencePath>().map_err(|e| TestCaseError::fail(e.to_string()))?;
        let data = Payload::from_value(&json!({key:number})).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let context = EvaluationContext::from(&data);
        prop_assert_eq!(path.lookup(&data, &context), Some(&json!(number)));
    }

    #[test]
    fn test_should_preserve_original_when_merging_object_member(input in any::<i64>(), result in any::<i64>()) {
        let original = Payload::from_value(&json!({"input":input})).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let value = Payload::from_value(&json!(result)).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let path = "$.result".parse::<ResultPath>().map_err(|e| TestCaseError::fail(e.to_string()))?;
        let output = path.merge(&original, &value).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(original.value(), &json!({"input":input}));
        prop_assert_eq!(output.value(), &json!({"input":input,"result":result}));
    }
}
