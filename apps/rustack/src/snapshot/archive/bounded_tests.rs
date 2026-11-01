//! Workflow payload limits allow framing and reject before section allocation.

use serde::{Serialize, Serializer};

use super::*;

#[derive(Debug)]
struct Blob(Vec<u8>);
impl Serialize for Blob {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&self.0)
    }
}
fn noise() -> Vec<u8> {
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    (0..4096)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let [byte, ..] = state.to_le_bytes();
            byte
        })
        .collect()
}
#[tokio::test]
async fn test_should_allow_incompressible_payload_at_exact_limit_with_frame_overhead() -> Result<()>
{
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("meta.ss.zst");
    let raw = noise();
    let payload = to_cbor(&Blob(raw.clone()))?;
    let limit = payload.len();
    let stats = write_archive(
        &path,
        ArchiveKind::ServiceMeta,
        vec![ArchiveSection::new(SECTION_STATE_CBOR, payload.clone(), 1)],
    )
    .await?;
    assert!(
        stats.compressed_bytes > usize_to_u64(limit)?,
        "fixture failed to exercise compressed overhead"
    );
    let read = read_archive_limited(&path, ArchiveKind::ServiceMeta, Some(limit)).await?;
    assert_eq!(get_required_section(&read, SECTION_STATE_CBOR)?, payload);
    let decoded: ciborium::Value = from_cbor(get_required_section(&read, SECTION_STATE_CBOR)?)?;
    assert_eq!(decoded, ciborium::Value::Bytes(raw));
    let error = read_archive_limited(&path, ArchiveKind::ServiceMeta, Some(limit - 1))
        .await
        .err()
        .ok_or_else(|| anyhow::anyhow!("accepted oversized section"))?;
    assert!(error.is::<BudgetLimit>());
    Ok(())
}
#[tokio::test]
async fn test_should_reject_compressed_and_expansion_limits_as_policy_errors() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("meta.ss.zst");
    fs::write(&path, vec![0; 1024 * 1024 + 4096]).await?;
    assert!(
        read_archive_limited(&path, ArchiveKind::ServiceMeta, Some(1))
            .await
            .err()
            .is_some_and(|error| error.is::<BudgetLimit>())
    );
    write_archive(
        &path,
        ArchiveKind::ServiceMeta,
        vec![ArchiveSection::new(SECTION_STATE_CBOR, vec![0; 4096], 1)],
    )
    .await?;
    assert!(
        read_archive_limited(&path, ArchiveKind::ServiceMeta, Some(1))
            .await
            .err()
            .is_some_and(|error| error.is::<BudgetLimit>())
    );
    Ok(())
}
#[tokio::test]
async fn test_should_reject_wrong_or_extra_workflow_data_entries_before_extraction() -> Result<()> {
    let directory = tempfile::tempdir()?;
    for extra in [false, true] {
        let staging = directory.path().join(if extra { "two" } else { "wrong" });
        fs::create_dir(&staging).await?;
        fs::write(staging.join("unexpected"), b"private payload").await?;
        if extra {
            fs::write(staging.join("history.cbor"), b"history").await?;
        }
        let path = directory.path().join("data.ss.zst");
        pack_data_archive(&staging, &path).await?;
        let target = directory.path().join("not-created");
        assert!(
            unpack_data_archive(&path, &target, Some(4096), Some("history.cbor"))
                .await
                .is_err()
        );
        assert!(!fs::try_exists(target).await?);
    }
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn test_should_reject_fifo_without_a_writer_and_join_the_exact_read_task() -> Result<()> {
    use nix::{sys::stat::Mode, unistd::mkfifo};
    use tokio::time::{Duration, timeout};

    let directory = tempfile::tempdir()?;
    let path = directory.path().join("meta.ss.zst");
    let create = path.clone();
    tokio::task::spawn_blocking(move || mkfifo(&create, Mode::S_IRUSR | Mode::S_IWUSR)).await??;
    let mut task = tokio::spawn(async move {
        read_archive_limited(&path, ArchiveKind::ServiceMeta, Some(4096)).await
    });
    let result = timeout(Duration::from_secs(2), &mut task).await??;
    assert!(result.is_err());
    assert!(task.is_finished());
    Ok(())
}

#[derive(Debug, Default)]
pub(super) struct OpenGate {
    entered: tokio::sync::Notify,
    released: tokio::sync::Notify,
}
impl OpenGate {
    pub(super) async fn wait(&self) {
        let release = self.released.notified();
        self.entered.notify_one();
        release.await;
    }
}
#[cfg(unix)]
#[tokio::test]
async fn test_should_reject_fifo_and_symlink_swap_after_regular_file_precheck() -> Result<()> {
    use std::sync::Arc;

    use nix::{sys::stat::Mode, unistd::mkfifo};
    use tokio::time::{Duration, timeout};

    for symlink in [false, true] {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("meta.ss.zst");
        let victim = directory.path().join("private");
        fs::write(&path, b"regular input").await?;
        fs::write(&victim, b"private file").await?;
        let gate = Arc::new(OpenGate::default());
        let delayed = gate.clone();
        let input = path.clone();
        let mut task =
            tokio::spawn(
                async move { read_file_bounded_owned(&input, 4096, Some(&delayed)).await },
            );
        timeout(Duration::from_secs(2), gate.entered.notified()).await?;
        fs::remove_file(&path).await?;
        if symlink {
            fs::symlink(&victim, &path).await?;
        } else {
            tokio::task::spawn_blocking(move || mkfifo(&path, Mode::S_IRUSR | Mode::S_IWUSR))
                .await??;
        }
        gate.released.notify_one();
        assert!(timeout(Duration::from_secs(2), &mut task).await??.is_err());
        assert!(task.is_finished());
    }
    Ok(())
}
