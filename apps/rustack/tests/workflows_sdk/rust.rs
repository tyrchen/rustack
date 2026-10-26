//! Ordinary signed Rust SDK requests against the actual executable.

use std::{
    collections::BTreeSet,
    io::{Cursor, Write},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use aws_credential_types::Credentials;
use aws_sdk_lambda::{
    Client as Lambda,
    config::{BehaviorVersion, Region, retry::RetryConfig, timeout::TimeoutConfig},
    primitives::Blob,
    types::{Architecture, Environment, FunctionCode, Runtime},
};
use aws_sdk_sfn::{
    Client,
    error::ProvideErrorMetadata,
    operation::{
        describe_execution::DescribeExecutionOutput,
        describe_state_machine::DescribeStateMachineOutput, start_execution::StartExecutionOutput,
    },
    types::{ExecutionStatus, HistoryEvent, Tag, ValidateStateMachineDefinitionSeverity},
};
use serde_json::{Value, json};
use tokio::{
    fs,
    time::{sleep, timeout},
};
use uuid::Uuid;
use zip::{ZipWriter, write::SimpleFileOptions};

use super::process::{ACCESS, SECRET, Server};

#[path = "rust/nested.rs"]
mod nested;
#[path = "rust/structured.rs"]
mod structured;

const ROLE: &str = "arn:aws:iam::000000000000:role/Workflows";
const PREFIX: &str = "arn:aws:states:us-east-1:000000000000:";
fn credentials() -> Credentials {
    Credentials::new(ACCESS, SECRET, None, None, "workflow-local")
}
fn timeouts() -> TimeoutConfig {
    TimeoutConfig::builder()
        .connect_timeout(Duration::from_secs(2))
        .read_timeout(Duration::from_secs(15))
        .operation_timeout(Duration::from_secs(15))
        .build()
}
fn sfn(server: &Server, secret: &str) -> Client {
    Client::from_conf(
        aws_sdk_sfn::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .endpoint_url(&server.endpoint)
            .credentials_provider(Credentials::new(
                ACCESS,
                secret,
                None,
                None,
                "workflow-local",
            ))
            .retry_config(RetryConfig::disabled())
            .timeout_config(timeouts())
            .build(),
    )
}
fn lambda(server: &Server) -> Lambda {
    Lambda::from_conf(
        aws_sdk_lambda::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .endpoint_url(&server.endpoint)
            .credentials_provider(credentials())
            .retry_config(RetryConfig::disabled())
            .timeout_config(timeouts())
            .build(),
    )
}
fn archive(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer.start_file(
        "bootstrap",
        SimpleFileOptions::default().unix_permissions(0o755),
    )?;
    writer.write_all(bytes)?;
    Ok(writer.finish()?.into_inner())
}
async fn deploy(server: &Server, client: &Lambda) -> Result<()> {
    let bytes = fs::read(&server.bootstrap).await?;
    let archive = tokio::task::spawn_blocking(move || archive(&bytes)).await??;
    for (name, variables, seconds) in [
        ("SdkCompute", vec![("WORKFLOW_MODE", "1")], 5),
        (
            "SdkRetry",
            vec![("WORKFLOW_MODE", "1"), ("WORKFLOW_FAIL_ONCE", "1")],
            5,
        ),
        ("SdkFail", vec![("FAIL_MODE", "panic")], 5),
        ("SdkSlow", vec![("SLEEP_SECS", "5")], 5),
        ("SdkFunctionTimeout", vec![("SLEEP_SECS", "5")], 1),
    ] {
        let mut environment = Environment::builder().variables(
            "PROCESS_MARKER_DIR",
            server.markers.to_str().context("marker path")?,
        );
        for (key, value) in variables {
            environment = environment.variables(key, value);
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
            .timeout(seconds)
            .environment(environment.build())
            .code(
                FunctionCode::builder()
                    .zip_file(Blob::new(archive.clone()))
                    .build(),
            )
            .send()
            .await?;
    }
    let gates = server
        .markers
        .parent()
        .context("private root")?
        .join("gates");
    client
        .create_function()
        .function_name("SdkGate")
        .role(ROLE)
        .runtime(Runtime::Providedal2023)
        .handler("bootstrap")
        .architectures(if cfg!(target_arch = "aarch64") {
            Architecture::Arm64
        } else {
            Architecture::X8664
        })
        .timeout(900)
        .environment(
            Environment::builder()
                .variables("WORKFLOW_MODE", "1")
                .variables(
                    "PROCESS_MARKER_DIR",
                    server.markers.to_str().context("marker path")?,
                )
                .variables(
                    "INVOCATION_FILE",
                    gates.join("entered").to_str().context("gate path")?,
                )
                .variables(
                    "WORKFLOW_GATE_FILE",
                    gates.join("release").to_str().context("gate path")?,
                )
                .build(),
        )
        .code(
            FunctionCode::builder()
                .zip_file(Blob::new(archive.clone()))
                .build(),
        )
        .send()
        .await?;
    check_lambda_compute(client).await
}
async fn check_lambda_compute(client: &Lambda) -> Result<()> {
    let invoked = client
        .invoke()
        .function_name("SdkCompute")
        .payload(Blob::new(br#"{"n":21}"#.to_vec()))
        .send()
        .await?;
    ensure!(
        invoked.status_code() == 200 && invoked.function_error().is_none(),
        "actual Lambda SDK invoke failed"
    );
    let result: Value =
        serde_json::from_slice(invoked.payload().context("Lambda output")?.as_ref())?;
    ensure!(
        result["computed"] == 42 && result["pid"].as_u64().is_some_and(|pid| pid > 1),
        "real bootstrap computation missing"
    );
    Ok(())
}

async fn create(client: &Client, name: &str, definition: &Value) -> Result<String> {
    let response = client
        .create_state_machine()
        .name(name)
        .role_arn(ROLE)
        .definition(definition.to_string())
        .send()
        .await?;
    ensure!(
        response
            .state_machine_arn()
            .starts_with(&format!("{PREFIX}stateMachine:")),
        "wrong machine scope"
    );
    let now = i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())?;
    ensure!(
        (response.creation_date().secs() - now).abs() < 60,
        "incorrect native SDK timestamp unit"
    );
    Ok(response.state_machine_arn().to_owned())
}
async fn terminal(client: &Client, arn: &str) -> Result<DescribeExecutionOutput> {
    timeout(Duration::from_secs(20), async {
        loop {
            let value = client
                .describe_execution()
                .execution_arn(arn)
                .send()
                .await?;
            if value.status() != &ExecutionStatus::Running {
                return Ok(value);
            }
            sleep(Duration::from_millis(15)).await;
        }
    })
    .await
    .context("execution did not terminate")?
}
async fn history(
    client: &Client,
    arn: &str,
    reverse: bool,
    data: bool,
) -> Result<Vec<HistoryEvent>> {
    let mut pages = client
        .get_execution_history()
        .execution_arn(arn)
        .max_results(7)
        .reverse_order(reverse)
        .include_execution_data(data)
        .into_paginator()
        .send();
    let mut events = Vec::new();
    let mut count = 0;
    while let Some(page) = pages.try_next().await? {
        count += 1;
        ensure!(count <= 200, "unbounded history page count");
        ensure!(events.len() < 1000, "unbounded history paginator");
        events.extend(page.events);
    }
    Ok(events)
}
fn validated_catalog() -> Result<Value> {
    let value: Value =
        serde_json::from_str(include_str!("../../../../tests/workflows/sdk-cases.json"))?;
    ensure!(
        value["schemaVersion"] == 1 && value["profile"] == "rustack-standard-jsonpath-v1",
        "unsupported SDK catalog"
    );
    let cases = value["cases"].as_array().context("SDK case collection")?;
    ensure!(cases.len() == 17, "required SDK case manifest changed");
    let mut names = BTreeSet::new();
    for case in cases {
        let object = case.as_object().context("SDK case object")?;
        for key in object.keys() {
            ensure!(
                [
                    "id",
                    "definition",
                    "input",
                    "expectedOutput",
                    "uuidField",
                    "optimizedComputed",
                    "taskAttempts",
                    "expectedStatus",
                    "expectedError",
                    "expectedCause",
                    "largeInputBytes",
                    "evidence"
                ]
                .contains(&key.as_str()),
                "unknown SDK expectation field"
            );
        }
        let id = case["id"].as_str().context("case ID")?;
        ensure!(
            !id.is_empty() && id.len() <= 48 && names.insert(id),
            "missing/duplicate case ID"
        );
        ensure!(
            case["definition"].is_object() && object.contains_key("input"),
            "missing SDK definition/input"
        );
        let outputs = usize::from(object.contains_key("expectedOutput"))
            + usize::from(object.contains_key("optimizedComputed"))
            + usize::from(object.contains_key("expectedError"));
        ensure!(
            outputs == 1,
            "every SDK case requires exactly one outcome oracle"
        );
        if object.contains_key("expectedError") {
            ensure!(
                ["FAILED", "TIMED_OUT"]
                    .contains(&case["expectedStatus"].as_str().context("failure status")?),
                "invalid failure oracle"
            );
        }
        if let Some(number) = case.get("optimizedComputed") {
            ensure!(number.as_i64().is_some(), "invalid optimized oracle");
        }
        if let Some(attempts) = case.get("taskAttempts") {
            ensure!(
                attempts.as_u64().is_some_and(|n| (1..=10).contains(&n)),
                "invalid attempt oracle"
            );
        }
        for key in ["uuidField", "expectedError", "expectedCause"] {
            if let Some(value) = case.get(key) {
                ensure!(value.is_string(), "invalid string expectation");
            }
        }
        if let Some(size) = case.get("largeInputBytes") {
            ensure!(
                size.as_u64().is_some_and(|size| size <= 200_000) && case["input"].is_object(),
                "invalid large-input fixture"
            );
        }
        let sources = case["evidence"]["sources"]
            .as_array()
            .context("case provenance")?;
        ensure!(!sources.is_empty(), "missing independent source");
        for source in sources {
            ensure!(
                value["sources"]
                    .get(source.as_str().context("source identifier")?)
                    .is_some(),
                "unknown fixture source"
            );
        }
    }
    Ok(value)
}
#[test]
fn test_should_require_independent_expected_outcomes_for_all_sdk_cases() -> Result<()> {
    validated_catalog().map(|_| ())
}
async fn catalog(client: &Client) -> Result<()> {
    let catalog = validated_catalog()?;
    for case in catalog["cases"].as_array().context("SDK cases")? {
        let id = case["id"].as_str().context("case id")?;
        let mut input = case["input"].clone();
        if let Some(size) = case["largeInputBytes"].as_u64() {
            input["large"] = Value::String("x".repeat(usize::try_from(size)?));
        }
        let machine = create(client, &format!("SdkCase-{id}"), &case["definition"]).await?;
        let raw = input.to_string();
        let started = client
            .start_execution()
            .state_machine_arn(&machine)
            .name("Run")
            .input(&raw)
            .send()
            .await?;
        let result = terminal(client, started.execution_arn())
            .await
            .with_context(|| format!("case {id}"))?;
        ensure!(
            result.status().as_str() == case["expectedStatus"].as_str().unwrap_or("SUCCEEDED"),
            "case {id}: status {} error {:?}",
            result.status().as_str(),
            result.error()
        );
        ensure!(
            result.input() == Some(raw.as_str()),
            "case {id}: raw input changed"
        );
        ensure!(
            result.start_date() == started.start_date()
                && result
                    .stop_date()
                    .is_some_and(|date| date >= result.start_date()),
            "case {id}: SDK dates inconsistent"
        );
        verify_output(case, &result)?;
        let events = history(client, started.execution_arn(), false, true).await?;
        ensure!(!events.is_empty(), "case {id}: no actual history");
        for (offset, event) in events.iter().enumerate() {
            ensure!(
                event.id() == i64::try_from(offset + 1)?
                    && event.previous_event_id() == event.id() - 1
                    && event.timestamp() >= result.start_date(),
                "case {id}: broken causal history"
            );
        }
        let terminal_kind = match result.status().as_str() {
            "SUCCEEDED" => "ExecutionSucceeded",
            "TIMED_OUT" => "ExecutionTimedOut",
            _ => "ExecutionFailed",
        };
        ensure!(
            events
                .last()
                .is_some_and(|event| event.r#type().as_str() == terminal_kind),
            "missing terminal history event"
        );
        if let Some(attempts) = case["taskAttempts"].as_u64() {
            ensure!(
                u64::try_from(
                    events
                        .iter()
                        .filter(|event| event.r#type().as_str().ends_with("Scheduled"))
                        .count()
                )? == attempts,
                "hidden or missing business retries"
            );
        }
        if id == "optimized" {
            pagination(client, started.execution_arn(), &events).await?;
        }
    }
    Ok(())
}
fn verify_output(case: &Value, result: &DescribeExecutionOutput) -> Result<()> {
    let id = case["id"].as_str().context("case id")?;
    if let Some(expected) = case.get("expectedOutput") {
        let mut output: Value =
            serde_json::from_str(result.output().context("successful output")?)?;
        if let Some(field) = case["uuidField"].as_str() {
            let uuid = output
                .as_object_mut()
                .context("UUID output object")?
                .remove(field)
                .context("UUID output")?;
            ensure!(
                Uuid::parse_str(uuid.as_str().context("UUID string")?)?.get_version_num() == 4,
                "non-v4 UUID"
            );
        }
        ensure!(&output == expected, "case {id}: wrong output {output}");
    }
    if let Some(expected) = case["optimizedComputed"].as_i64() {
        let output: Value = serde_json::from_str(result.output().context("optimized output")?)?;
        ensure!(
            output["Payload"]["computed"] == expected
                && output["StatusCode"] == 200
                && output["SdkHttpMetadata"]["HttpStatusCode"] == 200
                && output["ExecutedVersion"] == "$LATEST",
            "bad optimized envelope"
        );
        ensure!(
            output["SdkResponseMetadata"]["RequestId"] == output["Payload"]["request_id"],
            "invented invocation request ID"
        );
        Uuid::parse_str(
            output["Payload"]["request_id"]
                .as_str()
                .context("actual request ID")?,
        )?;
        ensure!(
            output["Payload"]["pid"].as_u64().is_some_and(|pid| pid > 1),
            "no actual process"
        );
    }
    if let Some(error) = case["expectedError"].as_str() {
        ensure!(
            result.error() == Some(error),
            "case {id}: wrong error {:?}",
            result.error()
        );
    }
    if let Some(cause) = case["expectedCause"].as_str() {
        ensure!(
            result.cause() == Some(cause),
            "case {id}: changed Unicode cause"
        );
    }
    Ok(())
}
fn assert_omitted(events: &[HistoryEvent]) -> Result<()> {
    for event in events {
        let inputs = [
            event
                .execution_started_event_details()
                .and_then(|d| d.input()),
            event.state_entered_event_details().and_then(|d| d.input()),
            event
                .lambda_function_scheduled_event_details()
                .and_then(|d| d.input()),
        ];
        let outputs = [
            event
                .task_submitted_event_details()
                .and_then(|d| d.output()),
            event
                .execution_succeeded_event_details()
                .and_then(|d| d.output()),
            event.state_exited_event_details().and_then(|d| d.output()),
            event
                .lambda_function_succeeded_event_details()
                .and_then(|d| d.output()),
            event
                .task_succeeded_event_details()
                .and_then(|d| d.output()),
        ];
        let metadata_present = [
            event
                .task_submitted_event_details()
                .and_then(|d| d.output_details())
                .is_some(),
            event
                .execution_started_event_details()
                .and_then(|d| d.input_details())
                .is_some(),
            event
                .state_entered_event_details()
                .and_then(|d| d.input_details())
                .is_some(),
            event
                .lambda_function_scheduled_event_details()
                .and_then(|d| d.input_details())
                .is_some(),
            event
                .execution_succeeded_event_details()
                .and_then(|d| d.output_details())
                .is_some(),
            event
                .state_exited_event_details()
                .and_then(|d| d.output_details())
                .is_some(),
            event
                .lambda_function_succeeded_event_details()
                .and_then(|d| d.output_details())
                .is_some(),
            event
                .task_succeeded_event_details()
                .and_then(|d| d.output_details())
                .is_some(),
        ];
        ensure!(
            inputs.iter().all(Option::is_none)
                && outputs.iter().all(Option::is_none)
                && metadata_present.iter().all(|present| !present)
                && event.task_scheduled_event_details().is_none(),
            "execution data was not omitted"
        );
    }
    Ok(())
}
async fn pagination(client: &Client, arn: &str, events: &[HistoryEvent]) -> Result<()> {
    let reverse = history(client, arn, true, false).await?;
    ensure!(
        reverse
            .iter()
            .map(HistoryEvent::id)
            .eq(events.iter().rev().map(HistoryEvent::id)),
        "reverse traversal changed IDs"
    );
    ensure!(
        reverse
            .iter()
            .zip(events.iter().rev())
            .all(|(left, right)| left.previous_event_id() == right.previous_event_id()),
        "reverse traversal changed causal links"
    );
    assert_omitted(&reverse)?;
    let first = client
        .get_execution_history()
        .execution_arn(arn)
        .max_results(1)
        .send()
        .await?;
    let token = first.next_token().context("history token")?;
    let next = client
        .get_execution_history()
        .execution_arn(arn)
        .max_results(1)
        .next_token(token)
        .send()
        .await?;
    let replay = client
        .get_execution_history()
        .execution_arn(arn)
        .max_results(1)
        .next_token(token)
        .send()
        .await?;
    ensure!(
        next.events().first().map(HistoryEvent::id)
            == replay.events().first().map(HistoryEvent::id),
        "token was destructive"
    );
    let error = client
        .get_execution_history()
        .execution_arn(arn)
        .max_results(2)
        .next_token(token)
        .send()
        .await
        .err()
        .context("query-bound token accepted")?;
    ensure!(
        error
            .as_service_error()
            .and_then(ProvideErrorMetadata::code)
            == Some("InvalidToken"),
        "wrong cursor query error"
    );
    let error = client
        .get_execution_history()
        .execution_arn(arn)
        .next_token("0".repeat(64))
        .send()
        .await
        .err()
        .context("forged token accepted")?;
    ensure!(
        error
            .as_service_error()
            .and_then(ProvideErrorMetadata::code)
            == Some("InvalidToken"),
        "wrong forged token error"
    );
    Ok(())
}
fn pass(value: &str) -> Value {
    json!({"StartAt":"Done","States":{"Done":{"Type":"Pass","Result":{"revision":value},"End":true}}})
}
async fn tags(client: &Client, machine: &str) -> Result<()> {
    client
        .tag_resource()
        .resource_arn(machine)
        .tags(Tag::builder().key("owner").value("sdk").build())
        .send()
        .await?;
    let tags = client
        .list_tags_for_resource()
        .resource_arn(machine)
        .send()
        .await?;
    ensure!(
        tags.tags()
            .iter()
            .any(|tag| tag.key() == Some("owner") && tag.value() == Some("sdk")),
        "tag missing"
    );
    client
        .untag_resource()
        .resource_arn(machine)
        .tag_keys("owner")
        .send()
        .await?;
    ensure!(
        client
            .list_tags_for_resource()
            .resource_arn(machine)
            .send()
            .await?
            .tags()
            .is_empty(),
        "tag removal failed"
    );
    Ok(())
}
const RAW: &str = " {\"n\":1}";
async fn duplicate(client: &Client, machine: &str) -> Result<StartExecutionOutput> {
    let start = || {
        client
            .start_execution()
            .state_machine_arn(machine)
            .name("Same")
            .input(RAW)
            .send()
    };
    let (left, right) = tokio::join!(start(), start());
    let left = left?;
    let right = right?;
    ensure!(
        left.execution_arn() == right.execution_arn() && left.start_date() == right.start_date(),
        "RUNNING start not idempotent"
    );
    for (input, code) in [
        ("{}", "ExecutionAlreadyExists"),
        ("{", "InvalidExecutionInput"),
    ] {
        let error = client
            .start_execution()
            .state_machine_arn(machine)
            .name("Same")
            .input(input)
            .send()
            .await
            .err()
            .context("invalid duplicate accepted")?;
        ensure!(
            error
                .as_service_error()
                .and_then(ProvideErrorMetadata::code)
                == Some(code),
            "wrong duplicate validation error"
        );
    }
    Ok(left)
}
async fn consumed_gate(server: &Server) -> Result<()> {
    timeout(Duration::from_secs(15), async {
        let path = server
            .markers
            .parent()
            .context("private root")?
            .join("gates/entered");
        loop {
            if let Ok(bytes) = fs::read(&path).await {
                ensure!(bytes.len() <= 512, "oversized consumed-event marker");
                if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                    ensure!(
                        value["pid"].as_u64().is_some_and(|pid| pid > 1),
                        "invalid consumed PID"
                    );
                    Uuid::parse_str(value["request_id"].as_str().context("actual request ID")?)?;
                    return Ok(());
                }
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .context("handler consumption barrier expired")?
}
async fn revisions(
    client: &Client,
    server: &Server,
    machine: &str,
    original: &DescribeStateMachineOutput,
    first: &StartExecutionOutput,
) -> Result<()> {
    consumed_gate(server).await?;
    let original_id = original
        .revision_id()
        .context("missing original revision UUID")?;
    Uuid::parse_str(original_id)?;
    let update = client
        .update_state_machine()
        .state_machine_arn(machine)
        .definition(pass("new").to_string())
        .send()
        .await?;
    let update_id = update
        .revision_id()
        .context("missing update revision UUID")?;
    Uuid::parse_str(update_id)?;
    ensure!(update_id != original_id, "update did not advance revision");
    let captured = client
        .describe_state_machine_for_execution()
        .execution_arn(first.execution_arn())
        .send()
        .await?;
    let captured_id = captured
        .revision_id()
        .context("missing captured revision UUID")?;
    Uuid::parse_str(captured_id)?;
    ensure!(
        captured.definition() == original.definition()
            && captured_id == original_id
            && captured.update_date() == original.creation_date(),
        "captured revision/date drifted"
    );
    let new = client
        .start_execution()
        .state_machine_arn(machine)
        .name("New")
        .send()
        .await?;
    ensure!(
        terminal(client, new.execution_arn()).await?.output() == Some("{\"revision\":\"new\"}"),
        "new definition not used"
    );
    fs::write(
        server
            .markers
            .parent()
            .context("private root")?
            .join("gates/release"),
        b"release",
    )
    .await?;
    ensure!(
        terminal(client, first.execution_arn()).await?.output() == Some("{\"revision\":\"old\"}"),
        "running definition changed"
    );
    let error = client
        .start_execution()
        .state_machine_arn(machine)
        .name("Same")
        .input(RAW)
        .send()
        .await
        .err()
        .context("closed name reused")?;
    ensure!(
        error
            .as_service_error()
            .and_then(ProvideErrorMetadata::code)
            == Some("ExecutionAlreadyExists"),
        "wrong closed-name error"
    );
    Ok(())
}
async fn stop(client: &Client, machine: &str) -> Result<()> {
    let wait =
        json!({"StartAt":"Wait","States":{"Wait":{"Type":"Wait","Seconds":3600,"End":true}}});
    client
        .update_state_machine()
        .state_machine_arn(machine)
        .definition(wait.to_string())
        .send()
        .await?;
    let started = client
        .start_execution()
        .state_machine_arn(machine)
        .name("Stop")
        .send()
        .await?;
    client
        .stop_execution()
        .execution_arn(started.execution_arn())
        .error("")
        .cause("quoted \"stop\" 🙂")
        .send()
        .await?;
    let result = terminal(client, started.execution_arn()).await?;
    ensure!(
        result.status() == &ExecutionStatus::Aborted
            && result.error() == Some("")
            && result.cause() == Some("quoted \"stop\" 🙂"),
        "Stop fields changed"
    );
    Ok(())
}
async fn retained_lists(client: &Client, machine: &str, first: &str) -> Result<()> {
    let page = client
        .list_executions()
        .state_machine_arn(machine)
        .max_results(1)
        .send()
        .await?;
    let mut arns: BTreeSet<String> = page
        .executions()
        .iter()
        .map(|item| item.execution_arn().to_owned())
        .collect();
    let mut token = page.next_token().map(str::to_owned);
    let mut seen = BTreeSet::new();
    client
        .update_state_machine()
        .state_machine_arn(machine)
        .definition(pass("late").to_string())
        .send()
        .await?;
    let late = client
        .start_execution()
        .state_machine_arn(machine)
        .name("Late")
        .send()
        .await?;
    terminal(client, late.execution_arn()).await?;
    let first_history = client
        .get_execution_history()
        .execution_arn(first)
        .max_results(1)
        .send()
        .await?;
    client
        .delete_state_machine()
        .state_machine_arn(machine)
        .send()
        .await?;
    while let Some(next) = token {
        ensure!(
            seen.len() < 10 && seen.insert(next.clone()),
            "unbounded execution cursor"
        );
        let page = client
            .list_executions()
            .state_machine_arn(machine)
            .max_results(1)
            .next_token(next)
            .send()
            .await?;
        arns.extend(
            page.executions()
                .iter()
                .map(|item| item.execution_arn().to_owned()),
        );
        token = page.next_token().map(str::to_owned);
    }
    ensure!(
        arns.len() == 3 && !arns.contains(late.execution_arn()),
        "execution-list capture drifted after delete"
    );
    client
        .get_execution_history()
        .execution_arn(first)
        .max_results(1)
        .next_token(
            first_history
                .next_token()
                .context("retained history cursor")?,
        )
        .send()
        .await?;
    ensure!(
        client
            .describe_execution()
            .execution_arn(first)
            .send()
            .await?
            .input()
            == Some(RAW),
        "execution lost after deletion"
    );
    Ok(())
}
async fn list_and_validate(client: &Client) -> Result<()> {
    let mut pages = client
        .list_state_machines()
        .max_results(2)
        .into_paginator()
        .send();
    let mut count = 0;
    let mut names = BTreeSet::new();
    while let Some(page) = pages.try_next().await? {
        count += 1;
        ensure!(count <= 64, "unbounded machine pages");
        names.extend(
            page.state_machines()
                .iter()
                .map(|item| item.name().to_owned()),
        );
    }
    ensure!(
        names.contains("SdkCase-optimized") && !names.contains("SdkControl"),
        "resource list/delete projection failed"
    );
    let valid = client
        .validate_state_machine_definition()
        .definition(pass("valid").to_string())
        .severity(ValidateStateMachineDefinitionSeverity::Warning)
        .send()
        .await?;
    ensure!(valid.result().as_str() == "OK", "valid definition rejected");
    for definition in [
        json!({"StartAt":"Missing","States":{}}),
        json!({"StartAt":"Run","States":{"Run":{"Type":"Pass","InputPath":"$[","End":true}}}),
        json!({"StartAt":"Run","States":{"Run":{"Type":"Pass","Parameters":{"duplicate":1,"duplicate.$":"$.n"},"End":true}}}),
    ] {
        let invalid = client
            .validate_state_machine_definition()
            .definition(definition.to_string())
            .send()
            .await?;
        ensure!(
            invalid.result().as_str() == "FAIL" && !invalid.diagnostics().is_empty(),
            "invalid definition accepted"
        );
    }
    let unsupported = client
        .create_activity()
        .name("unsupported")
        .send()
        .await
        .err()
        .context("unsupported operation accepted")?;
    ensure!(
        unsupported
            .as_service_error()
            .and_then(ProvideErrorMetadata::code)
            == Some("UnsupportedOperation"),
        "unsupported operation disguised"
    );
    Ok(())
}
async fn controls(client: &Client, server: &Server) -> Result<()> {
    let old = json!({"StartAt":"Gate","States":{"Gate":{"Type":"Task","Resource":"arn:aws:lambda:us-east-1:000000000000:function:SdkGate","Parameters":{"n":21},"ResultSelector":{"revision":"old"},"End":true}}});
    let machine = create(client, "SdkControl", &old).await?;
    let described = client
        .describe_state_machine()
        .state_machine_arn(&machine)
        .send()
        .await?;
    let original = old.to_string();
    ensure!(described.definition() == original, "definition changed");
    tags(client, &machine).await?;
    let first = duplicate(client, &machine).await?;
    revisions(client, server, &machine, &described, &first).await?;
    stop(client, &machine).await?;
    retained_lists(client, &machine, first.execution_arn()).await?;
    list_and_validate(client).await
}
pub(super) async fn exercise(server: &Server) -> Result<()> {
    let client = sfn(server, SECRET);
    let bad = sfn(server, "wrong-local-secret")
        .list_state_machines()
        .send()
        .await
        .err()
        .context("strict signing not enforced")?;
    ensure!(
        bad.as_service_error().and_then(ProvideErrorMetadata::code)
            == Some("AccessDeniedException"),
        "wrong auth error"
    );
    deploy(server, &lambda(server)).await?;
    catalog(&client).await?;
    controls(&client, server).await?;
    nested::exercise(&client, server).await?;
    structured::exercise(&client, server).await?;
    Ok(())
}

pub(super) async fn nested_only(server: &Server) -> Result<()> {
    nested::exercise(&sfn(server, SECRET), server).await
}
