use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, atomic::AtomicBool},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use rstest::rstest;
use rustack_lambda_model::{
    input::{PublishVersionInput, UpdateFunctionCodeInput, UpdateFunctionConfigurationInput},
    types::ImageConfig,
};
use tokio::sync::watch;

use super::*;
use crate::{
    artifact::{ImageMetadata, ResolvedImage},
    config::LambdaConfig,
    provider::RustackLambda,
    storage::{FunctionStoreSnapshot, PolicyDocument},
};

fn image(
    reference: &str,
    letter: char,
    architecture: Architecture,
) -> (PinnedImage, watch::Receiver<()>, watch::Sender<bool>) {
    let mut metadata: ImageMetadata = ResolvedImage::fixture(architecture).into();
    metadata.requested_reference = reference.parse().unwrap();
    metadata.image_id = format!("sha256:{}", letter.to_string().repeat(64))
        .parse()
        .unwrap();
    metadata.inspection_hash = ImageMetadata::execution_fingerprint(
        &metadata.image_id,
        None,
        architecture,
        metadata.size_bytes,
        &metadata.execution,
    )
    .unwrap();
    let (release, released) = watch::channel(());
    let (stop, stopped) = watch::channel(false);
    let image = PinnedImage::new(
        metadata.try_into().unwrap(),
        4096.try_into().unwrap(),
        release,
        Arc::new(AtomicBool::new(true)),
        stopped,
    );
    (image, released, stop)
}
pub(super) fn record() -> FunctionRecord {
    FunctionRecord {
        resource_id: FunctionResourceId::new(),
        name: "worker".into(),
        arn: "arn:aws:lambda:us-east-1:000000000000:function:worker".into(),
        latest: VersionRecord {
            version: "$LATEST".into(),
            runtime: None,
            handler: None,
            role: "arn:aws:iam::000000000000:role/test".into(),
            description: "old description".into(),
            timeout: 3,
            memory_size: 128,
            environment: HashMap::from([("SECRET".into(), "private value".into())]),
            package_type: "Image".into(),
            code_path: None,
            image_uri: Some("worker:v1".into()),
            image: ImageArtifact::Unresolved,
            zip_bytes: None,
            deployment: DeploymentState::Ready,
            last_modified: "2026-10-25T00:00:00.000+0000".into(),
            architectures: vec!["arm64".into()],
            ephemeral_storage_size: 512,
            code_sha256: String::new(),
            code_size: 0,
            revision_id: Uuid::new_v4().to_string(),
            layers: Vec::new(),
            vpc_config: None,
            dead_letter_config: None,
            tracing_config: None,
            image_config: None,
            logging_config: None,
            snap_start: None,
        },
        versions: BTreeMap::new(),
        next_version: 1,
        aliases: HashMap::new(),
        policy: PolicyDocument::default(),
        tags: HashMap::new(),
        code_signing_config_arn: None,
        url_config: None,
        reserved_concurrent_executions: None,
        event_invoke_configs: HashMap::new(),
        created_at: "2026-10-25T00:00:00.000+0000".into(),
    }
}
fn store() -> (tempfile::TempDir, FunctionStore) {
    let root = tempfile::tempdir().unwrap();
    let store = FunctionStore::new(root.path());
    (root, store)
}
fn ready(store: &FunctionStore) -> FunctionRecord {
    let job = store.insert_image_deployment(record()).unwrap();
    store
        .settle_image_deployment(&job, Ok(image("worker:v1", 'a', Architecture::Arm64).0))
        .unwrap();
    store.get("worker").unwrap().unwrap()
}
fn update(store: &FunctionStore, reference: &str) -> ImageDeployment {
    let before = store.get("worker").unwrap().unwrap();
    store
        .begin_image_update(
            "worker",
            before.resource_id,
            &before.latest.revision_id,
            reference.parse().unwrap(),
            Architecture::Arm64,
        )
        .unwrap()
}
fn current(store: &FunctionStore) -> VersionRecord {
    store.get("worker").unwrap().unwrap().latest
}

#[test]
fn test_should_reserve_pending_and_settle_with_content_hash_and_unpacked_size() {
    let (_root, store) = store();
    let mut before = record();
    before.latest.code_sha256 = "unverified tag hash".into();
    before.latest.code_size = 999;
    let old_revision = before.latest.revision_id.clone();
    let resource = before.resource_id;
    let job = store.insert_image_deployment(before).unwrap();
    assert_eq!(current(&store).deployment.state(), "Pending");
    assert_eq!(current(&store).code_sha256, "");
    assert_eq!(current(&store).code_size, 0);
    assert_eq!(current(&store).deployment.update_status(), "InProgress");
    let (image, _, _) = image("worker:v1", 'a', Architecture::Arm64);
    let hash = image
        .resolved()
        .metadata()
        .inspection_hash
        .as_str()
        .strip_prefix("sha256:")
        .unwrap();
    let expected = STANDARD.encode(hex::decode(hash).unwrap());
    assert!(!format!("{job:?}").contains("private value"));
    assert!(!format!("{job:?}").contains("worker:v1"));
    assert!(store.settle_image_deployment(&job, Ok(image)).unwrap());
    let after = store.get("worker").unwrap().unwrap();
    assert_eq!(after.resource_id, resource);
    assert_eq!(after.latest.deployment, DeploymentState::Ready);
    assert_ne!(after.latest.revision_id, old_revision);
    assert_eq!(after.latest.code_sha256, expected);
    assert_eq!(after.latest.code_size, 4096); // Metadata Size is only1024.
    assert!(after.latest.image.pinned().unwrap().is_live());
    assert!(
        !store
            .settle_image_deployment(&job, Err(ImageDeploymentFailure::SupervisorFailed))
            .unwrap()
    );
}

#[test]
fn test_should_preserve_previous_revision_while_updating_and_after_failure() {
    let (_root, store) = store();
    let before = ready(&store).latest;
    let job = update(&store, "worker:v2");
    let pending = current(&store);
    assert_eq!(pending.deployment.state(), "Active");
    assert_eq!(pending.deployment.update_status(), "InProgress");
    assert_eq!(pending.revision_id, before.revision_id);
    assert_eq!(pending.image_uri, before.image_uri);
    assert_eq!(pending.code_sha256, before.code_sha256);
    assert_eq!(
        pending.image.resolved().unwrap().metadata().image_id,
        before.image.resolved().unwrap().metadata().image_id
    );
    assert!(
        store
            .settle_image_deployment(&job, Err(ImageDeploymentFailure::InvalidImage))
            .unwrap()
    );
    let failed = current(&store);
    assert_eq!(
        failed.deployment,
        DeploymentState::UpdateFailed(ImageDeploymentFailure::InvalidImage)
    );
    assert_eq!(failed.revision_id, before.revision_id);
    assert_eq!(failed.image_uri, before.image_uri);
    assert_eq!(failed.code_sha256, before.code_sha256);
    assert_eq!(failed.environment, before.environment);
    let retry = update(&store, "worker:v2");
    assert!(
        !store
            .settle_image_deployment(&job, Err(ImageDeploymentFailure::Cancelled))
            .unwrap()
    );
    assert!(
        store
            .settle_image_deployment(&retry, Ok(image("worker:v2", 'b', Architecture::Arm64).0))
            .unwrap()
    );
    assert_eq!(current(&store).image_uri.as_deref(), Some("worker:v2"));
    assert_ne!(current(&store).code_sha256, before.code_sha256);
}

#[test]
fn test_should_retry_failed_creation_without_claiming_a_previous_active_revision() {
    let (_root, store) = store();
    let initial = store.insert_image_deployment(record()).unwrap();
    assert!(
        store
            .settle_image_deployment(&initial, Err(ImageDeploymentFailure::ImageNotFound))
            .unwrap()
    );
    assert_eq!(current(&store).deployment.state(), "Failed");
    let retry = update(&store, "worker:fixed");
    assert_eq!(current(&store).deployment.state(), "Pending");
    assert_eq!(current(&store).image_uri.as_deref(), Some("worker:fixed"));
    assert!(
        store
            .settle_image_deployment(
                &retry,
                Ok(image("worker:fixed", 'b', Architecture::Arm64).0)
            )
            .unwrap()
    );
    assert_eq!(current(&store).deployment, DeploymentState::Ready);
}

#[test]
fn test_should_reject_stale_resource_revision_and_generation_independently() {
    let (_root, store) = store();
    let before = ready(&store);
    assert!(
        store
            .begin_image_update(
                "worker",
                FunctionResourceId::new(),
                &before.latest.revision_id,
                "worker:v2".parse().unwrap(),
                Architecture::Arm64
            )
            .is_err()
    );
    assert!(
        store
            .begin_image_update(
                "worker",
                before.resource_id,
                "stale",
                "worker:v2".parse().unwrap(),
                Architecture::Arm64
            )
            .is_err()
    );
    let job = update(&store, "worker:v2");
    assert!(
        store
            .begin_image_update(
                "worker",
                before.resource_id,
                &before.latest.revision_id,
                "worker:v3".parse().unwrap(),
                Architecture::Arm64
            )
            .is_err()
    );
    store
        .update("worker", |record| {
            record.latest.revision_id = Uuid::new_v4().to_string();
        })
        .unwrap();
    assert!(
        !store
            .settle_image_deployment(&job, Err(ImageDeploymentFailure::Cancelled))
            .unwrap()
    );
    store.remove("worker").unwrap();
    let mut recreated = before;
    recreated.resource_id = FunctionResourceId::new();
    recreated.latest.deployment = DeploymentState::Updating(job.generation);
    store.insert(recreated).unwrap();
    assert!(
        !store
            .settle_image_deployment(&job, Ok(image("worker:v2", 'b', Architecture::Arm64).0))
            .unwrap()
    );
    store.remove("worker").unwrap();
    assert!(
        !store
            .settle_image_deployment(&job, Err(ImageDeploymentFailure::Cancelled))
            .unwrap()
    );
}

#[rstest]
#[case("other:v1", Architecture::Arm64, false)]
#[case("worker:v1", Architecture::X86_64, false)]
#[case("worker:v1", Architecture::Arm64, true)]
fn test_should_reject_mismatching_or_revoked_preparation_without_partial_commit(
    #[case] reference: &str,
    #[case] architecture: Architecture,
    #[case] revoke: bool,
) {
    let (_root, store) = store();
    let job = store.insert_image_deployment(record()).unwrap();
    let pending = current(&store).deployment;
    let (image, _, stop) = image(reference, 'a', architecture);
    if revoke {
        stop.send_replace(true);
    }
    assert!(store.settle_image_deployment(&job, Ok(image)).is_err());
    assert_eq!(current(&store).deployment, pending);
    assert!(current(&store).image.pinned().is_none());
    assert!(
        store
            .settle_image_deployment(&job, Err(ImageDeploymentFailure::InvalidImage))
            .unwrap()
    );
}

#[test]
fn test_should_drop_stale_result_lease_without_modifying_a_new_generation() {
    let (_root, store) = store();
    ready(&store);
    let job = update(&store, "worker:v2");
    store
        .settle_image_deployment(&job, Err(ImageDeploymentFailure::Cancelled))
        .unwrap();
    let replacement = update(&store, "worker:v3");
    let (image, released, _) = image("worker:v2", 'b', Architecture::Arm64);
    assert!(!store.settle_image_deployment(&job, Ok(image)).unwrap());
    assert!(released.has_changed().is_err());
    assert_eq!(
        current(&store).deployment,
        DeploymentState::Updating(replacement.generation)
    );
}

#[test]
fn test_should_rollback_every_change_when_a_fallible_store_mutation_fails() {
    let (_root, store) = store();
    let before = ready(&store);
    let result: Result<(), _> =
        store.try_update_if_revision("worker", Some(&before.latest.revision_id), |record| {
            record.latest.environment.clear();
            record.latest.description = "must not commit".into();
            record.latest.revision_id = "changed".into();
            Err(invalid("injected validation failure"))
        });
    assert!(result.is_err());
    let after = current(&store);
    assert_eq!(after.environment, before.latest.environment);
    assert_eq!(after.description, before.latest.description);
    assert_eq!(after.revision_id, before.latest.revision_id);
}

#[tokio::test]
async fn test_should_derive_public_status_and_guard_mutations_and_publication() {
    let (root, store) = store();
    let job = store.insert_image_deployment(record()).unwrap();
    let provider = RustackLambda::with_store(store, LambdaConfig::default());
    let pending = provider.get_function_configuration("worker", None).unwrap();
    assert_eq!(pending.state.as_deref(), Some("Pending"));
    assert!(pending.code_sha256.is_none());
    assert_eq!(pending.last_update_status.as_deref(), Some("InProgress"));
    assert!(
        provider
            .publish_version("worker", &PublishVersionInput::default())
            .is_err()
    );
    assert!(
        provider
            .update_function_configuration(
                "worker",
                &UpdateFunctionConfigurationInput {
                    description: Some("no".into()),
                    ..Default::default()
                }
            )
            .is_err()
    );
    assert!(
        provider
            .update_function_code(
                "worker",
                UpdateFunctionCodeInput {
                    image_uri: Some("worker:v2".into()),
                    ..Default::default()
                }
            )
            .await
            .is_err()
    );
    assert!(
        provider
            .store()
            .settle_image_deployment(&job, Err(ImageDeploymentFailure::ImageNotFound))
            .unwrap()
    );
    let failed = provider.get_function_configuration("worker", None).unwrap();
    assert_eq!(failed.state.as_deref(), Some("Failed"));
    assert_eq!(
        failed.state_reason.as_deref(),
        Some(ImageDeploymentFailure::ImageNotFound.reason())
    );
    assert_eq!(failed.state_reason_code.as_deref(), Some("ImageDeleted"));
    assert_eq!(failed.last_update_status.as_deref(), Some("Failed"));
    assert!(
        provider
            .publish_version("worker", &PublishVersionInput::default())
            .is_err()
    );
    drop(provider);
    drop(root);
}

#[tokio::test]
async fn test_should_keep_published_leases_and_report_resolved_uri() {
    let (_root, store) = store();
    let job = store.insert_image_deployment(record()).unwrap();
    let (image, released, _) = image("worker:v1", 'a', Architecture::Arm64);
    store.settle_image_deployment(&job, Ok(image)).unwrap();
    let provider = RustackLambda::with_store(store, LambdaConfig::default());
    let published = provider
        .publish_version("worker", &PublishVersionInput::default())
        .unwrap();
    assert_eq!(published.version.as_deref(), Some("1"));
    let code = provider
        .get_function("worker", Some("1"))
        .unwrap()
        .code
        .unwrap();
    assert_eq!(code.image_uri.as_deref(), Some("worker:v1"));
    assert!(
        code.resolved_image_uri
            .unwrap()
            .starts_with("local://sha256:")
    );
    let update = update(provider.store(), "worker:v2");
    provider
        .store()
        .settle_image_deployment(&update, Ok(image_fixture("worker:v2", 'b')))
        .unwrap();
    assert!(!released.has_changed().unwrap());
    provider.delete_function("worker", Some("1")).await.unwrap();
    assert!(released.has_changed().is_err());
}
fn image_fixture(reference: &str, letter: char) -> PinnedImage {
    image(reference, letter, Architecture::Arm64).0
}

#[tokio::test]
async fn test_should_rollback_invalid_effective_configuration_and_guard_publish_keys() {
    let (_root, store) = store();
    let before = ready(&store).latest;
    let provider = RustackLambda::with_store(store, LambdaConfig::default());
    assert!(
        provider
            .update_function_configuration(
                "worker",
                &UpdateFunctionConfigurationInput {
                    description: Some("must not commit".into()),
                    image_config: Some(ImageConfig {
                        entry_point: Some(Vec::new()),
                        command: Some(Vec::new()),
                        working_directory: None
                    }),
                    ..Default::default()
                }
            )
            .is_err()
    );
    assert_eq!(current(provider.store()).description, before.description);
    assert_eq!(current(provider.store()).revision_id, before.revision_id);
    for input in [
        PublishVersionInput {
            revision_id: Some("stale".into()),
            ..Default::default()
        },
        PublishVersionInput {
            code_sha256: Some("wrong".into()),
            ..Default::default()
        },
    ] {
        assert!(provider.publish_version("worker", &input).is_err());
    }
    assert!(
        provider
            .store()
            .get("worker")
            .unwrap()
            .unwrap()
            .versions
            .is_empty()
    );
}

#[rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn test_should_restore_in_progress_snapshots_as_failed_without_inventing_a_lease(
    #[case] updating: bool,
) {
    let (_root, store) = store();
    if updating {
        ready(&store);
        update(&store, "worker:v2");
    } else {
        store.insert_image_deployment(record()).unwrap();
    }
    let json = serde_json::to_vec(&store.export_snapshot()).unwrap();
    let restore_root = tempfile::tempdir().unwrap();
    let restored = FunctionStore::new(restore_root.path());
    restored
        .import_snapshot(serde_json::from_slice(&json).unwrap())
        .await
        .unwrap();
    let after = current(&restored);
    assert_eq!(
        after.deployment,
        if updating {
            DeploymentState::UpdateFailed(ImageDeploymentFailure::Cancelled)
        } else {
            DeploymentState::CreateFailed(ImageDeploymentFailure::Cancelled)
        }
    );
    assert!(after.image.pinned().is_none());
    assert_eq!(after.image.resolved().is_some(), updating);
}

#[tokio::test]
async fn test_should_reject_contradictory_snapshot_states_and_accept_legacy_active_metadata() {
    let (_root, store) = store();
    ready(&store);
    let baseline = current(&store).revision_id;
    let mut snapshot = store.export_snapshot();
    snapshot.functions.first_mut().unwrap().latest.state = "Failed".into();
    assert!(store.import_snapshot(snapshot).await.is_err());
    assert_eq!(current(&store).revision_id, baseline);
    let mut legacy = serde_json::to_value(store.export_snapshot()).unwrap();
    legacy["functions"][0]["latest"]
        .as_object_mut()
        .unwrap()
        .remove("deployment");
    let restore_root = tempfile::tempdir().unwrap();
    let restored = FunctionStore::new(restore_root.path());
    restored
        .import_snapshot(serde_json::from_value::<FunctionStoreSnapshot>(legacy).unwrap())
        .await
        .unwrap();
    assert_eq!(current(&restored).deployment, DeploymentState::Ready);
    assert!(matches!(
        current(&restored).image,
        ImageArtifact::Restored(_)
    ));
}

#[test]
fn test_should_reject_nil_invalid_and_unknown_deployment_state_values() {
    assert!(DeploymentGeneration::try_from(Uuid::nil()).is_err());
    for value in [Uuid::nil().to_string(), "x".repeat(2048), "invalid".into()] {
        assert!(
            serde_json::from_value::<DeploymentGeneration>(serde_json::Value::String(value))
                .is_err()
        );
    }
    assert!(serde_json::from_str::<DeploymentState>(r#"{"kind":"ready","extra":true}"#).is_err());
    let state = DeploymentState::Creating(DeploymentGeneration(Uuid::new_v4()));
    assert_eq!(
        serde_json::from_slice::<DeploymentState>(&serde_json::to_vec(&state).unwrap()).unwrap(),
        state
    );
}

#[rstest]
#[case("hash")]
#[case("zero")]
#[case("oversize")]
#[tokio::test]
async fn test_should_reject_tampered_resolved_snapshot_metadata_atomically(
    #[case] corruption: &str,
) {
    let (_root, store) = store();
    let before = ready(&store).latest;
    let mut snapshot = store.export_snapshot();
    let version = &mut snapshot.functions.first_mut().unwrap().latest;
    match corruption {
        "hash" => version.code_sha256 = "forged".into(),
        "zero" => version.code_size = 0,
        "oversize" => version.code_size = crate::artifact::MAX_IMAGE_BYTES + 1,
        _ => unreachable!("trusted case"),
    }
    assert!(store.import_snapshot(snapshot).await.is_err());
    let after = current(&store);
    assert_eq!(after.revision_id, before.revision_id);
    assert_eq!(after.code_sha256, before.code_sha256);
    assert!(after.image.pinned().unwrap().is_live());
}

#[test]
fn test_should_reject_a_live_image_size_that_disagrees_with_its_proof() {
    let (_root, store) = store();
    let before = ready(&store).latest;
    assert!(
        store
            .update("worker", |record| record.latest.code_size += 1)
            .is_err()
    );
    assert_eq!(current(&store).code_size, before.code_size);
}

#[test]
fn test_should_commit_only_one_of_two_concurrent_completion_results() {
    let (_root, store) = store();
    ready(&store);
    let job = update(&store, "worker:v2");
    let barrier = std::sync::Barrier::new(2);
    let wins = std::thread::scope(|threads| {
        let handles: Vec<_> = ['b', 'c']
            .into_iter()
            .map(|letter| {
                let store = &store;
                let job = &job;
                let barrier = &barrier;
                threads.spawn(move || {
                    let prepared = image("worker:v2", letter, Architecture::Arm64).0;
                    barrier.wait();
                    store.settle_image_deployment(job, Ok(prepared)).unwrap()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .filter(|won| *won)
            .count()
    });
    assert_eq!(wins, 1);
    assert_eq!(current(&store).deployment, DeploymentState::Ready);
}

#[tokio::test]
async fn test_should_preserve_failed_readiness_when_restoring_legacy_zip_progress() {
    let (_root, store) = store();
    let mut source = record();
    source.latest.package_type = "Zip".into();
    source.latest.runtime = Some("provided.al2023".into());
    source.latest.handler = Some("bootstrap".into());
    source.latest.image_uri = None;
    store.insert(source).unwrap();
    let mut snapshot = store.export_snapshot();
    let version = &mut snapshot.functions.first_mut().unwrap().latest;
    version.state = "Pending".into();
    version.deployment = None;
    store.import_snapshot(snapshot).await.unwrap();
    assert_eq!(
        current(&store).deployment,
        DeploymentState::CreateFailed(ImageDeploymentFailure::Cancelled)
    );
}

#[tokio::test]
async fn test_should_reject_image_update_publication_before_mutating_the_previous_revision() {
    let (_root, store) = store();
    let before = ready(&store).latest;
    let provider = RustackLambda::with_store(store, LambdaConfig::default());
    let outcome = provider
        .update_function_code(
            "worker",
            UpdateFunctionCodeInput {
                image_uri: Some("worker:v2".into()),
                publish: Some(true),
                ..Default::default()
            },
        )
        .await;
    assert!(matches!(
        outcome,
        Err(LambdaServiceError::InvalidParameter { .. })
    ));
    let after = current(provider.store());
    assert_eq!(after.revision_id, before.revision_id);
    assert_eq!(after.code_sha256, before.code_sha256);
    assert_eq!(after.deployment, DeploymentState::Ready);
}
