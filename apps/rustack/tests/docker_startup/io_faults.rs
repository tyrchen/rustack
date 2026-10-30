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
    custom::Alias,
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
    CreateTimeout,
    DeleteTimeout,
    Pull,
    Negative,
    RuntimeBounds,
    GlobalLimit,
    DefaultLimit,
    MemoryLimit,
    EnvironmentLimit,
    ZipCreate,
    ZipDelete,
}
impl Case {
    fn zip(self) -> bool {
        matches!(self, Self::ZipCreate | Self::ZipDelete)
    }
    fn sources(self) -> Result<[String; 2]> {
        let [first, second] = if self.zip() {
            ["RUSTACK_PROVIDED_ZIP_A", "RUSTACK_PROVIDED_ZIP_B"]
        } else {
            ["RUSTACK_CUSTOM_IMAGE_A", "RUSTACK_CUSTOM_IMAGE_B"]
        };
        Ok([
            env::var(first).context("explicit IO source A")?,
            env::var(second).context("explicit IO source B")?,
        ])
    }
    fn phase(self) -> &'static str {
        match self {
            Self::ZipCreate => "zip_create",
            Self::ZipDelete => "zip_delete",
            Self::Create => "create",
            Self::Delete => "delete",
            Self::Loss => "loss",
            Self::Saturation => "saturation",
            Self::CreateTimeout => "create_timeout",
            Self::DeleteTimeout => "delete_timeout",
            Self::Pull => "pull",
            Self::Negative => "negative",
            Self::RuntimeBounds => "runtime_bounds",
            Self::GlobalLimit => "global_limit",
            Self::DefaultLimit => "default_limit",
            Self::MemoryLimit => "memory_limit",
            Self::EnvironmentLimit => "environment_limit",
        }
    }
    fn expected(self) -> Value {
        match self {
            Self::ZipCreate | Self::ZipDelete | Self::Create | Self::Delete => json!([
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
            Self::CreateTimeout | Self::DeleteTimeout => json!([
                format!("{}-held-after-effect", self.phase()),
                "live-caller-production-io-expiration",
                "physical-retirement-before-release",
                "typed-primary-error",
                "cold-computed-recovery"
            ]),
            Self::Pull => json!([
                "actual-ifMissing-pull",
                "truncated-progress-rejected",
                "failed-update-preserves-active-A",
                "backend-pull-positively-drained",
                "recovered-B-computation"
            ]),
            Self::RuntimeBounds => json!([
                "runtime-http-405-431-413-413-400",
                "same-attempt-computed-result",
                "warm-counter-two",
                "retired-healthy"
            ]),
            Self::GlobalLimit | Self::DefaultLimit => json!([
                format!("{}-positive-capacity-proof", self.phase()),
                "held-delete-retains-charge",
                "physical-retirement-before-recovery",
                "computed-recovery"
            ]),
            Self::MemoryLimit | Self::EnvironmentLimit => json!([
                format!("{}-independent-ceiling", self.phase()),
                "admitted-B-waits-without-allocation",
                "retained-until-old-pair-retired",
                "B-cold-computed-recovery"
            ]),
            Self::Negative => json!([
                "real-wrong-os-rejected",
                "real-wrong-architecture-rejected",
                "protocol-free-bootstrap-timeout",
                "cold-computed-recovery"
            ]),
        }
    }
}

pub(super) async fn exercise(case: Case) -> Result<()> {
    let mut root = Builder::new().prefix("rustack-io-").tempdir_in("/tmp")?;
    root.disable_cleanup(true);
    eprintln!("management IO evidence: {}", root.path().display());
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let sources = case.sources()?;
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
    let mut pull = None;
    let semantic = handoff(
        &mut server,
        &mut pull,
        &mut child,
        root.path(),
        &sources,
        case,
    )
    .await;
    // Keep the proxy transparent and alive through server shutdown on every result.
    let restored = fs::write(root.path().join("restore"), b"restore").await;
    let stopped = match &mut server {
        Some(ordinary) => {
            if case == Case::Loss {
                ordinary.stop_after_transport_loss().await
            } else {
                ordinary.stop(true).await
            }
        }
        None => Ok(()),
    };
    let cache_cleanup = match &mut pull {
        Some(owner) => owner.cleanup_after_pull(root.path()).await,
        None => Ok(()),
    };
    let (audited, logs) = match &server {
        Some(ordinary) => (ordinary.audit().await, check_logs(ordinary).await),
        None => (Ok(()), Ok(())),
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
        cache_cleanup?;
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
    pull: &mut Option<Alias>,
    child: &mut Runner,
    root: &Path,
    sources: &[String; 2],
    case: Case,
) -> Result<()> {
    let pid = child.await_pid().await?;
    ensure!(
        child.await_pid().await? == pid,
        "proxy readiness PID changed"
    );
    let profile = match case {
        Case::ZipCreate | Case::ZipDelete => Profile::ZipControlled,
        Case::CreateTimeout | Case::DeleteTimeout => Profile::Deadline,
        Case::Pull => Profile::Pull,
        Case::Negative => Profile::Negative,
        Case::MemoryLimit => Profile::MemoryPressure,
        Case::DefaultLimit | Case::EnvironmentLimit => Profile::EnvironmentPressure,
        _ => Profile::Controlled,
    };
    let mut ordinary = Server::spawn_with_socket(profile, Some(&root.join("engine.sock"))).await?;
    ordinary
        .root
        .as_mut()
        .context("ordinary evidence guard")?
        .disable_cleanup(true);
    *server = Some(ordinary);
    let ordinary = server.as_mut().context("ordinary server owner")?;
    if case.zip() {
        ordinary
            .track_image(&env::var("RUSTACK_ZIP_BASE_IMAGE")?)
            .await?;
    } else {
        for source in sources {
            ordinary.track_image(source).await?;
        }
    }
    let mut fixtures = Vec::new();
    if case == Case::Negative {
        let selected = architecture()?;
        let opposite = if selected == "arm64" {
            "amd64"
        } else {
            "arm64"
        };
        let mut identities = Vec::new();
        for (name, os, arch) in [
            ("invalid-os", "windows", "amd64"),
            ("invalid-architecture", "linux", opposite),
            ("no-runtime", "linux", selected.as_str()),
        ] {
            let reference = format!("rustack-{name}:{selected}");
            let id = ordinary.track_platform_image(&reference, os, arch).await?;
            ensure!(
                !identities.contains(&id),
                "negative fixtures share content identity"
            );
            identities.push(id.clone());
            let proof = json!({"reference":reference,"id":id,"os":os,"architecture":arch});
            fs::write(
                root.join(format!("{name}-platform.json")),
                serde_json::to_vec(&proof)?,
            )
            .await?;
            fixtures.push(reference);
        }
    }
    if case == Case::Pull {
        let source =
            env::var("RUSTACK_PULL_IMAGE").context("explicit immutable public pull fixture")?;
        let suffix = source
            .strip_prefix("public.ecr.aws/lambda/nodejs@sha256:")
            .context("public Node digest fixture required")?;
        ensure!(
            suffix.len() == 64
                && suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
            "canonical source digest required"
        );
        let id = ordinary.track_image(&source).await?;
        let alternate = format!("public.ecr.aws:443/lambda/nodejs@sha256:{suffix}");
        *pull = Some(Alias::for_pull(ordinary, alternate.clone(), id.clone()).await?);
        fixtures.extend([source, alternate, id]);
    }
    ordinary.ready().await?;
    let (owner, engine) = ordinary.observation_identity().await?;
    let bytes = serde_json::to_vec(
        &json!({"endpoint":ordinary.endpoint,"owner":owner,"engine":engine,"fixtures":fixtures}),
    )?;
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
