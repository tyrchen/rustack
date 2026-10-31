//! Image export budget failures must preserve both existing archive generations.

use std::io::{Cursor, Write};

use anyhow::ensure;
use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};
use rustack_lambda_core::{
    artifact::Architecture,
    config::LambdaConfig,
    executor::{Executor, ExecutorBackend, ExecutorError, InvokeRequest, InvokeResponse},
    invocation::InvocationContext,
    storage::{LayerRecord, LayerStoreSnapshot, PolicyStatement},
};
use rustack_lambda_model::{
    input::{CreateFunctionInput, PublishVersionInput},
    types::FunctionCode,
};
use zip::{ZipWriter, write::SimpleFileOptions};

use super::*;

#[derive(Debug)]
struct MetadataOnlyExecutor;
#[async_trait]
impl Executor for MetadataOnlyExecutor {
    fn image_available(&self) -> Result<(), ExecutorError> {
        Ok(())
    }
    fn available(&self, _: &InvokeRequest) -> Result<(), ExecutorError> {
        Err(ExecutorError::Unsupported("metadata-only fixture".into()))
    }
    async fn invoke(
        &self,
        _: InvokeRequest,
        _: InvocationContext,
    ) -> Result<InvokeResponse, ExecutorError> {
        Err(ExecutorError::Unsupported("metadata-only fixture".into()))
    }
    async fn shutdown(&self) -> Result<(), ExecutorError> {
        Ok(())
    }
}
#[rstest::rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn test_should_preserve_generations_when_live_image_metadata_cannot_be_exported(
    #[case] deep: bool,
) -> Result<()> {
    let directory = tempfile::tempdir()?;
    let config = SnapshotConfig {
        root: directory.path().to_owned(),
        name: SnapshotName::try_from("image-export".to_owned())?,
    };
    config.save(&RuntimeProviders::default(), "good").await?;
    let previous = directory.path().join(".image-export.previous");
    fs::create_dir_all(&previous).await?;
    let original = fs::read(config.snapshot_dir().join(MANIFEST_FILE)).await?;
    fs::write(previous.join(MANIFEST_FILE), &original).await?;
    let provider = Arc::new(RustackLambda::with_executor(
        rustack_lambda_core::storage::FunctionStore::new(directory.path().join("code")),
        LambdaConfig {
            executor: ExecutorBackend::Docker,
            ..LambdaConfig::default()
        },
        Arc::new(MetadataOnlyExecutor),
    ));
    let mut layer: LayerRecord = serde_json::from_value(
        serde_json::json!({"name":"bad","layerArn":"arn:aws:lambda:us-east-1:000000000000:layer:bad","nextVersion":2,
        "versions":{"1":{"version":1,"description":"","compatibleRuntimes":[],"compatibleArchitectures":[],"licenseInfo":null,"codeSha256":"unused","codeSize":0,"createdDate":"2026-10-27T00:00:00Z","layerArn":"arn:aws:lambda:us-east-1:000000000000:layer:bad","layerVersionArn":"arn:aws:lambda:us-east-1:000000000000:layer:bad:1","policy":{"statements":[]}}}}),
    )?;
    let version = layer
        .versions
        .get_mut(&1)
        .context("layer fixture version")?;
    if deep {
        let mut condition = serde_json::Value::Null;
        for _ in 0..50_000 {
            condition = serde_json::Value::Array(vec![condition]);
        }
        version.policy.statements.push(PolicyStatement {
            sid: "deep".into(),
            effect: "Allow".into(),
            principal: "*".into(),
            action: "lambda:GetLayerVersion".into(),
            resource: "*".into(),
            condition: Some(condition),
        });
    } else {
        version.description = "x".repeat(20 * 1024 * 1024);
    }
    provider.layer_store().import_snapshot(LayerStoreSnapshot {
        layers: vec![layer],
    });
    let mut providers = RuntimeProviders::default();
    providers.register_lambda(provider);
    let error = config
        .save(&providers, "bad")
        .await
        .err()
        .context("accepted oversized/deep live Image export")?;
    assert!(format!("{error:#}").contains("Invalid or oversized Docker archive metadata"));
    assert_eq!(
        fs::read(config.snapshot_dir().join(MANIFEST_FILE)).await?,
        original
    );
    assert_eq!(fs::read(previous.join(MANIFEST_FILE)).await?, original);
    Ok(())
}

#[derive(Debug)]
struct ManagedZipMetadataExecutor;
#[async_trait]
impl Executor for ManagedZipMetadataExecutor {
    fn image_available(&self) -> Result<(), ExecutorError> {
        Ok(())
    }
    fn docker_zip_architecture(&self) -> Option<Architecture> {
        Some(Architecture::Arm64)
    }
    fn zip_available(&self, architecture: Architecture) -> Result<(), ExecutorError> {
        if architecture == Architecture::Arm64 {
            Ok(())
        } else {
            Err(ExecutorError::Unsupported(
                "controlled ZIP architecture".into(),
            ))
        }
    }
    async fn invoke(
        &self,
        _: InvokeRequest,
        _: InvocationContext,
    ) -> Result<InvokeResponse, ExecutorError> {
        Err(ExecutorError::Unsupported("snapshot-only fixture".into()))
    }
    async fn shutdown(&self) -> Result<(), ExecutorError> {
        Ok(())
    }
}
fn deep_tree_zip() -> Result<String> {
    let mut elf = vec![0_u8; 64];
    elf.get_mut(..7)
        .context("ELF prefix")?
        .copy_from_slice(b"\x7fELF\x02\x01\x01");
    elf.get_mut(16..18)
        .context("ELF type")?
        .copy_from_slice(&2_u16.to_le_bytes());
    elf.get_mut(18..20)
        .context("ELF architecture")?
        .copy_from_slice(&183_u16.to_le_bytes());
    elf.get_mut(20..24)
        .context("ELF version")?
        .copy_from_slice(&1_u32.to_le_bytes());
    elf.get_mut(52..54)
        .context("ELF header")?
        .copy_from_slice(&64_u16.to_le_bytes());
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file(
        "bootstrap",
        SimpleFileOptions::default().unix_permissions(0o755),
    )?;
    zip.write_all(&elf)?;
    for branch in 0..16 {
        let path = format!("branch{branch}/{}/payload", vec!["d"; 32].join("/"));
        zip.start_file(path, SimpleFileOptions::default())?;
    }
    Ok(STANDARD.encode(zip.finish()?.into_inner()))
}
#[tokio::test]
async fn test_should_preserve_both_generations_when_shared_zip_revisions_exceed_restore_node_budget()
-> Result<()> {
    let directory = tempfile::tempdir()?;
    let config = SnapshotConfig {
        root: directory.path().to_owned(),
        name: SnapshotName::try_from("zip-export".to_owned())?,
    };
    config.save(&RuntimeProviders::default(), "good").await?;
    let previous = directory.path().join(".zip-export.previous");
    fs::create_dir_all(&previous).await?;
    let original = fs::read(config.snapshot_dir().join(MANIFEST_FILE)).await?;
    fs::write(previous.join(MANIFEST_FILE), &original).await?;
    let provider = Arc::new(RustackLambda::with_executor(
        rustack_lambda_core::storage::FunctionStore::new(directory.path().join("code")),
        LambdaConfig {
            executor: ExecutorBackend::Docker,
            ..Default::default()
        },
        Arc::new(ManagedZipMetadataExecutor),
    ));
    provider
        .create_function(CreateFunctionInput {
            function_name: "bounded-zip".into(),
            runtime: Some("provided.al2023".into()),
            handler: Some("handler.entry".into()),
            role: "arn:aws:iam::000000000000:role/test".into(),
            architectures: Some(vec!["arm64".into()]),
            code: FunctionCode {
                zip_file: Some(deep_tree_zip()?),
                ..Default::default()
            },
            ..Default::default()
        })
        .await?;
    // Original artifact ownership is shared; each saved revision still charges nodes.
    for _ in 0..39 {
        provider.publish_version("bounded-zip", &PublishVersionInput::default())?;
    }
    let mut providers = RuntimeProviders::default();
    providers.register_lambda(provider.clone());
    let error = config
        .save(&providers, "over-profile")
        .await
        .err()
        .context("accepted unrestorable ZIP generation")?;
    ensure!(
        format!("{error:#}").contains("Invalid or oversized Docker ZIP archive candidate"),
        "wrong producer guard rejected ZIP archive"
    );
    ensure!(
        fs::read(config.snapshot_dir().join(MANIFEST_FILE)).await? == original,
        "current generation replaced"
    );
    ensure!(
        fs::read(previous.join(MANIFEST_FILE)).await? == original,
        "previous generation replaced"
    );
    provider
        .shutdown_before(Deadline::now() + Duration::from_secs(10))
        .await?;
    Ok(())
}
