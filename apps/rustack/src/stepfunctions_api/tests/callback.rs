//! Real public callback requests over a registered observed ASL port.

use rustack_stepfunctions_core::{
    AslError, CallbackCommand, CallbackError, CallbackInvocationMetadata, CallbackLimits,
    CallbackRegistry, CallbackSupervisor, CallbackToken, ExecutionOutcome, LambdaTaskCommand,
    LambdaTaskPort, Payload, RegistryCommand, RegistryResult, StateSupport, TaskCall, TaskFailure,
    TaskResponder,
};
use rustack_stepfunctions_model::StepFunctionsOperation as Op;
use tokio::time::{sleep, sleep_until, timeout_at};
use uuid::Uuid;

use super::*;

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}
#[derive(Debug)]
struct Port(mpsc::Sender<(LambdaTaskCommand, TaskResponder)>);
impl LambdaTaskPort for Port {
    fn start(&self, command: LambdaTaskCommand) -> Result<TaskCall, TaskFailure> {
        let (call, responder) = TaskCall::channel();
        self.0
            .try_send((command, responder))
            .map_err(|_| TaskFailure::service_exception())?;
        Ok(call)
    }
}
struct CallbackFixture {
    inner: Fixture,
    registry: Registry,
    callbacks: CallbackSupervisor,
    calls: mpsc::Receiver<(LambdaTaskCommand, TaskResponder)>,
}
impl CallbackFixture {
    fn new() -> Self {
        Self::scoped(TenantScope::default())
    }
    fn scoped(scope: TenantScope) -> Self {
        let (callbacks, callback_owner) =
            CallbackRegistry::new(scope.clone(), CallbackLimits::default()).unwrap();
        let (sender, calls) = mpsc::channel(8);
        let mut runtime = ExecutionSupervisor::new(
            RuntimeLimits::new(8, 1, 2).unwrap(),
            HistoryLimits::default(),
            256 * 1024 * 1024,
            Some(Arc::new(Port(sender))),
        )
        .unwrap();
        runtime.install_callbacks(callbacks).unwrap();
        let (registry, owner) = Registry::new_with_state_support(
            scope,
            RegistryLimits::default(),
            runtime,
            ExecutionLimits::default(),
            StateSupport::Structured,
        )
        .unwrap();
        let inner = Fixture::from_registry(
            registry.clone(),
            owner,
            TaskSupport::Lambda,
            ApiLimits::default(),
        );
        Self {
            inner,
            registry,
            callbacks: callback_owner,
            calls,
        }
    }
    async fn start(&self, extra: Value) -> String {
        let mut task = json!({"Type":"Task","Resource":"arn:aws:states:::lambda:invoke.waitForTaskToken","TimeoutSeconds":60,"Parameters":{"FunctionName":"Worker","Payload":{"token.$":"$$.Task.Token","retry.$":"$$.State.RetryCount"}},"End":true});
        task.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let arn = self
            .inner
            .create("Callback", json!({"StartAt":"Call","States":{"Call":task}}))
            .await;
        let (status, response) = self
            .inner
            .call("StartExecution", json!({"stateMachineArn":arn}))
            .await;
        assert_eq!(status, StatusCode::OK);
        response["executionArn"].as_str().unwrap().into()
    }
    async fn next(&mut self) -> (LambdaTaskCommand, TaskResponder) {
        timeout_at(deadline(), self.calls.recv())
            .await
            .unwrap()
            .unwrap()
    }
    async fn pending(&self, execution: &str) {
        assert_eq!(
            self.inner
                .call("DescribeExecution", json!({"executionArn":execution}))
                .await
                .1["status"],
            "RUNNING"
        );
    }
    async fn history(&self, execution: &str) -> Value {
        self.inner
            .call("GetExecutionHistory", json!({"executionArn":execution}))
            .await
            .1
    }
    async fn submitted(&self, execution: &str) {
        timeout_at(deadline(), async {
            loop {
                if self.history(execution).await["events"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v["type"] == "TaskSubmitted")
                {
                    break;
                }
                sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
    }
    async fn close(&mut self) {
        self.inner.close().await;
        self.callbacks.shutdown(deadline()).await.unwrap();
    }
}
fn token(command: &LambdaTaskCommand) -> String {
    command.dispatch().parameters().value()["Payload"]["token"]
        .as_str()
        .unwrap()
        .into()
}
fn report(
    command: &mut LambdaTaskCommand,
    responder: &mut TaskResponder,
    result: Result<(), TaskFailure>,
) {
    responder.started().unwrap();
    command
        .take_callback_reporter()
        .unwrap()
        .unwrap()
        .complete_at(
            result.map(|()| CallbackInvocationMetadata::new(Uuid::new_v4(), "$LATEST").unwrap()),
            Instant::now(),
        )
        .unwrap();
}
fn finish(mut command: LambdaTaskCommand, mut responder: TaskResponder) {
    report(&mut command, &mut responder, Ok(()));
    responder.finish(Ok(Payload::parse(b"null").unwrap()));
}
async fn success(f: &CallbackFixture, token: &str, output: Value) {
    let (status, body) = f
        .inner
        .call(
            "SendTaskSuccess",
            json!({"taskToken":token,"output":output.to_string()}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, Value::Null);
}
#[rstest::rstest]
#[case(true, json!({"answer":42}))]
#[case(false, json!([1,"value",null]))]
#[case(false, json!(true))]
#[case(false, Value::Null)]
#[tokio::test]
async fn test_should_route_public_success_before_or_after_invocation_without_double_encoding(
    #[case] early: bool,
    #[case] output: Value,
) {
    let mut f = CallbackFixture::new();
    let execution = f.start(json!({})).await;
    let (command, responder) = f.next().await;
    let token = token(&command);
    if early {
        success(&f, &token, output.clone()).await;
        f.pending(&execution).await;
    }
    finish(command, responder);
    if !early {
        f.submitted(&execution).await;
        f.pending(&execution).await;
        success(&f, &token, output.clone()).await;
    }
    let done = f.inner.wait(&execution).await;
    assert_eq!(done["status"], "SUCCEEDED");
    assert_eq!(
        serde_json::from_str::<Value>(done["output"].as_str().unwrap()).unwrap(),
        output
    );
    assert!(!f.history(&execution).await.to_string().contains(&token));
    for action in ["SendTaskSuccess", "SendTaskFailure", "SendTaskHeartbeat"] {
        let mut input = json!({"taskToken":token});
        if action == "SendTaskSuccess" {
            input["output"] = json!("null");
        }
        let (status, body) = f.inner.call(action, input).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["__type"], "TaskTimedOut");
        assert!(!body.to_string().contains(&token));
    }
    f.close().await;
}
#[tokio::test]
async fn test_should_preserve_real_invocation_failure_after_public_early_success() {
    let mut f = CallbackFixture::new();
    let execution = f.start(json!({})).await;
    let (mut command, mut responder) = f.next().await;
    success(&f, &token(&command), json!({"falseSuccess":true})).await;
    report(
        &mut command,
        &mut responder,
        Err(TaskFailure::new("HandlerFailure", Some("actual failure")).unwrap()),
    );
    responder.finish(Err(TaskFailure::service_exception()));
    let done = f.inner.wait(&execution).await;
    assert_eq!(done["status"], "FAILED");
    assert_eq!(done["error"], "HandlerFailure");
    assert!(done.get("output").is_none());
    f.close().await;
}
#[rstest::rstest]
#[case(None, None)]
#[case(Some(""), Some(""))]
#[case(Some("ExternalFailure"), Some("quoted \"cause\" 中文"))]
#[tokio::test]
async fn test_should_route_public_failure_and_preserve_omitted_or_empty_fields(
    #[case] error: Option<&str>,
    #[case] cause: Option<&str>,
) {
    let mut f = CallbackFixture::new();
    let execution = f.start(json!({})).await;
    let (command, responder) = f.next().await;
    let token = token(&command);
    finish(command, responder);
    f.submitted(&execution).await;
    let mut input = json!({"taskToken":token});
    if let Some(error) = error {
        input["error"] = json!(error);
    }
    if let Some(cause) = cause {
        input["cause"] = json!(cause);
    }
    assert_eq!(
        f.inner.call("SendTaskFailure", input).await,
        (StatusCode::OK, Value::Null)
    );
    let done = f.inner.wait(&execution).await;
    assert_eq!(done["status"], "FAILED");
    assert_eq!(done.get("error"), error.map(|_| &done["error"]));
    assert_eq!(done["error"], json!(error));
    assert_eq!(done["cause"], json!(cause));
    assert!(!f.history(&execution).await.to_string().contains(&token));
    f.close().await;
}
#[tokio::test]
async fn test_should_route_public_heartbeat_without_history_mutation_then_complete() {
    let mut f = CallbackFixture::new();
    let execution = f.start(json!({"HeartbeatSeconds":30})).await;
    let (command, responder) = f.next().await;
    let token = token(&command);
    finish(command, responder);
    f.submitted(&execution).await;
    let before = f.history(&execution).await;
    for _ in 0..3 {
        assert_eq!(
            f.inner
                .call("SendTaskHeartbeat", json!({"taskToken":token}))
                .await,
            (StatusCode::OK, Value::Null)
        );
    }
    assert_eq!(f.history(&execution).await, before);
    f.pending(&execution).await;
    success(&f, &token, json!(42)).await;
    assert_eq!(f.inner.wait(&execution).await["status"], "SUCCEEDED");
    f.close().await;
}
#[tokio::test]
async fn test_should_route_public_retry_with_new_token_and_reject_old_attempt() {
    let mut f = CallbackFixture::new();
    let execution=f.start(json!({"Retry":[{"ErrorEquals":["ExternalFailure"],"IntervalSeconds":1,"MaxAttempts":1}]})).await;
    let (command, responder) = f.next().await;
    let old = token(&command);
    finish(command, responder);
    assert_eq!(
        f.inner
            .call(
                "SendTaskFailure",
                json!({"taskToken":old,"error":"ExternalFailure"})
            )
            .await
            .0,
        StatusCode::OK
    );
    let (command, responder) = f.next().await;
    let next = token(&command);
    assert_ne!(old, next);
    assert_eq!(
        command.dispatch().parameters().value()["Payload"]["retry"],
        1
    );
    assert_eq!(
        f.inner
            .call("SendTaskHeartbeat", json!({"taskToken":old}))
            .await
            .1["__type"],
        "TaskTimedOut"
    );
    success(&f, &next, json!({"retried":true})).await;
    finish(command, responder);
    assert_eq!(f.inner.wait(&execution).await["status"], "SUCCEEDED");
    f.close().await;
}
#[rstest::rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn test_should_reject_closed_public_token_after_absolute_or_heartbeat_timeout(
    #[case] heartbeat: bool,
) {
    let mut f = CallbackFixture::new();
    let extra = if heartbeat {
        json!({"HeartbeatSeconds":1})
    } else {
        json!({"TimeoutSeconds":1})
    };
    let execution = f.start(extra).await;
    let (command, responder) = f.next().await;
    let token = token(&command);
    finish(command, responder);
    let done = f.inner.wait(&execution).await;
    assert_eq!(
        done["error"],
        if heartbeat {
            "States.HeartbeatTimeout"
        } else {
            "States.Timeout"
        }
    );
    assert_eq!(
        f.inner
            .call("SendTaskHeartbeat", json!({"taskToken":token}))
            .await
            .1["__type"],
        "TaskTimedOut"
    );
    assert!(
        f.history(&execution).await["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["type"] == "TaskTimedOut")
    );
    f.close().await;
}
#[tokio::test]
async fn test_should_revoke_public_token_on_stop_and_join_physical_call() {
    let mut f = CallbackFixture::new();
    let execution = f.start(json!({})).await;
    let (command, responder) = f.next().await;
    let token = token(&command);
    let (status, _) = f
        .inner
        .call("StopExecution", json!({"executionArn":execution}))
        .await;
    assert_eq!(status, StatusCode::OK);
    let mut cancelled = responder.cancellation();
    timeout_at(deadline(), async {
        while !*cancelled.borrow_and_update() {
            cancelled.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    responder.finish(Err(TaskFailure::service_exception()));
    drop(command);
    assert_eq!(f.inner.wait(&execution).await["status"], "ABORTED");
    assert_eq!(
        f.inner
            .call("SendTaskHeartbeat", json!({"taskToken":token}))
            .await
            .1["__type"],
        "TaskTimedOut"
    );
    f.close().await;
}
#[tokio::test]
async fn test_should_reject_invalid_public_output_or_credentials_without_consuming_valid_token() {
    let mut f = CallbackFixture::new();
    let execution = f.start(json!({})).await;
    let (command, responder) = f.next().await;
    let token = token(&command);
    for (action, input, code) in [
        (
            "SendTaskSuccess",
            json!({"taskToken":token,"output":"{"}),
            "InvalidOutput",
        ),
        (
            "SendTaskSuccess",
            json!({"taskToken":token,"output":"x".repeat(262_145)}),
            "InvalidOutput",
        ),
        (
            "SendTaskFailure",
            json!({"taskToken":token,"error":"x".repeat(257)}),
            "ValidationException",
        ),
        (
            "SendTaskFailure",
            json!({"taskToken":token,"cause":"x".repeat(32769)}),
            "ValidationException",
        ),
        (
            "SendTaskHeartbeat",
            json!({"taskToken":"malformed"}),
            "InvalidToken",
        ),
        (
            "SendTaskHeartbeat",
            json!({"taskToken":"f".repeat(64)}),
            "InvalidToken",
        ),
    ] {
        let (status, body) = f.inner.call(action, input).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["__type"], code);
        assert!(!body.to_string().contains(&token));
    }
    success(&f, &token, json!("still-live")).await;
    finish(command, responder);
    assert_eq!(f.inner.wait(&execution).await["status"], "SUCCEEDED");
    f.close().await;
}
#[tokio::test]
async fn test_should_bind_callback_epoch_and_trusted_http_scope() {
    let mut f = CallbackFixture::new();
    let mut other_epoch = CallbackFixture::new();
    let mut foreign =
        CallbackFixture::scoped(TenantScope::new("aws", "us-west-2", "111111111111").unwrap());
    let execution = f.start(json!({})).await;
    let (command, responder) = f.next().await;
    let token = token(&command);
    assert_eq!(
        other_epoch
            .inner
            .call("SendTaskHeartbeat", json!({"taskToken":token}))
            .await
            .1["__type"],
        "InvalidToken"
    );
    assert_eq!(
        foreign
            .inner
            .call("SendTaskHeartbeat", json!({"taskToken":token}))
            .await
            .1["__type"],
        "InvalidToken"
    );
    let config = StepFunctionsHttpConfig::new(
        RequestScope::new("aws".into(), "us-west-2".into(), "111111111111".into()).unwrap(),
        true,
        None,
    )
    .unwrap();
    let original = f.inner.service.clone();
    f.inner.service = StepFunctionsHttpService::new(f.inner.handle.clone(), config);
    assert_eq!(
        f.inner
            .call("SendTaskHeartbeat", json!({"taskToken":token}))
            .await
            .1["__type"],
        "AccessDeniedException"
    );
    f.inner.service = original;
    success(&f, &token, json!(42)).await;
    finish(command, responder);
    assert_eq!(f.inner.wait(&execution).await["status"], "SUCCEEDED");
    f.close().await;
    foreign.close().await;
    other_epoch.close().await;
}
#[tokio::test]
async fn test_should_reject_unbound_or_closed_callback_api_and_inconsistent_compiler_composition() {
    let mut unbound = Fixture::new(ApiLimits::default());
    for action in ["SendTaskSuccess", "SendTaskFailure", "SendTaskHeartbeat"] {
        let mut input = json!({"taskToken":"a".repeat(64)});
        if action == "SendTaskSuccess" {
            input["output"] = json!("null");
        }
        assert_eq!(
            unbound.call(action, input).await.1["__type"],
            "UnsupportedOperation"
        );
    }
    unbound.close().await;
    let mut f = CallbackFixture::new();
    assert!(matches!(
        StepFunctionsApi::new(
            TenantScope::default(),
            TaskSupport::Disabled,
            f.registry.clone(),
            ApiLimits::default()
        ),
        Err(ApiError::Configuration)
    ));
    f.callbacks.close();
    assert_eq!(
        f.inner
            .call("SendTaskHeartbeat", json!({"taskToken":"a".repeat(64)}))
            .await
            .1["__type"],
        "ServiceUnavailable"
    );
    f.close().await;
    assert!(matches!(
        f.registry.submit_callback(CallbackCommand::heartbeat(
            CallbackToken::try_from("a".repeat(64)).unwrap()
        )),
        Err(CallbackError::Unavailable)
    ));
}
#[tokio::test]
async fn test_should_submit_callback_independently_of_metadata_admission() {
    let mut f = CallbackFixture::new();
    let mut metadata = Vec::new();
    for _ in 0..256 {
        match f.registry.submit(RegistryCommand::Machines) {
            Ok(receipt) => metadata.push(receipt),
            Err(_) => break,
        }
    }
    assert!(!metadata.is_empty());
    assert!(f.registry.submit(RegistryCommand::Machines).is_err());
    let error = f
        .registry
        .submit_callback(CallbackCommand::heartbeat(
            "a".repeat(64).try_into().unwrap(),
        ))
        .unwrap()
        .wait(deadline())
        .await
        .unwrap_err();
    assert_eq!(
        super::super::error::callback(error).code,
        Code::InvalidToken
    );
    drop(metadata);
    f.close().await;
}

#[tokio::test]
async fn test_should_return_public_callback_throttling_without_consuming_the_live_token() {
    let mut f = CallbackFixture::new();
    let execution = f.start(json!({})).await;
    let (command, responder) = f.next().await;
    let token = token(&command);
    let mut acknowledgments = Vec::new();
    for _ in 0..2048 {
        match f.registry.submit_callback(CallbackCommand::heartbeat(
            "a".repeat(64).try_into().unwrap(),
        )) {
            Ok(ack) => acknowledgments.push(ack),
            Err(CallbackError::Capacity) => break,
            Err(error) => panic!("unexpected admission error: {error:?}"),
        }
    }
    assert!(!acknowledgments.is_empty());
    let (status, error) = f
        .inner
        .call("SendTaskSuccess", json!({"taskToken":token,"output":"42"}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["__type"], "ThrottlingException");
    drop(acknowledgments);
    timeout_at(deadline(), async {
        loop {
            let (status, body) = f
                .inner
                .call("SendTaskHeartbeat", json!({"taskToken":token}))
                .await;
            if status == StatusCode::OK {
                break;
            }
            assert_eq!(body["__type"], "ThrottlingException");
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    success(&f, &token, json!(42)).await;
    finish(command, responder);
    assert_eq!(f.inner.wait(&execution).await["status"], "SUCCEEDED");
    f.close().await;
}

async fn caller_context() -> RequestContext {
    let (tx, mut rx) = mpsc::channel(1);
    let config = StepFunctionsHttpConfig::new(
        RequestScope::new("aws".into(), "us-east-1".into(), "000000000000".into()).unwrap(),
        true,
        None,
    )
    .unwrap()
    .with_budgets(
        rustack_core::http::BodyBudget::control(),
        Duration::from_secs(1),
    )
    .unwrap();
    let service = StepFunctionsHttpService::new(Arc::new(CaptureContext(tx)), config);
    service
        .handle(
            HttpRequest::builder()
                .method("POST")
                .uri("/")
                .header("content-type", CONTENT_TYPE)
                .header("x-amz-target", "AWSStepFunctions.ListStateMachines")
                .body(Full::new(Bytes::from_static(b"{}")))
                .unwrap(),
        )
        .await;
    rx.recv().await.unwrap()
}
#[rstest::rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn test_should_keep_accepted_public_callback_after_abandoned_or_late_ready_observer(
    #[case] late: bool,
) {
    let mut f = CallbackFixture::new();
    let execution = f.start(json!({})).await;
    let (command, responder) = f.next().await;
    let token = token(&command);
    let context = caller_context().await;
    let caller_deadline = context.deadline();
    let request = Request::decode(
        Op::SendTaskSuccess,
        &serde_json::to_vec(&json!({"taskToken":token,"output":"42"})).unwrap(),
    )
    .unwrap();
    let mut future = f.inner.handle.handle(context, request);
    let flag = Arc::new(WakeFlag::default());
    let waker = std::task::Waker::from(flag.clone());
    assert!(
        future
            .as_mut()
            .poll(&mut std::task::Context::from_waker(&waker))
            .is_pending()
    );
    if late {
        timeout_at(caller_deadline, async {
            while !flag.0.load(std::sync::atomic::Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        sleep_until(caller_deadline).await;
        assert_eq!(
            future.await.unwrap_err().code,
            Code::RequestTimeoutException
        );
    } else {
        drop(future);
    }
    // API intake is independently joined before the actual Lambda report.
    f.inner.api.shutdown(deadline()).await.unwrap();
    finish(command, responder);
    let mut receipt = f
        .registry
        .submit(RegistryCommand::Execution(execution.parse().unwrap()))
        .unwrap();
    let RegistryResult::Execution(snapshot) = receipt.wait(deadline()).await.unwrap() else {
        panic!("execution");
    };
    let view = snapshot.handle().wait(deadline()).await.unwrap();
    assert!(
        matches!(view.outcome().unwrap().as_ref(),ExecutionOutcome::Succeeded(output) if output.value()==&json!(42))
    );
    f.close().await;
}
#[tokio::test]
async fn test_should_reject_expired_public_request_before_consuming_live_token() {
    let mut f = CallbackFixture::new();
    let execution = f.start(json!({})).await;
    let (command, responder) = f.next().await;
    let token = token(&command);
    let context = caller_context().await;
    sleep_until(context.deadline()).await;
    let request = Request::decode(
        Op::SendTaskFailure,
        &serde_json::to_vec(&json!({"taskToken":token,"error":"must-not-run"})).unwrap(),
    )
    .unwrap();
    assert_eq!(
        f.inner
            .handle
            .handle(context, request)
            .await
            .unwrap_err()
            .code,
        Code::RequestTimeoutException
    );
    success(&f, &token, json!(true)).await;
    finish(command, responder);
    assert_eq!(f.inner.wait(&execution).await["status"], "SUCCEEDED");
    f.close().await;
}
#[rstest::rstest]
#[case(CallbackError::InvalidToken, Code::InvalidToken, 400)]
#[case(CallbackError::TaskTimedOut, Code::TaskTimedOut, 400)]
#[case(CallbackError::Data(AslError::DataLimit), Code::InvalidOutput, 400)]
#[case(CallbackError::Validation, Code::ValidationException, 400)]
#[case(CallbackError::Capacity, Code::ThrottlingException, 400)]
#[case(CallbackError::Deadline, Code::RequestTimeoutException, 408)]
#[case(CallbackError::Unavailable, Code::ServiceUnavailable, 503)]
#[case(CallbackError::OwnerLost, Code::InternalFailure, 500)]
#[case(CallbackError::Progress, Code::InternalFailure, 500)]
#[case(CallbackError::AttemptExists, Code::InternalFailure, 500)]
#[test]
fn test_should_map_cached_callback_sources_to_static_action_errors(
    #[case] cause: CallbackError,
    #[case] code: Code,
    #[case] status: u16,
) {
    let error = super::super::error::callback(CallbackError::Cached(Arc::new(cause)));
    assert_eq!(error.code, code);
    assert_eq!(error.status_code.as_u16(), status);
    assert!(
        error
            .source
            .as_ref()
            .unwrap()
            .downcast_ref::<CallbackError>()
            .is_some()
    );
}
