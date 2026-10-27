//! Direct child ownership with bounded output and joined timeout cleanup.

use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
};

use anyhow::{Context, Result, ensure};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{Child, Command},
    time::{Duration, Instant, timeout, timeout_at},
};

#[derive(Debug)]
pub(super) struct Runner {
    child: Child,
}
#[derive(Debug)]
pub(super) struct Report {
    pub pid: u32,
    pub forced: bool,
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}
pub(super) fn interpreter() -> Result<PathBuf> {
    let value = PathBuf::from(
        env::var_os("RUSTACK_WORKFLOWS_PYTHON")
            .context("run make test-stepfunctions-sdk to provision pinned Python")?,
    );
    ensure!(
        value.is_absolute(),
        "missing absolute pinned Python executable"
    );
    Ok(value)
}
impl Runner {
    pub fn spawn(program: &Path, arguments: &[OsString]) -> Result<Self> {
        let child = Command::new(program)
            .args(arguments)
            .env_clear()
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("AWS_EC2_METADATA_DISABLED", "true")
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("spawn directly owned fixture process")?;
        Ok(Self { child })
    }
    pub async fn await_pid(&mut self) -> Result<u32> {
        self.await_pid_before(Instant::now() + Duration::from_secs(5))
            .await
    }
    pub async fn await_pid_before(&mut self, deadline: Instant) -> Result<u32> {
        ensure!(
            Instant::now() < deadline,
            "PID barrier deadline already expired"
        );
        let expected = self.child.id().context("owned Python PID")?;
        let observed = timeout_at(deadline, async {
            let output = self.child.stdout.as_mut().context("PID barrier stdout")?;
            let mut text = Vec::new();
            loop {
                let byte = output.read_u8().await?;
                if byte == b'\n' {
                    break;
                }
                ensure!(text.len() < 16, "oversized PID barrier");
                text.push(byte);
            }
            let observed: u32 = std::str::from_utf8(&text)?.parse()?;
            ensure!(
                observed == expected,
                "Python PID differs from directly owned child"
            );
            Ok(observed)
        })
        .await
        .context("Python execution barrier deadline")??;
        ensure!(Instant::now() < deadline, "late PID barrier observation");
        Ok(observed)
    }
    pub async fn wait(mut self, deadline: Instant) -> Result<Report> {
        let pid = self.child.id().context("owned Python PID")?;
        let stdout = self.child.stdout.take().context("Python stdout")?;
        let stderr = self.child.stderr.take().context("Python stderr")?;
        let (status, stdout, stderr) = tokio::join!(
            wait_child(&mut self.child, deadline),
            capture(stdout, deadline + Duration::from_secs(10)),
            capture(stderr, deadline + Duration::from_secs(10))
        );
        let (status, forced) = status?;
        Ok(Report {
            pid,
            forced,
            status,
            stdout: stdout?,
            stderr: stderr?,
        })
    }
}
async fn wait_child(child: &mut Child, deadline: Instant) -> Result<(ExitStatus, bool)> {
    if let Ok(status) = timeout_at(deadline, child.wait()).await {
        return Ok((status?, false));
    }
    child
        .start_kill()
        .context("kill exact owned fixture process after deadline")?;
    let status = timeout(Duration::from_secs(5), child.wait())
        .await
        .context("fixture process forced reap deadline")??;
    Ok((status, true))
}
async fn capture<R: AsyncRead + Unpin>(mut input: R, deadline: Instant) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let count = timeout_at(deadline, input.read(&mut buffer))
            .await
            .context("fixture pipe drain deadline")??;
        if count == 0 {
            return Ok(output);
        }
        ensure!(
            output.len().saturating_add(count) <= 32768,
            "fixture output exceeded32KiB cap"
        );
        output.extend_from_slice(buffer.get(..count).context("invalid pipe read count")?);
    }
}
