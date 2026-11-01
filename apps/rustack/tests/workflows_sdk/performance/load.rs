//! Real running loops, bounded history exhaustion and one-slot Stop cycles.

use std::time::Duration;

use anyhow::{Context, Result, ensure};
use aws_sdk_sfn::{
    Client,
    types::{ExecutionStatus, HistoryEventType},
};
use serde::Serialize;
use serde_json::json;
use tokio::time::{Instant, sleep, timeout};

use super::{Measurement, SAMPLES, Samples, bounded, input, latency, micros};
use crate::{process::Server, rust::create};

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ExhaustionProbes {
    probe_count: u32,
    live_probe_count: u32,
    max_readiness_micros: u64,
}
impl ExhaustionProbes {
    pub(super) fn record(
        &mut self,
        before: &ExecutionStatus,
        after: &ExecutionStatus,
        elapsed: u64,
    ) -> Result<()> {
        self.probe_count = self
            .probe_count
            .checked_add(1)
            .context("probe count overflow")?;
        ensure!(self.probe_count <= 10_000, "unbounded exhaustion probes");
        if before == &ExecutionStatus::Running && after == &ExecutionStatus::Running {
            self.live_probe_count = self
                .live_probe_count
                .checked_add(1)
                .context("live probe count overflow")?;
        }
        self.max_readiness_micros = self.max_readiness_micros.max(elapsed);
        Ok(())
    }
    pub(super) fn check(&self) -> Result<()> {
        ensure!(
            self.live_probe_count > 0,
            "no readiness probe overlapped live exhaustion work"
        );
        ensure!(
            self.max_readiness_micros < 2_000_000,
            "exhaustion readiness budget exceeded"
        );
        Ok(())
    }
}

async fn start(client: &Client, machine: &str, name: String, input: &str) -> Result<String> {
    let result = bounded(
        client
            .start_execution()
            .state_machine_arn(machine)
            .name(name)
            .input(input)
            .send(),
    )
    .await?;
    Ok(result.execution_arn().to_owned())
}
async fn live_history(client: &Client, arn: &str) -> Result<()> {
    bounded(async {
        loop {
            let result = client
                .get_execution_history()
                .execution_arn(arn)
                .max_results(5)
                .include_execution_data(false)
                .send()
                .await?;
            if result.events().len() >= 5 {
                let state = client
                    .describe_execution()
                    .execution_arn(arn)
                    .send()
                    .await?;
                ensure!(
                    state.status() == &ExecutionStatus::Running,
                    "loop ended before Stop"
                );
                return Ok::<_, anyhow::Error>(());
            }
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
}
async fn exhaust(
    client: &Client,
    server: &Server,
    machine: &str,
    input: &str,
) -> Result<(i64, ExhaustionProbes)> {
    let arn = start(client, machine, "Exhaust".to_owned(), input).await?;
    let mut probes = ExhaustionProbes::default();
    let began = Instant::now();
    let result = timeout(Duration::from_secs(20), async {
        loop {
            let before = bounded(client.describe_execution().execution_arn(&arn).send()).await?;
            if before.status() != &ExecutionStatus::Running {
                return Ok::<_, anyhow::Error>(before);
            }
            let began = Instant::now();
            server.readiness().await?;
            let elapsed = micros(began.elapsed())?;
            let state = bounded(client.describe_execution().execution_arn(&arn).send()).await?;
            probes.record(before.status(), state.status(), elapsed)?;
            if state.status() != &ExecutionStatus::Running {
                return Ok::<_, anyhow::Error>(state);
            }
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .context("loop did not reach its configured history limit")??;
    ensure!(
        began.elapsed() < Duration::from_secs(20),
        "late exhaustion result"
    );
    ensure!(
        result.status() == &ExecutionStatus::Failed
            && result.error() == Some("States.HistoryLimitExceeded"),
        "loop did not fail at history bound"
    );
    let history = bounded(
        client
            .get_execution_history()
            .execution_arn(&arn)
            .reverse_order(true)
            .include_execution_data(false)
            .max_results(1)
            .send(),
    )
    .await?;
    let event = history
        .events()
        .first()
        .context("missing reserved terminal event")?;
    ensure!(
        event.r#type() == &HistoryEventType::ExecutionFailed && (5..=25_000).contains(&event.id()),
        "invalid bounded terminal history"
    );
    ensure!(
        event
            .execution_failed_event_details()
            .and_then(|details| details.error())
            == Some("States.HistoryLimitExceeded"),
        "terminal history disagrees with outcome"
    );
    probes.check()?;
    Ok((event.id(), probes))
}
pub(super) async fn measure(client: &Client, server: &Server) -> Result<Measurement> {
    let definition = json!({"StartAt":"Choose","States":{
        "Choose":{"Type":"Choice","Choices":[{"Variable":"$.halt","IsPresent":true,"Next":"Done"}],"Default":"Loop"},
        "Loop":{"Type":"Pass","Next":"Choose"},
        "Done":{"Type":"Succeed"}
    }});
    let machine = bounded(create(client, "Loop", &definition)).await?;
    let input = input();
    let (history_event_high_water, exhaustion) = exhaust(client, server, &machine, &input).await?;
    let mut readiness = Vec::with_capacity(SAMPLES);
    let mut stop = Vec::with_capacity(SAMPLES);
    for index in 0..SAMPLES {
        let arn = start(client, &machine, format!("Stop{index}"), &input).await?;
        live_history(client, &arn).await?;
        let began = Instant::now();
        server.readiness().await?;
        readiness.push(micros(began.elapsed())?);
        let began = Instant::now();
        let state = bounded(Box::pin(async {
            client
                .stop_execution()
                .execution_arn(&arn)
                .error("Performance.Stop")
                .cause("fixed synthetic workload")
                .send()
                .await?;
            latency::terminal(client, &arn).await
        }))
        .await?;
        stop.push(micros(began.elapsed())?);
        ensure!(
            state.status() == &ExecutionStatus::Aborted
                && state.error() == Some("Performance.Stop")
                && state.input() == Some(input.as_str()),
            "Stop did not abort the live loop"
        );
    }
    server.readiness().await?;
    Ok(Measurement::LoopResponsiveness {
        open_execution_slots: 1,
        completed_cycles: SAMPLES,
        input_bytes: input.len(),
        history_event_high_water,
        exhaustion,
        readiness: Samples::new(readiness)?,
        stop: Samples::new(stop)?,
    })
}
