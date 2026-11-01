use std::collections::VecDeque;

use super::*;

#[rstest::rstest]
#[case("name")]
#[case("name_charset")]
#[case("arn")]
#[case("definition")]
#[case("execution_input")]
#[case("empty_input")]
#[case("output")]
#[case("token")]
#[case("next_token")]
#[case("tags")]
#[case("tag_key")]
#[case("tag_value")]
#[case("tag_keys")]
#[case("cause")]
#[case("error")]
#[case("logging")]
#[case("version")]
#[case("encryption_period")]
#[tokio::test]
async fn test_should_bound_operation_fields_before_handler_entry(#[case] problem: &str) {
    let (action, body, code) = match problem {
        "name" => (
            "StartExecution",
            json!({"stateMachineArn":"machine","name":"a".repeat(81)}),
            "InvalidName",
        ),
        "name_charset" => (
            "StartExecution",
            json!({"stateMachineArn":"machine","name":"bad name"}),
            "InvalidName",
        ),
        "arn" => (
            "DescribeExecution",
            json!({"executionArn":"a".repeat(257)}),
            "InvalidArn",
        ),
        "definition" => (
            "ValidateStateMachineDefinition",
            json!({"definition":"a".repeat(1024*1024+1)}),
            "ValidationException",
        ),
        "execution_input" => (
            "StartExecution",
            json!({"stateMachineArn":"machine","input":serde_json::to_string(&"a".repeat(262_144)).unwrap()}),
            "InvalidExecutionInput",
        ),
        "empty_input" => (
            "StartExecution",
            json!({"stateMachineArn":"machine","input":""}),
            "InvalidExecutionInput",
        ),
        "output" => (
            "SendTaskSuccess",
            json!({"taskToken":"token","output":"not-json"}),
            "InvalidOutput",
        ),
        "token" => (
            "SendTaskHeartbeat",
            json!({"taskToken":"é"}),
            "InvalidToken",
        ),
        "next_token" => (
            "ListStateMachines",
            json!({"nextToken":"a".repeat(2049)}),
            "InvalidToken",
        ),
        "tags" => (
            "TagResource",
            json!({"resourceArn":"machine","tags":vec![json!({"key":"a"});51]}),
            "TooManyTags",
        ),
        "tag_key" => (
            "TagResource",
            json!({"resourceArn":"machine","tags":[{"key":"a".repeat(129)}]}),
            "ValidationException",
        ),
        "tag_value" => (
            "TagResource",
            json!({"resourceArn":"machine","tags":[{"key":"a","value":"a".repeat(257)}]}),
            "ValidationException",
        ),
        "tag_keys" => (
            "UntagResource",
            json!({"resourceArn":"machine","tagKeys":vec!["key";51]}),
            "TooManyTags",
        ),
        "cause" => (
            "StopExecution",
            json!({"executionArn":"execution","cause":"a".repeat(32_769)}),
            "ValidationException",
        ),
        "error" => (
            "SendTaskFailure",
            json!({"taskToken":"token","error":"a".repeat(257)}),
            "ValidationException",
        ),
        "logging" => (
            "CreateStateMachine",
            json!({"name":"machine","roleArn":"role","definition":"{}","loggingConfiguration":{"destinations":[{},{}]}}),
            "ValidationException",
        ),
        "version" => (
            "UpdateStateMachine",
            json!({"stateMachineArn":"machine","versionDescription":"a".repeat(257)}),
            "ValidationException",
        ),
        "encryption_period" => (
            "CreateStateMachine",
            json!({"name":"machine","roleArn":"role","definition":"{}","encryptionConfiguration":{"type":"AWS_OWNED_KEY","kmsDataKeyReusePeriodSeconds":59}}),
            "ValidationException",
        ),
        _ => panic!("fixture case"),
    };
    let (service, handler, _accepted) = fixture(Mode::Normal, false);
    let response = service
        .handle(request(action, serde_json::to_vec(&body).unwrap()))
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(data(response).await["__type"], code);
    assert_eq!(handler.count.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn test_should_accept_exact_byte_bounds_unicode_business_data_and_trace_ascii() {
    let (service, handler, mut accepted) = fixture(Mode::Normal, false);
    for trace in [
        String::new(),
        "Root=value; Parent=value".to_owned(),
        "t".repeat(256),
    ] {
        let body = json!({"stateMachineArn":"a".repeat(256),"name":"a".repeat(80),"input":"null","traceHeader":trace});
        assert_eq!(
            service
                .handle(request(
                    "StartExecution",
                    serde_json::to_vec(&body).unwrap()
                ))
                .await
                .status(),
            StatusCode::OK
        );
        let (_, request) = accepted.recv().await.unwrap();
        let Request::StartExecution(value) = request else {
            panic!("start fixture")
        };
        assert_eq!(value.trace_header, Some(trace));
    }
    let exact = serde_json::to_string(&"é".repeat(131_071)).unwrap();
    assert_eq!(exact.len(), 262_144);
    let body = json!({"stateMachineArn":"machine","input":exact});
    assert_eq!(
        service
            .handle(request(
                "StartExecution",
                serde_json::to_vec(&body).unwrap()
            ))
            .await
            .status(),
        StatusCode::OK
    );
    accepted.recv().await.unwrap();
    let body = json!({"definition":"a".repeat(1024*1024)});
    assert_eq!(
        service
            .handle(request(
                "ValidateStateMachineDefinition",
                serde_json::to_vec(&body).unwrap()
            ))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(handler.count.load(Ordering::SeqCst), 5);
}
#[rstest::rstest]
#[case("é".to_owned())]
#[case("a".repeat(257))]
#[tokio::test]
async fn test_should_reject_non_ascii_or_oversized_trace_header(#[case] trace: String) {
    let (service, handler, _accepted) = fixture(Mode::Normal, false);
    let body = json!({"stateMachineArn":"machine","traceHeader":trace});
    assert_eq!(
        service
            .handle(request(
                "StartExecution",
                serde_json::to_vec(&body).unwrap()
            ))
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(handler.count.load(Ordering::SeqCst), 0);
}
#[rstest::rstest]
#[case("+2")]
#[case("-0")]
#[case("")]
#[tokio::test]
async fn test_should_reject_non_decimal_content_length(#[case] length: &str) {
    let (service, handler, _accepted) = fixture(Mode::Normal, false);
    let mut request = request("ListStateMachines", b"{}".as_slice());
    request
        .headers_mut()
        .insert("content-length", HeaderValue::from_str(length).unwrap());
    assert_eq!(
        service.handle(request).await.status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(handler.count.load(Ordering::SeqCst), 0);
}
struct Frames {
    frames: VecDeque<Frame<Bytes>>,
}
impl Body for Frames {
    type Data = Bytes;
    type Error = Infallible;
    fn poll_frame(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        Poll::Ready(self.get_mut().frames.pop_front().map(Ok))
    }
}
#[rstest::rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn test_should_reject_all_trailers_before_handler_even_when_metadata_is_oversized(
    #[case] oversized: bool,
) {
    let (service, handler, _accepted) = fixture(Mode::Normal, false);
    let mut trailers = http::HeaderMap::new();
    trailers.insert(
        "x-trailer-secret",
        HeaderValue::from_str(&"s".repeat(if oversized { 9000 } else { 1 })).unwrap(),
    );
    let (parts, _) = request("ListStateMachines", Bytes::new()).into_parts();
    let body = Frames {
        frames: VecDeque::from([
            Frame::data(Bytes::from_static(b"{}")),
            Frame::trailers(trailers),
        ]),
    };
    let response = service.handle(http::Request::from_parts(parts, body)).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        data(response).await["__type"],
        "MalformedHttpRequestException"
    );
    assert_eq!(handler.count.load(Ordering::SeqCst), 0);
}
