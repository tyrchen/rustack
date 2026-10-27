//! Image export budget failures must preserve both existing archive generations.

use async_trait::async_trait;
use rustack_lambda_core::{
    config::LambdaConfig,
    executor::{Executor, ExecutorBackend, ExecutorError, InvokeRequest, InvokeResponse},
    invocation::InvocationContext,
    storage::{LayerRecord, LayerStoreSnapshot, PolicyStatement},
};

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
    assert!(format!("{error:#}").contains("Invalid or oversized Docker image archive metadata"));
    assert_eq!(
        fs::read(config.snapshot_dir().join(MANIFEST_FILE)).await?,
        original
    );
    assert_eq!(fs::read(previous.join(MANIFEST_FILE)).await?, original);
    Ok(())
}
