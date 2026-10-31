//! Shared ordinary image SDK child and server ownership through every exit.

use std::{ffi::OsString, time::Duration};

use anyhow::{Context, Result, ensure};
use serde_json::Value;
use tokio::time::{Instant, timeout_at};

use super::{
    faults::check_logs,
    process::{Profile, Server},
    runner::{Report, Runner, interpreter},
};

pub(super) async fn exercise<F, V>(profile: Profile, prepare: F, validate: V) -> Result<()>
where
    F: for<'a> AsyncFnOnce(&'a mut Server) -> Result<Vec<OsString>>,
    V: FnOnce(&Value) -> Result<()>,
{
    let mut server = Server::spawn(profile).await?;
    server
        .root
        .as_mut()
        .context("image SDK evidence root")?
        .disable_cleanup(true);
    let cutoff = Instant::now() + Duration::from_secs(480);
    let prepared = timeout_at(cutoff, prepare(&mut server))
        .await
        .context("ordinary image SDK preparation expired")
        .and_then(|result| result);
    let result = async {
        let arguments = prepared?;
        let child = Runner::spawn(&interpreter()?, &arguments)?;
        let (entered, report) = join_with_pid_before(child, cutoff).await?;
        ensure!(
            report.pid == entered? && !report.forced && report.status.success(),
            "ordinary image SDK failed: {}",
            String::from_utf8_lossy(&report.stderr)
        );
        let observed: Value = serde_json::from_slice(&report.stdout)?;
        validate(&observed)
    }
    .await;
    let stopped = server.stop(true).await;
    let audited = server.audit().await;
    let logs = check_logs(&server).await;
    if result.is_err() || stopped.is_err() || audited.is_err() || logs.is_err() {
        server.retain_on_error().await;
    }
    result?;
    stopped?;
    audited?;
    logs?;
    server
        .root
        .as_mut()
        .context("image SDK root after cleanup")?
        .disable_cleanup(false);
    Ok(())
}

/// Join this same child after the original-cutoff PID barrier, including failure.
pub(super) async fn join_with_pid_before(
    mut child: Runner,
    cutoff: Instant,
) -> Result<(Result<u32>, Report)> {
    let entered = child
        .await_pid_before(cutoff.min(Instant::now() + Duration::from_secs(5)))
        .await;
    let report = child
        .wait(if entered.is_ok() {
            cutoff
        } else {
            Instant::now()
        })
        .await?;
    Ok((entered, report))
}
