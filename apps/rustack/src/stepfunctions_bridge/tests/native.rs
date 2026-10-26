//! Actual host process → Runtime API → ASL fixtures. Built by the Makefile gate.

use std::{env, process, time::Duration};

use rustack_lambda_model::input::{
    CreateAliasInput, UpdateAliasInput, UpdateFunctionCodeInput, UpdateFunctionConfigurationInput,
};
use tokio::{
    fs,
    time::{sleep, timeout_at},
};

use super::*;

#[derive(Debug)]
struct NativeFixture {
    provider: Arc<RustackLambda>,
    owner: ExecutionSupervisor,
    bridge: LambdaBridgeSupervisor,
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
            owner: executions(port),
            bridge,
            directory: Some(directory),
            bootstrap,
        }
    }
    async fn cleanup(&mut self) {
        let absolute = Instant::now() + Duration::from_secs(15);
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
        fixture.cleanup().await;
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
        let probe = tokio::process::Command::new("/bin/kill")
            .arg("-0")
            .arg(pid.to_string())
            .output()
            .await
            .unwrap();
        assert!(
            !probe.status.success(),
            "actual Native PID remains present after joined cleanup"
        );
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
    fixture.cleanup().await;
    let view = result.unwrap();
    assert_eq!(
        output(view.outcome().unwrap()).get("Error"),
        Some(&json!("Sandbox.Timedout"))
    );
}
