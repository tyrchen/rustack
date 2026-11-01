//! Both inner deadline expiry and a dropped observer keep the exact acquisition.

use std::sync::atomic::{AtomicBool, Ordering};

use tokio::{sync::Notify, time::timeout_at};

use super::*;
use crate::snapshot::SnapshotName;

#[derive(Debug, Default)]
pub(super) struct Gate {
    entered: Notify,
    released: AtomicBool,
}
impl Gate {
    pub(super) fn enter(&self) {
        self.entered.notify_one();
        while !self.released.load(Ordering::Acquire) {
            std::thread::park_timeout(Duration::from_millis(1));
        }
    }
}
#[derive(Debug)]
struct Release(Arc<Gate>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.released.store(true, Ordering::Release);
    }
}
#[tokio::test]
async fn test_should_retain_delayed_lease_acquisition_after_timeout_or_observer_drop() -> Result<()>
{
    for inner_deadline in [false, true] {
        let directory = tempfile::tempdir()?;
        let config = SnapshotConfig {
            root: directory.path().to_owned(),
            name: SnapshotName::try_from("lease".to_owned())?,
        };
        let gate = Arc::new(Gate::default());
        let release = Release(gate.clone());
        let mut acquisition = config.begin_owned_lease(Some(gate.clone()));
        timeout_at(
            Instant::now() + Duration::from_secs(2),
            gate.entered.notified(),
        )
        .await?;
        let short = Instant::now() + Duration::from_millis(5);
        if inner_deadline {
            assert!(acquisition.wait(short).await.is_err());
        } else {
            assert!(
                timeout_at(
                    short,
                    acquisition.wait(Instant::now() + Duration::from_secs(2))
                )
                .await
                .is_err()
            );
        }
        assert!(
            acquisition
                .join
                .as_ref()
                .is_some_and(|join| !join.is_finished())
        );
        drop(release);
        let lease = acquisition
            .wait(Instant::now() + Duration::from_secs(2))
            .await?;
        assert!(config.acquire_lease().await.is_err());
        drop(lease);
        drop(acquisition);
        let _next = config.acquire_lease().await?;
    }
    Ok(())
}
