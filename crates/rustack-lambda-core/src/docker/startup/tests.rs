use std::{path::PathBuf, time::Duration};

use rstest::rstest;
use rustack_core::settings::RuntimeBudgets;
use tokio::time::Instant;

use super::*;
use crate::docker::{EnvironmentConfig, ImageLeaseConfig};

fn settings() -> DockerRuntimeSettings {
    DockerRuntimeSettings::from_lookup(
        |key| match key {
            "LAMBDA_DOCKER_RELAY_IMAGE" => Some("relay:v1".into()),
            "LAMBDA_DOCKER_NETWORK" => Some("none".into()),
            "LAMBDA_DOCKER_SOCKET" => {
                Some("unix:///private/rustack-missing-fixture/docker.sock".into())
            }
            _ => None,
        },
        &RuntimeBudgets::default(),
        Duration::from_secs(600),
    )
    .unwrap()
}
fn lambda() -> LambdaConfig {
    LambdaConfig {
        executor: ExecutorBackend::Docker,
        ..Default::default()
    }
}
async fn begin(config: DockerStartupConfig) -> DockerRuntime {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match DockerRuntime::new(config.clone()) {
                Ok(owner) => return owner,
                Err(DockerStartupError::Worker) => tokio::task::yield_now().await,
                Err(error) => panic!("unexpected startup constructor: {error}"),
            }
        }
    })
    .await
    .unwrap()
}

fn config(directory: PathBuf) -> DockerStartupConfig {
    DockerStartupConfig::new(
        settings(),
        directory,
        GatewayInstanceId::generate().unwrap(),
        lambda(),
    )
    .unwrap()
}
#[rstest]
#[case(0)]
#[case(1)]
#[case(2)]
#[case(3)]
#[case(4)]
#[test]
fn test_should_reject_mismatching_startup_inputs_before_io(#[case] case: u8) {
    let mut lambda = lambda();
    match case {
        0 => lambda.executor = ExecutorBackend::Native,
        1 => lambda.max_warm_instances = 2,
        2 => lambda.init_timeout = Duration::ZERO,
        3 => lambda.init_timeout = Duration::from_secs(901),
        _ => lambda.idle_timeout = Duration::from_secs(599),
    }
    assert!(matches!(
        DockerStartupConfig::new(
            settings(),
            "/private/data".into(),
            GatewayInstanceId::generate().unwrap(),
            lambda
        ),
        Err(DockerStartupError::Configuration)
    ));
}
#[test]
fn test_should_reject_disconnected_configured_endpoint_before_startup() {
    let settings = DockerRuntimeSettings::from_lookup(
        |key| match key {
            "LAMBDA_DOCKER_RELAY_IMAGE" => Some("relay:v1".into()),
            "LAMBDA_DOCKER_NETWORK" => Some("none".into()),
            "LAMBDA_DOCKER_GATEWAY_ENDPOINT" => Some("http://localhost:4566".into()),
            _ => None,
        },
        &RuntimeBudgets::default(),
        Duration::from_secs(600),
    )
    .unwrap();
    assert!(
        DockerStartupConfig::new(
            settings,
            "/private/data".into(),
            GatewayInstanceId::generate().unwrap(),
            lambda()
        )
        .is_err()
    );
}
#[tokio::test]
async fn test_should_retain_failed_attempt_without_replay_and_release_after_owned_shutdown() {
    let directory = tempfile::tempdir().unwrap();
    let mut owner = begin(config(directory.path().into())).await;
    let status = owner.status();
    assert!(!status.capabilities().ready && !status.capabilities().sdk_gateway);
    assert!(status.capabilities().capacity.is_none());
    assert!(
        serde_json::to_value(status.capabilities())
            .unwrap()
            .get("capacity")
            .is_none()
    );
    assert!(owner.installation().is_err());
    assert!(
        owner
            .qualify(Instant::now() + Duration::from_secs(10))
            .await
            .is_err()
    );
    assert_eq!(owner.phase, Phase::Failed);
    assert!(matches!(
        owner
            .qualify(Instant::now() + Duration::from_secs(10))
            .await,
        Err(DockerStartupError::Unavailable)
    ));
    assert!(matches!(
        DirectoryLease::acquire(
            directory.path().into(),
            Instant::now() + Duration::from_secs(5)
        )
        .await,
        Err(DirectoryLeaseError::InUse)
    ));
    owner
        .shutdown(Instant::now() + Duration::from_secs(10))
        .await
        .unwrap();
    owner
        .shutdown(Instant::now() + Duration::from_secs(10))
        .await
        .unwrap();
    assert!(!status.capabilities().ready);
    let lease = DirectoryLease::acquire(
        directory.path().into(),
        Instant::now() + Duration::from_secs(5),
    )
    .await
    .unwrap();
    lease.require_journaled().unwrap();
}
#[tokio::test]
async fn test_should_reject_expired_qualification_without_publishing_or_touching_directory() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("absent");
    let mut owner = begin(config(path.clone())).await;
    assert!(matches!(
        owner.qualify(Instant::now()).await,
        Err(DockerStartupError::Deadline)
    ));
    assert!(!tokio::fs::try_exists(path).await.unwrap());
    assert!(owner.installation().is_err());
    owner
        .shutdown(Instant::now() + Duration::from_secs(5))
        .await
        .unwrap();
}
#[test]
fn test_should_redact_startup_configuration_and_error_diagnostics() {
    let config = config("/private/OPERATOR-SECRET".into());
    assert!(!format!("{config:?}").contains("OPERATOR-SECRET"));
    assert_eq!(
        config.settings.container_policy().idle(),
        Duration::from_secs(600)
    );
    assert!(DockerRuntime::new(config).is_err());
}

#[tokio::test]
async fn test_should_keep_abandoned_qualification_io_and_directory_until_same_cleanup_joins() {
    let root = tempfile::tempdir().unwrap();
    let mut daemon = crate::docker::tests::Daemon::new();
    let socket = daemon.socket_uri();
    let settings = DockerRuntimeSettings::from_lookup(
        |key| match key {
            "LAMBDA_DOCKER_RELAY_IMAGE" => Some("relay:v1".into()),
            "LAMBDA_DOCKER_NETWORK" => Some("none".into()),
            "LAMBDA_DOCKER_SOCKET" => Some(socket.clone()),
            _ => None,
        },
        &RuntimeBudgets::default(),
        Duration::from_secs(600),
    )
    .unwrap();
    let config = DockerStartupConfig::new(
        settings,
        root.path().into(),
        GatewayInstanceId::generate().unwrap(),
        lambda(),
    )
    .unwrap();
    let mut owner = begin(config).await;
    let mut qualification = Box::pin(owner.qualify(Instant::now() + Duration::from_secs(20)));
    let request = tokio::select! {
        request=daemon.next()=>request,
        result=&mut qualification=>panic!("qualification completed before inventory: {result:?}"),
    };
    assert!(request.head.contains("/containers/json?"));
    drop(qualification);
    assert_eq!(owner.phase, Phase::Qualifying);
    assert!(matches!(
        owner
            .qualify(Instant::now() + Duration::from_secs(10))
            .await,
        Err(DockerStartupError::Unavailable)
    ));
    assert!(matches!(
        owner.shutdown(Instant::now()).await,
        Err(DockerStartupError::Deadline)
    ));
    drop(request);
    owner
        .shutdown(Instant::now() + Duration::from_secs(10))
        .await
        .unwrap();
    daemon.close().await;
    DirectoryLease::acquire(root.path().into(), Instant::now() + Duration::from_secs(5))
        .await
        .unwrap();
}

#[tokio::test]
async fn test_should_keep_cancelled_version_connect_owned_until_response_and_repeated_shutdown() {
    use tokio::net::UnixListener;
    let root = tempfile::tempdir().unwrap();
    let socket_root = tempfile::tempdir().unwrap();
    let socket = socket_root.path().join("docker.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let settings_lookup = std::collections::BTreeMap::from([
        ("LAMBDA_DOCKER_RELAY_IMAGE", "relay:v1".to_owned()),
        ("LAMBDA_DOCKER_NETWORK", "none".to_owned()),
        (
            "LAMBDA_DOCKER_SOCKET",
            format!("unix://{}", socket.display()),
        ),
    ]);
    let settings = DockerRuntimeSettings::from_lookup(
        |key| settings_lookup.get(key).cloned(),
        &RuntimeBudgets::default(),
        Duration::from_secs(600),
    )
    .unwrap();
    let config = DockerStartupConfig::new(
        settings,
        root.path().into(),
        GatewayInstanceId::generate().unwrap(),
        lambda(),
    )
    .unwrap();
    let mut owner = begin(config).await;
    let mut work = Box::pin(owner.qualify(Instant::now() + Duration::from_secs(10)));
    let (stream, _) = tokio::select! {connection=listener.accept()=>connection.unwrap(),result=&mut work=>panic!("early connect {result:?}")};
    let request = crate::docker::tests::read_request(stream).await.unwrap();
    assert!(request.head.starts_with("GET /version "));
    drop(work);
    assert!(matches!(
        owner.shutdown(Instant::now()).await,
        Err(DockerStartupError::Deadline)
    ));
    assert!(matches!(
        DirectoryLease::acquire(root.path().into(), Instant::now() + Duration::from_secs(3)).await,
        Err(DirectoryLeaseError::InUse)
    ));
    request
        .reply(200, br#"{"ApiVersion":"1.53","MinAPIVersion":"1.44"}"#)
        .await;
    owner
        .shutdown(Instant::now() + Duration::from_secs(10))
        .await
        .unwrap();
    owner
        .shutdown(Instant::now() + Duration::from_secs(10))
        .await
        .unwrap();
    DirectoryLease::acquire(root.path().into(), Instant::now() + Duration::from_secs(3))
        .await
        .unwrap();
}
#[tokio::test]
async fn test_should_cache_connect_worker_failure_and_never_repoll_completed_join() {
    let root = tempfile::tempdir().unwrap();
    let mut owner = begin(config(root.path().into())).await;
    owner.state.as_mut().unwrap().connecting = Some(tokio::spawn(async {
        panic!("injected connect worker panic")
    }));
    for _ in 0..2 {
        assert!(matches!(
            owner
                .shutdown(Instant::now() + Duration::from_secs(5))
                .await,
            Err(DockerStartupError::Cleanup(_))
        ));
    }
}
#[tokio::test]
async fn test_should_continue_physical_image_and_wire_cleanup_after_sticky_environment_fault() {
    use uuid::Uuid;

    use crate::docker::{EnvironmentManager, ImageLeaseManager, ImagePinScope, tests::Daemon};
    let root = tempfile::tempdir().unwrap();
    let mut owner = begin(config(root.path().into())).await;
    let daemon = Daemon::new();
    let wire = daemon.wire().await;
    let port = wire.port();
    let directory = DirectoryLease::testing(Uuid::new_v4(), Uuid::new_v4());
    let environments = EnvironmentManager::start(
        port.clone(),
        directory.clone(),
        EnvironmentConfig::default(),
    )
    .unwrap();
    environments.abort_test_worker();
    let images = ImageLeaseManager::start(
        port.clone(),
        ImagePinScope::new(directory),
        ImageLeaseConfig::default(),
    )
    .unwrap();
    let image_status = images.port();
    let state = owner.state.as_mut().unwrap();
    state.wire = Some(wire);
    state.images = Some(images);
    state.environments = Some(environments);
    for _ in 0..2 {
        assert!(matches!(
            owner
                .shutdown(Instant::now() + Duration::from_secs(5))
                .await,
            Err(DockerStartupError::Cleanup(_))
        ));
    }
    assert!(!image_status.status().accepting);
    assert_eq!(image_status.status().pins, 0);
    assert!(matches!(
        port.info(Instant::now() + Duration::from_secs(3)).await,
        Err(DockerError::Stopped)
    ));
    daemon.close().await;
}

async fn synthetic_ready(
    owner: &mut DockerRuntime,
) -> (crate::docker::tests::Daemon, crate::docker::DockerPort) {
    use uuid::Uuid;

    use crate::docker::{
        DockerExecutorConfig, EnvironmentProbeConfig, ImageLeaseManager, ImagePinScope,
        NetworkSelection, plan::tests::image_with, probe::tests::synthetic_resource, tests::Daemon,
    };

    let daemon = Daemon::new();
    let wire = daemon.wire().await;
    let port = wire.port();
    let directory = DirectoryLease::testing(Uuid::new_v4(), Uuid::new_v4());
    let environments = EnvironmentManager::start(
        port.clone(),
        directory.clone(),
        EnvironmentConfig::default(),
    )
    .unwrap();
    let images = ImageLeaseManager::start(
        port.clone(),
        ImagePinScope::new(directory.clone()),
        ImageLeaseConfig::default(),
    )
    .unwrap();
    let network: NetworkSelection = "none".parse().unwrap();
    let base = EnvironmentProbeConfig::builder()
        .directory(directory.clone())
        .relay(image_with("1000", vec![]))
        .environments(environments.port())
        .policy(
            owner
                .state
                .as_ref()
                .unwrap()
                .config
                .settings
                .container_policy()
                .clone(),
        )
        .network(
            network
                .qualify(&port, Instant::now() + Duration::from_secs(5))
                .await
                .unwrap(),
        )
        .build();
    owner
        .qualification
        .evidence
        .set(
            Evidence::builder()
                .base(base.clone())
                .resource(synthetic_resource(&base))
                .endpoint(None)
                .images(images.port())
                .docker(port.clone())
                .architecture(Architecture::Arm64)
                .build(),
        )
        .unwrap();
    let executor = Arc::new(
        DockerExecutor::new(
            DockerExecutorConfig::builder()
                .directory(directory.clone())
                .relay(base.relay)
                .policy(base.policy)
                .environments(base.environments)
                .readiness(owner.qualification.clone())
                .build(),
        )
        .unwrap(),
    );
    let state = owner.state.as_mut().unwrap();
    state.directory = Some(directory);
    state.environments = Some(environments);
    state.images = Some(images);
    state.wire = Some(wire);
    state.executor = Some(executor);
    owner.phase = Phase::Ready;
    (daemon, port)
}
#[tokio::test]
async fn test_should_refuse_late_installation_after_dependency_revocation_and_join_faulted_graph() {
    let root = tempfile::tempdir().unwrap();
    let mut owner = begin(config(root.path().into())).await;
    let (daemon, port) = synthetic_ready(&mut owner).await;
    owner.phase = Phase::Ready;
    assert!(owner.installation().is_ok());
    let environments = owner.state.as_mut().unwrap().environments.as_mut().unwrap();
    environments.abort_test_worker();
    assert!(
        environments
            .shutdown(Instant::now() + Duration::from_secs(5))
            .await
            .is_err()
    );
    assert!(matches!(
        owner.installation(),
        Err(DockerStartupError::Unavailable)
    ));
    assert!(matches!(
        owner
            .shutdown(Instant::now() + Duration::from_secs(5))
            .await,
        Err(DockerStartupError::Cleanup(_))
    ));
    assert!(matches!(
        port.info(Instant::now() + Duration::from_secs(5)).await,
        Err(DockerError::Stopped)
    ));
    daemon.close().await;
}
#[test]
fn test_should_reject_mutable_zip_selection_before_startup() {
    assert!(
        DockerRuntimeSettings::from_lookup(
            |key| match key {
                "LAMBDA_DOCKER_RELAY_IMAGE" => Some("relay:v1".into()),
                "LAMBDA_DOCKER_NETWORK" => Some("none".into()),
                "LAMBDA_DOCKER_ZIP_IMAGE" => Some("zip:v1".into()),
                _ => None,
            },
            &RuntimeBudgets::default(),
            Duration::from_secs(600),
        )
        .is_err()
    );
}

#[rstest]
#[case::insufficient_pins(1, false)]
#[case::separate_source_and_relay(2, true)]
#[test]
fn test_should_accept_pinned_zip_only_with_separate_source_pin_capacity(
    #[case] pins: usize,
    #[case] accepted: bool,
) {
    let mut budgets = RuntimeBudgets::default();
    budgets.lambda_docker_image_pins = pins;
    budgets.lambda_docker_deployment_jobs = 1;
    let settings = DockerRuntimeSettings::from_lookup(
        |key| match key {
            "LAMBDA_DOCKER_RELAY_IMAGE" => Some("relay:v1".into()),
            "LAMBDA_DOCKER_NETWORK" => Some("none".into()),
            "LAMBDA_DOCKER_ZIP_IMAGE" => Some(format!("base@sha256:{}", "a".repeat(64))),
            _ => None,
        },
        &budgets,
        Duration::from_secs(600),
    )
    .unwrap();
    assert_eq!(
        DockerStartupConfig::new(
            settings,
            "/private/data".into(),
            GatewayInstanceId::generate().unwrap(),
            lambda()
        )
        .is_ok(),
        accepted
    );
}

#[test]
fn test_should_never_issue_handler_sdk_context_without_qualified_endpoint_evidence() {
    let qualification = Arc::new(Qualification::default());
    let credentials: LocalSdkCredentials = ("local-key".into(), "local-secret".into())
        .try_into()
        .unwrap();
    assert!(matches!(
        qualification.sdk_environment(credentials),
        Err(DockerStartupError::Unavailable)
    ));
}
