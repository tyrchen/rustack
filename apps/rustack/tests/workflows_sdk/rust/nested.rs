//! Independent optimized nested envelopes and actual child Native cleanup.

use super::*;
use crate::process::assert_absent;

fn catalog() -> Result<Value> {
    let value: Value = serde_json::from_str(include_str!(
        "../../../../../tests/workflows/nested-cases.json"
    ))?;
    ensure!(
        value["schemaVersion"] == 1 && value["profile"] == "rustack-standard-jsonpath-nested-v1",
        "invalid nested profile"
    );
    let cases = value["cases"].as_array().context("cases")?;
    ensure!(cases.len() == 6, "missing nested cases");
    let mut names = BTreeSet::new();
    for case in cases {
        ensure!(
            names.insert(case["id"].as_str().context("ID")?)
                && case["suffix"].is_string()
                && case.get("input").is_some()
                && case.get("expected").is_some(),
            "invalid nested case"
        );
    }
    Ok(value)
}
#[test]
fn test_should_validate_independent_nested_catalog() -> Result<()> {
    catalog().map(|_| ())
}
fn definition(child: &str, suffix: &str, input: &Value) -> Value {
    json!({"StartAt":"T","States":{"T":{"Type":"Task","Resource":format!("arn:aws:states:::states:startExecution{suffix}"),"Parameters":{"StateMachineArn":child,"Input":input},"End":true}}})
}
async fn check_history(client: &Client, arn: &str, suffix: &str, child: &str) -> Result<()> {
    let events = history(client, arn, false, true).await?;
    let mut expected = vec![
        "ExecutionStarted",
        "TaskStateEntered",
        "TaskScheduled",
        "TaskStarted",
    ];
    if !suffix.is_empty() {
        expected.push("TaskSubmitted");
    }
    expected.extend(["TaskSucceeded", "TaskStateExited", "ExecutionSucceeded"]);
    ensure!(
        events.iter().map(|e| e.r#type().as_str()).eq(expected),
        "wrong nested history"
    );
    for (index, event) in events.iter().enumerate() {
        ensure!(
            event.id() == i64::try_from(index + 1)?
                && event.previous_event_id() == sequential_predecessor(index)?,
            "invalid nested predecessor"
        );
        if let Some(details) = event.task_submitted_event_details() {
            ensure!(
                details.resource_type() == "states"
                    && details.resource() == format!("startExecution{suffix}"),
                "wrong submitted resource"
            );
            let accepted: Value =
                serde_json::from_str(details.output().context("submitted output")?)?;
            ensure!(accepted["ExecutionArn"] == child, "invented child identity");
        }
    }
    let reverse = history(client, arn, true, false).await?;
    assert_omitted(&reverse)?;
    ensure!(
        events
            .iter()
            .rev()
            .map(|e| (e.id(), e.previous_event_id()))
            .eq(reverse.iter().map(|e| (e.id(), e.previous_event_id()))),
        "changed nested pagination identity"
    );
    Ok(())
}
async fn cases(client: &Client) -> Result<()> {
    let child = create(
        client,
        "SdkNestedEcho",
        &json!({"StartAt":"D","States":{"D":{"Type":"Pass","End":true}}}),
    )
    .await?;
    for case in catalog()?["cases"].as_array().context("cases")? {
        let suffix = case["suffix"].as_str().context("suffix")?;
        let id = case["id"].as_str().context("id")?;
        let machine = create(
            client,
            &format!("SdkNested-{id}"),
            &definition(&child, suffix, &case["input"]),
        )
        .await?;
        let start = client
            .start_execution()
            .state_machine_arn(machine)
            .name("Run")
            .input("{}")
            .send()
            .await?;
        let result = terminal(client, start.execution_arn()).await?;
        ensure!(
            result.status() == &ExecutionStatus::Succeeded,
            "nested {id}: {result:?}"
        );
        let output: Value = serde_json::from_str(result.output().context("output")?)?;
        let child_arn = output["ExecutionArn"].as_str().context("child ARN")?;
        let nested = terminal(client, child_arn).await?;
        ensure!(
            serde_json::from_str::<Value>(nested.output().context("child output")?)?
                == case["expected"],
            "wrong child result"
        );
        ensure!(
            output["StartDate"].as_i64().is_some_and(|millis| nested
                .start_date()
                .to_millis()
                .is_ok_and(|observed| millis.abs_diff(observed) <= 1)),
            "wrong nested millisecond date"
        );
        if !suffix.is_empty() {
            ensure!(
                output["Status"] == "SUCCEEDED"
                    && output["StateMachineArn"] == child
                    && output["InputDetails"] == json!({"Included":true})
                    && output["OutputDetails"] == json!({"Included":true}),
                "wrong optimized envelope"
            );
            if suffix == ".sync" {
                ensure!(
                    serde_json::from_str::<Value>(
                        output["Input"].as_str().context("string input")?
                    )? == case["expected"]
                        && serde_json::from_str::<Value>(
                            output["Output"].as_str().context("string output")?
                        )? == case["expected"],
                    "wrong sync string data"
                );
                if case["input"].is_string() {
                    ensure!(
                        output["Input"] == case["input"],
                        "changed serialized input bytes"
                    );
                }
            } else {
                ensure!(
                    output["Input"] == case["expected"] && output["Output"] == case["expected"],
                    "wrong sync2 JSON data"
                );
            }
        }
        check_history(client, start.execution_arn(), suffix, child_arn).await?;
    }
    Ok(())
}
async fn gated(client: &Client, server: &Server, stop: bool) -> Result<()> {
    let name = if stop {
        "SdkNestedStop"
    } else {
        "SdkNestedCompute"
    };
    let directory = structured::deploy_named(server, &lambda(server), name).await?;
    let child = create(client, &format!("{name}Child"), &json!({"StartAt":"L","States":{"L":{"Type":"Task","Resource":format!("arn:aws:lambda:us-east-1:000000000000:function:{name}"),"End":true}}})).await?;
    let machine = create(
        client,
        &format!("{name}Parent"),
        &definition(&child, ".sync:2", &json!({"n":21})),
    )
    .await?;
    let start = client
        .start_execution()
        .state_machine_arn(machine)
        .input("{}")
        .send()
        .await?;
    let markers = structured::consumed(&directory, 1).await?;
    let (pid, request) = markers.first().context("consumed handler")?;
    let events = history(client, start.execution_arn(), false, true).await?;
    let submitted: Vec<_> = events
        .iter()
        .filter_map(HistoryEvent::task_submitted_event_details)
        .collect();
    ensure!(submitted.len() == 1, "missing unique nested submission");
    let accepted: Value = serde_json::from_str(
        submitted
            .first()
            .context("submitted")?
            .output()
            .context("accepted output")?,
    )?;
    let child_arn = accepted["ExecutionArn"].as_str().context("child ARN")?;
    if stop {
        client
            .stop_execution()
            .execution_arn(start.execution_arn())
            .error("NestedStopped")
            .send()
            .await?;
        ensure!(
            terminal(client, start.execution_arn()).await?.status() == &ExecutionStatus::Aborted,
            "parent not stopped"
        );
        ensure!(
            terminal(client, child_arn).await?.status() == &ExecutionStatus::Aborted,
            "child not stopped"
        );
        timeout(Duration::from_secs(5), async {
            while assert_absent(*pid).await.is_err() {
                sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .context("nested handler PID survived")?;
    } else {
        fs::write(directory.join("release"), b"release").await?;
        let result = terminal(client, start.execution_arn()).await?;
        ensure!(
            result.status() == &ExecutionStatus::Succeeded,
            "nested compute failed: {result:?}"
        );
        let output: Value = serde_json::from_str(result.output().context("nested output")?)?;
        ensure!(
            output["ExecutionArn"] == child_arn
                && output["Output"]["computed"] == 42
                && output["Output"]["pid"].as_u64() == Some(u64::from(*pid))
                && output["Output"]["request_id"] == *request,
            "returned identity differs from actual consumed child"
        );
        check_history(client, start.execution_arn(), ".sync:2", child_arn).await?;
    }
    Ok(())
}
pub(super) async fn exercise(client: &Client, server: &Server) -> Result<()> {
    cases(client).await?;
    for stop in [false, true] {
        gated(client, server, stop).await?;
    }
    Ok(())
}
