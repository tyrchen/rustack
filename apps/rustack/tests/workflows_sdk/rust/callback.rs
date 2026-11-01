//! Actual strict-auth service and both handler-side SDKs, using one dispatch slot.

use std::{collections::BTreeSet, env, path::PathBuf};

use serde::Deserialize;

use super::*;
use crate::{process::assert_absent, runner::interpreter};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Case {
    id: String,
    mode: String,
    status: String,
    #[serde(default)]
    error: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Catalog {
    source: String,
    cases: Vec<Case>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Evidence {
    request_id: String,
    pid: u32,
    sdk: String,
    stage: String,
    retry: u32,
    heartbeats: u32,
    history_safe: bool,
    closed_token: bool,
}

fn root(server: &Server) -> Result<PathBuf> {
    Ok(server
        .markers
        .parent()
        .context("private fixture root")?
        .to_owned())
}
async fn markers(server: &Server) -> Result<Vec<Evidence>> {
    let mut directory = fs::read_dir(root(server)?.join("callbacks")).await?;
    let mut events = Vec::new();
    let mut files = 0;
    while let Some(entry) = directory.next_entry().await? {
        files += 1;
        ensure!(
            files <= 64 && entry.file_type().await?.is_file(),
            "invalid callback evidence collection"
        );
        if entry
            .path()
            .extension()
            .is_none_or(|extension| extension != "json")
        {
            continue;
        }
        ensure!(
            entry.metadata().await?.len() <= 1024,
            "oversized callback evidence"
        );
        let event: Evidence = serde_json::from_slice(&fs::read(entry.path()).await?)?;
        ensure!(
            event.request_id.len() == 36
                && Uuid::parse_str(&event.request_id)?.to_string() == event.request_id
                && entry.file_name() == format!("{}.json", event.request_id).as_str()
                && event.pid > 1
                && event.retry <= 1
                && ["rust", "boto3"].contains(&event.sdk.as_str())
                && ["consumed", "returned", "heartbeat", "callback"]
                    .contains(&event.stage.as_str()),
            "invalid callback evidence"
        );
        events.push(event);
    }
    Ok(events)
}
async fn next(server: &Server, seen: &BTreeSet<String>, sdk: &str) -> Result<Evidence> {
    timeout(Duration::from_secs(10), async {
        loop {
            if let Some(event) = markers(server)
                .await?
                .into_iter()
                .find(|v| !seen.contains(&v.request_id) && v.sdk == sdk)
            {
                return Ok(event);
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .context("handler did not consume an actual event")?
}
async fn stage(server: &Server, id: &str, expected: &str) -> Result<Evidence> {
    timeout(Duration::from_secs(10), async {
        loop {
            if let Some(event) = markers(server)
                .await?
                .into_iter()
                .find(|v| v.request_id == id && v.stage == expected)
            {
                return Ok(event);
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .context("handler SDK stage did not complete")?
}
async fn release(server: &Server, id: &str) -> Result<()> {
    fs::write(
        root(server)?
            .join("callback-gates")
            .join(format!("{id}.release")),
        b"release",
    )
    .await?;
    Ok(())
}
async fn submitted(client: &Client, arn: &str) -> Result<()> {
    timeout(Duration::from_secs(10), async {
        loop {
            let events = history(client, arn, false, true).await?;
            if events
                .iter()
                .any(|event| event.r#type().as_str() == "TaskSubmitted")
            {
                return Ok(());
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .context("actual Lambda invocation was not submitted")?
}
async fn running(client: &Client, arn: &str) -> Result<()> {
    ensure!(
        client
            .describe_execution()
            .execution_arn(arn)
            .send()
            .await?
            .status()
            == &ExecutionStatus::Running,
        "callback task terminated before its gate"
    );
    Ok(())
}
async fn deploy_callbacks(server: &Server) -> Result<()> {
    let rust = PathBuf::from(
        env::var_os("RUSTACK_CALLBACK_BOOTSTRAP")
            .context("build actual callback bootstrap with make")?,
    );
    ensure!(
        (1..=64 * 1024 * 1024).contains(&fs::metadata(&rust).await?.len()),
        "invalid callback bootstrap size"
    );
    let python = interpreter()?;
    let python = python.to_str().context("interpreter path")?;
    let client = lambda(server);
    let private = root(server)?;
    let native = fs::read(rust).await?;
    for (name, python_handler) in [("CallbackRust", false), ("CallbackPython", true)] {
        let bytes = native.clone();
        let archive =
            tokio::task::spawn_blocking(move || callback_archive(&bytes, python_handler)).await??;
        let mut variables = Environment::builder()
            .variables("CALLBACK_ENDPOINT", &server.endpoint)
            .variables("CALLBACK_ACCESS_KEY", ACCESS)
            .variables("CALLBACK_SECRET_KEY", SECRET)
            .variables(
                "PROCESS_MARKER_DIR",
                server.markers.to_str().context("process path")?,
            )
            .variables(
                "CALLBACK_MARKER_DIR",
                private
                    .join("callbacks")
                    .to_str()
                    .context("callback path")?,
            )
            .variables(
                "CALLBACK_GATE_DIR",
                private
                    .join("callback-gates")
                    .to_str()
                    .context("gate path")?,
            );
        if python_handler {
            variables = variables.variables("CALLBACK_PYTHON", python);
        }
        client
            .create_function()
            .function_name(name)
            .role(ROLE)
            .runtime(Runtime::Providedal2023)
            .handler("bootstrap")
            .architectures(if cfg!(target_arch = "aarch64") {
                Architecture::Arm64
            } else {
                Architecture::X8664
            })
            .timeout(30)
            .environment(variables.build())
            .code(FunctionCode::builder().zip_file(Blob::new(archive)).build())
            .send()
            .await?;
    }
    Ok(())
}
fn callback_archive(native: &[u8], python: bool) -> Result<Vec<u8>> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer.start_file(
        "bootstrap",
        SimpleFileOptions::default().unix_permissions(0o755),
    )?;
    writer.write_all(native)?;
    if python {
        writer.start_file(
            "callback.py",
            SimpleFileOptions::default().unix_permissions(0o600),
        )?;
        writer.write_all(include_bytes!(
            "../../../../../tests/lambda-fixtures/callback_bootstrap.py"
        ))?;
    }
    Ok(writer.finish()?.into_inner())
}

fn definition(function: &str, case: &Case) -> Value {
    let mut task = json!({"Type":"Task","Resource":"arn:aws:states:::lambda:invoke.waitForTaskToken","TimeoutSeconds":60,
        "Parameters":{"FunctionName":function,"Payload":{"token.$":"$$.Task.Token","executionArn.$":"$$.Execution.Id","mode":case.mode,"n":21,"retry.$":"$$.State.RetryCount"}},"End":true});
    if ["heartbeat", "absoluteTimeout", "heartbeatTimeout"].contains(&case.mode.as_str()) {
        task["HeartbeatSeconds"] = json!(1);
    }
    if case.mode == "absoluteTimeout" {
        task["TimeoutSeconds"] = json!(2);
    }
    if case.mode == "retry" {
        task["Retry"] =
            json!([{"ErrorEquals":["CallbackRetry"],"IntervalSeconds":1,"MaxAttempts":1}]);
    }
    json!({"StartAt":"Call","States":{"Call":task}})
}
async fn unrelated(client: &Client) -> Result<()> {
    let definition = json!({"StartAt":"Call","States":{"Call":{"Type":"Task","Resource":"arn:aws:lambda:us-east-1:000000000000:function:SdkCompute","End":true}}});
    let machine = create(client, &format!("Probe{}", Uuid::new_v4()), &definition).await?;
    let execution = client
        .start_execution()
        .state_machine_arn(machine)
        .input("{\"n\":21}")
        .send()
        .await?;
    let result = terminal(client, execution.execution_arn()).await?;
    ensure!(
        result.status() == &ExecutionStatus::Succeeded,
        "unrelated workflow could not acquire dispatch capacity"
    );
    let output: Value = serde_json::from_str(result.output().context("probe output")?)?;
    ensure!(
        output["computed"] == 42,
        "unrelated actual Lambda computation missing"
    );
    Ok(())
}
async fn retired(pid: u32) -> Result<()> {
    timeout(Duration::from_secs(5), async {
        loop {
            if assert_absent(pid).await.is_ok() {
                return Ok(());
            }
            sleep(Duration::from_millis(15)).await;
        }
    })
    .await
    .context("consumed handler PID survived cancellation")?
}
async fn verify(client: &Client, execution: &str, case: &Case, events: &[Evidence]) -> Result<()> {
    let result = terminal(client, execution).await?;
    ensure!(
        result.status().as_str() == case.status,
        "callback case {}: status {:?}, error {:?}",
        case.id,
        result.status(),
        result.error()
    );
    ensure!(
        result.error() == case.error.as_deref(),
        "callback case {}: incorrect error",
        case.id
    );
    if case.status == "SUCCEEDED" {
        let output: Value = serde_json::from_str(result.output().context("callback output")?)?;
        let last = events.last().context("consumed callback")?;
        ensure!(
            output
                == json!({"value":42,"sdk":last.sdk,"requestId":last.request_id,"retry":last.retry}),
            "actual handler result lost or double encoded"
        );
    }
    let history = history(client, execution, false, true).await?;
    let mut submitted_count = 0;
    for (offset, event) in history.iter().enumerate() {
        ensure!(
            event.id() == i64::try_from(offset + 1)?
                && event.previous_event_id() == sequential_predecessor(offset)?,
            "broken callback causal history"
        );
        if let Some(details) = event.task_scheduled_event_details() {
            let parameters: Value = serde_json::from_str(details.parameters())?;
            ensure!(
                parameters["Payload"]["token"].is_null(),
                "injected token exposed in Scheduled history"
            );
        }
        if let Some(details) = event.task_submitted_event_details() {
            submitted_count += 1;
            let value: Value =
                serde_json::from_str(details.output().context("submitted metadata")?)?;
            let request = value["SdkResponseMetadata"]["RequestId"]
                .as_str()
                .context("actual request metadata")?;
            ensure!(
                events.iter().any(|e| e.request_id == request)
                    && value["ExecutedVersion"] == "$LATEST"
                    && value["SdkHttpMetadata"]["HttpStatusCode"] == 200,
                "invented callback invocation metadata"
            );
            ensure!(
                details.output_details().is_some_and(|d| d.truncated),
                "missing callback body omission marker"
            );
        }
    }
    let expected = if case.mode == "stop" || case.mode == "invocationFailure" {
        0
    } else if case.mode == "retry" {
        2
    } else {
        1
    };
    ensure!(
        submitted_count == expected,
        "missing or duplicate callback submission"
    );
    Ok(())
}
async fn exercise_case(
    client: &Client,
    server: &Server,
    case: &Case,
    function: &str,
    sdk: &str,
    seen: &mut BTreeSet<String>,
) -> Result<()> {
    let machine = create(
        client,
        &format!("{function}-{}", case.id),
        &definition(function, case),
    )
    .await?;
    let execution = client
        .start_execution()
        .state_machine_arn(machine)
        .send()
        .await?;
    let arn = execution.execution_arn();
    let first = next(server, seen, sdk).await?;
    seen.insert(first.request_id.clone());
    let mut events = vec![first];
    let first = events.first().context("first consumed callback")?;
    if case.mode == "stop" {
        running(client, arn).await?;
        client
            .stop_execution()
            .execution_arn(arn)
            .error("Stopped")
            .send()
            .await?;
        retired(first.pid).await?;
    } else if ["late", "heartbeat", "absoluteTimeout", "heartbeatTimeout"]
        .contains(&case.mode.as_str())
    {
        submitted(client, arn).await?;
        running(client, arn).await?;
        if case.mode == "late" {
            unrelated(client).await?;
            running(client, arn).await?;
        }
        if case.mode == "heartbeat" {
            sleep(Duration::from_millis(1400)).await;
            running(client, arn).await?;
            ensure!(
                stage(server, &first.request_id, "heartbeat")
                    .await?
                    .heartbeats
                    >= 3,
                "actual SDK heartbeats missing"
            );
        }
        if ["absoluteTimeout", "heartbeatTimeout"].contains(&case.mode.as_str()) {
            terminal(client, arn).await?;
        }
        release(server, &first.request_id).await?;
        let complete = stage(server, &first.request_id, "callback").await?;
        ensure!(
            complete.history_safe && complete.closed_token,
            "handler token/history checks failed"
        );
    } else {
        let complete = stage(server, &first.request_id, "callback").await?;
        ensure!(
            complete.history_safe && complete.closed_token,
            "handler SDK evidence missing"
        );
        running(client, arn).await?;
        release(server, &first.request_id).await?;
        if case.mode == "retry" {
            let next = next(server, seen, sdk).await?;
            seen.insert(next.request_id.clone());
            ensure!(
                next.retry == 1 && next.pid == first.pid,
                "fresh retry context or retained old-token proof missing"
            );
            let complete = stage(server, &next.request_id, "callback").await?;
            ensure!(
                complete.history_safe && complete.closed_token,
                "retry SDK checks failed"
            );
            running(client, arn).await?;
            release(server, &next.request_id).await?;
            events.push(next);
        }
    }
    verify(client, arn, case, &events).await?;
    if case.mode == "invocationFailure" {
        retired(events.first().context("invocation")?.pid).await?;
    }
    Ok(())
}
pub(super) async fn exercise(server: &Server) -> Result<()> {
    let capabilities = server.capabilities().await?;
    let profile = &capabilities["workflows"];
    ensure!(
        capabilities["ready"] == true
            && profile["callbacks"] == true
            && profile["dockerQualified"] == false
            && profile["limits"]["dispatches"] == 1
            && profile["limits"]["callbackTokens"] == 2
            && profile["limits"]["callbackTombstones"] == 64
            && profile["limits"]["callbackCommands"] == 8
            && profile["limits"]["callbackWorkers"] == 1
            && profile["limits"]["callbackRetentionSeconds"] == 86_400,
        "actual service callback capability mismatch"
    );
    let catalog: Catalog = serde_json::from_str(include_str!(
        "../../../../../tests/workflows/callback-cases.json"
    ))?;
    ensure!(
        catalog.cases.len() == 9
            && catalog.source
                == "https://docs.aws.amazon.com/step-functions/latest/dg/connect-lambda.html",
        "invalid callback catalog"
    );
    let client = sfn(server, SECRET);
    deploy(server, &lambda(server)).await?;
    deploy_callbacks(server).await?;
    let mut seen = BTreeSet::new();
    for (function, sdk) in [("CallbackRust", "rust"), ("CallbackPython", "boto3")] {
        for case in &catalog.cases {
            exercise_case(&client, server, case, function, sdk, &mut seen)
                .await
                .with_context(|| format!("{sdk} handler callback case {}", case.id))?;
            eprintln!("{sdk} handler callback case {} passed", case.id);
        }
    }
    ensure!(
        seen.len() == 20,
        "missing actual consumed callback attempts"
    );
    Ok(())
}
