//! Actual host process → Runtime API → ASL fixtures. Built by the Makefile gate.

use std::{
    collections::BTreeSet,
    env,
    error::Error,
    path::Path,
    process::{self, ExitStatus, Stdio},
    time::{Duration, SystemTime},
};

use http::{Request as HttpRequest, StatusCode};
use http_body_util::{BodyExt, Full};
use rustack_lambda_model::input::{
    CreateAliasInput, UpdateAliasInput, UpdateFunctionCodeInput, UpdateFunctionConfigurationInput,
};
use rustack_stepfunctions_core::{
    ChildPattern, ChildStartParameters, ExecutionSnapshot, PreparedDefinition, Registry,
    RegistryCommand, RegistryError, RegistryLimits, RegistryResult, StateMachineArn, StateSupport,
    Tags,
};
use rustack_stepfunctions_http::{RequestScope, StepFunctionsHttpConfig, StepFunctionsHttpService};
use tokio::{
    fs,
    io::AsyncReadExt,
    process::{Child, Command},
    time::{error::Elapsed, sleep, timeout, timeout_at},
};

use super::*;
use crate::stepfunctions_api::{ApiLimits, StepFunctionsApi};

#[derive(Debug)]
struct NativeFixture {
    provider: Arc<RustackLambda>,
    owner: ExecutionSupervisor,
    bridge: LambdaBridgeSupervisor,
    port: Arc<StepFunctionsLambdaBridge>,
    directory: Option<TempDir>,
    bootstrap: Bytes,
}
impl NativeFixture {
    async fn new(env: &[(&str, &str)], publish: bool) -> Self {
        let path = env::var_os("RUSTACK_WORKFLOWS_BOOTSTRAP")
            .expect("run make test-stepfunctions-bridge-native");
        let bootstrap = Bytes::from(fs::read(path).await.unwrap());
        let directory = tempfile::tempdir().unwrap();
        let config = LambdaConfig {
            executor: ExecutorBackend::Native,
            init_timeout: Duration::from_secs(5),
            max_warm_instances: 2,
            ..LambdaConfig::default()
        };
        let provider = Arc::new(RustackLambda::with_store(
            FunctionStore::new(directory.path()),
            config,
        ));
        let bytes = bootstrap.clone();
        let code = tokio::task::spawn_blocking(move || zip(&bytes, "native-v1"))
            .await
            .unwrap();
        let marker = directory.path().join("invocation.json");
        let mut variables = env.to_vec();
        variables.push(("INVOCATION_FILE", marker.to_str().unwrap()));
        let mut input = create(&code, &variables);
        input.publish = Some(publish);
        provider.create_function(input).await.unwrap();
        let (port, bridge) = StepFunctionsLambdaBridge::new(
            provider.clone(),
            TenantScope::default(),
            BridgeLimits::default(),
        )
        .unwrap();
        Self {
            provider,
            owner: executions(port.clone()),
            port,
            bridge,
            directory: Some(directory),
            bootstrap,
        }
    }
    async fn ready_after_calls(&self) -> bool {
        timeout_at(deadline(), async {
            while self.bridge.owned_calls() != 0 {
                sleep(Duration::from_millis(1)).await;
            }
            self.port.is_ready()
        })
        .await
        .unwrap_or(false)
    }
    async fn cleanup(&mut self) {
        self.cleanup_at(Instant::now() + Duration::from_secs(15))
            .await;
    }
    async fn cleanup_at(&mut self, absolute: Instant) {
        let workflow = self.owner.drain(absolute).await;
        let bridge = self.bridge.shutdown(absolute).await;
        let provider = self
            .provider
            .quiesce(absolute.saturating_duration_since(Instant::now()))
            .await;
        let processes = timeout_at(absolute, self.provider.executor().shutdown()).await;
        if workflow.is_err()
            || bridge.is_err()
            || provider.is_err()
            || !matches!(&processes, Ok(Ok(())))
        {
            // Unproven process retirement must not remove its still-owned files.
            if let Some(directory) = self.directory.take() {
                let _retained = directory.keep();
            }
        }
        assert!(workflow.is_ok(), "workflow cleanup: {workflow:?}");
        assert!(bridge.is_ok(), "bridge cleanup: {bridge:?}");
        assert!(provider.is_ok(), "provider cleanup: {provider:?}");
        assert!(
            matches!(processes, Ok(Ok(()))),
            "native process cleanup: {processes:?}"
        );
    }
}
fn workflow(optimized: bool) -> Value {
    let parameters = json!({"n.$":"$.n","retryCount.$":"$$.State.RetryCount"});
    if optimized {
        definition(
            "arn:aws:states:::lambda:invoke",
            Some(json!({"FunctionName":"Worker","Payload":parameters})),
        )
    } else {
        definition(
            "arn:aws:lambda:us-east-1:000000000000:function:Worker",
            Some(parameters),
        )
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the actual host bootstrap; run make test-stepfunctions-bridge-native"]
async fn native_e2e_should_execute_computation_and_return_actual_invocation_metadata() {
    for optimized in [false, true] {
        let mut fixture = NativeFixture::new(&[("WORKFLOW_MODE", "1")], false).await;
        let mut handle = fixture
            .owner
            .start(driver(&workflow(optimized), &json!({"n":21})))
            .unwrap();
        let result = handle.wait(deadline()).await;
        fixture.cleanup().await;
        let view = result.unwrap();
        let response = output(view.outcome().unwrap());
        let body = if optimized {
            response.get("Payload").unwrap()
        } else {
            response
        };
        assert_eq!(body.get("computed"), Some(&json!(42)));
        assert_eq!(body.get("codeMarker"), Some(&json!("native-v1")));
        assert_ne!(
            body.get("pid").and_then(Value::as_u64).unwrap(),
            u64::from(process::id())
        );
        let request_id = body.get("request_id").and_then(Value::as_str).unwrap();
        assert_eq!(request_id.len(), 36);
        if optimized {
            assert_eq!(
                response
                    .pointer("/SdkResponseMetadata/RequestId")
                    .and_then(Value::as_str),
                Some(request_id)
            );
            assert_eq!(response.get("StatusCode"), Some(&json!(200)));
        }
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the actual host bootstrap; run make test-stepfunctions-bridge-native"]
async fn native_e2e_should_retry_function_error_and_recalculate_parameters() {
    let mut fixture = NativeFixture::new(
        &[("WORKFLOW_MODE", "1"), ("WORKFLOW_FAIL_ONCE", "1")],
        false,
    )
    .await;
    let mut definition = workflow(false);
    definition["States"]["Call"]["Retry"] =
        json!([{"ErrorEquals":["WorkflowTransient"],"MaxAttempts":1,"IntervalSeconds":1}]);
    let mut handle = fixture
        .owner
        .start(driver(&definition, &json!({"n":6})))
        .unwrap();
    let result = handle.wait(deadline()).await;
    fixture.cleanup().await;
    let view = result.unwrap();
    let response = output(view.outcome().unwrap());
    assert_eq!(response.get("computed"), Some(&json!(12)));
    assert_eq!(response.get("retryCount"), Some(&json!(1)));
    let history = view.history().page(1, 100, true).unwrap();
    let kinds = history
        .events()
        .iter()
        .map(|e| e.r#type.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        kinds.iter().filter(|&&k| k == "TaskStateEntered").count(),
        1
    );
    assert_eq!(
        kinds
            .iter()
            .filter(|&&k| k == "LambdaFunctionStarted")
            .count(),
        2
    );
    assert_eq!(
        kinds
            .iter()
            .filter(|&&k| k == "LambdaFunctionFailed")
            .count(),
        1
    );
    assert_eq!(kinds.last().copied(), Some("ExecutionSucceeded"));
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the actual host bootstrap; run make test-stepfunctions-bridge-native"]
async fn native_e2e_should_resolve_alias_again_on_retry_and_capture_new_version() {
    let mut fixture =
        NativeFixture::new(&[("WORKFLOW_MODE", "1"), ("WORKFLOW_FAIL_ONCE", "1")], true).await;
    let bytes = fixture.bootstrap.clone();
    let code = tokio::task::spawn_blocking(move || zip(&bytes, "native-v2"))
        .await
        .unwrap();
    fixture
        .provider
        .update_function_code(
            "Worker",
            UpdateFunctionCodeInput {
                zip_file: Some(STANDARD.encode(code)),
                publish: Some(true),
                ..UpdateFunctionCodeInput::default()
            },
        )
        .await
        .unwrap();
    fixture
        .provider
        .create_alias(
            "Worker",
            CreateAliasInput {
                name: "live".into(),
                function_version: "1".into(),
                ..CreateAliasInput::default()
            },
        )
        .unwrap();
    let mut definition = workflow(true);
    definition["States"]["Call"]["Parameters"]["Qualifier"] = json!("live");
    definition["States"]["Call"]["Retry"] =
        json!([{"ErrorEquals":["WorkflowTransient"],"MaxAttempts":1,"IntervalSeconds":1}]);
    let mut handle = fixture
        .owner
        .start(driver(&definition, &json!({"n":5})))
        .unwrap();
    let failed = timeout_at(deadline(), async {
        loop {
            let view = handle.snapshot();
            let history = view.history().page(1, 100, true).unwrap();
            if history
                .events()
                .iter()
                .any(|e| e.r#type.as_str() == "TaskFailed")
            {
                break;
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    let updated = fixture.provider.update_alias(
        "Worker",
        "live",
        &UpdateAliasInput {
            function_version: Some("2".into()),
            ..UpdateAliasInput::default()
        },
    );
    let result = handle.wait(deadline()).await;
    fixture.cleanup().await;
    failed.unwrap();
    updated.unwrap();
    let view = result.unwrap();
    let response = output(view.outcome().unwrap());
    assert_eq!(response.get("ExecutedVersion"), Some(&json!("2")));
    assert_eq!(
        response.pointer("/Payload/codeMarker"),
        Some(&json!("native-v2"))
    );
    assert_eq!(response.pointer("/Payload/computed"), Some(&json!(10)));
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the actual host bootstrap; run make test-stepfunctions-bridge-native"]
async fn native_e2e_should_catch_actual_function_error() {
    let mut fixture = NativeFixture::new(&[("FAIL_MODE", "panic")], false).await;
    let mut definition = direct();
    definition["States"]["Call"]["Catch"] =
        json!([{"ErrorEquals":["TestError"],"ResultPath":"$.failure","Next":"Recovered"}]);
    definition["States"]["Recovered"] = json!({"Type":"Succeed"});
    let mut handle = fixture
        .owner
        .start(driver(&definition, &json!({"keep":7})))
        .unwrap();
    let result = handle.wait(deadline()).await;
    fixture.cleanup().await;
    let view = result.unwrap();
    let response = output(view.outcome().unwrap());
    assert_eq!(response.get("keep"), Some(&json!(7)));
    assert_eq!(
        response.pointer("/failure/Error"),
        Some(&json!("TestError"))
    );
    assert!(
        response
            .pointer("/failure/Cause")
            .and_then(Value::as_str)
            .unwrap()
            .contains("boom")
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the actual host bootstrap; run make test-stepfunctions-bridge-native"]
async fn native_e2e_should_cancel_actual_process_on_stop_and_task_timeout() {
    for stop in [false, true] {
        let mut fixture = NativeFixture::new(&[("SLEEP_SECS", "5")], false).await;
        let mut definition = direct();
        definition["States"]["Call"]["TimeoutSeconds"] = json!(1);
        let mut handle = fixture
            .owner
            .start(driver(&definition, &json!({})))
            .unwrap();
        let marker = fixture
            .directory
            .as_ref()
            .unwrap()
            .path()
            .join("invocation.json");
        let started = timeout_at(deadline(), async {
            loop {
                match fs::read_to_string(&marker).await {
                    Ok(body) => match serde_json::from_str::<Value>(&body) {
                        Ok(marker) => break Ok(marker),
                        Err(_) => sleep(Duration::from_millis(5)).await,
                    },
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        sleep(Duration::from_millis(5)).await;
                    }
                    Err(error) => break Err(error),
                }
            }
        })
        .await;
        let result = if stop {
            handle
                .stop(TaskFailure::new("Stopped", None).unwrap(), deadline())
                .await
        } else {
            handle.wait(deadline()).await
        };
        let ready = fixture.ready_after_calls().await;
        fixture.cleanup().await;
        assert!(ready, "settled Task timeout/Stop poisoned bridge readiness");
        let marker = started.unwrap().unwrap();
        let pid = marker.get("pid").and_then(Value::as_u64).unwrap();
        assert!(pid > 1 && pid != u64::from(process::id()));
        assert_eq!(
            marker
                .get("request_id")
                .and_then(Value::as_str)
                .unwrap()
                .len(),
            36
        );
        verify_absent_probe(pid).await.unwrap();
        let view = result.unwrap();
        if stop {
            assert!(matches!(
                view.outcome().unwrap().as_ref(),
                ExecutionOutcome::Aborted(_)
            ));
        } else {
            assert!(
                matches!(view.outcome().unwrap().as_ref(),ExecutionOutcome::Failed(f) if f.error()==Some("States.Timeout"))
            );
        }
        assert_eq!(
            handle.snapshot().cleanup(),
            rustack_stepfunctions_core::CleanupState::Clean
        );
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the actual host bootstrap; run make test-stepfunctions-bridge-native"]
async fn native_e2e_should_preserve_optimized_payload_json_types_and_default() {
    let mut fixture = NativeFixture::new(&[], false).await;
    let mut results = Vec::new();
    for payload in [
        None,
        Some(Value::Null),
        Some(json!("json-string")),
        Some(json!([1, 2])),
    ] {
        let mut params = json!({"FunctionName":"Worker"});
        if let Some(payload) = &payload {
            params["Payload"] = payload.clone();
        }
        let definition = definition("arn:aws:states:::lambda:invoke", Some(params));
        let result = match fixture
            .owner
            .start(driver(&definition, &json!({"notForwarded":true})))
        {
            Ok(mut handle) => handle.wait(deadline()).await,
            Err(error) => Err(error),
        };
        let failed = result.is_err();
        results.push((payload, result));
        if failed {
            break;
        }
    }
    fixture.cleanup().await;
    assert_eq!(results.len(), 4);
    for (payload, result) in results {
        let view = result.unwrap();
        assert_eq!(
            output(view.outcome().unwrap()).pointer("/Payload/echo"),
            Some(&payload.unwrap_or_else(|| json!({})))
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the actual host bootstrap; run make test-stepfunctions-bridge-native"]
async fn native_e2e_should_preserve_function_timeout_class_for_asl_catch() {
    let mut fixture = NativeFixture::new(&[("SLEEP_SECS", "5")], false).await;
    fixture
        .provider
        .update_function_configuration(
            "Worker",
            &UpdateFunctionConfigurationInput {
                timeout: Some(1),
                ..UpdateFunctionConfigurationInput::default()
            },
        )
        .unwrap();
    let mut definition = direct();
    definition["States"]["Call"]["Catch"] =
        json!([{"ErrorEquals":["Sandbox.Timedout"],"Next":"Recovered"}]);
    definition["States"]["Recovered"] = json!({"Type":"Succeed"});
    let mut handle = fixture
        .owner
        .start(driver(&definition, &json!({})))
        .unwrap();
    let result = handle.wait(deadline()).await;
    let ready = fixture.ready_after_calls().await;
    fixture.cleanup().await;
    assert!(ready, "settled function timeout poisoned bridge readiness");
    let view = result.unwrap();
    assert_eq!(
        output(view.outcome().unwrap()).get("Error"),
        Some(&json!("Sandbox.Timedout"))
    );
}

async fn run_registry_case(
    registry: &Registry,
) -> Result<(ExecutionSnapshot, ExecutionSnapshot, ExecutionSnapshot), Box<dyn Error>> {
    let compiler = AslCompiler::new(TenantScope::default(), TaskSupport::Lambda);
    let prepared = PreparedDefinition::new(
        compiler
            .compile(&serde_json::to_vec(&workflow(true))?)?
            .machine()
            .clone(),
    );
    let mut create = registry.submit(RegistryCommand::Create(
        "NativeWorkflow".parse()?,
        prepared,
        "arn:aws:iam::000000000000:role/Workflow".parse()?,
        Tags::default(),
    ))?;
    let RegistryResult::Machine(machine) = create.wait(deadline()).await? else {
        return Err(RegistryError::Unavailable.into());
    };
    let mut start = registry.submit(RegistryCommand::Start(
        machine.arn().clone(),
        Some("first".parse()?),
        Payload::parse(br#"{"n":21}"#)?,
        None,
    ))?;
    let RegistryResult::Execution(first) = start.wait(deadline()).await? else {
        return Err(RegistryError::Unavailable.into());
    };
    first.handle().wait(deadline()).await?;
    let mut updated = workflow(true);
    updated["States"]["Call"]["Parameters"]["Payload"] = json!({"n":22,"retryCount":0});
    let prepared = PreparedDefinition::new(
        compiler
            .compile(&serde_json::to_vec(&updated)?)?
            .machine()
            .clone(),
    );
    let mut update = registry.submit(RegistryCommand::Update(
        machine.arn().clone(),
        Some(prepared),
        None,
    ))?;
    update.wait(deadline()).await?;
    let mut start = registry.submit(RegistryCommand::Start(
        machine.arn().clone(),
        Some("second".parse()?),
        Payload::parse(br#"{"n":21}"#)?,
        None,
    ))?;
    let RegistryResult::Execution(second) = start.wait(deadline()).await? else {
        return Err(RegistryError::Unavailable.into());
    };
    second.handle().wait(deadline()).await?;
    let mut delete = registry.submit(RegistryCommand::Delete(machine.arn().clone()))?;
    delete.wait(deadline()).await?;
    let mut read = registry.submit(RegistryCommand::Execution(first.arn().clone()))?;
    let RegistryResult::Execution(first_final) = read.wait(deadline()).await? else {
        return Err(RegistryError::Unavailable.into());
    };
    let mut read = registry.submit(RegistryCommand::Execution(second.arn().clone()))?;
    let RegistryResult::Execution(second_final) = read.wait(deadline()).await? else {
        return Err(RegistryError::Unavailable.into());
    };
    Ok((first, first_final, second_final))
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the actual host bootstrap; run make test-stepfunctions-bridge-native"]
async fn native_e2e_should_execute_captured_registry_revisions_and_read_after_machine_deletion() {
    let mut fixture = NativeFixture::new(&[("WORKFLOW_MODE", "1")], false).await;
    let (port, mut bridge) = StepFunctionsLambdaBridge::new(
        fixture.provider.clone(),
        TenantScope::default(),
        BridgeLimits::default(),
    )
    .unwrap();
    let (registry, mut owner) = Registry::new(
        TenantScope::default(),
        RegistryLimits::default(),
        executions(port),
        ExecutionLimits::default(),
    )
    .unwrap();
    let result = run_registry_case(&registry).await;
    let absolute = Instant::now() + Duration::from_secs(15);
    let workflow = owner.shutdown(absolute).await;
    let calls = bridge.shutdown(absolute).await;
    if workflow.is_err() || calls.is_err() {
        if let Some(directory) = fixture.directory.take() {
            let _retained = directory.keep();
        }
    }
    fixture.cleanup().await;
    assert!(workflow.is_ok(), "registry cleanup: {workflow:?}");
    assert!(calls.is_ok(), "bridge cleanup: {calls:?}");
    let (first, captured, second) = result.unwrap();
    assert_eq!(first.revision().id(), captured.revision().id());
    assert_ne!(captured.revision().id(), second.revision().id());
    let first_result = output(captured.view().outcome().unwrap());
    let second_result = output(second.view().outcome().unwrap());
    assert_eq!(first_result.pointer("/Payload/computed"), Some(&json!(42)));
    assert_eq!(second_result.pointer("/Payload/computed"), Some(&json!(44)));
    for response in [first_result, second_result] {
        let actual = response
            .pointer("/Payload/request_id")
            .and_then(Value::as_str)
            .unwrap();
        assert_eq!(actual.len(), 36);
        assert_eq!(
            response
                .pointer("/SdkResponseMetadata/RequestId")
                .and_then(Value::as_str),
            Some(actual)
        );
    }
    assert_ne!(
        first_result.pointer("/Payload/request_id"),
        second_result.pointer("/Payload/request_id")
    );
}

async fn api_json(
    service: &StepFunctionsHttpService,
    action: &str,
    input: Value,
) -> Result<Value, Box<dyn Error>> {
    let request = HttpRequest::builder()
        .method("POST")
        .uri("/")
        .header("content-type", "application/x-amz-json-1.0")
        .header("x-amz-target", format!("AWSStepFunctions.{action}"))
        .body(Full::new(Bytes::from(serde_json::to_vec(&input)?)))?;
    let response = service.handle(request).await;
    let status = response.status();
    let body = response.into_body().collect().await?.to_bytes();
    let value = serde_json::from_slice(&body)?;
    if status != StatusCode::OK {
        return Err(RegistryError::Unavailable.into());
    }
    Ok(value)
}
async fn run_http_registry_case(
    service: &StepFunctionsHttpService,
) -> Result<(Value, Value, Value), Box<dyn Error>> {
    let created=api_json(service,"CreateStateMachine",json!({"name":"HttpNative","roleArn":"arn:aws:iam::000000000000:role/Workflow","definition":workflow(true).to_string()})).await?;
    let started=api_json(service,"StartExecution",json!({"stateMachineArn":created["stateMachineArn"],"name":"computed","input":"{\"n\":21}"})).await?;
    let absolute = deadline();
    let description = loop {
        let description = api_json(
            service,
            "DescribeExecution",
            json!({"executionArn":started["executionArn"]}),
        )
        .await?;
        if description["status"] != "RUNNING" {
            break description;
        }
        if Instant::now() >= absolute {
            return Err(RegistryError::Deadline.into());
        }
        sleep(Duration::from_millis(10)).await;
    };
    let history=api_json(service,"GetExecutionHistory",json!({"executionArn":started["executionArn"],"reverseOrder":true,"includeExecutionData":false,"maxResults":1})).await?;
    api_json(
        service,
        "DeleteStateMachine",
        json!({"stateMachineArn":created["stateMachineArn"]}),
    )
    .await?;
    let next=api_json(service,"GetExecutionHistory",json!({"executionArn":started["executionArn"],"reverseOrder":true,"includeExecutionData":false,"maxResults":1,"nextToken":history["nextToken"]})).await?;
    Ok((description, history, next))
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the actual host bootstrap; run make test-stepfunctions-bridge-native"]
async fn native_e2e_should_execute_real_lambda_through_typed_http_api_and_paginated_history() {
    let mut fixture = NativeFixture::new(&[("WORKFLOW_MODE", "1")], false).await;
    let (port, mut bridge) = StepFunctionsLambdaBridge::new(
        fixture.provider.clone(),
        TenantScope::default(),
        BridgeLimits::default(),
    )
    .unwrap();
    let (registry, mut owner) = Registry::new(
        TenantScope::default(),
        RegistryLimits::default(),
        executions(port),
        ExecutionLimits::default(),
    )
    .unwrap();
    let (handler, mut api) = StepFunctionsApi::new(
        TenantScope::default(),
        TaskSupport::Lambda,
        registry,
        ApiLimits::default(),
    )
    .unwrap();
    let config = StepFunctionsHttpConfig::new(
        RequestScope::new("aws".into(), "us-east-1".into(), "000000000000".into()).unwrap(),
        true,
        None,
    )
    .unwrap();
    let service = StepFunctionsHttpService::new(handler, config);
    let result = run_http_registry_case(&service).await;
    let absolute = Instant::now() + Duration::from_secs(15);
    let requests = api.shutdown(absolute).await;
    let workflows = owner.shutdown(absolute).await;
    let calls = bridge.shutdown(absolute).await;
    if requests.is_err() || workflows.is_err() || calls.is_err() {
        if let Some(directory) = fixture.directory.take() {
            let _retained = directory.keep();
        }
    }
    fixture.cleanup_at(absolute).await;
    assert!(requests.is_ok(), "API cleanup: {requests:?}");
    assert!(workflows.is_ok(), "registry cleanup: {workflows:?}");
    assert!(calls.is_ok(), "bridge cleanup: {calls:?}");
    let (description, history, next) = result.unwrap();
    assert_eq!(description["status"], "SUCCEEDED");
    let result: Value = serde_json::from_str(description["output"].as_str().unwrap()).unwrap();
    assert_eq!(result["Payload"]["computed"], 42);
    assert_eq!(
        result["Payload"]["request_id"],
        result["SdkResponseMetadata"]["RequestId"]
    );
    assert_eq!(result["Payload"]["request_id"].as_str().unwrap().len(), 36);
    assert_eq!(history["events"][0]["type"], "ExecutionSucceeded");
    assert!(
        history["events"][0]["executionSucceededEventDetails"]
            .get("output")
            .is_none()
    );
    assert!(
        next["events"][0]["id"].as_i64().unwrap() < history["events"][0]["id"].as_i64().unwrap()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires actual host bootstrap; run make test-stepfunctions-bridge-native"]
async fn native_e2e_should_execute_overlapping_parallel_functions_through_structured_owner()
-> Result<(), Box<dyn Error>> {
    let gate = tempfile::tempdir()?;
    let release = gate.path().join("release");
    let release_text = release.to_str().ok_or("invalid gate path")?;
    let consumed = gate.path().join("consumed");
    fs::create_dir(&consumed).await?;
    let consumed_text = consumed.to_str().ok_or("invalid consumed marker path")?;
    let mut fixture = NativeFixture::new(
        &[
            ("WORKFLOW_MODE", "1"),
            ("WORKFLOW_GATE_FILE", release_text),
            ("WORKFLOW_INVOCATION_DIR", consumed_text),
        ],
        false,
    )
    .await;
    let result = async {
        let definition = json!({
            "StartAt": "Parallel",
            "States": {"Parallel": {
                "Type": "Parallel",
                "Branches": [workflow(false), workflow(false)],
                "End": true
            }}
        });
        let compiled = AslCompiler::new(TenantScope::default(), TaskSupport::Lambda)
            .with_state_support(StateSupport::Structured)
            .compile(&serde_json::to_vec(&definition)?)?;
        let input = Payload::from_value(&json!({"n": 21}))?;
        let driver = ExecutionDriver::for_structured_owner(
            compiled.machine().clone(),
            input.clone(),
            EvaluationContext::from(&input),
            ClockSample::new(Instant::now().into_std(), SystemTime::now().into()),
            ExecutionLimits::default(),
        )?;
        let mut handle = fixture.owner.start_structured(driver)?;
        let markers = timeout_at(deadline(), consumed_invocations(&consumed)).await??;
        fs::write(&release, b"release").await?;
        let view = handle.wait(deadline()).await?;
        Ok::<_, Box<dyn Error>>((view, markers))
    }
    .await;
    let released = fs::write(&release, b"release").await;
    fixture.cleanup().await;
    released?;
    let (view, markers) = result?;
    let values = output(view.outcome().ok_or("outcome")?)
        .as_array()
        .ok_or("expected ordered array")?;
    assert_eq!(values.len(), 2);
    let returned: BTreeSet<_> = values
        .iter()
        .map(|value| {
            Ok((
                value
                    .get("pid")
                    .and_then(Value::as_u64)
                    .ok_or("missing PID")?,
                value
                    .get("request_id")
                    .and_then(Value::as_str)
                    .ok_or("missing request ID")?
                    .to_owned(),
            ))
        })
        .collect::<Result<_, Box<dyn Error>>>()?;
    assert_eq!(returned, markers);
    for value in values {
        assert_eq!(value.get("computed"), Some(&json!(42)));
        assert_eq!(value.get("codeMarker"), Some(&json!("native-v1")));
        assert_ne!(
            value.get("pid").and_then(Value::as_u64),
            Some(u64::from(process::id()))
        );
    }
    assert_ne!(
        values.first().and_then(|v| v.get("request_id")),
        values.get(1).and_then(|v| v.get("request_id"))
    );
    Ok(())
}

async fn consumed_invocations(directory: &Path) -> Result<BTreeSet<(u64, String)>, Box<dyn Error>> {
    loop {
        let mut entries = fs::read_dir(directory).await?;
        let mut markers = BTreeSet::new();
        let mut count = 0;
        while let Some(entry) = entries.next_entry().await? {
            if entry
                .path()
                .extension()
                .is_none_or(|extension| extension != "json")
            {
                continue;
            }
            count += 1;
            if count > 2 || entry.metadata().await?.len() > 1024 {
                return Err("unexpected consumed marker count or size".into());
            }
            let marker: Value = serde_json::from_slice(&fs::read(entry.path()).await?)?;
            let pid = marker
                .get("pid")
                .and_then(Value::as_u64)
                .filter(|pid| *pid > 0)
                .ok_or("invalid consumed PID")?;
            let request_id = marker
                .get("request_id")
                .and_then(Value::as_str)
                .filter(|id| id.len() == 36)
                .ok_or("invalid consumed request ID")?;
            markers.insert((pid, request_id.to_owned()));
        }
        if markers.len() == 2 {
            let pids: BTreeSet<_> = markers.iter().map(|(pid, _)| *pid).collect();
            let requests: BTreeSet<_> = markers.iter().map(|(_, request)| request).collect();
            if pids.len() != 2 || requests.len() != 2 {
                return Err("handlers did not consume distinct concurrent invocations".into());
            }
            return Ok(markers);
        }
        sleep(Duration::from_millis(1)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires actual host bootstrap; run make test-stepfunctions-bridge-native"]
async fn native_e2e_should_execute_and_cancel_real_child_through_atomic_registry_port()
-> Result<(), Box<dyn Error>> {
    for cancel in [false, true] {
        let directory = tempfile::tempdir()?;
        let release = directory.path().join("release");
        let mut fixture = NativeFixture::new(
            &[
                ("WORKFLOW_MODE", "1"),
                ("WORKFLOW_GATE_FILE", release.to_str().ok_or("gate path")?),
            ],
            false,
        )
        .await;
        let runtime = ExecutionSupervisor::new(
            RuntimeLimits::new(8, 1, 4)?,
            HistoryLimits::default(),
            256 * 1024 * 1024,
            Some(fixture.port.clone()),
        )?;
        let (registry, mut owner) = Registry::new(
            TenantScope::default(),
            RegistryLimits::default(),
            runtime,
            ExecutionLimits::default(),
        )?;
        let result = match timeout(
            Duration::from_secs(30),
            run_native_child_port(&fixture, &registry, &release, cancel),
        )
        .await
        {
            Ok(Ok(Some(pid))) => verify_absent_probe(pid).await,
            Ok(Ok(None)) => Ok(()),
            Ok(Err(error)) => Err(error),
            Err(error) => Err(Box::<dyn Error>::from(error)),
        };
        let cleanup = owner
            .shutdown(Instant::now() + Duration::from_secs(15))
            .await;
        let released = fs::write(&release, b"release").await;
        fixture.cleanup().await;
        cleanup?;
        released?;
        result?;
    }
    Ok(())
}
async fn child_port_machine(
    registry: &Registry,
    name: &str,
    definition: &Value,
) -> Result<StateMachineArn, Box<dyn Error>> {
    let compiled = AslCompiler::new(TenantScope::default(), TaskSupport::Lambda)
        .compile(&serde_json::to_vec(definition)?)?;
    let result = registry
        .submit(RegistryCommand::Create(
            name.parse()?,
            PreparedDefinition::new(compiled.machine().clone()),
            "arn:aws:iam::000000000000:role/Workflows".parse()?,
            Tags::default(),
        ))?
        .wait(deadline())
        .await?;
    match result {
        RegistryResult::Machine(machine) => Ok(machine.arn().clone()),
        _ => Err("machine".into()),
    }
}
async fn run_native_child_port(
    fixture: &NativeFixture,
    registry: &Registry,
    release: &Path,
    cancel: bool,
) -> Result<Option<u64>, Box<dyn Error>> {
    fixture.provider.update_function_configuration(
        "Worker",
        &UpdateFunctionConfigurationInput {
            timeout: Some(900),
            ..UpdateFunctionConfigurationInput::default()
        },
    )?;
    let parent = child_port_machine(
        registry,
        "PortParent",
        &json!({"StartAt":"W","States":{"W":{"Type":"Wait","Seconds":3600,"End":true}}}),
    )
    .await?;
    let target = child_port_machine(registry, "PortChild", &workflow(false)).await?;
    let RegistryResult::Execution(parent) = registry
        .submit(RegistryCommand::Start(
            parent,
            Some("parent".parse()?),
            Payload::parse(b"{}")?,
            None,
        ))?
        .wait(deadline())
        .await?
    else {
        return Err("parent".into());
    };
    let parameters = ChildStartParameters::parse(
        &TenantScope::default(),
        &Payload::from_value(&json!({"StateMachineArn":target.as_str(),"Input":{"n":21}}))?,
    )?;
    let mut submission = registry.child_port().start(
        parent.child_parent(),
        parameters,
        ChildPattern::Sync2,
        Instant::now() + Duration::from_secs(300),
    )?;
    submission.admitted(deadline()).await?;
    let mut child = submission.into_execution()?;
    let marker = child_port_marker(fixture).await?;
    let pid = marker
        .get("pid")
        .and_then(Value::as_u64)
        .filter(|pid| *pid > 1 && *pid != u64::from(process::id()))
        .ok_or("handler PID")?;
    let request = marker
        .get("request_id")
        .and_then(Value::as_str)
        .filter(|id| id.len() == 36)
        .ok_or("request ID")?;
    if child.snapshot().handle().snapshot().outcome().is_some() {
        return Err("child completed before consumed gate release".into());
    }
    if cancel {
        child.cancel();
    } else {
        fs::write(release, b"release").await?;
    }
    child.wait(deadline()).await?;
    child.wait_cleanup(deadline()).await?;
    let result = child.result()?;
    if cancel {
        let failure = result.err().ok_or("cancelled child succeeded")?;
        if failure.details().error() != Some("States.TaskFailed") {
            return Err("child cancellation error class".into());
        }
        let cause: Value = serde_json::from_str(failure.details().cause().ok_or("cause")?)?;
        if cause.get("Status").and_then(Value::as_str) != Some("ABORTED") {
            return Err("child was not aborted".into());
        }
        return Ok(Some(pid));
    }
    let result = result.map_err(|_| "child failed")?;
    let output = result.value().get("Output").ok_or("Output")?;
    if output.get("computed") != Some(&json!(42))
        || output.get("pid").and_then(Value::as_u64) != Some(pid)
        || output.get("request_id").and_then(Value::as_str) != Some(request)
    {
        return Err("child result did not match consumed Native invocation".into());
    }
    Ok(None)
}

async fn child_port_marker(fixture: &NativeFixture) -> Result<Value, Box<dyn Error>> {
    let marker = fixture
        .directory
        .as_ref()
        .ok_or("directory")?
        .path()
        .join("invocation.json");
    timeout_at(deadline(), async {
        loop {
            match fs::read(&marker).await {
                Ok(bytes) => {
                    if bytes.len() > 1024 {
                        return Err("marker size".into());
                    }
                    if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                        return Ok::<_, Box<dyn Error>>(value);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await?
}

async fn verify_absent_probe(pid: u64) -> Result<(), Box<dyn Error>> {
    let child = Command::new("/bin/kill")
        .arg("-0")
        .arg(pid.to_string())
        .env_clear()
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let (mut child, status) = reap_probe(child, deadline()).await?;
    let stderr = child.stderr.take().ok_or("probe stderr")?;
    let mut text = Vec::new();
    timeout_at(deadline(), stderr.take(1025).read_to_end(&mut text)).await??;
    if status.success()
        || text.len() > 1024
        || !String::from_utf8_lossy(&text).contains("No such process")
    {
        return Err("owned cancellation did not retire consumed PID before global cleanup".into());
    }
    Ok(())
}

async fn reap_probe(
    mut child: Child,
    until: Instant,
) -> Result<(Child, ExitStatus), Box<dyn Error>> {
    let status = match timeout_at(until, child.wait()).await {
        Ok(Ok(status)) => status,
        observed => {
            let saved: Box<dyn Error> = match observed {
                Ok(Err(error)) => Box::new(error),
                Err(error) => Box::new(error),
                Ok(Ok(_)) => return Err("probe wait invariant".into()),
            };
            let killed = child.start_kill();
            match timeout_at(deadline(), child.wait()).await {
                Ok(Ok(_)) => {}
                failed => {
                    // Unproven helper retirement retains the exact process handle.
                    let _retained = Box::leak(Box::new(child));
                    return Err(format!("PID probe retirement unproven: {failed:?}").into());
                }
            }
            killed?;
            return Err(saved);
        }
    };
    if Instant::now() >= until {
        return Err("PID probe observation expired after joined exit".into());
    }
    Ok((child, status))
}

#[tokio::test]
#[ignore = "requires Unix process tools; run make test-stepfunctions-bridge-native"]
async fn native_e2e_should_kill_and_reap_timed_out_owned_probe_before_returning_error()
-> Result<(), Box<dyn Error>> {
    let child = Command::new("/bin/sleep")
        .arg("60")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let pid = child.id().ok_or("owned probe PID")?;
    let result = reap_probe(child, Instant::now() + Duration::from_millis(10)).await;
    verify_absent_probe(u64::from(pid)).await?;
    let error = result
        .err()
        .ok_or("slow probe did not report its observation timeout")?;
    if error.downcast_ref::<Elapsed>().is_none() {
        return Err("probe failed without observing its execution timeout".into());
    }
    Ok(())
}
