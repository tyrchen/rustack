use std::{sync::Arc, time::Duration};

use rstest::rstest;
use rustack_lambda_model::types::ImageConfig;
use serde_json::to_vec;
use tempfile::TempDir;
use tokio::{spawn, sync::mpsc, task::yield_now, time::timeout};
use uuid::Uuid;

use super::*;
use crate::{
    artifact::{Architecture, DeploymentArtifact},
    docker::{
        CodeVolumeConfig, CodeVolumeIdentity, CodeVolumeManager, DockerExecutorError, DockerPort,
        EnvironmentError, EnvironmentManager, EnvironmentPlan, EnvironmentPort,
        EnvironmentSettings, Operation,
        code::tests::{elf, source, zip},
        code_volume::tests::{
            allocated, base, begin_volume, code, deadline, finish_loader, fixture, retire,
        },
        plan::tests::{function_body, image_with, policy, revision},
    },
    error::LambdaServiceError,
    executor::ExecutorError,
};

async fn plan(
    directory: &DirectoryLease,
    runtime: DockerZipRuntime,
) -> (TempDir, Arc<EnvironmentPlan>) {
    let (root, code) = source(zip(&[("bootstrap", &elf(Architecture::Arm64), 0o755)])).await;
    let mut revision = revision();
    revision.artifact = DeploymentArtifact::Zip(code);
    revision.runtime = Some("provided.al2023".into());
    revision.handler = Some("handler.entry".into());
    let plan = EnvironmentPlan::new_zip(
        directory.clone(),
        image_with("1000", vec![]),
        Arc::new(revision),
        policy(),
        runtime,
    )
    .unwrap();
    (root, Arc::new(plan))
}
fn environment(docker: DockerPort, directory: DirectoryLease) -> EnvironmentManager {
    EnvironmentManager::start(
        docker,
        directory,
        EnvironmentSettings::builder()
            .environments(2)
            .io(Duration::from_secs(1))
            .start(Duration::from_secs(10))
            .build()
            .try_into()
            .unwrap(),
    )
    .unwrap()
}
async fn empty(port: &EnvironmentPort) {
    timeout(Duration::from_secs(5), async {
        while port.status().allocated != 0 {
            yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_should_materialize_fixed_provided_body_and_reject_function_creation_before_code_ready()
 {
    let (_root, directory, mut daemon, mut wire, docker) = fixture().await;
    let mut code = CodeVolumeManager::start(
        docker.clone(),
        directory.clone(),
        CodeVolumeConfig::default(),
    )
    .unwrap();
    let (image, _stop, _released) = base(&directory);
    let runtime = DockerZipRuntime::new(&directory, image, code.port()).unwrap();
    let (_source, plan) = plan(&directory, runtime).await;
    let body = function_body(&plan);
    assert_eq!(body.entrypoint, Some(vec!["/var/task/bootstrap".into()]));
    assert_eq!(body.cmd, Some(vec![]));
    assert_eq!(body.user.as_deref(), Some("1000:1000"));
    assert_eq!(body.working_dir.as_deref(), Some("/var/task"));
    let host = body.host_config.as_ref().unwrap();
    let mounts = host.mounts.as_ref().unwrap();
    assert_eq!(mounts.len(), 1);
    let mount = mounts.first().unwrap();
    assert_eq!(mount.target.as_deref(), Some("/var/task"));
    assert_eq!(mount.read_only, Some(true));
    assert_eq!(mount.volume_options.as_ref().unwrap().no_copy, Some(true));
    assert!(
        body.env
            .as_ref()
            .unwrap()
            .contains(&"_HANDLER=handler.entry".into())
    );
    assert!(matches!(
        plan.create_function(&docker, deadline()).await,
        Err(DockerError::Configuration)
    ));
    assert!(
        timeout(Duration::from_millis(20), daemon.next())
            .await
            .is_err()
    );
    code.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_keep_environment_memory_until_cancelled_preparation_physically_retires() {
    let (_root, directory, mut daemon, mut wire, docker) = fixture().await;
    let mut code = CodeVolumeManager::start(
        docker.clone(),
        directory.clone(),
        CodeVolumeConfig::default(),
    )
    .unwrap();
    let (image, _stop, _released) = base(&directory);
    let runtime = DockerZipRuntime::new(&directory, image, code.port()).unwrap();
    let (_source, plan) = plan(&directory, runtime).await;
    let mut owner = environment(docker, directory.clone());
    let port = owner.port();
    let mut handle = port.spawn_shared(plan.clone(), deadline()).unwrap();
    let (request, volume) = begin_volume(&mut daemon, &directory).await;
    handle.cancel();
    assert!(matches!(
        handle
            .shutdown(Instant::now() + Duration::from_millis(20))
            .await,
        Err(EnvironmentError::Deadline)
    ));
    assert_eq!(port.status().allocated, 1);
    assert!(port.status().memory_bytes > 0);
    assert!(!plan.code_clean());
    assert_eq!(code.port().status().volumes, 1);
    request.reply(201, &to_vec(&volume).unwrap()).await;
    retire(&mut daemon, &volume).await;
    handle.shutdown(deadline()).await.unwrap();
    empty(&port).await;
    assert_eq!(port.status().memory_bytes, 0);
    assert!(plan.code_clean());
    assert!(matches!(
        handle.status().error,
        Some(EnvironmentError::Docker(DockerError::Cancelled))
    ));
    owner.shutdown(deadline()).await.unwrap();
    code.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[rstest]
#[case(1)]
#[case(2)]
#[case(3)]
#[tokio::test]
async fn test_should_reap_stage_panic_before_enqueue_without_inventing_acceptance(
    #[case] point: u8,
) {
    let (_root, directory, mut daemon, mut wire, docker) = fixture().await;
    let mut code = CodeVolumeManager::start(
        docker.clone(),
        directory.clone(),
        CodeVolumeConfig::default(),
    )
    .unwrap();
    let (image, _stop, _released) = base(&directory);
    let runtime = DockerZipRuntime::new(&directory, image, code.port()).unwrap();
    let (_source, plan) = plan(&directory, runtime).await;
    plan.zip
        .as_ref()
        .unwrap()
        .fault
        .store(point, Ordering::Release);
    let mut owner = environment(docker, directory.clone());
    let port = owner.port();
    let mut handle = port.spawn_shared(plan.clone(), deadline()).unwrap();
    timeout(Duration::from_secs(5), async {
        while !port.status().worker_failed {
            yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        handle.shutdown(deadline()).await,
        Err(EnvironmentError::Worker)
    ));
    empty(&port).await;
    assert_eq!(port.status().memory_bytes, 0);
    assert!(plan.code_clean());
    assert_eq!(code.port().status().volumes, 0);
    assert_eq!(directory.code_records(deadline()).await.unwrap(), []);
    assert!(
        timeout(Duration::from_millis(20), daemon.next())
            .await
            .is_err()
    );
    assert!(matches!(
        owner.shutdown(deadline()).await,
        Err(EnvironmentError::Worker)
    ));
    code.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_join_original_upload_and_receipt_after_environment_root_loss() {
    let (_root, directory, mut daemon, mut wire, docker) = fixture().await;
    let mut code = CodeVolumeManager::start(
        docker.clone(),
        directory.clone(),
        CodeVolumeConfig::default(),
    )
    .unwrap();
    let (image, _stop, _released) = base(&directory);
    let runtime = DockerZipRuntime::new(&directory, image, code.port()).unwrap();
    let (_source, plan) = plan(&directory, runtime).await;
    let mut owner = environment(docker, directory.clone());
    let port = owner.port();
    let mut handle = port.spawn_shared(plan.clone(), deadline()).unwrap();
    let (volume, inspected) = allocated(&mut daemon, &directory).await;
    let upload = daemon.next().await;
    assert!(upload.head.starts_with("PUT "));
    owner.abort_test_worker();
    assert!(matches!(
        handle
            .shutdown(Instant::now() + Duration::from_millis(20))
            .await,
        Err(EnvironmentError::Deadline)
    ));
    assert_eq!(port.status().allocated, 1);
    assert!(port.status().memory_bytes > 0);
    assert!(!plan.code_clean());
    upload.reply(200, b"").await;
    finish_loader(&mut daemon, &inspected).await;
    retire(&mut daemon, &volume).await;
    assert!(matches!(
        handle.shutdown(deadline()).await,
        Err(EnvironmentError::Worker)
    ));
    empty(&port).await;
    assert_eq!(port.status().memory_bytes, 0);
    assert!(plan.code_clean());
    assert!(matches!(
        owner.shutdown(deadline()).await,
        Err(EnvironmentError::Worker)
    ));
    code.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_exclude_cross_manager_plan_replay_and_reset_only_unaccepted_claims() {
    let (_root, directory, mut daemon, mut wire, docker) = fixture().await;
    let mut code = CodeVolumeManager::start(
        docker.clone(),
        directory.clone(),
        CodeVolumeConfig::default(),
    )
    .unwrap();
    let (image, _stop, _released) = base(&directory);
    let runtime = DockerZipRuntime::new(&directory, image, code.port()).unwrap();
    let (_source, plan) = plan(&directory, runtime).await;
    let mut first = environment(docker.clone(), directory.clone());
    let mut second = environment(docker, directory.clone());
    let one = first.port();
    let two = second.port();
    let mut queued = one.spawn_shared(plan.clone(), deadline()).unwrap();
    assert!(two.spawn_shared(plan.clone(), deadline()).is_err());
    queued.cancel();
    queued.shutdown(deadline()).await.unwrap();
    empty(&one).await;
    assert!(!plan.zip.as_ref().unwrap().claimed.load(Ordering::Acquire));
    let mut committed = two.spawn_shared(plan.clone(), deadline()).unwrap();
    let (request, volume) = begin_volume(&mut daemon, &directory).await;
    assert!(one.spawn_shared(plan.clone(), deadline()).is_err());
    committed.cancel();
    request.reply(201, &to_vec(&volume).unwrap()).await;
    retire(&mut daemon, &volume).await;
    committed.shutdown(deadline()).await.unwrap();
    empty(&two).await;
    assert!(one.spawn_shared(plan, deadline()).is_err());
    assert_eq!(one.status().allocated, 0);
    assert_eq!(two.status().allocated, 0);
    first.shutdown(deadline()).await.unwrap();
    second.shutdown(deadline()).await.unwrap();
    code.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_keep_warm_identity_independent_of_fresh_volume_and_isolate_handler_changes() {
    let (_root, directory, daemon, mut wire, docker) = fixture().await;
    let mut owner =
        CodeVolumeManager::start(docker, directory.clone(), CodeVolumeConfig::default()).unwrap();
    let (image, _stop, _released) = base(&directory);
    let runtime = DockerZipRuntime::new(&directory, image, owner.port()).unwrap();
    let (_source, original) = plan(&directory, runtime.clone()).await;
    let relay = image_with("1000", vec![]);
    let second = EnvironmentPlan::new_zip(
        directory.clone(),
        relay.clone(),
        Arc::new(original.revision().clone()),
        policy(),
        runtime.clone(),
    )
    .unwrap();
    assert_eq!(original.warm_key(), second.warm_key());
    assert_ne!(original.function_key(), second.function_key());
    let mut revised = original.revision().clone();
    revised.handler = Some("other.entry".into());
    let changed = EnvironmentPlan::new_zip(
        directory.clone(),
        relay,
        Arc::new(revised),
        policy(),
        runtime,
    )
    .unwrap();
    assert_ne!(original.warm_key(), changed.warm_key());
    owner.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[rstest]
#[case::runtime(0)]
#[case::handler(1)]
#[case::architecture(2)]
#[case::image_config(3)]
#[tokio::test]
async fn test_should_reject_incompatible_zip_plan_before_any_effect(#[case] case: u8) {
    let (_root, directory, mut daemon, mut wire, docker) = fixture().await;
    let mut owner =
        CodeVolumeManager::start(docker, directory.clone(), CodeVolumeConfig::default()).unwrap();
    let (image, _stop, _released) = base(&directory);
    let runtime = DockerZipRuntime::new(&directory, image, owner.port()).unwrap();
    let (_source, original) = plan(&directory, runtime.clone()).await;
    let mut revision = original.revision().clone();
    match case {
        0 => revision.runtime = Some("python3.13".into()),
        1 => revision.handler = Some("bad\nhandler".into()),
        2 => revision.architecture = Architecture::X86_64,
        _ => revision.image_config = Some(ImageConfig::default()),
    }
    assert!(
        EnvironmentPlan::new_zip(
            directory.clone(),
            image_with("1000", vec![]),
            Arc::new(revision),
            policy(),
            runtime
        )
        .is_err()
    );
    assert!(
        timeout(Duration::from_millis(20), daemon.next())
            .await
            .is_err()
    );
    owner.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_throttle_only_no_enqueue_code_admission_and_release_parent_memory() {
    let (_root, directory, mut daemon, mut wire, docker) = fixture().await;
    let mut code_owner = CodeVolumeManager::start(
        docker.clone(),
        directory.clone(),
        CodeVolumeConfig::new(2, 1, Duration::from_secs(1), Duration::from_secs(2)).unwrap(),
    )
    .unwrap();
    let (image, _stop, _released) = base(&directory);
    let runtime = DockerZipRuntime::new(&directory, image.clone(), code_owner.port()).unwrap();
    let (_first_source, archive) = code().await;
    let first = code_owner
        .port()
        .start_preparation(
            CodeVolumeIdentity::new(Uuid::new_v4(), Uuid::new_v4()).unwrap(),
            archive,
            image,
            deadline(),
        )
        .unwrap();
    let mut first_retirement = first.retirement();
    let (held, volume) = begin_volume(&mut daemon, &directory).await;
    let (_source, plan) = plan(&directory, runtime).await;
    let mut owner = environment(docker, directory.clone());
    let port = owner.port();
    let mut handle = port.spawn_shared(plan.clone(), deadline()).unwrap();
    let error = handle.wait_ready(deadline()).await.unwrap_err();
    assert!(matches!(error, EnvironmentError::Busy));
    assert!(matches!(
        LambdaServiceError::from(ExecutorError::Docker(DockerExecutorError::Environment(
            error
        ))),
        LambdaServiceError::TooManyRequests
    ));
    handle.shutdown(deadline()).await.unwrap();
    empty(&port).await;
    assert_eq!(port.status().memory_bytes, 0);
    assert!(plan.code_clean());
    assert_eq!(code_owner.port().status().volumes, 1);
    assert_eq!(code_owner.port().status().preparing, 1);
    assert!(
        timeout(Duration::from_millis(20), daemon.next())
            .await
            .is_err()
    );
    drop(first);
    held.reply(201, &to_vec(&volume).unwrap()).await;
    retire(&mut daemon, &volume).await;
    first_retirement.wait(deadline()).await.unwrap();
    owner.shutdown(deadline()).await.unwrap();
    code_owner.shutdown(deadline()).await.unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_keep_accepted_nested_wire_busy_as_infrastructure_error_until_retirement() {
    let (_root, directory, mut daemon, mut wire, docker) = fixture().await;
    let (sender, mut receiver) = mpsc::channel(32);
    let actual = docker.raw.business.clone();
    let mut code_docker = docker.clone();
    code_docker.raw.business = sender;
    let forward = spawn(async move {
        while let Some(message) = receiver.recv().await {
            if matches!(message.request.operation, Operation::UploadCode) {
                let _ = message.reply.send(Err(DockerError::Busy));
            } else {
                actual.send(message).await.unwrap();
            }
        }
    });
    let mut code_owner =
        CodeVolumeManager::start(code_docker, directory.clone(), CodeVolumeConfig::default())
            .unwrap();
    let (image, _stop, _released) = base(&directory);
    let runtime = DockerZipRuntime::new(&directory, image, code_owner.port()).unwrap();
    let (_source, plan) = plan(&directory, runtime).await;
    let mut owner = environment(docker, directory.clone());
    let port = owner.port();
    let mut handle = port.spawn_shared(plan.clone(), deadline()).unwrap();
    let (volume, inspected) = allocated(&mut daemon, &directory).await;
    let error = handle.wait_ready(deadline()).await.unwrap_err();
    assert!(matches!(error, EnvironmentError::Docker(DockerError::Busy)));
    assert!(matches!(
        LambdaServiceError::from(ExecutorError::Docker(DockerExecutorError::Environment(
            error
        ))),
        LambdaServiceError::Internal { .. }
    ));
    assert_eq!(port.status().allocated, 1);
    assert!(port.status().memory_bytes > 0);
    assert!(!plan.code_clean());
    finish_loader(&mut daemon, &inspected).await;
    retire(&mut daemon, &volume).await;
    handle.shutdown(deadline()).await.unwrap();
    empty(&port).await;
    assert_eq!(port.status().memory_bytes, 0);
    assert!(plan.code_clean());
    owner.shutdown(deadline()).await.unwrap();
    code_owner.shutdown(deadline()).await.unwrap();
    timeout(Duration::from_secs(3), forward)
        .await
        .unwrap()
        .unwrap();
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
