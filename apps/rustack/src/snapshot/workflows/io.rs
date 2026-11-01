//! Disk futures are retained in a joined task, including after observation expiry.

use std::{
    fmt,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use tokio::fs;

use super::{HISTORY_FILE, read_file_bounded};

pub(super) enum Output {
    History(Vec<u8>),
    Written,
}
impl fmt::Debug for Output {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ArchiveIoOutput(<redacted>)")
    }
}
async fn check(directory: &Path, required: bool) -> Result<()> {
    let mut entries = fs::read_dir(directory)
        .await
        .context("read workflow archive staging directory")?;
    let first = entries
        .next_entry()
        .await
        .context("read workflow history entry")?;
    match first {
        Some(entry) => {
            if entry.file_name() != HISTORY_FILE
                || !entry.file_type().await?.is_file()
                || entries.next_entry().await?.is_some()
            {
                bail!("invalid workflow history payload directory");
            }
        }
        None if required => bail!("missing workflow history payload"),
        None => {}
    }
    Ok(())
}
pub(super) async fn read(directory: PathBuf, bytes: usize) -> Result<Output> {
    check(&directory, true).await?;
    let history = read_file_bounded(
        &directory.join(HISTORY_FILE),
        u64::try_from(bytes).context("workflow archive byte overflow")?,
    )
    .await?;
    Ok(Output::History(history))
}
pub(super) async fn write(directory: PathBuf, bytes: Vec<u8>) -> Result<Output> {
    check(&directory, false).await?;
    fs::write(directory.join(HISTORY_FILE), bytes)
        .await
        .context("write workflow history payload")?;
    Ok(Output::Written)
}
