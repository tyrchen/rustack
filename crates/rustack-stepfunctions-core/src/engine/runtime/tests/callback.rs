//! Actual registry/actor callback executions over a bounded controlled observed port.

use rustack_stepfunctions_model::types::HistoryEventType;
use tokio::time::{advance, pause, resume, sleep, timeout_at};
use uuid::Uuid;

use super::*;
use crate::{
    CallbackCommand, CallbackError, CallbackInvocationMetadata, CallbackLimits, CallbackRegistry,
    CallbackSupervisor, ExecutionSnapshot, FrameCapacity, FrameLimits, PreparedDefinition,
    Registry, RegistryCommand, RegistryError, RegistryLimits, RegistryResult, RegistrySupervisor,
    StateMachineArn, StateSupport, Tags,
};

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}
fn definition(extra: Value) -> Value {
    let mut task = json!({"Type":"Task","Resource":"arn:aws:states:::lambda:invoke.waitForTaskToken","Parameters":{"FunctionName":"Worker","Payload":{"token.$":"$$.Task.Token","n.$":"$.n","retry.$":"$$.State.RetryCount"}},"End":true});
    let Value::Object(extra) = extra else {
        panic!("fixture object");
    };
    task.as_object_mut().unwrap().extend(extra);
    json!({"StartAt":"Call","States":{"Call":task}})
}
struct Fixture {
    registry: Registry,
    owner: RegistrySupervisor,
    callbacks: (CallbackRegistry, CallbackSupervisor),
    calls: (
        mpsc::Receiver<(LambdaTaskCommand, TaskResponder)>,
        Arc<Port>,
    ),
    frames: Arc<FrameCapacity>,
}
impl Fixture {
    fn new(history: HistoryLimits, limits: FrameLimits) -> Self {
        Self::configured(history, limits, None, false, CallbackLimits::default())
    }
    fn configured(
        history: HistoryLimits,
        limits: FrameLimits,
        hook: Option<Arc<WorkerHook>>,
        panic_port: bool,
        callback_limits: CallbackLimits,
    ) -> Self {
        let (callback, callback_owner) =
            CallbackRegistry::new(TenantScope::default(), callback_limits).unwrap();
        let (sender, receiver) = mpsc::channel(32);
        let port = Arc::new(Port {
            sender,
            starts: AtomicUsize::new(0),
            panic: panic_port,
        });
        let frames = Arc::new(FrameCapacity::default());
        let mut runtime = ExecutionSupervisor::new_with_frames(
            RuntimeLimits::new(8, 1, 2).unwrap(),
            history,
            256 * 1024 * 1024,
            Some(port.clone()),
            frames.clone(),
            limits,
        )
        .unwrap();
        Arc::get_mut(&mut runtime.shared).unwrap().callback_hook = hook;
        runtime.install_callbacks(callback.clone()).unwrap();
        let (registry, owner) = Registry::new_with_state_support(
            TenantScope::default(),
            RegistryLimits::default(),
            runtime,
            ExecutionLimits::new(Duration::from_secs(60), 10000).unwrap(),
            StateSupport::Structured,
        )
        .unwrap();
        assert!(registry.callback_support());
        Self {
            registry,
            owner,
            callbacks: (callback, callback_owner),
            calls: (receiver, port),
            frames,
        }
    }
    async fn create(&self, name: &str, source: &Value) -> StateMachineArn {
        let machine = AslCompiler::new(TenantScope::default(), TaskSupport::Lambda)
            .with_state_support(StateSupport::Structured)
            .with_callback_support(true)
            .with_child_support(true)
            .compile(&serde_json::to_vec(source).unwrap())
            .unwrap()
            .machine()
            .clone();
        let RegistryResult::Machine(snapshot) = self
            .registry
            .submit(RegistryCommand::Create(
                name.parse().unwrap(),
                PreparedDefinition::new(machine),
                "arn:aws:iam::000000000000:role/Test".parse().unwrap(),
                Tags::default(),
            ))
            .unwrap()
            .wait(deadline())
            .await
            .unwrap()
        else {
            panic!("machine");
        };
        snapshot.arn().clone()
    }
    async fn start(&self, machine: &StateMachineArn, name: &str) -> ExecutionSnapshot {
        let RegistryResult::Execution(snapshot) = self
            .registry
            .submit(RegistryCommand::Start(
                machine.clone(),
                Some(name.parse().unwrap()),
                Payload::parse(br#"{"n":21,"keep":"original"}"#).unwrap(),
                None,
            ))
            .unwrap()
            .wait(deadline())
            .await
            .unwrap()
        else {
            panic!("execution");
        };
        snapshot
    }
    async fn next(&mut self) -> (LambdaTaskCommand, TaskResponder) {
        timeout_at(deadline(), self.calls.0.recv())
            .await
            .unwrap()
            .unwrap()
    }
    async fn action(&self, command: CallbackCommand) -> Result<(), CallbackError> {
        self.callbacks
            .0
            .submit(&TenantScope::default(), command)?
            .wait(deadline())
            .await
    }
    async fn close(&mut self) {
        let a = self.owner.shutdown(deadline()).await;
        let b = self.callbacks.1.shutdown(deadline()).await;
        assert!(a.is_ok(), "registry cleanup: {a:?}");
        assert!(b.is_ok(), "callback cleanup: {b:?}");
        assert_eq!(self.frames.available_frames(), 4096);
    }
}
fn token(command: &LambdaTaskCommand) -> String {
    command.dispatch().parameters().value()["Payload"]["token"]
        .as_str()
        .unwrap()
        .to_owned()
}
fn report(
    command: &mut LambdaTaskCommand,
    responder: &mut TaskResponder,
    result: Result<(), TaskFailure>,
) -> CallbackInvocationMetadata {
    responder.started().unwrap();
    let metadata = CallbackInvocationMetadata::new(Uuid::new_v4(), "$LATEST").unwrap();
    let mut reporter = command.take_callback_reporter().unwrap().unwrap();
    reporter
        .complete_at(result.map(|()| metadata.clone()), Instant::now())
        .unwrap();
    metadata
}
async fn submitted(snapshot: &ExecutionSnapshot) {
    timeout_at(deadline(), async {
        loop {
            if snapshot
                .handle()
                .snapshot()
                .history()
                .page(1, 1000, true)
                .unwrap()
                .events()
                .iter()
                .any(|event| event.r#type == HistoryEventType::TaskSubmitted)
            {
                return;
            }
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
}
#[rstest::rstest]
#[case(true)]
#[case(false)]
#[tokio::test]
async fn test_should_execute_registered_callback_before_or_after_real_report_and_keep_causal_safe_history(
    #[case] early: bool,
) {
    let mut f = Fixture::new(HistoryLimits::default(), FrameLimits::default());
    let machine=f.create("Callback",&definition(json!({"ResultSelector":{"chosen.$":"$.external"},"ResultPath":"$.result","OutputPath":"$.result"}))).await;
    let execution = f.start(&machine, "run").await;
    let (mut command, mut responder) = f.next().await;
    let token = token(&command);
    let scheduled = execution.handle().snapshot();
    assert_eq!(
        scheduled
            .history()
            .page(1, 1000, true)
            .unwrap()
            .events()
            .last()
            .unwrap()
            .r#type,
        HistoryEventType::TaskScheduled
    );
    assert!(
        !serde_json::to_string(scheduled.history().page(1, 1000, true).unwrap().events())
            .unwrap()
            .contains(&token)
    );
    if early {
        f.action(CallbackCommand::success(token.parse().unwrap(), br#"{"external":42}"#).unwrap())
            .await
            .unwrap();
    }
    assert!(execution.handle().snapshot().outcome().is_none());
    let metadata = report(&mut command, &mut responder, Ok(()));
    responder.finish(Ok(metadata.payload().unwrap()));
    if !early {
        submitted(&execution).await;
        f.action(CallbackCommand::success(token.parse().unwrap(), br#"{"external":42}"#).unwrap())
            .await
            .unwrap();
    }
    let view = execution.handle().wait(deadline()).await.unwrap();
    assert!(
        matches!(view.outcome().unwrap().as_ref(),ExecutionOutcome::Succeeded(p) if p.value()==&json!({"chosen":42}))
    );
    let page = view.history().page(1, 1000, true).unwrap();
    let events = page.events();
    let kinds = events
        .iter()
        .map(|event| event.r#type.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            HistoryEventType::ExecutionStarted,
            HistoryEventType::TaskStateEntered,
            HistoryEventType::TaskScheduled,
            HistoryEventType::TaskStarted,
            HistoryEventType::TaskSubmitted,
            HistoryEventType::TaskSucceeded,
            HistoryEventType::TaskStateExited,
            HistoryEventType::ExecutionSucceeded
        ]
    );
    assert!(
        events
            .windows(2)
            .all(|pair| pair[1].previous_event_id == Some(pair[0].id))
    );
    assert_eq!(
        events[4]
            .task_submitted_event_details
            .as_ref()
            .unwrap()
            .output_details
            .as_ref()
            .unwrap()
            .truncated,
        Some(true)
    );
    assert!(!serde_json::to_string(events).unwrap().contains(&token));
    assert!(
        f.action(CallbackCommand::heartbeat(token.parse().unwrap()))
            .await
            .is_err()
    );
    drop(command);
    f.close().await;
}
#[tokio::test]
async fn test_should_keep_function_failure_over_successful_provisional_callback() {
    let mut f = Fixture::new(HistoryLimits::default(), FrameLimits::default());
    let machine = f.create("Callback", &definition(json!({}))).await;
    let execution = f.start(&machine, "run").await;
    let (mut command, mut responder) = f.next().await;
    let token = token(&command);
    f.action(CallbackCommand::success(token.parse().unwrap(), b"true").unwrap())
        .await
        .unwrap();
    let failure = TaskFailure::new("FunctionFailure", None).unwrap();
    report(&mut command, &mut responder, Err(failure.clone()));
    responder.finish(Err(failure));
    let view = execution.handle().wait(deadline()).await.unwrap();
    assert!(
        matches!(view.outcome().unwrap().as_ref(),ExecutionOutcome::Failed(f) if f.error()==Some("FunctionFailure"))
    );
    assert!(
        !view
            .history()
            .page(1, 1000, true)
            .unwrap()
            .events()
            .iter()
            .any(|event| event.r#type == HistoryEventType::TaskSubmitted)
    );
    drop(command);
    f.close().await;
}
#[tokio::test]
async fn test_should_release_one_dispatch_slot_while_external_callback_waits() {
    let mut f = Fixture::new(HistoryLimits::default(), FrameLimits::default());
    let callback = f.create("Callback", &definition(json!({}))).await;
    let first = f.start(&callback, "waiting").await;
    let (mut command, mut responder) = f.next().await;
    let first_token = token(&command);
    let metadata = report(&mut command, &mut responder, Ok(()));
    responder.finish(Ok(metadata.payload().unwrap()));
    submitted(&first).await;
    let second = f.start(&callback, "second").await;
    let (mut command2, mut responder2) = f.next().await;
    assert!(first.handle().snapshot().outcome().is_none());
    let second_token = token(&command2);
    let metadata = report(&mut command2, &mut responder2, Ok(()));
    responder2.finish(Ok(metadata.payload().unwrap()));
    for token in [first_token, second_token] {
        f.action(CallbackCommand::success(token.parse().unwrap(), b"true").unwrap())
            .await
            .unwrap();
    }
    first.handle().wait(deadline()).await.unwrap();
    second.handle().wait(deadline()).await.unwrap();
    drop((command, command2));
    f.close().await;
}
#[tokio::test]
async fn test_should_preserve_selected_callback_result_through_quarantine_and_resumed_cleanup() {
    let mut f = Fixture::new(HistoryLimits::default(), FrameLimits::default());
    let machine = f.create("Callback", &definition(json!({}))).await;
    let execution = f.start(&machine, "run").await;
    let (mut command, mut responder) = f.next().await;
    let secret = token(&command);
    report(&mut command, &mut responder, Ok(()));
    f.action(CallbackCommand::success(secret.parse().unwrap(), b"42").unwrap())
        .await
        .unwrap();
    let view = execution.handle().wait(deadline()).await.unwrap();
    assert!(
        matches!(view.outcome().unwrap().as_ref(),ExecutionOutcome::Succeeded(p) if p.value()==&json!(42))
    );
    assert_eq!(view.cleanup(), CleanupState::Pending);
    let retained = responder.quarantine(TaskFailure::service_exception());
    assert!(
        timeout(
            Duration::from_millis(5),
            execution.handle().wait_cleanup(deadline())
        )
        .await
        .is_err()
    );
    retained.settle();
    execution.handle().wait_cleanup(deadline()).await.unwrap();
    assert!(
        matches!(execution.handle().snapshot().outcome().unwrap().as_ref(),ExecutionOutcome::Succeeded(p) if p.value()==&json!(42))
    );
    drop(command);
    f.close().await;
}
#[tokio::test]
async fn test_should_revoke_and_join_actual_call_on_stop_before_retiring_the_frame() {
    let mut f = Fixture::new(HistoryLimits::default(), FrameLimits::default());
    let machine = f.create("Callback", &definition(json!({}))).await;
    let execution = f.start(&machine, "run").await;
    let (command, mut responder) = f.next().await;
    let token = token(&command);
    responder.started().unwrap();
    let mut cancelled = responder.cancellation();
    execution
        .handle()
        .request_stop(TaskFailure::new("Stopped", None).unwrap());
    cancelled.wait_for(|v| *v).await.unwrap();
    let view = execution.handle().wait(deadline()).await.unwrap();
    assert!(matches!(
        view.outcome().unwrap().as_ref(),
        ExecutionOutcome::Aborted(_)
    ));
    assert_eq!(view.cleanup(), CleanupState::Pending);
    assert!(
        f.action(CallbackCommand::success(token.parse().unwrap(), b"true").unwrap())
            .await
            .is_err()
    );
    responder.finish(Err(TaskFailure::service_exception()));
    execution.handle().wait_cleanup(deadline()).await.unwrap();
    drop(command);
    f.close().await;
}
#[tokio::test]
async fn test_should_reject_callback_data_capacity_before_token_registration_or_invocation() {
    let mut f = Fixture::new(HistoryLimits::default(), FrameLimits::new(8, 4096).unwrap());
    let machine = f.create("Callback", &definition(json!({}))).await;
    let execution = f.start(&machine, "run").await;
    let view = execution.handle().wait(deadline()).await.unwrap();
    assert!(
        matches!(view.outcome().unwrap().as_ref(),ExecutionOutcome::Failed(f) if f.error()==Some("Rustack.ResultLimitExceeded"))
    );
    assert_eq!(f.calls.1.starts.load(Ordering::SeqCst), 0);
    assert_eq!(f.callbacks.0.status().live, 0);
    f.close().await;
}
#[tokio::test]
async fn test_should_fail_history_before_any_callback_business_issuance() {
    let mut f = Fixture::new(
        HistoryLimits::new(3, 1024 * 1024).unwrap(),
        FrameLimits::default(),
    );
    let machine = f.create("Callback", &definition(json!({}))).await;
    let execution = f.start(&machine, "run").await;
    let view = execution.handle().wait(deadline()).await.unwrap();
    assert!(
        matches!(view.outcome().unwrap().as_ref(),ExecutionOutcome::Failed(f) if f.error()==Some("States.HistoryLimitExceeded"))
    );
    assert_eq!(f.calls.1.starts.load(Ordering::SeqCst), 0);
    f.close().await;
}

#[tokio::test]
async fn test_should_fail_unreported_owner_loss_without_retry_or_catch_and_retain_physical_cleanup()
{
    let mut f = Fixture::new(HistoryLimits::default(), FrameLimits::default());
    let mut source = definition(
        json!({"Retry":[{"ErrorEquals":["States.ALL"],"IntervalSeconds":1,"MaxAttempts":1}],"Catch":[{"ErrorEquals":["States.ALL"],"Next":"Recovered"}]}),
    );
    source["States"]["Recovered"] = json!({"Type":"Succeed"});
    let machine = f.create("Callback", &source).await;
    let execution = f.start(&machine, "lost").await;
    let (mut command, mut responder) = f.next().await;
    responder.started().unwrap();
    drop(command.take_callback_reporter().unwrap().unwrap());
    let view = execution.handle().wait(deadline()).await.unwrap();
    assert!(
        matches!(view.outcome().unwrap().as_ref(),ExecutionOutcome::Failed(f) if f.error()==Some("Rustack.InternalError"))
    );
    assert!(
        matches!(view.diagnostic(),Some(ExecutionError::Callback(source)) if matches!(source.as_ref(),CallbackError::OwnerLost))
    );
    assert_eq!(view.cleanup(), CleanupState::Pending);
    assert_eq!(f.calls.1.starts.load(Ordering::SeqCst), 1);
    assert!(matches!(
        f.owner
            .shutdown(Instant::now() + Duration::from_millis(5))
            .await,
        Err(RegistryError::Deadline)
    ));
    responder.finish(Err(TaskFailure::service_exception()));
    execution.handle().wait_cleanup(deadline()).await.unwrap();
    drop(command);
    f.close().await;
}
#[tokio::test]
async fn test_should_retain_unknown_closed_port_settlement_in_quarantine() {
    let mut f = Fixture::new(HistoryLimits::default(), FrameLimits::default());
    let machine = f.create("Callback", &definition(json!({}))).await;
    let execution = f.start(&machine, "lost").await;
    let (mut command, mut responder) = f.next().await;
    responder.started().unwrap();
    drop(command.take_callback_reporter().unwrap().unwrap());
    drop(responder);
    let view = execution.handle().wait(deadline()).await.unwrap();
    assert!(
        matches!(view.outcome().unwrap().as_ref(),ExecutionOutcome::Failed(f) if f.error()==Some("Rustack.InternalError"))
    );
    assert!(matches!(
        execution.handle().wait_cleanup(deadline()).await,
        Err(ExecutionError::Quarantined)
    ));
    let failure = f.owner.shutdown(deadline()).await.unwrap_err();
    assert!(matches!(
        registry_error(&failure),
        RegistryError::Execution(ExecutionError::Quarantined)
    ));
    f.callbacks.1.shutdown(deadline()).await.unwrap();
    assert!(f.frames.available_frames() < 4096);
}
#[tokio::test]
async fn test_should_preserve_preparation_evaluation_failure_instead_of_catching_it_as_business_error()
 {
    let mut f = Fixture::new(HistoryLimits::default(), FrameLimits::default());
    let mut source = definition(
        json!({"Parameters":{"FunctionName":"Worker","Payload":{"token.$":"$$.Task.Token","missing.$":"$.absent"}},"Catch":[{"ErrorEquals":["States.ALL"],"Next":"Recovered"}]}),
    );
    source["States"]["Recovered"] = json!({"Type":"Succeed"});
    let machine = f.create("Callback", &source).await;
    let execution = f.start(&machine, "bad").await;
    let view = execution.handle().wait(deadline()).await.unwrap();
    assert!(
        matches!(view.outcome().unwrap().as_ref(),ExecutionOutcome::Failed(f) if f.error()==Some("States.Runtime"))
    );
    assert_eq!(f.calls.1.starts.load(Ordering::SeqCst), 0);
    assert!(
        !view
            .history()
            .page(1, 1000, true)
            .unwrap()
            .events()
            .iter()
            .any(|event| event.r#type == HistoryEventType::TaskScheduled)
    );
    f.close().await;
}
fn hook(panic: bool, before: bool) -> WorkerGate {
    WorkerGate {
        hook: Arc::new(WorkerHook {
            entered: tokio::sync::Notify::new(),
            released: std::sync::atomic::AtomicBool::new(false),
            skip: AtomicUsize::new(0),
            panic,
            before,
        }),
    }
}
#[tokio::test]
async fn test_should_join_held_callback_preparation_after_shutdown_observation_expires_without_issuance()
 {
    let gate = hook(false, false);
    let mut f = Fixture::configured(
        HistoryLimits::default(),
        FrameLimits::default(),
        Some(gate.hook.clone()),
        false,
        CallbackLimits::default(),
    );
    let machine = f.create("Callback", &definition(json!({}))).await;
    let execution = f.start(&machine, "held").await;
    timeout(Duration::from_secs(5), gate.entered.notified())
        .await
        .unwrap();
    execution
        .handle()
        .request_stop(TaskFailure::new("Stopped", None).unwrap());
    execution.handle().wait(deadline()).await.unwrap();
    assert!(matches!(
        f.owner
            .shutdown(Instant::now() + Duration::from_millis(5))
            .await,
        Err(RegistryError::Deadline)
    ));
    assert_eq!(f.calls.1.starts.load(Ordering::SeqCst), 0);
    assert!(f.frames.available_frames() < 4096);
    gate.release();
    f.close().await;
}
#[tokio::test]
async fn test_should_keep_callback_preparation_panic_source_and_clean_only_proven_unissued_work() {
    let gate = hook(true, true);
    let mut f = Fixture::configured(
        HistoryLimits::default(),
        FrameLimits::default(),
        Some(gate.hook.clone()),
        false,
        CallbackLimits::default(),
    );
    let machine = f.create("Callback", &definition(json!({}))).await;
    let execution = f.start(&machine, "panic").await;
    timeout(Duration::from_secs(5), gate.entered.notified())
        .await
        .unwrap();
    gate.release();
    let view = execution.handle().wait(deadline()).await.unwrap();
    assert!(
        matches!(view.diagnostic(),Some(ExecutionError::Callback(source)) if matches!(source.as_ref(),CallbackError::Worker(_)))
    );
    assert!(!format!("{view:?}").contains("controlled CPU"));
    assert_eq!(f.calls.1.starts.load(Ordering::SeqCst), 0);
    execution.handle().wait_cleanup(deadline()).await.unwrap();
    f.close().await;
}
#[tokio::test]
async fn test_should_retain_callback_frame_data_and_dispatch_on_port_worker_panic() {
    let mut f = Fixture::configured(
        HistoryLimits::default(),
        FrameLimits::default(),
        None,
        true,
        CallbackLimits::default(),
    );
    let machine = f.create("Callback", &definition(json!({}))).await;
    let execution = f.start(&machine, "panic").await;
    let view = execution.handle().wait(deadline()).await.unwrap();
    assert!(
        matches!(view.outcome().unwrap().as_ref(),ExecutionOutcome::Failed(f) if f.error()==Some("Rustack.InternalError"))
    );
    assert!(matches!(
        execution.handle().wait_cleanup(deadline()).await,
        Err(ExecutionError::Quarantined)
    ));
    assert!(f.owner.shutdown(deadline()).await.is_err());
    f.callbacks.1.shutdown(deadline()).await.unwrap();
    assert!(f.frames.available_frames() < 4096);
}

fn registry_error(error: &RegistryError) -> &RegistryError {
    match error {
        RegistryError::Owner(error) => registry_error(error),
        _ => error,
    }
}

#[tokio::test]
async fn test_should_retry_external_failure_with_new_token_and_captured_retry_context() {
    let mut f = Fixture::new(HistoryLimits::default(), FrameLimits::default());
    let machine = f
        .create(
            "Callback",
            &definition(
                json!({"Retry":[{"ErrorEquals":["Busy"],"IntervalSeconds":1,"MaxAttempts":1}]}),
            ),
        )
        .await;
    let execution = f.start(&machine, "retry").await;
    let (mut first, mut responder) = f.next().await;
    let old = token(&first);
    let metadata = report(&mut first, &mut responder, Ok(()));
    responder.finish(Ok(metadata.payload().unwrap()));
    submitted(&execution).await;
    f.action(
        CallbackCommand::failure(
            old.parse().unwrap(),
            Some("Busy".into()),
            Some("retry".into()),
        )
        .unwrap(),
    )
    .await
    .unwrap();
    let (mut second, mut responder) = f.next().await;
    let fresh = token(&second);
    assert_ne!(old, fresh);
    assert_eq!(
        second.dispatch().parameters().value()["Payload"]["retry"],
        1
    );
    assert!(matches!(
        callback_error(
            &f.action(CallbackCommand::success(old.parse().unwrap(), b"false").unwrap())
                .await
                .unwrap_err()
        ),
        CallbackError::TaskTimedOut
    ));
    let metadata = report(&mut second, &mut responder, Ok(()));
    responder.finish(Ok(metadata.payload().unwrap()));
    f.action(CallbackCommand::success(fresh.parse().unwrap(), b"42").unwrap())
        .await
        .unwrap();
    let view = execution.handle().wait(deadline()).await.unwrap();
    assert!(
        matches!(view.outcome().unwrap().as_ref(),ExecutionOutcome::Succeeded(p) if p.value()==&json!(42))
    );
    let page = view.history().page(1, 1000, true).unwrap();
    let text = serde_json::to_string(page.events()).unwrap();
    assert!(!text.contains(&old) && !text.contains(&fresh));
    assert_eq!(
        page.events()
            .iter()
            .filter(|event| event.r#type == HistoryEventType::TaskScheduled)
            .count(),
        2
    );
    drop((first, second));
    f.close().await;
}
#[tokio::test]
async fn test_should_reset_heartbeat_and_recover_its_expiry_with_timed_out_history() {
    let mut f = Fixture::new(HistoryLimits::default(), FrameLimits::default());
    let mut source = definition(
        json!({"HeartbeatSeconds":2,"TimeoutSeconds":30,"Catch":[{"ErrorEquals":["States.HeartbeatTimeout"],"Next":"Recovered"}]}),
    );
    source["States"]["Recovered"] = json!({"Type":"Pass","Result":{"recovered":true},"End":true});
    let machine = f.create("Callback", &source).await;
    let execution = f.start(&machine, "heartbeat").await;
    let (mut command, mut responder) = f.next().await;
    let secret = token(&command);
    let metadata = report(&mut command, &mut responder, Ok(()));
    responder.finish(Ok(metadata.payload().unwrap()));
    submitted(&execution).await;
    pause();
    advance(Duration::from_secs(1)).await;
    f.action(CallbackCommand::heartbeat(secret.parse().unwrap()))
        .await
        .unwrap();
    advance(Duration::from_secs(1)).await;
    assert!(execution.handle().snapshot().outcome().is_none());
    advance(Duration::from_secs(1)).await;
    resume();
    let view = execution.handle().wait(deadline()).await.unwrap();
    assert!(
        matches!(view.outcome().unwrap().as_ref(),ExecutionOutcome::Succeeded(p) if p.value()==&json!({"recovered":true}))
    );
    let page = view.history().page(1, 1000, true).unwrap();
    assert!(page.events().iter().any(|e| {
        e.task_timed_out_event_details
            .as_ref()
            .is_some_and(|d| d.error.as_deref() == Some("States.HeartbeatTimeout"))
    }));
    drop(command);
    f.close().await;
}
#[tokio::test]
async fn test_should_enforce_absolute_task_timeout_during_invocation_and_join_late_cleanup() {
    let mut f = Fixture::new(HistoryLimits::default(), FrameLimits::default());
    let machine = f
        .create("Callback", &definition(json!({"TimeoutSeconds":2})))
        .await;
    let execution = f.start(&machine, "timeout").await;
    let (command, mut responder) = f.next().await;
    responder.started().unwrap();
    let secret = token(&command);
    pause();
    advance(Duration::from_secs(2)).await;
    resume();
    let view = execution.handle().wait(deadline()).await.unwrap();
    assert!(
        matches!(view.outcome().unwrap().as_ref(),ExecutionOutcome::Failed(f) if f.error()==Some("States.Timeout"))
    );
    assert_eq!(view.cleanup(), CleanupState::Pending);
    assert!(
        f.action(CallbackCommand::heartbeat(secret.parse().unwrap()))
            .await
            .is_err()
    );
    responder.finish(Err(TaskFailure::service_exception()));
    execution.handle().wait_cleanup(deadline()).await.unwrap();
    drop(command);
    f.close().await;
}
#[tokio::test]
async fn test_should_cancel_callback_sibling_and_join_physical_cleanup_before_parallel_catch() {
    let mut f = Fixture::new(HistoryLimits::default(), FrameLimits::default());
    let source = json!({"StartAt":"Both","States":{"Both":{"Type":"Parallel","Branches":[definition(json!({})),definition(json!({}))],"Catch":[{"ErrorEquals":["States.ALL"],"Next":"Recovered"}],"End":true},"Recovered":{"Type":"Pass","Result":{"recovered":true},"End":true}}});
    let machine = f.create("Callback", &source).await;
    let execution = f.start(&machine, "siblings").await;
    let (mut first, mut responder) = f.next().await;
    let failed = token(&first);
    let metadata = report(&mut first, &mut responder, Ok(()));
    responder.finish(Ok(metadata.payload().unwrap()));
    let (second, mut responder) = f.next().await;
    let cancelled_token = token(&second);
    responder.started().unwrap();
    let mut cancelled = responder.cancellation();
    f.action(
        CallbackCommand::failure(failed.parse().unwrap(), Some("Failure".into()), None).unwrap(),
    )
    .await
    .unwrap();
    timeout_at(deadline(), cancelled.wait_for(|v| *v))
        .await
        .unwrap()
        .unwrap();
    assert!(
        timeout(
            Duration::from_millis(5),
            execution.handle().wait(deadline())
        )
        .await
        .is_err()
    );
    assert!(execution.handle().snapshot().outcome().is_none());
    assert!(
        f.action(CallbackCommand::success(cancelled_token.parse().unwrap(), b"true").unwrap())
            .await
            .is_err()
    );
    responder.finish(Err(TaskFailure::service_exception()));
    let view = execution.handle().wait(deadline()).await.unwrap();
    assert!(
        matches!(view.outcome().unwrap().as_ref(),ExecutionOutcome::Succeeded(p) if p.value()==&json!({"recovered":true}))
    );
    drop((first, second));
    f.close().await;
}
#[tokio::test]
async fn test_should_complete_registered_map_callbacks_out_of_order_with_stable_results() {
    let mut f = Fixture::new(HistoryLimits::default(), FrameLimits::default());
    let source = json!({"StartAt":"Items","States":{"Items":{"Type":"Map","ItemsPath":"$.items","ItemSelector":{"n.$":"$$.Map.Item.Value"},"ItemProcessor":definition(json!({})),"End":true}}});
    let machine = f.create("Callback", &source).await;
    let RegistryResult::Execution(execution) = f
        .registry
        .submit(RegistryCommand::Start(
            machine,
            Some("map".parse().unwrap()),
            Payload::parse(br#"{"items":[21,22,23]}"#).unwrap(),
            None,
        ))
        .unwrap()
        .wait(deadline())
        .await
        .unwrap()
    else {
        panic!("execution");
    };
    let mut completions = Vec::new();
    for _ in 0..3 {
        let (mut command, mut responder) = f.next().await;
        let n = command.dispatch().parameters().value()["Payload"]["n"]
            .as_u64()
            .unwrap();
        let secret = token(&command);
        let metadata = report(&mut command, &mut responder, Ok(()));
        responder.finish(Ok(metadata.payload().unwrap()));
        completions.push((secret, n));
    }
    for (secret, n) in completions.iter().rev() {
        f.action(
            CallbackCommand::success(
                secret.parse().unwrap(),
                &serde_json::to_vec(&json!(n * 2)).unwrap(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    }
    let view = execution.handle().wait(deadline()).await.unwrap();
    assert!(
        matches!(view.outcome().unwrap().as_ref(),ExecutionOutcome::Succeeded(p) if p.value()==&json!([42,44,46]))
    );
    let text = serde_json::to_string(view.history().page(1, 1000, true).unwrap().events()).unwrap();
    assert!(completions.iter().all(|(secret, _)| !text.contains(secret)));
    f.close().await;
}
fn callback_error(error: &CallbackError) -> &CallbackError {
    match error {
        CallbackError::Cached(error) => callback_error(error),
        _ => error,
    }
}

#[tokio::test]
async fn test_should_require_real_port_committed_registration_and_matching_callback_scope() {
    let (callback, mut callback_owner) =
        CallbackRegistry::new(TenantScope::default(), CallbackLimits::default()).unwrap();
    let mut pure = ExecutionSupervisor::new(
        RuntimeLimits::default(),
        HistoryLimits::default(),
        256 * 1024 * 1024,
        None,
    )
    .unwrap();
    assert!(matches!(
        pure.install_callbacks(callback.clone()),
        Err(ExecutionError::Unavailable)
    ));
    assert!(pure.callback_scope().is_none());
    pure.drain(deadline()).await.unwrap();
    let (mut runtime, _, port) = fixture(8, 1, HistoryLimits::default());
    runtime.install_callbacks(callback.clone()).unwrap();
    let source = definition(json!({}));
    let machine = AslCompiler::new(TenantScope::default(), TaskSupport::Lambda)
        .with_callback_support(true)
        .compile(&serde_json::to_vec(&source).unwrap())
        .unwrap()
        .machine()
        .clone();
    let input = Payload::parse(br#"{"n":21}"#).unwrap();
    let driver = ExecutionDriver::for_structured_owner(
        machine,
        input.clone(),
        EvaluationContext::from(&input),
        ClockSample::new(Instant::now().into_std(), Utc::now()),
        ExecutionLimits::default(),
    )
    .unwrap();
    assert!(matches!(
        runtime.start_structured(driver),
        Err(ExecutionError::Unavailable)
    ));
    let foreign = TenantScope::new("aws", "us-west-2", "000000000000").unwrap();
    assert!(matches!(
        Registry::new(
            foreign,
            RegistryLimits::default(),
            runtime,
            ExecutionLimits::default()
        ),
        Err(RegistryError::Validation)
    ));
    assert_eq!(port.starts.load(Ordering::SeqCst), 0);
    assert_eq!(callback.status().retained, 0);
    callback_owner.shutdown(deadline()).await.unwrap();
}
#[tokio::test]
async fn test_should_seal_callback_configuration_after_completed_work_and_reflect_root_health() {
    let (callback, mut callback_owner) =
        CallbackRegistry::new(TenantScope::default(), CallbackLimits::default()).unwrap();
    let (mut runtime, _, _) = fixture(8, 1, HistoryLimits::default());
    let mut handle = runtime
        .start(driver(
            &json!({"StartAt":"Done","States":{"Done":{"Type":"Succeed"}}}),
            &json!({}),
        ))
        .unwrap();
    handle.wait_cleanup(deadline()).await.unwrap();
    runtime.reap_ready();
    assert!(matches!(
        runtime.install_callbacks(callback.clone()),
        Err(ExecutionError::Invalid)
    ));
    assert!(runtime.callback_scope().is_none());
    runtime.drain(deadline()).await.unwrap();
    let (mut bound, _, _) = fixture(8, 1, HistoryLimits::default());
    bound.install_callbacks(callback.clone()).unwrap();
    assert!(matches!(
        bound.install_callbacks(callback.clone()),
        Err(ExecutionError::Invalid)
    ));
    assert!(bound.is_ready());
    callback_owner.shutdown(deadline()).await.unwrap();
    assert!(!bound.is_ready());
    bound.drain(deadline()).await.unwrap();
    let (mut fresh, _, _) = fixture(8, 1, HistoryLimits::default());
    assert!(matches!(
        fresh.install_callbacks(callback),
        Err(ExecutionError::Unavailable)
    ));
    assert!(fresh.callback_scope().is_none());
    fresh.drain(deadline()).await.unwrap();
}
#[tokio::test]
async fn test_should_bound_preparation_bytes_after_mint_and_revoke_without_issuing() {
    let mut f = Fixture::new(
        HistoryLimits::default(),
        FrameLimits::new(8, crate::MAX_PAYLOAD_BYTES + 65_536 + 4096).unwrap(),
    );
    let machine = f.create("Callback", &definition(json!({}))).await;
    let execution = f.start(&machine, "quota").await;
    let view = execution.handle().wait(deadline()).await.unwrap();
    assert!(
        matches!(view.outcome().unwrap().as_ref(),ExecutionOutcome::Failed(f) if f.error()==Some("Rustack.ResultLimitExceeded"))
    );
    execution.handle().wait_cleanup(deadline()).await.unwrap();
    assert_eq!(f.calls.1.starts.load(Ordering::SeqCst), 0);
    assert_eq!(f.callbacks.0.status().live, 0);
    assert_eq!(f.callbacks.0.status().retained, 1);
    f.close().await;
}

#[tokio::test]
async fn test_should_retire_proved_physical_call_even_when_callback_root_cleanup_is_unknown() {
    let mut f = Fixture::new(HistoryLimits::default(), FrameLimits::default());
    let callback = f.create("Callback", &definition(json!({}))).await;
    let ordinary=f.create("Ordinary",&json!({"StartAt":"Call","States":{"Call":{"Type":"Task","Resource":"arn:aws:states:::lambda:invoke","Parameters":{"FunctionName":"Worker"},"End":true}}})).await;
    let first = f.start(&callback, "first").await;
    let (mut command, mut responder) = f.next().await;
    let metadata = report(&mut command, &mut responder, Ok(()));
    submitted(&first).await;
    f.callbacks.1.shutdown(deadline()).await.unwrap();
    let view = first.handle().wait(deadline()).await.unwrap();
    assert!(
        matches!(view.outcome().unwrap().as_ref(),ExecutionOutcome::Failed(f) if f.error()==Some("Rustack.InternalError"))
    );
    assert_eq!(view.cleanup(), CleanupState::Pending);
    let second = f.start(&ordinary, "second").await;
    assert!(
        timeout(Duration::from_millis(5), f.calls.0.recv())
            .await
            .is_err()
    );
    responder.finish(Ok(metadata.payload().unwrap()));
    let (_, mut normal) = f.next().await;
    normal.started().unwrap();
    normal.finish(Ok(Payload::parse(b"true").unwrap()));
    second.handle().wait(deadline()).await.unwrap();
    assert!(matches!(
        first.handle().wait_cleanup(deadline()).await,
        Err(ExecutionError::Quarantined)
    ));
    assert!(f.owner.shutdown(deadline()).await.is_err());
}

#[tokio::test]
async fn test_should_classify_token_capacity_separately_without_revoking_an_existing_waiter() {
    let limits = CallbackLimits::new(1, 8, 8, 1, Duration::from_secs(60)).unwrap();
    let mut f = Fixture::configured(
        HistoryLimits::default(),
        FrameLimits::default(),
        None,
        false,
        limits,
    );
    let machine = f.create("Callback", &definition(json!({}))).await;
    let first = f.start(&machine, "waiting").await;
    let (mut command, mut responder) = f.next().await;
    let secret = token(&command);
    let metadata = report(&mut command, &mut responder, Ok(()));
    responder.finish(Ok(metadata.payload().unwrap()));
    submitted(&first).await;
    let second = f.start(&machine, "over-capacity").await;
    let view = second.handle().wait(deadline()).await.unwrap();
    assert!(
        matches!(view.outcome().unwrap().as_ref(),ExecutionOutcome::Failed(f) if f.error()==Some("Rustack.CallbackLimitExceeded"))
    );
    assert_eq!(f.calls.1.starts.load(Ordering::SeqCst), 1);
    assert!(first.handle().snapshot().outcome().is_none());
    f.action(CallbackCommand::success(secret.parse().unwrap(), b"42").unwrap())
        .await
        .unwrap();
    first.handle().wait(deadline()).await.unwrap();
    drop(command);
    f.close().await;
}
