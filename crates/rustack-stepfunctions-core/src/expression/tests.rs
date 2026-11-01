use std::{collections::BTreeSet, error::Error};

use proptest::prelude::*;
use rstest::rstest;
use rustack_core::json::JsonError;
use serde_json::{Map, Value, json};
use uuid::{Builder, Uuid, Variant, Version};

use super::Budget;
use crate::{AslError, EvaluationContext, Expression, Payload, SystemUuidSource, UuidSource};

type TestResult = Result<(), Box<dyn Error>>;

#[derive(Default)]
struct Fixed(u8);
impl UuidSource for Fixed {
    fn next_uuid(&mut self) -> Result<Uuid, AslError> {
        self.0 = self.0.checked_add(1).ok_or(AslError::WorkLimit)?;
        Ok(Builder::from_random_bytes([self.0; 16]).into_uuid())
    }
}

fn evaluate(expression: &str, input: &Value) -> Result<Payload, AslError> {
    let expression: Expression = expression.parse()?;
    let data = Payload::from_value(input)?;
    expression.evaluate_with_source(
        &data,
        &EvaluationContext::from(&data),
        &mut Fixed::default(),
    )
}

#[test]
fn test_should_execute_all_twelve_independent_intrinsic_catalog_cases() -> TestResult {
    let catalog: Value = serde_json::from_str(include_str!(
        "../../../../tests/workflows/semantic-cases.json"
    ))?;
    let mut count = 0;
    for case in catalog["cases"].as_array().ok_or("missing cases")? {
        if case["kind"] != "intrinsic" {
            continue;
        }
        let result = evaluate(
            case["expression"].as_str().ok_or("missing expression")?,
            &case["input"],
        )?;
        if case.get("expectedPattern").is_some() {
            let id = Uuid::parse_str(result.value().as_str().ok_or("not UUID")?)?;
            assert_eq!(id.get_version(), Some(Version::Random));
            assert_eq!(id.get_variant(), Variant::RFC4122);
        } else {
            assert_eq!(result.value(), &case["expected"], "{}", case["id"]);
        }
        count += 1;
    }
    assert_eq!(count, 12);
    Ok(())
}

#[rstest]
#[case("States.Array()",json!(null),json!([]))]
#[case("States.Array(true, false, null, -1, 1.25e1)",json!({}),json!([true,false,null,-1,12.5]))]
#[case(r"States.Array('\'', '\\', '\{', '\}')",json!({}),json!(["'","\\","{","}"]))]
#[case(r"States.Format('it\'s {}', 'fine')",json!({}),json!("it's fine"))]
#[case(r"States.Format('\{\} {}', 'x')",json!({}),json!("{} x"))]
#[case(r"States.Format('\\{}', 'x')",json!({}),json!("\\x"))]
#[case(r"States.Format(States.Format('\{\}'), 'x')",json!({}),json!("x"))]
#[case("States.Format('{}:{}:{}:{}', true, null, 3, 'text')",json!({}),json!("true:null:3:text"))]
#[case("States.Format($.format, $.x)",json!({"format":"\\{ {} \\}","x":"yes"}),json!("{ yes }"))]
#[case("States.Array(States.ArrayLength($.x), States.ArrayGetItem($.x, 1))",json!({"x":[1,2]}),json!([2,2]))]
#[case("States.ArrayContains($.x, 1.0)",json!({"x":[1]}),json!(true))]
#[case("States.ArrayContains($.x, $.y)",json!({"x":[{"a":[null,2.0]}],"y":{"a":[null,2]}}),json!(true))]
#[case("States.ArrayContains($.x, $.y)",json!({"x":[{"a":2}],"y":{"b":2}}),json!(false))]
#[case("States.ArrayPartition($.x, 1.5)",json!({"x":[1,2,3]}),json!([[1,2],[3]]))]
#[case("States.ArrayPartition($.x, 1)",json!({"x":[]}),json!([]))]
#[case("States.ArrayPartition($.x, 2147483647)",json!({"x":[1]}),json!([[1]]))]
#[case("States.StringSplit($.x, ',')",json!({"x":",a,,b,"}),json!(["a","b"]))]
#[case("States.StringSplit($.x, '')",json!({"x":"ab"}),json!(["ab"]))]
#[case("States.StringSplit($.x, '')",json!({"x":""}),json!([]))]
#[case("States.StringSplit($.x, '中🙂')",json!({"x":"中a🙂b中"}),json!(["a","b"]))]
#[case(r#"States.StringToJson('\{"n":1\}')"#,json!({}),json!({"n":1}))]
#[case("States.StringSplit($.x, '\n')",json!({"x":"one\ntwo"}),json!(["one","two"]))]
#[case("States.Array('\0')",json!({}),json!(["\0"]))]
#[case("States.JsonMerge($.a, $.b, $.deep)",json!({"a":{"x":{"left":1},"keep":2},"b":{"x":{"right":3}},"deep":false}),json!({"x":{"right":3},"keep":2}))]
fn test_should_preserve_selected_intrinsic_value_and_escape_semantics(
    #[case] expression: &str,
    #[case] data: Value,
    #[case] expected: Value,
) -> TestResult {
    assert_eq!(evaluate(expression, &data)?.value(), &expected);
    Ok(())
}

#[test]
fn test_should_pin_historical_aws_half_rounding_and_split_edges() -> TestResult {
    for (left, right, result) in [
        (1.49, 1.50, 3),
        (1.50, 1.51, 4),
        (-1.49, -1.50, -2),
        (-1.50, -1.51, -3),
        (0.5, 0.0, 1),
        (2.5, 0.0, 3),
        (-0.5, 0.5, 1),
        (-0.5, 0.0, 0),
        (-1.5, 0.0, -1),
        (-2.5, 0.0, -2),
    ] {
        assert_eq!(
            evaluate("States.MathAdd($.a, $.b)", &json!({"a":left,"b":right}))?.value(),
            &json!(result)
        );
    }
    for (input, delimiters, expected) in [
        (" ", ",", vec![" "]),
        (" , ", ",", vec![" ", " "]),
        (", , ,", ",", vec![" ", " "]),
        (",,,,", ",", vec![]),
        (
            "This.is+a,test=string",
            ".+,=",
            vec!["This", "is", "a", "test", "string"],
        ),
        (
            "split on T and \nnew line",
            "T\n",
            vec!["split on ", " and ", "new line"],
        ),
    ] {
        assert_eq!(
            evaluate(
                "States.StringSplit($.a, $.b)",
                &json!({"a":input,"b":delimiters})
            )?
            .value(),
            &json!(expected)
        );
    }
    assert_eq!(
        evaluate(
            "States.MathAdd($.x, 0)",
            &json!({"x":0.499_999_999_999_999_94})
        )?
        .value(),
        &json!(0)
    );
    Ok(())
}

#[rstest]
#[case("")]
#[case("'literal'")]
#[case("true")]
#[case(" States.Array()")]
#[case("States.Array() ")]
#[case("States.Array ()")]
#[case("States.Unknown()")]
#[case("States.MathRandom(0,1)")]
#[case("States.Array(,)")]
#[case("States.Array(1,)")]
#[case("States.Array(1 2)")]
#[case("States.Array([])")]
#[case("States.Array({})")]
#[case("States.Array(+1)")]
#[case("States.Array(1e999)")]
#[case(r"States.Array('\n')")]
#[case("States.Array('unterminated)")]
#[case(r"States.Array('unescaped{')")]
#[case("States.ArrayLength()")]
#[case("States.MathAdd(1)")]
#[case("States.UUID(1)")]
#[case("States.JsonMerge($.a,$.b,true)")]
#[case("States.Format('{}')")]
#[case("States.Format('{x}', 1)")]
#[case("States.Format('}', 1)")]
#[case("States.Format('{}', 1, 2)")]
#[case("States.Array($.x['bad)")]
fn test_should_reject_malformed_unsupported_or_ambiguous_expression_syntax(#[case] text: &str) {
    assert!(text.parse::<Expression>().is_err(), "{text}");
}

#[rstest]
#[case("States.ArrayLength(1)")]
#[case("States.ArrayGetItem(1, 0)")]
#[case("States.ArrayGetItem($.a, -1)")]
#[case("States.ArrayGetItem($.a, 1.0)")]
#[case("States.ArrayGetItem($.a, 1)")]
#[case("States.ArrayPartition($.a, 0)")]
#[case("States.ArrayPartition($.a, -1)")]
#[case("States.ArrayPartition($.a, 0.49)")]
#[case("States.MathAdd('1', 0)")]
#[case("States.MathAdd(2147483647, 1)")]
#[case("States.MathAdd(-2147483648, -1)")]
#[case("States.MathAdd(2147483648, 0)")]
#[case("States.StringSplit(1, ',')")]
#[case("States.Format('{}', $.a)")]
#[case("States.JsonMerge($.a, $.a, false)")]
fn test_should_return_typed_intrinsic_value_failures(#[case] text: &str) {
    assert!(
        matches!(
            evaluate(text, &json!({"a":[1]})),
            Err(AslError::IntrinsicValue)
        ),
        "{text}"
    );
}

#[test]
fn test_should_preserve_context_and_quoted_argument_paths() -> TestResult {
    let data = Payload::from_value(&json!({"a,b)":1,"quote'key":2}))?;
    let context = EvaluationContext::from_value(&json!({"Execution":{"Name":"run"}}))?;
    let expression: Expression =
        r"States.Array($['a,b)'], $['quote\'key'], $$.Execution.Name)".parse()?;
    assert_eq!(
        expression
            .evaluate_with_source(&data, &context, &mut Fixed::default())?
            .value(),
        &json!([1, 2, "run"])
    );
    assert!(matches!(
        evaluate("States.Array($.missing)", &json!({})),
        Err(AslError::Missing)
    ));
    assert!(matches!(
        evaluate("States.Format($.format)", &json!({"format":"{"})),
        Err(AslError::IntrinsicValue)
    ));
    assert!(matches!(
        evaluate(
            "States.StringToJson($.text)",
            &json!({"text":"{\"a\":1,\"a\":2}"})
        ),
        Err(AslError::Json(JsonError::Duplicate))
    ));
    Ok(())
}

#[test]
fn test_should_enforce_expression_bytes_argument_count_and_call_nesting() -> TestResult {
    let fixed = "States.Array('')".len();
    let exact = format!("States.Array('{}')", "x".repeat(4096 - fixed));
    assert_eq!(exact.len(), 4096);
    exact.parse::<Expression>()?;
    assert!(format!("{exact}x").parse::<Expression>().is_err());
    format!("States.Array({})", vec!["0"; 128].join(",")).parse::<Expression>()?;
    assert!(
        format!("States.Array({})", vec!["0"; 129].join(","))
            .parse::<Expression>()
            .is_err()
    );
    assert!(
        format!("States.Array({})", "1".repeat(129))
            .parse::<Expression>()
            .is_err()
    );
    let mut nested = "$.x".to_owned();
    for _ in 0..10 {
        nested = format!("States.Array({nested})");
    }
    evaluate(&nested, &json!({"x":1}))?;
    assert!(
        format!("States.Array({nested})")
            .parse::<Expression>()
            .is_err()
    );
    let full = format!("States.Array({})", vec!["0"; 127].join(","));
    let short = format!("States.Array({})", vec!["0"; 126].join(","));
    let mut groups = vec![full.clone(); 7];
    groups.push(short);
    let exact_nodes = format!("States.Array({})", groups.join(","));
    assert!(exact_nodes.len() < 4096);
    evaluate(&exact_nodes, &json!(null))?;
    groups.pop();
    groups.push(full);
    assert!(matches!(
        format!("States.Array({})", groups.join(",")).parse::<Expression>(),
        Err(AslError::InvalidExpression)
    ));
    Ok(())
}

#[test]
fn test_should_bound_output_and_share_path_data_and_materialization_budgets() -> TestResult {
    assert!(matches!(
        evaluate(
            "States.Format('{}{}', $.x, $.x)",
            &json!({"x":"a".repeat(140_000)})
        ),
        Err(AslError::DataLimit)
    ));
    assert!(matches!(
        evaluate(
            "States.JsonToString($.x)",
            &json!({"x":"\"".repeat(130_000)})
        ),
        Err(AslError::DataLimit)
    ));
    let many = format!("States.Array({})", vec!["$.x"; 128].join(","));
    assert!(matches!(
        evaluate(&many, &json!({"x":"a".repeat(100_000)})),
        Err(AslError::WorkLimit)
    ));
    assert_eq!(
        evaluate("States.ArrayLength($[*])", &json!(vec![0; 9997]))?.value(),
        &json!(9997)
    );
    assert!(matches!(
        evaluate("States.ArrayLength($[*])", &json!(vec![0; 9998])),
        Err(AslError::WorkLimit)
    ));
    assert!(matches!(
        evaluate(
            "States.ArrayContains($.a, $.x)",
            &json!({"a":vec!["a";100],"x":"a".repeat(100_000)})
        ),
        Err(AslError::WorkLimit)
    ));
    let too_many: String = (0x1000..0x1401).filter_map(char::from_u32).collect();
    assert!(matches!(
        evaluate(
            "States.StringSplit('x', $.delim)",
            &json!({"delim":too_many})
        ),
        Err(AslError::WorkLimit)
    ));
    Ok(())
}

#[test]
fn test_should_keep_large_integer_equality_exact_and_reject_float_aliases() -> TestResult {
    assert_eq!(
        evaluate(
            "States.ArrayContains($.a, $.x)",
            &json!({"a":[u64::MAX],"x":u64::MAX})
        )?
        .value(),
        &json!(true)
    );
    assert_eq!(
        evaluate(
            "States.ArrayContains($.a, $.x)",
            &json!({"a":[u64::MAX],"x":u64::MAX-1})
        )?
        .value(),
        &json!(false)
    );
    let data = Payload::parse(br#"{"a":[9007199254740992],"x":9007199254740993.0}"#)?;
    let expression: Expression = "States.ArrayContains($.a, $.x)".parse()?;
    assert!(matches!(
        expression.evaluate(&data, &EvaluationContext::from(&data)),
        Err(AslError::NumericRange)
    ));
    Ok(())
}

#[test]
fn test_should_charge_duplicate_delimiters_and_failed_object_key_scans() -> TestResult {
    assert!(matches!(
        evaluate(
            "States.StringSplit('x', $.separators)",
            &json!({"separators":"a".repeat(20_000)})
        ),
        Err(AslError::WorkLimit)
    ));
    let mut candidate = Map::new();
    for index in 0..199 {
        candidate.insert(format!("k{index:03}"), json!(0));
    }
    let mut needle = candidate.clone();
    candidate.insert("z".into(), json!(0));
    needle.insert("zz".into(), json!(0));
    let small = json!({"items":[candidate],"needle":needle});
    assert_eq!(
        evaluate("States.ArrayContains($.items, $.needle)", &small)?.value(),
        &json!(false)
    );
    let repeated = json!({"items":vec![candidate;100],"needle":needle});
    assert!(matches!(
        evaluate("States.ArrayContains($.items, $.needle)", &repeated),
        Err(AslError::WorkLimit)
    ));
    Ok(())
}

struct Fails;
impl UuidSource for Fails {
    fn next_uuid(&mut self) -> Result<Uuid, AslError> {
        Err(AslError::Entropy(getrandom::Error::UNSUPPORTED))
    }
}
struct InvalidId;
impl UuidSource for InvalidId {
    fn next_uuid(&mut self) -> Result<Uuid, AslError> {
        Ok(Uuid::nil())
    }
}

#[test]
fn test_should_use_typed_uuid_sources_and_retain_redacted_entropy_failure() -> TestResult {
    let expression: Expression = "States.UUID()".parse()?;
    let data = Payload::parse(b"null")?;
    let context = EvaluationContext::from(&data);
    let mut system = SystemUuidSource;
    let mut ids = BTreeSet::new();
    for _ in 0..8 {
        let id = system.next_uuid()?;
        assert_eq!(id.get_version(), Some(Version::Random));
        ids.insert(id);
    }
    assert_eq!(ids.len(), 8);
    let fixed = expression.evaluate_with_source(&data, &context, &mut Fixed::default())?;
    assert_eq!(
        fixed.value(),
        &json!(Builder::from_random_bytes([1; 16]).into_uuid().to_string())
    );
    let error = expression
        .evaluate_with_source(&data, &context, &mut Fails)
        .err()
        .ok_or("missing failure")?;
    assert!(error.source().is_some());
    assert_eq!(format!("{error:?}"), "ASL UUID entropy unavailable");
    assert!(matches!(
        expression.evaluate_with_source(&data, &context, &mut InvalidId),
        Err(AslError::IntrinsicValue)
    ));
    assert_eq!(
        format!("{:?}", "States.Array('private')".parse::<Expression>()?),
        "Expression(<redacted>)"
    );
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]
    #[test]
    fn test_should_parse_arbitrary_utf8_expressions_without_panicking(text in ".{0,4200}") { let _=text.parse::<Expression>(); }

    #[test]
    fn test_should_preserve_checked_integer_addition(a in -1_000_000_i32..1_000_000,b in -1_000_000_i32..1_000_000) {
        let result=evaluate("States.MathAdd($.a,$.b)",&json!({"a":a,"b":b})).map_err(|e|TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(result.value(),&json!(a+b));
    }
}

#[rstest]
#[case("-0", json!(0))]
#[case(" [ -0, {\"x\": -0, \"-0\": \"-0\"} ] ", json!([0,{"x":0,"-0":"-0"}]))]
#[case(r#"{"escaped":"\\\"-0","unicode":"🙂-0","n":-0}"#, json!({"escaped":"\\\"-0","unicode":"🙂-0","n":0}))]
#[case("[-0.0,-0e0,-1,0]", json!([-0.0,-0.0,-1,0]))]
fn test_should_normalize_only_complete_unquoted_negative_integer_zero_tokens(
    #[case] source: &str,
    #[case] expected: Value,
) -> TestResult {
    let result = evaluate("States.StringToJson($.text)", &json!({"text":source}))?;
    assert_eq!(result.value(), &expected);
    Ok(())
}

#[rstest]
#[case("-00")]
#[case("[-0,]")]
#[case("{\"x\":-0,\"x\":0}")]
#[case(" ")]
fn test_should_reject_invalid_json_before_negative_zero_normalization(#[case] source: &str) {
    assert!(evaluate("States.StringToJson($.text)", &json!({"text":source})).is_err());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]
    #[test]
    fn test_should_preserve_arbitrary_quoted_text_while_normalizing_integer_zero(text in ".{0,128}") {
        let quoted = serde_json::to_string(&text).map_err(|error|TestCaseError::fail(error.to_string()))?;
        let source = format!("[{quoted},-0,-0.0]");
        let result = evaluate("States.StringToJson($.text)",&json!({"text":source})).map_err(|error|TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(result.value(),&json!([text,0,-0.0]));
    }
}

#[rstest]
#[case("1e-0")]
#[case("-0e-0")]
#[case("[-0.0,1e-0,\"-0\"]")]
fn test_should_preserve_original_zero_exponent_and_string_spellings(
    #[case] source: &str,
) -> TestResult {
    let result = evaluate("States.StringToJson($.text)", &json!({"text":source}))?;
    assert_eq!(result.original_bytes(), source.as_bytes());
    Ok(())
}

#[test]
fn test_should_charge_the_conditional_normalized_json_reparse() -> TestResult {
    let expression: Expression = "States.StringToJson($.text)".parse()?;
    let data = Payload::from_value(&json!({"text":"-0"}))?;
    let context = EvaluationContext::from(&data);
    for (text, accepted) in [(4, false), (5, true)] {
        let mut budget = Budget {
            text,
            ..Budget::new()
        };
        let result =
            expression.evaluate_scoped(&data, &context, &mut Fixed::default(), &mut budget);
        if accepted {
            assert_eq!(result?.value(), &json!(0));
        } else {
            assert!(matches!(result, Err(AslError::WorkLimit)));
        }
    }
    Ok(())
}
