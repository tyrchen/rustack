//! Independent structured SDK expectations and consumed-event lifecycle barriers.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use aws_sdk_sfn::types::MapIterationEventDetails;

use super::*;
use crate::process::assert_absent;

fn catalog() -> Result<Value> {
    let catalog: Value = serde_json::from_str(include_str!(
        "../../../../../tests/workflows/structured-cases.json"
    ))?;
    ensure!(
        catalog["schemaVersion"] == 1
            && catalog["profile"] == "rustack-standard-jsonpath-structured-v1",
        "invalid structured catalog profile"
    );
    let cases = catalog["cases"].as_array().context("structured cases")?;
    ensure!(cases.len() == 6, "missing structured cases");
    let mut names = BTreeSet::new();
    for case in cases {
        let id = case["id"].as_str().context("case ID")?;
        ensure!(names.insert(id), "duplicate structured case");
        ensure!(
            case.get("expectedOutput").is_some()
                && case["definition"].is_object()
                && case.get("input").is_some(),
            "incomplete structured expectation"
        );
    }
    Ok(catalog)
}
#[test]
fn test_should_validate_independent_structured_catalog() -> Result<()> {
    catalog().map(|_| ())
}
async fn check_history(client: &Client, arn: &str, round: &str) -> Result<Vec<HistoryEvent>> {
    let events = history(client, arn, false, true).await?;
    ensure!(!events.is_empty(), "missing structured history");
    for (offset, event) in events.iter().enumerate() {
        ensure!(
            event.id() == i64::try_from(offset + 1)?
                && event.previous_event_id() >= 0
                && event.previous_event_id() < event.id(),
            "invalid structured causal predecessor"
        );
    }
    ensure!(
        events.iter().any(|event| event.r#type().as_str() == round),
        "missing {round}"
    );
    let reverse = history(client, arn, true, false).await?;
    ensure!(
        events
            .iter()
            .rev()
            .map(|e| (e.id(), e.previous_event_id()))
            .eq(reverse.iter().map(|e| (e.id(), e.previous_event_id()))),
        "causal links changed under reverse/data-disabled pagination"
    );
    assert_omitted(&reverse)?;
    for event in &events {
        if event.r#type().as_str() == "MapIterationStarted" {
            ensure!(
                events
                    .iter()
                    .find(|parent| parent.id() == event.previous_event_id())
                    .is_some_and(|parent| parent.r#type().as_str() == "MapStateStarted"),
                "Map iteration did not link to its actual round start"
            );
        }
    }
    Ok(events)
}
async fn cases(client: &Client) -> Result<()> {
    for case in catalog()?["cases"].as_array().context("cases")? {
        let id = case["id"].as_str().context("case ID")?;
        let machine = create(client, &format!("SdkStructured-{id}"), &case["definition"]).await?;
        let started = client
            .start_execution()
            .state_machine_arn(machine)
            .name("Run")
            .input(case["input"].to_string())
            .send()
            .await?;
        let result = terminal(client, started.execution_arn()).await?;
        ensure!(
            result.status() == &ExecutionStatus::Succeeded,
            "structured {id}: {result:?}"
        );
        let output: Value = serde_json::from_str(result.output().context("structured output")?)?;
        ensure!(
            output == case["expectedOutput"],
            "structured {id}: {output}"
        );
        let events = check_history(
            client,
            started.execution_arn(),
            case["round"].as_str().context("round")?,
        )
        .await?;
        if let Some(count) = case["taskAttempts"].as_u64() {
            ensure!(
                events
                    .iter()
                    .filter(|e| e.r#type().as_str().ends_with("Scheduled"))
                    .count()
                    == usize::try_from(count)?,
                "wrong fanout invocation count"
            );
        }
    }
    Ok(())
}
async fn deploy_gate(server: &Server, client: &Lambda, stop: bool) -> Result<PathBuf> {
    deploy_named(
        server,
        client,
        if stop {
            "SdkFanoutStop"
        } else {
            "SdkFanoutGate"
        },
    )
    .await
}
pub(super) async fn deploy_named(server: &Server, client: &Lambda, name: &str) -> Result<PathBuf> {
    let directory = server.markers.parent().context("root")?.join(name);
    fs::create_dir(&directory).await?;
    fs::create_dir(directory.join("consumed")).await?;
    let bytes = fs::read(&server.bootstrap).await?;
    let archive = tokio::task::spawn_blocking(move || archive(&bytes)).await??;
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
        .timeout(900)
        .environment(
            Environment::builder()
                .variables("WORKFLOW_MODE", "1")
                .variables(
                    "PROCESS_MARKER_DIR",
                    server.markers.to_str().context("marker path")?,
                )
                .variables(
                    "WORKFLOW_INVOCATION_DIR",
                    directory
                        .join("consumed")
                        .to_str()
                        .context("consumed path")?,
                )
                .variables(
                    "WORKFLOW_GATE_FILE",
                    directory.join("release").to_str().context("release path")?,
                )
                .build(),
        )
        .code(FunctionCode::builder().zip_file(Blob::new(archive)).build())
        .send()
        .await?;
    Ok(directory)
}
pub(super) async fn consumed(directory: &Path, count: usize) -> Result<BTreeSet<(u32, String)>> {
    timeout(Duration::from_secs(10), async {
        loop {
            let mut entries = fs::read_dir(directory.join("consumed")).await?;
            let mut markers = BTreeSet::new();
            while let Some(entry) = entries.next_entry().await? {
                if entry.path().extension().is_none_or(|v| v != "json") {
                    continue;
                }
                ensure!(
                    markers.len() < count && entry.metadata().await?.len() <= 1024,
                    "unexpected marker count/size"
                );
                let marker: Value = serde_json::from_slice(&fs::read(entry.path()).await?)?;
                let pid = u32::try_from(marker["pid"].as_u64().context("PID")?)?;
                let request = marker["request_id"].as_str().context("request ID")?;
                ensure!(
                    pid > 1 && Uuid::parse_str(request)?.get_version_num() == 4,
                    "invalid consumed identity"
                );
                markers.insert((pid, request.to_owned()));
            }
            if markers.len() == count {
                ensure!(
                    markers
                        .iter()
                        .map(|(pid, _)| pid)
                        .collect::<BTreeSet<_>>()
                        .len()
                        == count
                        && markers
                            .iter()
                            .map(|(_, id)| id)
                            .collect::<BTreeSet<_>>()
                            .len()
                            == count,
                    "handlers did not consume independent invocations"
                );
                return Ok(markers);
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .context("two handlers did not consume before release")?
}
async fn gated(client: &Client, server: &Server, stop: bool) -> Result<()> {
    let directory = deploy_gate(server, &lambda(server), stop).await?;
    let function = if stop {
        "SdkFanoutStop"
    } else {
        "SdkFanoutGate"
    };
    let task = json!({"StartAt":"Call","States":{"Call":{"Type":"Task","Resource":format!("arn:aws:lambda:us-east-1:000000000000:function:{function}"),"End":true}}});
    let definition = if stop {
        json!({"StartAt":"P","States":{"P":{"Type":"Parallel","Branches":[{"StartAt":"M","States":{"M":{"Type":"Map","MaxConcurrency":2,"ItemProcessor":task,"End":true}}}],"End":true}}})
    } else {
        json!({"StartAt":"P","States":{"P":{"Type":"Parallel","Branches":[task.clone(),task],"End":true}}})
    };
    let machine = create(
        client,
        if stop {
            "SdkStructuredStop"
        } else {
            "SdkStructuredOverlap"
        },
        &definition,
    )
    .await?;
    let started = client
        .start_execution()
        .state_machine_arn(machine)
        .name("Run")
        .input(if stop {
            r#"[{"n":21},{"n":7}]"#
        } else {
            r#"{"n":21}"#
        })
        .send()
        .await?;
    let markers = consumed(&directory, 2).await?;
    if stop {
        finish_stopped(client, started.execution_arn(), markers).await
    } else {
        fs::write(directory.join("release"), b"release").await?;
        finish_parallel(client, started.execution_arn(), markers).await
    }
}
async fn finish_stopped(
    client: &Client,
    arn: &str,
    markers: BTreeSet<(u32, String)>,
) -> Result<()> {
    client
        .stop_execution()
        .execution_arn(arn)
        .error("SdkStructuredStop")
        .cause("consumed nested tasks")
        .send()
        .await?;
    let result = terminal(client, arn).await?;
    ensure!(
        result.status() == &ExecutionStatus::Aborted && result.error() == Some("SdkStructuredStop"),
        "nested stop did not settle"
    );
    for (pid, _) in markers {
        timeout(Duration::from_secs(5), async {
            loop {
                if assert_absent(pid).await.is_ok() {
                    return;
                }
                sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .context("stopped handler PID survived")?;
    }
    let events = check_history(client, arn, "MapIterationAborted").await?;
    let aborted: Vec<_> = events
        .iter()
        .filter(|e| e.r#type().as_str() == "MapIterationAborted")
        .collect();
    ensure!(
        aborted.len() == 2
            && aborted
                .iter()
                .filter_map(|e| e
                    .map_iteration_aborted_event_details()
                    .map(MapIterationEventDetails::index))
                .collect::<BTreeSet<_>>()
                == BTreeSet::from([0, 1]),
        "missing stopped iteration identities"
    );
    ensure!(
        events
            .last()
            .is_some_and(|e| e.r#type().as_str() == "ExecutionAborted"),
        "iteration aborts were not committed before root termination"
    );
    Ok(())
}
async fn finish_parallel(
    client: &Client,
    arn: &str,
    markers: BTreeSet<(u32, String)>,
) -> Result<()> {
    let result = terminal(client, arn).await?;
    ensure!(
        result.status() == &ExecutionStatus::Succeeded,
        "parallel gate failed"
    );
    let output: Value = serde_json::from_str(result.output().context("parallel output")?)?;
    let values = output.as_array().context("parallel array")?;
    ensure!(values.len() == 2, "wrong parallel result length");
    let mut returned = BTreeSet::new();
    for value in values {
        ensure!(
            value["computed"] == 42,
            "actual handler computation changed"
        );
        returned.insert((
            u32::try_from(value["pid"].as_u64().context("PID")?)?,
            value["request_id"]
                .as_str()
                .context("request ID")?
                .to_owned(),
        ));
    }
    ensure!(
        returned == markers,
        "returned invocations differ from consumed handlers"
    );
    let events = check_history(client, arn, "ParallelStateSucceeded").await?;
    let fork = events
        .iter()
        .find(|e| e.r#type().as_str() == "ParallelStateStarted")
        .context("missing fork")?
        .id();
    let entries: Vec<_> = events
        .iter()
        .filter(|e| e.r#type().as_str() == "TaskStateEntered")
        .collect();
    ensure!(
        entries.len() == 2 && entries.iter().all(|e| e.previous_event_id() == fork),
        "concurrent branches do not share their actual fork predecessor"
    );
    Ok(())
}
pub(super) async fn exercise(client: &Client, server: &Server) -> Result<()> {
    cases(client).await?;
    gated(client, server, false).await?;
    gated(client, server, true).await
}
