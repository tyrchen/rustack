//! Bounded Runtime API IO and credential-free atomic fixture evidence.

use std::{env, io::Read, net::SocketAddr, path::PathBuf, time::Duration};

use anyhow::{Context, Result, ensure};
use serde::Serialize;
use tokio::{fs, task::spawn_blocking, time::timeout};
use typed_builder::TypedBuilder;
use ureq::Agent;
use uuid::Uuid;

#[derive(Debug)]
pub(super) struct Wire {
    agent: Agent,
    api: String,
    markers: PathBuf,
    gates: PathBuf,
}
#[derive(Debug, Serialize, TypedBuilder)]
#[serde(rename_all = "camelCase")]
struct Evidence<'a> {
    request_id: &'a str,
    pid: u32,
    sdk: &'static str,
    stage: &'static str,
    retry: u32,
    heartbeats: u32,
    history_safe: bool,
    closed_token: bool,
}
impl Wire {
    pub(super) async fn new() -> Result<Self> {
        let api = env::var("AWS_LAMBDA_RUNTIME_API")?;
        let address: SocketAddr = api.parse()?;
        ensure!(
            address.ip().is_loopback() && address.port() != 0,
            "invalid Runtime API address"
        );
        let markers =
            PathBuf::from(env::var_os("CALLBACK_MARKER_DIR").context("marker directory")?);
        let gates = PathBuf::from(env::var_os("CALLBACK_GATE_DIR").context("gate directory")?);
        ensure!(
            timeout(Duration::from_secs(2), fs::metadata(&markers))
                .await??
                .is_dir()
                && timeout(Duration::from_secs(2), fs::metadata(&gates))
                    .await??
                    .is_dir(),
            "missing fixture directories"
        );
        let processes =
            PathBuf::from(env::var_os("PROCESS_MARKER_DIR").context("process directory")?);
        let pid = std::process::id().to_string();
        timeout(
            Duration::from_secs(2),
            fs::write(processes.join(format!("pid-{pid}")), pid),
        )
        .await??;
        Ok(Self {
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(60))
                .build(),
            api,
            markers,
            gates,
        })
    }
    pub(super) async fn next(&self) -> Result<(String, Vec<u8>)> {
        let agent = self.agent.clone();
        let url = format!("http://{}/2018-06-01/runtime/invocation/next", self.api);
        spawn_blocking(move || {
            let response = agent.get(&url).call()?;
            let id = response
                .header("Lambda-Runtime-Aws-Request-Id")
                .context("request identity")?
                .to_owned();
            let parsed = Uuid::parse_str(&id)?;
            ensure!(
                id.len() == 36 && parsed.to_string() == id,
                "invalid request identity"
            );
            let mut body = Vec::new();
            response
                .into_reader()
                .take(524_289)
                .read_to_end(&mut body)?;
            ensure!(body.len() <= 524_288, "fixture body exceeded");
            Ok((id, body))
        })
        .await
        .context("Runtime API input worker")?
    }
    pub(super) async fn post(&self, request: &str, failure: bool) -> Result<()> {
        let (route, body) = if failure {
            (
                "error",
                r#"{"errorType":"CallbackInvocationFailure","errorMessage":"actual fixture error"}"#,
            )
        } else {
            ("response", r#"{"ignored":"synchronous body"}"#)
        };
        let agent = self.agent.clone();
        let url = format!(
            "http://{}/2018-06-01/runtime/invocation/{request}/{route}",
            self.api
        );
        spawn_blocking(move || {
            agent
                .post(&url)
                .send_string(body)
                .map(|_| ())
                .context("Runtime API output")
        })
        .await
        .context("Runtime API output worker")??;
        Ok(())
    }
    pub(super) async fn record(
        &self,
        request: &str,
        stage: &'static str,
        retry: u32,
        heartbeats: u32,
        history_safe: bool,
        closed_token: bool,
    ) -> Result<()> {
        let evidence = Evidence::builder()
            .request_id(request)
            .pid(std::process::id())
            .sdk("rust")
            .stage(stage)
            .retry(retry)
            .heartbeats(heartbeats)
            .history_safe(history_safe)
            .closed_token(closed_token)
            .build();
        let pending = self.markers.join(format!("{request}.pending"));
        timeout(
            Duration::from_secs(2),
            fs::write(&pending, serde_json::to_vec(&evidence)?),
        )
        .await??;
        timeout(
            Duration::from_secs(2),
            fs::rename(pending, self.markers.join(format!("{request}.json"))),
        )
        .await??;
        Ok(())
    }
    pub(super) async fn released(&self, request: &str) -> Result<bool> {
        let path = self.gates.join(format!("{request}.release"));
        Ok(
            timeout(Duration::from_secs(2), fs::try_exists(path)).await??
                || timeout(
                    Duration::from_secs(2),
                    fs::try_exists(self.gates.join("all.release")),
                )
                .await??,
        )
    }
}
