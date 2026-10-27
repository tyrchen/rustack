//! One named session owns acquisition and never reissues accepted transactions.

use tokio::{fs, time::Duration};

use super::*;
use crate::snapshot::{MANIFEST_FILE, SnapshotName, read_manifest};

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}
#[tokio::test]
async fn test_should_retain_named_ownership_and_observe_the_same_publication() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let config = SnapshotConfig {
        root: directory.path().to_owned(),
        name: SnapshotName::try_from("session".to_owned())?,
    };
    let providers = RuntimeProviders::default();
    let mut session = SnapshotSession::new(Some(config.clone()));
    session.load(&providers, deadline()).await?;
    session.load(&providers, deadline()).await?;
    assert!(config.acquire_lease().await.is_err());
    session.save(&providers, "original", deadline()).await?;
    let original = fs::read(config.snapshot_dir().join(MANIFEST_FILE)).await?;
    session
        .save(&providers, "must-not-reissue", deadline())
        .await?;
    assert_eq!(
        fs::read(config.snapshot_dir().join(MANIFEST_FILE)).await?,
        original
    );
    assert_eq!(
        read_manifest(&config.snapshot_dir().join(MANIFEST_FILE))
            .await?
            .rustack_version,
        "original"
    );
    session.finish(deadline()).await?;
    drop(session);
    let _next = config.acquire_lease().await?;
    Ok(())
}
#[tokio::test]
async fn test_should_refuse_publication_after_failed_startup() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let config = SnapshotConfig {
        root: directory.path().to_owned(),
        name: SnapshotName::try_from("session".to_owned())?,
    };
    let providers = RuntimeProviders::default();
    config.save(&providers, "old").await?;
    let manifest = config.snapshot_dir().join(MANIFEST_FILE);
    fs::write(&manifest, b"corrupt generation").await?;
    let mut session = SnapshotSession::new(Some(config));
    assert!(session.load(&providers, deadline()).await.is_err());
    assert!(
        session
            .save(&providers, "invalid", deadline())
            .await
            .is_err()
    );
    assert_eq!(fs::read(&manifest).await?, b"corrupt generation");
    assert!(session.finish(deadline()).await.is_err());
    Ok(())
}
