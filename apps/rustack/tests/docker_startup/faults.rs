//! Actual failing user code, signed caller loss and bounded accepted-work shutdown.

use std::{env, ffi::OsString, path::PathBuf, time::Duration};

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use tokio::{fs, io::AsyncReadExt, time::Instant};

use super::{
    process::{self, Profile, Server},
    runner,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Case {
    Faults,
    Initializing,
    Busy,
    Saturation,
}
impl Case {
    pub(super) fn profile(self) -> Profile {
        if self == Self::Faults {
            Profile::Connected
        } else {
            Profile::Controlled
        }
    }
    fn phase(self) -> &'static str {
        match self {
            Self::Faults => "faults",
            Self::Initializing => "initializing",
            Self::Busy => "busy",
            Self::Saturation => "saturation",
        }
    }
    fn expected(self) -> Value {
        match self {
            Self::Faults => json!([
                "init-error",
                "prepoll-exit",
                "bootstrap-timeout",
                "handler-exit",
                "cgroup-oom",
                "handler-timeout"
            ]),
            Self::Initializing => json!([
                "reserved-zero-no-allocation",
                "reserved-range-atomic",
                "held-reserved-one",
                "initializing-disconnect-removal-before-expiry",
                "cold-recovery"
            ]),
            Self::Busy => json!([
                "reserved-zero-no-allocation",
                "reserved-range-atomic",
                "held-reserved-one",
                "busy-disconnect-removal-before-expiry",
                "cold-recovery"
            ]),
            Self::Saturation => json!([
                "impossible-memory-429",
                "event-capacity-128",
                "event-129-throttled",
                "two-pairs-384MiB",
                "held-backlog-shutdown-cutoff"
            ]),
        }
    }
}

pub(super) async fn exercise(server: &mut Server, case: Case) -> Result<()> {
    server.ready().await?;
    let source = env::var("RUSTACK_CUSTOM_IMAGE_A").context("explicit fault fixture source")?;
    server.track_image(&source).await?;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let architecture = if process::architecture()? == "arm64" {
        "arm64"
    } else {
        "x86_64"
    };
    let (owner, engine) = server.observation_identity().await?;
    let arguments = [
        root.join("tests/workflows/lambda_fault_sdk.py")
            .into_os_string(),
        OsString::from(&server.endpoint),
        OsString::from(source),
        OsString::from(architecture),
        OsString::from(case.phase()),
        env::var_os("RUSTACK_RUNTIME_GATEWAY_HOST").context("explicit hold collector host")?,
        env::var_os("RUSTACK_RUNTIME_DOCKER_SOCKET").context("explicit metadata socket")?,
        OsString::from(owner),
        OsString::from(engine),
    ];
    let mut child = runner::Runner::spawn(&runner::interpreter()?, &arguments)?;
    let entered = child.await_pid().await;
    let gate = if case == Case::Saturation && entered.is_ok() {
        child
            .await_pid_before(Instant::now() + Duration::from_secs(180))
            .await
            .map(Some)
    } else {
        Ok(None)
    };
    let shutdown_started = Instant::now();
    let stopped = if case == Case::Saturation || entered.is_err() || gate.is_err() {
        server.stop(case != Case::Saturation).await
    } else {
        Ok(())
    };
    let shutdown_elapsed = shutdown_started.elapsed();
    let deadline = if entered.is_ok() && gate.is_ok() {
        Instant::now() + Duration::from_secs(180)
    } else {
        Instant::now()
    };
    // Every barrier and server-stop outcome is saved before joining the same child.
    let report = child.wait(deadline).await?;
    let diagnostics = String::from_utf8_lossy(&report.stderr);
    let pid = entered.with_context(|| format!("fault SDK entry failed: {diagnostics}"))?;
    let ready = gate.with_context(|| format!("fault SDK shutdown gate failed: {diagnostics}"))?;
    if let Some(ready) = ready {
        ensure!(ready == pid, "shutdown gate PID changed");
    }
    stopped?;
    if case == Case::Saturation {
        ensure!(
            shutdown_elapsed >= Duration::from_secs(25),
            "server failed before the declared drain cutoff"
        );
        let file = fs::File::open(server.path()?.join("server.log")).await?;
        let mut bytes = Vec::new();
        file.take(32769).read_to_end(&mut bytes).await?;
        ensure!(
            bytes.len() <= 32768
                && String::from_utf8_lossy(&bytes)
                    .contains("Lambda quiesce did not confirm all invocation cleanup"),
            "wrong shutdown failure instead of the declared drain cutoff"
        );
    }
    ensure!(
        report.pid == pid && !report.forced && report.status.success(),
        "actual fault SDK process failed: {diagnostics}"
    );
    let observed: Value = serde_json::from_slice(&report.stdout)?;
    ensure!(
        observed["schemaVersion"] == 1 && observed["cases"] == case.expected(),
        "fault semantic inventory differs"
    );
    Ok(())
}

pub(super) async fn check_logs(server: &Server) -> Result<()> {
    let file = fs::File::open(server.path()?.join("server.log")).await?;
    let mut bytes = Vec::new();
    file.take(32769).read_to_end(&mut bytes).await?;
    ensure!(bytes.len() <= 32768, "fault log evidence exceeds cap");
    ensure!(
        !String::from_utf8_lossy(&bytes).contains("fixture-init-payload-sentinel")
            && !String::from_utf8_lossy(&bytes).contains("runtime-boundary-secret"),
        "fixture payload/header leaked into diagnostics"
    );
    Ok(())
}
