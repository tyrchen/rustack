use std::{
    future::Future,
    task::{Context, Poll, Waker},
};

use async_trait::async_trait;
use rstest::rstest;
use rustack_lambda_model::{
    input::{
        CreateFunctionInput, PublishVersionInput, UpdateFunctionCodeInput,
        UpdateFunctionConfigurationInput,
    },
    types::FunctionCode,
};
use tokio::time::timeout;

use super::*;
use crate::{
    artifact::ArtifactError,
    config::LambdaConfig,
    deployment::{DeploymentState, tests::record},
    docker::{
        ImageLeaseManager,
        image_fixture::{Command, FakePins, Mode as DaemonMode},
    },
    executor::{Executor, ExecutorBackend, InvokeRequest, InvokeResponse, NoopExecutor},
    invocation::InvocationContext,
    provider::RustackLambda,
};

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
async fn until(condition: impl Fn() -> bool) {
    timeout(Duration::from_secs(10), async {
        while !condition() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
fn latest(store: &FunctionStore) -> crate::storage::VersionRecord {
    store.get("worker").unwrap().unwrap().latest
}
fn named(name: &str) -> FunctionRecord {
    let mut record = record();
    record.name = name.into();
    record.arn = format!("arn:aws:lambda:us-east-1:000000000000:function:{name}");
    record
}
struct Fixture {
    root: tempfile::TempDir,
    store: Arc<FunctionStore>,
    fake: FakePins,
    images: ImageLeaseManager,
    manager: ImageDeploymentManager,
}
impl Fixture {
    async fn new(mode: DaemonMode, jobs: u8, budget: Duration) -> Self {
        let fake = FakePins::new(mode).await;
        let images = fake.owner(8, jobs);
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(FunctionStore::new(root.path()));
        let manager = ImageDeploymentManager::start(
            store.clone(),
            images.port(),
            ImageDeploymentConfig::new(jobs, budget).unwrap(),
        )
        .unwrap();
        Self {
            root,
            store,
            fake,
            images,
            manager,
        }
    }
    async fn idle(&self) {
        until(|| self.manager.status().accepted == 0).await;
    }
    async fn failed_preparation(&self, reason: ImageDeploymentFailure) {
        until(|| latest(&self.store).deployment == DeploymentState::CreateFailed(reason)).await;
        assert_eq!(self.manager.status().accepted, 1);
    }
    async fn ready(&self) {
        let pending = self.manager.create(record()).unwrap();
        assert_eq!(pending.latest.deployment.state(), "Pending");
        self.idle().await;
        assert_eq!(latest(&self.store).deployment, DeploymentState::Ready);
    }
    async fn close(self, failed: bool) {
        let Self {
            root,
            store,
            fake,
            mut images,
            manager,
        } = self;
        assert_eq!(manager.shutdown(deadline()).await.is_err(), failed);
        drop(manager);
        drop(store);
        images.shutdown(deadline()).await.unwrap();
        fake.close().await;
        drop(root);
    }
}

#[tokio::test]
async fn test_should_keep_accepted_preparation_owned_after_response_is_discarded() {
    let fixture = Fixture::new(DaemonMode::Normal, 2, Duration::from_secs(10)).await;
    drop(fixture.manager.create(record()).unwrap());
    fixture.idle().await;
    let active = latest(&fixture.store);
    assert_eq!(active.deployment, DeploymentState::Ready);
    assert_eq!(active.code_size, 4096);
    assert!(active.image.pinned().unwrap().is_live());
    assert!(!format!("{:?}", fixture.manager.status()).contains("private"));
    drop(active);
    fixture.close(false).await;
}

#[tokio::test]
async fn test_should_reject_capacity_before_resource_insertion_and_drain_after_cancellation() {
    let fixture = Fixture::new(DaemonMode::Never, 1, Duration::from_secs(10)).await;
    fixture.manager.create(record()).unwrap();
    fixture
        .fake
        .stats
        .until(|| fixture.fake.stats.tags.load(Ordering::Acquire) == 1)
        .await;
    assert!(matches!(
        fixture.manager.create(named("other")),
        Err(LambdaServiceError::TooManyRequests)
    ));
    assert!(!fixture.store.contains("other").unwrap());
    assert!(fixture.manager.shutdown(Instant::now()).await.is_err());
    fixture
        .failed_preparation(ImageDeploymentFailure::Cancelled)
        .await;
    assert_eq!(
        latest(&fixture.store).deployment,
        DeploymentState::CreateFailed(ImageDeploymentFailure::Cancelled)
    );
    assert!(fixture.images.port().status().pins > 0);
    fixture.fake.commands.send(Command::Commit).await.unwrap();
    fixture.idle().await;
    assert_eq!(fixture.fake.stats.tags.load(Ordering::Acquire), 1);
    fixture.close(false).await;
}

#[tokio::test]
async fn test_should_preserve_old_revision_during_update_and_after_preparation_failure() {
    let fixture = Fixture::new(DaemonMode::Normal, 2, Duration::from_secs(10)).await;
    fixture.ready().await;
    let before = fixture.store.get("worker").unwrap().unwrap();
    let accepted = fixture
        .manager
        .update(&before, "missing:v2".parse().unwrap(), Architecture::Arm64)
        .unwrap();
    assert_eq!(accepted.latest.deployment.update_status(), "InProgress");
    assert_eq!(accepted.latest.code_sha256, before.latest.code_sha256);
    fixture.idle().await;
    let failed = latest(&fixture.store);
    assert_eq!(
        failed.deployment,
        DeploymentState::UpdateFailed(ImageDeploymentFailure::ImageNotFound)
    );
    assert_eq!(failed.revision_id, before.latest.revision_id);
    assert_eq!(failed.code_sha256, before.latest.code_sha256);
    drop(failed);
    drop(accepted);
    drop(before);
    let before = fixture.store.get("worker").unwrap().unwrap();
    fixture.fake.commands.send(Command::Retarget).await.unwrap();
    fixture
        .manager
        .update(&before, "worker:v1".parse().unwrap(), Architecture::Arm64)
        .unwrap();
    fixture.idle().await;
    assert_eq!(latest(&fixture.store).deployment, DeploymentState::Ready);
    assert_ne!(
        latest(&fixture.store).code_sha256,
        before.latest.code_sha256
    );
    drop(before);
    fixture.close(false).await;
}

#[tokio::test]
async fn test_should_keep_deployment_deadline_absolute_and_leave_critical_tag_cleanup_owned() {
    let fixture = Fixture::new(DaemonMode::Never, 1, Duration::from_secs(1)).await;
    fixture.manager.create(record()).unwrap();
    fixture
        .fake
        .stats
        .until(|| fixture.fake.stats.tags.load(Ordering::Acquire) == 1)
        .await;
    // Notifications must recheck identity without restarting the preparation future.
    for _ in 0..4 {
        fixture
            .store
            .update("worker", |record| {
                record.tags.insert("progress".into(), "value".into());
            })
            .unwrap();
    }
    fixture
        .failed_preparation(ImageDeploymentFailure::DeadlineExceeded)
        .await;
    assert_eq!(
        latest(&fixture.store).deployment,
        DeploymentState::CreateFailed(ImageDeploymentFailure::DeadlineExceeded)
    );
    assert!(fixture.images.port().status().pins > 0);
    fixture.fake.commands.send(Command::Commit).await.unwrap();
    fixture.idle().await;
    fixture.close(false).await;
}

#[tokio::test]
async fn test_should_cancel_deleted_generation_without_waiting_for_its_deployment_deadline() {
    let fixture = Fixture::new(DaemonMode::Never, 1, Duration::from_secs(300)).await;
    fixture.manager.create(record()).unwrap();
    fixture
        .fake
        .stats
        .until(|| fixture.fake.stats.tags.load(Ordering::Acquire) == 1)
        .await;
    fixture.store.remove("worker").unwrap();
    let mut replacement = record();
    replacement.latest.description = "replacement".into();
    let resource = replacement.resource_id;
    fixture.store.insert(replacement).unwrap();
    fixture.manager.quiesce(deadline()).await.unwrap();
    assert_eq!(fixture.manager.status().accepted, 1);
    assert_eq!(
        fixture.store.get("worker").unwrap().unwrap().resource_id,
        resource
    );
    assert_eq!(latest(&fixture.store).description, "replacement");
    assert!(latest(&fixture.store).image.pinned().is_none());
    fixture.fake.commands.send(Command::Commit).await.unwrap();
    fixture.idle().await;
    fixture.close(false).await;
}

#[tokio::test]
async fn test_should_return_the_atomic_acceptance_receipt_after_immediate_deletion() {
    let fixture = Fixture::new(DaemonMode::Normal, 1, Duration::from_secs(10)).await;
    let reservation = fixture.manager.reserve().unwrap();
    let (job, receipt) = fixture.store.reserve_image_create(record()).unwrap();
    fixture.store.remove("worker").unwrap();
    let accepted = fixture.manager.accept(reservation, job, receipt);
    assert_eq!(accepted.latest.deployment.state(), "Pending");
    fixture.idle().await;
    assert!(!fixture.manager.status().failed);
    assert_eq!(fixture.fake.stats.tags.load(Ordering::Acquire), 0);
    fixture.close(false).await;
}

#[tokio::test]
async fn test_should_retain_join_when_drain_or_shutdown_waiter_is_abandoned() {
    let fixture = Fixture::new(DaemonMode::Never, 1, Duration::from_secs(10)).await;
    fixture.manager.create(record()).unwrap();
    fixture
        .fake
        .stats
        .until(|| fixture.fake.stats.tags.load(Ordering::Acquire) == 1)
        .await;
    let mut waiter = Box::pin(fixture.manager.quiesce(deadline()));
    assert!(matches!(
        waiter
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ));
    drop(waiter);
    assert!(!fixture.manager.status().accepting);
    assert_eq!(fixture.manager.status().accepted, 1);
    let mut waiter = Box::pin(fixture.manager.shutdown(deadline()));
    let _ = waiter
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()));
    drop(waiter);
    fixture
        .failed_preparation(ImageDeploymentFailure::Cancelled)
        .await;
    assert_eq!(
        latest(&fixture.store).deployment,
        DeploymentState::CreateFailed(ImageDeploymentFailure::Cancelled)
    );
    fixture.fake.commands.send(Command::Commit).await.unwrap();
    fixture.idle().await;
    fixture.close(false).await;
}

#[rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn test_should_settle_failed_workers_or_queued_root_work_and_cache_failure(
    #[case] root: bool,
) {
    let fixture = Fixture::new(DaemonMode::Normal, 2, Duration::from_secs(10)).await;
    if root {
        fixture
            .manager
            .health
            .crash_root
            .store(true, Ordering::Release);
    } else {
        fixture
            .manager
            .health
            .crash_worker
            .store(true, Ordering::Release);
    }
    fixture.manager.create(record()).unwrap();
    fixture.idle().await;
    assert!(fixture.manager.status().failed);
    assert_eq!(
        latest(&fixture.store).deployment,
        DeploymentState::CreateFailed(ImageDeploymentFailure::SupervisorFailed)
    );
    assert!(fixture.manager.shutdown(deadline()).await.is_err());
    assert!(fixture.manager.shutdown(deadline()).await.is_err());
    assert!(fixture.manager.create(named("other")).is_err());
    fixture.close(true).await;
}

#[tokio::test]
async fn test_should_recover_joined_rows_after_root_drop_outside_an_entered_runtime() {
    let fake = FakePins::new(DaemonMode::Never).await;
    let mut images = fake.owner(4, 1);
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(FunctionStore::new(root.path()));
    let job = store.insert_image_deployment(record()).unwrap();
    let health = Arc::new(Health::default());
    let (control, _) = watch::channel(Mode::Running);
    let emergency = Arc::new(CleanupTask::default());
    let (_sender, inbox) = mpsc::channel(1);
    let mut actor = Actor::builder()
        .inbox(inbox)
        .preparation(images.port())
        .control(control.clone())
        .health(health.clone())
        .runtime(Handle::current())
        .emergency(emergency.clone())
        .build();
    let capacity = Arc::new(Semaphore::new(1));
    actor.start(Envelope {
        id: Uuid::new_v4(),
        pending: Arc::new(
            Pending::builder()
                .job(job)
                .store(store.clone())
                .health(health.clone())
                .control(control)
                .build(),
        ),
        slot: capacity.clone().acquire_owned().await.unwrap(),
        deadline: deadline(),
    });
    fake.stats
        .until(|| fake.stats.tags.load(Ordering::Acquire) == 1)
        .await;
    let thread = std::thread::spawn(move || drop(actor));
    tokio::task::spawn_blocking(move || thread.join().unwrap())
        .await
        .unwrap();
    emergency.wait().await.unwrap();
    assert!(health.failed.load(Ordering::Acquire));
    assert_eq!(capacity.available_permits(), 0);
    assert_eq!(
        latest(&store).deployment,
        DeploymentState::CreateFailed(ImageDeploymentFailure::SupervisorFailed)
    );
    fake.commands.send(Command::Commit).await.unwrap();
    drop(store);
    images.shutdown(deadline()).await.unwrap();
    assert_eq!(capacity.available_permits(), 1);
    fake.close().await;
}

#[test]
fn test_should_validate_deployment_budgets_and_map_bounded_failure_classes() {
    assert!(ImageDeploymentConfig::new(0, Duration::from_secs(1)).is_err());
    assert!(ImageDeploymentConfig::new(33, Duration::from_secs(1)).is_err());
    assert!(ImageDeploymentConfig::new(1, Duration::from_nanos(1)).is_err());
    assert!(ImageDeploymentConfig::new(1, Duration::from_secs(3601)).is_err());
    assert_eq!(
        map_failure(&DockerError::Status(StatusCode::NOT_FOUND)),
        ImageDeploymentFailure::ImageNotFound
    );
    assert_eq!(
        map_failure(&DockerError::Quarantined),
        ImageDeploymentFailure::RuntimeUnavailable
    );
    assert_eq!(
        map_failure(&DockerError::Protocol),
        ImageDeploymentFailure::InvalidImage
    );
    for error in [
        DockerError::PullDenied,
        DockerError::PullFailed,
        DockerError::Image(ArtifactError::Inspection),
    ] {
        assert_eq!(map_failure(&error), ImageDeploymentFailure::InvalidImage);
    }
    assert_eq!(
        map_failure(&DockerError::Worker),
        ImageDeploymentFailure::SupervisorFailed
    );
}

#[derive(Debug)]
struct ControlPlaneExecutor;
#[async_trait]
impl Executor for ControlPlaneExecutor {
    fn image_available(&self) -> Result<(), ExecutorError> {
        Ok(())
    }
    fn available(&self, _: &InvokeRequest) -> Result<(), ExecutorError> {
        Err(ExecutorError::Unsupported(
            "Control-plane fixture does not execute code".into(),
        ))
    }
    async fn invoke(
        &self,
        _: InvokeRequest,
        _: InvocationContext,
    ) -> Result<InvokeResponse, ExecutorError> {
        Err(ExecutorError::Unsupported(
            "Control-plane fixture does not execute code".into(),
        ))
    }
    async fn shutdown(&self) -> Result<(), ExecutorError> {
        Ok(())
    }
}
fn input(name: &str) -> CreateFunctionInput {
    CreateFunctionInput {
        function_name: name.into(),
        role: "arn:aws:iam::000000000000:role/test".into(),
        package_type: Some("Image".into()),
        architectures: Some(vec!["arm64".into()]),
        code: FunctionCode {
            image_uri: Some("worker:v1".into()),
            ..Default::default()
        },
        ..Default::default()
    }
}
async fn wait_provider(provider: &RustackLambda) {
    until(|| {
        let config = provider.get_function_configuration("worker", None).unwrap();
        config.last_update_status.as_deref() != Some("InProgress")
    })
    .await;
    // Give the joined root a chance to reap the completed worker before another admission.
    tokio::task::yield_now().await;
}

#[tokio::test]
async fn test_should_route_provider_image_create_update_status_and_explicit_revision_preconditions()
{
    let fake = FakePins::new(DaemonMode::Normal).await;
    let mut images = fake.owner(8, 2);
    let root = tempfile::tempdir().unwrap();
    let config = LambdaConfig {
        executor: ExecutorBackend::Docker,
        ..Default::default()
    };
    let provider = RustackLambda::with_executor(
        FunctionStore::new(root.path()),
        config,
        Arc::new(ControlPlaneExecutor),
    )
    .with_image_preparation(
        images.port(),
        ImageDeploymentConfig::new(2, Duration::from_secs(10)).unwrap(),
    )
    .unwrap();
    let pending = provider.create_function(input("worker")).await.unwrap();
    assert_eq!(pending.state.as_deref(), Some("Pending"));
    assert!(pending.code_sha256.is_none());
    wait_provider(&provider).await;
    let before = provider.get_function_configuration("worker", None).unwrap();
    assert_eq!(before.state.as_deref(), Some("Active"));
    assert!(before.code_sha256.is_some());
    assert!(matches!(
        provider
            .update_function_code(
                "worker",
                UpdateFunctionCodeInput {
                    image_uri: Some("worker:v1".into()),
                    revision_id: Some("stale".into()),
                    ..Default::default()
                }
            )
            .await,
        Err(LambdaServiceError::PreconditionFailed)
    ));
    assert!(matches!(
        provider.update_function_configuration(
            "worker",
            &UpdateFunctionConfigurationInput {
                revision_id: Some("stale".into()),
                ..Default::default()
            }
        ),
        Err(LambdaServiceError::PreconditionFailed)
    ));
    fake.commands.send(Command::Retarget).await.unwrap();
    let updating = provider
        .update_function_code(
            "worker",
            UpdateFunctionCodeInput {
                image_uri: Some("worker:v1".into()),
                revision_id: before.revision_id.clone(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updating.state.as_deref(), Some("Active"));
    assert_eq!(updating.last_update_status.as_deref(), Some("InProgress"));
    assert_eq!(updating.code_sha256, before.code_sha256);
    wait_provider(&provider).await;
    let after = provider.get_function_configuration("worker", None).unwrap();
    assert_ne!(after.code_sha256, before.code_sha256);
    assert!(matches!(
        provider.publish_version(
            "worker",
            &PublishVersionInput {
                revision_id: before.revision_id,
                ..Default::default()
            }
        ),
        Err(LambdaServiceError::PreconditionFailed)
    ));
    let published = provider
        .publish_version("worker", &PublishVersionInput::default())
        .unwrap();
    assert_eq!(published.version.as_deref(), Some("1"));
    provider.shutdown().await;
    drop(provider);
    images.shutdown(deadline()).await.unwrap();
    fake.close().await;
}

#[tokio::test]
async fn test_should_reject_unqualified_image_deployment_before_resource_mutation() {
    let provider = RustackLambda::new(LambdaConfig::default());
    assert!(matches!(
        provider.create_function(input("worker")).await,
        Err(LambdaServiceError::DockerNotAvailable)
    ));
    assert!(!provider.store().contains("worker").unwrap());
    assert!(NoopExecutor::new().image_available().is_err());
    let fake = FakePins::new(DaemonMode::Normal).await;
    let mut images = fake.owner(2, 1);
    assert!(
        RustackLambda::new(LambdaConfig {
            executor: ExecutorBackend::Docker,
            ..Default::default()
        })
        .with_image_preparation(
            images.port(),
            ImageDeploymentConfig::new(1, Duration::from_secs(1)).unwrap()
        )
        .is_err()
    );
    images.shutdown(deadline()).await.unwrap();
    fake.close().await;
}

#[tokio::test]
async fn test_should_poll_ready_preparation_ahead_of_continuously_ready_unrelated_changes() {
    let (_stop, mut control) = watch::channel(Mode::Running);
    let (publisher, mut changes) = watch::channel(());
    for _ in 0..16 {
        publisher.send_replace(());
        let image =
            PinnedImage::fixture(crate::artifact::ResolvedImage::fixture(Architecture::Arm64));
        let prepared = std::future::ready(Ok(image));
        tokio::pin!(prepared);
        assert!(matches!(
            next(prepared.as_mut(), &mut control, &mut changes, deadline()).await,
            Ok(Progress::Prepared(Ok(_)))
        ));
    }
}

#[tokio::test]
async fn test_should_accept_a_long_overall_budget_while_bounding_each_docker_read() {
    let fixture = Fixture::new(DaemonMode::Normal, 1, Duration::from_secs(3600)).await;
    fixture.ready().await;
    assert_eq!(fixture.fake.stats.tags.load(Ordering::Acquire), 1);
    fixture.close(false).await;
}

#[tokio::test]
async fn test_should_keep_signalled_cleanup_running_when_owner_is_dropped_off_runtime() {
    let fixture = Fixture::new(DaemonMode::Never, 1, Duration::from_secs(300)).await;
    fixture.manager.create(record()).unwrap();
    fixture
        .fake
        .stats
        .until(|| fixture.fake.stats.tags.load(Ordering::Acquire) == 1)
        .await;
    let Fixture {
        root,
        store,
        fake,
        mut images,
        manager,
    } = fixture;
    let thread = std::thread::spawn(move || drop(manager));
    tokio::task::spawn_blocking(move || thread.join().unwrap())
        .await
        .unwrap();
    until(|| {
        latest(&store).deployment
            == DeploymentState::CreateFailed(ImageDeploymentFailure::Cancelled)
    })
    .await;
    fake.commands.send(Command::Commit).await.unwrap();
    drop(store);
    images.shutdown(deadline()).await.unwrap();
    fake.close().await;
    drop(root);
}

#[tokio::test]
async fn test_should_mark_an_empty_abnormally_dropped_root_as_failed() {
    let fake = FakePins::new(DaemonMode::Normal).await;
    let mut images = fake.owner(2, 1);
    let health = Arc::new(Health::default());
    let (control, _) = watch::channel(Mode::Running);
    let (_sender, inbox) = mpsc::channel(1);
    let actor = Actor::builder()
        .inbox(inbox)
        .preparation(images.port())
        .control(control.clone())
        .health(health.clone())
        .runtime(Handle::current())
        .emergency(Arc::new(CleanupTask::default()))
        .build();
    drop(actor);
    assert!(health.failed.load(Ordering::Acquire));
    assert_eq!(*control.borrow(), Mode::Cancel);
    images.shutdown(deadline()).await.unwrap();
    fake.close().await;
}

#[tokio::test]
async fn test_should_fail_quiescence_at_one_deadline_then_cancel_without_losing_ownership() {
    let fake = FakePins::new(DaemonMode::Never).await;
    let mut images = fake.owner(2, 1);
    let root = tempfile::tempdir().unwrap();
    let provider = RustackLambda::with_executor(
        FunctionStore::new(root.path()),
        LambdaConfig {
            executor: ExecutorBackend::Docker,
            ..Default::default()
        },
        Arc::new(ControlPlaneExecutor),
    )
    .with_image_preparation(
        images.port(),
        ImageDeploymentConfig::new(1, Duration::from_secs(300)).unwrap(),
    )
    .unwrap();
    provider.create_function(input("worker")).await.unwrap();
    fake.stats
        .until(|| fake.stats.tags.load(Ordering::Acquire) == 1)
        .await;
    assert!(provider.quiesce(Duration::from_millis(10)).await.is_err());
    until(|| {
        latest(provider.store()).deployment
            == DeploymentState::CreateFailed(ImageDeploymentFailure::Cancelled)
    })
    .await;
    fake.commands.send(Command::Commit).await.unwrap();
    provider.shutdown().await;
    drop(provider);
    images.shutdown(deadline()).await.unwrap();
    fake.close().await;
}

#[tokio::test]
async fn test_should_reject_deployment_capacity_larger_than_the_preparation_owner() {
    let fake = FakePins::new(DaemonMode::Normal).await;
    let mut images = fake.owner(2, 1);
    let root = tempfile::tempdir().unwrap();
    assert!(
        ImageDeploymentManager::start(
            Arc::new(FunctionStore::new(root.path())),
            images.port(),
            ImageDeploymentConfig::new(2, Duration::from_secs(1)).unwrap()
        )
        .is_err()
    );
    images.shutdown(deadline()).await.unwrap();
    fake.close().await;
}

#[tokio::test]
async fn test_should_validate_image_code_dry_run_without_preparation_or_state_changes() {
    let fake = FakePins::new(DaemonMode::Normal).await;
    let mut images = fake.owner(2, 1);
    let root = tempfile::tempdir().unwrap();
    let provider = RustackLambda::with_executor(
        FunctionStore::new(root.path()),
        LambdaConfig {
            executor: ExecutorBackend::Docker,
            ..Default::default()
        },
        Arc::new(ControlPlaneExecutor),
    )
    .with_image_preparation(
        images.port(),
        ImageDeploymentConfig::new(1, Duration::from_secs(10)).unwrap(),
    )
    .unwrap();
    provider.create_function(input("worker")).await.unwrap();
    wait_provider(&provider).await;
    let before = serde_json::to_value(provider.export_snapshot()).unwrap();
    let response = provider
        .update_function_code(
            "worker",
            UpdateFunctionCodeInput {
                image_uri: Some("missing:new-image".into()),
                dry_run: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(response.last_update_status.as_deref(), Some("Successful"));
    assert_eq!(
        serde_json::to_value(provider.export_snapshot()).unwrap(),
        before
    );
    assert_eq!(fake.stats.tags.load(Ordering::Acquire), 1);
    assert_eq!(fake.stats.absent.load(Ordering::Acquire), 0);
    provider.shutdown().await;
    drop(provider);
    images.shutdown(deadline()).await.unwrap();
    fake.close().await;
}

#[tokio::test]
async fn test_should_keep_cancelled_deployment_admission_after_outer_worker_join_until_image_io_retires()
 {
    let fake = FakePins::new(DaemonMode::Never).await;
    let mut images = fake.owner(4, 1);
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(FunctionStore::new(root.path()));
    let job = store.insert_image_deployment(record()).unwrap();
    let health = Arc::new(Health::default());
    let (control, _) = watch::channel(Mode::Running);
    let (sender, inbox) = mpsc::channel(1);
    let mut actor = Actor::builder()
        .inbox(inbox)
        .preparation(images.port())
        .control(control.clone())
        .health(health.clone())
        .runtime(Handle::current())
        .emergency(Arc::new(CleanupTask::default()))
        .build();
    let capacity = Arc::new(Semaphore::new(1));
    let pending = Arc::new(
        Pending::builder()
            .job(job)
            .store(store.clone())
            .health(health)
            .control(control)
            .build(),
    );
    actor.start(Envelope {
        id: Uuid::new_v4(),
        pending: pending.clone(),
        slot: capacity.clone().acquire_owned().await.unwrap(),
        deadline: deadline(),
    });
    fake.stats
        .until(|| fake.stats.tags.load(Ordering::Acquire) == 1)
        .await;
    store.remove("worker").unwrap();
    until(|| pending.settled.load(Ordering::Acquire)).await;
    let joined = timeout(Duration::from_secs(2), actor.jobs.join_next_with_id())
        .await
        .unwrap()
        .unwrap();
    actor.reap(&joined);
    assert!(actor.rows.is_empty());
    assert_eq!(capacity.available_permits(), 0);
    assert!(capacity.clone().try_acquire_owned().is_err());
    assert_eq!(images.port().status().preparing, 1);
    fake.commands.send(Command::Commit).await.unwrap();
    until(|| images.port().status().pins == 0).await;
    assert_eq!(capacity.available_permits(), 1);
    assert!(!store.contains("worker").unwrap());
    drop(sender);
    timeout(Duration::from_secs(2), actor.run())
        .await
        .unwrap()
        .unwrap();
    images.shutdown(deadline()).await.unwrap();
    fake.close().await;
}
