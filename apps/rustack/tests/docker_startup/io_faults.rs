//! A joined real-daemon proxy remains live through ordinary server cleanup.

use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use tempfile::Builder;
use tokio::{fs, time::Instant};

use super::{
    faults::check_logs,
    process::{Profile, Server, architecture},
    runner::{Runner, interpreter},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Case {
    Create,
    Delete,
    Loss,
    Saturation,
}
impl Case {
    fn phase(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Delete => "delete",
            Self::Loss => "loss",
            Self::Saturation => "saturation",
        }
    }
    fn expected(self) -> Value {
        match self {
            Self::Create | Self::Delete => json!([
                format!("{}-held-real-io", self.phase()),
                "retained-reserved-slot",
                "caller-disconnected",
                "post-release-physical-retirement",
                "cold-computed-recovery"
            ]),
            Self::Loss => json!([
                "image-management-ack-lost",
                "failed-update-preserves-active-A",
                "published-A-preserved",
                "recovered-B-computation",
                "missing-source-never-active",
                "full-loss-revokes-readiness"
            ]),
            Self::Saturation => json!([
                "two-held-pending-deployments",
                "third-rejected-before-insertion",
                "deleted-preparation-retains-capacity",
                "deleted-job-never-resurrects",
                "capacity-recovery-computation"
            ]),
        }
    }
}

pub(super) async fn exercise(case: Case) -> Result<()> {
    let mut root = Builder::new().prefix("rustack-io-").tempdir_in("/tmp")?;
    root.disable_cleanup(true);
    eprintln!("management IO evidence: {}", root.path().display());
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let sources = [
        env::var("RUSTACK_CUSTOM_IMAGE_A").context("explicit IO image A")?,
        env::var("RUSTACK_CUSTOM_IMAGE_B").context("explicit IO image B")?,
    ];
    let architecture = if architecture()? == "arm64" {
        "arm64"
    } else {
        "x86_64"
    };
    let arguments = [
        repository
            .join("tests/workflows/lambda_io_fault_sdk.py")
            .into_os_string(),
        root.path().as_os_str().to_owned(),
        env::var_os("RUSTACK_RUNTIME_DOCKER_SOCKET").context("explicit actual daemon")?,
        OsString::from(&sources[0]),
        OsString::from(&sources[1]),
        OsString::from(architecture),
        OsString::from(case.phase()),
        env::var_os("RUSTACK_RUNTIME_GATEWAY_HOST").context("explicit init hold host")?,
    ];
    let mut child = Runner::spawn(&interpreter()?, &arguments)?;
    let mut server = None;
    let semantic = handoff(&mut server, &mut child, root.path(), &sources).await;
    // Keep the proxy transparent and alive through server shutdown on every result.
    let restored = fs::write(root.path().join("restore"), b"restore").await;
    let (stopped, audited, logs) = match &mut server {
        Some(ordinary) => (
            if case == Case::Loss {
                ordinary.stop_after_transport_loss().await
            } else {
                ordinary.stop(true).await
            },
            ordinary.audit().await,
            check_logs(ordinary).await,
        ),
        None => (Ok(()), Ok(()), Ok(())),
    };
    let stop = fs::write(root.path().join("stop"), b"stop").await;
    let joined = child.wait(Instant::now() + Duration::from_secs(15)).await;
    let result = (|| {
        let report = joined?;
        let diagnostic = String::from_utf8_lossy(&report.stderr);
        semantic.with_context(|| format!("IO SDK handoff failed: {diagnostic}"))?;
        restored?;
        stop?;
        ensure!(
            !report.forced && report.status.success(),
            "IO SDK failed: {diagnostic}"
        );
        stopped?;
        audited?;
        logs?;
        let observed: Value = serde_json::from_slice(&report.stdout)?;
        ensure!(
            observed["schemaVersion"] == 1 && observed["cases"] == case.expected(),
            "IO semantic inventory differs"
        );
        Ok::<(), anyhow::Error>(())
    })();
    if result.is_err() {
        if let Some(ordinary) = &mut server {
            ordinary.retain_on_error().await;
        }
    } else {
        root.disable_cleanup(false);
        if let Some(ordinary) = &mut server {
            ordinary
                .root
                .as_mut()
                .context("ordinary evidence guard")?
                .disable_cleanup(false);
        }
    }
    result
}

async fn handoff(
    server: &mut Option<Server>,
    child: &mut Runner,
    root: &Path,
    sources: &[String; 2],
) -> Result<()> {
    let pid = child.await_pid().await?;
    ensure!(
        child.await_pid().await? == pid,
        "proxy readiness PID changed"
    );
    let mut ordinary =
        Server::spawn_with_socket(Profile::Controlled, Some(&root.join("engine.sock"))).await?;
    ordinary
        .root
        .as_mut()
        .context("ordinary evidence guard")?
        .disable_cleanup(true);
    *server = Some(ordinary);
    let ordinary = server.as_mut().context("ordinary server owner")?;
    for source in sources {
        ordinary.track_image(source).await?;
    }
    ordinary.ready().await?;
    let (owner, engine) = ordinary.observation_identity().await?;
    let bytes =
        serde_json::to_vec(&json!({"endpoint":ordinary.endpoint,"owner":owner,"engine":engine}))?;
    ensure!(bytes.len() <= 2048, "proxy handoff byte cap");
    fs::write(root.join("config.pending"), bytes).await?;
    fs::rename(root.join("config.pending"), root.join("config.json")).await?;
    ensure!(
        child
            .await_pid_before(Instant::now() + Duration::from_secs(180))
            .await?
            == pid,
        "SDK completion PID changed"
    );
    Ok::<(), anyhow::Error>(())
}
