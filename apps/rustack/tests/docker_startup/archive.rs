//! Independent bounded fixture codec for one real v2 Lambda metadata section.

use std::{
    io::{Cursor, Read},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use rustack_lambda_core::provider::LambdaSnapshot;
use tokio::{fs, io::AsyncReadExt, task::spawn_blocking, time::timeout};

const PLAIN_LIMIT: usize = 16 * 1024 * 1024;
const COMPRESSED_LIMIT: u64 = 4 * 1024 * 1024;
fn metadata(root: &Path) -> PathBuf {
    root.join("snapshots/images/services/lambda/meta.ss.zst")
}
async fn bytes(path: &Path) -> Result<Vec<u8>> {
    let file = fs::File::open(path).await?;
    let mut bytes = Vec::new();
    file.take(COMPRESSED_LIMIT + 1)
        .read_to_end(&mut bytes)
        .await?;
    ensure!(
        u64::try_from(bytes.len())? <= COMPRESSED_LIMIT,
        "fixture compressed metadata cap"
    );
    Ok(bytes)
}
fn u16_at(bytes: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        bytes
            .get(at..at.checked_add(2).context("offset")?)
            .context("u16")?
            .try_into()?,
    ))
}
fn u32_at(bytes: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(at..at.checked_add(4).context("offset")?)
            .context("u32")?
            .try_into()?,
    ))
}
fn u64_at(bytes: &[u8], at: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(
        bytes
            .get(at..at.checked_add(8).context("offset")?)
            .context("u64")?
            .try_into()?,
    ))
}
fn decode(bytes: &[u8]) -> Result<LambdaSnapshot> {
    let mut decoder = zstd::stream::read::Decoder::new(Cursor::new(bytes))?.take(
        u64::try_from(PLAIN_LIMIT)?
            .checked_add(1)
            .context("plain cap")?,
    );
    let mut plain = Vec::new();
    decoder.read_to_end(&mut plain)?;
    ensure!(
        plain.len() <= PLAIN_LIMIT && plain.len() >= 76,
        "fixture plain metadata cap/header"
    );
    ensure!(
        plain.get(..8) == Some(b"RSSNAP\0\x02")
            && u16_at(&plain, 8)? == 2
            && u16_at(&plain, 10)? == 0,
        "fixture v2 metadata kind"
    );
    ensure!(
        u32_at(&plain, 12)? == 36
            && u32_at(&plain, 16)? == 1
            && u64_at(&plain, 20)? == u64::try_from(plain.len())?
            && u64_at(&plain, 28)? == 0,
        "fixture metadata layout"
    );
    ensure!(
        u16_at(&plain, 36)? == 1
            && u16_at(&plain, 38)? == 0
            && u64_at(&plain, 40)? == 64
            && u64_at(&plain, 56)? == 1,
        "fixture single state section"
    );
    let end = plain.len().checked_sub(12).context("footer")?;
    ensure!(
        u64_at(&plain, 48)? == u64::try_from(end.checked_sub(64).context("payload")?)?
            && u64_at(&plain, end.checked_add(4).context("footer length")?)? == u64::try_from(end)?,
        "fixture section bounds"
    );
    ensure!(
        crc32fast::hash(plain.get(..end).context("CRC range")?) == u32_at(&plain, end)?,
        "fixture CRC"
    );
    let payload = plain.get(64..end).context("state payload")?;
    let mut reader = Cursor::new(payload);
    let snapshot = ciborium::de::from_reader_with_recursion_limit(&mut reader, 128)?;
    ensure!(
        reader.position() == u64::try_from(payload.len())?,
        "fixture trailing CBOR"
    );
    Ok(snapshot)
}
fn encode(snapshot: &LambdaSnapshot) -> Result<Vec<u8>> {
    let mut state = Vec::new();
    ciborium::ser::into_writer(snapshot, &mut state)?;
    let total = state.len().checked_add(76).context("archive length")?;
    ensure!(total <= PLAIN_LIMIT, "fixture encoded cap");
    let mut plain = Vec::with_capacity(total);
    plain.extend_from_slice(b"RSSNAP\0\x02");
    plain.extend_from_slice(&2u16.to_le_bytes());
    plain.extend_from_slice(&0u16.to_le_bytes());
    plain.extend_from_slice(&36u32.to_le_bytes());
    plain.extend_from_slice(&1u32.to_le_bytes());
    plain.extend_from_slice(&u64::try_from(total)?.to_le_bytes());
    plain.extend_from_slice(&0u64.to_le_bytes());
    plain.extend_from_slice(&1u16.to_le_bytes());
    plain.extend_from_slice(&0u16.to_le_bytes());
    plain.extend_from_slice(&64u64.to_le_bytes());
    plain.extend_from_slice(&u64::try_from(state.len())?.to_le_bytes());
    plain.extend_from_slice(&1u64.to_le_bytes());
    plain.extend_from_slice(&state);
    let payload_len = u64::try_from(plain.len())?;
    plain.extend_from_slice(&crc32fast::hash(&plain).to_le_bytes());
    plain.extend_from_slice(&payload_len.to_le_bytes());
    ensure!(plain.len() == total, "fixture encoded layout");
    let compressed = zstd::stream::encode_all(Cursor::new(plain), 1)?;
    ensure!(
        u64::try_from(compressed.len())? <= COMPRESSED_LIMIT,
        "fixture compressed output cap"
    );
    Ok(compressed)
}
pub(super) async fn read(root: &Path) -> Result<LambdaSnapshot> {
    let bytes = bytes(&metadata(root)).await?;
    let mut job = spawn_blocking(move || decode(&bytes));
    timeout(Duration::from_secs(30), &mut job).await??
}
pub(super) async fn corrupt_size(root: &Path) -> Result<()> {
    let path = metadata(root);
    let original = bytes(&path).await?;
    let mut job = spawn_blocking(move || {
        let mut snapshot = decode(&original)?;
        let target = snapshot
            .functions
            .functions
            .iter_mut()
            .find(|function| {
                function.latest.state == "Active" && function.latest.resolved_image.is_some()
            })
            .context("actual saved usable image")?;
        target.latest.code_size = target
            .latest
            .code_size
            .checked_add(1)
            .context("code size")?;
        encode(&snapshot)
    });
    let corrupted = timeout(Duration::from_secs(30), &mut job).await???;
    fs::write(path, corrupted).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_should_refuse_truncated_wrong_kind_and_invalid_crc_fixture_frames() -> Result<()> {
        let source = encode(&LambdaSnapshot::default())?;
        assert!(decode(source.get(..source.len() / 2).context("truncated source")?).is_err());
        for at in [8usize, 36, 64] {
            let mut plain = zstd::stream::decode_all(Cursor::new(&source))?;
            *plain.get_mut(at).context("fixed corrupt offset")? ^= 1;
            let corrupted = zstd::stream::encode_all(Cursor::new(plain), 1)?;
            assert!(decode(&corrupted).is_err());
        }
        Ok(())
    }
    #[test]
    fn test_should_refuse_fixture_decompression_above_the_plain_cap() -> Result<()> {
        let compressed = zstd::stream::encode_all(Cursor::new(vec![0; PLAIN_LIMIT + 1]), 1)?;
        assert!(decode(&compressed).is_err());
        Ok(())
    }
}
