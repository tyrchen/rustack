//! Source-controlled operator example through an owned strict-auth pure service.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use anyhow::{Context, Result, ensure};
use aws_sdk_sfn::{
    Client,
    types::{ExecutionStatus, HistoryEvent, StateEnteredEventDetails, StateExitedEventDetails},
};
use serde_json::{Value, json};

use super::{
    process::{SECRET, Server},
    rust::{history, sfn, terminal},
};

const DEFINITION: &str = include_str!("../../../../examples/workflows/pure.asl.json");

async fn run(client: &Client, machine: &str, positive: bool) -> Result<()> {
    let input = if positive {
        r#"{ "n": 6 }"#
    } else {
        r#"{"n":0}"#
    };
    let started = client
        .start_execution()
        .state_machine_arn(machine)
        .name(if positive { "Positive" } else { "Rejected" })
        .input(input)
        .send()
        .await?;
    let result = terminal(client, started.execution_arn()).await?;
    ensure!(
        result.input() == Some(input),
        "documented workflow lost raw input"
    );
    if positive {
        ensure!(
            result.status() == &ExecutionStatus::Succeeded,
            "positive example failed"
        );
        ensure!(
            serde_json::from_str::<Value>(result.output().context("missing output")?)?
                == json!({"doubled":12}),
            "wrong documented result"
        );
    } else {
        ensure!(
            result.status() == &ExecutionStatus::Failed
                && result.error() == Some("Example.NotPositive"),
            "Fail branch did not reject nonpositive input"
        );
    }
    let events = history(client, started.execution_arn(), false, true).await?;
    for (index, event) in events.iter().enumerate() {
        let previous = i64::try_from(index)?;
        ensure!(
            event.id() == previous.checked_add(1).context("event ID overflow")?
                && event.previous_event_id() == previous,
            "invalid example causal history"
        );
    }
    let kinds: Vec<&str> = events.iter().map(|event| event.r#type().as_str()).collect();
    let expected_kinds: &[&str] = if positive {
        &[
            "ExecutionStarted",
            "PassStateEntered",
            "PassStateExited",
            "ChoiceStateEntered",
            "ChoiceStateExited",
            "WaitStateEntered",
            "WaitStateExited",
            "SucceedStateEntered",
            "SucceedStateExited",
            "ExecutionSucceeded",
        ]
    } else {
        &[
            "ExecutionStarted",
            "PassStateEntered",
            "PassStateExited",
            "ChoiceStateEntered",
            "ChoiceStateExited",
            "FailStateEntered",
            "ExecutionFailed",
        ]
    };
    ensure!(kinds == expected_kinds, "wrong documented event sequence");
    let entered: Vec<&str> = events
        .iter()
        .filter_map(HistoryEvent::state_entered_event_details)
        .map(StateEnteredEventDetails::name)
        .collect();
    let expected: &[&str] = if positive {
        &["Compute", "Positive", "Pause", "Done"]
    } else {
        &["Compute", "Positive", "Reject"]
    };
    ensure!(entered == expected, "wrong example branch/state order");
    let exited: Vec<&str> = events
        .iter()
        .filter_map(HistoryEvent::state_exited_event_details)
        .map(StateExitedEventDetails::name)
        .collect();
    let expected_exits: &[&str] = if positive {
        &["Compute", "Positive", "Pause", "Done"]
    } else {
        &["Compute", "Positive"]
    };
    ensure!(exited == expected_exits, "wrong example state exit names");
    Ok(())
}

pub(super) async fn exercise(server: &Server) -> Result<()> {
    let capabilities = server.capabilities().await?;
    for key in [
        "lambdaRequestResponse",
        "callbacks",
        "dockerQualified",
        "snapshotResume",
    ] {
        ensure!(
            capabilities.pointer(&format!("/workflows/{key}")) == Some(&Value::Bool(false)),
            "pure service advertised unavailable integration"
        );
    }
    ensure!(
        capabilities.pointer("/workflows/snapshot") == Some(&json!("resources-and-history"))
            && capabilities.pointer("/workflows/paginationEpoch")
                == Some(&json!("fresh-on-restart")),
        "wrong pure snapshot capability"
    );
    ensure!(
        server.bootstrap().is_err(),
        "pure fixture exposed a Native bootstrap"
    );
    let client = sfn(server, SECRET);
    let validation = client
        .validate_state_machine_definition()
        .definition(DEFINITION)
        .send()
        .await?;
    ensure!(
        validation.result().as_str() == "OK",
        "documented raw definition did not validate"
    );
    let created = client
        .create_state_machine()
        .name("Documented")
        .role_arn("arn:aws:iam::000000000000:role/Workflows")
        .definition(DEFINITION)
        .send()
        .await?;
    let machine = created.state_machine_arn();
    let captured = client
        .describe_state_machine()
        .state_machine_arn(machine)
        .send()
        .await?;
    ensure!(
        captured.definition() == DEFINITION,
        "documented raw source was normalized or replaced"
    );
    run(&client, machine, true).await?;
    run(&client, machine, false).await?;
    server.readiness().await
}
