//! Real controller composition, observer loss and admission-independent health.

use bytes::Bytes;
use http::{Request, StatusCode};
use http_body_util::{BodyExt, Full};
use rustack_stepfunctions_core::{ExecutionOutcome, RegistryCommand, RegistryResult};
use rustack_stepfunctions_http::{RequestScope, StepFunctionsHttpConfig, StepFunctionsHttpService};
use serde_json::json;

use super::*;

fn runtime(budgets: RuntimeBudgets) -> Result<WorkflowRuntime> {
    WorkflowRuntime::new(
        TenantScope::default(),
        budgets,
        #[cfg(feature = "lambda")]
        None,
    )
}
async fn request(
    api: Arc<StepFunctionsApi>,
    operation: &str,
    body: Value,
) -> Result<(StatusCode, Value)> {
    let config = StepFunctionsHttpConfig::new(
        RequestScope::new("aws".into(), "us-east-1".into(), "000000000000".into())?,
        true,
        None,
    )?;
    let service = StepFunctionsHttpService::new(api, config);
    let request = Request::builder()
        .method("POST")
        .uri("/")
        .header("x-amz-target", format!("AWSStepFunctions.{operation}"))
        .header("content-type", "application/x-amz-json-1.0")
        .body(Full::new(Bytes::from(serde_json::to_vec(&body)?)))?;
    let response = service.handle(request).await;
    let status = response.status();
    let body = response.into_body().collect().await?.to_bytes();
    Ok((status, serde_json::from_slice(&body)?))
}
async fn start(
    runtime: &WorkflowRuntime,
    seconds: u32,
) -> Result<rustack_stepfunctions_core::ExecutionSnapshot> {
    let definition =
        json!({"StartAt":"Wait", "States":{"Wait":{"Type":"Wait","Seconds":seconds,"End":true}}})
            .to_string();
    let (status, created) = request(runtime.api.clone(), "CreateStateMachine", json!({"name":"Machine","roleArn":"arn:aws:iam::000000000000:role/Test","definition":definition})).await?;
    assert_eq!(status, StatusCode::OK, "{created}");
    let (status, started) = request(runtime.api.clone(), "StartExecution", json!({"stateMachineArn":created["stateMachineArn"],"name":"Run","input":"{\"answer\":42}"})).await?;
    assert_eq!(status, StatusCode::OK, "{started}");
    let arn = started["executionArn"]
        .as_str()
        .context("execution ARN")?
        .parse()?;
    let mut receipt = runtime
        .status
        .registry
        .submit(RegistryCommand::Execution(arn))?;
    let RegistryResult::Execution(snapshot) = receipt
        .wait(Instant::now() + Duration::from_secs(5))
        .await?
    else {
        anyhow::bail!("unexpected capture")
    };
    Ok(snapshot)
}
#[tokio::test]
async fn test_should_finish_accepted_execution_during_grace_before_registry_close() -> Result<()> {
    let mut runtime = runtime(RuntimeBudgets::default())?;
    let snapshot = start(&runtime, 1).await?;
    assert!(runtime.status.is_ready());
    assert_eq!(runtime.status.registry.status().owned_executions, 1);
    runtime
        .owner
        .quiesce(Instant::now() + Duration::from_secs(10))
        .await?;
    assert!(!runtime.status.is_ready());
    let complete = snapshot
        .wait(Instant::now() + Duration::from_secs(1))
        .await?;
    assert!(matches!(
        complete.view().outcome().map(Arc::as_ref),
        Some(ExecutionOutcome::Succeeded(_))
    ));
    assert_eq!(
        complete.view().cleanup(),
        rustack_stepfunctions_core::CleanupState::Clean
    );
    runtime
        .owner
        .quiesce(Instant::now() + Duration::from_secs(1))
        .await?;
    Ok(())
}
#[tokio::test]
async fn test_should_resume_same_grace_after_cancelled_shutdown_observer() -> Result<()> {
    let mut runtime = runtime(RuntimeBudgets::default())?;
    let snapshot = start(&runtime, 3600).await?;
    tokio::time::pause();
    let deadline = Instant::now() + Duration::from_secs(100);
    assert!(
        tokio::time::timeout(Duration::from_millis(1), runtime.owner.quiesce(deadline))
            .await
            .is_err()
    );
    let Phase::Grace(cutoff) = runtime.owner.phase else {
        anyhow::bail!("grace not reached")
    };
    assert!(!runtime.status.is_ready());
    tokio::time::advance(cutoff.saturating_duration_since(Instant::now())).await;
    runtime.owner.quiesce(deadline).await?;
    let complete = snapshot.wait(deadline).await?;
    let Some(ExecutionOutcome::Aborted(failure)) = complete.view().outcome().map(Arc::as_ref)
    else {
        anyhow::bail!("execution was not aborted")
    };
    assert_eq!(failure.details().error(), Some("Rustack.Shutdown"));
    let page = complete.view().history().page(1, 100, true)?;
    assert!(page.events().iter().any(|event| {
        event
            .execution_aborted_event_details
            .as_ref()
            .is_some_and(|details| details.error.as_deref() == Some("Rustack.Shutdown"))
    }));
    Ok(())
}
#[tokio::test]
async fn test_should_preserve_grace_cutoff_after_expired_observer() -> Result<()> {
    let mut runtime = runtime(RuntimeBudgets::default())?;
    let snapshot = start(&runtime, 3600).await?;
    tokio::time::pause();
    assert!(
        runtime
            .owner
            .quiesce(Instant::now() + Duration::from_millis(10))
            .await
            .is_err()
    );
    let Phase::Grace(cutoff) = runtime.owner.phase else {
        anyhow::bail!("grace not retained")
    };
    tokio::time::advance(Duration::from_secs(6)).await;
    runtime
        .owner
        .quiesce(Instant::now() + Duration::from_secs(10))
        .await?;
    assert!(Instant::now() >= cutoff);
    assert!(matches!(
        snapshot
            .wait(Instant::now() + Duration::from_secs(1))
            .await?
            .view()
            .outcome()
            .map(Arc::as_ref),
        Some(ExecutionOutcome::Aborted(_))
    ));
    Ok(())
}
#[tokio::test]
async fn test_should_read_health_when_registry_command_admission_is_full() -> Result<()> {
    let mut budgets = RuntimeBudgets::default();
    budgets.workflow_registry_commands = 1;
    let mut runtime = runtime(budgets)?;
    let mut receipt = runtime.status.registry.submit(RegistryCommand::Machines)?;
    assert!(matches!(
        runtime.status.registry.submit(RegistryCommand::Machines),
        Err(RegistryError::Capacity)
    ));
    assert!(runtime.status.is_ready());
    receipt
        .wait(Instant::now() + Duration::from_secs(5))
        .await?;
    runtime
        .owner
        .quiesce(Instant::now() + Duration::from_secs(5))
        .await?;
    Ok(())
}
#[tokio::test]
async fn test_should_report_pure_profile_and_reject_lambda_without_actual_port() -> Result<()> {
    let mut runtime = runtime(RuntimeBudgets::default())?;
    let capabilities = runtime.status.capabilities();
    assert_eq!(capabilities["lambdaRequestResponse"], false);
    assert_eq!(capabilities["snapshot"], "unsupported");
    assert_eq!(capabilities["dockerQualified"], false);
    assert_eq!(capabilities["limits"]["apiWorkers"], 8);
    let definition = json!({"StartAt":"Call","States":{"Call":{"Type":"Task","Resource":"arn:aws:lambda:us-east-1:000000000000:function:Fn","End":true}}}).to_string();
    let (status, body) = request(runtime.api.clone(), "CreateStateMachine", json!({"name":"Task","definition":definition,"roleArn":"arn:aws:iam::000000000000:role/Test"})).await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    runtime
        .owner
        .quiesce(Instant::now() + Duration::from_secs(5))
        .await?;
    Ok(())
}
#[cfg(feature = "lambda")]
#[tokio::test]
async fn test_should_install_only_registered_selected_actual_lambda_provider() -> Result<()> {
    use rustack_lambda_core::config::LambdaConfig;

    let config =
        LambdaConfig::from_lookup(|key| (key == "LAMBDA_EXECUTOR").then(|| "native".to_owned()))?;
    let provider = Arc::new(RustackLambda::new(config));
    let mut runtime = WorkflowRuntime::new(
        TenantScope::default(),
        RuntimeBudgets::default(),
        Some(provider.clone()),
    )?;
    assert_eq!(runtime.status.capabilities()["lambdaRequestResponse"], true);
    assert!(runtime.status.is_ready());
    runtime
        .owner
        .quiesce(Instant::now() + Duration::from_secs(5))
        .await?;
    provider.quiesce(Duration::from_secs(5)).await?;
    provider.shutdown().await;
    Ok(())
}

#[cfg(feature = "lambda")]
#[tokio::test]
async fn test_should_reject_unqualified_manual_docker_provider() {
    use rustack_lambda_core::config::LambdaConfig;

    let provider = Arc::new(RustackLambda::new(LambdaConfig {
        executor: ExecutorBackend::Docker,
        ..LambdaConfig::default()
    }));
    assert!(
        WorkflowRuntime::new(
            TenantScope::default(),
            RuntimeBudgets::default(),
            Some(provider)
        )
        .is_err()
    );
}

#[test]
fn test_should_retain_typed_cleanup_source_and_redact_debug() {
    let failure = WorkflowFailure(Arc::new(anyhow::anyhow!("sensitive-marker")));
    assert!(!format!("{failure:?}").contains("sensitive-marker"));
    assert!(failure.source().is_some());
}

#[tokio::test]
async fn test_should_skip_grace_on_final_cleanup_and_cancel_before_provider_destruction()
-> Result<()> {
    let mut budgets = RuntimeBudgets::default();
    budgets.workflow_drain_seconds = 60;
    let mut runtime = runtime(budgets)?;
    let snapshot = start(&runtime, 3600).await?;
    runtime
        .owner
        .shutdown(Instant::now() + Duration::from_secs(5))
        .await?;
    assert!(matches!(runtime.owner.phase, Phase::Done));
    let complete = snapshot
        .wait(Instant::now() + Duration::from_secs(1))
        .await?;
    assert!(
        matches!(complete.view().outcome().map(Arc::as_ref), Some(ExecutionOutcome::Aborted(failure)) if failure.details().error() == Some("Rustack.Shutdown"))
    );
    Ok(())
}

#[tokio::test]
async fn test_should_compile_validate_update_and_execute_structured_profile_through_real_api()
-> Result<()> {
    let mut budgets = RuntimeBudgets::default();
    budgets.workflow_frames = 16;
    budgets.workflow_frames_per_execution = 8;
    budgets.workflow_result_bytes = 2 * 1024 * 1024;
    budgets.workflow_result_bytes_per_execution = 1024 * 1024;
    let mut runtime = runtime(budgets)?;
    let result = async {
        let definition = json!({"StartAt":"M","States":{"M":{"Type":"Map","ItemProcessor":{"StartAt":"D","States":{"D":{"Type":"Pass","End":true}}},"End":true}}}).to_string();
        let (status, validated) = request(runtime.api.clone(), "ValidateStateMachineDefinition", json!({"definition":definition})).await?;
        assert_eq!(status, StatusCode::OK, "{validated}");
        assert_eq!(validated["result"], "OK");
        let (status, created) = request(runtime.api.clone(), "CreateStateMachine", json!({"name":"Structured","roleArn":"arn:aws:iam::000000000000:role/Test","definition":definition})).await?;
        assert_eq!(status, StatusCode::OK, "{created}");
        let definition = json!({"StartAt":"P","States":{"P":{"Type":"Parallel","Branches":[serde_json::from_str::<Value>(&definition)?,serde_json::from_str::<Value>(&definition)?],"End":true}}}).to_string();
        let (status, updated) = request(runtime.api.clone(), "UpdateStateMachine", json!({"stateMachineArn":created["stateMachineArn"],"definition":definition})).await?;
        assert_eq!(status, StatusCode::OK, "{updated}");
        let (status, started) = request(runtime.api.clone(), "StartExecution", json!({"stateMachineArn":created["stateMachineArn"],"input":"[1,null,3]"})).await?;
        assert_eq!(status, StatusCode::OK, "{started}");
        let arn = started["executionArn"].as_str().context("execution ARN")?.parse()?;
        let RegistryResult::Execution(snapshot) = runtime.status.registry.submit(RegistryCommand::Execution(arn))?.wait(Instant::now()+Duration::from_secs(5)).await? else { anyhow::bail!("missing execution"); };
        let complete = snapshot.wait(Instant::now()+Duration::from_secs(5)).await?;
        assert!(matches!(complete.view().outcome().map(Arc::as_ref),Some(ExecutionOutcome::Succeeded(value)) if value.value()==&json!([[1,null,3],[1,null,3]])));
        let capabilities = runtime.status.capabilities();
        assert_eq!(capabilities["stateTypes"].as_array().context("states")?.len(), 8);
        assert_eq!(capabilities["inlineMapMaxConcurrency"], 40);
        assert_eq!(capabilities["limits"]["frames"], 16);
        assert_eq!(capabilities["limits"]["framesPerExecution"], 8);
        assert_eq!(capabilities["limits"]["resultBytes"], 2*1024*1024);
        assert_eq!(capabilities["limits"]["resultBytesPerExecution"], 1024*1024);
        let invalid = json!({"StartAt":"M","States":{"M":{"Type":"Map","ItemProcessor":{"ProcessorConfig":{"Mode":"DISTRIBUTED"},"StartAt":"D","States":{"D":{"Type":"Succeed"}}},"End":true}}}).to_string();
        let (_, validation) = request(runtime.api.clone(), "ValidateStateMachineDefinition", json!({"definition":invalid})).await?;
        assert_eq!(validation["result"], "FAIL");
        Ok(())
    }.await;
    runtime
        .owner
        .quiesce(Instant::now() + Duration::from_secs(10))
        .await?;
    result
}

#[tokio::test]
async fn test_should_enforce_configured_frame_and_result_limits_in_public_execution_view()
-> Result<()> {
    for (frames, bytes, expected) in [
        (1, 1024 * 1024, "Rustack.FrameLimitExceeded"),
        (8, 4096, "Rustack.ResultLimitExceeded"),
    ] {
        let mut budgets = RuntimeBudgets::default();
        budgets.workflow_frames = frames;
        budgets.workflow_frames_per_execution = frames;
        budgets.workflow_result_bytes = bytes;
        budgets.workflow_result_bytes_per_execution = bytes;
        let mut runtime = runtime(budgets)?;
        let result = async {
            let definition = json!({"StartAt":"P","States":{"P":{"Type":"Parallel","Branches":[{"StartAt":"D","States":{"D":{"Type":"Pass","Result":"x".repeat(3000),"End":true}}}],"End":true}}}).to_string();
            let (status, created) = request(runtime.api.clone(), "CreateStateMachine", json!({"name":"Quota","roleArn":"arn:aws:iam::000000000000:role/Test","definition":definition})).await?;
            assert_eq!(status, StatusCode::OK, "{created}");
            let (status, started) = request(runtime.api.clone(), "StartExecution", json!({"stateMachineArn":created["stateMachineArn"],"name":"Run"})).await?;
            assert_eq!(status, StatusCode::OK, "{started}");
            let arn = started["executionArn"].as_str().context("ARN")?.parse()?;
            let RegistryResult::Execution(snapshot) = runtime.status.registry.submit(RegistryCommand::Execution(arn))?.wait(Instant::now()+Duration::from_secs(5)).await? else { anyhow::bail!("missing execution"); };
            snapshot.wait(Instant::now()+Duration::from_secs(5)).await?;
            let (status, described) = request(runtime.api.clone(), "DescribeExecution", json!({"executionArn":started["executionArn"]})).await?;
            assert_eq!(status, StatusCode::OK, "{described}");
            assert_eq!(described["status"], "FAILED");
            assert_eq!(described["error"], expected);
            Ok(())
        }.await;
        runtime
            .owner
            .quiesce(Instant::now() + Duration::from_secs(5))
            .await?;
        result?;
    }
    Ok(())
}
