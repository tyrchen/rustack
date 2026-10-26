use std::{
    convert::Infallible,
    future::pending,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};

use async_trait::async_trait;
use bytes::Bytes;
use http::{HeaderValue, StatusCode};
use http_body::{Body, Frame};
use http_body_util::{BodyExt, Full};
use rustack_auth::{
    StaticCredentialProvider,
    canonical::build_canonical_request,
    hash_payload,
    sigv4::{build_string_to_sign, compute_signature, derive_signing_key},
};
use rustack_core::http::BodyBudget;
use rustack_stepfunctions_model::{
    StepFunctionsError, StepFunctionsErrorCode, StepFunctionsOperation as Op, output,
};
use serde_json::{Value, json};
use tokio::{sync::mpsc, time::Instant};

use crate::*;

#[derive(Debug, Clone, Copy)]
enum Mode {
    Normal,
    Mismatch,
    Error,
    Oversize,
    Pending,
    OversizeError,
    SuccessError,
}
struct Handler {
    count: AtomicUsize,
    sender: mpsc::Sender<(RequestContext, Request)>,
    mode: Mode,
}
#[async_trait]
impl StepFunctionsHandler for Handler {
    async fn handle(
        &self,
        context: RequestContext,
        request: Request,
    ) -> Result<Response, StepFunctionsError> {
        self.count.fetch_add(1, Ordering::SeqCst);
        let op = request.operation();
        self.sender.try_send((context, request)).unwrap();
        match self.mode {
            Mode::Pending => pending().await,
            Mode::Error => Err(StepFunctionsError::with_message(
                StepFunctionsErrorCode::InvalidName,
                "domain rejected",
            )),
            Mode::OversizeError => Err(StepFunctionsError::with_message(
                StepFunctionsErrorCode::InvalidName,
                "private".repeat(1024),
            )),
            Mode::SuccessError => {
                let mut error = StepFunctionsError::new(StepFunctionsErrorCode::InvalidName);
                error.status_code = StatusCode::OK;
                Err(error)
            }
            Mode::Mismatch => Ok(output::DeleteStateMachineOutput::default().into()),
            Mode::Oversize => {
                let mut output = output::StartExecutionOutput::default();
                output.execution_arn = "\"".repeat(8 * 1024 * 1024);
                Ok(output.into())
            }
            Mode::Normal => Ok(reply(op)),
        }
    }
}
fn reply(op: Op) -> Response {
    match op {
        Op::CreateStateMachine => output::CreateStateMachineOutput::default().into(),
        Op::UpdateStateMachine => output::UpdateStateMachineOutput::default().into(),
        Op::DeleteStateMachine => output::DeleteStateMachineOutput::default().into(),
        Op::DescribeStateMachine => output::DescribeStateMachineOutput::default().into(),
        Op::ListStateMachines => output::ListStateMachinesOutput::default().into(),
        Op::ValidateStateMachineDefinition => {
            output::ValidateStateMachineDefinitionOutput::default().into()
        }
        Op::StartExecution => {
            let mut value = output::StartExecutionOutput::default();
            value.start_date = chrono::DateTime::from_timestamp_millis(-1250).unwrap();
            value.execution_arn =
                "arn:aws:states:us-east-1:000000000000:execution:test:wire".into();
            value.into()
        }
        Op::DescribeExecution => output::DescribeExecutionOutput::default().into(),
        Op::ListExecutions => output::ListExecutionsOutput::default().into(),
        Op::StopExecution => output::StopExecutionOutput::default().into(),
        Op::GetExecutionHistory => output::GetExecutionHistoryOutput::default().into(),
        Op::DescribeStateMachineForExecution => {
            output::DescribeStateMachineForExecutionOutput::default().into()
        }
        Op::TagResource => output::TagResourceOutput::default().into(),
        Op::UntagResource => output::UntagResourceOutput::default().into(),
        Op::ListTagsForResource => output::ListTagsForResourceOutput::default().into(),
        Op::SendTaskSuccess => output::SendTaskSuccessOutput::default().into(),
        Op::SendTaskFailure => output::SendTaskFailureOutput::default().into(),
        Op::SendTaskHeartbeat => output::SendTaskHeartbeatOutput::default().into(),
        _ => panic!("unsupported fixture action"),
    }
}
fn scope() -> RequestScope {
    RequestScope::new("aws".into(), "us-east-1".into(), "000000000000".into()).unwrap()
}
fn fixture(
    mode: Mode,
    strict: bool,
) -> (
    StepFunctionsHttpService,
    Arc<Handler>,
    mpsc::Receiver<(RequestContext, Request)>,
) {
    let (sender, receiver) = mpsc::channel(64);
    let handler = Arc::new(Handler {
        count: AtomicUsize::new(0),
        sender,
        mode,
    });
    let credentials = Arc::new(StaticCredentialProvider::new(vec![(
        "local-key".into(),
        "local-secret".into(),
    )]));
    let config = StepFunctionsHttpConfig::new(scope(), !strict, Some(credentials)).unwrap();
    (
        StepFunctionsHttpService::new(handler.clone(), config),
        handler,
        receiver,
    )
}
fn request(action: &str, body: impl Into<Bytes>) -> http::Request<Full<Bytes>> {
    http::Request::builder()
        .method("POST")
        .uri("/")
        .header("content-type", CONTENT_TYPE)
        .header("x-amz-target", format!("AWSStepFunctions.{action}"))
        .body(Full::new(body.into()))
        .unwrap()
}
async fn data(response: http::Response<StepFunctionsBody>) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
fn cases() -> Vec<(Op, Value)> {
    vec![
        (
            Op::CreateStateMachine,
            json!({"name":"machine","roleArn":"role","definition":"{}"}),
        ),
        (
            Op::UpdateStateMachine,
            json!({"stateMachineArn":"machine","definition":"{}"}),
        ),
        (Op::DeleteStateMachine, json!({"stateMachineArn":"machine"})),
        (
            Op::DescribeStateMachine,
            json!({"stateMachineArn":"machine"}),
        ),
        (Op::ListStateMachines, json!({})),
        (
            Op::ValidateStateMachineDefinition,
            json!({"definition":"not-asl"}),
        ),
        (
            Op::StartExecution,
            json!({"stateMachineArn":"machine","input":"{ \"secret\": 7 }"}),
        ),
        (Op::DescribeExecution, json!({"executionArn":"execution"})),
        (Op::ListExecutions, json!({"stateMachineArn":"machine"})),
        (Op::StopExecution, json!({"executionArn":"execution"})),
        (Op::GetExecutionHistory, json!({"executionArn":"execution"})),
        (
            Op::DescribeStateMachineForExecution,
            json!({"executionArn":"execution"}),
        ),
        (
            Op::TagResource,
            json!({"resourceArn":"machine","tags":[{"key":"a","value":"b"}]}),
        ),
        (
            Op::UntagResource,
            json!({"resourceArn":"machine","tagKeys":["a"]}),
        ),
        (Op::ListTagsForResource, json!({"resourceArn":"machine"})),
        (
            Op::SendTaskSuccess,
            json!({"taskToken":"secret-token","output":"null"}),
        ),
        (
            Op::SendTaskFailure,
            json!({"taskToken":"secret-token","error":"error","cause":"cause"}),
        ),
        (Op::SendTaskHeartbeat, json!({"taskToken":"secret-token"})),
    ]
}
#[tokio::test]
async fn test_should_dispatch_all_typed_actions_without_enum_wrappers_or_data_in_debug() {
    let (service, handler, mut accepted) = fixture(Mode::Normal, false);
    for (op, body) in cases() {
        let response = service
            .handle(request(op.as_str(), serde_json::to_vec(&body).unwrap()))
            .await;
        assert_eq!(response.status(), StatusCode::OK, "{op:?}");
        assert_eq!(response.headers()["content-type"], CONTENT_TYPE);
        assert!(!response.headers().contains_key("x-amzn-errortype"));
        let id = response.headers()["x-amzn-requestid"]
            .to_str()
            .unwrap()
            .parse::<uuid::Uuid>()
            .unwrap();
        let decoded = data(response).await;
        if op == Op::StartExecution {
            assert_eq!(decoded["startDate"], json!(-1.25));
            assert!(!decoded.as_object().unwrap().contains_key("StartExecution"));
        }
        let (context, request) = accepted.recv().await.unwrap();
        assert_eq!(context.request_id(), id);
        assert_eq!(context.scope(), &scope());
        assert!(!context.authenticated());
        assert_eq!(request.operation(), op);
        assert!(!format!("{request:?}").contains("secret"));
        if let Request::StartExecution(value) = request {
            assert_eq!(value.input.as_deref(), Some("{ \"secret\": 7 }"));
        }
    }
    assert_eq!(handler.count.load(Ordering::SeqCst), 18);
}
#[tokio::test]
async fn test_should_accept_empty_body_as_empty_json_and_reject_unknown_or_unsupported_actions() {
    let (service, handler, _accepted) = fixture(Mode::Normal, false);
    assert_eq!(
        service
            .handle(request("ListStateMachines", Bytes::new()))
            .await
            .status(),
        StatusCode::OK
    );
    let unknown = service.handle(request("MadeUp", b"{}".as_slice())).await;
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    assert_eq!(data(unknown).await["__type"], "UnknownOperationException");
    let unsupported = service
        .handle(request("StartSyncExecution", b"not-json".as_slice()))
        .await;
    assert_eq!(unsupported.status(), StatusCode::BAD_REQUEST);
    assert_eq!(data(unsupported).await["__type"], "UnsupportedOperation");
    assert_eq!(handler.count.load(Ordering::SeqCst), 1);
}
#[rstest::rstest]
#[case("method")]
#[case("path")]
#[case("query")]
#[case("content_type")]
#[case("duplicate_type")]
#[case("duplicate_target")]
#[case("duplicate_auth")]
#[case("compression")]
#[case("length_mismatch")]
#[case("length_and_transfer")]
#[case("header_count")]
#[case("header_value")]
#[tokio::test]
async fn test_should_reject_ambiguous_envelopes_before_handler(#[case] problem: &str) {
    let (service, handler, _accepted) = fixture(Mode::Normal, false);
    let mut request = request("ListStateMachines", b"{}".as_slice());
    match problem {
        "method" => *request.method_mut() = http::Method::GET,
        "path" => *request.uri_mut() = "/other".parse().unwrap(),
        "query" => *request.uri_mut() = "/?ignored=1".parse().unwrap(),
        "content_type" => {
            request.headers_mut().insert(
                "content-type",
                HeaderValue::from_static("application/x-amz-json-1.1"),
            );
        }
        "duplicate_type" => {
            request
                .headers_mut()
                .append("content-type", HeaderValue::from_static(CONTENT_TYPE));
        }
        "duplicate_target" => {
            request.headers_mut().append(
                "x-amz-target",
                HeaderValue::from_static("AWSStepFunctions.DeleteStateMachine"),
            );
        }
        "duplicate_auth" => {
            request
                .headers_mut()
                .append("authorization", HeaderValue::from_static("one"));
            request
                .headers_mut()
                .append("authorization", HeaderValue::from_static("two"));
        }
        "compression" => {
            request
                .headers_mut()
                .insert("content-encoding", HeaderValue::from_static("gzip"));
        }
        "length_mismatch" => {
            request
                .headers_mut()
                .insert("content-length", HeaderValue::from_static("0"));
        }
        "length_and_transfer" => {
            request
                .headers_mut()
                .insert("content-length", HeaderValue::from_static("2"));
            request
                .headers_mut()
                .insert("transfer-encoding", HeaderValue::from_static("chunked"));
        }
        "header_count" => {
            for index in 0..65 {
                request.headers_mut().insert(
                    format!("x-extra-{index}")
                        .parse::<http::HeaderName>()
                        .unwrap(),
                    HeaderValue::from_static("x"),
                );
            }
        }
        "header_value" => {
            request
                .headers_mut()
                .insert("x-extra", HeaderValue::from_str(&"a".repeat(8193)).unwrap());
        }
        _ => panic!("fixture case"),
    }
    assert!(claims_request(request.headers()));
    assert_eq!(
        service.handle(request).await.status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(handler.count.load(Ordering::SeqCst), 0);
}
#[test]
fn test_should_claim_any_matching_occurrence_but_not_other_services() {
    let mut headers = http::HeaderMap::new();
    headers.append(
        "x-amz-target",
        HeaderValue::from_static("AWSEvents.PutEvents"),
    );
    assert!(!claims_request(&headers));
    headers.append(
        "x-amz-target",
        HeaderValue::from_static("AWSStepFunctions.Unknown"),
    );
    assert!(claims_request(&headers));
    assert!(resolve_operation(&headers).is_err());
    headers.clear();
    headers.insert("x-amz-target", HeaderValue::from_static("AWSStepFunctions"));
    assert!(claims_request(&headers));
}
#[rstest::rstest]
#[case(b"{\"maxResults\":1,\"maxResults\":2}".as_slice())]
#[case(b"{\"unknown\":1}".as_slice())]
#[case(b"[]".as_slice())]
#[case(b"{\"maxResults\":-1}".as_slice())]
#[tokio::test]
async fn test_should_reject_bad_shape_duplicates_unknown_fields_and_ranges(#[case] body: &[u8]) {
    let (service, handler, _accepted) = fixture(Mode::Normal, false);
    assert_eq!(
        service
            .handle(request("ListStateMachines", Bytes::copy_from_slice(body)))
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(handler.count.load(Ordering::SeqCst), 0);
}
#[rstest::rstest]
#[case(Mode::Mismatch)]
#[case(Mode::Oversize)]
#[case(Mode::Error)]
#[case(Mode::OversizeError)]
#[case(Mode::SuccessError)]
#[tokio::test]
async fn test_should_bound_and_correlate_replies_and_preserve_domain_errors(#[case] mode: Mode) {
    let (service, handler, _accepted) = fixture(mode, false);
    let response = service
        .handle(request(
            "StartExecution",
            b"{\"stateMachineArn\":\"machine\"}".as_slice(),
        ))
        .await;
    let code = if matches!(mode, Mode::Error) {
        "InvalidName"
    } else {
        "InternalFailure"
    };
    assert_eq!(response.headers()["x-amzn-errortype"], code);
    assert_eq!(
        response.status(),
        if matches!(mode, Mode::Error) {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    );
    assert_eq!(data(response).await["__type"], code);
    assert_eq!(handler.count.load(Ordering::SeqCst), 1);
}
struct NeverBody {
    polled: Arc<AtomicUsize>,
}
impl Body for NeverBody {
    type Data = Bytes;
    type Error = Infallible;
    fn poll_frame(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        self.polled.fetch_add(1, Ordering::SeqCst);
        Poll::Pending
    }
}
#[tokio::test(start_paused = true)]
async fn test_should_bound_slow_body_and_never_poll_oversized_declared_input() {
    let (service, handler, _accepted) = fixture(Mode::Normal, false);
    let polled = Arc::new(AtomicUsize::new(0));
    let (mut parts, _) = request("ListStateMachines", Bytes::new()).into_parts();
    parts
        .headers
        .insert("content-length", HeaderValue::from(3 * 1024 * 1024));
    let response = service
        .handle(http::Request::from_parts(
            parts,
            NeverBody {
                polled: polled.clone(),
            },
        ))
        .await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(polled.load(Ordering::SeqCst), 0);
    let (parts, _) = request("ListStateMachines", Bytes::new()).into_parts();
    let response = service
        .handle(http::Request::from_parts(parts, NeverBody { polled }))
        .await;
    assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
    assert_eq!(handler.count.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn test_should_cap_actual_body_bytes_without_trusting_length() {
    let (service, handler, _accepted) = fixture(Mode::Normal, false);
    let response = service
        .handle(request(
            "ListStateMachines",
            Bytes::from(vec![b'a'; 2 * 1024 * 1024 + 1]),
        ))
        .await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(handler.count.load(Ordering::SeqCst), 0);
}
#[tokio::test(start_paused = true)]
async fn test_should_timeout_only_the_waiter_after_a_typed_command_was_accepted() {
    let (_original, handler, mut accepted) = fixture(Mode::Pending, false);
    let config = StepFunctionsHttpConfig::new(scope(), true, None)
        .unwrap()
        .with_budgets(
            BodyBudget::new(
                2 * 1024 * 1024,
                Duration::from_secs(5),
                Duration::from_secs(2),
            )
            .unwrap(),
            Duration::from_secs(1),
        )
        .unwrap();
    let service = StepFunctionsHttpService::new(handler.clone(), config);
    let before = Instant::now();
    let response = service
        .handle(request(
            "StartExecution",
            b"{\"stateMachineArn\":\"machine\"}".as_slice(),
        ))
        .await;
    assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
    assert!(Instant::now() >= before + Duration::from_secs(1));
    let (context, request) = accepted.recv().await.unwrap();
    assert_eq!(request.operation(), Op::StartExecution);
    assert!(context.deadline() <= Instant::now());
    assert_eq!(handler.count.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn test_should_leave_accepted_command_owned_when_http_caller_disappears() {
    let (service, _handler, mut accepted) = fixture(Mode::Pending, false);
    let task = tokio::spawn(async move {
        service
            .handle(request(
                "StartExecution",
                b"{\"stateMachineArn\":\"machine\"}".as_slice(),
            ))
            .await
    });
    let (context, request) = accepted.recv().await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(request.operation(), Op::StartExecution);
    assert!(!context.request_id().is_nil());
}
fn signed(
    body: &[u8],
    region: &str,
    service: &str,
    bind_target: bool,
) -> http::Request<Full<Bytes>> {
    let mut request = request("ListStateMachines", Bytes::copy_from_slice(body));
    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let date = &timestamp[..8];
    request
        .headers_mut()
        .insert("host", HeaderValue::from_static("localhost"));
    request
        .headers_mut()
        .insert("x-amz-date", HeaderValue::from_str(&timestamp).unwrap());
    let mut names = vec!["content-type", "host", "x-amz-date"];
    if bind_target {
        names.push("x-amz-target");
    }
    let headers: Vec<_> = names
        .iter()
        .map(|name| (*name, request.headers()[*name].to_str().unwrap()))
        .collect();
    let canonical = build_canonical_request("POST", "/", "", &headers, &names, &hash_payload(body));
    let scope = format!("{date}/{region}/{service}/aws4_request");
    let string = build_string_to_sign(&timestamp, &scope, &hash_payload(canonical.as_bytes()));
    let key = derive_signing_key("local-secret", date, region, service).unwrap();
    let signature = compute_signature(&key, &string).unwrap();
    request.headers_mut().insert(
        "authorization",
        HeaderValue::from_str(&format!(
            "AWS4-HMAC-SHA256 Credential=local-key/{scope}, SignedHeaders={}, \
             Signature={signature}",
            names.join(";")
        ))
        .unwrap(),
    );
    request
}
#[tokio::test]
async fn test_should_verify_original_payload_and_bind_service_region_and_action() {
    let (service, handler, mut accepted) = fixture(Mode::Normal, true);
    assert_eq!(
        service
            .handle(signed(b"{}", "us-east-1", "states", true))
            .await
            .status(),
        StatusCode::OK
    );
    assert!(accepted.recv().await.unwrap().0.authenticated());
    for (region, name, target) in [
        ("us-west-2", "states", true),
        ("us-east-1", "lambda", true),
        ("us-east-1", "states", false),
    ] {
        let response = service.handle(signed(b"{}", region, name, target)).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(data(response).await["__type"], "AccessDeniedException");
    }
    let (mut parts, _) = signed(b"{}", "us-east-1", "states", true).into_parts();
    parts.headers.remove("x-amz-content-sha256");
    let response = service
        .handle(http::Request::from_parts(
            parts,
            Full::new(Bytes::from_static(b"{\"maxResults\":1}")),
        ))
        .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(handler.count.load(Ordering::SeqCst), 1);
}
#[test]
fn test_should_fail_closed_config_and_redact_scope_and_auth_sources() {
    assert!(StepFunctionsHttpConfig::new(scope(), false, None).is_err());
    assert!(RequestScope::new("other".into(), "us-east-1".into(), "000000000000".into()).is_err());
    assert!(RequestScope::new("aws".into(), "-".into(), "000000000000".into()).is_err());
    assert!(RequestScope::new("aws".into(), "us-east-1".into(), "0".into()).is_err());
    let error = HttpError::Authentication(rustack_auth::AuthError::AccessKeyNotFound(
        "secret-access-key".into(),
    ));
    assert!(!format!("{error:?} {error}").contains("secret"));
    assert!(!format!("{:?}", scope()).contains("000000000000"));
}

mod bounds;
