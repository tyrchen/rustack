//! Bounded private ZIP staging under one original import deadline.

use std::io::Cursor;

use base64::{Engine, engine::general_purpose::STANDARD};
use bytes::Bytes;
use tokio::{
    task::spawn_blocking,
    time::{Instant, timeout_at},
};
use zip::ZipArchive;

use super::{
    FunctionStore, MAX_EXTRACTED_SIZE, StagedArtifact, ZipLayout, stage_zip_before,
    validate_zip_directory, validated_zip_path,
};
use crate::{
    artifact::{MAX_ZIP_BASE64_BYTES, MAX_ZIP_BYTES},
    error::LambdaServiceError,
    provider::DockerArchiveProfile,
};

const EXPANDED: u64 = 512 * 1024 * 1024;
const NODES: usize = 20_000;
const SOURCES: usize = DockerArchiveProfile::MAX_ZIP_DATA_BYTES;

fn invalid() -> LambdaServiceError {
    LambdaServiceError::InvalidParameter {
        message: "Invalid or oversized Docker ZIP archive candidate".into(),
    }
}
pub(super) fn check(deadline: Option<Instant>) -> Result<(), LambdaServiceError> {
    if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        return Err(LambdaServiceError::ResourceNotReady {
            message: "ZIP archive import deadline exceeded".into(),
        });
    }
    Ok(())
}
fn stage_error(error: LambdaServiceError) -> LambdaServiceError {
    match error {
        LambdaServiceError::InvalidParameter { .. } | LambdaServiceError::InvalidZipFile { .. } => {
            invalid()
        }
        LambdaServiceError::Internal { .. } => LambdaServiceError::Internal {
            message: "ZIP archive staging failed".into(),
        },
        other => other,
    }
}
#[derive(Debug, Clone, Copy)]
pub(super) struct Measurement {
    expanded: u64,
    nodes: usize,
}
fn measure(bytes: &Bytes) -> Result<Measurement, LambdaServiceError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|_| invalid())?;
    if archive.len() > 10_000 || archive.has_duplicate_names() {
        return Err(invalid());
    }
    let mut layout = ZipLayout::default();
    let mut expanded = 0_u64;
    for n in 0..archive.len() {
        let mut entry = archive.by_index(n).map_err(|_| invalid())?;
        let path = validated_zip_path(&entry).map_err(stage_error)?;
        layout.admit(&path, entry.is_dir()).map_err(stage_error)?;
        if entry.is_dir() {
            validate_zip_directory(&mut entry).map_err(stage_error)?;
        } else {
            expanded = expanded
                .checked_add(entry.size())
                .filter(|n| *n <= MAX_EXTRACTED_SIZE)
                .ok_or_else(invalid)?;
        }
    }
    let nodes = layout.node_count().checked_add(3).ok_or_else(invalid)?;
    Ok(Measurement { expanded, nodes })
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ZipMeasurements {
    expanded: u64,
    nodes: usize,
    sources: usize,
}
impl Default for ZipMeasurements {
    fn default() -> Self {
        Self {
            expanded: EXPANDED,
            nodes: NODES,
            sources: SOURCES,
        }
    }
}
impl ZipMeasurements {
    pub(super) fn charge(&mut self, bytes: &Bytes) -> Result<Measurement, LambdaServiceError> {
        if bytes.len() > MAX_ZIP_BYTES {
            return Err(invalid());
        }
        let sources = self.sources.checked_sub(bytes.len()).ok_or_else(invalid)?;
        let measured = measure(bytes)?;
        let expanded = self
            .expanded
            .checked_sub(measured.expanded)
            .ok_or_else(invalid)?;
        let nodes = self.nodes.checked_sub(measured.nodes).ok_or_else(invalid)?;
        self.sources = sources;
        self.expanded = expanded;
        self.nodes = nodes;
        Ok(measured)
    }
}

#[cfg(test)]
#[derive(Debug)]
struct StageGate {
    entered: tokio::sync::oneshot::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}

#[derive(Debug)]
pub(super) struct ZipRestoreBudget {
    deadline: Instant,
    limits: ZipMeasurements,
    #[cfg(test)]
    gate: Option<StageGate>,
}
impl ZipRestoreBudget {
    #[cfg(unix)]
    pub(super) fn new(deadline: Instant) -> Self {
        Self {
            deadline,
            limits: ZipMeasurements::default(),
            #[cfg(test)]
            gate: None,
        }
    }
    pub(super) fn check(&self) -> Result<(), LambdaServiceError> {
        check(Some(self.deadline))
    }
    pub(super) async fn stage<'a>(
        &mut self,
        store: &'a FunctionStore,
        encoded: String,
    ) -> Result<StagedArtifact<'a>, LambdaServiceError> {
        self.check()?;
        if encoded.len() > MAX_ZIP_BASE64_BYTES {
            return Err(invalid());
        }
        timeout_at(self.deadline, store.validate_root())
            .await
            .map_err(|_| LambdaServiceError::ResourceNotReady {
                message: "ZIP archive import deadline exceeded".into(),
            })??;
        self.check()?;
        let permit = store.staging.clone().try_acquire_owned().map_err(|_| {
            LambdaServiceError::ResourceNotReady {
                message: "ZIP staging capacity exhausted".into(),
            }
        })?;
        let root = store.code_dir.clone();
        let deadline = self.deadline;
        let mut limits = self.limits;
        #[cfg(test)]
        let gate = self.gate.take();
        let mut job = spawn_blocking(move || {
            let retained = (encoded, permit);
            check(Some(deadline))?;
            let bytes = STANDARD
                .decode(retained.0.as_bytes())
                .map(Bytes::from)
                .map_err(|_| invalid())?;
            let measured = limits.charge(&bytes)?;
            check(Some(deadline))?;
            let artifact = stage_zip_before(&root, bytes, measured.expanded, Some(deadline))
                .map_err(stage_error)?;
            #[cfg(test)]
            if let Some(gate) = gate {
                let _ = gate.entered.send(());
                gate.release
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .map_err(|_| invalid())?;
            }
            check(Some(deadline))?;
            Ok::<_, LambdaServiceError>((artifact, limits))
        });
        let Ok(outcome) = timeout_at(deadline, &mut job).await else {
            // Retain the original transaction until this same extraction joins.
            drop(job.await);
            return Err(LambdaServiceError::ResourceNotReady {
                message: "ZIP archive import deadline exceeded".into(),
            });
        };
        let (artifact, limits) =
            outcome.map_err(|source| LambdaServiceError::ImageRestoreWorker {
                source: source.into(),
            })??;
        // No detached worker has a store capability. Fence before registration.
        self.check()?;
        self.limits = limits;
        store
            .artifacts
            .insert(artifact.code_root().to_path_buf(), artifact.clone());
        Ok(StagedArtifact::new(store, artifact))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::{
        io::{Cursor, Write},
        path::Path,
        sync::{Arc, mpsc},
        time::Duration,
    };

    use rstest::rstest;
    use tokio::{
        fs,
        sync::oneshot,
        time::{sleep, sleep_until, timeout},
    };
    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    use super::*;

    fn source() -> String {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file(
                "tree/file",
                SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
            )
            .unwrap();
        writer.write_all(&[42_u8; 64]).unwrap();
        STANDARD.encode(writer.finish().unwrap().into_inner())
    }
    async fn entries(path: &Path) -> usize {
        let mut entries = fs::read_dir(path).await.unwrap();
        let mut count = 0;
        while entries.next_entry().await.unwrap().is_some() {
            count += 1;
        }
        count
    }
    fn held(deadline: Instant) -> (ZipRestoreBudget, oneshot::Receiver<()>, mpsc::Sender<()>) {
        let (entered, observed) = oneshot::channel();
        let (release, receiver) = mpsc::channel();
        let mut budget = ZipRestoreBudget::new(deadline);
        budget.gate = Some(StageGate {
            entered,
            release: receiver,
        });
        (budget, observed, release)
    }
    #[rstest]
    #[case::exact(64, 5, true)]
    #[case::expanded(63, 5, false)]
    #[case::nodes(64, 4, false)]
    #[tokio::test]
    async fn test_should_charge_expansion_and_wrapper_nodes_before_artifact_allocation(
        #[case] expanded: u64,
        #[case] nodes: usize,
        #[case] accepted: bool,
    ) {
        let root = tempfile::tempdir().unwrap();
        let store = FunctionStore::new(root.path());
        let mut budget = ZipRestoreBudget::new(Instant::now() + Duration::from_secs(5));
        budget.limits.expanded = expanded;
        budget.limits.nodes = nodes;
        let stage = budget.stage(&store, source()).await;
        assert_eq!(stage.is_ok(), accepted);
        assert_eq!(entries(root.path()).await, usize::from(accepted));
        if accepted {
            assert_eq!(budget.limits.expanded, 0);
            assert_eq!(budget.limits.nodes, 0);
            assert!(budget.stage(&store, source()).await.is_err());
            assert_eq!(entries(root.path()).await, 1);
        }
        drop(stage);
        assert!(store.artifacts.is_empty());
        assert_eq!(entries(root.path()).await, 0);
    }
    #[tokio::test]
    async fn test_should_charge_repeated_compressed_sources_and_preserve_first_pending_artifact() {
        let root = tempfile::tempdir().unwrap();
        let store = FunctionStore::new(root.path());
        let encoded = source();
        let bytes = STANDARD.decode(&encoded).unwrap().len();
        let mut budget = ZipRestoreBudget::new(Instant::now() + Duration::from_secs(5));
        budget.limits.sources = bytes;
        let first = budget.stage(&store, encoded.clone()).await.unwrap();
        assert!(budget.stage(&store, encoded).await.is_err());
        assert_eq!(entries(root.path()).await, 1);
        assert!(first.artifact().code_root().is_absolute());
        drop(first);
        assert_eq!(entries(root.path()).await, 0);
    }
    #[tokio::test]
    async fn test_should_join_accepted_private_extraction_past_original_deadline_before_returning_error()
     {
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(FunctionStore::new(root.path()));
        let deadline = Instant::now() + Duration::from_secs(2);
        let (mut budget, entered, release) = held(deadline);
        let worker_store = store.clone();
        let job =
            tokio::spawn(async move { budget.stage(&worker_store, source()).await.map(drop) });
        timeout(Duration::from_secs(3), entered)
            .await
            .unwrap()
            .unwrap();
        sleep_until(deadline + Duration::from_millis(10)).await;
        assert!(!job.is_finished());
        assert!(store.artifacts.is_empty());
        assert_eq!(store.staging.available_permits(), 3);
        assert_eq!(entries(root.path()).await, 1);
        release.send(()).unwrap();
        assert!(matches!(
            timeout(Duration::from_secs(3), job).await.unwrap().unwrap(),
            Err(LambdaServiceError::ResourceNotReady { .. })
        ));
        assert!(store.artifacts.is_empty());
        assert_eq!(store.staging.available_permits(), 4);
        assert_eq!(entries(root.path()).await, 0);
    }
    #[tokio::test]
    async fn test_should_retain_abandoned_worker_cleanup_without_late_store_registration() {
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(FunctionStore::new(root.path()));
        let (mut budget, entered, release) = held(Instant::now() + Duration::from_secs(10));
        let worker_store = store.clone();
        let job =
            tokio::spawn(async move { budget.stage(&worker_store, source()).await.map(drop) });
        timeout(Duration::from_secs(3), entered)
            .await
            .unwrap()
            .unwrap();
        job.abort();
        assert!(job.await.unwrap_err().is_cancelled());
        assert_eq!(store.staging.available_permits(), 3);
        assert!(store.artifacts.is_empty());
        release.send(()).unwrap();
        timeout(Duration::from_secs(3), async {
            loop {
                if entries(root.path()).await == 0 && store.staging.available_permits() == 4 {
                    break;
                }
                sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert!(store.artifacts.is_empty());
    }
    #[tokio::test]
    async fn test_should_probe_size_overflow_without_writing_beyond_the_charged_bytes() {
        let root = tempfile::tempdir().unwrap();
        let mut bytes = STANDARD.decode(source()).unwrap();
        let central = bytes
            .windows(4)
            .position(|bytes| bytes == b"PK\x01\x02")
            .unwrap();
        bytes
            .get_mut(central + 24..central + 28)
            .unwrap()
            .copy_from_slice(&2_u32.to_le_bytes());
        let path = root.path().to_path_buf();
        let result = spawn_blocking(move || super::super::extract_zip(&bytes, &path, 2, None))
            .await
            .unwrap();
        assert!(result.is_err());
        assert_eq!(
            fs::metadata(root.path().join("tree/file"))
                .await
                .unwrap()
                .len(),
            2
        );
    }
}
