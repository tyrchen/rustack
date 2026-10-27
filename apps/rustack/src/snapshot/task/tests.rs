//! Retained transaction observation and complete sibling retirement on error.

use std::path::Path;

use async_trait::async_trait;
use tokio::{
    fs,
    sync::{Notify, watch},
    time::{Duration, advance, timeout},
};

use super::*;
use crate::snapshot::{MANIFEST_FILE, SnapshotName, SnapshotService, to_cbor};

#[derive(Debug)]
struct GateService {
    name: &'static str,
    fail: bool,
    entered: Arc<Notify>,
    release: watch::Receiver<bool>,
}
impl GateService {
    async fn hold(&self) -> Result<()> {
        self.entered.notify_one();
        let mut release = self.release.clone();
        release
            .wait_for(|ready| *ready)
            .await
            .context("test gate lost")?;
        Ok(())
    }
}
#[async_trait]
impl SnapshotService for GateService {
    fn service_name(&self) -> &'static str {
        self.name
    }
    async fn save_meta(&self, _directory: &Path) -> Result<Vec<u8>> {
        if self.fail {
            anyhow::bail!("controlled service failure");
        }
        self.hold().await?;
        to_cbor(&())
    }
    async fn load_meta(&self, _metadata: Vec<u8>, directory: &Path) -> Result<()> {
        if self.fail {
            anyhow::bail!("controlled service failure");
        }
        self.hold().await?;
        anyhow::ensure!(
            fs::metadata(directory).await?.is_dir(),
            "staging removed before sibling retirement"
        );
        Ok(())
    }
}
fn config(directory: &tempfile::TempDir) -> Result<SnapshotConfig> {
    Ok(SnapshotConfig {
        root: directory.path().to_owned(),
        name: SnapshotName::try_from("transaction".to_owned())?,
    })
}
#[tokio::test]
async fn test_should_keep_transaction_and_named_lease_after_observer_expiry() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let config = config(&directory)?;
    let entered = Arc::new(Notify::new());
    let (release, receiver) = watch::channel(false);
    let providers = RuntimeProviders {
        services: vec![Arc::new(GateService {
            name: "owned",
            fail: false,
            entered: entered.clone(),
            release: receiver,
        })],
    };
    let lease = config.acquire_lease().await?;
    let mut task = SnapshotTask::start(config.clone(), providers, lease, Some("test"));
    timeout(Duration::from_secs(2), entered.notified()).await?;
    assert!(
        task.wait(Instant::now() + Duration::from_millis(5))
            .await
            .is_err()
    );
    assert!(task.join.as_ref().is_some_and(|join| !join.is_finished()));
    assert!(config.acquire_lease().await.is_err());
    release.send_replace(true);
    task.wait(Instant::now() + Duration::from_secs(5)).await?;
    let _new = config.acquire_lease().await?;
    Ok(())
}
#[tokio::test(start_paused = true)]
async fn test_should_cache_ready_completion_but_reject_first_late_observation() -> Result<()> {
    let entered = Arc::new(Notify::new());
    let signal = entered.clone();
    let mut task = SnapshotTask {
        join: Some(tokio::spawn(async move {
            signal.notify_one();
            Ok(())
        })),
        failure: None,
    };
    let deadline = Instant::now() + Duration::from_secs(1);
    entered.notified().await;
    assert!(task.join.as_ref().is_some_and(JoinHandle::is_finished));
    advance(Duration::from_secs(2)).await;
    assert!(task.wait(deadline).await.is_err());
    assert!(task.join.is_none());
    task.wait(Instant::now() + Duration::from_secs(1)).await?;
    Ok(())
}
#[tokio::test]
async fn test_should_drain_started_load_and_save_siblings_before_returning_first_error()
-> Result<()> {
    for saving in [false, true] {
        let directory = tempfile::tempdir()?;
        let config = config(&directory)?;
        let (ready, receiver) = watch::channel(true);
        let original = RuntimeProviders {
            services: ["broken", "slow"]
                .into_iter()
                .map(|name| {
                    Arc::new(GateService {
                        name,
                        fail: false,
                        entered: Arc::new(Notify::new()),
                        release: receiver.clone(),
                    }) as Arc<dyn SnapshotService>
                })
                .collect(),
        };
        config.save(&original, "old").await?;
        let original_manifest = fs::read(config.snapshot_dir().join(MANIFEST_FILE)).await?;
        let entered = Arc::new(Notify::new());
        let (release, receiver) = watch::channel(false);
        let replacement = RuntimeProviders {
            services: vec![
                Arc::new(GateService {
                    name: "broken",
                    fail: true,
                    entered: Arc::new(Notify::new()),
                    release: receiver.clone(),
                }),
                Arc::new(GateService {
                    name: "slow",
                    fail: false,
                    entered: entered.clone(),
                    release: receiver,
                }),
            ],
        };
        let requested = config.clone();
        let mut task = tokio::spawn(async move {
            if saving {
                requested.save(&replacement, "invalid").await
            } else {
                requested.load(&replacement).await
            }
        });
        timeout(Duration::from_secs(2), entered.notified()).await?;
        assert!(timeout(Duration::from_millis(5), &mut task).await.is_err());
        assert!(!task.is_finished(), "returned before sibling retirement");
        release.send_replace(true);
        assert!(timeout(Duration::from_secs(5), task).await??.is_err());
        assert_eq!(
            fs::read(config.snapshot_dir().join(MANIFEST_FILE)).await?,
            original_manifest
        );
        drop(ready);
    }
    Ok(())
}
