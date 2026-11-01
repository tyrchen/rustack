//! A configured limit is not evidence of corruption of a committed generation.

use super::*;

async fn generation(root: &Path, directory: &str, version: &str, bytes: usize) -> Result<PathBuf> {
    let target = root.join(directory);
    let meta = target.join("services/limited/meta.ss.zst");
    let stats = write_archive(
        &meta,
        ArchiveKind::ServiceMeta,
        vec![ArchiveSection::new(
            SECTION_STATE_CBOR,
            to_cbor(&"x".repeat(bytes))?,
            1,
        )],
    )
    .await?;
    let mut manifest = SnapshotManifest::new("budget", version)?;
    manifest.add_service(
        "limited".into(),
        SnapshotServiceManifest {
            kind: SnapshotKind::Resource,
            meta_file: "services/limited/meta.ss.zst".into(),
            data_file: None,
            meta: stats,
            data: None,
        },
    );
    write_manifest(&target.join(MANIFEST_FILE), &manifest).await?;
    Ok(target)
}
#[tokio::test]
async fn test_should_refuse_reduced_policy_without_replacing_valid_current_or_previous_generation()
-> Result<()> {
    let directory = tempfile::tempdir()?;
    let current = generation(directory.path(), "budget", "new", 4096).await?;
    let previous = generation(directory.path(), ".budget.previous", "old", 1).await?;
    let new_bytes = fs::read(current.join(MANIFEST_FILE)).await?;
    let old_bytes = fs::read(previous.join(MANIFEST_FILE)).await?;
    let error = recover_snapshot(
        directory.path(),
        &current,
        "budget",
        &BTreeMap::from([("limited", 4098)]),
    )
    .await
    .err()
    .context("accepted reduced budget")?;
    assert!(error.is::<BudgetLimit>());
    assert_eq!(fs::read(current.join(MANIFEST_FILE)).await?, new_bytes);
    assert_eq!(fs::read(previous.join(MANIFEST_FILE)).await?, old_bytes);
    recover_snapshot(
        directory.path(),
        &current,
        "budget",
        &BTreeMap::from([("limited", 8192)]),
    )
    .await?;
    assert_eq!(
        read_manifest(&current.join(MANIFEST_FILE))
            .await?
            .rustack_version,
        "new"
    );
    assert!(fs::try_exists(&previous).await?);
    SnapshotConfig {
        root: directory.path().to_owned(),
        name: SnapshotName::try_from("budget".to_owned())?,
    }
    .load(&RuntimeProviders::default())
    .await?;
    assert!(!fs::try_exists(previous).await?);
    Ok(())
}
