//! Real Native invocation composition; ASL callback activation is separately gated.

use rustack_stepfunctions_core::{
    CallbackAttempt, CallbackCommand, CallbackDispatch, CallbackInvocation,
    CallbackInvocationMetadata, CallbackLimits, CallbackOutcome, CallbackRegistry, CallbackTask,
    DriverStep, FrameCallId, FrameCapacity, FrameLimits, FrameScheduler, FrameUpdate,
    LambdaTaskCommand, SystemUuidSource, TaskCall, TaskEvent, TaskFailure, TaskSettlement,
};
use uuid::Uuid;

use super::*;
use crate::stepfunctions_bridge::{mapping::Mapped, worker};

fn observed_dispatch() -> Result<(CallbackDispatch, FrameCallId), Box<dyn Error>> {
    let mut definition = workflow(true);
    definition["States"]["Call"]["Resource"] =
        json!("arn:aws:states:::lambda:invoke.waitForTaskToken");
    definition["States"]["Call"]["Parameters"]["Payload"]["token.$"] = json!("$$.Task.Token");
    let machine = AslCompiler::new(TenantScope::default(), TaskSupport::Lambda)
        .with_callback_support(true)
        .compile(&serde_json::to_vec(&definition)?)?
        .machine()
        .clone();
    let now = ClockSample::new(Instant::now().into_std(), SystemTime::now().into());
    let input = Payload::from_value(&json!({"n":21}))?;
    let driver = ExecutionDriver::for_structured_owner(
        machine,
        input.clone(),
        EvaluationContext::from_value(&json!({}))?,
        now,
        ExecutionLimits::default(),
    )?;
    let mut frames = FrameScheduler::new(
        driver,
        Arc::new(FrameCapacity::default()),
        FrameLimits::default(),
    )?;
    for _ in 0..20 {
        if let FrameUpdate::Transition(t) = frames.advance(now, &mut SystemUuidSource::default()) {
            if let DriverStep::Callback(effect) = t.step() {
                return Ok((effect.clone(), t.call().ok_or("call")?));
            }
        }
    }
    Err("missing deferred callback".into())
}

fn command(
    effect: &CallbackDispatch,
    call: FrameCallId,
    task: &mut CallbackTask,
) -> Result<LambdaTaskCommand, Box<dyn Error>> {
    let (dispatch, history) =
        effect.prepare_with_source(task.token(), &mut SystemUuidSource::default())?;
    let scheduled = effect.scheduled_event(&history, SystemTime::now().into())?;
    if !history.redacted() || serde_json::to_string(&scheduled)?.contains(task.token().expose()) {
        return Err("unsafe callback scheduling projection".into());
    }
    Ok(LambdaTaskCommand::observed_callback(
        effect.machine().clone(),
        dispatch,
        call,
        task.take_invocation_reporter()?,
    )?)
}

async fn success(registry: &CallbackRegistry, task: &CallbackTask) -> Result<(), Box<dyn Error>> {
    registry
        .submit(
            &TenantScope::default(),
            CallbackCommand::success(task.token().expose().parse()?, br#"{"external":42}"#)?,
        )?
        .wait(deadline())
        .await?;
    Ok(())
}
async fn result(call: &mut TaskCall) -> Result<Payload, Box<dyn Error>> {
    loop {
        match call.next().await? {
            TaskEvent::Completed(result) => return result.map_err(|_| "invocation failed".into()),
            TaskEvent::Started | TaskEvent::Submitted(_) => {}
            _ => return Err("unexpected task event".into()),
        }
    }
}
async fn run(
    fixture: &NativeFixture,
    registry: &Registry,
    callbacks: &CallbackRegistry,
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
    let arn = child_port_machine(
        registry,
        "CallbackParent",
        &json!({"StartAt":"W","States":{"W":{"Type":"Wait","Seconds":3600,"End":true}}}),
    )
    .await?;
    let RegistryResult::Execution(parent) = registry
        .submit(RegistryCommand::Start(
            arn,
            Some("parent".parse()?),
            Payload::parse(b"{}")?,
            None,
        ))?
        .wait(deadline())
        .await?
    else {
        return Err("parent".into());
    };
    let (effect, call_id) = observed_dispatch()?;
    let mut mint = callbacks.register(CallbackAttempt::new(
        &parent,
        call_id,
        Instant::from_std(effect.deadline()),
        effect.heartbeat(),
    )?)?;
    mint.admitted(deadline()).await?;
    let mut task = mint.into_task()?;
    let secret = task.token().expose().to_owned();
    let command = command(&effect, call_id, &mut task)?;
    let mut call = fixture
        .port
        .start(command)
        .map_err(|_| "bridge admission")?;
    let marker = child_port_marker(fixture).await?;
    let pid = marker.get("pid").and_then(Value::as_u64).ok_or("pid")?;
    let request = marker
        .get("request_id")
        .and_then(Value::as_str)
        .ok_or("request")?;
    if !matches!(task.snapshot().as_ref(), CallbackOutcome::Invoking)
        || task.invocation().is_some()
        || call.settlement() != TaskSettlement::Pending
    {
        return Err("gated invocation completed".into());
    }
    if kind == 3 {
        task.revoke();
        call.cancel();
        timeout_at(deadline(), call.settled()).await??;
        if !matches!(
            task.wait(deadline()).await?.as_ref(),
            CallbackOutcome::Revoked
        ) {
            return Err("callback cancellation".into());
        }
        return Ok(Some(pid));
    }
    if kind != 1 {
        success(callbacks, &task).await?;
    }
    if !matches!(task.snapshot().as_ref(), CallbackOutcome::Invoking) {
        return Err("early callback bypassed invocation".into());
    }
    fs::write(release, b"release").await?;
    let report = task.wait_invocation(deadline()).await?;
    if kind == 2 {
        if report.failure().and_then(|f| f.details().error()) != Some("TestError")
            || !matches!(
                task.wait(deadline()).await?.as_ref(),
                CallbackOutcome::InvocationFailed(_)
            )
        {
            return Err("function failure hidden by callback".into());
        }
        timeout_at(deadline(), call.settled()).await??;
        return Ok(None);
    }
    finish(
        callbacks, &mut task, &mut call, report, request, &secret, kind,
    )
    .await?;
    Ok(None)
}
async fn finish(
    callbacks: &CallbackRegistry,
    task: &mut CallbackTask,
    call: &mut TaskCall,
    report: Arc<CallbackInvocation>,
    request: &str,
    secret: &str,
    kind: u8,
) -> Result<(), Box<dyn Error>> {
    let metadata = report.metadata().ok_or("actual metadata")?;
    if metadata.request_id().to_string() != request || metadata.executed_version() != "$LATEST" {
        return Err("invented invocation metadata".into());
    }
    if kind == 1 {
        // Waiting is published independently; repeat heartbeat as an actor barrier.
        callbacks
            .submit(
                &TenantScope::default(),
                CallbackCommand::heartbeat(task.token().expose().parse()?),
            )?
            .wait(deadline())
            .await?;
        if !matches!(task.snapshot().as_ref(), CallbackOutcome::Waiting { .. }) {
            return Err("not waiting after real response".into());
        }
        success(callbacks, task).await?;
    }
    let outcome = task.wait(deadline()).await?;
    if !matches!(outcome.as_ref(),CallbackOutcome::Succeeded(p) if p.value()==&json!({"external":42}))
    {
        return Err("synchronous body replaced callback output".into());
    }
    let output = timeout_at(deadline(), result(call)).await??;
    timeout_at(deadline(), call.settled()).await??;
    if output.value().get("Payload").is_some()
        || output.value()["SdkResponseMetadata"]["RequestId"] != request
        || format!("{report:?}{output:?}").contains(secret)
    {
        return Err("unsafe callback metadata".into());
    }
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the actual host bootstrap; run make test-stepfunctions-bridge-native"]
async fn native_e2e_should_observe_callbacks_before_and_after_real_return_prioritize_failure_and_reap_cancelled_process()
-> Result<(), Box<dyn Error>> {
    for kind in 0..4 {
        let directory = tempfile::tempdir()?;
        let release = directory.path().join("release");
        let mut vars = vec![
            ("WORKFLOW_MODE", "1"),
            (
                "WORKFLOW_GATE_FILE",
                release.to_str().ok_or("release path")?,
            ),
        ];
        if kind == 2 {
            vars.push(("FAIL_MODE", "panic"));
        }
        let mut fixture = NativeFixture::new(&vars, false).await;
        let (registry, mut owner) = Registry::new(
            TenantScope::default(),
            RegistryLimits::default(),
            executions(fixture.port.clone()),
            ExecutionLimits::default(),
        )?;
        let (callbacks, mut callback_owner) =
            CallbackRegistry::new(TenantScope::default(), CallbackLimits::default())?;
        let outcome = timeout_at(
            deadline(),
            run(&fixture, &registry, &callbacks, &release, kind),
        )
        .await;
        let probe = match &outcome {
            Ok(Ok(Some(pid))) => verify_absent_probe(*pid).await,
            _ => Ok(()),
        };
        let released = fs::write(&release, b"release").await;
        let callback_join = callback_owner.shutdown(deadline()).await;
        let joined = owner.shutdown(deadline()).await;
        if callback_join.is_err() || joined.is_err() {
            if let Some(dir) = fixture.directory.take() {
                let _retained = dir.keep();
            }
        }
        fixture.cleanup().await;
        released?;
        callback_join?;
        joined?;
        probe?;
        outcome??;
    }
    Ok(())
}

#[tokio::test]
async fn test_should_publish_return_before_bridge_settlement_and_preserve_report_on_quarantine()
-> Result<(), Box<dyn Error>> {
    let runtime = ExecutionSupervisor::new(
        RuntimeLimits::default(),
        HistoryLimits::default(),
        256 * 1024 * 1024,
        None,
    )?;
    let (registry, mut owner) = Registry::new(
        TenantScope::default(),
        RegistryLimits::default(),
        runtime,
        ExecutionLimits::default(),
    )?;
    let (callbacks, mut callback_owner) =
        CallbackRegistry::new(TenantScope::default(), CallbackLimits::default())?;
    let outcome = async {
        let arn = child_port_machine(
            &registry,
            "ReportParent",
            &json!({"StartAt":"W","States":{"W":{"Type":"Wait","Seconds":3600,"End":true}}}),
        )
        .await?;
        let RegistryResult::Execution(parent) = registry
            .submit(RegistryCommand::Start(
                arn,
                Some("parent".parse()?),
                Payload::parse(b"{}")?,
                None,
            ))?
            .wait(deadline())
            .await?
        else {
            return Err::<(), Box<dyn Error>>("parent".into());
        };
        let (_, call_id) = observed_dispatch()?;
        let mut mint =
            callbacks.register(CallbackAttempt::new(&parent, call_id, deadline(), None)?)?;
        mint.admitted(deadline()).await?;
        let mut task = mint.into_task()?;
        let mut reporter = Some(task.take_invocation_reporter()?);
        let metadata = CallbackInvocationMetadata::new(Uuid::new_v4(), "$LATEST")?;
        let mut mapped = Mapped {
            result: Ok(metadata.payload()?),
            diagnostic: None,
        };
        let (mut call, mut responder) = TaskCall::channel();
        responder.started()?;
        worker::report(&mut reporter, &mut mapped, Some(metadata), Instant::now());
        let report = task.wait_invocation(deadline()).await?;
        if !report.succeeded() || call.settlement() != TaskSettlement::Pending {
            return Err("report depended on settlement".into());
        }
        success(&callbacks, &task).await?;
        task.wait(deadline()).await?;
        let retained = responder.quarantine(TaskFailure::service_exception());
        if call.settlement() != TaskSettlement::Quarantined
            || !Arc::ptr_eq(&report, &task.wait_invocation(deadline()).await?)
        {
            return Err("cleanup rewrote invocation report".into());
        }
        if timeout(Duration::from_millis(1), call.settled())
            .await
            .is_ok()
        {
            return Err("quarantine fabricated clean".into());
        }
        retained.settle();
        call.settled().await?;
        Ok(())
    }
    .await;
    let a = callback_owner.shutdown(deadline()).await;
    let b = owner.shutdown(deadline()).await;
    a?;
    b?;
    outcome
}
