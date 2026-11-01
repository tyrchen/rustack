//! Standalone Linux code-volume probe; this is not a Lambda Runtime API client.

#![forbid(unsafe_code)]

use std::{
    error::Error,
    fs::Permissions,
    io,
    os::unix::fs::{MetadataExt, PermissionsExt},
    process::ExitCode,
    str::from_utf8,
    time::Duration,
};

use serde_json::json;
use tokio::{
    fs::{self, File},
    io::{AsyncReadExt, AsyncWriteExt, stderr, stdout},
    time::timeout,
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match timeout(Duration::from_secs(30), probe()).await {
        Ok(Ok(())) => ExitCode::SUCCESS,
        result => {
            let message = match result {
                Ok(Err(error)) => format!("code volume probe failed: {error}\n"),
                _ => "code volume probe deadline exceeded\n".to_owned(),
            };
            let _ = timeout(
                Duration::from_secs(1),
                stderr().write_all(message.as_bytes()),
            )
            .await;
            ExitCode::FAILURE
        }
    }
}

async fn probe() -> Result<(), Box<dyn Error>> {
    let mut input = Vec::new();
    File::open("/var/task/data/input")
        .await?
        .take(65)
        .read_to_end(&mut input)
        .await?;
    if input.len() > 64 {
        return Err(io::Error::from(io::ErrorKind::InvalidData).into());
    }
    let input = from_utf8(&input)?.trim().parse::<i64>()?;
    let result = input
        .checked_mul(2)
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData))?;
    let mut observations = Vec::new();
    for (path, mode) in [
        ("/var/task/bootstrap", 0o555),
        ("/var/task/data", 0o555),
        ("/var/task/data/input", 0o444),
    ] {
        let metadata = fs::metadata(path).await?;
        if metadata.uid() != 0 || metadata.gid() != 0 || metadata.mode() & 0o7777 != mode {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "code metadata mismatch: uid={} gid={} mode={:o}",
                    metadata.uid(),
                    metadata.gid(),
                    metadata.mode() & 0o7777
                ),
            )
            .into());
        }
        observations.push(
            json!({"uid":metadata.uid(), "gid":metadata.gid(), "mode":metadata.mode() & 0o7777}),
        );
    }
    match fs::set_permissions("/var/task/data/input", Permissions::from_mode(0o600)).await {
        // chmod reaches the mount write check independently of data-write DAC.
        Err(error) if error.raw_os_error() == Some(30) => {}
        Err(error) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("code mount permission mutation: {:?}", error.raw_os_error()),
            )
            .into());
        }
        Ok(()) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "code mount accepted metadata mutation",
            )
            .into());
        }
    }
    fs::write("/tmp/rustack-code-probe", b"writable").await?;
    if fs::read("/tmp/rustack-code-probe").await? != b"writable" {
        return Err(io::Error::from(io::ErrorKind::InvalidData).into());
    }
    fs::remove_file("/tmp/rustack-code-probe").await?;
    let output = serde_json::to_vec(
        &json!({"computed":result, "readOnly":true, "tmpWritable":true, "metadata":observations}),
    )?;
    stdout().write_all(&output).await?;
    Ok(())
}
