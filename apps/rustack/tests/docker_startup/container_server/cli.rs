//! Direct bounded CLI observers; every child is joined before interpretation.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use rustack_core::json::{JsonLimits, from_slice};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use tokio::time::Instant;
use uuid::Uuid;

use super::super::runner::{Report, Runner};

#[derive(Debug)]
pub(super) struct Cli {
    program: PathBuf,
    socket: String,
    pub engine: Uuid,
}

impl Cli {
    pub async fn connect(program: PathBuf, socket: String, cutoff: Instant) -> Result<Self> {
        let bytes = command(&program, &socket, &info_arguments(), cutoff).await?;
        let engine = uuid(&decode::<String>(&bytes.stdout)?)?;
        Ok(Self {
            program,
            socket,
            engine,
        })
    }

    pub async fn run(&self, args: &[String], cutoff: Instant) -> Result<Vec<u8>> {
        Ok(self.output(args, cutoff).await?.stdout)
    }

    pub async fn output(&self, args: &[String], cutoff: Instant) -> Result<Report> {
        let at = command_deadline(args, cutoff);
        self.require_engine(at).await?;
        let bytes = command(&self.program, &self.socket, args, at).await?;
        self.require_engine(at).await?;
        ensure!(Instant::now() < at, "late Docker CLI observation");
        Ok(bytes)
    }

    pub async fn json<T: DeserializeOwned>(&self, args: &[String], cutoff: Instant) -> Result<T> {
        decode(&self.run(args, cutoff).await?)
    }

    async fn require_engine(&self, cutoff: Instant) -> Result<()> {
        let bytes = command(&self.program, &self.socket, &info_arguments(), cutoff).await?;
        ensure!(
            uuid(&decode::<String>(&bytes.stdout)?)? == self.engine,
            "fixture Docker daemon identity changed"
        );
        Ok(())
    }
}

pub(super) fn decode<T: DeserializeOwned>(raw: &[u8]) -> Result<T> {
    // Docker configuration paths/env entries require4KiB strings; the whole
    // response, depth, keys and aggregate members retain independent caps.
    Ok(from_slice(
        raw,
        JsonLimits::new(32768, 16, 2048, 4096, 256)?,
    )?)
}

pub(super) fn uuid(text: &str) -> Result<Uuid> {
    ensure!(
        text.len() == 36
            && text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || b == b'-'),
        "invalid fixture UUID grammar"
    );
    let id = Uuid::parse_str(text)?;
    ensure!(
        !id.is_nil() && id.to_string() == text,
        "noncanonical fixture UUID"
    );
    Ok(id)
}

pub(super) fn container_id(text: &str) -> Result<String> {
    ensure!(
        text.len() == 64
            && text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "invalid full container ID"
    );
    Ok(text.to_owned())
}

pub(super) fn arguments(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_owned()).collect()
}

fn info_arguments() -> Vec<String> {
    arguments(&["info", "--format", "{{json .ID}}"])
}

fn command_deadline(args: &[String], cutoff: Instant) -> Instant {
    let seconds = if args.first().is_some_and(|arg| arg == "wait") {
        70
    } else {
        60
    };
    cutoff.min(Instant::now() + Duration::from_secs(seconds))
}

async fn command(program: &Path, socket: &str, args: &[String], cutoff: Instant) -> Result<Report> {
    let cutoff = command_deadline(args, cutoff);
    ensure!(
        Instant::now() < cutoff,
        "Docker CLI deadline expired before dispatch"
    );
    ensure!(
        (1..=96).contains(&args.len())
            && args
                .iter()
                .all(|arg| arg.len() <= 4096 && !arg.chars().any(char::is_control)),
        "Docker CLI argv boundary"
    );
    let mut invocation = vec![OsString::from("--host"), OsString::from(socket)];
    invocation.extend(args.iter().map(OsString::from));
    let child = Runner::spawn(program, &invocation)?;
    let report = child
        .wait(cutoff)
        .await
        .context("join exact Docker CLI child")?;
    // stderr may contain daemon-selected text. Preserve only its digest in
    // diagnostics, never a response body or a private env-file value.
    ensure!(
        !report.forced && report.status.success(),
        "Docker CLI failed (exit {:?}, forced {}, stderr SHA256/base64 {})",
        report.status.code(),
        report.forced,
        STANDARD.encode(Sha256::digest(&report.stderr))
    );
    ensure!(
        Instant::now() < cutoff,
        "Docker CLI result expired after join"
    );
    Ok(report)
}
