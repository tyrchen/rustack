use std::error::Error;

use proptest::prelude::*;
use rstest::rstest;
use serde_json::{Map, Value, json};

use crate::{AslError, DataFlow, EvaluationContext, Payload};

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn test_should_execute_all_four_independent_pipeline_catalog_cases() -> TestResult {
    let catalog: Value = serde_json::from_str(include_str!(
        "../../../../tests/workflows/semantic-cases.json"
    ))?;
    let mut count = 0;
    for case in catalog["cases"].as_array().ok_or("missing catalog")? {
        if case["kind"] != "pipeline" {
            continue;
        }
        let mut config = Map::new();
        for (source, target) in [
            ("inputPath", "InputPath"),
            ("parameters", "Parameters"),
            ("resultSelector", "ResultSelector"),
            ("resultPath", "ResultPath"),
            ("outputPath", "OutputPath"),
        ] {
            if let Some(value) = case.get(source) {
                config.insert(target.to_owned(), value.clone());
            }
        }
        let flow = DataFlow::compile(&Value::Object(config))?;
        let input = Payload::from_value(&case["input"])?;
        let context = EvaluationContext::from(&input);
        assert_eq!(
            flow.prepare(&input, &context)?.value(),
            &case["expectedTaskInput"]
        );
        let result = Payload::from_value(&case["rawResult"])?;
        assert_eq!(
            flow.finish(&input, &result, &context)?.value(),
            &case["expected"],
            "{}",
            case["id"]
        );
        assert_eq!(input.value(), &case["input"]);
        count += 1;
    }
    assert_eq!(count, 4);
    Ok(())
}

#[rstest]
#[case(json!({"Unknown":true}))]
#[case(json!({"Type":"Pass"}))]
#[case(json!([]))]
#[case(json!({"InputPath":1}))]
#[case(json!({"OutputPath":false}))]
#[case(json!({"ResultPath":1}))]
#[case(json!({"ResultPath":"$$.State"}))]
#[case(json!({"Parameters":null}))]
#[case(json!({"ResultSelector":[]}))]
fn test_should_reject_invalid_dataflow_fields_without_defaulting(#[case] config: Value) {
    assert!(DataFlow::compile(&config).is_err());
}

#[test]
fn test_should_distinguish_json_null_from_null_path_and_preserve_original_raw_bytes() -> TestResult
{
    let null = Payload::parse(b" null ")?;
    let context = EvaluationContext::from(&null);
    let default = DataFlow::parse(b"{}")?;
    assert!(default.prepare(&null, &context)?.value().is_null());
    let empty = DataFlow::parse(br#"{"InputPath":null}"#)?;
    assert_eq!(empty.prepare(&null, &context)?.value(), &json!({}));
    assert_eq!(null.original_bytes(), b" null ");
    let original = Payload::parse(br#" { "keep":1,"request":{"n":2} } "#)?;
    let bytes = original.original_bytes().to_vec();
    let flow=DataFlow::parse(br#"{"InputPath":"$.request","Parameters":{"n.$":"States.MathAdd($.n,1)"},"ResultSelector":{"answer.$":"$.raw"},"ResultPath":"$.result","OutputPath":"$.result"}"#)?;
    let context = EvaluationContext::from(&original);
    assert_eq!(flow.prepare(&original, &context)?.value(), &json!({"n":3}));
    assert_eq!(
        flow.finish(&original, &Payload::parse(br#"{"raw":9}"#)?, &context)?
            .value(),
        &json!({"answer":9})
    );
    assert_eq!(original.original_bytes(), bytes);
    assert_eq!(format!("{flow:?}"), "DataFlow(<redacted>)");
    Ok(())
}

#[test]
fn test_should_enforce_intermediate_limits_before_later_discarding_operations() -> TestResult {
    let input = Payload::from_value(&json!({"x":"a".repeat(140_000)}))?;
    let result = Payload::from_value(&json!({"x":"b".repeat(140_000)}))?;
    let context = EvaluationContext::from(&input);
    let merge = DataFlow::compile(&json!({"ResultPath":"$.result","OutputPath":null}))?;
    assert!(matches!(
        merge.finish(&input, &result, &context),
        Err(AslError::DataLimit)
    ));
    let selector = DataFlow::compile(
        &json!({"ResultSelector":{"a.$":"$.x","b.$":"$.x"},"ResultPath":null,"OutputPath":null}),
    )?;
    assert!(matches!(
        selector.finish(&input, &result, &context),
        Err(AslError::DataLimit)
    ));
    let parameters =
        DataFlow::compile(&json!({"Parameters":{"a.$":"$.x","b.$":"$.x"},"OutputPath":null}))?;
    assert!(matches!(
        parameters.prepare(&input, &context),
        Err(AslError::DataLimit)
    ));
    let invalid = DataFlow::compile(&json!({"ResultPath":"$.missing[0]","OutputPath":null}))?;
    assert!(matches!(
        invalid.finish(&input, &result, &context),
        Err(AslError::Merge)
    ));
    assert!(DataFlow::parse(br#"{"InputPath":null,"InputPath":"$"}"#).is_err());
    Ok(())
}

#[test]
fn test_should_keep_default_and_explicit_root_path_materialization_equivalent() -> TestResult {
    let raw = format!("[{}]", vec!["1e9"; 30_000].join(","));
    let input = Payload::parse(raw.as_bytes())?;
    let context = EvaluationContext::from(&input);
    for config in [json!({}), json!({"InputPath":"$"})] {
        assert!(matches!(
            DataFlow::compile(&config)?.prepare(&input, &context),
            Err(AslError::DataLimit)
        ));
    }
    assert_eq!(
        DataFlow::compile(&json!({"InputPath":null}))?
            .prepare(&input, &context)?
            .value(),
        &json!({})
    );
    Ok(())
}

#[test]
fn test_should_share_result_phase_budget_between_selector_and_output_path() -> TestResult {
    let original = Payload::parse(b"null")?;
    let result = Payload::from_value(&json!(vec![0; 6000]))?;
    let context = EvaluationContext::from(&original);
    let selector = json!({"ResultSelector":{"all.$":"$[*]"}});
    assert_eq!(
        DataFlow::compile(&selector)?
            .finish(&original, &result, &context)?
            .value()["all"]
            .as_array()
            .ok_or("missing array")?
            .len(),
        6000
    );
    let both = json!({"ResultSelector":{"all.$":"$[*]"},"OutputPath":"$.all[*]"});
    assert!(matches!(
        DataFlow::compile(&both)?.finish(&original, &result, &context),
        Err(AslError::WorkLimit)
    ));
    assert!(matches!(
        DataFlow::compile(&json!({"InputPath":"$.absent"}))?.prepare(&original, &context),
        Err(AslError::Missing)
    ));
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]
    #[test]
    fn test_should_preserve_original_under_discarded_results(n in any::<i64>(), result in any::<i64>()) {
        let original=Payload::from_value(&json!({"n":n})).map_err(|e|TestCaseError::fail(e.to_string()))?;
        let result=Payload::from_value(&json!(result)).map_err(|e|TestCaseError::fail(e.to_string()))?;
        let flow=DataFlow::compile(&json!({"ResultPath":null})).map_err(|e|TestCaseError::fail(e.to_string()))?;
        let output=flow.finish(&original,&result,&EvaluationContext::from(&original)).map_err(|e|TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(output.value(),original.value());
    }
}
