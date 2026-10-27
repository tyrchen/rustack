//! Full pure workflow latency, with exact public output/history validation.

use std::time::Duration;

use anyhow::{Context, Result, ensure};
use aws_sdk_sfn::{
    Client,
    operation::describe_execution::DescribeExecutionOutput,
    types::{
        ExecutionStatus, HistoryEvent, HistoryEventType, StateEnteredEventDetails,
        StateExitedEventDetails,
    },
};
use serde_json::{Map, Value, json};
use tokio::time::{Instant, sleep};

use super::{Measurement, SAMPLES, Samples, bounded, input, micros};
use crate::rust::create;

fn definition() -> Value {
    let mut states = Map::new();
    for index in 0..50 {
        let name = format!("S{index:02}");
        let state = if index == 49 {
            json!({"Type":"Succeed"})
        } else if index % 2 == 0 {
            json!({"Type":"Pass","Next":format!("S{:02}",index+1)})
        } else {
            json!({"Type":"Choice","Choices":[{"Variable":"$.payload","IsString":true,"Next":format!("S{:02}",index+1)}],"Default":"S49"})
        };
        states.insert(name, state);
    }
    json!({"StartAt":"S00","States":states})
}
pub(super) async fn terminal(client: &Client, arn: &str) -> Result<DescribeExecutionOutput> {
    bounded(async {
        loop {
            let result = client
                .describe_execution()
                .execution_arn(arn)
                .send()
                .await?;
            if result.status() != &ExecutionStatus::Running {
                return Ok::<_, anyhow::Error>(result);
            }
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
}
async fn check_history(client: &Client, arn: &str) -> Result<()> {
    let history = bounded(
        client
            .get_execution_history()
            .execution_arn(arn)
            .max_results(1000)
            .include_execution_data(false)
            .send(),
    )
    .await?;
    ensure!(
        history.next_token().is_none(),
        "unexpected history pagination"
    );
    validate_history(history.events())
}
pub(super) fn validate_history(events: &[HistoryEvent]) -> Result<()> {
    ensure!(events.len() == 102, "wrong pure history event count");
    for (index, event) in events.iter().enumerate() {
        let previous = i64::try_from(index)?;
        ensure!(
            event.id() == previous.checked_add(1).context("event ID overflow")?
                && event.previous_event_id() == previous,
            "invalid sequential causal history"
        );
        if index == 0 {
            ensure!(
                event.r#type() == &HistoryEventType::ExecutionStarted,
                "missing first start"
            );
        } else if index == 101 {
            ensure!(
                event.r#type() == &HistoryEventType::ExecutionSucceeded,
                "missing final success"
            );
        } else {
            let offset = index.checked_sub(1).context("invalid state event index")?;
            let state = offset / 2;
            let entered = offset % 2 == 0;
            let expected = match (state, entered) {
                (49, true) => HistoryEventType::SucceedStateEntered,
                (49, false) => HistoryEventType::SucceedStateExited,
                (state, true) if state % 2 == 0 => HistoryEventType::PassStateEntered,
                (state, false) if state % 2 == 0 => HistoryEventType::PassStateExited,
                (_, true) => HistoryEventType::ChoiceStateEntered,
                (_, false) => HistoryEventType::ChoiceStateExited,
            };
            let name = if entered {
                event
                    .state_entered_event_details()
                    .map(StateEnteredEventDetails::name)
            } else {
                event
                    .state_exited_event_details()
                    .map(StateExitedEventDetails::name)
            };
            ensure!(
                event.r#type() == &expected && name == Some(format!("S{state:02}").as_str()),
                "wrong state event kind/name"
            );
        }
    }
    Ok(())
}

pub(super) async fn measure(client: &Client) -> Result<Measurement> {
    let input = input();
    ensure!(input.len() == 1024, "incorrect workload input size");
    let machine = bounded(create(client, "Latency", &definition())).await?;
    let mut samples = Vec::with_capacity(SAMPLES);
    for index in 0..SAMPLES {
        let began = Instant::now();
        let arn = bounded(Box::pin(async {
            let started = client
                .start_execution()
                .state_machine_arn(&machine)
                .name(format!("Latency{index}"))
                .input(&input)
                .send()
                .await?;
            let arn = started.execution_arn().to_owned();
            let result = terminal(client, &arn).await?;
            ensure!(
                result.status() == &ExecutionStatus::Succeeded
                    && result.input() == Some(input.as_str())
                    && result.output() == Some(input.as_str()),
                "wrong pure workflow result"
            );
            Ok::<_, anyhow::Error>(arn)
        }))
        .await?;
        samples.push(micros(began.elapsed())?);
        check_history(client, &arn).await?;
    }
    Ok(Measurement::PureLatency {
        state_count: 50,
        input_bytes: input.len(),
        executions: SAMPLES,
        budget_micros: 100_000,
        latency: Samples::new(samples)?,
    })
}
