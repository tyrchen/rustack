//! Controlled provider tests plus separately gated actual Native process fixtures.

use std::{
    collections::{BTreeSet, HashMap},
    io::{Cursor, Write},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};
use bytes::Bytes;
use rustack_lambda_core::{
    config::LambdaConfig,
    error::LambdaServiceError,
    executor::{Executor, ExecutorBackend, ExecutorError, InvokeRequest, InvokeResponse},
    invocation::InvocationContext,
    provider::RustackLambda,
    storage::FunctionStore,
};
use rustack_lambda_model::{
    input::CreateFunctionInput,
    types::{Environment, FunctionCode},
};
use rustack_stepfunctions_core::{
    AslCompiler, AslError, ClockSample, EvaluationContext, ExecutionDriver, ExecutionLimits,
    ExecutionOutcome, ExecutionSupervisor, HistoryLimits, Payload, RuntimeLimits, SystemUuidSource,
    TaskSupport, TenantScope, UuidSource,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::{
    sync::Semaphore,
    time::{Instant, timeout},
};
use uuid::Uuid;
use zip::{ZipWriter, write::SimpleFileOptions};

use super::*;

#[derive(Debug)]
struct TestExecutor {
    calls: AtomicUsize,
    entered: Arc<Semaphore>,
    release: Arc<Semaphore>,
    cleanup: Arc<Semaphore>,
    fail_cleanup: bool,
}
#[async_trait]
impl Executor for TestExecutor {
    async fn invoke(
        &self,
        req: InvokeRequest,
        context: InvocationContext,
    ) -> Result<InvokeResponse, ExecutorError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.entered.add_permits(1);
        tokio::select! {biased;_=context.cancelled()=>{},permit=self.release.acquire()=>{if let Ok(permit)=permit {permit.forget();}}}
        if let Ok(permit) = self.cleanup.acquire().await {
            permit.forget();
        }
        if self.fail_cleanup {
            return Err(ExecutorError::CleanupFailed(
                "controlled cleanup failure".into(),
            ));
        }
        Ok(InvokeResponse::success(req.payload, "$LATEST".into()))
    }
    async fn shutdown(&self) -> Result<(), ExecutorError> {
        Ok(())
    }
}
fn executor() -> Arc<TestExecutor> {
    Arc::new(TestExecutor {
        calls: AtomicUsize::new(0),
        entered: Arc::new(Semaphore::new(0)),
        release: Arc::new(Semaphore::new(1)),
        cleanup: Arc::new(Semaphore::new(1)),
        fail_cleanup: false,
    })
}
fn zip(bytes: &[u8], marker: &str) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().unix_permissions(0o755);
    zip.start_file("bootstrap", options).unwrap();
    zip.write_all(bytes).unwrap();
    zip.start_file("revision.txt", options).unwrap();
    zip.write_all(marker.as_bytes()).unwrap();
    zip.finish().unwrap().into_inner()
}
fn create(code: &[u8], env: &[(&str, &str)]) -> CreateFunctionInput {
    let architecture = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        _ => "x86_64",
    };
    CreateFunctionInput {
        function_name: "Worker".into(),
        runtime: Some("provided.al2023".into()),
        handler: Some("bootstrap".into()),
        role: "arn:aws:iam::000000000000:role/test".into(),
        timeout: Some(5),
        architectures: Some(vec![architecture.into()]),
        code: FunctionCode {
            zip_file: Some(STANDARD.encode(code)),
            ..FunctionCode::default()
        },
        environment: Some(Environment {
            variables: Some(
                env.iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect::<HashMap<_, _>>(),
            ),
        }),
        ..CreateFunctionInput::default()
    }
}
fn provider(executor: Arc<TestExecutor>) -> (Arc<RustackLambda>, TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let config = LambdaConfig {
        executor: ExecutorBackend::Native,
        ..LambdaConfig::default()
    };
    (
        Arc::new(RustackLambda::with_executor(
            FunctionStore::new(directory.path()),
            config,
            executor,
        )),
        directory,
    )
}
fn definition(resource: &str, parameters: Option<Value>) -> Value {
    let mut state = json!({"Type":"Task","Resource":resource,"End":true});
    if let Some(parameters) = parameters {
        state["Parameters"] = parameters;
    }
    json!({"StartAt":"Call","States":{"Call":state}})
}
fn direct() -> Value {
    definition(
        "arn:aws:lambda:us-east-1:000000000000:function:Worker",
        None,
    )
}
fn driver(definition: &Value, input: &Value) -> ExecutionDriver {
    let machine = AslCompiler::new(TenantScope::default(), TaskSupport::Lambda)
        .compile(&serde_json::to_vec(definition).unwrap())
        .unwrap()
        .machine()
        .clone();
    let payload = Payload::from_value(input).unwrap();
    ExecutionDriver::new(
        machine,
        payload,
        EvaluationContext::from_value(
            &json!({"Execution":{"Input":input,"Id":"execution-fixture"}}),
        )
        .unwrap(),
        ClockSample::new(
            Instant::now().into_std(),
            std::time::SystemTime::now().into(),
        ),
        ExecutionLimits::new(Duration::from_secs(20), 1000).unwrap(),
    )
    .unwrap()
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
fn executions(bridge: Arc<StepFunctionsLambdaBridge>) -> ExecutionSupervisor {
    ExecutionSupervisor::new(
        RuntimeLimits::new(8, 8, 2).unwrap(),
        HistoryLimits::default(),
        256 * 1024 * 1024,
        Some(bridge),
    )
    .unwrap()
}
fn output(outcome: &ExecutionOutcome) -> &Value {
    match outcome {
        ExecutionOutcome::Succeeded(output) => output.value(),
        other => panic!("expected success, got {other:?}"),
    }
}
async fn clean(
    owner: &mut ExecutionSupervisor,
    bridge: &mut LambdaBridgeSupervisor,
    provider: &RustackLambda,
) {
    let workflow = owner.drain(deadline()).await;
    let joined = bridge.shutdown(deadline()).await;
    let provider = provider.quiesce(Duration::from_secs(10)).await;
    assert!(workflow.is_ok(), "workflow cleanup {workflow:?}");
    assert!(joined.is_ok(), "bridge cleanup {joined:?}");
    assert!(provider.is_ok(), "provider cleanup {provider:?}");
}
#[tokio::test]
async fn test_should_invoke_shared_provider_direct_and_optimized_without_http_loopback() {
    for optimized in [false, true] {
        let exec = executor();
        let (provider, _dir) = provider(exec.clone());
        provider
            .create_function(create(&zip(b"fixture", "v1"), &[]))
            .await
            .unwrap();
        let (port, mut bridge) = StepFunctionsLambdaBridge::new(
            provider.clone(),
            TenantScope::default(),
            BridgeLimits::default(),
        )
        .unwrap();
        let mut owner = executions(port);
        let def = if optimized {
            definition(
                "arn:aws:states:::lambda:invoke",
                Some(json!({"FunctionName":"Worker","Payload.$":"$"})),
            )
        } else {
            direct()
        };
        let mut handle = owner.start(driver(&def, &json!({"number":7}))).unwrap();
        let result = handle.wait(deadline()).await;
        clean(&mut owner, &mut bridge, &provider).await;
        let result = result.unwrap();
        let body = output(result.outcome().unwrap());
        if optimized {
            assert_eq!(body.get("Payload"), Some(&json!({"number":7})));
            assert_eq!(body.get("StatusCode"), Some(&json!(200)));
            assert!(
                body.pointer("/SdkResponseMetadata/RequestId")
                    .and_then(Value::as_str)
                    .is_some_and(|v| v.len() == 36)
            );
        } else {
            assert_eq!(body, &json!({"number":7}));
        }
        assert_eq!(exec.calls.load(Ordering::SeqCst), 1);
    }
}
#[tokio::test]
async fn test_should_reject_dynamic_partial_and_cross_scope_references_before_execution() {
    for function in [
        "999999999999:function:Worker",
        "000000000000:function:Worker",
        "999999999999:function:Worker:1",
        "arn:aws:lambda:us-west-2:000000000000:function:Worker",
        "arn:aws:lambda:us-east-1:999999999999:function:Worker",
    ] {
        let exec = executor();
        let (provider, _dir) = provider(exec.clone());
        provider
            .create_function(create(&zip(b"fixture", "v1"), &[]))
            .await
            .unwrap();
        let (port, mut bridge) = StepFunctionsLambdaBridge::new(
            provider.clone(),
            TenantScope::default(),
            BridgeLimits::default(),
        )
        .unwrap();
        let mut owner = executions(port);
        let def = definition(
            "arn:aws:states:::lambda:invoke",
            Some(json!({"FunctionName.$":"$.function"})),
        );
        let mut handle = owner
            .start(driver(&def, &json!({"function":function})))
            .unwrap();
        let result = handle.wait(deadline()).await;
        clean(&mut owner, &mut bridge, &provider).await;
        assert!(
            matches!(result.unwrap().outcome().unwrap().as_ref(),ExecutionOutcome::Failed(f) if f.error()==Some("States.Runtime"))
        );
        assert_eq!(exec.calls.load(Ordering::SeqCst), 0);
    }
}
#[test]
fn test_should_map_function_error_only_from_transport_flag_and_bound_causes() {
    let payload = Bytes::from_static(
        br#"{"errorType":"BusinessError","errorMessage":"boom","FunctionError":"business data"}"#,
    );
    let response = InvokeResponse::success(payload.clone(), "$LATEST".into());
    let mapped = mapping::response(&response, false, "id").unwrap();
    assert!(mapped.result.is_ok());
    let mut response = InvokeResponse::success(payload, "$LATEST".into());
    response.function_error = Some("Unhandled".into());
    let mapped = mapping::response(&response, false, "id").unwrap();
    assert!(format!("{mapped:?}").find("boom").is_none());
    assert!(
        matches!(mapped.result,Err(ref failure) if failure.details().error()==Some("BusinessError"))
    );
}

mod native;

#[test]
fn test_should_redact_call_plan_names_and_reject_invalid_limits() {
    let plan = mapping::CallPlan {
        function: "function-debug-marker".into(),
        qualifier: Some("qualifier-debug-marker".into()),
        payload: Payload::parse(b"{}").unwrap(),
        mode: mapping::ResponseMode::Optimized,
        deadline: deadline(),
    };
    let debug = format!("{plan:?}");
    assert!(!debug.contains("function-debug-marker"));
    assert!(!debug.contains("qualifier-debug-marker"));
    for (accepted, seconds) in [(0, 1), (1025, 1), (1, 0), (1, 61)] {
        assert!(BridgeLimits::new(accepted, Duration::from_secs(seconds)).is_err());
    }
}
#[test]
fn test_should_enforce_result_quota_utf8_json_and_envelope_depth() {
    for bytes in [Bytes::from_static(b"not-json"), Bytes::from_static(&[0xff])] {
        let mapped = mapping::response(
            &InvokeResponse::success(bytes, "$LATEST".into()),
            false,
            "id",
        )
        .unwrap();
        assert!(matches!(mapped.result,Err(f) if f.details().error()==Some("States.Runtime")));
        assert!(mapped.diagnostic.is_some());
    }
    let exact = Bytes::from(serde_json::to_vec(&"x".repeat(262_142)).unwrap());
    assert_eq!(exact.len(), 262_144);
    assert!(
        mapping::response(
            &InvokeResponse::success(exact.clone(), "$LATEST".into()),
            false,
            "id"
        )
        .unwrap()
        .result
        .is_ok()
    );
    let optimized = mapping::response(
        &InvokeResponse::success(exact, "$LATEST".into()),
        true,
        "id",
    )
    .unwrap();
    assert!(
        matches!(optimized.result,Err(f) if f.details().error()==Some("States.DataLimitExceeded"))
    );
    let oversized = Bytes::from(vec![b' '; 262_145]);
    let mapped = mapping::response(
        &InvokeResponse::success(oversized, "$LATEST".into()),
        false,
        "id",
    )
    .unwrap();
    assert!(
        matches!(mapped.result,Err(f) if f.details().error()==Some("States.DataLimitExceeded"))
    );
    let mut nested = Value::Null;
    for _ in 0..64 {
        nested = json!([nested]);
    }
    let nested = Bytes::from(serde_json::to_vec(&nested).unwrap());
    let mapped = mapping::response(
        &InvokeResponse::success(nested, "$LATEST".into()),
        true,
        "id",
    )
    .unwrap();
    assert!(matches!(mapped.result,Err(f) if f.details().error()==Some("States.Runtime")));
}
#[test]
fn test_should_bound_function_error_type_and_utf8_cause_without_leaking_debug_data() {
    for error_type in [
        "States.Timeout".to_owned(),
        String::new(),
        "x".repeat(257),
        "bad\nname".to_owned(),
        "BusinessError".to_owned(),
    ] {
        let body =
            serde_json::to_vec(&json!({"errorType":error_type,"errorMessage":"é".repeat(40_000)}))
                .unwrap();
        let mut response = InvokeResponse::success(Bytes::from(body), "$LATEST".into());
        response.function_error = Some("Unhandled".into());
        let mapped = mapping::response(&response, false, "id").unwrap();
        let failure = mapped.result.unwrap_err();
        assert_eq!(
            failure.details().error(),
            Some(if error_type == "BusinessError" {
                "BusinessError"
            } else {
                "Lambda.Unknown"
            })
        );
        let cause = failure.details().cause().unwrap();
        assert!(cause.len() <= 32_768);
        assert!(cause.ends_with(" [truncated]"));
        assert!(!format!("{failure:?}").contains("errorMessage"));
    }
}
#[tokio::test]
async fn test_should_preserve_cleanup_after_stop_and_resume_same_shutdown_waiter() {
    let exec = Arc::new(TestExecutor {
        calls: AtomicUsize::new(0),
        entered: Arc::new(Semaphore::new(0)),
        release: Arc::new(Semaphore::new(0)),
        cleanup: Arc::new(Semaphore::new(0)),
        fail_cleanup: false,
    });
    let (provider, _directory) = provider(exec.clone());
    provider
        .create_function(create(&zip(b"fixture", "v1"), &[]))
        .await
        .unwrap();
    let (port, mut bridge) = StepFunctionsLambdaBridge::new(
        provider.clone(),
        TenantScope::default(),
        BridgeLimits::new(1, Duration::from_secs(1)).unwrap(),
    )
    .unwrap();
    let mut owner = executions(port);
    let mut handle = owner.start(driver(&direct(), &json!({}))).unwrap();
    timeout(Duration::from_secs(5), exec.entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    let stopped = handle
        .stop(TaskFailure::new("Stopped", None).unwrap(), deadline())
        .await
        .unwrap();
    assert!(matches!(
        stopped.outcome().unwrap().as_ref(),
        ExecutionOutcome::Aborted(_)
    ));
    assert!(
        matches!(bridge.shutdown(Instant::now()+Duration::from_millis(20)).await,Err(e) if matches!(e.as_ref(),BridgeError::Deadline))
    );
    assert_eq!(bridge.owned_calls(), 1);
    // Keep the same handle alive past the first bounded cleanup observation.
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(bridge.owned_calls(), 1);
    exec.cleanup.add_permits(1);
    clean(&mut owner, &mut bridge, &provider).await;
    assert_eq!(bridge.owned_calls(), 0);
    assert_eq!(
        handle.snapshot().cleanup(),
        rustack_stepfunctions_core::CleanupState::Clean
    );
}
#[tokio::test]
async fn test_should_reject_evaluated_invalid_invocation_type_without_executing() {
    let exec = executor();
    let (provider, _directory) = provider(exec.clone());
    provider
        .create_function(create(&zip(b"fixture", "v1"), &[]))
        .await
        .unwrap();
    let (port, mut bridge) = StepFunctionsLambdaBridge::new(
        provider.clone(),
        TenantScope::default(),
        BridgeLimits::default(),
    )
    .unwrap();
    let mut owner = executions(port);
    let definition = definition(
        "arn:aws:states:::lambda:invoke",
        Some(json!({"FunctionName":"Worker","InvocationType.$":"$.kind"})),
    );
    let mut handle = owner
        .start(driver(&definition, &json!({"kind":"Event"})))
        .unwrap();
    let result = handle.wait(deadline()).await;
    clean(&mut owner, &mut bridge, &provider).await;
    assert!(
        matches!(result.unwrap().outcome().unwrap().as_ref(),ExecutionOutcome::Failed(f) if f.error()==Some("States.Runtime"))
    );
    assert_eq!(exec.calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn test_should_refuse_mismatched_provider_scope_and_disabled_backend() {
    let exec = executor();
    let (provider, _directory) = provider(exec);
    let scope = TenantScope::new("aws", "us-west-2", "000000000000").unwrap();
    assert!(matches!(
        StepFunctionsLambdaBridge::new(provider, scope, BridgeLimits::default()),
        Err(BridgeError::Configuration)
    ));
    let provider = Arc::new(RustackLambda::new(LambdaConfig::default()));
    assert!(matches!(
        StepFunctionsLambdaBridge::new(provider, TenantScope::default(), BridgeLimits::default()),
        Err(BridgeError::Configuration)
    ));
}

#[tokio::test]
async fn test_should_retain_uncertain_backend_cleanup_and_refuse_clean_shutdown() {
    let exec = Arc::new(TestExecutor {
        calls: AtomicUsize::new(0),
        entered: Arc::new(Semaphore::new(0)),
        release: Arc::new(Semaphore::new(1)),
        cleanup: Arc::new(Semaphore::new(1)),
        fail_cleanup: true,
    });
    let (provider, _directory) = provider(exec);
    provider
        .create_function(create(&zip(b"fixture", "v1"), &[]))
        .await
        .unwrap();
    let (port, mut bridge) = StepFunctionsLambdaBridge::new(
        provider.clone(),
        TenantScope::default(),
        BridgeLimits::new(1, Duration::from_secs(1)).unwrap(),
    )
    .unwrap();
    let mut owner = executions(port);
    let mut handle = owner.start(driver(&direct(), &json!({}))).unwrap();
    let view = handle.wait(deadline()).await.unwrap();
    assert!(
        matches!(view.outcome().unwrap().as_ref(),ExecutionOutcome::Failed(f) if f.error()==Some("Lambda.ServiceException"))
    );
    assert!(
        matches!(bridge.shutdown(deadline()).await,Err(e) if matches!(e.as_ref(),BridgeError::Quarantined))
    );
    assert_eq!(bridge.owned_calls(), 1);
    assert!(matches!(
        owner
            .drain(Instant::now() + Duration::from_millis(20))
            .await,
        Err(rustack_stepfunctions_core::ExecutionError::Deadline)
    ));
    assert!(!format!("{:?}", bridge.diagnostic()).contains("controlled cleanup"));
}
#[tokio::test]
async fn test_should_preserve_root_admission_permit_after_manager_abort() {
    let exec = Arc::new(TestExecutor {
        calls: AtomicUsize::new(0),
        entered: Arc::new(Semaphore::new(0)),
        release: Arc::new(Semaphore::new(0)),
        cleanup: Arc::new(Semaphore::new(0)),
        fail_cleanup: false,
    });
    let (provider, _directory) = provider(exec.clone());
    provider
        .create_function(create(&zip(b"fixture", "v1"), &[]))
        .await
        .unwrap();
    let (port, mut bridge) = StepFunctionsLambdaBridge::new(
        provider.clone(),
        TenantScope::default(),
        BridgeLimits::new(1, Duration::from_secs(1)).unwrap(),
    )
    .unwrap();
    let mut owner = executions(port);
    let mut handle = owner.start(driver(&direct(), &json!({}))).unwrap();
    timeout(Duration::from_secs(5), exec.entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    bridge.manager.as_ref().unwrap().abort();
    assert!(
        matches!(bridge.shutdown(deadline()).await,Err(e) if matches!(e.as_ref(),BridgeError::Worker(_)))
    );
    assert_eq!(bridge.owned_calls(), 1);
    exec.cleanup.add_permits(1);
    let view = handle.wait(deadline()).await.unwrap();
    assert!(matches!(
        view.outcome().unwrap().as_ref(),
        ExecutionOutcome::Failed(_)
    ));
    owner.drain(deadline()).await.unwrap();
    provider.quiesce(Duration::from_secs(10)).await.unwrap();
    assert_eq!(bridge.owned_calls(), 1);
}

#[tokio::test]
async fn test_should_keep_bridge_ready_after_settled_missing_function_diagnostic() {
    let exec = executor();
    let (provider, _directory) = provider(exec.clone());
    let (port, mut bridge) = StepFunctionsLambdaBridge::new(
        provider.clone(),
        TenantScope::default(),
        BridgeLimits::default(),
    )
    .unwrap();
    let mut owner = executions(port.clone());
    let mut handle = owner.start(driver(&direct(), &json!({}))).unwrap();
    let outcome = handle.wait(deadline()).await;
    let settled = timeout(Duration::from_secs(5), async {
        while bridge.owned_calls() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    let ready = port.is_ready();
    let diagnostic = bridge.diagnostic();
    clean(&mut owner, &mut bridge, &provider).await;
    assert!(settled.is_ok());
    assert!(
        matches!(outcome.unwrap().outcome().unwrap().as_ref(),ExecutionOutcome::Failed(f) if f.error()==Some("Lambda.ResourceNotFoundException"))
    );
    assert!(diagnostic.is_some());
    assert!(ready, "historical diagnostic is not owner failure");
    assert_eq!(exec.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn test_should_ignore_callback_success_body_but_preserve_function_failures_and_actual_metadata() {
    use rustack_stepfunctions_core::CallbackInvocationMetadata;
    use uuid::Uuid;

    let id = Uuid::new_v4();
    for body in [
        Bytes::from_static(b"not-json"),
        Bytes::from_static(&[0xff]),
        Bytes::from(vec![b'x'; 262_145]),
    ] {
        let response = InvokeResponse::success(body, "42".into());
        assert!(
            mapping::response(&response, true, &id.to_string())
                .unwrap()
                .result
                .is_err()
        );
        let (mapped, metadata) =
            mapping::observed_response(&response, CallbackInvocationMetadata::new(id, "42"))
                .unwrap();
        let output = mapped.result.unwrap();
        assert!(output.value().get("Payload").is_none());
        assert_eq!(output.value()["ExecutedVersion"], "42");
        assert_eq!(
            output.value()["SdkResponseMetadata"]["RequestId"],
            id.to_string()
        );
        assert_eq!(metadata.unwrap().request_id(), id);
    }
    let mut response = InvokeResponse::success(
        Bytes::from_static(br#"{"errorType":"RealFailure"}"#),
        "$LATEST".into(),
    );
    response.function_error = Some("Unhandled".into());
    let (mapped, metadata) =
        mapping::observed_response(&response, CallbackInvocationMetadata::new(id, "$LATEST"))
            .unwrap();
    assert!(matches!(mapped.result,Err(f) if f.details().error()==Some("RealFailure")));
    assert!(metadata.is_none());
    response.function_error = None;
    let (mapped, metadata) =
        mapping::observed_response(&response, CallbackInvocationMetadata::new(id, "alias"))
            .unwrap();
    assert!(matches!(mapped.result,Err(f) if f.details().error()==Some("Lambda.ServiceException")));
    assert!(mapped.diagnostic.is_some());
    assert!(metadata.is_none());
}

#[rstest::rstest]
#[case(Value::Null, json!({}))]
#[case(json!("null"), Value::Null)]
#[case(json!("42"), json!(42))]
#[case(json!("\"hello\""), json!("hello"))]
#[case(json!("[1,true]"), json!([1,true]))]
#[case(json!({"number":7}), json!({"number":7}))]
#[tokio::test]
async fn test_should_decode_optimized_serialized_payload_without_double_encoding(
    #[case] payload: Value,
    #[case] expected: Value,
) {
    let exec = executor();
    let (provider, _directory) = provider(exec.clone());
    provider
        .create_function(create(&zip(b"fixture", "v1"), &[]))
        .await
        .unwrap();
    let (port, mut bridge) = StepFunctionsLambdaBridge::new(
        provider.clone(),
        TenantScope::default(),
        BridgeLimits::default(),
    )
    .unwrap();
    let mut owner = executions(port);
    let def = definition(
        "arn:aws:states:::lambda:invoke",
        Some(json!({"FunctionName":"Worker","Payload":payload})),
    );
    let mut handle = owner.start(driver(&def, &json!({}))).unwrap();
    let result = handle.wait(deadline()).await;
    clean(&mut owner, &mut bridge, &provider).await;
    assert_eq!(
        output(result.unwrap().outcome().unwrap()).get("Payload"),
        Some(&expected)
    );
    assert_eq!(exec.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_should_report_missing_optimized_function_as_started_service_failure() {
    let exec = executor();
    let (provider, _directory) = provider(exec.clone());
    let (port, mut bridge) = StepFunctionsLambdaBridge::new(
        provider.clone(),
        TenantScope::default(),
        BridgeLimits::default(),
    )
    .unwrap();
    let mut owner = executions(port);
    let def = definition(
        "arn:aws:states:::lambda:invoke",
        Some(json!({"FunctionName":"Worker"})),
    );
    let mut handle = owner.start(driver(&def, &json!({}))).unwrap();
    let result = handle.wait(deadline()).await;
    clean(&mut owner, &mut bridge, &provider).await;
    let result = result.unwrap();
    let page = result.history().page(1, 100, true).unwrap();
    assert_eq!(
        page.events()
            .iter()
            .map(|e| e.r#type.as_str())
            .collect::<Vec<_>>(),
        [
            "ExecutionStarted",
            "TaskStateEntered",
            "TaskScheduled",
            "TaskStarted",
            "TaskFailed",
            "ExecutionFailed"
        ]
    );
    assert_eq!(exec.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn test_should_compact_function_error_cause_without_changing_its_values() {
    let mut response = InvokeResponse::success(
        Bytes::from_static(br#"{ "errorType" : "Oops", "errorMessage" : "line\nvalue" }"#),
        "$LATEST".into(),
    );
    response.function_error = Some("Unhandled".into());
    let mapped = mapping::response(&response, false, "id").unwrap();
    let failure = mapped.result.unwrap_err();
    assert_eq!(
        failure.details().cause(),
        Some(r#"{"errorMessage":"line\nvalue","errorType":"Oops"}"#)
    );
}

#[test]
fn test_should_preserve_function_error_when_compact_numeric_cause_expands_past_payload_limit() {
    let mut raw = String::from(r#"{"errorType":"Oops","items":["#);
    raw.push_str(&vec!["1e9"; 40_000].join(","));
    raw.push_str("]}");
    assert!(raw.len() < 262_144);
    let mut response = InvokeResponse::success(Bytes::from(raw), "$LATEST".into());
    response.function_error = Some("Unhandled".into());
    let failure = mapping::response(&response, false, "id")
        .unwrap()
        .result
        .unwrap_err();
    assert_eq!(failure.details().error(), Some("Oops"));
    let cause = failure.details().cause().unwrap();
    assert_eq!(cause.len(), 32_768);
    assert!(cause.ends_with(" [truncated]"));
    assert!(cause.starts_with(r#"{"errorType":"Oops","items":[1000000000.0"#));
}

#[derive(Debug)]
struct FixedServiceId(Uuid);
impl UuidSource for FixedServiceId {
    fn next_uuid(&mut self) -> Result<Uuid, AslError> {
        Ok(self.0)
    }
}
#[derive(Debug)]
struct FailedServiceId;
impl UuidSource for FailedServiceId {
    fn next_uuid(&mut self) -> Result<Uuid, AslError> {
        Err(AslError::IntrinsicValue)
    }
}
fn service_plan(function: &str, qualifier: Option<&str>) -> mapping::CallPlan {
    mapping::CallPlan {
        function: function.into(),
        qualifier: qualifier.map(str::to_owned),
        payload: Payload::parse(b"{}").unwrap(),
        mode: mapping::ResponseMode::Optimized,
        deadline: deadline(),
    }
}

#[rstest::rstest]
#[case("Worker", None, "$LATEST")]
#[case("Worker", Some("alias"), "alias")]
#[case(
    "arn:aws-cn:lambda:cn-north-1:123456789012:function:Worker:2",
    None,
    "2"
)]
#[case(
    "arn:aws-cn:lambda:cn-north-1:123456789012:function:Worker:alias",
    Some("alias"),
    "alias"
)]
#[case(
    "arn:aws-cn:lambda:cn-north-1:123456789012:function:Worker",
    None,
    "$LATEST"
)]
fn test_should_render_missing_function_cause_from_validated_target_and_scope(
    #[case] function: &str,
    #[case] qualifier: Option<&str>,
    #[case] expected: &str,
) {
    let scope = TenantScope::new("aws-cn", "cn-north-1", "123456789012").unwrap();
    let target = mapping::ServiceTarget::new(&service_plan(function, qualifier), &scope);
    let id = Uuid::parse_str("5a09f4e4-fbdc-49eb-9a92-9194264473cf").unwrap();
    let mapped = mapping::service(
        LambdaServiceError::FunctionNotFound {
            name: "private-provider-name\n/host/path".into(),
        },
        &target,
        &mut FixedServiceId(id),
    )
    .unwrap();
    assert!(!format!("{target:?} {mapped:?}").contains("Worker"));
    let failure = mapped.result.unwrap_err();
    assert_eq!(
        failure.details().error(),
        Some("Lambda.ResourceNotFoundException")
    );
    assert_eq!(
        failure.details().cause(),
        Some(
            format!(
                "Function not found: \
                 arn:aws-cn:lambda:cn-north-1:123456789012:function:Worker:{expected} (Service: \
                 AWSLambda; Status Code: 404; Error Code: ResourceNotFoundException; Request ID: \
                 {id}; Proxy: null)"
            )
            .as_str()
        )
    );
}

#[test]
fn test_should_preserve_missing_version_alias_and_unrelated_service_error_categories() {
    let target = mapping::ServiceTarget::new(
        &service_plan("Worker", Some("missing")),
        &TenantScope::default(),
    );
    for error in [
        LambdaServiceError::VersionNotFound {
            function_name: "private".into(),
            version: "123".into(),
        },
        LambdaServiceError::AliasNotFound {
            function_name: "private".into(),
            alias: "private".into(),
        },
    ] {
        let failure = mapping::service(error, &target, &mut SystemUuidSource::default())
            .unwrap()
            .result
            .unwrap_err();
        assert!(failure.details().cause().unwrap().starts_with(
            "Function not found: arn:aws:lambda:us-east-1:000000000000:function:Worker:missing "
        ));
    }
    let mapped = mapping::service(
        LambdaServiceError::TooManyRequests,
        &target,
        &mut FailedServiceId,
    )
    .unwrap();
    assert!(
        matches!(mapped.result,Err(f) if f.details().error()==Some("Lambda.TooManyRequestsException") && f.details().cause()==Some("Lambda invocation admission failed"))
    );
}

#[test]
fn test_should_reject_invalid_service_targets_and_failed_or_invalid_identity_sources() {
    let id = Uuid::parse_str("5a09f4e4-fbdc-49eb-9a92-9194264473cf").unwrap();
    for (function, qualifier) in [
        (
            "arn:aws:lambda:us-west-2:000000000000:function:Worker",
            None,
        ),
        ("Worker", Some("bad\nqualifier")),
        (
            "arn:aws:lambda:us-east-1:000000000000:function:Worker:1",
            Some("2"),
        ),
    ] {
        let target = mapping::ServiceTarget::new(
            &service_plan(function, qualifier),
            &TenantScope::default(),
        );
        assert!(
            mapping::service(
                LambdaServiceError::FunctionNotFound {
                    name: "ignored".into()
                },
                &target,
                &mut FixedServiceId(id)
            )
            .is_err()
        );
    }
    let target =
        mapping::ServiceTarget::new(&service_plan("Worker", None), &TenantScope::default());
    assert!(
        mapping::service(
            LambdaServiceError::FunctionNotFound {
                name: "ignored".into()
            },
            &target,
            &mut FailedServiceId
        )
        .is_err()
    );
    for id in [
        Uuid::nil(),
        Uuid::from_u128(0x0000_0000_0000_4000_c000_0000_0000_0000),
    ] {
        assert!(
            mapping::service(
                LambdaServiceError::FunctionNotFound {
                    name: "ignored".into()
                },
                &target,
                &mut FixedServiceId(id)
            )
            .is_err()
        );
    }
    let mut causes = BTreeSet::new();
    for _ in 0..2 {
        let failure = mapping::service(
            LambdaServiceError::FunctionNotFound {
                name: "ignored".into(),
            },
            &target,
            &mut SystemUuidSource::default(),
        )
        .unwrap()
        .result
        .unwrap_err();
        assert!(causes.insert(failure.details().cause().unwrap().to_owned()));
    }
}
