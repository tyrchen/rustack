//! Real typed HTTP → API owner → registry → execution semantics.

use std::{sync::Arc, time::Duration};

use bytes::Bytes;
use http::{Request as HttpRequest, StatusCode};
use http_body_util::{BodyExt, Full};
use rustack_stepfunctions_core::{
    ExecutionLimits, ExecutionSupervisor, HistoryLimits, RegistryLimits, RegistrySupervisor,
    RuntimeLimits,
};
use rustack_stepfunctions_http::{
    CONTENT_TYPE, RequestScope, StepFunctionsHttpConfig, StepFunctionsHttpService,
};
use serde_json::{Value, json};

use super::*;

struct Fixture {
    service: StepFunctionsHttpService,
    api: ApiSupervisor,
    registry: RegistrySupervisor,
    handle: Arc<StepFunctionsApi>,
}
impl Fixture {
    fn new(limits: ApiLimits) -> Self {
        let scope = TenantScope::default();
        let runtime = ExecutionSupervisor::new(
            RuntimeLimits::default(),
            HistoryLimits::default(),
            256 * 1024 * 1024,
            None,
        )
        .unwrap();
        let (registry, owner) = Registry::new(
            scope.clone(),
            RegistryLimits::default(),
            runtime,
            ExecutionLimits::default(),
        )
        .unwrap();
        Self::from_registry(registry, owner, TaskSupport::Disabled, limits)
    }
    fn from_registry(
        registry: Registry,
        owner: RegistrySupervisor,
        tasks: TaskSupport,
        limits: ApiLimits,
    ) -> Self {
        let scope = registry.scope().clone();
        let (handle, api) = StepFunctionsApi::new(scope.clone(), tasks, registry, limits).unwrap();
        let config = StepFunctionsHttpConfig::new(
            RequestScope::new(
                scope.partition().into(),
                scope.region().into(),
                scope.account().into(),
            )
            .unwrap(),
            true,
            None,
        )
        .unwrap();
        Self {
            service: StepFunctionsHttpService::new(handle.clone(), config),
            api,
            registry: owner,
            handle,
        }
    }
    async fn call(&self, action: &str, input: Value) -> (StatusCode, Value) {
        let request = HttpRequest::builder()
            .method("POST")
            .uri("/")
            .header("content-type", CONTENT_TYPE)
            .header("x-amz-target", format!("AWSStepFunctions.{action}"))
            .body(Full::new(Bytes::from(serde_json::to_vec(&input).unwrap())))
            .unwrap();
        let response = self.service.handle(request).await;
        let status = response.status();
        assert!(response.headers().contains_key("x-amzn-requestid"));
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            if body.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&body).unwrap()
            },
        )
    }
    async fn create(&self, name: &str, definition: Value) -> String {
        let(status,response)=self.call("CreateStateMachine",json!({"name":name,"roleArn":"arn:aws:iam::000000000000:role/Workflow","definition":definition.to_string()})).await;
        assert_eq!(status, StatusCode::OK, "{response}");
        response["stateMachineArn"].as_str().unwrap().into()
    }
    async fn wait(&self, arn: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let (status, response) = self
                .call("DescribeExecution", json!({"executionArn":arn}))
                .await;
            assert_eq!(status, StatusCode::OK);
            if response["status"] != "RUNNING" {
                return response;
            }
            assert!(Instant::now() < deadline);
            tokio::task::yield_now().await;
        }
    }
    async fn close(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        let api = self.api.shutdown(deadline).await;
        let registry = self.registry.shutdown(deadline).await;
        assert!(api.is_ok(), "{api:?}");
        assert!(registry.is_ok(), "{registry:?}");
    }
}

mod callback;
fn succeed() -> Value {
    json!({"StartAt":"Done","States":{"Done":{"Type":"Succeed"}}})
}
fn wait() -> Value {
    json!({"StartAt":"Waiting","States":{"Waiting":{"Type":"Wait","Seconds":60,"Next":"Done"},"Done":{"Type":"Succeed"}}})
}
async fn tag_roundtrip(f: &Fixture, arn: &str) {
    let (status, _) = f
        .call(
            "TagResource",
            json!({"resourceArn":arn,"tags":[{"key":"team","value":"workflow"}]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, tags) = f
        .call("ListTagsForResource", json!({"resourceArn":arn}))
        .await;
    assert_eq!(tags["tags"][0]["value"], "workflow");
    assert_eq!(
        f.call(
            "UntagResource",
            json!({"resourceArn":arn,"tagKeys":["team"]})
        )
        .await
        .0,
        StatusCode::OK
    );
}
#[tokio::test]
async fn test_should_serve_all_fifteen_real_m1_operations_and_captured_revisions() {
    let mut f = Fixture::new(ApiLimits::default());
    let arn = f.create("Workflow", succeed()).await;
    let (status, original) = f
        .call("DescribeStateMachine", json!({"stateMachineArn":arn}))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(original["status"], "ACTIVE");
    assert!(original["creationDate"].is_number());
    tag_roundtrip(&f, &arn).await;
    let (_, started) = f
        .call(
            "StartExecution",
            json!({"stateMachineArn":arn,"name":"first","input":" {\"n\":7} ","traceHeader":""}),
        )
        .await;
    let execution = started["executionArn"].as_str().unwrap();
    let finished = f.wait(execution).await;
    assert_eq!(finished["status"], "SUCCEEDED");
    assert_eq!(finished["input"], " {\"n\":7} ");
    assert_eq!(
        serde_json::from_str::<Value>(finished["output"].as_str().unwrap()).unwrap(),
        json!({"n":7})
    );
    let (_, updated) = f
        .call(
            "UpdateStateMachine",
            json!({"stateMachineArn":arn,"definition":wait().to_string()}),
        )
        .await;
    assert_ne!(updated["revisionId"], original["revisionId"]);
    let (_, captured) = f
        .call(
            "DescribeStateMachineForExecution",
            json!({"executionArn":execution}),
        )
        .await;
    assert_eq!(captured["revisionId"], original["revisionId"]);
    assert_eq!(captured["definition"], original["definition"]);
    assert_eq!(captured["updateDate"], original["creationDate"]);
    let (_, list) = f
        .call("ListExecutions", json!({"stateMachineArn":arn}))
        .await;
    assert_eq!(list["executions"].as_array().unwrap().len(), 1);
    let (_, history) = f
        .call("GetExecutionHistory", json!({"executionArn":execution}))
        .await;
    assert_eq!(
        history["events"].as_array().unwrap().last().unwrap()["type"],
        "ExecutionSucceeded"
    );
    assert_eq!(
        f.call(
            "ValidateStateMachineDefinition",
            json!({"definition":succeed().to_string()})
        )
        .await
        .1["result"],
        "OK"
    );
    assert_eq!(
        f.call("ListStateMachines", json!({})).await.1["stateMachines"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let (_, running) = f
        .call(
            "StartExecution",
            json!({"stateMachineArn":arn,"name":"stop"}),
        )
        .await;
    let stop = running["executionArn"].as_str().unwrap();
    let (_, stopped) = f
        .call(
            "StopExecution",
            json!({"executionArn":stop,"error":"","cause":""}),
        )
        .await;
    let terminal = f.wait(stop).await;
    assert_eq!(terminal["status"], "ABORTED");
    assert_eq!(terminal["stopDate"], stopped["stopDate"]);
    assert_eq!(terminal["error"], "");
    assert_eq!(
        f.call("DeleteStateMachine", json!({"stateMachineArn":arn}))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        f.call("DescribeExecution", json!({"executionArn":execution}))
            .await
            .0,
        StatusCode::OK
    );
    f.close().await;
}
#[tokio::test]
async fn test_should_filter_validate_diagnostics_without_mutation_and_reject_profiles() {
    let mut f = Fixture::new(ApiLimits::default());
    let definition = json!({"StartAt":"Done","States":{"Done":{"Type":"Succeed"},"Unused":{"Type":"Succeed"},"Other":{"Type":"Succeed"}}});
    let (_, errors) = f
        .call(
            "ValidateStateMachineDefinition",
            json!({"definition":definition.to_string()}),
        )
        .await;
    assert_eq!(errors["result"], "OK");
    assert_eq!(errors["diagnostics"], json!([]));
    let (_, warnings) = f
        .call(
            "ValidateStateMachineDefinition",
            json!({"definition":definition.to_string(),"severity":"WARNING","maxResults":1}),
        )
        .await;
    assert_eq!(warnings["diagnostics"].as_array().unwrap().len(), 1);
    assert_eq!(warnings["truncated"], true);
    let (status, invalid) = f
        .call(
            "ValidateStateMachineDefinition",
            json!({"definition":"not json"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(invalid["result"], "FAIL");
    for extra in [
        json!({"type":"EXPRESS"}),
        json!({"publish":true}),
        json!({"tracingConfiguration":{"enabled":true}}),
        json!({"loggingConfiguration":{"level":"ALL"}}),
        json!({"encryptionConfiguration":{"type":"CUSTOMER_MANAGED_KMS_KEY"}}),
    ] {
        let mut input = json!({"name":"Rejected","roleArn":"arn:aws:iam::000000000000:role/Workflow","definition":succeed().to_string()});
        input
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert_eq!(
            f.call("CreateStateMachine", input).await.0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        f.call("ListStateMachines", json!({})).await.1["stateMachines"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    f.close().await;
}
#[tokio::test]
async fn test_should_bind_stable_machine_cursor_to_full_query_and_replay_after_deletion() {
    let mut f = Fixture::new(ApiLimits::default());
    let a = f.create("A", succeed()).await;
    let b = f.create("B", succeed()).await;
    let (_, first) = f.call("ListStateMachines", json!({"maxResults":1})).await;
    let token = first["nextToken"].as_str().unwrap();
    assert_eq!(token.len(), 64);
    f.call("DeleteStateMachine", json!({"stateMachineArn":b}))
        .await;
    f.create("C", succeed()).await;
    for _ in 0..2 {
        let (status, next) = f
            .call(
                "ListStateMachines",
                json!({"maxResults":1,"nextToken":token}),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(next["stateMachines"][0]["name"], "B");
        assert!(next.get("nextToken").is_none());
    }
    for (op, input) in [
        (
            "ListStateMachines",
            json!({"maxResults":2,"nextToken":token}),
        ),
        (
            "ListExecutions",
            json!({"stateMachineArn":a,"maxResults":1,"nextToken":token}),
        ),
        (
            "ListStateMachines",
            json!({"maxResults":1,"nextToken":"f".repeat(64)}),
        ),
    ] {
        let (status, error) = f.call(op, input).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error["__type"], "InvalidToken");
    }
    f.close().await;
}
#[tokio::test]
async fn test_should_page_reverse_and_data_disabled_history_with_fixed_high_water() {
    let mut f = Fixture::new(ApiLimits::default());
    let arn = f.create("History", succeed()).await;
    let (_, started) = f
        .call(
            "StartExecution",
            json!({"stateMachineArn":arn,"input":"{\"secret\":7}"}),
        )
        .await;
    let arn = started["executionArn"].as_str().unwrap();
    f.wait(arn).await;
    let(_,first)=f.call("GetExecutionHistory",json!({"executionArn":arn,"maxResults":1,"reverseOrder":true,"includeExecutionData":false})).await;
    assert_eq!(first["events"][0]["type"], "ExecutionSucceeded");
    assert!(
        first["events"][0]["executionSucceededEventDetails"]
            .get("output")
            .is_none()
    );
    let token = first["nextToken"].clone();
    let(_,next)=f.call("GetExecutionHistory",json!({"executionArn":arn,"maxResults":1,"reverseOrder":true,"includeExecutionData":false,"nextToken":token})).await;
    assert!(next["events"][0]["id"].as_i64().unwrap() < first["events"][0]["id"].as_i64().unwrap());
    let(_,invalid)=f.call("GetExecutionHistory",json!({"executionArn":arn,"maxResults":1,"reverseOrder":false,"includeExecutionData":false,"nextToken":token})).await;
    assert_eq!(invalid["__type"], "InvalidToken");
    f.close().await;
}
#[tokio::test]
async fn test_should_preserve_running_start_idempotency_and_raw_input_rejection_over_http() {
    let mut f = Fixture::new(ApiLimits::default());
    let arn = f.create("Wait", wait()).await;
    let input = json!({"stateMachineArn":arn,"name":"run","input":" {\"n\":7}"});
    let (_, first) = f.call("StartExecution", input.clone()).await;
    let (_, second) = f.call("StartExecution", input).await;
    assert_eq!(first, second);
    assert_eq!(
        f.call(
            "StartExecution",
            json!({"stateMachineArn":arn,"name":"run","input":"{\"n\":7}"})
        )
        .await
        .1["__type"],
        "ExecutionAlreadyExists"
    );
    assert_eq!(
        f.call("StartExecution", json!({"stateMachineArn":arn,"input":""}))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    f.close().await;
}
#[tokio::test]
async fn test_should_report_explicit_unsupported_fields_and_keep_scope_and_readiness() {
    let mut f = Fixture::new(ApiLimits::default());
    assert!(f.handle.is_ready());
    let arn = f.create("Scope", succeed()).await;
    assert_eq!(
        f.call(
            "UpdateStateMachine",
            json!({"stateMachineArn":arn,"loggingConfiguration":{"level":"OFF"}})
        )
        .await
        .1["__type"],
        "MissingRequiredParameter"
    );
    for (op, input) in [
        (
            "DescribeStateMachine",
            json!({"stateMachineArn":arn,"includedData":"METADATA_ONLY"}),
        ),
        (
            "ListExecutions",
            json!({"stateMachineArn":arn,"statusFilter":"PENDING_REDRIVE"}),
        ),
        ("SendTaskHeartbeat", json!({"taskToken":"opaque"})),
    ] {
        assert_eq!(f.call(op, input).await.1["__type"], "UnsupportedOperation");
    }
    assert_eq!(
        f.call(
            "DescribeStateMachine",
            json!({"stateMachineArn":"arn:aws:states:us-west-2:000000000000:stateMachine:Scope"})
        )
        .await
        .1["__type"],
        "InvalidArn"
    );
    f.close().await;
    assert!(!f.handle.is_ready());
}

#[tokio::test]
async fn test_should_map_missing_tag_resources_and_reject_cumulative_tag_quota_atomically() {
    let mut f = Fixture::new(ApiLimits::default());
    let missing = "arn:aws:states:us-east-1:000000000000:stateMachine:Missing";
    for (op, input) in [
        ("TagResource", json!({"resourceArn":missing,"tags":[]})),
        ("UntagResource", json!({"resourceArn":missing,"tagKeys":[]})),
        ("ListTagsForResource", json!({"resourceArn":missing})),
    ] {
        let (status, reply) = f.call(op, input).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(reply["__type"], "ResourceNotFound");
    }
    let arn = f.create("Tags", succeed()).await;
    let tags: Vec<_> = (0..50)
        .map(|n| json!({"key":format!("key{n}"),"value":"v"}))
        .collect();
    assert_eq!(
        f.call("TagResource", json!({"resourceArn":arn,"tags":tags}))
            .await
            .0,
        StatusCode::OK
    );
    let (status, reply) = f
        .call(
            "TagResource",
            json!({"resourceArn":arn,"tags":[{"key":"extra"}]}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(reply["__type"], "TooManyTags");
    assert_eq!(
        f.call("ListTagsForResource", json!({"resourceArn":arn}))
            .await
            .1["tags"]
            .as_array()
            .unwrap()
            .len(),
        50
    );
    f.close().await;
}
#[tokio::test]
async fn test_should_hold_execution_status_cursor_boundary_across_later_stop() {
    let mut f = Fixture::new(ApiLimits::default());
    let arn = f.create("Filtered", wait()).await;
    let mut executions = Vec::new();
    for name in ["a", "b", "c"] {
        executions.push(
            f.call("StartExecution", json!({"stateMachineArn":arn,"name":name}))
                .await
                .1["executionArn"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
    }
    let (_, first) = f
        .call(
            "ListExecutions",
            json!({"stateMachineArn":arn,"statusFilter":"RUNNING","maxResults":1}),
        )
        .await;
    let token = first["nextToken"].clone();
    for execution in executions {
        f.call("StopExecution", json!({"executionArn":execution}))
            .await;
    }
    let(status,second)=f.call("ListExecutions",json!({"stateMachineArn":arn,"statusFilter":"RUNNING","maxResults":1,"nextToken":token})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(second["executions"][0]["status"], "RUNNING");
    assert_ne!(
        first["executions"][0]["executionArn"],
        second["executions"][0]["executionArn"]
    );
    assert_eq!(
        f.call(
            "ListExecutions",
            json!({"stateMachineArn":arn,"statusFilter":"ABORTED","maxResults":1,"nextToken":token})
        )
        .await
        .1["__type"],
        "InvalidToken"
    );
    f.close().await;
}
fn controlled(
    work: impl FnOnce() -> Result<Response, StepFunctionsError> + Send + 'static,
    deadline: Instant,
    panic_worker: bool,
) -> (
    Arc<StepFunctionsApi>,
    ApiSupervisor,
    RegistrySupervisor,
    oneshot::Receiver<Reply>,
) {
    let runtime = ExecutionSupervisor::new(
        RuntimeLimits::default(),
        HistoryLimits::default(),
        256 * 1024 * 1024,
        None,
    )
    .unwrap();
    let (registry, registry_owner) = Registry::new(
        TenantScope::default(),
        RegistryLimits::default(),
        runtime,
        ExecutionLimits::default(),
    )
    .unwrap();
    let (api, mut owner, stop) = StepFunctionsApi::compose(
        TenantScope::default(),
        TaskSupport::Disabled,
        registry,
        ApiLimits::new(1, 1, 32, 8 * 1024 * 1024).unwrap(),
    )
    .unwrap();
    let state = owner.state.as_mut().unwrap();
    let context = state.io.context.clone();
    let (reply, receiver) = oneshot::channel();
    let permit = api.admission.capacity.clone().try_acquire_owned().unwrap();
    let job = state.jobs.spawn(async move {
        assert!(!panic_worker, "controlled API worker failure");
        worker::cpu(&context, deadline, work).await
    });
    state.slots.insert(job.id(), Slot { reply, permit });
    let supervisor = ApiSupervisor {
        closing: api.admission.closing.clone(),
        join: Some(spawn(owner.run(stop))),
        result: None,
    };
    (api, supervisor, registry_owner, receiver)
}
fn ready_response() -> Response {
    rustack_stepfunctions_model::output::ListStateMachinesOutput::default().into()
}
#[tokio::test]
async fn test_should_join_same_blocking_work_after_shutdown_observation_expires() {
    let (entered, ready) = oneshot::channel();
    let (release, gate) = std::sync::mpsc::channel::<()>();
    let (api, mut owner, mut registry, receiver) = controlled(
        move || {
            let _sent = entered.send(());
            let _released = gate.recv();
            Ok(ready_response())
        },
        Instant::now() + Duration::from_secs(5),
        false,
    );
    ready.await.unwrap();
    assert!(matches!(
        owner
            .shutdown(Instant::now() + Duration::from_millis(5))
            .await,
        Err(ApiError::Deadline)
    ));
    assert_eq!(api.admission.capacity.available_permits(), 0);
    drop(release);
    owner
        .shutdown(Instant::now() + Duration::from_secs(5))
        .await
        .unwrap();
    let reply = receiver.await.unwrap();
    assert!(reply.result.is_ok());
    assert_eq!(api.admission.capacity.available_permits(), 0);
    drop(reply.permit);
    assert_eq!(api.admission.capacity.available_permits(), 1);
    registry
        .shutdown(Instant::now() + Duration::from_secs(5))
        .await
        .unwrap();
}
#[tokio::test]
async fn test_should_retain_root_slot_after_abort_even_when_blocking_work_finishes() {
    let (entered, ready) = oneshot::channel();
    let (release, gate) = std::sync::mpsc::channel::<()>();
    let (done, finished) = oneshot::channel();
    let (api, mut owner, mut registry, _receiver) = controlled(
        move || {
            let _sent = entered.send(());
            let _released = gate.recv();
            let _done = done.send(());
            Ok(ready_response())
        },
        Instant::now() + Duration::from_secs(5),
        false,
    );
    ready.await.unwrap();
    owner.join.as_ref().unwrap().abort();
    assert!(
        owner
            .shutdown(Instant::now() + Duration::from_secs(5))
            .await
            .is_err()
    );
    drop(release);
    finished.await.unwrap();
    assert_eq!(api.admission.capacity.available_permits(), 0);
    assert!(!api.is_ready());
    registry
        .shutdown(Instant::now() + Duration::from_secs(5))
        .await
        .unwrap();
}
#[tokio::test]
async fn test_should_keep_worker_diagnostic_after_quarantined_shutdown() {
    let (api, mut owner, mut registry, receiver) = controlled(
        || Ok(ready_response()),
        Instant::now() + Duration::from_secs(5),
        true,
    );
    let reply = receiver.await.unwrap();
    assert_eq!(reply.result.unwrap_err().code, Code::InternalFailure);
    assert!(matches!(
        api.diagnostic().as_deref(),
        Some(ApiError::Worker(_))
    ));
    assert!(
        owner
            .shutdown(Instant::now() + Duration::from_secs(5))
            .await
            .is_err()
    );
    assert!(matches!(
        api.diagnostic().as_deref(),
        Some(ApiError::Worker(_))
    ));
    assert_eq!(api.admission.capacity.available_permits(), 0);
    registry
        .shutdown(Instant::now() + Duration::from_secs(5))
        .await
        .unwrap();
}
#[tokio::test(start_paused = true)]
async fn test_should_join_cpu_then_reject_a_ready_result_after_its_deadline() {
    let (entered, mut ready) = oneshot::channel();
    let (release, gate) = std::sync::mpsc::channel::<()>();
    let deadline = Instant::now() + Duration::from_secs(5);
    let (_, mut owner, mut registry, receiver) = controlled(
        move || {
            let _sent = entered.send(());
            let _released = gate.recv();
            Ok(ready_response())
        },
        deadline,
        false,
    );
    let observed = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        match ready.try_recv() {
            Ok(()) => break,
            Err(oneshot::error::TryRecvError::Empty) => {
                assert!(std::time::Instant::now() < observed);
                tokio::task::yield_now().await;
            }
            Err(oneshot::error::TryRecvError::Closed) => panic!("CPU worker did not enter"),
        }
    }
    tokio::time::advance(Duration::from_secs(6)).await;
    drop(release);
    let error = receiver.await.unwrap().result.unwrap_err();
    assert_eq!(error.code, Code::RequestTimeoutException);
    assert_eq!(error.status_code, StatusCode::REQUEST_TIMEOUT);
    owner
        .shutdown(Instant::now() + Duration::from_secs(5))
        .await
        .unwrap();
    registry
        .shutdown(Instant::now() + Duration::from_secs(5))
        .await
        .unwrap();
}
#[derive(Default)]
struct WakeFlag(std::sync::atomic::AtomicBool);
impl std::task::Wake for WakeFlag {
    fn wake(self: Arc<Self>) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}
struct CaptureContext(mpsc::Sender<RequestContext>);
#[async_trait]
impl StepFunctionsHandler for CaptureContext {
    async fn handle(
        &self,
        context: RequestContext,
        _request: Request,
    ) -> Result<Response, StepFunctionsError> {
        self.0.send(context).await.unwrap();
        Ok(ready_response())
    }
}
#[tokio::test(start_paused = true)]
async fn test_should_reject_an_already_ready_typed_reply_when_observer_resumes_after_deadline() {
    let mut fixture = Fixture::new(ApiLimits::default());
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
    let request = HttpRequest::builder()
        .method("POST")
        .uri("/")
        .header("content-type", CONTENT_TYPE)
        .header("x-amz-target", "AWSStepFunctions.ListStateMachines")
        .body(Full::new(Bytes::from_static(b"{}")))
        .unwrap();
    service.handle(request).await;
    let context = rx.recv().await.unwrap();
    let deadline = context.deadline();
    let api = fixture.handle.clone();
    let request = Request::decode(
        rustack_stepfunctions_model::StepFunctionsOperation::ListStateMachines,
        b"{}",
    )
    .unwrap();
    let mut future = api.handle(context, request);
    let flag = Arc::new(WakeFlag::default());
    let waker = std::task::Waker::from(flag.clone());
    let mut task = std::task::Context::from_waker(&waker);
    assert!(future.as_mut().poll(&mut task).is_pending());
    let observed = std::time::Instant::now() + Duration::from_secs(5);
    while !flag.0.load(std::sync::atomic::Ordering::SeqCst) {
        assert!(std::time::Instant::now() < observed);
        assert!(Instant::now() < deadline);
        tokio::task::yield_now().await;
    }
    assert!(Instant::now() < deadline);
    tokio::time::advance(Duration::from_secs(2)).await;
    assert_eq!(
        future.await.unwrap_err().code,
        Code::RequestTimeoutException
    );
    fixture.close().await;
}

#[tokio::test]
async fn test_should_reject_mismatched_trusted_http_scope_before_dispatch() {
    let mut fixture = Fixture::new(ApiLimits::default());
    let config = StepFunctionsHttpConfig::new(
        RequestScope::new("aws".into(), "us-west-2".into(), "111111111111".into()).unwrap(),
        true,
        None,
    )
    .unwrap();
    fixture.service = StepFunctionsHttpService::new(fixture.handle.clone(), config);
    let (status, body) = fixture.call("ListStateMachines", json!({})).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["__type"], "AccessDeniedException");
    fixture.close().await;
}

#[tokio::test]
async fn test_should_preserve_optional_failure_fields_in_describe_and_terminal_history() {
    let mut f = Fixture::new(ApiLimits::default());
    for (name, state) in [
        ("OmittedFailure", json!({"Type":"Fail"})),
        (
            "NamedFailure",
            json!({"Type":"Fail","Error":"BusinessError","Cause":"quoted \"cause\" 中文"}),
        ),
    ] {
        let arn = f
            .create(name, json!({"StartAt":"F","States":{"F":state}}))
            .await;
        let (_, started) = f
            .call("StartExecution", json!({"stateMachineArn":arn}))
            .await;
        let execution = started["executionArn"].as_str().unwrap();
        let description = f.wait(execution).await;
        let (_, history) = f
            .call(
                "GetExecutionHistory",
                json!({"executionArn":execution,"reverseOrder":true}),
            )
            .await;
        assert_eq!(description["status"], "FAILED");
        assert_eq!(description.get("error"), state.get("Error"));
        assert_eq!(description.get("cause"), state.get("Cause"));
        assert_eq!(description.get("output"), None);
        let details = &history["events"][0]["executionFailedEventDetails"];
        assert_eq!(details.get("error"), description.get("error"));
        assert_eq!(details.get("cause"), description.get("cause"));
    }
    f.close().await;
}
#[tokio::test]
async fn test_should_map_execution_timeout_consistently_after_start_request_has_completed() {
    let mut f = Fixture::new(ApiLimits::default());
    let mut definition = wait();
    definition["TimeoutSeconds"] = json!(1);
    let arn = f.create("Timeout", definition).await;
    let (_, started) = f
        .call("StartExecution", json!({"stateMachineArn":arn}))
        .await;
    let execution = started["executionArn"].as_str().unwrap();
    let description = f.wait(execution).await;
    let (_, history) = f
        .call(
            "GetExecutionHistory",
            json!({"executionArn":execution,"reverseOrder":true}),
        )
        .await;
    assert_eq!(description["status"], "TIMED_OUT");
    assert_eq!(description["error"], "States.Timeout");
    assert_eq!(
        description["cause"],
        history["events"][0]["executionTimedOutEventDetails"]["cause"]
    );
    assert_eq!(history["events"][0]["type"], "ExecutionTimedOut");
    f.close().await;
}

#[tokio::test]
async fn test_should_list_only_empty_published_inventory_for_existing_scoped_machines() {
    let mut fixture = Fixture::new(ApiLimits::default());
    let arn = fixture.create("UnpublishedInventory", succeed()).await;
    let before = fixture
        .call("DescribeStateMachine", json!({"stateMachineArn":arn}))
        .await
        .1;
    for maximum in [None, Some(0), Some(1), Some(1000)] {
        let mut input = json!({"stateMachineArn":arn});
        if let Some(maximum) = maximum {
            input["maxResults"] = json!(maximum);
        }
        let (status, response) = fixture.call("ListStateMachineVersions", input).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(response, json!({"stateMachineVersions":[]}));
    }
    for (input, code) in [
        (
            json!({"stateMachineArn":arn,"nextToken":"unissued"}),
            "InvalidToken",
        ),
        (
            json!({"stateMachineArn":arn,"nextToken":""}),
            "InvalidToken",
        ),
        (
            json!({"stateMachineArn":arn,"nextToken":"x".repeat(1025)}),
            "InvalidToken",
        ),
        (
            json!({"stateMachineArn":arn,"nextToken":"é"}),
            "InvalidToken",
        ),
        (
            json!({"stateMachineArn":arn,"maxResults":-1}),
            "ValidationException",
        ),
        (
            json!({"stateMachineArn":arn,"maxResults":1001}),
            "ValidationException",
        ),
        (json!({"stateMachineArn":format!("{arn}:1")}), "InvalidArn"),
        (
            json!({"stateMachineArn":format!("{arn}:alias")}),
            "InvalidArn",
        ),
        (
            json!({"stateMachineArn":"arn:aws:states:us-west-2:000000000000:stateMachine:Foreign"}),
            "InvalidArn",
        ),
        (
            json!({"stateMachineArn":"arn:aws:states:us-east-1:000000000000:stateMachine:Missing"}),
            "StateMachineDoesNotExist",
        ),
    ] {
        let (status, response) = fixture.call("ListStateMachineVersions", input).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(response["__type"], code);
    }
    let (status, response) = fixture
        .call(
            "UpdateStateMachine",
            json!({"stateMachineArn":arn,"publish":true}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(response["__type"], "ValidationException");
    assert_eq!(
        fixture
            .call("DescribeStateMachine", json!({"stateMachineArn":arn}))
            .await
            .1,
        before
    );
    assert_eq!(
        fixture
            .call("DeleteStateMachine", json!({"stateMachineArn":arn}))
            .await
            .0,
        StatusCode::OK
    );
    let (status, response) = fixture
        .call("ListStateMachineVersions", json!({"stateMachineArn":arn}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(response["__type"], "StateMachineDoesNotExist");
    fixture.close().await;
}
