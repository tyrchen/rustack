//! Real named binary generations, fresh handle epochs and zero side-effect replay.

use std::{collections::BTreeMap, fmt, io::Read, path::Path, time::Duration};

use anyhow::{Context, Result, ensure};
use aws_sdk_lambda::{
    Client as Lambda,
    primitives::Blob,
    types::{Architecture, Environment, FunctionCode, Runtime},
};
use aws_sdk_sfn::{
    Client,
    error::ProvideErrorMetadata,
    types::{ExecutionStatus, HistoryEvent, Tag},
};
use rustack_stepfunctions_core::CallbackToken;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
    time::{Instant, timeout_at},
};

use super::{
    process::{SECRET, Server},
    rust::{archive, create, history, lambda, sfn, terminal},
};

const ROLE: &str = "arn:aws:iam::000000000000:role/Workflows";
const UPDATED_ROLE: &str = "arn:aws:iam::000000000000:role/Updated";

#[derive(Debug)]
struct EvidenceDirectory {
    directory: Option<TempDir>,
}
impl EvidenceDirectory {
    fn path(&self) -> Result<&Path> {
        Ok(self
            .directory
            .as_ref()
            .context("missing owned restart directory")?
            .path())
    }
}
impl Drop for EvidenceDirectory {
    fn drop(&mut self) {
        if let Some(directory) = self.directory.take() {
            eprintln!(
                "restart evidence retained at {}",
                directory.keep().display()
            );
        }
    }
}
#[derive(Debug)]
struct Collector {
    port: u16,
    join: Option<JoinHandle<Result<CallbackToken>>>,
}
impl Collector {
    async fn start() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let join = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await?;
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") {
                ensure!(headers.len() < 4096, "oversized local collector header");
                headers.push(stream.read_u8().await?);
            }
            let headers = String::from_utf8(headers)?;
            let length = headers
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .context("collector request length")?
                .1
                .trim()
                .parse::<usize>()?;
            ensure!(length == 64, "invalid local token length");
            let mut body = vec![0; length];
            stream.read_exact(&mut body).await?;
            let token = CallbackToken::try_from(String::from_utf8(body)?)?;
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await?;
            stream.shutdown().await?;
            Ok(token)
        });
        Ok(Self {
            port,
            join: Some(join),
        })
    }
    async fn token(&mut self) -> Result<CallbackToken> {
        let result = timeout_at(
            deadline(),
            self.join.as_mut().context("collector already consumed")?,
        )
        .await?;
        self.join = None;
        result?
    }
    async fn close(&mut self) -> Result<()> {
        if let Some(join) = &mut self.join {
            join.abort();
            let result = timeout_at(deadline(), join).await?;
            self.join = None;
            if let Err(error) = result {
                ensure!(error.is_cancelled(), "collector worker panicked");
            }
        }
        Ok(())
    }
}
impl Drop for Collector {
    fn drop(&mut self) {
        if let Some(join) = &self.join {
            join.abort();
        }
    }
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(40)
}
async fn deploy(server: &Server, root: &Path, collector: &Collector) -> Result<()> {
    let bytes = fs::read(&server.bootstrap).await?;
    let zip = tokio::task::spawn_blocking(move || archive(&bytes)).await??;
    fs::create_dir_all(root.join("invocations")).await?;
    let client: Lambda = lambda(server);
    for callback in [false, true] {
        let mut environment = Environment::builder()
            .variables("WORKFLOW_MODE", "1")
            .variables(
                "PROCESS_MARKER_DIR",
                server.markers.to_str().context("PID marker path")?,
            );
        if callback {
            environment = environment.variables("WORKFLOW_TOKEN_PORT", collector.port.to_string());
        } else {
            environment = environment.variables(
                "WORKFLOW_INVOCATION_DIR",
                root.join("invocations")
                    .to_str()
                    .context("invocation marker path")?,
            );
        }
        client
            .create_function()
            .function_name(if callback {
                "RestartToken"
            } else {
                "RestartCompute"
            })
            .role(ROLE)
            .runtime(Runtime::Providedal2023)
            .handler("bootstrap")
            .architectures(if cfg!(target_arch = "aarch64") {
                Architecture::Arm64
            } else {
                Architecture::X8664
            })
            .timeout(10)
            .environment(environment.build())
            .code(
                FunctionCode::builder()
                    .zip_file(Blob::new(zip.clone()))
                    .build(),
            )
            .send()
            .await?;
    }
    Ok(())
}
fn pass(value: i64) -> Value {
    json!({"StartAt":"P","States":{"P":{"Type":"Pass","Result":value,"End":true}}})
}
async fn start(client: &Client, machine: &str, name: &str, input: &str) -> Result<String> {
    Ok(client
        .start_execution()
        .state_machine_arn(machine)
        .name(name)
        .input(input)
        .send()
        .await?
        .execution_arn()
        .to_owned())
}
struct Saved {
    pure: String,
    pure_run: String,
    compute: String,
    compute_run: String,
    waiting: String,
    callback: String,
    old_page: String,
    token: CallbackToken,
    output: String,
    events: Vec<HistoryEvent>,
    definition: String,
    current_definition: String,
}
impl fmt::Debug for Saved {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SavedRestartEvidence(<redacted>)")
    }
}
async fn seed(server: &Server, root: &Path, collector: &mut Collector) -> Result<Saved> {
    deploy(server, root, collector).await?;
    let client = sfn(server, SECRET);
    let definition = pass(7).to_string();
    let pure = create(&client, "RestartPure", &pass(7)).await?;
    let pure_run = start(&client, &pure, "old", " {\"raw\":1}\n").await?;
    ensure!(
        terminal(&client, &pure_run).await?.output() == Some("7"),
        "pure source did not execute"
    );
    let current_definition = pass(8).to_string();
    client
        .update_state_machine()
        .state_machine_arn(&pure)
        .definition(&current_definition)
        .role_arn(UPDATED_ROLE)
        .send()
        .await?;
    client
        .tag_resource()
        .resource_arn(&pure)
        .tags(Tag::builder().key("purpose").value("snapshot").build())
        .send()
        .await?;
    let compute = create(&client, "RestartCompute", &json!({"StartAt":"T","States":{"T":{"Type":"Task","Resource":"arn:aws:states:::lambda:invoke","Parameters":{"FunctionName":"RestartCompute","Payload.$":"$"},"End":true}}})).await?;
    let compute_run = start(&client, &compute, "old", "{\"n\":21}").await?;
    let completed = terminal(&client, &compute_run).await?;
    let output = completed
        .output()
        .context("missing source Native output")?
        .to_owned();
    ensure!(
        serde_json::from_str::<Value>(&output)?.pointer("/Payload/computed") == Some(&json!(42)),
        "source Native computation missing"
    );
    let events = history(&client, &compute_run, false, true).await?;
    let wait = create(
        &client,
        "RestartWait",
        &json!({"StartAt":"W","States":{"W":{"Type":"Wait","Seconds":600,"End":true}}}),
    )
    .await?;
    let waiting = start(&client, &wait, "pending", "{}").await?;
    let callback_machine = create(&client, "RestartCallback", &json!({"StartAt":"T","States":{"T":{"Type":"Task","Resource":"arn:aws:states:::lambda:invoke.waitForTaskToken","Parameters":{"FunctionName":"RestartToken","Payload":{"token.$":"$$.Task.Token","n":21}},"End":true}}})).await?;
    let callback = start(&client, &callback_machine, "pending", "{}").await?;
    let token = collector.token().await?;
    client
        .send_task_heartbeat()
        .task_token(token.expose())
        .send()
        .await?;
    let page = client.list_state_machines().max_results(1).send().await?;
    let old_page = page
        .next_token()
        .context("missing source page handle")?
        .to_owned();
    Ok(Saved {
        pure,
        pure_run,
        compute,
        compute_run,
        waiting,
        callback,
        old_page,
        token,
        output,
        events,
        definition,
        current_definition,
    })
}
async fn markers(root: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut entries = fs::read_dir(root.join("invocations")).await?;
    let mut markers = BTreeMap::new();
    while let Some(entry) = entries.next_entry().await? {
        ensure!(
            markers.len() < 8
                && entry.file_type().await?.is_file()
                && entry.metadata().await?.len() <= 1024,
            "invalid invocation evidence"
        );
        markers.insert(
            entry.file_name().to_string_lossy().into_owned(),
            fs::read(entry.path()).await?,
        );
    }
    Ok(markers)
}
async fn old_state(server: &Server, saved: &Saved) -> Result<()> {
    let client = sfn(server, SECRET);
    let old = client
        .describe_execution()
        .execution_arn(&saved.compute_run)
        .send()
        .await?;
    ensure!(
        old.output() == Some(saved.output.as_str()),
        "old Native result changed or replayed"
    );
    ensure!(
        history(&client, &saved.compute_run, false, true).await? == saved.events,
        "old full Native history changed"
    );
    let pure = client
        .describe_execution()
        .execution_arn(&saved.pure_run)
        .send()
        .await?;
    ensure!(
        pure.input() == Some(" {\"raw\":1}\n") && pure.output() == Some("7"),
        "original pure input/output changed"
    );
    let captured = client
        .describe_state_machine_for_execution()
        .execution_arn(&saved.pure_run)
        .send()
        .await?;
    ensure!(
        captured.definition() == saved.definition && captured.role_arn() == ROLE,
        "captured revision/role changed"
    );
    let current = client
        .describe_state_machine()
        .state_machine_arn(&saved.pure)
        .send()
        .await?;
    ensure!(
        current.definition() == saved.current_definition && current.role_arn() == UPDATED_ROLE,
        "current revision/role not restored"
    );
    let tags = client
        .list_tags_for_resource()
        .resource_arn(&saved.pure)
        .send()
        .await?;
    ensure!(
        tags.tags()
            .iter()
            .any(|tag| tag.key() == Some("purpose") && tag.value() == Some("snapshot")),
        "tags not restored"
    );
    for arn in [&saved.waiting, &saved.callback] {
        let stopped = client
            .describe_execution()
            .execution_arn(arn)
            .send()
            .await?;
        ensure!(
            stopped.status() == &ExecutionStatus::Aborted
                && stopped.error() == Some("Rustack.Shutdown"),
            "shutdown did not persist terminal interruption"
        );
    }
    stale_handles(&client, saved).await
}
async fn stale_handles(client: &Client, saved: &Saved) -> Result<()> {
    let duplicate = client
        .start_execution()
        .state_machine_arn(&saved.compute)
        .name("old")
        .input("{\"n\":21}")
        .send()
        .await
        .err()
        .context("lost closed execution name")?;
    ensure!(
        duplicate
            .as_service_error()
            .and_then(ProvideErrorMetadata::code)
            == Some("ExecutionAlreadyExists"),
        "closed name reservation changed"
    );
    let page = client
        .list_state_machines()
        .max_results(1)
        .next_token(&saved.old_page)
        .send()
        .await
        .err()
        .context("old pagination handle survived restart")?;
    ensure!(
        page.as_service_error().and_then(ProvideErrorMetadata::code) == Some("InvalidToken"),
        "old pagination handle classification changed"
    );
    let callback = client
        .send_task_heartbeat()
        .task_token(saved.token.expose())
        .send()
        .await
        .err()
        .context("old callback token survived restart")?;
    ensure!(
        callback
            .as_service_error()
            .and_then(ProvideErrorMetadata::code)
            == Some("InvalidToken"),
        "old callback root epoch was restored"
    );
    Ok(())
}
async fn assert_no_token_archive(root: &Path, saved: &Saved) -> Result<()> {
    for relative in [
        "services/stepfunctions/meta.ss.zst",
        "services/stepfunctions/data.ss.zst",
        "services/lambda/meta.ss.zst",
    ] {
        let compressed = fs::read(root.join("snapshots/restart").join(relative)).await?;
        let mut decoded = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
            let decoder = zstd::stream::read::Decoder::new(compressed.as_slice())?;
            let mut bytes = Vec::new();
            decoder.take(16 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
            Ok(bytes)
        })
        .await??;
        ensure!(
            decoded.len() <= 16 * 1024 * 1024,
            "oversized fixture archive"
        );
        ensure!(
            !decoded
                .windows(saved.token.expose().len())
                .any(|window| window == saved.token.expose().as_bytes()),
            "callback credential persisted"
        );
        decoded.fill(0);
    }
    Ok(())
}
async fn reject_corruption(root: &Path, saved: &Saved) -> Result<()> {
    let meta = root.join("snapshots/restart/services/stepfunctions/meta.ss.zst");
    let original = fs::read(&meta).await?;
    let mut corrupt = original.clone();
    let first = corrupt.first_mut().context("missing workflow archive")?;
    *first ^= 1;
    fs::write(&meta, &corrupt).await?;
    let manifest = fs::read(root.join("snapshots/restart/manifest.ss.zst")).await?;
    Server::reject_named(root).await?;
    ensure!(
        fs::read(&meta).await? == corrupt
            && fs::read(root.join("snapshots/restart/manifest.ss.zst")).await? == manifest,
        "failed startup replaced the generation"
    );
    fs::write(&meta, original).await?;
    assert_no_token_archive(root, saved).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires actual host bootstrap; run make test-workflows-persistence"]
async fn test_should_restart_named_native_snapshots_without_replay_and_lose_only_unsaved_work()
-> Result<()> {
    let mut evidence = EvidenceDirectory {
        directory: Some(tempfile::tempdir()?),
    };
    let root = evidence.path()?.to_owned();
    let mut collector = Collector::start().await?;
    let mut source = Server::start_named(&root).await?;
    let mut target = None;
    let mut crashing = None;
    let mut final_server = None;
    let result = async {
        let saved = seed(&source, &root, &mut collector).await?;
        source.shutdown(deadline()).await?;
        assert_no_token_archive(&root, &saved).await?;
        let before = markers(&root).await?;
        ensure!(before.len() == 1, "source invocation marker count");
        target = Some(Server::start_named(&root).await?);
        let active = target.as_mut().context("missing restored server")?;
        old_state(active, &saved).await?;
        ensure!(
            markers(&root).await? == before,
            "restore replayed a physical Lambda call"
        );
        let client = sfn(active, SECRET);
        let next = start(&client, &saved.compute, "after", "{\"n\":22}").await?;
        let outcome = terminal(&client, &next).await?;
        ensure!(
            serde_json::from_str::<Value>(outcome.output().context("missing new result")?)?
                .pointer("/Payload/computed")
                == Some(&json!(44)),
            "new restored Lambda execution failed"
        );
        ensure!(
            markers(&root).await?.len() == 2,
            "new Lambda invocation was not observed"
        );
        active.shutdown(deadline()).await?;
        let committed = fs::read(root.join("snapshots/restart/manifest.ss.zst")).await?;
        crashing = Some(Server::start_named(&root).await?);
        let active = crashing.as_mut().context("missing crash server")?;
        let client = sfn(active, SECRET);
        let unsaved = create(&client, "Unsaved", &pass(99)).await?;
        terminal(&client, &start(&client, &unsaved, "unsaved", "{}").await?).await?;
        active.crash(deadline()).await?;
        ensure!(
            fs::read(root.join("snapshots/restart/manifest.ss.zst")).await? == committed,
            "SIGKILL wrote a new generation"
        );
        final_server = Some(Server::start_named(&root).await?);
        let active = final_server.as_mut().context("missing final server")?;
        old_state(active, &saved).await?;
        let missing = sfn(active, SECRET)
            .describe_state_machine()
            .state_machine_arn(unsaved)
            .send()
            .await
            .err()
            .context("unsaved work survived SIGKILL")?;
        ensure!(
            missing
                .as_service_error()
                .and_then(ProvideErrorMetadata::code)
                == Some("StateMachineDoesNotExist"),
            "wrong unsaved resource error"
        );
        ensure!(
            markers(&root).await?.len() == 2,
            "restart replayed Lambda work"
        );
        active.shutdown(deadline()).await?;
        reject_corruption(&root, &saved).await?;
        Ok(())
    }
    .await;
    let one = source.shutdown(deadline()).await;
    let two = if let Some(server) = &mut target {
        server.shutdown(deadline()).await.map(|_| ())
    } else {
        Ok(())
    };
    let three = if let Some(server) = &mut crashing {
        server.shutdown(deadline()).await.map(|_| ())
    } else {
        Ok(())
    };
    let four = if let Some(server) = &mut final_server {
        server.shutdown(deadline()).await.map(|_| ())
    } else {
        Ok(())
    };
    let collected = collector.close().await;
    one?;
    two?;
    three?;
    four?;
    collected?;
    result?;
    drop(evidence.directory.take());
    Ok(())
}

#[tokio::test]
async fn test_should_close_failed_token_collector_without_repolling_completed_task() -> Result<()> {
    for early_eof in [false, true] {
        let mut collector = Collector::start().await?;
        let result = async {
            let mut client = tokio::net::TcpStream::connect(("127.0.0.1", collector.port)).await?;
            client
                .write_all(b"POST /token HTTP/1.1\r\nHost: localhost\r\nContent-Length: 64\r\n\r\n")
                .await?;
            client
                .write_all(if early_eof { b"early EOF" } else { &[b'z'; 64] })
                .await?;
            client.shutdown().await?;
            ensure!(
                collector.token().await.is_err(),
                "malformed token was accepted"
            );
            ensure!(
                collector.join.is_none(),
                "completed failure join was retained"
            );
            Ok::<(), anyhow::Error>(())
        }
        .await;
        collector.close().await?;
        result?;
        collector.close().await?;
    }
    Ok(())
}
