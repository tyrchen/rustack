//! Unique reporting, timestamp fences and explicit command composition.

use uuid::Uuid;

use super::*;
use crate::{LambdaTaskCommand, TaskDispatch};

fn metadata() -> CallbackInvocationMetadata {
    CallbackInvocationMetadata::new(Uuid::new_v4(), "$LATEST").unwrap()
}
#[rstest]
#[case("$LATEST", true)]
#[case("1", true)]
#[case("18446744073709551615", true)]
#[case("18446744073709551616", false)]
#[case("01", false)]
#[case("0", false)]
#[case("-1", false)]
#[case("1.0", false)]
#[case("", false)]
#[case("alias", false)]
#[case("１", false)]
#[case("1\n", false)]
fn test_should_validate_actual_version_and_exclude_business_payload(
    #[case] version: &str,
    #[case] valid: bool,
) {
    let id = Uuid::new_v4();
    let result = CallbackInvocationMetadata::new(id, version);
    assert_eq!(result.is_ok(), valid);
    if let Ok(value) = result {
        assert_eq!(value.request_id(), id);
        assert_eq!(value.executed_version(), version);
        let payload = value.payload().unwrap();
        assert_eq!(
            payload.value(),
            &json!({"StatusCode":200,"ExecutedVersion":version,"SdkHttpMetadata":{"HttpStatusCode":200},"SdkResponseMetadata":{"RequestId":id.to_string()}})
        );
        assert!(!format!("{value:?}").contains(&id.to_string()));
    }
    assert!(CallbackInvocationMetadata::new(Uuid::nil(), version).is_err());
    let mut bytes = *id.as_bytes();
    bytes[8] = 0;
    assert!(CallbackInvocationMetadata::new(Uuid::from_bytes(bytes), version).is_err());
}
#[tokio::test]
async fn test_should_transfer_one_reporter_and_observe_report_after_callback_completion()
-> TestResult {
    let mut f = Fixture::new(CallbackLimits::default()).await?;
    let mut task = f.task(0, 300, Some(20)).await?;
    let secret = task.token().expose().to_owned();
    let mut reporter = task.take_invocation_reporter()?;
    assert!(matches!(
        task.take_invocation_reporter(),
        Err(CallbackError::Progress)
    ));
    assert!(matches!(
        task.invocation_complete(Ok(())),
        Err(CallbackError::Progress)
    ));
    assert_eq!(
        reporter.bind(&TenantScope::default(), f.calls[0])?,
        task.deadline
    );
    assert!(matches!(
        reporter.bind(&TenantScope::default(), f.calls[1]),
        Err(CallbackError::Validation)
    ));
    assert!(matches!(
        reporter.bind(
            &TenantScope::new("aws", "us-west-2", "000000000000")?,
            f.calls[0]
        ),
        Err(CallbackError::Validation)
    ));
    action(
        &f.callbacks,
        CallbackCommand::success(token(&task), b"{\"selected\":42}")?,
    )
    .await?;
    assert!(matches!(
        task.snapshot().as_ref(),
        CallbackOutcome::Invoking
    ));
    let meta = metadata();
    let id = meta.request_id();
    let observed = Instant::now();
    reporter.complete_at(Ok(meta), observed)?;
    assert!(matches!(
        reporter.complete_at(Ok(metadata()), observed),
        Err(CallbackError::Progress)
    ));
    drop(reporter);
    let outcome = task.wait(deadline()).await?;
    assert!(
        matches!(outcome.as_ref(), CallbackOutcome::Succeeded(p) if p.value()==&json!({"selected":42}))
    );
    let report = task.wait_invocation(deadline()).await?;
    assert_eq!(report.metadata().ok_or("metadata")?.request_id(), id);
    assert_eq!(report.observed(), observed);
    assert!(Arc::ptr_eq(
        &report,
        &task.wait_invocation(deadline()).await?
    ));
    assert!(!format!("{report:?}{task:?}").contains(&secret));
    f.close().await
}
#[tokio::test]
async fn test_should_revoke_an_unreported_delegated_owner_but_keep_reported_waiting() -> TestResult
{
    let mut f = Fixture::new(CallbackLimits::default()).await?;
    let mut first = f.task(0, 300, None).await?;
    drop(first.take_invocation_reporter()?);
    assert!(matches!(
        first.wait(deadline()).await?.as_ref(),
        CallbackOutcome::Revoked
    ));
    let mut second = f.task(1, 300, None).await?;
    let mut reporter = second.take_invocation_reporter()?;
    reporter.complete_at(Ok(metadata()), Instant::now())?;
    drop(reporter);
    activated(&mut second).await?;
    action(
        &f.callbacks,
        CallbackCommand::success(token(&second), b"true")?,
    )
    .await?;
    assert!(matches!(
        second.wait(deadline()).await?.as_ref(),
        CallbackOutcome::Succeeded(_)
    ));
    f.close().await
}
#[tokio::test]
async fn test_should_anchor_heartbeat_before_delayed_report_mapping() -> TestResult {
    let mut f = Fixture::new(CallbackLimits::default()).await?;
    let mut task = f.task(0, 300, Some(10)).await?;
    let mut reporter = task.take_invocation_reporter()?;
    pause();
    let observed = Instant::now();
    advance(Duration::from_secs(11)).await;
    reporter.complete_at(Ok(metadata()), observed)?;
    let report = task.wait_invocation(deadline()).await?;
    assert_eq!(report.observed(), observed);
    let outcome = task.wait(deadline()).await?;
    assert!(matches!(
        outcome.as_ref(),
        CallbackOutcome::TimedOut(CallbackTimeout::Heartbeat)
    ));
    resume();
    f.close().await
}
#[tokio::test]
async fn test_should_reject_invalid_report_times_expired_and_revoked_control() -> TestResult {
    let mut f = Fixture::new(CallbackLimits::default()).await?;
    let mut task = f.task(0, 300, None).await?;
    let mut reporter = task.take_invocation_reporter()?;
    assert!(matches!(
        reporter.complete_at(Ok(metadata()), Instant::now() + Duration::from_secs(10)),
        Err(CallbackError::Validation)
    ));
    assert!(matches!(
        reporter.complete_at(Ok(metadata()), Instant::now() - Duration::from_secs(60)),
        Err(CallbackError::Validation)
    ));
    assert!(task.invocation().is_none());
    task.revoke();
    assert!(matches!(
        reporter.complete_at(Ok(metadata()), Instant::now()),
        Err(CallbackError::TaskTimedOut)
    ));
    let mut task = f.task(1, 30, None).await?;
    let mut reporter = task.take_invocation_reporter()?;
    pause();
    let observed = Instant::now();
    advance(Duration::from_secs(31)).await;
    assert!(matches!(
        reporter.complete_at(Ok(metadata()), observed),
        Err(CallbackError::TaskTimedOut)
    ));
    assert!(task.invocation().is_none());
    resume();
    f.close().await
}
#[tokio::test]
async fn test_should_keep_invocation_failure_over_early_success() -> TestResult {
    let mut f = Fixture::new(CallbackLimits::default()).await?;
    let mut task = f.task(0, 300, None).await?;
    let mut reporter = task.take_invocation_reporter()?;
    action(
        &f.callbacks,
        CallbackCommand::success(token(&task), b"true")?,
    )
    .await?;
    reporter.complete_at(Err(TaskFailure::new("TestError", None)?), Instant::now())?;
    assert_eq!(
        task.wait_invocation(deadline())
            .await?
            .failure()
            .ok_or("failure")?
            .details()
            .error(),
        Some("TestError")
    );
    assert!(matches!(
        task.wait(deadline()).await?.as_ref(),
        CallbackOutcome::InvocationFailed(_)
    ));
    f.close().await
}
fn command(
    task: &mut CallbackTask,
    call: FrameCallId,
    optimized: bool,
) -> Result<LambdaTaskCommand, CallbackError> {
    let resource = if optimized {
        "arn:aws:states:::lambda:invoke"
    } else {
        "arn:aws:lambda:us-east-1:000000000000:function:Worker"
    };
    let source =
        json!({"StartAt":"T","States":{"T":{"Type":"Task","Resource":resource,"End":true}}});
    let machine = AslCompiler::new(TenantScope::default(), TaskSupport::Lambda)
        .compile(&serde_json::to_vec(&source).unwrap())
        .unwrap()
        .machine()
        .clone();
    let dispatch = TaskDispatch {
        attempt: call.attempt(),
        state: machine.start(),
        parameters: Payload::parse(br#"{"FunctionName":"Worker"}"#).unwrap(),
        deadline: (Instant::now() + Duration::from_secs(600)).into_std(),
    };
    LambdaTaskCommand::observed_callback(machine, dispatch, call, task.take_invocation_reporter()?)
}
#[tokio::test]
async fn test_should_keep_observed_mode_after_one_shot_transfer_and_clamp_deadline() -> TestResult {
    let mut f = Fixture::new(CallbackLimits::default()).await?;
    let mut task = f.task(0, 300, None).await?;
    let mut command = command(&mut task, f.calls[0], true)?;
    assert!(command.is_observed_callback());
    assert_eq!(command.dispatch().deadline(), task.deadline.into_std());
    let reporter = command.take_callback_reporter()?.ok_or("reporter")?;
    assert!(command.is_observed_callback());
    assert!(matches!(
        command.take_callback_reporter(),
        Err(CallbackError::Progress)
    ));
    drop(reporter);
    f.close().await
}
#[rstest]
#[case(false, false)]
#[case(true, true)]
#[tokio::test]
async fn test_should_reject_wrong_command_resource_or_attempt(
    #[case] optimized: bool,
    #[case] mismatch: bool,
) -> TestResult {
    let mut f = Fixture::new(CallbackLimits::default()).await?;
    let mut task = f.task(0, 300, None).await?;
    assert!(matches!(
        command(&mut task, f.calls[usize::from(mismatch)], optimized),
        Err(CallbackError::Validation)
    ));
    assert!(matches!(
        task.wait(deadline()).await?.as_ref(),
        CallbackOutcome::Revoked
    ));
    f.close().await
}

#[rstest]
#[case(0, true)]
#[case(9, true)]
#[case(10, false)]
#[case(11, false)]
#[tokio::test]
async fn test_should_fence_provisional_callback_by_captured_heartbeat_during_delayed_report(
    #[case] seconds: u64,
    #[case] succeeds: bool,
) -> TestResult {
    let mut f = Fixture::new(CallbackLimits::default()).await?;
    let mut task = f.task(0, 300, Some(10)).await?;
    let mut reporter = task.take_invocation_reporter()?;
    pause();
    let observed = Instant::now();
    advance(Duration::from_secs(seconds)).await;
    action(
        &f.callbacks,
        CallbackCommand::success(token(&task), b"true")?,
    )
    .await?;
    advance(Duration::from_secs(12 - seconds)).await;
    reporter.complete_at(Ok(metadata()), observed)?;
    let outcome = task.wait(deadline()).await?;
    assert_eq!(
        matches!(outcome.as_ref(), CallbackOutcome::Succeeded(_)),
        succeeds
    );
    if !succeeds {
        assert!(matches!(
            outcome.as_ref(),
            CallbackOutcome::TimedOut(CallbackTimeout::Heartbeat)
        ));
    }
    resume();
    f.close().await
}
#[tokio::test]
async fn test_should_keep_preactivation_heartbeat_noop_during_delayed_report() -> TestResult {
    let mut f = Fixture::new(CallbackLimits::default()).await?;
    let mut task = f.task(0, 300, Some(10)).await?;
    let mut reporter = task.take_invocation_reporter()?;
    pause();
    let observed = Instant::now();
    advance(Duration::from_secs(9)).await;
    action(&f.callbacks, CallbackCommand::heartbeat(token(&task))).await?;
    advance(Duration::from_secs(2)).await;
    reporter.complete_at(Ok(metadata()), observed)?;
    assert!(matches!(
        task.wait(deadline()).await?.as_ref(),
        CallbackOutcome::TimedOut(CallbackTimeout::Heartbeat)
    ));
    resume();
    f.close().await
}

#[rstest]
#[case(0)]
#[case(1)]
#[case(2)]
#[tokio::test]
async fn test_should_fence_late_ready_invocation_success_failure_and_owner_loss_without_resending(
    #[case] kind: u8,
) -> TestResult {
    let mut task = manual_task(control());
    let (view, reader) = watch::channel(Arc::new(CallbackOutcome::Invoking));
    task.view = reader;
    let mut reporter = Some(task.take_invocation_reporter()?);
    pause();
    let observed = Instant::now();
    let until = observed + Duration::from_secs(1);
    {
        let wait = task.wait_invocation(until);
        tokio::pin!(wait);
        pending_once(wait.as_mut()).await;
        if kind == 2 {
            drop(reporter.take());
        } else {
            let result = if kind == 0 {
                Ok(metadata())
            } else {
                Err(TaskFailure::new("RealFailure", None)?)
            };
            reporter
                .as_mut()
                .ok_or("reporter")?
                .complete_at(result, observed)?;
        }
        advance(Duration::from_secs(1)).await;
        assert!(matches!(wait.await, Err(CallbackError::Deadline)));
    }
    let next = task.wait_invocation(deadline()).await;
    if kind == 2 {
        assert!(matches!(next, Err(CallbackError::OwnerLost)));
    } else {
        let report = next?;
        assert_eq!(report.succeeded(), kind == 0);
        assert_eq!(report.observed(), observed);
    }
    drop(view);
    resume();
    Ok(())
}
#[tokio::test]
async fn test_should_release_report_payload_when_its_real_observers_and_reporter_leave()
-> TestResult {
    let mut f = Fixture::new(CallbackLimits::default()).await?;
    let mut task = f.task(0, 300, None).await?;
    let mut reporter = task.take_invocation_reporter()?;
    reporter.complete_at(
        Err(TaskFailure::new("Failure", Some("bounded cause"))?),
        Instant::now(),
    )?;
    let report = task.wait_invocation(deadline()).await?;
    let weak = Arc::downgrade(&report);
    task.wait(deadline()).await?;
    let old = token(&task);
    drop(report);
    drop(reporter);
    drop(task);
    assert!(weak.upgrade().is_none());
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

#[tokio::test]
async fn test_should_reject_dispatch_generation_mismatch_even_with_the_right_reporter_call()
-> TestResult {
    let mut f = Fixture::new(CallbackLimits::default()).await?;
    let mut task = f.task(0, 300, None).await?;
    let machine=AslCompiler::new(TenantScope::default(),TaskSupport::Lambda).compile(br#"{"StartAt":"T","States":{"T":{"Type":"Task","Resource":"arn:aws:states:::lambda:invoke","End":true}}}"#)?.machine().clone();
    let dispatch = TaskDispatch {
        attempt: f.calls[1].attempt(),
        state: machine.start(),
        parameters: Payload::parse(b"{}")?,
        deadline: deadline().into_std(),
    };
    assert!(matches!(
        LambdaTaskCommand::observed_callback(
            machine,
            dispatch,
            f.calls[0],
            task.take_invocation_reporter()?
        ),
        Err(CallbackError::Validation)
    ));
    assert!(matches!(
        task.wait(deadline()).await?.as_ref(),
        CallbackOutcome::Revoked
    ));
    f.close().await
}
