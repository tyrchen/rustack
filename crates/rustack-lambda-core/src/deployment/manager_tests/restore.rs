//! Complete-candidate restore must not publish partial resources or live placeholders.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::*;
use crate::{
    artifact::{ImageArtifact, ImageMetadata, ResolvedImage},
    docker::ImageResolver,
    provider::LambdaSnapshot,
    storage::{AliasRecord, EventSourceMappingRecord, LayerRecord, LayerStoreSnapshot},
};

struct ImportFixture {
    fake: FakePins,
    images: ImageLeaseManager,
    provider: Arc<RustackLambda>,
    _root: tempfile::TempDir,
}
impl ImportFixture {
    async fn new(mode: DaemonMode, old_pin: bool) -> Self {
        let fake = FakePins::new(mode).await;
        let images = fake.owner(8, 2);
        let root = tempfile::tempdir().unwrap();
        let store = FunctionStore::new(root.path());
        let mut old = named("old");
        if old_pin {
            let image = images
                .port()
                .prepare(
                    &"worker:v1".parse().unwrap(),
                    Architecture::Arm64,
                    None,
                    deadline(),
                )
                .await
                .unwrap();
            old.latest.code_sha256 = image.resolved().code_sha256().unwrap();
            old.latest.code_size = image.unpacked_size().get();
            old.latest.image = ImageArtifact::Pinned(image);
        } else {
            old.latest.deployment =
                DeploymentState::CreateFailed(ImageDeploymentFailure::Cancelled);
        }
        store.insert(old).unwrap();
        let provider = Arc::new(
            RustackLambda::with_executor(
                store,
                LambdaConfig {
                    executor: ExecutorBackend::Docker,
                    ..LambdaConfig::default()
                },
                Arc::new(ControlPlaneExecutor),
            )
            .with_image_preparation(
                images.port(),
                ImageDeploymentConfig::new(2, Duration::from_secs(10)).unwrap(),
            )
            .unwrap(),
        );
        provider.layer_store().import_snapshot(LayerStoreSnapshot {
            layers: vec![LayerRecord {
                name: "old-layer".into(),
                layer_arn: "arn:aws:lambda:us-east-1:000000000000:layer:old-layer".into(),
                versions: BTreeMap::new(),
                next_version: 1,
            }],
        });
        let mapping: EventSourceMappingRecord = serde_json::from_value(json!({
            "uuid":"old-mapping","eventSourceArn":"arn:aws:sqs:us-east-1:000000000000:old",
            "functionArn":"arn:aws:lambda:us-east-1:000000000000:function:old","enabled":false,
            "batchSize":1,"maximumBatchingWindowInSeconds":0,"functionResponseTypes":[],
            "state":"Disabled","stateTransitionReason":"fixture","lastModified":0,"lastProcessingResult":"None"
        })).unwrap();
        provider.esm_store().create(mapping);
        Self {
            fake,
            images,
            provider,
            _root: root,
        }
    }
    async fn saved(&self) -> ResolvedImage {
        ImageResolver::new(self.fake.wire.port())
            .resolve_local(
                &"worker:v1".parse().unwrap(),
                Architecture::Arm64,
                None,
                deadline(),
            )
            .await
            .unwrap()
    }
    fn state(&self) -> Value {
        serde_json::to_value(self.provider.export_snapshot()).unwrap()
    }
    async fn close(mut self) {
        self.provider.shutdown_before(deadline()).await.unwrap();
        drop(self.provider);
        self.images.shutdown(deadline()).await.unwrap();
        self.fake.close().await;
    }
}
fn saved_record(name: &str, saved: &ResolvedImage) -> FunctionRecord {
    let mut value = named(name);
    value.latest.code_sha256 = saved.code_sha256().unwrap();
    value.latest.code_size = 4096;
    value.latest.image = ImageArtifact::Restored(saved.clone());
    value
}
fn snapshot(records: Vec<FunctionRecord>) -> LambdaSnapshot {
    let store = FunctionStore::new("unused-image-snapshot-source");
    for record in records {
        store.insert(record).unwrap();
    }
    LambdaSnapshot {
        functions: store.export_snapshot(),
        ..LambdaSnapshot::default()
    }
}

#[tokio::test]
async fn test_should_restore_latest_published_and_alias_with_one_shared_exact_lease() {
    let fixture = ImportFixture::new(DaemonMode::Normal, true).await;
    let saved = fixture.saved().await;
    let mut record = saved_record("restored", &saved);
    record.tags.insert("retained".into(), "yes".into());
    let mut published = record.latest.clone();
    published.version = "1".into();
    record.versions.insert(1, published);
    record.next_version = 2;
    record.aliases.insert(
        "prod".into(),
        AliasRecord {
            name: "prod".into(),
            function_version: "1".into(),
            description: "saved".into(),
            routing_config: None,
            revision_id: Uuid::new_v4().to_string(),
        },
    );
    let source = snapshot(vec![record]);
    let expected = serde_json::to_value(&source).unwrap();
    fixture.fake.commands.send(Command::Retarget).await.unwrap();
    fixture
        .provider
        .import_snapshot_before(source, deadline())
        .await
        .unwrap();
    until(|| fixture.images.port().status().pins == 1).await;
    assert_eq!(fixture.state(), expected);
    fixture.provider.validate_runtime_restore().unwrap();
    let restored = fixture.provider.store().get("restored").unwrap().unwrap();
    assert!(restored.latest.image.pinned().unwrap().is_live());
    assert!(
        restored
            .versions
            .get(&1)
            .unwrap()
            .image
            .pinned()
            .unwrap()
            .is_live()
    );
    assert_eq!(fixture.fake.stats.tags.load(Ordering::Acquire), 2);
    drop(restored);
    fixture.close().await;
}

#[rstest]
#[case("duplicate")]
#[case("memory")]
#[case("hash")]
#[case("missing-metadata")]
#[case("empty-plan")]
#[case("alias")]
#[case("region")]
#[case("account")]
#[tokio::test]
async fn test_should_validate_all_records_scope_and_plans_before_any_pin(#[case] corruption: &str) {
    let fixture = ImportFixture::new(DaemonMode::Normal, true).await;
    let saved = fixture.saved().await;
    let mut source = snapshot(vec![saved_record("a", &saved), saved_record("z", &saved)]);
    let before = fixture.state();
    let last = source.functions.functions.last_mut().unwrap();
    match corruption {
        "memory" => last.latest.memory_size = 0,
        "hash" => last.latest.code_sha256 = "mismatch".into(),
        "missing-metadata" => last.latest.resolved_image = None,
        "empty-plan" => {
            last.latest.image_config = Some(rustack_lambda_model::types::ImageConfig {
                entry_point: Some(vec![]),
                command: Some(vec![]),
                ..Default::default()
            });
        }
        "alias" => {
            last.aliases.insert(
                "prod".into(),
                AliasRecord {
                    name: "prod".into(),
                    function_version: "7".into(),
                    description: String::new(),
                    routing_config: None,
                    revision_id: Uuid::new_v4().to_string(),
                },
            );
        }
        "region" => last.arn = last.arn.replace("us-east-1", "us-west-2"),
        "account" => last.arn = last.arn.replace("000000000000", "111111111111"),
        "duplicate" => source
            .functions
            .functions
            .push(source.functions.functions.first().unwrap().clone()),
        _ => panic!("fixed corruption table"),
    }
    assert!(
        fixture
            .provider
            .import_snapshot_before(source, deadline())
            .await
            .is_err()
    );
    assert_eq!(fixture.fake.stats.tags.load(Ordering::Acquire), 1);
    assert_eq!(fixture.state(), before);
    assert!(
        fixture
            .provider
            .store()
            .get("old")
            .unwrap()
            .unwrap()
            .latest
            .image
            .pinned()
            .unwrap()
            .is_live()
    );
    fixture.close().await;
}

#[rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn test_should_drop_prepared_candidates_on_later_size_or_missing_content_failure(
    #[case] missing: bool,
) {
    let fixture = ImportFixture::new(DaemonMode::Normal, true).await;
    let saved = fixture.saved().await;
    let mut source = snapshot(vec![saved_record("a", &saved), saved_record("z", &saved)]);
    let last = source.functions.functions.last_mut().unwrap();
    if missing {
        let mut metadata: ImageMetadata = saved.into();
        metadata.image_id = format!("sha256:{}", "e".repeat(64)).parse().unwrap();
        metadata.inspection_hash = ImageMetadata::execution_fingerprint(
            &metadata.image_id,
            metadata.selected_manifest.as_ref(),
            metadata.architecture,
            metadata.size_bytes,
            &metadata.execution,
        )
        .unwrap();
        let absent: ResolvedImage = metadata.try_into().unwrap();
        last.latest.code_sha256 = absent.code_sha256().unwrap();
        last.latest.resolved_image = Some(absent);
    } else {
        last.latest.code_size = 4097;
    }
    let before = fixture.state();
    let result = fixture
        .provider
        .import_snapshot_before(source, deadline())
        .await;
    if missing {
        assert!(
            matches!(result, Err(LambdaServiceError::ImageRestore { source }) if matches!(*source, DockerError::Status(StatusCode::NOT_FOUND)))
        );
    } else {
        assert!(matches!(
            result,
            Err(LambdaServiceError::InvalidParameter { .. })
        ));
    }
    until(|| fixture.images.port().status().pins == 1).await;
    assert_eq!(fixture.state(), before);
    assert_eq!(fixture.fake.stats.tags.load(Ordering::Acquire), 2);
    assert_eq!(fixture.fake.stats.deletes.load(Ordering::Acquire), 1);
    fixture.close().await;
}

#[tokio::test]
async fn test_should_reject_expired_commit_after_complete_pin_preparation_without_old_resource_changes()
 {
    let fixture = ImportFixture::new(DaemonMode::Normal, true).await;
    let saved = fixture.saved().await;
    let before = fixture.state();
    let source = snapshot(vec![saved_record("new", &saved)]);
    let candidate = fixture
        .provider
        .store()
        .prepare_image_snapshot(source.functions, &fixture.images.port(), deadline())
        .await
        .unwrap();
    assert_eq!(fixture.fake.stats.tags.load(Ordering::Acquire), 2);
    assert!(
        matches!(candidate.commit_before(Instant::now()), Err(LambdaServiceError::ImageRestore { source }) if matches!(*source, DockerError::Deadline))
    );
    until(|| fixture.images.port().status().pins == 1).await;
    assert_eq!(fixture.state(), before);
    fixture.close().await;
}

#[tokio::test]
async fn test_should_retain_uncertain_restore_mutation_after_caller_cancellation_without_publication()
 {
    let fixture = ImportFixture::new(DaemonMode::Never, false).await;
    let saved = fixture.saved().await;
    let before = fixture.state();
    let source = snapshot(vec![saved_record("new", &saved)]);
    let provider = fixture.provider.clone();
    let caller =
        tokio::spawn(async move { provider.import_snapshot_before(source, deadline()).await });
    fixture
        .fake
        .stats
        .until(|| fixture.fake.stats.tags.load(Ordering::Acquire) == 1)
        .await;
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    assert_eq!(fixture.images.port().status().pins, 1);
    assert_eq!(fixture.state(), before);
    fixture.fake.commands.send(Command::Commit).await.unwrap();
    fixture.images.port().wait_empty(deadline()).await.unwrap();
    assert_eq!(fixture.fake.stats.tags.load(Ordering::Acquire), 1);
    assert_eq!(fixture.fake.stats.deletes.load(Ordering::Acquire), 1);
    assert_eq!(fixture.state(), before);
    fixture.close().await;
}

#[rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn test_should_restore_failed_and_interrupted_creation_without_an_executable_pin(
    #[case] interrupted: bool,
) {
    let fixture = ImportFixture::new(DaemonMode::Normal, true).await;
    let mut value = named("failed");
    value.latest.deployment = if interrupted {
        DeploymentState::Creating(Uuid::new_v4().to_string().try_into().unwrap())
    } else {
        DeploymentState::CreateFailed(ImageDeploymentFailure::ImageNotFound)
    };
    fixture
        .provider
        .import_snapshot_before(snapshot(vec![value]), deadline())
        .await
        .unwrap();
    fixture.images.port().wait_empty(deadline()).await.unwrap();
    fixture.provider.validate_runtime_restore().unwrap();
    let restored = fixture.provider.store().get("failed").unwrap().unwrap();
    assert!(matches!(
        restored.latest.deployment,
        DeploymentState::CreateFailed(_)
    ));
    assert!(restored.latest.image.pinned().is_none());
    assert_eq!(fixture.fake.stats.tags.load(Ordering::Acquire), 1);
    fixture.close().await;
}

#[tokio::test]
async fn test_should_export_borrowed_records_with_the_existing_owned_snapshot_schema() {
    let fixture = ImportFixture::new(DaemonMode::Normal, true).await;
    let expected = fixture.state();
    let actual = fixture
        .provider
        .export_snapshot_before(deadline())
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(actual).unwrap(), expected);
    fixture.close().await;
}

#[rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn test_should_reject_deep_live_policy_and_oversized_ancillary_export_before_clone(
    #[case] deep: bool,
) {
    let fixture = ImportFixture::new(DaemonMode::Normal, true).await;
    if deep {
        let mut condition = Value::Null;
        for _ in 0..50_000 {
            condition = Value::Array(vec![condition]);
        }
        fixture
            .provider
            .store()
            .update("old", |record| {
                record
                    .policy
                    .statements
                    .push(crate::storage::PolicyStatement {
                        sid: "deep".into(),
                        effect: "Allow".into(),
                        principal: "*".into(),
                        action: "lambda:InvokeFunction".into(),
                        resource: "*".into(),
                        condition: Some(condition),
                    });
            })
            .unwrap();
    } else {
        let mut layer: LayerRecord = serde_json::from_value(json!({"name":"oversized","layerArn":"arn:aws:lambda:us-east-1:000000000000:layer:oversized","nextVersion":2,
            "versions":{"1":{"version":1,"description":"","compatibleRuntimes":[],"compatibleArchitectures":[],"licenseInfo":null,"codeSha256":"unused","codeSize":0,"createdDate":"2026-10-27T00:00:00Z","layerArn":"arn:aws:lambda:us-east-1:000000000000:layer:oversized","layerVersionArn":"arn:aws:lambda:us-east-1:000000000000:layer:oversized:1","policy":{"statements":[]}}}})).unwrap();
        layer.versions.get_mut(&1).unwrap().description = "x".repeat(20 * 1024 * 1024);
        fixture
            .provider
            .layer_store()
            .import_snapshot(LayerStoreSnapshot {
                layers: vec![layer],
            });
    }
    let error = fixture
        .provider
        .export_snapshot_before(deadline())
        .await
        .unwrap_err();
    assert!(matches!(error, LambdaServiceError::InvalidParameter { .. }));
    assert_eq!(fixture.fake.stats.tags.load(Ordering::Acquire), 1);
    assert_eq!(fixture.provider.store().len(), 1);
    fixture.close().await;
}
