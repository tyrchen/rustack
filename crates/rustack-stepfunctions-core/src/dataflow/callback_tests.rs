//! Structural provenance, single source consumption and actual history DTO boundaries.

use std::error::Error;

use chrono::Utc;
use proptest::prelude::*;
use rstest::rstest;
use rustack_stepfunctions_model::types::{
    HistoryEvent, HistoryEventType, TaskScheduledEventDetails,
};
use serde_json::{Value, json};
use uuid::{Builder, Uuid};

use crate::{
    AslError, CallbackParameters, CallbackToken, DataFlow, EvaluationContext, MAX_PAYLOAD_BYTES,
    Payload, UuidSource,
};

type TestResult = Result<(), Box<dyn Error>>;
#[derive(Debug, Default)]
struct Ids {
    calls: u8,
    fail: bool,
}
impl UuidSource for Ids {
    fn next_uuid(&mut self) -> Result<Uuid, AslError> {
        self.calls = self.calls.checked_add(1).ok_or(AslError::WorkLimit)?;
        if self.fail {
            return Err(AslError::WorkLimit);
        }
        Ok(Builder::from_random_bytes([self.calls; 16]).into_uuid())
    }
}
fn secret() -> String {
    "a".repeat(64)
}
fn token() -> CallbackToken {
    secret().parse().unwrap()
}
fn original() -> Payload {
    Payload::from_value(&json!({"n":21,"Task":{"Token":"business"},"extra":{"flag":true}})).unwrap()
}
fn context() -> EvaluationContext {
    EvaluationContext::from_value(&json!({"Execution":{"Name":"Run"},"State":{"Name":"Call","RetryCount":2},"TaskLike":{"Token":"ordinary"}})).unwrap()
}
fn prepare(config: &Value) -> Result<CallbackParameters, AslError> {
    DataFlow::compile(config)?.prepare_callback_with_source(
        &original(),
        &context(),
        &token(),
        &mut Ids::default(),
    )
}

#[rstest]
#[case("$", false, false)]
#[case("$.n", false, false)]
#[case("$['Task']['Token']", false, false)]
#[case("$$.Task.Token", true, true)]
#[case("$$['Task']['Token']", true, true)]
#[case("$$.Task", true, true)]
#[case("$$", true, true)]
#[case("$$[*]", true, true)]
#[case("$$[*].Token", true, true)]
#[case("$$.Task[*]", true, true)]
#[case("$$[*][*]", true, true)]
#[case("$$.Task.Token[*]", false, false)]
#[case("$$.Task[0:1]", false, false)]
#[case("$$.Task[*][0:1]", false, false)]
#[case("$$.TaskLike.Token", false, false)]
#[case("$$['TaskLike'].Token", false, false)]
#[case("$$.State[*]", false, false)]
#[case("$$.Execution", false, false)]
#[case("$$.Execution.Name", false, false)]
fn test_should_classify_compiled_context_token_paths_ancestors_and_wildcards(
    #[case] path: &str,
    #[case] sensitive: bool,
    #[case] contains_token: bool,
) -> TestResult {
    let pair = prepare(&json!({"Parameters":{"value.$":path}}))?;
    assert_eq!(pair.history().redacted(), sensitive);
    assert_eq!(
        pair.outbound().value()["value"]
            .to_string()
            .contains(&secret()),
        contains_token
    );
    if sensitive {
        assert!(pair.history().payload().value()["value"].is_null());
    } else {
        assert_eq!(pair.history().payload().value(), pair.outbound().value());
    }
    Ok(())
}
#[rstest]
#[case("States.Format('{}:{}', $$.Task.Token, States.UUID())", true, true, 1)]
#[case("States.JsonToString($$.Task)", true, true, 0)]
#[case("States.StringToJson(States.JsonToString($$.Task))", true, true, 0)]
#[case(
    "States.ArrayGetItem(States.Array($$.Task.Token, 'public'),1)",
    true,
    false,
    0
)]
#[case("States.ArrayLength($$.Task[*])", true, false, 0)]
#[case(
    "States.ArrayContains(States.Array($$.Task.Token),'public')",
    true,
    false,
    0
)]
#[case("States.ArrayPartition(States.Array($$.Task.Token),1)", true, true, 0)]
#[case("States.StringSplit($$.Task.Token,'x')", true, true, 0)]
#[case(
    "States.MathAdd(States.ArrayLength(States.Array($$.Task.Token)),1)",
    true,
    false,
    0
)]
#[case("States.JsonMerge($$.Task,$.extra,false)", true, true, 0)]
#[case("States.UUID()", false, false, 1)]
#[case("States.Format('literal:$$.Task.Token')", false, false, 0)]
fn test_should_conservatively_propagate_sensitive_intrinsic_arguments_without_scanning_results(
    #[case] expression: &str,
    #[case] sensitive: bool,
    #[case] contains_token: bool,
    #[case] calls: u8,
) -> TestResult {
    let mut source = Ids::default();
    let flow = DataFlow::compile(&json!({"Parameters":{"value.$":expression}}))?;
    let pair = flow.prepare_callback_with_source(&original(), &context(), &token(), &mut source)?;
    assert_eq!(source.calls, calls);
    assert_eq!(pair.history().redacted(), sensitive);
    assert_eq!(
        pair.outbound().value()["value"]
            .to_string()
            .contains(&secret()),
        contains_token
    );
    if sensitive {
        assert!(pair.history().payload().value()["value"].is_null());
    } else {
        assert_eq!(pair.outbound().value(), pair.history().payload().value());
    }
    Ok(())
}
#[rstest]
#[case(Some("$$.Task"), "$.Token", true)]
#[case(Some("$$"), "$.Task.Token", true)]
#[case(Some("$$"), "$.Execution.Name", true)]
#[case(Some("$$.Execution"), "$.Name", false)]
#[case(Some("$"), "$.n", false)]
#[case(None, "$$.Task.Token", true)]
fn test_should_propagate_sensitive_effective_input_into_data_references(
    #[case] input: Option<&str>,
    #[case] expression: &str,
    #[case] sensitive: bool,
) -> TestResult {
    let pair = prepare(
        &json!({"InputPath":input,"Parameters":{"value.$":expression,"id.$":"States.UUID()","literal":true}}),
    )?;
    assert_eq!(pair.history().redacted(), sensitive);
    if sensitive {
        assert!(pair.history().payload().value()["value"].is_null());
    } else {
        assert_eq!(
            pair.history().payload().value()["value"],
            pair.outbound().value()["value"]
        );
    }
    assert_eq!(
        pair.history().payload().value()["id"],
        pair.outbound().value()["id"]
    );
    assert_eq!(pair.history().payload().value()["literal"], true);
    Ok(())
}
#[rstest]
#[case(Some("$$.Task"), true)]
#[case(Some("$$.Task.Token"), true)]
#[case(Some("$$"), true)]
#[case(Some("$"), false)]
#[case(None, false)]
fn test_should_project_omitted_parameters_with_effective_input_provenance(
    #[case] input: Option<&str>,
    #[case] sensitive: bool,
) -> TestResult {
    let pair = prepare(&json!({"InputPath":input}))?;
    assert_eq!(pair.history().redacted(), sensitive);
    if sensitive {
        assert!(pair.history().payload().value().is_null());
    } else {
        assert_eq!(pair.history().payload().value(), pair.outbound().value());
    }
    Ok(())
}
#[test]
fn test_should_preserve_literal_and_business_values_equal_to_token_without_reinterpreting_text()
-> TestResult {
    let input = Payload::from_value(&json!({"same":secret()}))?;
    let flow = DataFlow::compile(
        &json!({"Parameters":{"fixed":secret(),"business.$":"$.same","text":"$$.Task.Token","actual.$":"$$.Task.Token"}}),
    )?;
    let pair =
        flow.prepare_callback_with_source(&input, &context(), &token(), &mut Ids::default())?;
    assert_eq!(pair.history().payload().value()["fixed"], secret());
    assert_eq!(pair.history().payload().value()["business"], secret());
    assert_eq!(pair.history().payload().value()["text"], "$$.Task.Token");
    assert!(pair.history().payload().value()["actual"].is_null());
    Ok(())
}
#[test]
fn test_should_evaluate_nested_template_once_and_preserve_untainted_uuid_in_both_projections()
-> TestResult {
    let original = Payload::parse(b" { \"n\":21 } ")?;
    let base = context();
    let before = base.value().clone();
    let mut source = Ids::default();
    let flow = DataFlow::compile(
        &json!({"Parameters":{"a.$":"States.UUID()","nested":[{"token.$":"$$.Task.Token","n.$":"$.n"},"$$.Task.Token"],"z.$":"States.Format('{}:{}',$$.Task.Token,States.UUID())"}}),
    )?;
    let pair = flow.prepare_callback_with_source(&original, &base, &token(), &mut source)?;
    assert_eq!(source.calls, 2);
    assert_eq!(
        pair.outbound().value()["a"],
        pair.history().payload().value()["a"]
    );
    assert!(Uuid::parse_str(pair.outbound().value()["a"].as_str().ok_or("UUID")?).is_ok());
    assert!(pair.history().payload().value()["nested"][0]["token"].is_null());
    assert_eq!(pair.history().payload().value()["nested"][0]["n"], 21);
    assert_eq!(
        pair.history().payload().value()["nested"][1],
        "$$.Task.Token"
    );
    assert!(pair.history().payload().value()["z"].is_null());
    assert_eq!(base.value(), &before);
    assert_eq!(original.original_bytes(), b" { \"n\":21 } ");
    for rendered in [
        format!("{pair:?}"),
        format!("{:?}", pair.history()),
        format!("{base:?}"),
    ] {
        assert!(!rendered.contains(&secret()));
    }
    assert!(base.value().get("Task").is_none());
    Ok(())
}
#[rstest]
#[case(&json!({"Task":null}))]
#[case(&json!({"Task":{}}))]
#[case(&json!({"Task":{"Token":"old"}}))]
#[case(&json!({"Task":"wrong"}))]
#[case(&json!([]))]
#[case(&json!(null))]
fn test_should_reject_stale_or_nonobject_context_before_consuming_uuid_source(
    #[case] value: &Value,
) -> TestResult {
    let base = EvaluationContext::from_value(value)?;
    let mut source = Ids::default();
    let flow = DataFlow::compile(
        &json!({"Parameters":{"id.$":"States.UUID()","token.$":"$$.Task.Token"}}),
    )?;
    assert!(matches!(
        flow.prepare_callback_with_source(&original(), &base, &token(), &mut source),
        Err(AslError::InvalidCallbackContext)
    ));
    assert_eq!(source.calls, 0);
    assert_eq!(base.value(), value);
    Ok(())
}
#[test]
fn test_should_keep_source_and_selection_failures_without_returning_partial_pair() -> TestResult {
    let input = original();
    let base = context();
    let flow =
        DataFlow::compile(&json!({"Parameters":{"a.$":"States.UUID()","z.$":"$$.Task.Token"}}))?;
    let mut source = Ids {
        calls: 0,
        fail: true,
    };
    assert!(matches!(
        flow.prepare_callback_with_source(&input, &base, &token(), &mut source),
        Err(AslError::WorkLimit)
    ));
    assert_eq!(source.calls, 1);
    assert!(matches!(
        prepare(&json!({"Parameters":{"missing.$":"$.missing"}})),
        Err(AslError::Missing)
    ));
    assert!(base.value().get("Task").is_none());
    Ok(())
}
#[test]
fn test_should_enforce_outbound_limit_even_when_history_would_be_small() -> TestResult {
    let input = Payload::from_value(&json!({"body":"x".repeat(MAX_PAYLOAD_BYTES-20)}))?;
    let flow = DataFlow::compile(
        &json!({"Parameters":{"value.$":"States.Format('{}{}',$$.Task.Token,$.body)"}}),
    )?;
    assert!(matches!(
        flow.prepare_callback_with_source(&input, &context(), &token(), &mut Ids::default()),
        Err(AslError::DataLimit)
    ));
    Ok(())
}
#[test]
fn test_should_bound_history_redaction_expansion_independently_of_valid_outbound() -> TestResult {
    let overhead = serde_json::to_vec(&json!({"body":"","n":1}))?.len();
    for (spare, success) in [(0, false), (3, true)] {
        let body = "x".repeat(MAX_PAYLOAD_BYTES - overhead - spare);
        let flow = DataFlow::compile(
            &json!({"Parameters":{"body":body,"n.$":"States.ArrayLength(States.Array($$.Task.Token))"}}),
        )?;
        let result = flow.prepare_callback_with_source(
            &original(),
            &context(),
            &token(),
            &mut Ids::default(),
        );
        if success {
            let pair = result?;
            assert_eq!(
                pair.outbound().original_bytes().len(),
                MAX_PAYLOAD_BYTES - 3
            );
            assert_eq!(
                pair.history().payload().original_bytes().len(),
                MAX_PAYLOAD_BYTES
            );
            assert!(pair.history().payload().value()["n"].is_null());
        } else {
            assert!(matches!(result, Err(AslError::DataLimit)));
        }
    }
    Ok(())
}
#[test]
fn test_should_serialize_null_redacted_parameters_through_the_official_history_model() -> TestResult
{
    let pair = prepare(
        &json!({"Parameters":{"FunctionName":"Worker","Payload":{"token.$":"$$.Task.Token","n.$":"$.n"}}}),
    )?;
    let details = TaskScheduledEventDetails::builder()
        .parameters(String::from_utf8(
            pair.history().payload().original_bytes().to_vec(),
        )?)
        .region("us-east-1".into())
        .resource("invoke.waitForTaskToken".into())
        .resource_type("lambda".into())
        .build();
    let mut event = HistoryEvent::builder()
        .id(1)
        .r#type(HistoryEventType::TaskScheduled)
        .timestamp(Utc::now())
        .build();
    event.task_scheduled_event_details = Some(details);
    let encoded = serde_json::to_string(&event)?;
    assert!(!encoded.contains(&secret()));
    let projected: Value = serde_json::from_str(&encoded)?;
    let fields = projected["taskScheduledEventDetails"]
        .as_object()
        .ok_or("event details")?;
    assert_eq!(fields.len(), 4);
    assert!(!fields.contains_key("truncated"));
    assert!(!fields.contains_key("parametersDetails"));
    let params: Value = serde_json::from_str(fields["parameters"].as_str().ok_or("parameters")?)?;
    assert_eq!(
        params,
        json!({"FunctionName":"Worker","Payload":{"token":null,"n":21}})
    );
    let decoded: HistoryEvent = serde_json::from_str(&encoded)?;
    assert_eq!(decoded.r#type, HistoryEventType::TaskScheduled);
    Ok(())
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]
    #[test]
    fn test_should_never_emit_injected_token_from_successful_compiled_context_path_projection(
        segments in prop::collection::vec(0_u8..7,0..5), context_root in any::<bool>(), bytes in any::<[u8;32]>()
    ){
        let text=hex::encode(bytes);let credential:CallbackToken=text.parse().unwrap();
        let mut path=if context_root {"$$".to_owned()}else{"$".to_owned()};
        for segment in segments{path.push_str(match segment{0=>".Task",1=>"['Token']",2=>".Execution",3=>"[*]",4=>"[0]",5=>"[0:1]",_=>".Other"});}
        let flow=DataFlow::compile(&json!({"Parameters":{"value.$":path}})).unwrap();
        if let Ok(pair)=flow.prepare_callback_with_source(&original(),&context(),&credential,&mut Ids::default()){
            prop_assert!(!pair.history().payload().value().to_string().contains(&text));
            if pair.outbound().value().to_string().contains(&text){prop_assert!(pair.history().redacted());}
        }
    }
    #[test]
    fn test_should_preserve_arbitrary_untainted_business_fields_and_same_uuid_result(n in any::<i64>(), label in "[a-z]{0,32}"){
        let input=Payload::from_value(&json!({"n":n,"label":label})).unwrap();let mut source=Ids::default();
        let flow=DataFlow::compile(&json!({"Parameters":{"n.$":"$.n","label.$":"$.label","id.$":"States.UUID()","token.$":"$$.Task.Token"}})).unwrap();
        let pair=flow.prepare_callback_with_source(&input,&context(),&token(),&mut source).unwrap();
        prop_assert_eq!(source.calls,1);prop_assert_eq!(&pair.history().payload().value()["n"],&json!(n));prop_assert_eq!(&pair.history().payload().value()["label"],&input.value()["label"]);
        prop_assert_eq!(&pair.outbound().value()["id"],&pair.history().payload().value()["id"]);prop_assert!(pair.history().payload().value()["token"].is_null());
    }
}
