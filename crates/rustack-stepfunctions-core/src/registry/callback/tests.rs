//! Actual captured execution ownership and explicit-clock callback protocol tests.

use std::{
    error::Error,
    future::{Future, poll_fn},
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::Poll,
    time::Duration,
};

use chrono::Utc;
use rstest::rstest;
use serde_json::json;
use tokio::{
    sync::{Notify, Semaphore},
    time::{Instant, advance, pause, resume, timeout},
};

use super::*;
use crate::{
    AslCompiler, ClockSample, DataFlow, DriverStep, EvaluationContext, ExecutionDriver,
    ExecutionLimits, ExecutionSnapshot, ExecutionSupervisor, FrameCallId, FrameCapacity,
    FrameLimits, FrameScheduler, FrameUpdate, HistoryLimits, Payload, PreparedDefinition, Registry,
    RegistryCommand, RegistryLimits, RegistryResult, RegistrySupervisor, RuntimeLimits,
    SystemUuidSource, Tags, TaskSupport,
};

type TestResult = Result<(), Box<dyn Error>>;
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}
#[derive(Debug)]
pub(super) struct EntropyHook {
    entered: Notify,
    released: AtomicBool,
    panic: bool,
}
impl EntropyHook {
    fn new(panic: bool) -> Arc<Self> {
        Arc::new(Self {
            entered: Notify::new(),
            released: AtomicBool::new(false),
            panic,
        })
    }
    pub(super) fn run(&self) {
        self.entered.notify_one();
        while !self.released.load(Ordering::Acquire) {
            std::thread::park_timeout(Duration::from_millis(1));
        }
        assert!(!self.panic, "controlled callback entropy panic");
    }
    fn release(&self) {
        self.released.store(true, Ordering::Release);
    }
}
#[derive(Debug)]
struct Gate(Arc<EntropyHook>);
impl Drop for Gate {
    fn drop(&mut self) {
        self.0.release();
    }
}
struct Fixture {
    callbacks: CallbackRegistry,
    owner: CallbackSupervisor,
    registry: Registry,
    registry_owner: RegistrySupervisor,
    parent: ExecutionSnapshot,
    calls: Vec<FrameCallId>,
}
impl Fixture {
    async fn new(limits: CallbackLimits) -> Result<Self, Box<dyn Error>> {
        let runtime = ExecutionSupervisor::new(
            RuntimeLimits::default(),
            HistoryLimits::default(),
            256 * 1024 * 1024,
            None,
        )?;
        let (registry, registry_owner) = Registry::new(
            TenantScope::default(),
            RegistryLimits::default(),
            runtime,
            ExecutionLimits::default(),
        )?;
        let definition = AslCompiler::default().compile(
            br#"{"StartAt":"W","States":{"W":{"Type":"Wait","Seconds":3600,"End":true}}}"#,
        )?;
        let RegistryResult::Machine(machine) = registry
            .submit(RegistryCommand::Create(
                "Parent".parse()?,
                PreparedDefinition::new(definition.machine().clone()),
                "arn:aws:iam::000000000000:role/Worker".parse()?,
                Tags::default(),
            ))?
            .wait(deadline())
            .await?
        else {
            return Err("parent machine".into());
        };
        let RegistryResult::Execution(parent) = registry
            .submit(RegistryCommand::Start(
                machine.arn().clone(),
                Some("Run".parse()?),
                Payload::parse(b"{}")?,
                None,
            ))?
            .wait(deadline())
            .await?
        else {
            return Err("parent execution".into());
        };
        let (callbacks, owner) = CallbackRegistry::new(TenantScope::default(), limits)?;
        Ok(Self {
            callbacks,
            owner,
            registry,
            registry_owner,
            parent,
            calls: calls()?,
        })
    }
    fn attempt(
        &self,
        index: usize,
        seconds: u64,
        heartbeat: Option<u64>,
    ) -> Result<CallbackAttempt, CallbackError> {
        CallbackAttempt::new(
            &self.parent,
            *self.calls.get(index).ok_or(CallbackError::Validation)?,
            Instant::now() + Duration::from_secs(seconds),
            heartbeat.map(Duration::from_secs),
        )
    }
    async fn task(
        &self,
        index: usize,
        seconds: u64,
        heartbeat: Option<u64>,
    ) -> Result<CallbackTask, Box<dyn Error>> {
        let mut mint = self
            .callbacks
            .register(self.attempt(index, seconds, heartbeat)?)?;
        mint.admitted(deadline()).await?;
        Ok(mint.into_task()?)
    }
    async fn close(&mut self) -> TestResult {
        let a = self.owner.shutdown(deadline()).await;
        let b = self.registry_owner.shutdown(deadline()).await;
        a?;
        b?;
        Ok(())
    }
}
fn calls() -> Result<Vec<FrameCallId>, Box<dyn Error>> {
    let definition = json!({"StartAt":"A","States":{"A":{"Type":"Task","Resource":"arn:aws:lambda:us-east-1:000000000000:function:Worker","Next":"B"},"B":{"Type":"Task","Resource":"arn:aws:lambda:us-east-1:000000000000:function:Worker","End":true}}});
    let machine = AslCompiler::new(TenantScope::default(), TaskSupport::Lambda)
        .compile(&serde_json::to_vec(&definition)?)?
        .machine()
        .clone();
    let now = ClockSample::new(Instant::now().into_std(), Utc::now());
    let driver = ExecutionDriver::for_structured_owner(
        machine,
        Payload::parse(b"{}")?,
        EvaluationContext::from_value(&json!({}))?,
        now,
        ExecutionLimits::default(),
    )?;
    let mut frames = FrameScheduler::new(
        driver,
        Arc::new(FrameCapacity::default()),
        FrameLimits::default(),
    )?;
    let mut calls = Vec::new();
    let mut source = SystemUuidSource::default();
    for _ in 0..100 {
        if let FrameUpdate::Transition(t) = frames.advance(now, &mut source) {
            if matches!(t.step(), DriverStep::Dispatch(_)) {
                let call = t.call().ok_or("call identity")?;
                calls.push(call);
                frames.complete(call, Ok(Payload::parse(b"null")?), now, &mut source)?;
                frames.retired(call)?;
            }
        }
        if calls.len() == 2 {
            return Ok(calls);
        }
    }
    Err("missing actual frame attempts".into())
}
fn token(task: &CallbackTask) -> CallbackToken {
    task.token().expose().parse().unwrap()
}
fn underlying(error: &CallbackError) -> &CallbackError {
    match error {
        CallbackError::Cached(error) => underlying(error),
        other => other,
    }
}
async fn action(
    registry: &CallbackRegistry,
    command: CallbackCommand,
) -> Result<(), CallbackError> {
    registry
        .submit(&TenantScope::default(), command)?
        .wait(deadline())
        .await
}
async fn activated(task: &mut CallbackTask) -> TestResult {
    timeout(
        Duration::from_secs(5),
        task.view
            .wait_for(|v| matches!(v.as_ref(), CallbackOutcome::Waiting { .. })),
    )
    .await??;
    Ok(())
}
async fn inspect(registry: &CallbackRegistry) -> Result<(usize, usize, usize), Box<dyn Error>> {
    let (sender, receiver) = oneshot::channel();
    registry
        .shared
        .sender
        .send(Request::Inspect(sender))
        .await?;
    Ok(timeout(Duration::from_secs(5), receiver).await??)
}

#[rstest]
#[case(false, false)]
#[case(false, true)]
#[case(true, false)]
#[case(true, true)]
#[tokio::test]
async fn test_should_commit_early_or_late_success_and_failure_only_after_invocation_success(
    #[case] early: bool,
    #[case] failure: bool,
) -> TestResult {
    let mut f = Fixture::new(CallbackLimits::default()).await?;
    let mut task = f.task(0, 60, None).await?;
    if !early {
        task.invocation_complete(Ok(()))?;
        activated(&mut task).await?;
    }
    let command = if failure {
        CallbackCommand::failure(
            token(&task),
            Some("BusinessFailure".into()),
            Some(String::new()),
        )?
    } else {
        CallbackCommand::success(token(&task), b" { \"answer\":42 } ")?
    };
    action(&f.callbacks, command).await?;
    if early {
        assert!(matches!(
            task.snapshot().as_ref(),
            CallbackOutcome::Invoking
        ));
        task.invocation_complete(Ok(()))?;
    }
    let outcome = task.wait(deadline()).await?;
    match outcome.as_ref() {
        CallbackOutcome::Succeeded(output) if !failure => {
            assert_eq!(output.original_bytes(), b" { \"answer\":42 } ");
        }
        CallbackOutcome::Failed(failure) => {
            assert_eq!(failure.details().error(), Some("BusinessFailure"));
            assert_eq!(failure.details().cause(), Some(""));
        }
        other => return Err(format!("unexpected outcome: {other:?}").into()),
    }
    assert!(matches!(
        underlying(
            &action(
                &f.callbacks,
                CallbackCommand::success(token(&task), b"true")?
            )
            .await
            .unwrap_err()
        ),
        CallbackError::TaskTimedOut
    ));
    assert!(matches!(
        underlying(
            &action(&f.callbacks, CallbackCommand::heartbeat(token(&task)))
                .await
                .unwrap_err()
        ),
        CallbackError::TaskTimedOut
    ));
    f.close().await
}
#[tokio::test]
async fn test_should_discard_early_callback_when_lambda_invocation_fails() -> TestResult {
    let mut f = Fixture::new(CallbackLimits::default()).await?;
    let mut task = f.task(0, 60, None).await?;
    action(&f.callbacks, CallbackCommand::success(token(&task), b"42")?).await?;
    assert!(matches!(
        underlying(
            &action(&f.callbacks, CallbackCommand::heartbeat(token(&task)))
                .await
                .unwrap_err()
        ),
        CallbackError::TaskTimedOut
    ));
    task.invocation_complete(Err(TaskFailure::new(
        "Lambda.ServiceException",
        Some("invoke failed"),
    )?))?;
    assert!(
        matches!(task.wait(deadline()).await?.as_ref(),CallbackOutcome::InvocationFailed(f) if f.details().error()==Some("Lambda.ServiceException"))
    );
    assert!(matches!(
        task.invocation_complete(Ok(())),
        Err(CallbackError::Progress)
    ));
    f.close().await
}
#[tokio::test]
async fn test_should_start_heartbeat_only_after_successful_invocation_and_replace_one_timer()
-> TestResult {
    let mut f = Fixture::new(CallbackLimits::default()).await?;
    let mut task = f.task(0, 60, Some(2)).await?;
    pause();
    advance(Duration::from_secs(5)).await;
    action(&f.callbacks, CallbackCommand::heartbeat(token(&task))).await?;
    assert!(matches!(
        task.snapshot().as_ref(),
        CallbackOutcome::Invoking
    ));
    task.invocation_complete(Ok(()))?;
    activated(&mut task).await?;
    advance(Duration::from_secs(1)).await;
    for _ in 0..200 {
        advance(Duration::from_millis(1)).await;
        action(&f.callbacks, CallbackCommand::heartbeat(token(&task))).await?;
    }
    assert_eq!(inspect(&f.callbacks).await?, (1, 1, 1));
    advance(Duration::from_secs(2)).await;
    assert!(matches!(
        task.wait(deadline()).await?.as_ref(),
        CallbackOutcome::TimedOut(CallbackTimeout::Heartbeat)
    ));
    assert!(matches!(
        underlying(
            &action(&f.callbacks, CallbackCommand::heartbeat(token(&task)))
                .await
                .unwrap_err()
        ),
        CallbackError::TaskTimedOut
    ));
    f.close().await
}
#[tokio::test]
async fn test_should_preserve_absolute_task_deadline_under_heartbeats_and_early_callback()
-> TestResult {
    let mut f = Fixture::new(CallbackLimits::default()).await?;
    let mut task = f.task(0, 3, Some(10)).await?;
    pause();
    task.invocation_complete(Ok(()))?;
    activated(&mut task).await?;
    for _ in 0..2 {
        advance(Duration::from_secs(1)).await;
        action(&f.callbacks, CallbackCommand::heartbeat(token(&task))).await?;
    }
    advance(Duration::from_secs(1)).await;
    assert!(matches!(
        task.wait(deadline()).await?.as_ref(),
        CallbackOutcome::TimedOut(CallbackTimeout::Task)
    ));
    resume();
    let mut early = f.task(1, 2, None).await?;
    pause();
    action(
        &f.callbacks,
        CallbackCommand::success(token(&early), b"true")?,
    )
    .await?;
    advance(Duration::from_secs(2)).await;
    assert!(matches!(
        early.wait(deadline()).await?.as_ref(),
        CallbackOutcome::TimedOut(CallbackTimeout::Task)
    ));
    assert!(matches!(
        early.invocation_complete(Ok(())),
        Err(CallbackError::TaskTimedOut)
    ));
    f.close().await
}
#[tokio::test]
async fn test_should_have_no_heartbeat_timer_when_unconfigured() -> TestResult {
    let mut f = Fixture::new(CallbackLimits::default()).await?;
    let mut task = f.task(0, 100, None).await?;
    pause();
    task.invocation_complete(Ok(()))?;
    activated(&mut task).await?;
    advance(Duration::from_secs(50)).await;
    action(&f.callbacks, CallbackCommand::heartbeat(token(&task))).await?;
    assert!(matches!(
        task.snapshot().as_ref(),
        CallbackOutcome::Waiting {
            heartbeat_deadline: None
        }
    ));
    action(
        &f.callbacks,
        CallbackCommand::success(token(&task), b"null")?,
    )
    .await?;
    assert!(
        matches!(task.wait(deadline()).await?.as_ref(),CallbackOutcome::Succeeded(v) if v.value().is_null())
    );
    f.close().await
}
#[tokio::test]
async fn test_should_revoke_independently_of_saturated_acknowledgment_capacity() -> TestResult {
    let mut f = Fixture::new(CallbackLimits::new(2, 4, 1, 1, Duration::from_secs(60))?).await?;
    let mut task = f.task(0, 60, None).await?;
    let ack = f.callbacks.submit(
        &TenantScope::default(),
        CallbackCommand::heartbeat(token(&task)),
    )?;
    assert!(matches!(
        f.callbacks.submit(
            &TenantScope::default(),
            CallbackCommand::heartbeat(token(&task))
        ),
        Err(CallbackError::Capacity)
    ));
    task.revoke();
    assert!(matches!(
        task.wait(deadline()).await?.as_ref(),
        CallbackOutcome::Revoked
    ));
    assert!(matches!(
        f.callbacks.submit(
            &TenantScope::default(),
            CallbackCommand::heartbeat(token(&task))
        ),
        Err(CallbackError::Capacity)
    ));
    drop(ack);
    assert!(matches!(
        underlying(
            &action(&f.callbacks, CallbackCommand::heartbeat(token(&task)))
                .await
                .unwrap_err()
        ),
        CallbackError::TaskTimedOut
    ));
    f.close().await
}
#[tokio::test]
async fn test_should_revoke_on_parent_stop_and_isolate_old_generation_from_new_attempt()
-> TestResult {
    let mut f = Fixture::new(CallbackLimits::default()).await?;
    let mut old = f.task(0, 60, None).await?;
    assert!(matches!(
        underlying(&{
            let mut mint = f.callbacks.register(f.attempt(0, 60, None)?)?;
            mint.admitted(deadline()).await.unwrap_err()
        }),
        CallbackError::AttemptExists
    ));
    let mut next = f.task(1, 60, None).await?;
    assert_eq!(f.calls[0].frame(), f.calls[1].frame());
    assert_ne!(f.calls[0].attempt(), f.calls[1].attempt());
    old.revoke();
    assert!(matches!(
        old.wait(deadline()).await?.as_ref(),
        CallbackOutcome::Revoked
    ));
    assert!(matches!(
        underlying(
            &action(
                &f.callbacks,
                CallbackCommand::success(token(&old), b"true")?
            )
            .await
            .unwrap_err()
        ),
        CallbackError::TaskTimedOut
    ));
    assert!(matches!(
        next.snapshot().as_ref(),
        CallbackOutcome::Invoking
    ));
    f.parent
        .handle()
        .request_stop(TaskFailure::new("Stopped", None)?);
    f.parent.handle().wait(deadline()).await?;
    assert!(matches!(
        next.wait(deadline()).await?.as_ref(),
        CallbackOutcome::Revoked
    ));
    assert!(matches!(
        f.attempt(0, 60, None),
        Err(CallbackError::TaskTimedOut)
    ));
    f.close().await
}
#[tokio::test]
async fn test_should_classify_forged_foreign_consumed_and_malformed_tokens_without_exposure()
-> TestResult {
    let mut f = Fixture::new(CallbackLimits::default()).await?;
    let mut task = f.task(0, 60, None).await?;
    let foreign = TenantScope::new("aws", "us-west-2", "000000000000")?;
    assert!(matches!(
        f.callbacks
            .submit(&foreign, CallbackCommand::heartbeat(token(&task))),
        Err(CallbackError::InvalidToken)
    ));
    let unknown = CallbackToken::from_str(&"0".repeat(64))?;
    assert!(matches!(
        underlying(
            &action(&f.callbacks, CallbackCommand::heartbeat(unknown))
                .await
                .unwrap_err()
        ),
        CallbackError::InvalidToken
    ));
    for value in [
        String::new(),
        "a".repeat(63),
        "A".repeat(64),
        "g".repeat(64),
        "a".repeat(2049),
    ] {
        assert!(matches!(
            value.parse::<CallbackToken>(),
            Err(CallbackError::InvalidToken)
        ));
    }
    let raw = task.token().expose().to_owned();
    assert_eq!(raw.len(), 64);
    assert!(
        raw.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );
    let command = CallbackCommand::success(token(&task), b"true")?;
    for rendered in [
        format!("{task:?}"),
        format!("{:?}", f.callbacks),
        format!("{:?}", task.token()),
        format!("{command:?}"),
        format!("{:?}", task.token().digest()),
    ] {
        assert!(!rendered.contains(&raw));
    }
    task.revoke();
    task.wait(deadline()).await?;
    assert!(matches!(
        f.callbacks
            .submit(&foreign, CallbackCommand::heartbeat(token(&task))),
        Err(CallbackError::InvalidToken)
    ));
    f.close().await
}
#[tokio::test]
async fn test_should_continue_accepted_callback_after_ack_waiter_loss_and_task_observer_expiry()
-> TestResult {
    let mut f = Fixture::new(CallbackLimits::default()).await?;
    let mut task = f.task(0, 60, None).await?;
    assert!(matches!(
        task.wait(Instant::now()).await,
        Err(CallbackError::Deadline)
    ));
    task.invocation_complete(Ok(()))?;
    drop(f.callbacks.submit(
        &TenantScope::default(),
        CallbackCommand::success(token(&task), b"42")?,
    )?);
    assert!(
        matches!(task.wait(deadline()).await?.as_ref(),CallbackOutcome::Succeeded(v) if v.value()==&json!(42))
    );
    f.close().await
}
#[tokio::test]
async fn test_should_keep_closed_classification_until_parent_execution_retention_expires()
-> TestResult {
    let mut f = Fixture::new(CallbackLimits::new(1, 1, 4, 1, Duration::from_secs(2))?).await?;
    let mut task = f.task(0, 60, None).await?;
    task.invocation_complete(Ok(()))?;
    action(
        &f.callbacks,
        CallbackCommand::success(token(&task), b"true")?,
    )
    .await?;
    task.wait(deadline()).await?;
    assert!(matches!(
        f.callbacks.register(f.attempt(1, 60, None)?),
        Err(CallbackError::Capacity)
    ));
    let RegistryResult::Execution(next_parent) = f
        .registry
        .submit(RegistryCommand::Start(
            f.parent.arn().machine().clone(),
            Some("Next".parse()?),
            Payload::parse(b"{}")?,
            None,
        ))?
        .wait(deadline())
        .await?
    else {
        return Err("next parent".into());
    };
    pause();
    advance(Duration::from_secs(100)).await;
    assert!(matches!(
        underlying(
            &action(&f.callbacks, CallbackCommand::heartbeat(token(&task)))
                .await
                .unwrap_err()
        ),
        CallbackError::TaskTimedOut
    ));
    assert_eq!(inspect(&f.callbacks).await?, (1, 0, 0));
    f.parent
        .handle()
        .request_stop(TaskFailure::new("Stopped", None)?);
    f.parent.handle().wait(deadline()).await?;
    for _ in 0..3 {
        advance(Duration::from_secs(1)).await;
        if inspect(&f.callbacks).await?.2 == 1 {
            break;
        }
    }
    assert_eq!(inspect(&f.callbacks).await?, (1, 0, 1));
    advance(Duration::from_secs(2)).await;
    assert_eq!(inspect(&f.callbacks).await?, (0, 0, 0));
    assert!(matches!(
        underlying(
            &action(&f.callbacks, CallbackCommand::heartbeat(token(&task)))
                .await
                .unwrap_err()
        ),
        CallbackError::InvalidToken
    ));
    resume();
    let mut mint = f.callbacks.register(CallbackAttempt::new(
        &next_parent,
        f.calls[0],
        Instant::now() + Duration::from_secs(60),
        None,
    )?)?;
    mint.admitted(deadline()).await?;
    let replacement = mint.into_task()?;
    assert!(matches!(
        replacement.snapshot().as_ref(),
        CallbackOutcome::Invoking
    ));
    f.close().await
}
#[tokio::test]
async fn test_should_retain_entropy_join_and_process_live_control_during_shutdown_timeout()
-> TestResult {
    let mut f = Fixture::new(CallbackLimits::default()).await?;
    let mut task = f.task(0, 60, None).await?;
    let hook = EntropyHook::new(false);
    let _gate = Gate(hook.clone());
    f.callbacks.shared.entropy_hook.set(hook.clone()).unwrap();
    let mut mint = f.callbacks.register(f.attempt(1, 60, None)?)?;
    timeout(Duration::from_secs(5), hook.entered.notified()).await?;
    task.revoke();
    assert!(matches!(
        task.wait(deadline()).await?.as_ref(),
        CallbackOutcome::Revoked
    ));
    assert!(matches!(
        f.owner
            .shutdown(Instant::now() + Duration::from_millis(10))
            .await,
        Err(CallbackError::Deadline)
    ));
    assert!(!f.callbacks.status().ready);
    hook.release();
    f.owner.shutdown(deadline()).await?;
    assert!(matches!(
        underlying(&mint.admitted(deadline()).await.unwrap_err()),
        CallbackError::Unavailable
    ));
    f.close().await
}
#[tokio::test]
async fn test_should_revoke_lost_mint_observer_before_entropy_finishes_without_issuing_token()
-> TestResult {
    let mut f = Fixture::new(CallbackLimits::default()).await?;
    let hook = EntropyHook::new(false);
    let _gate = Gate(hook.clone());
    f.callbacks.shared.entropy_hook.set(hook.clone()).unwrap();
    let mint = f.callbacks.register(f.attempt(0, 60, None)?)?;
    timeout(Duration::from_secs(5), hook.entered.notified()).await?;
    drop(mint);
    hook.release();
    timeout(Duration::from_secs(5), async {
        loop {
            if f.callbacks.shared.live.available_permits() == 4096 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await?;
    assert_eq!(inspect(&f.callbacks).await?, (0, 0, 0));
    f.close().await
}
#[rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn test_should_publish_unavailable_and_preserve_fatal_root_or_entropy_source(
    #[case] entropy: bool,
) -> TestResult {
    let mut f = Fixture::new(CallbackLimits::default()).await?;
    let mut task = f.task(0, 60, None).await?;
    let mut pending = None;
    let gate = if entropy {
        let hook = EntropyHook::new(true);
        f.callbacks.shared.entropy_hook.set(hook.clone()).unwrap();
        pending = Some(f.callbacks.register(f.attempt(1, 60, None)?)?);
        timeout(Duration::from_secs(5), hook.entered.notified()).await?;
        hook.release();
        Some(Gate(hook))
    } else {
        f.callbacks.shared.sender.send(Request::Panic).await?;
        None
    };
    assert!(matches!(
        task.wait(deadline()).await?.as_ref(),
        CallbackOutcome::Unavailable
    ));
    let error = f.owner.shutdown(deadline()).await.unwrap_err();
    assert!(matches!(underlying(&error),CallbackError::Worker(source) if source.is_panic()));
    if let Some(mut mint) = pending {
        assert!(matches!(
            underlying(&mint.admitted(deadline()).await.unwrap_err()),
            CallbackError::OwnerLost
        ));
    }
    assert!(!f.callbacks.status().ready);
    drop(gate);
    f.registry_owner.shutdown(deadline()).await?;
    Ok(())
}
#[test]
fn test_should_validate_limits_output_failure_heartbeat_and_missing_runtime() {
    for limits in [
        (0, 1, 1, 1),
        (2, 1, 1, 1),
        (1, 1, 0, 1),
        (1, 1, 1, 0),
        (65_537, 100_000, 1, 1),
        (1, 1_000_001, 1, 1),
    ] {
        assert!(
            CallbackLimits::new(
                limits.0,
                limits.1,
                limits.2,
                limits.3,
                Duration::from_secs(1)
            )
            .is_err()
        );
    }
    assert!(matches!(
        CallbackRegistry::new(TenantScope::default(), CallbackLimits::default()),
        Err(CallbackError::Unavailable)
    ));
    let secret = "a".repeat(64);
    for bytes in [b"invalid".to_vec(), vec![b' '; 262_145]] {
        assert!(CallbackCommand::success(secret.parse().unwrap(), &bytes).is_err());
    }
    assert!(
        CallbackCommand::failure(secret.parse().unwrap(), Some("x".repeat(257)), None).is_err()
    );
    assert!(
        CallbackCommand::failure(secret.parse().unwrap(), None, Some("x".repeat(32_769))).is_err()
    );
}

#[tokio::test]
async fn test_should_release_terminal_payload_ownership_while_retaining_closed_token_classification()
-> TestResult {
    let mut f = Fixture::new(CallbackLimits::default()).await?;
    let mut task = f.task(0, 60, None).await?;
    let old = token(&task);
    task.invocation_complete(Ok(()))?;
    action(
        &f.callbacks,
        CallbackCommand::success(token(&task), b"{\"data\":42}")?,
    )
    .await?;
    let outcome = task.wait(deadline()).await?;
    let weak = Arc::downgrade(&outcome);
    drop(outcome);
    drop(task);
    assert_eq!(inspect(&f.callbacks).await?.0, 1);
    assert!(
        weak.upgrade().is_none(),
        "tombstone retained completed business output"
    );
    assert!(matches!(
        underlying(
            &action(&f.callbacks, CallbackCommand::heartbeat(old))
                .await
                .unwrap_err()
        ),
        CallbackError::TaskTimedOut
    ));
    f.close().await
}

fn manual_task(control: Arc<Control>) -> CallbackTask {
    let (invocation, _invoked) = watch::channel(None);
    let (_view, reader) = watch::channel(Arc::new(CallbackOutcome::Invoking));
    CallbackTask {
        token: "a".repeat(64).parse().unwrap(),
        control,
        invocation,
        view: reader,
        deadline: Instant::now() + Duration::from_secs(300),
        reported: false,
    }
}
fn control() -> Arc<Control> {
    Arc::new(Control {
        revoked: AtomicBool::new(false),
        wake: Arc::new(Notify::new()),
    })
}
fn permit() -> tokio::sync::OwnedSemaphorePermit {
    Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap()
}
async fn pending_once<F: Future>(future: std::pin::Pin<&mut F>) {
    let mut future = future;
    poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
}
#[rstest]
#[case(0)]
#[case(1)]
#[case(2)]
#[tokio::test]
async fn test_should_cache_late_ready_mint_success_error_and_owner_loss_before_observer_fence(
    #[case] kind: u8,
) -> TestResult {
    let control = control();
    let (sender, receiver) = oneshot::channel();
    let mut mint = CallbackMint::new(receiver, control.clone());
    pause();
    let mut observe = Box::pin(mint.admitted(Instant::now() + Duration::from_secs(1)));
    pending_once(observe.as_mut()).await;
    match kind {
        0 => {
            sender
                .send(MintReply {
                    result: Ok(manual_task(control)),
                    _permit: permit(),
                })
                .unwrap();
        }
        1 => {
            sender
                .send(MintReply {
                    result: Err(Arc::new(CallbackError::Capacity)),
                    _permit: permit(),
                })
                .unwrap();
        }
        _ => drop(sender),
    }
    advance(Duration::from_secs(2)).await;
    assert!(matches!(observe.await, Err(CallbackError::Deadline)));
    if kind == 0 {
        mint.admitted(deadline()).await?;
        let task = mint.into_task()?;
        assert_eq!(task.token().expose().len(), 64);
    } else {
        let error = mint.admitted(deadline()).await.unwrap_err();
        assert!(matches!(
            (kind, underlying(&error)),
            (1, CallbackError::Capacity) | (2, CallbackError::OwnerLost)
        ));
    }
    Ok(())
}
#[rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn test_should_cache_late_ready_api_error_and_owner_loss_without_resending(
    #[case] lost: bool,
) -> TestResult {
    let (sender, receiver) = oneshot::channel();
    let mut ack = CallbackAck::new(receiver);
    pause();
    let mut observe = Box::pin(ack.wait(Instant::now() + Duration::from_secs(1)));
    pending_once(observe.as_mut()).await;
    if lost {
        drop(sender);
    } else {
        sender
            .send(AckReply {
                result: Err(Arc::new(CallbackError::TaskTimedOut)),
                _permit: permit(),
            })
            .unwrap();
    }
    advance(Duration::from_secs(2)).await;
    assert!(matches!(observe.await, Err(CallbackError::Deadline)));
    let error = ack.wait(deadline()).await.unwrap_err();
    assert!(matches!(
        (lost, underlying(&error)),
        (true, CallbackError::OwnerLost) | (false, CallbackError::TaskTimedOut)
    ));
    Ok(())
}
#[tokio::test]
async fn test_should_validate_captured_parent_scope_heartbeat_and_task_deadline_clamp() -> TestResult
{
    let mut f = Fixture::new(CallbackLimits::default()).await?;
    for heartbeat in [
        Duration::ZERO,
        Duration::from_millis(500),
        Duration::from_secs(31_536_001),
    ] {
        assert!(matches!(
            CallbackAttempt::new(&f.parent, f.calls[0], deadline(), Some(heartbeat)),
            Err(CallbackError::Validation)
        ));
    }
    let attempt = f.attempt(0, 1_000_000, None)?;
    assert_eq!(
        attempt.deadline,
        Instant::from_std(f.parent.record.deadline)
    );
    let (foreign, mut owner) = CallbackRegistry::new(
        TenantScope::new("aws", "us-west-2", "000000000000")?,
        CallbackLimits::default(),
    )?;
    assert!(matches!(
        foreign.register(attempt),
        Err(CallbackError::Validation)
    ));
    owner.shutdown(deadline()).await?;
    let first = f.task(0, 60, None).await?;
    let second = f.task(1, 60, None).await?;
    assert_ne!(first.token().digest(), second.token().digest());
    f.close().await
}

#[rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn test_should_fence_late_ready_task_result_and_owner_loss_without_revoking(
    #[case] lost: bool,
) -> TestResult {
    let control = control();
    let mut task = manual_task(control.clone());
    let (sender, reader) = watch::channel(Arc::new(CallbackOutcome::Invoking));
    task.view = reader;
    pause();
    let mut observe = Box::pin(task.wait(Instant::now() + Duration::from_secs(1)));
    pending_once(observe.as_mut()).await;
    if lost {
        drop(sender);
    } else {
        sender.send_replace(Arc::new(CallbackOutcome::Succeeded(Payload::parse(
            b"true",
        )?)));
    }
    advance(Duration::from_secs(2)).await;
    assert!(matches!(observe.await, Err(CallbackError::Deadline)));
    assert!(!control.revoked());
    if lost {
        assert!(matches!(
            task.wait(deadline()).await,
            Err(CallbackError::OwnerLost)
        ));
    } else {
        assert!(matches!(
            task.wait(deadline()).await?.as_ref(),
            CallbackOutcome::Succeeded(_)
        ));
    }
    Ok(())
}
#[tokio::test]
async fn test_should_preserve_mint_after_observer_expiry_and_bound_live_entropy_jobs() -> TestResult
{
    let mut f = Fixture::new(CallbackLimits::new(2, 4, 4, 1, Duration::from_secs(60))?).await?;
    let hook = EntropyHook::new(false);
    let _gate = Gate(hook.clone());
    f.callbacks.shared.entropy_hook.set(hook.clone()).unwrap();
    let mut mint = f.callbacks.register(f.attempt(0, 60, None)?)?;
    timeout(Duration::from_secs(5), hook.entered.notified()).await?;
    assert!(matches!(
        mint.admitted(Instant::now() + Duration::from_millis(10))
            .await,
        Err(CallbackError::Deadline)
    ));
    let mut excess = f.callbacks.register(f.attempt(1, 60, None)?)?;
    assert!(matches!(
        underlying(&excess.admitted(deadline()).await.unwrap_err()),
        CallbackError::Capacity
    ));
    drop(excess);
    hook.release();
    mint.admitted(deadline()).await?;
    let mut task = mint.into_task()?;
    task.invocation_complete(Ok(()))?;
    action(&f.callbacks, CallbackCommand::success(token(&task), b"42")?).await?;
    assert!(matches!(
        task.wait(deadline()).await?.as_ref(),
        CallbackOutcome::Succeeded(_)
    ));
    f.close().await
}

#[tokio::test]
async fn test_should_drain_unique_root_after_controller_drop_without_aborting_receipts()
-> TestResult {
    let mut f = Fixture::new(CallbackLimits::default()).await?;
    let mut task = f.task(0, 60, None).await?;
    drop(f.owner);
    assert!(matches!(
        task.wait(deadline()).await?.as_ref(),
        CallbackOutcome::Unavailable
    ));
    timeout(Duration::from_secs(5), async {
        while f.callbacks.shared.status.has_changed().is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    assert!(!f.callbacks.status().ready);
    f.registry_owner.shutdown(deadline()).await?;
    Ok(())
}
#[test]
fn test_should_preserve_unavailable_receipts_and_typed_root_failure_after_runtime_loss()
-> TestResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let (mut f, mut task) = runtime.block_on(async {
        let f = Fixture::new(CallbackLimits::default()).await?;
        let task = f.task(0, 60, None).await?;
        Ok::<_, Box<dyn Error>>((f, task))
    })?;
    drop(runtime);
    let observer = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    observer.block_on(async {
        assert!(matches!(
            task.wait(deadline()).await?.as_ref(),
            CallbackOutcome::Unavailable
        ));
        assert!(!f.callbacks.status().ready);
        let error = f.owner.shutdown(deadline()).await.unwrap_err();
        assert!(
            matches!(underlying(&error),CallbackError::Worker(source) if source.is_cancelled())
        );
        assert!(f.registry_owner.shutdown(deadline()).await.is_err());
        Ok::<_, Box<dyn Error>>(())
    })
}

#[tokio::test]
async fn test_should_prepare_real_registered_token_once_with_separate_safe_history() -> TestResult {
    let mut f = Fixture::new(CallbackLimits::default()).await?;
    let mut task = f.task(0, 60, None).await?;
    let original = Payload::parse(b"{\"n\":21}")?;
    let base = EvaluationContext::from_value(&json!({"Execution":{"Name":"Run"}}))?;
    let flow = DataFlow::compile(
        &json!({"Parameters":{"FunctionName":"Worker","Payload":{"token.$":"$$.Task.Token","n.$":"$.n"}}}),
    )?;
    let parameters = flow.prepare_callback_with_source(
        &original,
        &base,
        task.token(),
        &mut SystemUuidSource::default(),
    )?;
    let exposed: CallbackToken = parameters.outbound().value()["Payload"]["token"]
        .as_str()
        .ok_or("outbound token")?
        .parse()?;
    assert_eq!(exposed.digest(), task.token().digest());
    assert!(parameters.history().payload().value()["Payload"]["token"].is_null());
    assert_eq!(parameters.history().payload().value()["Payload"]["n"], 21);
    assert!(matches!(
        task.snapshot().as_ref(),
        CallbackOutcome::Invoking
    ));
    assert!(base.value().get("Task").is_none());
    task.revoke();
    assert!(matches!(
        task.wait(deadline()).await?.as_ref(),
        CallbackOutcome::Revoked
    ));
    f.close().await
}
