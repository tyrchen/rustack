//! Actual registered ASL callback owner → Native Lambda; API activation is separate.

use rustack_stepfunctions_core::{
    CallbackCommand, CallbackLimits, CallbackRegistry, CallbackToken, ExecutionView,
    LambdaTaskCommand, LambdaTaskPort, TaskCall, TaskFailure,
};
use tokio::sync::mpsc;

use super::*;

/// Explicit bounded harness credential transport; never a service history/log sink.
#[derive(Debug)]
struct Capture {
    inner: Arc<StepFunctionsLambdaBridge>,
    tokens: mpsc::Sender<CallbackToken>,
}
impl LambdaTaskPort for Capture {
    fn start(&self, command: LambdaTaskCommand) -> Result<TaskCall, TaskFailure> {
        let token = command
            .dispatch()
            .parameters()
            .value()
            .pointer("/Payload/token")
            .and_then(Value::as_str)
            .ok_or_else(TaskFailure::service_exception)?
            .parse()
            .map_err(|_| TaskFailure::service_exception())?;
        self.tokens
            .try_send(token)
            .map_err(|_| TaskFailure::service_exception())?;
        self.inner.start(command)
    }
}
async fn machine(registry: &Registry) -> Result<StateMachineArn, Box<dyn Error>> {
    let source = json!({"StartAt":"Call","States":{"Call":{"Type":"Task","Resource":"arn:aws:states:::lambda:invoke.waitForTaskToken","TimeoutSeconds":300,"Parameters":{"FunctionName":"Worker","Payload":{"n.$":"$.n","retryCount.$":"$$.State.RetryCount","token.$":"$$.Task.Token"}},"End":true}}});
    let compiled = AslCompiler::new(TenantScope::default(), TaskSupport::Lambda)
        .with_callback_support(true)
        .compile(&serde_json::to_vec(&source)?)?;
    let RegistryResult::Machine(machine) = registry
        .submit(RegistryCommand::Create(
            "NativeCallback".parse()?,
            PreparedDefinition::new(compiled.machine().clone()),
            "arn:aws:iam::000000000000:role/Worker".parse()?,
            Tags::default(),
        ))?
        .wait(deadline())
        .await?
    else {
        return Err("machine".into());
    };
    Ok(machine.arn().clone())
}
async fn run(
    fixture: &NativeFixture,
    registry: &Registry,
    callbacks: &CallbackRegistry,
    tokens: &mut mpsc::Receiver<CallbackToken>,
    release: &Path,
    kind: u8,
) -> Result<Option<u64>, Box<dyn Error>> {
    fixture.provider.update_function_configuration(
        "Worker",
        &UpdateFunctionConfigurationInput {
            timeout: Some(900),
            ..UpdateFunctionConfigurationInput::default()
        },
    )?;
    let machine = machine(registry).await?;
    let RegistryResult::Execution(execution) = registry
        .submit(RegistryCommand::Start(
            machine,
            Some("run".parse()?),
            Payload::parse(br#"{"n":21}"#)?,
            None,
        ))?
        .wait(deadline())
        .await?
    else {
        return Err("execution".into());
    };
    let token = timeout_at(deadline(), tokens.recv())
        .await?
        .ok_or("token capture")?;
    let marker = child_port_marker(fixture).await?;
    let pid = marker
        .get("pid")
        .and_then(Value::as_u64)
        .filter(|pid| *pid > 1 && *pid != u64::from(process::id()))
        .ok_or("consumed PID")?;
    let request = marker
        .get("request_id")
        .and_then(Value::as_str)
        .ok_or("consumed request")?;
    if execution.handle().snapshot().outcome().is_some() {
        return Err("callback completed before consumed gate release".into());
    }
    if kind == 3 {
        execution
            .handle()
            .request_stop(TaskFailure::new("Stopped", None).map_err(|_| "stop failure")?);
        let view = execution.handle().wait(deadline()).await?;
        if !matches!(
            view.outcome().map(Arc::as_ref),
            Some(ExecutionOutcome::Aborted(_))
        ) {
            return Err("callback stop outcome".into());
        }
        execution.handle().wait_cleanup(deadline()).await?;
        if callbacks
            .submit(
                &TenantScope::default(),
                CallbackCommand::success(token.expose().parse()?, b"true")?,
            )?
            .wait(deadline())
            .await
            .is_ok()
        {
            return Err("stopped token remained live".into());
        }
        return Ok(Some(pid));
    }
    if kind != 1 {
        send(callbacks, &token).await?;
    }
    if execution.handle().snapshot().outcome().is_some() {
        return Err("early callback bypassed actual Native return".into());
    }
    fs::write(release, b"release").await?;
    if kind == 1 {
        wait_submitted(&execution).await?;
        if execution.handle().snapshot().outcome().is_some() {
            return Err("Lambda body completed callback Task".into());
        }
        send(callbacks, &token).await?;
    }
    let view = execution.handle().wait(deadline()).await?;
    execution.handle().wait_cleanup(deadline()).await?;
    verify(&view, &token, request, kind)?;
    Ok(None)
}
async fn send(callbacks: &CallbackRegistry, token: &CallbackToken) -> Result<(), Box<dyn Error>> {
    callbacks
        .submit(
            &TenantScope::default(),
            CallbackCommand::success(token.expose().parse()?, br#"{"external":42}"#)?,
        )?
        .wait(deadline())
        .await?;
    Ok(())
}
async fn wait_submitted(execution: &ExecutionSnapshot) -> Result<(), Box<dyn Error>> {
    timeout_at(deadline(), async {
        loop {
            let view = execution.handle().snapshot();
            if view
                .history()
                .page(1, 1000, true)?
                .events()
                .iter()
                .any(|event| event.task_submitted_event_details.is_some())
            {
                return Ok::<_, Box<dyn Error>>(());
            }
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await?
}
fn verify(
    view: &ExecutionView,
    token: &CallbackToken,
    request: &str,
    kind: u8,
) -> Result<(), Box<dyn Error>> {
    if kind == 2 {
        if !matches!(view.outcome().map(Arc::as_ref),Some(ExecutionOutcome::Failed(f)) if f.error()==Some("TestError"))
        {
            return Err("Native failure hidden by provisional callback".into());
        }
    } else if !matches!(view.outcome().map(Arc::as_ref),Some(ExecutionOutcome::Succeeded(p)) if p.value()==&json!({"external":42}))
    {
        return Err("callback output was replaced by function body".into());
    }
    let page = view.history().page(1, 1000, true)?;
    let events = page.events();
    if serde_json::to_string(events)?.contains(token.expose()) {
        return Err("plaintext token in callback history".into());
    }
    let scheduled = events
        .iter()
        .find_map(|e| e.task_scheduled_event_details.as_ref())
        .ok_or("scheduled history")?;
    let parameters: Value = serde_json::from_str(&scheduled.parameters)?;
    if parameters.pointer("/Payload/token") != Some(&Value::Null) {
        return Err("callback history provenance missing".into());
    }
    if kind != 2 {
        let submitted = events
            .iter()
            .find_map(|e| e.task_submitted_event_details.as_ref())
            .ok_or("submitted history")?;
        let output: Value = serde_json::from_str(submitted.output.as_deref().ok_or("metadata")?)?;
        if output
            .pointer("/SdkResponseMetadata/RequestId")
            .and_then(Value::as_str)
            != Some(request)
            || output.get("Payload").is_some()
            || output.get("ExecutedVersion").and_then(Value::as_str) != Some("$LATEST")
            || submitted.output_details.as_ref().and_then(|d| d.truncated) != Some(true)
        {
            return Err("actual Native metadata or callback truncation mismatch".into());
        }
    }
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the actual host bootstrap; run make test-stepfunctions-bridge-native"]
async fn native_e2e_should_execute_registered_asl_callbacks_and_join_consumed_native_cancellation()
-> Result<(), Box<dyn Error>> {
    for kind in 0..4 {
        let gate = tempfile::tempdir()?;
        let release = gate.path().join("release");
        let mut vars = vec![
            ("WORKFLOW_MODE", "1"),
            ("WORKFLOW_GATE_FILE", release.to_str().ok_or("gate path")?),
        ];
        if kind == 2 {
            vars.push(("FAIL_MODE", "panic"));
        }
        let mut fixture = NativeFixture::new(&vars, false).await;
        let (sender, mut tokens) = mpsc::channel(8);
        let port = Arc::new(Capture {
            inner: fixture.port.clone(),
            tokens: sender,
        });
        let (callbacks, mut callback_owner) =
            CallbackRegistry::new(TenantScope::default(), CallbackLimits::default())?;
        let mut runtime = ExecutionSupervisor::new(
            RuntimeLimits::new(8, 1, 2)?,
            HistoryLimits::default(),
            256 * 1024 * 1024,
            Some(port),
        )?;
        runtime.install_callbacks(callbacks.clone())?;
        let (registry, mut owner) = Registry::new_with_state_support(
            TenantScope::default(),
            RegistryLimits::default(),
            runtime,
            ExecutionLimits::new(Duration::from_secs(300), 10000)?,
            StateSupport::Structured,
        )?;
        let result = timeout_at(
            Instant::now() + Duration::from_secs(30),
            run(&fixture, &registry, &callbacks, &mut tokens, &release, kind),
        )
        .await;
        let probe = match &result {
            Ok(Ok(Some(pid))) => verify_absent_probe(*pid).await,
            _ => Ok(()),
        };
        let released = fs::write(&release, b"release").await;
        let workflow = owner.shutdown(deadline()).await;
        let callback = callback_owner.shutdown(deadline()).await;
        if workflow.is_err() || callback.is_err() {
            if let Some(dir) = fixture.directory.take() {
                let _retained = dir.keep();
            }
        }
        fixture.cleanup().await;
        released?;
        workflow?;
        callback?;
        probe?;
        result??;
    }
    Ok(())
}
