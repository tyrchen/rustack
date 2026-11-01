//! Actual service archives, metadata/history joins and atomic startup binding.

use std::sync::atomic::{AtomicBool, Ordering};

use http::StatusCode;
use rustack_core::settings::RuntimeBudgets;
use rustack_stepfunctions_core::RegistryMetadata;
use serde_json::json;
use tempfile::TempDir;
use tokio::{
    fs,
    sync::{Notify, watch},
};

use super::*;
use crate::{
    snapshot::{
        MANIFEST_FILE, RuntimeProviders, SnapshotConfig, SnapshotName,
        archive::{ArchiveKind, from_cbor, read_archive},
    },
    workflows::tests::{request, runtime, start},
};

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
fn config(root: &TempDir) -> Result<SnapshotConfig> {
    Ok(SnapshotConfig {
        name: SnapshotName::try_from("workflows".to_owned())?,
        root: root.path().to_owned(),
    })
}
fn providers(client: WorkflowArchive) -> RuntimeProviders {
    let mut providers = RuntimeProviders::default();
    providers.register_workflows(client);
    providers
}
#[tokio::test]
async fn test_should_use_real_meta_data_generation_and_restore_exact_old_execution() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let config = config(&directory)?;
    let mut source = runtime(RuntimeBudgets::default())?;
    let mut target = runtime(RuntimeBudgets::default())?;
    let result = async {
        let captured = start(&source, 0).await?;
        source.owner.quiesce(deadline()).await?;
        assert!(source.archive.is_ready(), "quiesce closed the save lane");
        let expected = serde_json::to_value(captured.handle().snapshot().history().page(1, 1000, true)?.events())?;
        config.save(&providers(source.archive.clone()), "test").await?;
        let service = config.snapshot_dir().join("services/stepfunctions");
        assert!(fs::metadata(service.join("meta.ss.zst")).await?.is_file());
        assert!(fs::metadata(service.join("data.ss.zst")).await?.is_file());
        assert!(fs::metadata(config.snapshot_dir().join(MANIFEST_FILE)).await?.is_file());
        let parts = read_archive(&service.join("meta.ss.zst"), ArchiveKind::ServiceMeta).await?;
        let metadata: RegistryMetadata = from_cbor(&parts.first().context("missing metadata section")?.bytes)?;
        assert_eq!(serde_json::to_value(metadata)?.pointer("/executions/0/history"), Some(&json!([])));
        config.load(&providers(target.archive.clone())).await?;
        let (status, saved) = request(target.api.clone(), "DescribeExecution", json!({"executionArn":captured.arn().as_str()})).await?;
        assert_eq!(status, StatusCode::OK, "{saved}");
        assert_eq!(saved.get("status"), Some(&json!("SUCCEEDED")));
        assert_eq!(saved.get("input"), Some(&json!("{\"answer\":42}")));
        let (status, history) = request(target.api.clone(), "GetExecutionHistory", json!({"executionArn":captured.arn().as_str()})).await?;
        assert_eq!(status, StatusCode::OK, "{history}");
        assert_eq!(history.get("events"), Some(&expected));
        let (status, duplicate) = request(target.api.clone(), "StartExecution", json!({"stateMachineArn":captured.arn().machine().as_str(),"name":"Run","input":"{\"answer\":42}"})).await?;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{duplicate}");
        let (status, next) = request(target.api.clone(), "StartExecution", json!({"stateMachineArn":captured.arn().machine().as_str(),"name":"After","input":"{}"})).await?;
        assert_eq!(status, StatusCode::OK, "{next}");
        assert_eq!(target.status.capabilities()["snapshot"], "resources-and-history");
        assert_eq!(providers(target.archive.clone()).coverage(), vec![("stepfunctions", "resources-and-history")]);
        Ok(())
    }.await;
    let one = source.owner.shutdown(deadline()).await;
    let two = target.owner.shutdown(deadline()).await;
    one?;
    two?;
    result
}
#[tokio::test]
async fn test_should_save_empty_registry_with_required_history_data() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let config = config(&directory)?;
    let mut source = runtime(RuntimeBudgets::default())?;
    let mut target = runtime(RuntimeBudgets::default())?;
    let result = async {
        source.owner.quiesce(deadline()).await?;
        config
            .save(&providers(source.archive.clone()), "test")
            .await?;
        config.load(&providers(target.archive.clone())).await?;
        let (status, rows) = request(target.api.clone(), "ListStateMachines", json!({})).await?;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(rows.get("stateMachines"), Some(&json!([])));
        Ok(())
    }
    .await;
    let one = source.owner.shutdown(deadline()).await;
    let two = target.owner.shutdown(deadline()).await;
    one?;
    two?;
    result
}
#[tokio::test]
async fn test_should_preserve_old_generation_when_export_is_not_sealed() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let config = config(&directory)?;
    let mut source = runtime(RuntimeBudgets::default())?;
    let mut live = runtime(RuntimeBudgets::default())?;
    let result = async {
        start(&source, 0).await?;
        source.owner.quiesce(deadline()).await?;
        config
            .save(&providers(source.archive.clone()), "old")
            .await?;
        let before = fs::read(config.snapshot_dir().join(MANIFEST_FILE)).await?;
        assert!(
            config
                .save(&providers(live.archive.clone()), "unproven")
                .await
                .is_err()
        );
        assert_eq!(
            fs::read(config.snapshot_dir().join(MANIFEST_FILE)).await?,
            before
        );
        Ok(())
    }
    .await;
    let one = source.owner.shutdown(deadline()).await;
    let two = live.owner.shutdown(deadline()).await;
    one?;
    two?;
    result
}

mod rejection;

#[derive(Debug)]
pub(super) struct Hooks {
    cpu: bool,
    cpu_entered: Notify,
    cpu_released: AtomicBool,
    io: bool,
    io_entered: Notify,
    io_released: watch::Sender<bool>,
}
impl Hooks {
    fn new(cpu: bool, io: bool) -> Arc<Self> {
        Arc::new(Self {
            cpu,
            cpu_entered: Notify::new(),
            cpu_released: AtomicBool::new(false),
            io,
            io_entered: Notify::new(),
            io_released: watch::channel(false).0,
        })
    }
    pub(super) fn cpu_wait(&self) {
        if self.cpu {
            self.cpu_entered.notify_one();
            while !self.cpu_released.load(Ordering::Acquire) {
                std::thread::park_timeout(Duration::from_millis(1));
            }
        }
    }
    pub(super) async fn io_wait(&self) -> Result<()> {
        if self.io {
            self.io_entered.notify_one();
            let mut release = self.io_released.subscribe();
            release
                .wait_for(|ready| *ready)
                .await
                .context("owned IO test gate lost")?;
        }
        Ok(())
    }
    fn release(&self) {
        self.cpu_released.store(true, Ordering::Release);
        self.io_released.send_replace(true);
    }
}
#[derive(Debug)]
struct Release(Arc<Hooks>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.release();
    }
}
mod ownership;
