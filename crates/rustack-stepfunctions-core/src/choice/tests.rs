use std::error::Error;

use proptest::prelude::*;
use rstest::rstest;
use rustack_core::json::JsonError;
use serde_json::{Value, json};

use crate::{AslError, EvaluationContext, Payload, Predicate};

type TestResult = Result<(), Box<dyn Error>>;

fn evaluate(rule: &Value, data: &Value) -> Result<bool, AslError> {
    let predicate = Predicate::compile(rule)?;
    let data = Payload::from_value(data)?;
    predicate.evaluate(&data, &EvaluationContext::from(&data))
}

#[test]
fn test_should_execute_all_independent_choice_catalog_examples() -> TestResult {
    let catalog: Value = serde_json::from_str(include_str!(
        "../../../../tests/workflows/semantic-cases.json"
    ))?;
    let mut count = 0;
    for case in catalog["cases"].as_array().ok_or("missing cases")? {
        if case["kind"] != "choice" {
            continue;
        }
        assert_eq!(
            evaluate(&case["rule"], &case["input"].clone())?,
            case["expected"].as_bool().ok_or("missing expected")?
        );
        count += 1;
    }
    assert_eq!(count, 4);
    Ok(())
}

#[test]
fn test_should_cover_all_ordered_literal_and_path_operator_variants() -> TestResult {
    let families = [
        ("String", vec![json!("a"), json!("b"), json!("c")]),
        ("Numeric", vec![json!(-1), json!(0), json!(1)]),
        (
            "Timestamp",
            vec![
                json!("2020-01-01T00:00:00Z"),
                json!("2020-01-02T00:00:00Z"),
                json!("2020-01-03T00:00:00Z"),
            ],
        ),
    ];
    let operators = [
        ("Equals", [false, true, false]),
        ("LessThan", [true, false, false]),
        ("GreaterThan", [false, false, true]),
        ("LessThanEquals", [true, true, false]),
        ("GreaterThanEquals", [false, true, true]),
    ];
    for (family, values) in families {
        let middle = values.get(1).ok_or("missing middle")?;
        for (suffix, expected) in operators {
            for path in [false, true] {
                for (index, left) in values.iter().enumerate() {
                    let op = format!("{family}{suffix}{}", if path { "Path" } else { "" });
                    let operand = if path {
                        json!("$.right")
                    } else {
                        middle.clone()
                    };
                    let rule = json!({"Variable":"$.left",op:operand});
                    assert_eq!(
                        evaluate(&rule, &json!({"left":left,"right":middle}))?,
                        *expected.get(index).ok_or("missing expected")?,
                        "{family} {suffix} {path} {index}"
                    );
                }
            }
        }
    }
    for path in [false, true] {
        for value in [false, true] {
            let op = if path {
                "BooleanEqualsPath"
            } else {
                "BooleanEquals"
            };
            let operand = if path { json!("$.right") } else { json!(true) };
            assert_eq!(
                evaluate(
                    &json!({"Variable":"$.left",op:operand}),
                    &json!({"left":value,"right":true})
                )?,
                value
            );
        }
    }
    Ok(())
}

#[rstest]
#[case("IsPresent",json!(null),true)]
#[case("IsNull",json!(null),true)]
#[case("IsString",json!("7"),true)]
#[case("IsNumeric",json!(7),true)]
#[case("IsBoolean",json!(true),true)]
#[case("IsTimestamp",json!("2020-01-01T00:00:00Z"),true)]
#[case("IsNull",json!(0),false)]
#[case("IsString",json!(7),false)]
#[case("IsNumeric",json!("7"),false)]
#[case("IsBoolean",json!("true"),false)]
#[case("IsTimestamp",json!("invalid"),false)]
#[case("IsTimestamp",json!("2016-12-31T23:59:60Z"),false)]
fn test_should_apply_type_tests_without_coercion(
    #[case] op: &str,
    #[case] value: Value,
    #[case] expected: bool,
) -> TestResult {
    for requested in [true, false] {
        assert_eq!(
            evaluate(&json!({"Variable":"$.x",op:requested}), &json!({"x":value}))?,
            expected == requested
        );
    }
    Ok(())
}

#[test]
fn test_should_short_circuit_in_definition_order_and_preserve_missing_errors() -> TestResult {
    let present = json!({"Variable":"$.absent","IsPresent":true});
    let missing = json!({"Variable":"$.absent","IsString":true});
    assert!(!evaluate(&json!({"And":[present,missing]}), &json!({}))?);
    assert!(evaluate(
        &json!({"Or":[{"Not":present},missing]}),
        &json!({})
    )?);
    assert!(matches!(
        evaluate(&json!({"And":[missing,present]}), &json!({})),
        Err(AslError::Missing)
    ));
    for op in [
        "IsNull",
        "IsString",
        "IsNumeric",
        "IsBoolean",
        "IsTimestamp",
    ] {
        assert!(matches!(
            evaluate(&json!({"Variable":"$.absent",op:false}), &json!({})),
            Err(AslError::Missing)
        ));
    }
    assert!(matches!(
        evaluate(
            &json!({"Variable":"$.x","StringEqualsPath":"$.absent"}),
            &json!({"x":1})
        ),
        Err(AslError::Missing)
    ));
    Ok(())
}

#[test]
fn test_should_compare_instants_and_avoid_numeric_precision_aliasing() -> TestResult {
    assert!(!evaluate(
        &json!({"Variable":"$.x","TimestampEquals":"2020-01-01T00:00:00.1234567891Z"}),
        &json!({"x":"2020-01-01T00:00:00.1234567892Z"})
    )?);
    assert!(evaluate(
        &json!({"Variable":"$.x","TimestampEquals":"2020-01-01T00:00:00.1234567891000Z"}),
        &json!({"x":"2020-01-01T01:00:00.1234567891+01:00"})
    )?);
    assert!(evaluate(
        &json!({"Variable":"$.x","TimestampEquals":"2020-01-01T00:00:00.123Z"}),
        &json!({"x":"2020-01-01T01:00:00.123+01:00"})
    )?);
    assert!(!evaluate(
        &json!({"Variable":"$.x","TimestampEquals":"2020-01-01T00:00:00Z"}),
        &json!({"x":"2020-01-01"})
    )?);
    assert!(evaluate(
        &json!({"Variable":"$.x","NumericEquals":9_007_199_254_740_992_u64}),
        &json!({"x":9_007_199_254_740_992_u64})
    )?);
    assert!(matches!(
        evaluate(
            &json!({"Variable":"$.x","NumericEquals":9_007_199_254_740_992_u64}),
            &json!({"x":9_007_199_254_740_993_u64})
        ),
        Err(AslError::NumericRange)
    ));
    assert!(matches!(
        evaluate(
            &json!({"Variable":"$.x","NumericEqualsPath":"$.right"}),
            &json!({"x":0,"right":-9_007_199_254_740_993_i64})
        ),
        Err(AslError::NumericRange)
    ));
    assert!(evaluate(
        &json!({"Variable":"$.x","IsNumeric":true}),
        &json!({"x":u64::MAX})
    )?);
    for (op, operand, left) in [
        ("StringEquals", json!("1"), json!(1)),
        ("NumericEquals", json!(1), json!("1")),
        ("BooleanEquals", json!(true), json!(1)),
    ] {
        assert!(!evaluate(
            &json!({"Variable":"$.x",op:operand}),
            &json!({"x":left})
        )?);
    }
    let predicate =
        Predicate::compile(&json!({"Variable":"$$.State.Name","StringEquals":"current"}))?;
    assert!(predicate.evaluate(
        &Payload::parse(b"null")?,
        &EvaluationContext::from_value(&json!({"State":{"Name":"current"}}))?
    )?);
    Ok(())
}

#[test]
fn test_should_reject_raw_float_boundary_lexemes_instead_of_aliasing_large_numbers() -> TestResult {
    let integer = Predicate::parse(br#"{"Variable":"$.x","NumericEquals":9007199254740992}"#)?;
    for text in [
        "9007199254740993.0",
        "9007199254740993e0",
        "9007199254740992.0",
        "-9007199254740993.0",
        "-9007199254740993e0",
    ] {
        let data = Payload::parse(format!("{{\"x\":{text}}}").as_bytes())?;
        assert!(
            matches!(
                integer.evaluate(&data, &EvaluationContext::from(&data)),
                Err(AslError::NumericRange)
            ),
            "{text}"
        );
        assert!(
            matches!(
                Predicate::parse(
                    format!("{{\"Variable\":\"$.x\",\"NumericEquals\":{text}}}").as_bytes()
                ),
                Err(AslError::InvalidPredicate)
            ),
            "{text}"
        );
    }
    let inside = Predicate::parse(br#"{"Variable":"$.x","NumericEquals":9007199254740991.0}"#)?;
    let data = Payload::parse(br#"{"x":9007199254740991e0}"#)?;
    assert!(inside.evaluate(&data, &EvaluationContext::from(&data))?);
    Ok(())
}

#[test]
fn test_should_charge_precompiled_timestamp_literal_bytes_to_shared_budget() -> TestResult {
    let actual = format!("2020-01-01T00:00:00.{}Z", "0".repeat(180_000));
    let expected = format!("2020-01-01T00:00:00.{}1Z", "0".repeat(180_000));
    let comparison = json!({"Variable":"$.x","TimestampLessThan":expected});
    let data = Payload::from_value(&json!({"x":actual}))?;
    let two = Predicate::compile(&json!({"And":[comparison,comparison]}))?;
    assert!(two.evaluate(&data, &EvaluationContext::from(&data))?);
    let three = Predicate::compile(&json!({"And":[comparison,comparison,comparison]}))?;
    assert!(matches!(
        three.evaluate(&data, &EvaluationContext::from(&data)),
        Err(AslError::WorkLimit)
    ));
    let timestamp = format!("2020-01-01T00:00:00.{}1Z", "0".repeat(220_000));
    let comparison = json!({"Variable":"$.x","TimestampLessThan":timestamp});
    let rule = json!({"And":[comparison,comparison,comparison,comparison,comparison]});
    assert!(Predicate::compile(&rule).is_err());
    // A shared Path operand supplies the same long fraction repeatedly without
    // exceeding the separately bounded source definition.
    let comparison = json!({"Variable":"$.x","TimestampLessThanPath":"$.right"});
    let data = Payload::from_value(&json!({"x":"2020-01-01T00:00:00Z","right":timestamp}))?;
    let rule = Predicate::compile(
        &json!({"And":[comparison,comparison,comparison,comparison,comparison]}),
    )?;
    assert!(matches!(
        rule.evaluate(&data, &EvaluationContext::from(&data)),
        Err(AslError::WorkLimit)
    ));
    Ok(())
}

#[rstest]
#[case("", "", true)]
#[case("*", "", true)]
#[case("**", "中🙂", true)]
#[case("中*文", "中🙂文", true)]
#[case("a\\*b", "a*b", true)]
#[case("a\\\\b", "a\\b", true)]
#[case("a?b", "a?b", true)]
#[case("a?b", "axb", false)]
#[case("*aba*b", "ababab", true)]
#[case("a*b*c", "abcc", true)]
#[case("a*b*c", "abdcx", false)]
#[case("literal", "Literal", false)]
fn test_should_match_compiled_unicode_globs(
    #[case] pattern: &str,
    #[case] input: &str,
    #[case] expected: bool,
) -> TestResult {
    assert_eq!(
        evaluate(
            &json!({"Variable":"$.x","StringMatches":pattern}),
            &json!({"x":input})
        )?,
        expected
    );
    Ok(())
}

#[rstest]
#[case(json!({}))]
#[case(json!([]))]
#[case(json!({"And":[]}))]
#[case(json!({"Or":[]}))]
#[case(json!({"And":[],"Not":{}}))]
#[case(json!({"Variable":"$.x","IsNull":"true"}))]
#[case(json!({"Variable":"$.x","NumericEquals":true}))]
#[case(json!({"Variable":"$.x","NumericEquals":u64::MAX}))]
#[case(json!({"Variable":"$.x","TimestampEquals":"invalid"}))]
#[case(json!({"Variable":"$.x","StringEquals":7}))]
#[case(json!({"Variable":"$.x","StringMatches":"\\q"}))]
#[case(json!({"Variable":"$.x","StringMatches":"\\"}))]
#[case(json!({"Variable":"$.x","IsNull":true,"IsString":false}))]
#[case(json!({"Variable":"$.x","Unknown":true}))]
#[case(json!({"Variable":"$.x","StringMatchesPath":"$.y"}))]
fn test_should_reject_ambiguous_invalid_or_unknown_predicates(#[case] value: Value) {
    assert!(matches!(
        Predicate::compile(&value),
        Err(AslError::InvalidPredicate)
    ));
}

#[test]
fn test_should_bound_source_structure_compiled_nodes_depth_and_glob_tokens() -> TestResult {
    assert!(matches!(
        Predicate::parse(br#"{"Variable":"$.a","IsNull":true,"IsNull":false}"#),
        Err(AslError::Json(JsonError::Duplicate))
    ));
    assert!(Predicate::compile(&json!({"Variable":"$[*]","IsNull":true})).is_err());
    assert!(Predicate::compile(&json!({"Variable":"$.x","StringEqualsPath":"$[*]"})).is_err());
    let leaf = json!({"Variable":"$.x","IsPresent":true});
    let mut nested = leaf.clone();
    for _ in 0..31 {
        nested = json!({"Not":nested});
    }
    Predicate::compile(&nested)?;
    assert!(matches!(
        Predicate::compile(&json!({"Not":nested})),
        Err(AslError::InvalidPredicate)
    ));
    Predicate::compile(&json!({"And":vec![leaf.clone();128]}))?;
    assert!(matches!(
        Predicate::compile(&json!({"And":vec![leaf.clone();129]})),
        Err(AslError::InvalidPredicate)
    ));
    let group = json!({"And":vec![leaf;8]});
    assert!(matches!(
        Predicate::compile(&json!({"And":vec![group;128]})),
        Err(AslError::InvalidPredicate)
    ));
    Predicate::compile(&json!({"Variable":"$.x","StringMatches":"🙂".repeat(1024)}))?;
    assert!(matches!(
        Predicate::compile(&json!({"Variable":"$.x","StringMatches":"a".repeat(1025)})),
        Err(AslError::InvalidPredicate)
    ));
    assert!(matches!(
        Predicate::compile(&json!({"Variable":"$.x","StringMatches":"🙂".repeat(1025)})),
        Err(AslError::InvalidPredicate)
    ));
    Ok(())
}

#[test]
fn test_should_share_step_and_string_byte_limits_across_the_complete_predicate() -> TestResult {
    assert!(matches!(
        evaluate(
            &json!({"Variable":"$.x","StringMatches":"*b"}),
            &json!({"x":"a".repeat(20_000)})
        ),
        Err(AslError::WorkLimit)
    ));
    assert!(evaluate(
        &json!({"Variable":"$.x","StringMatches":"a*"}),
        &json!({"x":"a".repeat(20_000)})
    )?);
    let comparison = json!({"Variable":"$.x","StringEqualsPath":"$.x"});
    let input = json!({"x":"a".repeat(260_000)});
    assert!(evaluate(
        &json!({"And":[comparison,comparison]}),
        &input.clone()
    )?);
    assert!(matches!(
        evaluate(&json!({"And":[comparison,comparison,comparison]}), &input),
        Err(AslError::WorkLimit)
    ));
    Ok(())
}

#[test]
fn test_should_reject_borrowed_extreme_predicate_without_recursive_drop_and_redact() -> TestResult {
    let mut tree = Value::Null;
    for _ in 0..50_000 {
        tree = Value::Array(vec![tree]);
    }
    let rejected = Predicate::compile(&tree);
    let mut owned = vec![tree];
    while let Some(value) = owned.pop() {
        if let Value::Array(mut children) = value {
            owned.append(&mut children);
        }
    }
    assert!(matches!(rejected, Err(AslError::Json(JsonError::Depth))));
    let predicate = Predicate::compile(&json!({"Variable":"$.secret","StringEquals":"private"}))?;
    assert_eq!(format!("{predicate:?}"), "Predicate(<redacted>)");
    Ok(())
}

fn glob_oracle(pattern: &str, text: &str) -> bool {
    let chars: Vec<_> = text.chars().collect();
    let mut row = vec![false; chars.len() + 1];
    row[0] = true;
    for ch in pattern.chars() {
        let mut next = vec![false; row.len()];
        if ch == '*' {
            next[0] = row[0];
        }
        for (i, input) in chars.iter().enumerate() {
            next[i + 1] = if ch == '*' {
                next[i] || row[i + 1]
            } else {
                row[i] && ch == *input
            };
        }
        row = next;
    }
    row.last().copied().unwrap_or(false)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]
    #[test]
    fn test_should_match_an_independent_glob_dynamic_program(pattern in "[ab*]{0,12}", text in "[ab]{0,18}") {
        let actual=evaluate(&json!({"Variable":"$.x","StringMatches":pattern}),&json!({"x":text})).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(actual,glob_oracle(&pattern,&text));
    }

    #[test]
    fn test_should_preserve_exact_integer_order_inside_the_supported_range(a in -1_000_000_i64..1_000_000, b in -1_000_000_i64..1_000_000) {
        let actual=evaluate(&json!({"Variable":"$.a","NumericLessThanPath":"$.b"}),&json!({"a":a,"b":b})).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(actual,a<b);
    }
}
