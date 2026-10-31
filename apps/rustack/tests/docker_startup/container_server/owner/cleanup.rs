//! Shutdown receipt precedes namespace audit and exact stopped-resource removal.

use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::json;
use tokio::time::Instant;

use super::{
    super::{
        cli::{self, arguments},
        policy,
    },
    ExpectedExit, Owner, Resources, SOURCE, Source,
};

#[derive(Debug, Deserialize)]
struct ImageRow {
    #[serde(rename = "Repository")]
    repository: String,
}

impl Owner {
    pub(crate) async fn cleanup(&mut self) -> Result<()> {
        let cutoff = Instant::now() + Duration::from_secs(180);
        if matches!(self.resources, Resources::Server { .. }) {
            self.stop_server(cutoff).await?;
            ensure!(
                !self.uncertain,
                "uncertain server mutation remains retained"
            );
            if self.identity.is_none() {
                self.capture_identity(cutoff).await?;
            }
            self.audit_runtime(cutoff).await?;
            self.remove_server(cutoff).await?;
        }
        ensure!(
            !self.uncertain,
            "uncertain server/volume creation remains retained"
        );
        if let Resources::Volume(_) = self.resources {
            self.remove_volume(cutoff).await?;
        }
        self.verify_sources(cutoff).await?;
        self.write(
            "cleanup.json",
            &serde_json::to_vec(&json!({
                "schemaVersion":1,"serverAbsent":true,"volumeAbsent":true,
                "uncertain":false,"sourcesPreserved":self.sources,
                "engine":self.expected.engine.to_string()
            }))?,
            cutoff,
        )
        .await?;
        Ok(())
    }

    async fn stop_server(&mut self, cutoff: Instant) -> Result<()> {
        let at = cutoff.min(Instant::now() + Duration::from_secs(70));
        let view = self.inspect_server(at).await?;
        let id = view.id.as_deref().context("owned server shutdown ID")?;
        let state = view.state.as_ref().context("server shutdown state")?;
        if state.running == Some(true) {
            let prior = self.uncertain;
            self.uncertain = true;
            let stopped = self
                .cli
                .run(&arguments(&["kill", "--signal", "TERM", id]), at)
                .await?;
            ensure!(
                stopped == format!("{id}\n").as_bytes(),
                "server signal reply differs"
            );
            self.uncertain = prior;
        }
        let output = self.cli.run(&arguments(&["wait", id]), at).await?;
        let text = std::str::from_utf8(&output)?
            .strip_suffix('\n')
            .context("server wait newline")?;
        let code: i64 = text.parse()?;
        ensure!(
            (0..=255).contains(&code) && code.to_string() == text,
            "invalid server exit receipt"
        );
        let expected = match self.expected_exit {
            ExpectedExit::Success => 0,
            ExpectedExit::EndpointRejected(code) => code,
        };
        let actual = self.inspect_server(at).await?;
        let state = actual.state.as_ref().context("server final state")?;
        ensure!(
            code == expected
                && state.running == Some(false)
                && state.exit_code == Some(code)
                && state.oom_killed == Some(false)
                && state.dead != Some(true),
            "unexpected or unjoined server shutdown"
        );
        let logs = self.logs(cutoff).await?;
        self.write("server.log", &logs, cutoff).await?;
        self.write(
            "shutdown.json",
            &serde_json::to_vec(&json!({"id":id,"exitCode":code,
            "engine":self.expected.engine.to_string(),"physicallyStopped":true}))?,
            cutoff,
        )
        .await?;
        Ok(())
    }

    pub(in crate::container_server) async fn runtime_ids(
        &self,
        cutoff: Instant,
    ) -> Result<Vec<String>> {
        let identity = self.identity()?;
        let raw = self
            .cli
            .run(
                &arguments(&[
                    "container",
                    "ls",
                    "--all",
                    "--no-trunc",
                    "--filter",
                    &format!("label=rustack.lambda.ownerId={}", identity.owner),
                    "--format",
                    "{{json .ID}}",
                ]),
                cutoff,
            )
            .await?;
        let text = std::str::from_utf8(&raw)?;
        let mut ids = Vec::new();
        for line in text.lines() {
            ensure!(ids.len() < 64, "runtime container inventory count");
            ids.push(cli::container_id(&cli::decode::<String>(line.as_bytes())?)?);
        }
        Ok(ids)
    }

    async fn audit_runtime(&self, cutoff: Instant) -> Result<()> {
        ensure!(
            self.runtime_ids(cutoff).await?.is_empty(),
            "production server left runtime containers"
        );
        let identity = self.identity()?;
        let volumes = self
            .cli
            .run(
                &arguments(&[
                    "volume",
                    "ls",
                    "--filter",
                    &format!("label=rustack.lambda.ownerId={}", identity.owner),
                    "--format",
                    "{{json .Name}}",
                ]),
                cutoff,
            )
            .await?;
        ensure!(
            volumes.iter().all(u8::is_ascii_whitespace),
            "production server left runtime volumes"
        );
        let raw = self
            .cli
            .run(
                &arguments(&[
                    "image",
                    "ls",
                    "--all",
                    "--no-trunc",
                    "--format",
                    "{{json .}}",
                ]),
                cutoff,
            )
            .await?;
        let text = std::str::from_utf8(&raw)?;
        let prefix = format!("rustack-internal/lambda-{}/", identity.owner.simple());
        for (index, line) in text.lines().enumerate() {
            ensure!(index < 4096, "complete image inventory count");
            let row: ImageRow = cli::decode(line.as_bytes())?;
            ensure!(
                row.repository.len() <= 1024 && !row.repository.chars().any(char::is_control),
                "image repository inventory bound"
            );
            let familiar = canonical(&row.repository);
            ensure!(
                !familiar.starts_with(&prefix),
                "production server left private image aliases"
            );
        }
        Ok(())
    }

    async fn remove_server(&mut self, cutoff: Instant) -> Result<()> {
        let view = self.inspect_server(cutoff).await?;
        let state = view.state.as_ref().context("server removal state")?;
        ensure!(
            state.running == Some(false),
            "refuse removal of a running server"
        );
        let Resources::Server { volume, id } = &self.resources else {
            anyhow::bail!("server removal authority missing");
        };
        let data = volume.clone();
        let id = id.clone();
        self.uncertain = true;
        let removed = self
            .cli
            .run(&arguments(&["container", "rm", &id]), cutoff)
            .await?;
        ensure!(
            removed == format!("{id}\n").as_bytes(),
            "server removal reply differs"
        );
        let remaining = self
            .cli
            .run(
                &arguments(&[
                    "container",
                    "ls",
                    "--all",
                    "--no-trunc",
                    "--filter",
                    &format!("id={id}"),
                    "--format",
                    "{{json .ID}}",
                ]),
                cutoff,
            )
            .await?;
        ensure!(
            remaining.iter().all(u8::is_ascii_whitespace),
            "server container remains after removal"
        );
        self.resources = Resources::Volume(data);
        self.uncertain = false;
        Ok(())
    }

    async fn remove_volume(&mut self, cutoff: Instant) -> Result<()> {
        let Resources::Volume(recorded) = &self.resources else {
            anyhow::bail!("server volume removal authority missing");
        };
        let volume = self.inspect_volume(cutoff).await?;
        policy::volume(&volume, &self.expected)?;
        ensure!(
            volume.mountpoint == recorded.mountpoint,
            "server volume mountpoint changed"
        );
        let attachments = self
            .cli
            .run(
                &arguments(&[
                    "container",
                    "ls",
                    "--all",
                    "--no-trunc",
                    "--filter",
                    &format!("volume={}", self.expected.volume),
                    "--format",
                    "{{json .ID}}",
                ]),
                cutoff,
            )
            .await?;
        ensure!(
            attachments.iter().all(u8::is_ascii_whitespace),
            "server volume still has an attachment"
        );
        self.uncertain = true;
        let removed = self
            .cli
            .run(&arguments(&["volume", "rm", &self.expected.volume]), cutoff)
            .await?;
        ensure!(
            removed == format!("{}\n", self.expected.volume).as_bytes(),
            "server volume removal reply differs"
        );
        let remaining = self
            .cli
            .run(
                &arguments(&[
                    "volume",
                    "ls",
                    "--filter",
                    &format!("label={}={}", policy::NONCE_LABEL, self.expected.nonce),
                    "--format",
                    "{{json .Name}}",
                ]),
                cutoff,
            )
            .await?;
        ensure!(
            remaining.iter().all(u8::is_ascii_whitespace),
            "server task volume remains after removal"
        );
        self.resources = Resources::Intent;
        self.uncertain = false;
        Ok(())
    }

    async fn verify_sources(&self, cutoff: Instant) -> Result<()> {
        for (reference, expected) in &self.sources {
            let source: Source = self
                .cli
                .json(
                    &arguments(&["image", "inspect", "--format", SOURCE, reference]),
                    cutoff,
                )
                .await?;
            ensure!(
                source.id == *expected
                    && source.os == "linux"
                    && source.architecture == self.inputs.architecture,
                "server fixture source image changed"
            );
        }
        Ok(())
    }
}

fn canonical(mut value: &str) -> &str {
    for prefix in ["docker.io/", "index.docker.io/", "registry-1.docker.io/"] {
        if let Some(tail) = value.strip_prefix(prefix) {
            value = tail;
            break;
        }
    }
    value.strip_prefix("library/").unwrap_or(value)
}
