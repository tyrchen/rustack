//! Docker attach + official RIC feasibility driver. Never contacts AWS APIs.
#![forbid(unsafe_code)]

use std::{
    collections::{BTreeSet, HashMap},
    env,
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use bollard::{
    API_DEFAULT_VERSION, Docker,
    container::{AttachContainerResults, LogOutput},
    errors::Error as DockerError,
    models::{ContainerCreateBody, HostConfig},
    query_parameters::{
        AttachContainerOptionsBuilder, ListContainersOptionsBuilder, LogsOptionsBuilder,
        RemoveContainerOptionsBuilder,
    },
};
use bytes::{Bytes, BytesMut};
use futures::StreamExt;
use rustack_workflows_spikes::{Frame, MAX_FRAME, decode, write_frame};
use serde_json::{Value, json};
use tokio::{
    io::AsyncWriteExt,
    time::{Instant, sleep, timeout},
};
use uuid::Uuid;

async fn next_frame(attach: &mut AttachContainerResults, buffer: &mut BytesMut) -> Result<Frame> {
    timeout(Duration::from_secs(45), async {
        loop {
            if buffer.len() >= 4 {
                let prefix: [u8; 4] = buffer.get(..4).context("length prefix")?.try_into()?;
                let n = usize::try_from(u32::from_be_bytes(prefix))?;
                ensure!(n <= MAX_FRAME, "frame exceeded cap");
                if buffer.len() >= n + 4 {
                    let complete = buffer.split_to(n + 4);
                    return Ok(decode(complete.get(4..).context("body slice")?)?);
                }
            }
            match attach.output.next().await {
                Some(Ok(LogOutput::StdOut { message })) => {
                    ensure!(
                        buffer
                            .len()
                            .checked_add(message.len())
                            .context("buffer overflow")?
                            <= MAX_FRAME + 4096,
                        "buffer exceeded cap"
                    );
                    buffer.extend_from_slice(&message);
                }
                Some(Ok(LogOutput::StdErr { message })) => {
                    bail!("relay diagnostic: {}", String::from_utf8_lossy(&message))
                }
                Some(Ok(_)) => bail!("unexpected Docker stream"),
                Some(Err(error)) => return Err(error.into()),
                None => bail!("attach stream ended"),
            }
        }
    })
    .await
    .context("relay frame deadline")?
}

fn limits(memory: i64, cpus: i64) -> HostConfig {
    HostConfig {
        memory: Some(memory),
        nano_cpus: Some(cpus),
        pids_limit: Some(64),
        readonly_rootfs: Some(true),
        cap_drop: Some(vec!["ALL".into()]),
        security_opt: Some(vec!["no-new-privileges:true".into()]),
        tmpfs: Some(HashMap::from([(
            "/tmp".into(),
            "rw,noexec,nosuid,size=536870912".into(),
        )])),
        ..Default::default()
    }
}

#[derive(Debug)]
struct Environment {
    identity: String,
    relay: String,
    function: String,
    attach: AttachContainerResults,
    buffer: BytesMut,
}

fn owner_labels(identity: &str, run_id: &str) -> HashMap<String, String> {
    HashMap::from([
        ("rustack.p0.environment".into(), identity.into()),
        ("rustack.p0.run".into(), run_id.into()),
    ])
}

async fn create_relay(
    docker: &Docker,
    architecture: &str,
    identity: &str,
    run_id: &str,
    stdin_once: bool,
) -> Result<String> {
    Ok(docker
        .create_container(
            None,
            ContainerCreateBody {
                image: Some(format!("rustack-p0-relay:{architecture}")),
                user: Some("1000:1000".into()),
                env: Some(vec![format!("SPIKE_ENVIRONMENT_UUID={identity}")]),
                labels: Some(owner_labels(identity, run_id)),
                attach_stdin: Some(true),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                open_stdin: Some(true),
                stdin_once: Some(stdin_once),
                tty: Some(false),
                host_config: Some(limits(64 * 1024 * 1024, 100_000_000)),
                ..Default::default()
            },
        )
        .await?
        .id)
}

async fn create_function(
    docker: &Docker,
    architecture: &str,
    identity: &str,
    relay: &str,
    run_id: &str,
) -> Result<String> {
    let mut host = limits(192 * 1024 * 1024, 1_000_000_000);
    host.network_mode = Some(format!("container:{relay}"));
    host.pids_limit = Some(256);
    Ok(docker
        .create_container(
            None,
            ContainerCreateBody {
                image: Some(format!("rustack-p0-handler:{architecture}")),
                user: Some("1000:1000".into()),
                labels: Some(owner_labels(identity, run_id)),
                env: Some(vec![
                    "AWS_LAMBDA_RUNTIME_API=127.0.0.1:9001".into(),
                    "AWS_LAMBDA_FUNCTION_NAME=spike".into(),
                    "AWS_LAMBDA_FUNCTION_VERSION=$LATEST".into(),
                    "AWS_LAMBDA_FUNCTION_MEMORY_SIZE=192".into(),
                    "AWS_REGION=us-east-1".into(),
                    "AWS_DEFAULT_REGION=us-east-1".into(),
                ]),
                host_config: Some(host),
                ..Default::default()
            },
        )
        .await?
        .id)
}

async fn expect_kind(
    attach: &mut AttachContainerResults,
    buffer: &mut BytesMut,
    kind: &str,
) -> Result<Frame> {
    let frame = next_frame(attach, buffer)
        .await
        .with_context(|| format!("await {kind}"))?;
    ensure!(
        frame.kind == kind,
        "unexpected control variant awaiting {kind}"
    );
    Ok(frame)
}

async fn create_environment(
    docker: &Docker,
    ids: &mut Vec<String>,
    architecture: &str,
    run_id: &str,
) -> Result<Environment> {
    let identity = Uuid::new_v4().to_string();
    let relay = create_relay(docker, architecture, &identity, run_id, true).await?;
    ids.push(relay.clone());
    let mut attach = docker
        .attach_container(
            &relay,
            Some(
                AttachContainerOptionsBuilder::default()
                    .stdin(true)
                    .stdout(true)
                    .stderr(true)
                    .stream(true)
                    .logs(false)
                    .build(),
            ),
        )
        .await?;
    docker.start_container(&relay, None).await?;
    let mut buffer = BytesMut::new();
    expect_kind(&mut attach, &mut buffer, "hello").await?;
    expect_kind(&mut attach, &mut buffer, "ready").await?;
    let function = create_function(docker, architecture, &identity, &relay, run_id).await?;
    ids.push(function.clone());
    docker.start_container(&function, None).await?;
    let ready = expect_kind(&mut attach, &mut buffer, "bootstrapReady").await?;
    ensure!(ready.environment == identity, "foreign bootstrap readiness");
    Ok(Environment {
        identity,
        relay,
        function,
        attach,
        buffer,
    })
}

async fn invoke(environment: &mut Environment, payload: Bytes, kind: &str) -> Result<Frame> {
    let request = Uuid::new_v4().to_string();
    write_frame(
        &mut environment.attach.input,
        &Frame::new(&environment.identity, "invoke", &request, payload),
    )
    .await?;
    let result = expect_kind(&mut environment.attach, &mut environment.buffer, kind).await?;
    ensure!(
        result.request_id == request && result.environment == environment.identity,
        "foreign invocation result"
    );
    Ok(result)
}

fn check_output(body: &Value, value: i32, count: i32) -> Result<()> {
    for (field, expected) in [
        ("computed", json!(value + 7)),
        ("counter", json!(count)),
        ("uid", json!(1000)),
        ("memoryMax", json!("201326592")),
        ("pidsMax", json!("256")),
        ("rootReadOnly", json!(true)),
    ] {
        ensure!(
            body.get(field) == Some(&expected),
            "unexpected fixture output: {field}"
        );
    }
    Ok(())
}

async fn wait_handler_started(docker: &Docker, function: &str, request: &str) -> Result<()> {
    let marker = format!("P0_BUSY_START={request}");
    let mut stream = docker.logs(
        function,
        Some(
            LogsOptionsBuilder::default()
                .stdout(true)
                .stderr(true)
                .follow(true)
                .tail("20")
                .build(),
        ),
    );
    timeout(Duration::from_secs(45), async {
        let mut bytes = Vec::new();
        while let Some(message) = stream.next().await {
            match message? {
                LogOutput::StdOut { message } | LogOutput::StdErr { message } => {
                    ensure!(
                        bytes
                            .len()
                            .checked_add(message.len())
                            .context("log size overflow")?
                            <= 16 * 1024,
                        "fixture logs exceeded cap"
                    );
                    bytes.extend_from_slice(&message);
                    if bytes
                        .windows(marker.len())
                        .any(|chunk| chunk == marker.as_bytes())
                    {
                        return Ok(());
                    }
                }
                _ => bail!("unexpected function log stream"),
            }
        }
        bail!("handler exited before busy marker")
    })
    .await
    .context("handler start deadline")?
}

async fn exercise(
    docker: &Docker,
    ids: &mut Vec<String>,
    architecture: &str,
    run_id: &str,
) -> Result<Value> {
    let start = Instant::now();
    let mut environment = create_environment(docker, ids, architecture, run_id).await?;
    let cold_ms = start.elapsed().as_millis();
    let mut outputs = Vec::new();
    for (value, count) in [(35, 1), (0, 2)] {
        let result = invoke(
            &mut environment,
            Bytes::from(serde_json::to_vec(&json!({"value":value}))?),
            "result",
        )
        .await?;
        let body: Value = serde_json::from_slice(&result.payload)?;
        check_output(&body, value, count)?;
        outputs.push(body);
    }
    let error = invoke(
        &mut environment,
        Bytes::from_static(br#"{"fail":true}"#),
        "error",
    )
    .await?;
    let failure: Value = serde_json::from_slice(&error.payload)?;
    ensure!(
        failure.get("errorType") == Some(&json!("ValueError")),
        "RIC error type lost"
    );
    let inspected = docker.inspect_container(&environment.relay, None).await?;
    if let Some(ports) = inspected
        .network_settings
        .and_then(|settings| settings.ports)
    {
        ensure!(
            ports.values().all(Option::is_none),
            "Runtime API ports published"
        );
    }
    let request = Uuid::new_v4().to_string();
    write_frame(
        &mut environment.attach.input,
        &Frame::new(
            &environment.identity,
            "invoke",
            &request,
            Bytes::from_static(br#"{"sleep":true}"#),
        ),
    )
    .await?;
    wait_handler_started(docker, &environment.function, &request).await?;
    docker
        .remove_container(
            &environment.function,
            Some(RemoveContainerOptionsBuilder::default().force(true).build()),
        )
        .await?;
    write_frame(
        &mut environment.attach.input,
        &Frame::new(&environment.identity, "close", "", Bytes::new()),
    )
    .await?;
    expect_kind(&mut environment.attach, &mut environment.buffer, "ack").await?;
    Ok(
        json!({"architecture":architecture,"coldMs":cold_ms,"outputs":outputs,"errorType":failure.get("errorType"),"noPublishedRuntimePorts":true,"closeAck":true,"cancelledBusyFunction":true}),
    )
}

async fn relay_is_running(docker: &Docker, relay: &str) -> Result<bool> {
    docker
        .inspect_container(relay, None)
        .await?
        .state
        .and_then(|state| state.running)
        .context("missing container state")
}

async fn probe_attach_eof(
    docker: &Docker,
    architecture: &str,
    run_id: &str,
    ids: &mut Vec<String>,
) -> Result<Value> {
    let mut observations = Vec::new();
    for stdin_once in [false, true] {
        let identity = Uuid::new_v4().to_string();
        let relay = create_relay(docker, architecture, &identity, run_id, stdin_once).await?;
        ids.push(relay.clone());
        let mut attach = docker
            .attach_container(
                &relay,
                Some(
                    AttachContainerOptionsBuilder::default()
                        .stdin(true)
                        .stdout(true)
                        .stderr(true)
                        .stream(true)
                        .build(),
                ),
            )
            .await?;
        docker.start_container(&relay, None).await?;
        let mut buffer = BytesMut::new();
        expect_kind(&mut attach, &mut buffer, "hello").await?;
        expect_kind(&mut attach, &mut buffer, "ready").await?;
        attach.input.shutdown().await?;
        if stdin_once {
            timeout(Duration::from_secs(20), async {
                while relay_is_running(docker, &relay).await? {
                    sleep(Duration::from_millis(25)).await;
                }
                Ok::<(), anyhow::Error>(())
            })
            .await
            .context("relay did not exit after stdin EOF")??;
        } else {
            sleep(Duration::from_millis(500)).await;
        }
        observations.push(json!({"stdinOnce":stdin_once,"runningAfterShutdown":relay_is_running(docker, &relay).await?}));
    }
    Ok(json!(observations))
}

async fn owned_containers(docker: &Docker, run_id: &str) -> Result<Vec<String>> {
    let filter: HashMap<String, Vec<String>> =
        HashMap::from([("label".into(), vec![format!("rustack.p0.run={run_id}")])]);
    let listed = docker
        .list_containers(Some(
            ListContainersOptionsBuilder::default()
                .all(true)
                .filters(&filter)
                .build(),
        ))
        .await?;
    ensure!(listed.len() <= 8, "unexpected probe resource count");
    listed
        .into_iter()
        .map(|record| record.id.context("owned container lacks ID"))
        .collect()
}

async fn cleanup_owned(docker: &Docker, run_id: &str, ids: &[String]) -> Result<()> {
    timeout(Duration::from_secs(20), async {
        let mut candidates: BTreeSet<String> = ids.iter().cloned().collect();
        let mut errors = Vec::new();
        for _ in 0..3 {
            candidates.extend(owned_containers(docker, run_id).await?);
            for id in &candidates {
                match docker
                    .remove_container(
                        id,
                        Some(RemoveContainerOptionsBuilder::default().force(true).build()),
                    )
                    .await
                {
                    Ok(())
                    | Err(DockerError::DockerResponseServerError {
                        status_code: 404, ..
                    }) => {}
                    Err(error) => errors.push(error),
                }
            }
            sleep(Duration::from_millis(200)).await;
        }
        ensure!(
            owned_containers(docker, run_id).await?.is_empty(),
            "probe-owned containers remain: run {run_id}"
        );
        ensure!(
            errors.is_empty(),
            "cleanup had uncertain failures for run {run_id}"
        );
        Ok::<(), anyhow::Error>(())
    })
    .await
    .context("owned cleanup deadline")?
}

#[tokio::main]
async fn main() -> Result<()> {
    let socket = env::args().nth(1).context("socket argument required")?;
    let architecture = env::args().nth(2).context("architecture required")?;
    ensure!(
        matches!(architecture.as_str(), "arm64" | "amd64"),
        "invalid architecture"
    );
    let docker = Docker::connect_with_unix(&socket, 60, API_DEFAULT_VERSION)?
        .negotiate_version()
        .await?;
    let run_id = Uuid::new_v4().to_string();
    let mut ids = Vec::new();
    let outcome = timeout(Duration::from_secs(180), async {
        let mut evidence = exercise(&docker, &mut ids, &architecture, &run_id).await?;
        let eof = probe_attach_eof(&docker, &architecture, &run_id, &mut ids).await?;
        evidence
            .as_object_mut()
            .context("evidence object")?
            .insert("attachEof".into(), eof);
        // Deliberately omit this ID from the registry to simulate losing a create response.
        let _unrecorded = create_relay(
            &docker,
            &architecture,
            &Uuid::new_v4().to_string(),
            &run_id,
            true,
        )
        .await?;
        evidence
            .as_object_mut()
            .context("evidence object")?
            .insert("runId".into(), json!(run_id));
        Ok::<Value, anyhow::Error>(evidence)
    })
    .await
    .context("spike deadline")
    .and_then(|value| value);
    cleanup_owned(&docker, &run_id, &ids).await?;
    match outcome {
        Ok(mut evidence) => {
            evidence
                .as_object_mut()
                .context("evidence object")?
                .insert("labelReconciliation".into(), json!(true));
            println!("{}", serde_json::to_string_pretty(&evidence)?);
            Ok(())
        }
        Err(error) => Err(error),
    }
}
