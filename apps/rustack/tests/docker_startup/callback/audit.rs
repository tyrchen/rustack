//! Bounded independent plaintext/hash observations over one stopped private generation.

use std::{
    collections::HashSet,
    fmt::{Formatter, Result as FmtResult, Write},
    future::Future,
    io::{Cursor, Read, Result as IoResult},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, ensure};
use nix::libc::{O_NOFOLLOW, O_NONBLOCK};
use rustack_lambda_core::provider::LambdaSnapshot;
use serde::{
    Deserialize, Deserializer,
    de::{Error as DeError, SeqAccess, Visitor},
};
use sha2::{Digest, Sha256};
use tokio::{
    fs::{self, OpenOptions},
    io::AsyncReadExt,
    task::{JoinHandle, spawn_blocking},
    time::{Instant, timeout_at},
};
use zstd::stream::read::Decoder;

use super::{super::archive, Evidence};

const COMPRESSED: usize = 4 * 1024 * 1024;
const PLAIN: usize = 16 * 1024 * 1024;
const TOTAL_COMPRESSED: usize = 16 * 1024 * 1024;
const TOTAL_PLAIN: usize = 64 * 1024 * 1024;
const CANDIDATES: usize = 65_536;

pub(super) fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn hex_digest(bytes: &[u8]) -> Result<String> {
    let mut encoded = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(&mut encoded, "{byte:02x}")?;
    }
    Ok(encoded)
}
#[derive(Debug)]
struct Scan {
    hashes: HashSet<String>,
    candidates: usize,
    plain: usize,
    deadline: Instant,
    failed: bool,
}
impl Scan {
    fn new(hashes: &[String], candidates: usize, plain: usize, deadline: Instant) -> Result<Self> {
        ensure!(
            (1..=64).contains(&hashes.len())
                && hashes.iter().all(|h| valid_hash(h))
                && candidates <= CANDIDATES
                && plain <= PLAIN,
            "invalid callback scan input"
        );
        Ok(Self {
            hashes: hashes.iter().cloned().collect(),
            candidates,
            plain,
            deadline,
            failed: false,
        })
    }
    fn scan(&mut self, bytes: &[u8]) -> Result<()> {
        ensure!(!self.failed, "callback scan already failed");
        let result = self.scan_inner(bytes);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn scan_inner(&mut self, bytes: &[u8]) -> Result<()> {
        ensure!(
            Instant::now() < self.deadline && bytes.len() <= PLAIN,
            "callback scan source/deadline cap"
        );
        self.plain = self
            .plain
            .checked_add(bytes.len())
            .context("callback plain bytes")?;
        ensure!(self.plain <= TOTAL_PLAIN, "callback aggregate plain cap");
        for credential in [b"local-key".as_slice(), b"local-secret".as_slice()] {
            ensure!(
                !bytes.windows(credential.len()).any(|v| v == credential),
                "injected SDK credential persisted"
            );
        }
        let mut run = 0usize;
        for (at, byte) in bytes.iter().enumerate() {
            run = if byte.is_ascii_digit() || (b'a'..=b'f').contains(byte) {
                run.checked_add(1).context("callback hex run")?
            } else {
                0
            };
            if run < 64 {
                continue;
            }
            self.candidates = self
                .candidates
                .checked_add(1)
                .context("callback candidates")?;
            ensure!(
                self.candidates <= CANDIDATES,
                "callback aggregate candidate cap"
            );
            let start = at.checked_sub(63).context("callback candidate offset")?;
            let value = bytes.get(start..=at).context("callback candidate window")?;
            let digest = hex_digest(value)?;
            ensure!(
                !self.hashes.contains(&digest),
                "callback credential persisted"
            );
        }
        ensure!(
            Instant::now() < self.deadline,
            "late callback scan observation"
        );
        Ok(())
    }
}

fn fresh(deadline: Instant) -> Result<()> {
    ensure!(Instant::now() < deadline, "callback observation expired");
    Ok(())
}
async fn io<T>(deadline: Instant, future: impl Future<Output = IoResult<T>>) -> Result<T> {
    fresh(deadline)?;
    let value = timeout_at(deadline, future)
        .await
        .context("callback IO deadline")??;
    fresh(deadline)?;
    Ok(value)
}
async fn files(root: &Path, deadline: Instant) -> Result<Vec<PathBuf>> {
    ensure!(
        io(deadline, fs::symlink_metadata(root))
            .await?
            .file_type()
            .is_dir(),
        "invalid callback generation root"
    );
    let root = io(deadline, fs::canonicalize(root)).await?;
    let mut stack = vec![(root.clone(), 0usize)];
    let mut entries = 0usize;
    let mut found = Vec::new();
    while let Some((directory, depth)) = stack.pop() {
        fresh(deadline)?;
        ensure!(depth <= 8, "callback archive depth cap");
        let mut directory = io(deadline, fs::read_dir(directory)).await?;
        while let Some(entry) = io(deadline, directory.next_entry()).await? {
            entries = entries.checked_add(1).context("callback archive nodes")?;
            ensure!(entries <= 128, "callback archive entry cap");
            let kind = io(deadline, entry.file_type()).await?;
            let path = entry.path();
            ensure!(
                path.as_os_str().len() <= 4096
                    && !kind.is_symlink()
                    && io(deadline, fs::canonicalize(&path))
                        .await?
                        .starts_with(&root),
                "unsafe callback archive path"
            );
            if kind.is_dir() {
                stack.push((
                    path,
                    depth.checked_add(1).context("callback archive depth")?,
                ));
            } else {
                ensure!(
                    kind.is_file() && found.len() < 64,
                    "callback archive file kind/count"
                );
                found.push(path);
            }
        }
    }
    Ok(found)
}
async fn read(path: &Path, cap: usize, deadline: Instant) -> Result<Vec<u8>> {
    let file = io(
        deadline,
        OpenOptions::new()
            .read(true)
            .custom_flags(O_NOFOLLOW | O_NONBLOCK)
            .open(path),
    )
    .await?;
    let metadata = io(deadline, file.metadata()).await?;
    ensure!(
        metadata.is_file() && metadata.len() <= u64::try_from(cap)?,
        "callback observed file cap/kind"
    );
    let limit = u64::try_from(cap)?
        .checked_add(1)
        .context("callback read cap")?;
    let mut bytes = Vec::new();
    io(deadline, file.take(limit).read_to_end(&mut bytes)).await?;
    ensure!(bytes.len() <= cap, "callback observed read cap");
    Ok(bytes)
}
async fn owned_cpu<T>(deadline: Instant, mut job: JoinHandle<Result<T>>) -> Result<T> {
    let result = if let Ok(result) = timeout_at(deadline, &mut job).await {
        result.map_err(|_| anyhow!("callback observer worker failed"))?
    } else {
        // Preserve ownership of the original bounded CPU job after expiry.
        let _late = job.await;
        return Err(anyhow!("callback observer worker expired"));
    };
    fresh(deadline)?;
    result
}
fn plain(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = Decoder::new(Cursor::new(bytes))?.take(
        u64::try_from(PLAIN)?
            .checked_add(1)
            .context("callback decoder cap")?,
    );
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    ensure!(out.len() <= PLAIN, "callback decompressed cap");
    Ok(out)
}

// Deliberately project only durable execution identities/outcomes; definitions and
// arbitrary business payloads cannot satisfy this independent presence oracle.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Workflows {
    schema_version: u32,
    #[serde(deserialize_with = "executions")]
    executions: Vec<Execution>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Execution {
    arn: String,
    stopped_at: Option<String>,
    outcome: Outcome,
}
#[derive(Debug, Deserialize)]
#[serde(tag = "status", rename_all = "SCREAMING_SNAKE_CASE")]
enum Outcome {
    Running,
    Succeeded,
    Failed,
    TimedOut,
    Aborted,
}
fn executions<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Execution>, D::Error> {
    struct Rows;
    impl<'de> Visitor<'de> for Rows {
        type Value = Vec<Execution>;
        fn expecting(&self, formatter: &mut Formatter<'_>) -> FmtResult {
            formatter.write_str("bounded callback execution rows")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut rows: A) -> Result<Self::Value, A::Error> {
            let mut values = Vec::new();
            while let Some(row) = rows.next_element::<Execution>()? {
                if values.len() >= 64
                    || row.arn.len() > 256
                    || row.stopped_at.as_ref().is_some_and(|v| v.len() > 64)
                {
                    return Err(A::Error::custom("callback execution projection cap"));
                }
                values.push(row);
            }
            Ok(values)
        }
    }
    deserializer.deserialize_seq(Rows)
}
fn workflows(data: &[u8], expected: &[String]) -> Result<()> {
    let snapshot: Workflows =
        archive::decode_plain(data).map_err(|_| anyhow!("invalid saved callback executions"))?;
    ensure!(snapshot.schema_version == 1, "callback metadata schema");
    let mut actual = HashSet::new();
    for row in snapshot.executions {
        ensure!(
            actual.insert(row.arn.clone()),
            "duplicate callback execution row"
        );
        if expected.contains(&row.arn) {
            ensure!(
                row.stopped_at.is_some() && !matches!(row.outcome, Outcome::Running),
                "callback execution not terminal"
            );
        }
    }
    ensure!(
        expected.iter().all(|arn| actual.contains(arn)),
        "actual terminal callback executions absent from archive"
    );
    Ok(())
}
#[derive(Debug, Clone, Copy)]
enum Kind {
    Other,
    Lambda,
    Workflows,
}
fn inspect(
    mut scan: Scan,
    bytes: Vec<u8>,
    compressed: bool,
    kind: Kind,
    expected: &[String],
) -> Result<Scan> {
    let data = if compressed { plain(&bytes)? } else { bytes };
    scan.scan(&data)?;
    match kind {
        Kind::Other => (),
        Kind::Lambda => {
            let snapshot: LambdaSnapshot = archive::decode_plain(&data)
                .map_err(|_| anyhow!("invalid saved callback functions"))?;
            let names: HashSet<_> = snapshot
                .functions
                .functions
                .iter()
                .map(|f| f.name.as_str())
                .collect();
            ensure!(
                names == expected.iter().map(String::as_str).collect(),
                "actual callback functions absent from archive"
            );
        }
        Kind::Workflows => workflows(&data, expected)?,
    }
    fresh(scan.deadline)?;
    Ok(scan)
}
pub(super) async fn run(root: &Path, input: &Evidence, deadline: Instant) -> Result<()> {
    let selected = root.join("snapshots/images");
    ensure!(
        io(deadline, fs::symlink_metadata(&selected))
            .await?
            .file_type()
            .is_dir(),
        "linked callback generation"
    );
    let generation = io(deadline, fs::canonicalize(&selected)).await?;
    let mut scan = Scan::new(
        &input.hashes,
        input.history_candidates,
        input.history_bytes,
        deadline,
    )?;
    let log = read(&root.join("server.log"), 1024 * 1024, deadline).await?;
    fresh(deadline)?;
    scan = owned_cpu(
        deadline,
        spawn_blocking(move || inspect(scan, log, false, Kind::Other, &[])),
    )
    .await?;
    let mut compressed = 0usize;
    let mut lambda = false;
    let mut registry = false;
    for path in files(&generation, deadline).await? {
        let bytes = read(&path, COMPRESSED, deadline).await?;
        compressed = compressed
            .checked_add(bytes.len())
            .context("callback compressed bytes")?;
        ensure!(
            compressed <= TOTAL_COMPRESSED,
            "callback aggregate compressed cap"
        );
        let relative = path.strip_prefix(&generation)?;
        let (kind, expected) = if relative == Path::new("services/lambda/meta.ss.zst") {
            lambda = true;
            (Kind::Lambda, input.functions.to_vec())
        } else if relative == Path::new("services/stepfunctions/meta.ss.zst") {
            registry = true;
            (Kind::Workflows, input.executions.clone())
        } else {
            (Kind::Other, Vec::new())
        };
        let compressed = path.extension().is_some_and(|v| v == "zst");
        fresh(deadline)?;
        scan = owned_cpu(
            deadline,
            spawn_blocking(move || inspect(scan, bytes, compressed, kind, &expected)),
        )
        .await?;
    }
    ensure!(
        lambda && registry,
        "callback saved service metadata missing"
    );
    fresh(deadline)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nix::{sys::stat::Mode, unistd::mkfifo};
    use serde_json::Value;
    use tokio::net::UnixListener;

    use super::*;

    fn digest(bytes: &[u8]) -> Result<String> {
        hex_digest(bytes)
    }
    fn scan(hashes: &[String]) -> Result<Scan> {
        Scan::new(hashes, 0, 0, Instant::now() + Duration::from_secs(10))
    }
    #[test]
    fn test_should_detect_every_embedded_and_chunk_split_token() -> Result<()> {
        let token = b"0123456789abcdef".repeat(4);
        for offset in 0..64 {
            let mut bytes = vec![b'a'; offset];
            bytes.extend_from_slice(&token);
            bytes.extend_from_slice(b"ffff");
            assert!(scan(&[digest(&token)?])?.scan(&bytes).is_err());
        }
        let encoded = zstd::stream::encode_all(Cursor::new(&token), 1)?;
        let decoded = plain(&encoded)?;
        assert!(scan(&[digest(&token)?])?.scan(&decoded).is_err());
        Ok(())
    }
    #[test]
    fn test_should_charge_overlapping_candidates_across_all_sources_and_fail_closed() -> Result<()>
    {
        let mut state = scan(&[digest(b"not a token")?])?;
        state.scan(&vec![b'a'; CANDIDATES + 63])?;
        assert_eq!(state.candidates, CANDIDATES);
        assert!(state.scan(&[b'c'; 64]).is_err());
        assert!(state.scan(b"safe").is_err());
        assert!(Scan::new(&["invalid".into()], 0, 0, Instant::now()).is_err());
        assert!(Scan::new(&[digest(b"safe")?], CANDIDATES + 1, 0, Instant::now()).is_err());
        Ok(())
    }
    #[test]
    fn test_should_refuse_decoder_expansion_and_late_or_oversized_observations() -> Result<()> {
        let encoded = zstd::stream::encode_all(Cursor::new(vec![0; PLAIN + 1]), 1)?;
        assert!(plain(&encoded).is_err());
        assert!(
            scan(&[digest(b"safe")?])?
                .scan(&vec![0; PLAIN + 1])
                .is_err()
        );
        assert!(
            Scan::new(&[digest(b"safe")?], 0, 0, Instant::now())?
                .scan(b"safe")
                .is_err()
        );
        Ok(())
    }
    #[tokio::test]
    async fn test_should_reject_linked_special_deep_and_excessive_archive_entries() -> Result<()> {
        let root = tempfile::tempdir()?;
        fs::symlink("/outside", root.path().join("link")).await?;
        assert!(
            files(root.path(), Instant::now() + Duration::from_secs(5))
                .await
                .is_err()
        );
        fs::remove_file(root.path().join("link")).await?;
        let socket = UnixListener::bind(root.path().join("socket"))?;
        assert!(
            files(root.path(), Instant::now() + Duration::from_secs(5))
                .await
                .is_err()
        );
        drop(socket);
        fs::remove_file(root.path().join("socket")).await?;
        let mut deep = root.path().to_owned();
        for _ in 0..9 {
            deep.push("nested");
        }
        fs::create_dir_all(&deep).await?;
        assert!(
            files(root.path(), Instant::now() + Duration::from_secs(5))
                .await
                .is_err()
        );
        fs::remove_dir_all(root.path().join("nested")).await?;
        for index in 0..65 {
            fs::write(root.path().join(index.to_string()), b"").await?;
        }
        assert!(
            files(root.path(), Instant::now() + Duration::from_secs(5))
                .await
                .is_err()
        );
        Ok(())
    }
    #[test]
    fn test_should_reject_both_private_sdk_values_with_redacted_errors() -> Result<()> {
        for value in [b"local-key".as_slice(), b"local-secret".as_slice()] {
            let error = scan(&[digest(b"safe")?])?
                .scan(value)
                .err()
                .context("credential leak accepted")?;
            assert_eq!(error.to_string(), "injected SDK credential persisted");
        }
        Ok(())
    }
    #[test]
    fn test_should_require_framed_actual_terminal_execution_rows() -> Result<()> {
        let arn = "arn:aws:states:us-east-1:000000000000:execution:machine:case";
        let expected = vec![arn.to_owned()];
        let frame = |value: &Value| -> Result<Vec<u8>> { plain(&archive::encode(value)?) };
        let empty = serde_json::json!({"schemaVersion":1,"executions":[],"unrelated":arn});
        assert!(workflows(&frame(&empty)?, &expected).is_err());
        let mut actual = serde_json::json!({"schemaVersion":1,"executions":[{
            "arn":arn,"stoppedAt":"2026-10-30T00:00:00Z","outcome":{"status":"SUCCEEDED","output":"42"}
        }]});
        let valid = frame(&actual)?;
        workflows(&valid, &expected)?;
        assert!(workflows(arn.as_bytes(), &expected).is_err());
        let mut bad_crc = valid.clone();
        *bad_crc.get_mut(64).context("CRC corruption")? ^= 1;
        assert!(workflows(&bad_crc, &expected).is_err());
        actual["executions"][0]["outcome"]["status"] = "RUNNING".into();
        assert!(workflows(&frame(&actual)?, &expected).is_err());
        actual["executions"][0]["outcome"]["status"] = "FAILED".into();
        actual["executions"][0]["stoppedAt"] = Value::Null;
        assert!(workflows(&frame(&actual)?, &expected).is_err());
        Ok(())
    }
    #[tokio::test]
    async fn test_should_reject_replaced_fifo_before_any_blocking_read() -> Result<()> {
        let root = tempfile::tempdir()?;
        let path = root.path().join("server.log");
        fs::write(&path, b"safe").await?;
        let deadline = Instant::now() + Duration::from_secs(2);
        assert_eq!(
            files(root.path(), deadline).await?,
            vec![fs::canonicalize(&path).await?]
        );
        fs::remove_file(&path).await?;
        mkfifo(&path, Mode::from_bits_truncate(0o600))?;
        assert!(read(&path, 1024, deadline).await.is_err());
        assert!(Instant::now() < deadline);
        Ok(())
    }
    #[tokio::test]
    async fn test_should_reject_completed_worker_observed_at_cutoff() -> Result<()> {
        let job = spawn_blocking(|| Ok(42));
        while !job.is_finished() {
            tokio::task::yield_now().await;
        }
        let deadline = Instant::now();
        assert!(owned_cpu(deadline, job).await.is_err());
        Ok(())
    }
}
