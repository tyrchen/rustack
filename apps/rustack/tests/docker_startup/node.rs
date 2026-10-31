//! Real official Node runtime consumption with retained server and SDK owners.

use std::{env, ffi::OsString, path::PathBuf, time::Duration};

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use tokio::time::{Instant, timeout_at};

use super::{
    faults::check_logs,
    process::{self, Profile, Server},
    runner::{Report, Runner, interpreter},
    sdk,
};

pub(super) async fn exercise() -> Result<()> {
    let mut server = Server::spawn(Profile::Connected).await?;
    server
        .root
        .as_mut()
        .context("Node evidence root")?
        .disable_cleanup(true);
    let cutoff = Instant::now() + Duration::from_secs(480);
    let prepared = timeout_at(cutoff, prepare(&mut server))
        .await
        .context("ordinary Node observer expired")
        .and_then(|result| result);
    let result = match prepared {
        Ok(arguments) => run_sdk(arguments, cutoff).await,
        Err(error) => Err(error),
    };
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
        .context("Node evidence root after cleanup")?
        .disable_cleanup(false);
    Ok(())
}

async fn prepare(server: &mut Server) -> Result<[OsString; 7]> {
    let image = env::var("RUSTACK_NODE_IMAGE").context("explicit official Node fixture")?;
    server.track_image(&image).await?;
    server.ready().await?;
    sdk::node_compute(&server.endpoint, &image, &process::architecture()?).await?;
    let (owner, engine) = server.observation_identity().await?;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    Ok([
        root.join("tests/workflows/node_image_sdk.py")
            .into_os_string(),
        OsString::from(&server.endpoint),
        OsString::from(image),
        OsString::from(if process::architecture()? == "arm64" {
            "arm64"
        } else {
            "x86_64"
        }),
        env::var_os("RUSTACK_RUNTIME_DOCKER_SOCKET").context("explicit Node metadata socket")?,
        OsString::from(owner),
        OsString::from(engine),
    ])
}

async fn join_sdk(mut child: Runner, cutoff: Instant) -> Result<(Result<u32>, Report)> {
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

async fn run_sdk(arguments: [OsString; 7], cutoff: Instant) -> Result<()> {
    let child = Runner::spawn(&interpreter()?, &arguments)?;
    let (entered, report) = join_sdk(child, cutoff).await?;
    ensure!(
        report.pid == entered? && !report.forced && report.status.success(),
        "ordinary Node SDK failed: {}",
        String::from_utf8_lossy(&report.stderr)
    );
    let observed: Value = serde_json::from_slice(&report.stdout)?;
    ensure!(
        observed["schemaVersion"] == 1
            && observed["cases"]
                == json!([
                    "cold-warm-context-filesystem-logs",
                    "published-alias-command-isolation",
                    "FunctionError-replacement-warm",
                    "ASL-node-computation"
                ])
            && observed["nodeVersion"]
                .as_str()
                .is_some_and(|version| version.starts_with("24.")),
        "Node semantic inventory or runtime differs"
    );
    eprintln!("official Node runtime version: {}", observed["nodeVersion"]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, path::Path};

    use anyhow::{Result, ensure};
    use nix::{errno::Errno, sys::signal::kill, unistd::Pid};
    use tokio::time::{Duration, Instant};

    use super::{Runner, join_sdk};

    #[tokio::test]
    async fn test_should_join_late_node_sdk_child_before_reporting_expired_handshake() -> Result<()>
    {
        let cutoff = Instant::now() - Duration::from_millis(1);
        let mut child = Runner::spawn(
            Path::new("/bin/sh"),
            &[
                OsString::from("-c"),
                OsString::from("echo $$; exec /bin/sleep 60"),
            ],
        )?;
        let held = child.await_pid().await;
        let (entered, report) = join_sdk(child, cutoff).await?;
        let held_pid = held?;
        ensure!(
            entered.is_err() && report.pid == held_pid && report.forced && !report.status.success(),
            "late SDK accepted"
        );
        let pid = Pid::from_raw(i32::try_from(report.pid)?);
        ensure!(
            kill(pid, None) == Err(Errno::ESRCH),
            "late SDK child remains after joined wait"
        );
        Ok(())
    }
}
