//! Immutable-content restoration must never become ordinary tag preparation.

use super::*;
use crate::artifact::{ImageExecutionMetadata, ImageMetadata};

#[tokio::test]
async fn test_should_restore_saved_metadata_without_following_a_retargeted_source() {
    let fake = FakePins::new(Mode::Normal).await;
    let mut owner = fake.owner(2, 1);
    let port = owner.port();
    let original = port
        .prepare(
            &"worker:v1".parse().unwrap(),
            Architecture::Arm64,
            None,
            deadline(),
        )
        .await
        .unwrap();
    let serialized = serde_json::to_vec(original.resolved()).unwrap();
    let saved: ResolvedImage = serde_json::from_slice(&serialized).unwrap();
    fake.commands.send(Command::Retarget).await.unwrap();
    let restored = port.restore(&saved, None, deadline()).await.unwrap();
    drop(original);
    assert_eq!(serde_json::to_vec(restored.resolved()).unwrap(), serialized);
    assert_eq!(
        restored.resolved().metadata().image_id.as_str(),
        digest('a')
    );
    assert_eq!(
        restored.resolved().code_sha256().unwrap(),
        saved.code_sha256().unwrap()
    );
    assert_eq!(restored.unpacked_size().get(), 4096);
    assert!(restored.is_live());
    assert_eq!(fake.stats.source_lookups.load(Ordering::Acquire), 1);
    assert_eq!(fake.stats.tags.load(Ordering::Acquire), 2);
    assert!(fake.stats.immutable_lookups.load(Ordering::Acquire) > 0);
    drop(restored);
    port.wait_empty(deadline()).await.unwrap();
    assert_eq!(fake.stats.deletes.load(Ordering::Acquire), 2);
    owner.shutdown(deadline()).await.unwrap();
    fake.close().await;
}

#[tokio::test]
async fn test_should_reject_missing_saved_content_without_source_fallback_or_mutation() {
    let fake = FakePins::new(Mode::Normal).await;
    let saved = fake.saved().await;
    let mut owner = fake.owner(1, 1);
    let port = owner.port();
    fake.commands.send(Command::Retarget).await.unwrap();
    assert!(matches!(
        port.restore(&saved, None, deadline()).await,
        Err(DockerError::Status(StatusCode::NOT_FOUND))
    ));
    port.wait_empty(deadline()).await.unwrap();
    assert_eq!(fake.stats.source_lookups.load(Ordering::Acquire), 1);
    assert_eq!(fake.stats.immutable_lookups.load(Ordering::Acquire), 1);
    assert_eq!(fake.stats.tags.load(Ordering::Acquire), 0);
    assert_eq!(fake.stats.deletes.load(Ordering::Acquire), 0);
    // A newer source is positively available, but cannot replace saved A.
    let newer = fake.saved().await;
    assert_eq!(newer.metadata().image_id.as_str(), digest('b'));
    owner.shutdown(deadline()).await.unwrap();
    fake.close().await;
}

#[rstest::rstest]
#[case::size(0)]
#[case::defaults(1)]
#[case::manifest(2)]
#[case::architecture(3)]
#[tokio::test]
async fn test_should_compare_valid_saved_fingerprints_with_actual_metadata_before_tag(
    #[case] field: u8,
) {
    let fake = FakePins::new(Mode::Normal).await;
    let saved = fake.saved().await;
    let mut changed: ImageMetadata = saved.clone().into();
    match field {
        0 => changed.size_bytes = 65,
        1 => {
            changed.execution = ImageExecutionMetadata::builder()
                .entrypoint(vec!["/entry".into()])
                .command(vec!["old_handler".into()])
                .working_directory("/task".into())
                .environment(vec!["SECRET=private_value".into()])
                .user("1001".into())
                .build()
                .try_into()
                .unwrap();
        }
        2 => changed.selected_manifest = Some(digest('e').parse().unwrap()),
        3 => changed.architecture = Architecture::X86_64,
        _ => unreachable!("fixed case table"),
    }
    changed.inspection_hash = ImageMetadata::execution_fingerprint(
        &changed.image_id,
        changed.selected_manifest.as_ref(),
        changed.architecture,
        changed.size_bytes,
        &changed.execution,
    )
    .unwrap();
    let changed: ResolvedImage = changed.try_into().unwrap();
    let mut owner = fake.owner(1, 1);
    let port = owner.port();
    assert!(matches!(
        port.restore(&changed, None, deadline()).await,
        Err(DockerError::Image(ArtifactError::Inspection))
    ));
    port.wait_empty(deadline()).await.unwrap();
    assert_eq!(fake.stats.tags.load(Ordering::Acquire), 0);
    assert_eq!(fake.stats.source_lookups.load(Ordering::Acquire), 1);
    assert_eq!(fake.stats.immutable_lookups.load(Ordering::Acquire), 1);
    let image = port.restore(&saved, None, deadline()).await.unwrap();
    assert!(image.is_live());
    drop(image);
    port.wait_empty(deadline()).await.unwrap();
    owner.shutdown(deadline()).await.unwrap();
    fake.close().await;
}

#[tokio::test]
async fn test_should_reject_a_different_root_id_before_platform_lookup_or_tag() {
    let fake = FakePins::new(Mode::WrongRoot).await;
    let saved = fake.saved().await;
    let mut owner = fake.owner(1, 1);
    let port = owner.port();
    assert!(matches!(
        port.restore(&saved, None, deadline()).await,
        Err(DockerError::Image(ArtifactError::Inspection))
    ));
    port.wait_empty(deadline()).await.unwrap();
    assert_eq!(fake.stats.source_lookups.load(Ordering::Acquire), 1);
    assert_eq!(fake.stats.immutable_lookups.load(Ordering::Acquire), 1);
    assert_eq!(fake.stats.platform_lookups.load(Ordering::Acquire), 1);
    assert_eq!(fake.stats.tags.load(Ordering::Acquire), 0);
    owner.shutdown(deadline()).await.unwrap();
    fake.close().await;
}
