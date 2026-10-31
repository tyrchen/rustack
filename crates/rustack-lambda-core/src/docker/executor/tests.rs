use std::{
    collections::HashMap,
    error::Error as _,
    future::Future,
    task::{Context, Poll, Waker},
};

use bytes::Bytes;
use serde_json::{to_string, to_value};
use tokio::sync::{mpsc::error::TryRecvError, oneshot, watch};
use uuid::Uuid;

use super::*;
use crate::{
    artifact::ImageArtifact,
    config::LambdaConfig,
    deployment::ImageDeploymentConfig,
    docker::{
        DockerWire, EnvironmentConfig, EnvironmentManager, ImageLeaseConfig, ImageLeaseManager,
        ImagePinScope, RelayBudgets,
        environment::test_peer,
        plan::tests::{function_body, image_with, policy, relay_body, revision},
        sdk::{LocalSdkCredentials, SdkAffiliation},
        tests::Daemon,
    },
    error::LambdaServiceError,
    executor::ExecutorBackend,
    provider::RustackLambda,
    storage::{FunctionStore, tests::sample_record},
    work::WorkManager,
};

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
#[derive(Debug)]
struct Readiness(AtomicBool);
impl DockerReadiness for Readiness {
    fn check(&self) -> Result<(), DockerExecutorError> {
        if self.0.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(DockerExecutorError::Unavailable)
        }
    }
}
struct Fixture {
    daemon: Daemon,
    wire: DockerWire,
    environments: EnvironmentManager,
    directory: DirectoryLease,
    ready: Arc<Readiness>,
}
impl Fixture {
    async fn new() -> Self {
        let daemon = Daemon::new();
        let wire = daemon.wire().await;
        let directory = DirectoryLease::testing(Uuid::new_v4(), Uuid::new_v4());
        let environments =
            EnvironmentManager::start(wire.port(), directory.clone(), EnvironmentConfig::default())
                .unwrap();
        Self {
            daemon,
            wire,
            environments,
            directory,
            ready: Arc::new(Readiness(AtomicBool::new(true))),
        }
    }
    fn config(&self) -> DockerExecutorConfig {
        DockerExecutorConfig::builder()
            .directory(self.directory.clone())
            .relay(image_with("1000", vec![]))
            .policy(policy())
            .environments(self.environments.port())
            .readiness(self.ready.clone())
            .settings(
                DockerExecutorSettings::new(PoolConfig::default(), Duration::from_secs(1)).unwrap(),
            )
            .build()
    }
    async fn close(mut self) {
        self.environments.shutdown(deadline()).await.unwrap();
        self.wire.shutdown().await.unwrap();
        self.daemon.close().await;
    }
}
fn request() -> InvokeRequest {
    InvokeRequest::new(
        Arc::new(revision()),
        "arn:aws:lambda:us-east-1:000000000000:function:policy-fixture".into(),
        Bytes::from_static(b"\x00\xffraw-event"),
    )
}

#[test]
fn test_should_bound_executor_cleanup_settings() {
    for invalid in [
        Duration::ZERO,
        Duration::from_millis(1500),
        Duration::from_secs(61),
    ] {
        assert!(matches!(
            DockerExecutorSettings::new(PoolConfig::default(), invalid),
            Err(DockerExecutorError::Configuration)
        ));
    }
}

#[rstest::rstest]
#[case::name("arn:aws:lambda:us-east-1:000000000000:function:other")]
#[case::region("arn:aws:lambda:us-west-2:000000000000:function:policy-fixture")]
#[case::alias_latest("arn:aws:lambda:us-east-1:000000000000:function:policy-fixture:prod")]
#[case::version_latest("arn:aws:lambda:us-east-1:000000000000:function:policy-fixture:2")]
#[case::malformed("private-invalid-metadata")]
#[tokio::test]
async fn test_should_reject_invalid_request_metadata_before_daemon_allocation(#[case] arn: &str) {
    let fixture = Fixture::new().await;
    let executor = DockerExecutor::new(fixture.config()).unwrap();
    let mut request = request();
    request.function_arn = arn.into();
    assert!(executor.available(&request).is_err());
    assert_eq!(fixture.environments.port().status().allocated, 0);
    executor.shutdown().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn test_should_validate_scope_readiness_and_payload_without_allocating() {
    let fixture = Fixture::new().await;
    let mut wrong = fixture.config();
    wrong.directory = DirectoryLease::testing(Uuid::new_v4(), Uuid::new_v4());
    assert!(matches!(
        DockerExecutor::new(wrong),
        Err(DockerExecutorError::Configuration)
    ));
    let mut revoked = fixture.config();
    revoked.relay = PinnedImage::new(
        revoked.relay.resolved().clone(),
        revoked.relay.unpacked_size(),
        watch::channel(()).0,
        Arc::new(AtomicBool::new(false)),
        watch::channel(false).1,
    );
    assert!(matches!(
        DockerExecutor::new(revoked),
        Err(DockerExecutorError::Configuration)
    ));
    let executor = DockerExecutor::new(fixture.config()).unwrap();
    executor.available(&request()).unwrap();
    fixture.ready.0.store(false, Ordering::Release);
    assert!(matches!(
        executor.image_available(),
        Err(ExecutorError::Docker(DockerExecutorError::Unavailable))
    ));
    fixture.ready.0.store(true, Ordering::Release);
    let mut invalid = request();
    invalid.payload = Bytes::from(vec![0; 6 * 1024 * 1024 + 1]);
    assert!(executor.available(&invalid).is_err());
    let mut invalid = request();
    Arc::make_mut(&mut invalid.revision).backend = crate::executor::ExecutorBackend::Native;
    assert!(executor.available(&invalid).is_err());
    let mut published = request();
    Arc::make_mut(&mut published.revision).qualifier = "1".into();
    assert!(executor.available(&published).is_err());
    published.function_arn.push_str(":prod");
    executor.available(&published).unwrap();
    published.function_arn = published.function_arn.replace(":prod", ":1");
    executor.available(&published).unwrap();
    assert_eq!(fixture.environments.port().status().allocated, 0);
    executor.shutdown().await.unwrap();
    assert!(executor.image_available().is_err());
    fixture.close().await;
}

#[rstest::rstest]
#[case::success(false, false)]
#[case::error(false, true)]
#[case::tail(true, false)]
#[case::tail_error(true, true)]
#[test]
fn test_should_preserve_binary_outcomes_and_encode_requested_window_once(
    #[case] tail: bool,
    #[case] failed: bool,
) {
    let mut request = request();
    request.capture_logs = tail;
    Arc::make_mut(&mut request.revision).qualifier = "17".into();
    request.function_arn.push_str(":prod");
    let raw = Bytes::from_static(b"\x00\xffprivate-handler-data");
    let payload = Payload::try_from(raw.clone()).unwrap();
    let logs = Bytes::from_static(b"\xff\x00private-log-data");
    let result = if failed {
        RuntimeResult::FunctionError(payload)
    } else {
        RuntimeResult::Success(payload)
    };
    let response = response(
        EnvironmentResponse {
            result,
            logs: logs.clone(),
        },
        &request,
    )
    .unwrap();
    assert_eq!(response.payload, raw);
    assert_eq!(response.status, 200);
    assert_eq!(
        response.function_error.as_deref(),
        if failed { Some("Unhandled") } else { None }
    );
    assert_eq!(response.log_tail, tail.then(|| STANDARD.encode(&logs)));
    assert_eq!(response.executed_version, "17");
    assert!(!format!("{response:?}").contains("private"));
}
#[test]
fn test_should_preserve_empty_tail_and_reject_oversized_logs_or_init_success() {
    let mut request = request();
    request.capture_logs = true;
    let value = || RuntimeResult::Success(Payload::try_from(Bytes::new()).unwrap());
    assert_eq!(
        response(
            EnvironmentResponse {
                result: value(),
                logs: Bytes::new()
            },
            &request
        )
        .unwrap()
        .log_tail
        .as_deref(),
        Some("")
    );
    assert!(
        response(
            EnvironmentResponse {
                result: value(),
                logs: Bytes::from(vec![0; 4097])
            },
            &request
        )
        .is_err()
    );
    for size in [32768, 32769] {
        let result =
            RuntimeResult::InitError(Payload::try_from(Bytes::from(vec![0; size])).unwrap());
        let error = response(
            EnvironmentResponse {
                result,
                logs: Bytes::new(),
            },
            &request,
        )
        .unwrap_err();
        if size == 32768 {
            assert!(matches!(
                error,
                ExecutorError::Docker(DockerExecutorError::Environment(
                    EnvironmentError::Initialization(_)
                ))
            ));
        } else {
            assert!(matches!(
                error,
                ExecutorError::Docker(DockerExecutorError::Environment(EnvironmentError::Docker(
                    DockerError::Protocol
                )))
            ));
        }
    }
}

#[test]
fn test_should_preserve_typed_environment_failures_and_static_service_mapping() {
    let source = DockerExecutorError::from(ProtocolError::Io(std::io::Error::other(
        "private-transport-data",
    )));
    assert!(source.source().is_some());
    assert!(!format!("{source:?} {source}").contains("private-transport-data"));
    assert!(matches!(
        environment_error(EnvironmentError::Timeout(Duration::from_secs(3))),
        ExecutorError::Timeout(_)
    ));
    assert!(matches!(
        environment_error(EnvironmentError::Deadline),
        ExecutorError::DeadlineExceeded
    ));
    assert!(matches!(
        environment_error(EnvironmentError::Cancelled),
        ExecutorError::Cancelled
    ));
    assert!(matches!(
        LambdaServiceError::from(environment_error(EnvironmentError::Busy)),
        LambdaServiceError::TooManyRequests
    ));
    let nested = environment_error(EnvironmentError::Docker(DockerError::Busy));
    assert!(matches!(
        &nested,
        ExecutorError::Docker(DockerExecutorError::Environment(EnvironmentError::Docker(
            DockerError::Busy
        )))
    ));
    assert!(matches!(
        LambdaServiceError::from(nested),
        LambdaServiceError::Internal { .. }
    ));
    assert!(matches!(
        LambdaServiceError::from(ExecutorError::from(DockerExecutorError::Unavailable)),
        LambdaServiceError::DockerNotAvailable
    ));
    for error in [
        EnvironmentError::Initialization(
            Payload::try_from(Bytes::from_static(b"private-error")).unwrap(),
        ),
        EnvironmentError::Exited {
            exit_code: 137,
            oom: true,
        },
        EnvironmentError::Docker(DockerError::Protocol),
    ] {
        let error = environment_error(error);
        assert!(error.source().is_some());
        assert!(!format!("{error:?} {error}").contains("private-error"));
    }
}

#[rstest::rstest]
#[case::released(false)]
#[case::cleanup_expired(true)]
#[tokio::test]
async fn test_should_hold_work_supervisor_concurrency_through_docker_cancellation_settlement(
    #[case] expired: bool,
) {
    let fixture = Fixture::new().await;
    let config = fixture.config();
    let request = request();
    let plan = EnvironmentPlan::new(
        config.directory.clone(),
        config.relay.clone(),
        request.revision.clone(),
        config.policy.clone(),
    )
    .unwrap();
    let (environment, peer) = test_peer::pair();
    let (entered, observed) = oneshot::channel();
    let (stopping, stopped) = oneshot::channel();
    let (release, gate) = oneshot::channel();
    let peer = tokio::spawn(peer.serve_pending(entered, gate, stopping));
    let pool = super::super::pool::testing_start(
        fixture.environments.port(),
        PoolConfig::default(),
        vec![(plan.warm_key().into(), environment)],
    );
    let port = pool.port();
    let executor = Arc::new(DockerExecutor::with_pool(
        config,
        pool,
        port,
        Handle::current(),
    ));
    let work = WorkManager::new();
    let handle = work
        .submit(executor.clone(), request.clone(), Some(1), false, None)
        .await
        .unwrap();
    let id = handle.id();
    let actual = observed.await.unwrap();
    assert_eq!(actual.parts().0.to_string(), id.to_string());
    assert_eq!(actual.parts().2.as_ref(), request.function_arn.as_str());
    assert_eq!(actual.parts().4.as_ref(), request.payload.as_ref());
    let mut waiting = Box::pin(handle.cancel_and_wait(deadline()));
    assert!(
        waiting
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    stopped.await.unwrap();
    let rejected = work
        .submit(executor.clone(), request, Some(1), false, None)
        .await;
    assert!(
        matches!(rejected, Err(LambdaServiceError::TooManyRequests)),
        "{rejected:?}"
    );
    assert!(
        waiting
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    let mut shutdown = Box::pin(executor.shutdown());
    assert!(
        shutdown
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    drop(shutdown);
    assert!(executor.image_available().is_err());
    if expired {
        assert!(matches!(
            executor.shutdown().await,
            Err(ExecutorError::CleanupFailed(_))
        ));
        assert!(matches!(
            waiting.await,
            Err(ExecutorError::CleanupFailed(_))
        ));
        assert!(work.quiesce(Duration::from_secs(1)).await.is_err());
        release.send(()).unwrap();
    } else {
        release.send(()).unwrap();
        waiting.await.unwrap();
        work.quiesce(Duration::from_secs(10)).await.unwrap();
    }
    peer.await.unwrap();
    executor.shutdown().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn test_should_reject_impossible_memory_before_provider_admission_as_a_throttle() {
    let fixture = Fixture::new().await;
    let executor = Arc::new(DockerExecutor::new(fixture.config()).unwrap());
    let mut oversized = request();
    Arc::make_mut(&mut oversized.revision).memory_mb = 10240;
    assert!(matches!(
        executor.available(&oversized),
        Err(ExecutorError::Docker(DockerExecutorError::Environment(
            EnvironmentError::MemoryLimit
        )))
    ));
    let work = WorkManager::new();
    assert!(matches!(
        work.submit(executor.clone(), oversized, Some(1), false, None)
            .await,
        Err(LambdaServiceError::TooManyRequests)
    ));
    assert_eq!(fixture.environments.port().status().allocated, 0);
    assert_eq!(fixture.environments.port().status().memory_bytes, 0);
    assert!(executor.available(&request()).is_ok());
    work.quiesce(Duration::from_secs(1)).await.unwrap();
    executor.shutdown().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn test_should_distinguish_unconfigured_zip_from_revoked_backend_without_allocating() {
    let fixture = Fixture::new().await;
    let executor = DockerExecutor::new(fixture.config()).unwrap();
    assert!(matches!(
        executor.zip_available(Architecture::Arm64),
        Err(ExecutorError::Unsupported(_))
    ));
    fixture.ready.0.store(false, Ordering::Release);
    assert!(matches!(
        executor.zip_available(Architecture::Arm64),
        Err(ExecutorError::Docker(DockerExecutorError::Unavailable))
    ));
    assert_eq!(fixture.environments.port().status().allocated, 0);
    executor.shutdown().await.unwrap();
    fixture.close().await;
}

fn sdk_context(config: &DockerExecutorConfig) -> DockerSdkEnvironment {
    let credentials: LocalSdkCredentials = ("private-sdk-key".into(), "private-sdk-secret".into())
        .try_into()
        .unwrap();
    DockerSdkEnvironment::new(
        credentials,
        "http://127.0.0.1:4566".parse().unwrap(),
        SdkAffiliation::new(
            config.directory.clone(),
            config.relay.clone(),
            config.policy.clone(),
            config.environments.clone(),
            config.readiness.clone(),
        ),
    )
    .unwrap()
}
#[rstest::rstest]
#[case::qualification("qualification")]
#[case::acquisition("acquisition")]
#[case::manager("manager")]
#[case::network("network")]
#[case::relay("relay")]
#[tokio::test]
async fn test_should_refuse_foreign_sdk_affiliation_before_pool_or_container_allocation(
    #[case] changed: &str,
) {
    let fixture = Fixture::new().await;
    let original = fixture.config();
    let sdk = sdk_context(&original);
    let mut other_manager = EnvironmentManager::start(
        fixture.wire.port(),
        original.directory.clone(),
        EnvironmentConfig::default(),
    )
    .unwrap();
    let mut foreign = original.clone();
    match changed {
        "qualification" => foreign.readiness = Arc::new(Readiness(AtomicBool::new(true))),
        "acquisition" => {
            foreign.directory =
                DirectoryLease::testing(original.directory.owner_id(), original.directory.run_id());
        }
        "manager" => foreign.environments = other_manager.port(),
        "network" => {
            foreign.policy = ContainerPolicy::new(
                "other-bridge".parse().unwrap(),
                1000,
                1000,
                1_000_000_000,
                RelayBudgets::default(),
            )
            .unwrap();
        }
        "relay" => foreign.relay = image_with("1000", vec!["CHANGED=value".into()]),
        _ => unreachable!("fixed affiliation cases"),
    }
    assert!(!sdk.matches(&foreign));
    foreign.sdk = Some(sdk.clone());
    assert!(matches!(
        DockerExecutor::new(foreign),
        Err(DockerExecutorError::Configuration)
    ));
    assert_eq!(fixture.environments.port().status().allocated, 0);
    assert_eq!(other_manager.port().status().allocated, 0);
    other_manager.shutdown(deadline()).await.unwrap();
    fixture.close().await;
}
#[tokio::test]
async fn test_should_isolate_sdk_warm_identity_without_mutating_revision_or_relay_environment() {
    let fixture = Fixture::new().await;
    let mut config = fixture.config();
    let first = sdk_context(&config);
    let second = sdk_context(&config);
    assert_eq!(first.identity(), first.clone().identity());
    assert_ne!(first.identity(), second.identity());
    assert_eq!(format!("{first:?}"), "DockerSdkEnvironment(<redacted>)");
    let mut call = request();
    Arc::make_mut(&mut call.revision).environment.insert(
        "AWS_ENDPOINT_URL".into(),
        "http://user-selected:4566".into(),
    );
    let original_env = call.revision.environment.clone();
    let plain = DockerExecutor::new(config.clone()).unwrap();
    let without_sdk = plain.plan(&call).unwrap();
    assert!(
        function_body(&without_sdk)
            .env
            .as_ref()
            .unwrap()
            .contains(&"AWS_ENDPOINT_URL=http://user-selected:4566".to_owned())
    );
    config.sdk = Some(first.clone());
    let enabled = DockerExecutor::new(config.clone()).unwrap();
    let first_plan = enabled.plan(&call).unwrap();
    let cloned_plan = enabled.plan(&call).unwrap();
    assert_ne!(without_sdk.warm_key(), first_plan.warm_key());
    assert_eq!(first_plan.warm_key(), cloned_plan.warm_key());
    assert!(
        function_body(&first_plan)
            .env
            .as_ref()
            .unwrap()
            .contains(&"AWS_ENDPOINT_URL=http://127.0.0.1:4566/".to_owned())
    );
    assert!(
        relay_body(&first_plan)
            .env
            .as_ref()
            .unwrap()
            .iter()
            .all(|entry| !entry.starts_with("AWS_") && !entry.contains("private-sdk"))
    );
    assert_eq!(call.revision.environment, original_env);
    config.sdk = Some(second);
    let other = DockerExecutor::new(config).unwrap();
    assert_ne!(first_plan.warm_key(), other.plan(&call).unwrap().warm_key());
    fixture.ready.0.store(false, Ordering::Release);
    assert!(matches!(
        enabled.plan(&call),
        Err(DockerExecutorError::Unavailable)
    ));
    assert_eq!(fixture.environments.port().status().allocated, 0);
    drop((without_sdk, first_plan, cloned_plan));
    plain.shutdown().await.unwrap();
    enabled.shutdown().await.unwrap();
    other.shutdown().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn test_should_retain_sdk_revocation_guard_in_a_prepared_plan_and_refuse_create_without_rpc()
{
    let mut fixture = Fixture::new().await;
    let mut config = fixture.config();
    config.sdk = Some(sdk_context(&config));
    let executor = DockerExecutor::new(config).unwrap();
    let plan = executor.plan(&request()).unwrap();
    plan.live().unwrap();
    fixture.ready.0.store(false, Ordering::Release);
    assert!(matches!(plan.live(), Err(DockerError::Stopped)));
    let port = fixture.wire.port();
    let mut relay = Box::pin(plan.create_relay(&port, deadline()));
    assert!(matches!(
        relay.as_mut().poll(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(Err(DockerError::Stopped))
    ));
    drop(relay);
    let mut function = Box::pin(plan.create_function(&port, deadline()));
    assert!(matches!(
        function
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(Err(DockerError::Stopped))
    ));
    drop(function);
    assert!(matches!(
        fixture.daemon.requests.try_recv(),
        Err(TryRecvError::Empty)
    ));
    assert_eq!(fixture.environments.port().status().allocated, 0);
    drop(plan);
    executor.shutdown().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn test_should_export_real_sdk_provider_snapshot_without_injected_keys_or_credentials() {
    let fixture = Fixture::new().await;
    let mut config = fixture.config();
    config.sdk = Some(sdk_context(&config));
    let executor = Arc::new(DockerExecutor::new(config).unwrap());
    let mut call = request();
    let user_env = HashMap::from([
        ("BUSINESS".to_owned(), "retained".to_owned()),
        (
            "AWS_ENDPOINT_URL".to_owned(),
            "http://user-selected:4566".to_owned(),
        ),
    ]);
    Arc::make_mut(&mut call.revision)
        .environment
        .extend(user_env.clone());
    let plan = executor.plan(&call).unwrap();
    assert!(
        function_body(&plan)
            .env
            .as_ref()
            .unwrap()
            .iter()
            .any(|entry| entry == "AWS_SECRET_ACCESS_KEY=private-sdk-secret")
    );
    let DeploymentArtifact::Image(image) = &call.revision.artifact else {
        panic!("fixed image fixture");
    };
    let root = tempfile::tempdir().unwrap();
    let store = FunctionStore::new(root.path());
    let mut record = sample_record(&call.revision.function_name);
    record.resource_id = call.revision.resource_id;
    record.arn = call.function_arn.clone();
    record.latest.package_type = "Image".into();
    record.latest.runtime = None;
    record.latest.handler = None;
    record.latest.image_uri = Some(image.resolved().metadata().requested_reference.to_string());
    record.latest.image = ImageArtifact::Pinned(image.clone());
    record.latest.architectures = vec!["arm64".into()];
    record.latest.environment = user_env.clone();
    record.latest.code_sha256 = image.resolved().code_sha256().unwrap();
    record.latest.code_size = image.unpacked_size().get();
    store.insert(record).unwrap();
    let mut images = ImageLeaseManager::start(
        fixture.wire.port(),
        ImagePinScope::new(fixture.directory.clone()),
        ImageLeaseConfig::default(),
    )
    .unwrap();
    let provider = RustackLambda::with_executor(
        store,
        LambdaConfig {
            executor: ExecutorBackend::Docker,
            ..Default::default()
        },
        executor,
    )
    .with_image_preparation(images.port(), ImageDeploymentConfig::default())
    .unwrap();
    let snapshot = provider.export_snapshot_before(deadline()).await.unwrap();
    let encoded = to_string(&snapshot).unwrap();
    for private in [
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
        "private-sdk-key",
        "private-sdk-secret",
    ] {
        assert!(!encoded.contains(private));
    }
    let projected = to_value(&snapshot).unwrap();
    assert_eq!(
        projected["functions"]["functions"][0]["latest"]["environment"],
        to_value(user_env).unwrap()
    );
    drop(plan);
    provider.shutdown_before(deadline()).await.unwrap();
    images.shutdown(deadline()).await.unwrap();
    fixture.close().await;
}
