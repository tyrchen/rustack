//! Real official Node runtime consumption with retained server and SDK owners.

use std::{env, ffi::OsString, path::PathBuf};

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

use super::{
    image_sdk,
    process::{self, Profile, Server},
    sdk,
};

pub(super) async fn exercise() -> Result<()> {
    image_sdk::exercise(Profile::Connected, prepare, validate).await
}

async fn prepare(server: &mut Server) -> Result<Vec<OsString>> {
    let image = env::var("RUSTACK_NODE_IMAGE").context("explicit official Node fixture")?;
    server.track_image(&image).await?;
    server.ready().await?;
    sdk::node_compute(&server.endpoint, &image, &process::architecture()?).await?;
    let (owner, engine) = server.observation_identity().await?;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    Ok(vec![
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

fn validate(observed: &Value) -> Result<()> {
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

    use super::super::{image_sdk::join_with_pid_before, runner::Runner};

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
        let (entered, report) = join_with_pid_before(child, cutoff).await?;
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
