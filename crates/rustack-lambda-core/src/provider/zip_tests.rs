//! Control-plane validation only; this fixture never executes a handler.

use std::{
    io::{Cursor, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};
use rstest::rstest;
use rustack_lambda_model::types::FunctionCode;
use tempfile::TempDir;
use tokio::{spawn, sync::Notify, time::timeout};
use zip::{ZipWriter, write::SimpleFileOptions};

use super::*;
use crate::{
    docker::DockerExecutorError,
    executor::{ExecutorError, InvokeResponse},
    invocation::InvocationContext,
};

#[derive(Debug)]
struct ZipControl(Arc<AtomicBool>);
#[async_trait]
impl Executor for ZipControl {
    fn image_available(&self) -> Result<(), ExecutorError> {
        Ok(())
    }
    fn zip_available(&self, architecture: Architecture) -> Result<(), ExecutorError> {
        if !self.0.load(Ordering::Acquire) {
            return Err(ExecutorError::Docker(DockerExecutorError::Unavailable));
        }
        if architecture != Architecture::Arm64 {
            return Err(ExecutorError::Unsupported("control platform".into()));
        }
        Ok(())
    }
    async fn invoke(
        &self,
        _: InvokeRequest,
        _: InvocationContext,
    ) -> Result<InvokeResponse, ExecutorError> {
        Err(ExecutorError::Unsupported(
            "control-plane-only fixture".into(),
        ))
    }
    async fn shutdown(&self) -> Result<(), ExecutorError> {
        Ok(())
    }
}
fn fixture() -> (TempDir, RustackLambda) {
    let root = tempfile::tempdir().unwrap();
    let provider = RustackLambda::with_executor(
        FunctionStore::new(root.path()),
        LambdaConfig {
            executor: ExecutorBackend::Docker,
            ..Default::default()
        },
        Arc::new(ZipControl(Arc::new(AtomicBool::new(true)))),
    );
    (root, provider)
}
fn package(elf: bool) -> String {
    let mut body = vec![0_u8; 64];
    if elf {
        body.get_mut(..7)
            .unwrap()
            .copy_from_slice(b"\x7fELF\x02\x01\x01");
        body.get_mut(16..18)
            .unwrap()
            .copy_from_slice(&2_u16.to_le_bytes());
        body.get_mut(18..20)
            .unwrap()
            .copy_from_slice(&183_u16.to_le_bytes());
        body.get_mut(20..24)
            .unwrap()
            .copy_from_slice(&1_u32.to_le_bytes());
        body.get_mut(52..54)
            .unwrap()
            .copy_from_slice(&64_u16.to_le_bytes());
    }
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file(
            "bootstrap",
            SimpleFileOptions::default().unix_permissions(0o755),
        )
        .unwrap();
    writer.write_all(&body).unwrap();
    STANDARD.encode(writer.finish().unwrap().into_inner())
}
fn input() -> CreateFunctionInput {
    CreateFunctionInput {
        function_name: "provided-control".into(),
        runtime: Some("provided.al2023".into()),
        handler: Some("handler.entry".into()),
        role: "arn:aws:iam::000000000000:role/test-role".into(),
        architectures: Some(vec!["arm64".into()]),
        code: FunctionCode {
            zip_file: Some(package(true)),
            ..Default::default()
        },
        ..Default::default()
    }
}

#[tokio::test]
async fn test_should_preserve_latest_and_publications_after_bad_zip_or_configuration_updates() {
    let (_root, provider) = fixture();
    let mut input = input();
    input.publish = Some(true);
    provider.create_function(input).await.unwrap();
    let before = provider.get_record("provided-control").unwrap();
    assert!(matches!(
        provider
            .update_function_code(
                "provided-control",
                UpdateFunctionCodeInput {
                    zip_file: Some(package(false)),
                    ..Default::default()
                }
            )
            .await,
        Err(LambdaServiceError::InvalidZipFile { .. })
    ));
    assert!(matches!(
        provider.update_function_configuration(
            "provided-control",
            &UpdateFunctionConfigurationInput {
                runtime: Some("python3.13".into()),
                ..Default::default()
            }
        ),
        Err(LambdaServiceError::InvalidParameter { .. })
    ));
    let after = provider.get_record("provided-control").unwrap();
    assert_eq!(before.latest.revision_id, after.latest.revision_id);
    assert_eq!(before.latest.code_sha256, after.latest.code_sha256);
    assert_eq!(before.latest.code_path, after.latest.code_path);
    assert_eq!(before.versions.len(), after.versions.len());
    assert_eq!(before.latest.runtime, after.latest.runtime);
    let loaded = provider
        .store
        .capture_artifact(after.latest.code_path.as_ref().unwrap())
        .await
        .unwrap();
    assert_eq!(loaded.sha256(), before.latest.code_sha256);
    assert!(
        provider
            .export_snapshot_before(Instant::now() + Duration::from_secs(5))
            .await
            .is_err()
    );
    provider
        .shutdown_before(Instant::now() + Duration::from_secs(5))
        .await
        .unwrap();
}

#[rstest]
#[case("python3.13", "handler.entry", "arm64")]
#[case("provided.al2", "handler.entry", "arm64")]
#[case("provided.al2023", "bad handler", "arm64")]
#[case("provided.al2023", "handler.entry", "x86_64")]
#[tokio::test]
async fn test_should_reject_uninstalled_runtime_handler_or_platform_before_publication(
    #[case] runtime: &str,
    #[case] handler: &str,
    #[case] architecture: &str,
) {
    let (_root, provider) = fixture();
    let mut input = input();
    input.runtime = Some(runtime.into());
    input.handler = Some(handler.into());
    input.architectures = Some(vec![architecture.into()]);
    assert!(matches!(
        provider.create_function(input).await,
        Err(LambdaServiceError::InvalidParameter { .. })
    ));
    assert!(provider.get_record("provided-control").is_err());
    provider
        .shutdown_before(Instant::now() + Duration::from_secs(5))
        .await
        .unwrap();
}

#[tokio::test]
async fn test_should_reject_bootstrap_before_initial_commit_and_wrong_architecture_before_update_staging()
 {
    let (_root, provider) = fixture();
    let mut bad = input();
    bad.code.zip_file = Some(package(false));
    assert!(matches!(
        provider.create_function(bad).await,
        Err(LambdaServiceError::InvalidZipFile { .. })
    ));
    assert!(provider.get_record("provided-control").is_err());
    provider.create_function(input()).await.unwrap();
    let before = provider.get_record("provided-control").unwrap();
    assert!(matches!(
        provider
            .update_function_code(
                "provided-control",
                UpdateFunctionCodeInput {
                    zip_file: Some(package(true)),
                    architectures: Some(vec!["x86_64".into()]),
                    ..Default::default()
                }
            )
            .await,
        Err(LambdaServiceError::InvalidParameter { .. })
    ));
    assert_eq!(
        provider
            .get_record("provided-control")
            .unwrap()
            .latest
            .revision_id,
        before.latest.revision_id
    );
    provider
        .shutdown_before(Instant::now() + Duration::from_secs(5))
        .await
        .unwrap();
}

#[derive(Debug)]
struct HeldZip {
    entered: Notify,
    release: Notify,
}
#[async_trait]
impl S3CodeFetcher for HeldZip {
    async fn fetch_code(
        &self,
        _: &str,
        _: &str,
        _: Option<&str>,
    ) -> Result<Bytes, S3CodeFetchError> {
        self.entered.notify_one();
        self.release.notified().await;
        Ok(Bytes::from(STANDARD.decode(package(true)).unwrap()))
    }
}

#[rstest]
#[case::create(false)]
#[case::update(true)]
#[tokio::test]
async fn test_should_recheck_zip_capability_at_commit_after_held_fetch(#[case] update: bool) {
    let root = tempfile::tempdir().unwrap();
    let live = Arc::new(AtomicBool::new(true));
    let fetch = Arc::new(HeldZip {
        entered: Notify::new(),
        release: Notify::new(),
    });
    let provider = Arc::new(
        RustackLambda::with_executor(
            FunctionStore::new(root.path()),
            LambdaConfig {
                executor: ExecutorBackend::Docker,
                ..Default::default()
            },
            Arc::new(ZipControl(live.clone())),
        )
        .with_code_fetcher(fetch.clone()),
    );
    if update {
        let mut request = input();
        request.publish = Some(true);
        provider.create_function(request).await.unwrap();
    }
    let before = provider.get_record("provided-control").ok();
    let call = provider.clone();
    let waiting = spawn(async move {
        if update {
            call.update_function_code(
                "provided-control",
                UpdateFunctionCodeInput {
                    s3_bucket: Some("test-bucket".into()),
                    s3_key: Some("test.zip".into()),
                    publish: Some(true),
                    ..Default::default()
                },
            )
            .await
        } else {
            let mut request = input();
            request.code = FunctionCode {
                s3_bucket: Some("test-bucket".into()),
                s3_key: Some("test.zip".into()),
                ..Default::default()
            };
            call.create_function(request).await
        }
    });
    timeout(Duration::from_secs(5), fetch.entered.notified())
        .await
        .unwrap();
    live.store(false, Ordering::Release);
    fetch.release.notify_one();
    let result = timeout(Duration::from_secs(5), waiting)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        result,
        Err(LambdaServiceError::DockerNotAvailable)
    ));
    if let Some(before) = before {
        let after = provider.get_record("provided-control").unwrap();
        assert_eq!(after.latest.revision_id, before.latest.revision_id);
        assert_eq!(after.latest.code_sha256, before.latest.code_sha256);
        assert_eq!(after.latest.code_path, before.latest.code_path);
        assert_eq!(after.versions.len(), before.versions.len());
    } else {
        assert!(provider.get_record("provided-control").is_err());
    }
}

#[rstest]
#[case::maximum(128, true)]
#[case::over_limit(129, false)]
#[tokio::test]
async fn test_should_apply_same_handler_byte_bound_to_create_and_configuration(
    #[case] length: usize,
    #[case] accepted: bool,
) {
    let (_root, provider) = fixture();
    let mut request = input();
    request.handler = Some("x".repeat(length));
    let result = provider.create_function(request).await;
    assert_eq!(result.is_ok(), accepted);
    if !accepted {
        provider.create_function(input()).await.unwrap();
    }
    let before = provider.get_record("provided-control").unwrap();
    let result = provider.update_function_configuration(
        "provided-control",
        &UpdateFunctionConfigurationInput {
            handler: Some("x".repeat(length)),
            ..Default::default()
        },
    );
    assert_eq!(result.is_ok(), accepted);
    if !accepted {
        assert_eq!(
            provider
                .get_record("provided-control")
                .unwrap()
                .latest
                .revision_id,
            before.latest.revision_id
        );
    }
}
