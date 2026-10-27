//! Custom-RIC contracts and a separately owned movable operator-side source alias.

use std::{env, ffi::OsString, future::pending, path::PathBuf, time::Duration};

use anyhow::{Context, Result, ensure};
use rustack_lambda_core::{
    artifact::ImageReference,
    docker::{DockerError, DockerWire},
};
use serde_json::{Value, json};
use tokio::{fs, io::AsyncReadExt, runtime::Handle, task::JoinHandle, time::Instant};
use uuid::Uuid;

use super::{
    process::{self, Server},
    runner, sdk,
};

#[derive(Debug)]
struct Alias {
    name: String,
    ids: [String; 2],
    pending: Option<String>,
    executable: PathBuf,
    record: PathBuf,
    command: Option<JoinHandle<Result<runner::Report>>>,
    runtime: Handle,
}
// Observer cancellation leaves the exact process waiter running. This captured-
// runtime reaper joins it; the evidence directory keeps any uncertain tag intent.
impl Drop for Alias {
    fn drop(&mut self) {
        if let Some(command) = self.command.take() {
            self.runtime.spawn(async move {
                if command.await.is_err() {
                    eprintln!(
                        "custom alias command owner failed; retained intent requires \
                         reconciliation"
                    );
                }
            });
        }
    }
}
impl Alias {
    async fn new(server: &Server, ids: [String; 2]) -> Result<Self> {
        let executable =
            PathBuf::from(env::var_os("RUSTACK_DOCKER_BIN").context("explicit Docker CLI")?);
        ensure!(executable.is_absolute(), "Docker CLI must be absolute");
        let nonce = Uuid::new_v4().simple().to_string();
        let owner = Self {
            name: format!("rustack-test-custom-ric-{nonce}:mutable"),
            ids,
            pending: None,
            executable,
            record: server.path()?.join(format!("source-alias-{nonce}.json")),
            command: None,
            runtime: Handle::current(),
        };
        ensure!(
            owner.inspect().await?.is_none(),
            "fixture source alias already exists"
        );
        owner.save().await?;
        Ok(owner)
    }
    async fn inspect(&self) -> Result<Option<String>> {
        let at = Instant::now() + Duration::from_secs(60);
        let mut wire = DockerWire::connect(process::socket()?.parse()?, at).await?;
        let reference: ImageReference = self.name.parse()?;
        let result = wire.port().inspect_image(&reference, at).await;
        let stopped = wire.shutdown().await;
        stopped?;
        match result {
            Ok(image) => Ok(Some(image.id.context("source alias identity")?)),
            Err(DockerError::Status(status)) if status.as_u16() == 404 => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
    async fn save(&self) -> Result<()> {
        let bytes = serde_json::to_vec(
            &json!({"alias":self.name,"sourceIds":self.ids,"pendingTarget":self.pending}),
        )?;
        ensure!(bytes.len() <= 4096, "source intent exceeds limit");
        let pending = self.record.with_extension("pending");
        fs::write(&pending, bytes).await?;
        // Keep the preceding complete identity visible if this observer drops
        // during writing. Each Alias has its own immutable nonce/path pair.
        fs::rename(pending, &self.record).await?;
        Ok(())
    }
    async fn command(&mut self, arguments: &[&str]) -> Result<()> {
        let mut args = vec![OsString::from("--host"), OsString::from(process::socket()?)];
        args.extend(arguments.iter().map(OsString::from));
        ensure!(self.command.is_none(), "alias command already running");
        let child = runner::Runner::spawn(&self.executable, &args)?;
        self.command = Some(tokio::spawn(async move {
            child.wait(Instant::now() + Duration::from_secs(60)).await
        }));
        let outcome = self
            .command
            .as_mut()
            .context("retained alias command")?
            .await;
        self.command.take();
        let report = outcome.context("alias command owner join")??;
        ensure!(
            !report.forced && report.status.success(),
            "owned alias command failed: {}",
            String::from_utf8_lossy(&report.stderr)
        );
        Ok(())
    }
    async fn select(&mut self, target: usize) -> Result<()> {
        let id = self.ids.get(target).context("source image index")?.clone();
        let current = self.inspect().await?;
        if target == 0 {
            ensure!(current.is_none(), "alias was concurrently created");
        } else {
            ensure!(
                current.as_ref() == self.ids.first(),
                "alias was concurrently retargeted"
            );
        }
        self.pending = Some(id.clone());
        self.save().await?;
        let name = self.name.clone();
        self.command(&["image", "tag", &id, &name]).await?;
        ensure!(
            self.inspect().await?.as_ref() == Some(&id),
            "tag target did not settle"
        );
        self.pending = None;
        self.save().await
    }
    async fn cleanup(&mut self) -> Result<()> {
        let current = self.inspect().await?;
        if let Some(pending) = &self.pending {
            ensure!(
                current.as_ref() == Some(pending),
                "uncertain alias mutation retained for reconciliation"
            );
            self.pending = None;
        }
        if let Some(current) = current {
            ensure!(
                self.ids.contains(&current),
                "foreign alias target preserved"
            );
            let name = self.name.clone();
            self.command(&["image", "rm", "--no-prune", &name]).await?;
        }
        ensure!(
            self.inspect().await?.is_none(),
            "fixture alias survived cleanup"
        );
        self.save().await
    }
}

async fn python(server: &Server, image: &str, phase: &str, name: &str, tag: &str) -> Result<Value> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let architecture = if process::architecture()? == "arm64" {
        "arm64"
    } else {
        "x86_64"
    };
    let args = [
        root.join("tests/workflows/custom_ric_sdk.py")
            .into_os_string(),
        OsString::from(&server.endpoint),
        OsString::from(image),
        OsString::from(architecture),
        OsString::from(phase),
        env::var_os("RUSTACK_RUNTIME_GATEWAY_HOST").context("explicit collector host")?,
        OsString::from(name),
        OsString::from(tag),
    ];
    let mut child = runner::Runner::spawn(&runner::interpreter()?, &args)?;
    let barrier = child.await_pid().await;
    let at = if barrier.is_ok() {
        Instant::now() + Duration::from_secs(360)
    } else {
        Instant::now()
    };
    let report = child.wait(at).await?;
    let pid = barrier?;
    ensure!(
        report.pid == pid && !report.forced && report.status.success(),
        "custom RIC SDK phase failed: {}",
        String::from_utf8_lossy(&report.stderr)
    );
    let output: Value = serde_json::from_slice(&report.stdout)?;
    ensure!(
        output["schemaVersion"] == 1,
        "missing custom RIC report schema"
    );
    Ok(output)
}

async fn abandon_controls(server: &Server, ids: &[String; 2]) -> Result<()> {
    for panic_after_tag in [false, true] {
        let mut owner = Alias::new(server, ids.clone()).await?;
        owner.select(0).await?;
        let (name, record, executable) = (
            owner.name.clone(),
            owner.record.clone(),
            owner.executable.clone(),
        );
        let (entered, observed) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _ = entered.send(());
            assert!(
                !panic_after_tag,
                "deliberate fixture panic after committed owned tag"
            );
            pending::<()>().await;
            drop(owner);
        });
        observed.await.context("alias abandonment control entry")?;
        if !panic_after_tag {
            task.abort();
        }
        let failure = task
            .await
            .expect_err("deliberately abandoned alias owner must fail");
        ensure!(
            if panic_after_tag {
                failure.is_panic()
            } else {
                failure.is_cancelled()
            },
            "wrong fixture owner failure"
        );
        let file = fs::File::open(&record).await?;
        let mut raw = Vec::new();
        file.take(4097).read_to_end(&mut raw).await?;
        ensure!(raw.len() <= 4096, "retained alias record exceeds cap");
        let saved: Value = serde_json::from_slice(&raw)?;
        ensure!(
            saved["alias"] == name
                && saved["sourceIds"] == json!(ids)
                && saved["pendingTarget"].is_null(),
            "abandoned fixture lost exact ownership evidence"
        );
        // Authority transfers only after the old task has joined and its Alias
        // dropped; this controlled fully-committed tag has no pending command.
        let mut recovered = Alias {
            name,
            ids: ids.clone(),
            pending: None,
            executable,
            record,
            command: None,
            runtime: Handle::current(),
        };
        ensure!(
            recovered.inspect().await?.as_ref() == ids.first(),
            "abandoned tag identity differs"
        );
        recovered.cleanup().await?;
    }
    Ok(())
}

pub(super) async fn exercise(server: &mut Server) -> Result<()> {
    server.ready().await?;
    let a = env::var("RUSTACK_CUSTOM_IMAGE_A").context("explicit custom source A")?;
    let b = env::var("RUSTACK_CUSTOM_IMAGE_B").context("explicit custom source B")?;
    let ids = [server.track_image(&a).await?, server.track_image(&b).await?];
    ensure!(
        ids.first() != ids.get(1),
        "custom fixture images are identical"
    );
    sdk::custom_compute(&server.endpoint, &a, &process::architecture()?).await?;
    let mut alias = Alias::new(server, ids.clone()).await?;
    let result = async {
        let report = python(server, &a, "contracts", "", "").await?;
        ensure!(
            report["cases"]
                == json!([
                    "custom-ric-compute-error",
                    "imageconfig-inherit-clear-replace",
                    "literal-argv",
                    "active-cross-invocation-rejection"
                ]),
            "custom RIC contract inventory differs"
        );
        alias.select(0).await?;
        let name = format!("MovingImage{}", Uuid::new_v4().simple());
        let report = python(server, &a, "deploy", &name, &alias.name).await?;
        ensure!(
            report["cases"] == json!(["published-A"]),
            "missing publication evidence"
        );
        alias.select(1).await?;
        let report = python(server, &a, "update", &name, &alias.name).await?;
        ensure!(
            report["coldRevisionCounters"] == json!([1, 1]),
            "missing cold published/latest image evidence"
        );
        ensure!(
            report["cases"]
                == json!([
                    "tag-move-retains-A",
                    "update-selects-B",
                    "published-alias-isolation",
                    "published-inflight-A-completes-after-B-update"
                ]),
            "immutable revision evidence differs"
        );
        Ok::<(), anyhow::Error>(())
    }
    .await;
    let cleanup = alias.cleanup().await;
    if cleanup.is_err() && result.is_err() {
        eprintln!("custom source alias cleanup remains unconfirmed; primary SDK failure retained");
    }
    result?;
    cleanup?;
    abandon_controls(server, &ids).await
}
