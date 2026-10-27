//! Actual signed SDK workloads; measurements publish only after process cleanup.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use std::{env::consts, future::Future, thread, time::Duration};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use tokio::time::{Instant, timeout};

use super::{process::Server, rust::sfn};

#[path = "performance/latency.rs"]
mod latency;
#[path = "performance/load.rs"]
mod load;
#[cfg(test)]
#[path = "performance/tests.rs"]
mod tests;

const SAMPLES: usize = 100;
const OPERATION_BUDGET: Duration = Duration::from_secs(2);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Samples {
    count: usize,
    observed_micros: Vec<u64>,
    p50_micros: u64,
    p95_micros: u64,
    max_micros: u64,
}
impl Samples {
    fn new(observed_micros: Vec<u64>) -> Result<Self> {
        ensure!(observed_micros.len() == SAMPLES, "wrong measurement count");
        let mut ranked = observed_micros.clone();
        ranked.sort_unstable();
        Ok(Self {
            count: observed_micros.len(),
            observed_micros,
            p50_micros: *ranked.get(49).context("missing median rank")?,
            p95_micros: *ranked.get(94).context("missing p95 rank")?,
            max_micros: *ranked.last().context("missing maximum")?,
        })
    }
    fn check_p95(&self, budget: u64) -> Result<()> {
        ensure!(self.p95_micros <= budget, "workflow p95 budget exceeded");
        Ok(())
    }
}

#[derive(Debug, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum Measurement {
    PureLatency {
        state_count: usize,
        input_bytes: usize,
        executions: usize,
        budget_micros: u64,
        latency: Samples,
    },
    LoopResponsiveness {
        open_execution_slots: usize,
        completed_cycles: usize,
        input_bytes: usize,
        history_event_high_water: i64,
        exhaustion: load::ExhaustionProbes,
        readiness: Samples,
        stop: Samples,
    },
}
impl Measurement {
    fn check(&self) -> Result<()> {
        match self {
            Self::PureLatency {
                latency,
                budget_micros,
                ..
            } => latency.check_p95(*budget_micros),
            Self::LoopResponsiveness {
                readiness,
                stop,
                exhaustion,
                ..
            } => {
                exhaustion.check()?;
                ensure!(
                    readiness.max_micros < 2_000_000,
                    "readiness budget exceeded"
                );
                ensure!(stop.max_micros < 2_000_000, "Stop budget exceeded");
                Ok(())
            }
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Host {
    os: &'static str,
    architecture: &'static str,
    available_logical_cpus: usize,
    debug_assertions: bool,
    reference_qualification: &'static str,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Limits {
    open_executions: usize,
    transitions: u32,
    history_events: usize,
    history_bytes: usize,
    total_history_bytes: usize,
}
impl Limits {
    async fn read(server: &Server) -> Result<Self> {
        let capabilities = server.capabilities().await?;
        let limits: Self = serde_json::from_value(
            capabilities
                .pointer("/workflows/limits")
                .context("missing workflow limits")?
                .clone(),
        )?;
        ensure!(
            limits.open_executions == 1
                && limits.transitions == 1_000_000
                && limits.history_events == 25_000
                && limits.history_bytes == 33_554_432
                && limits.total_history_bytes == 134_217_728,
            "actual performance limits differ from contract"
        );
        Ok(limits)
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    schema_version: u8,
    host: Host,
    limits: Limits,
    native_processes_observed: usize,
    server_joined: bool,
    measurement: Measurement,
}
impl Report {
    fn emit(
        measurement: Measurement,
        limits: Limits,
        native_processes_observed: usize,
    ) -> Result<()> {
        ensure!(
            native_processes_observed == 0,
            "pure workload invoked Lambda"
        );
        let report = Self {
            schema_version: 1,
            host: Host {
                os: consts::OS,
                architecture: consts::ARCH,
                available_logical_cpus: thread::available_parallelism()?.get(),
                debug_assertions: cfg!(debug_assertions),
                reference_qualification: "pending",
            },
            limits,
            native_processes_observed,
            server_joined: true,
            measurement,
        };
        let encoded = serde_json::to_string(&report)?;
        ensure!(encoded.len() <= 65_536, "oversized measurement report");
        println!("WORKFLOWS_PERFORMANCE {encoded}");
        report.measurement.check()
    }
}

fn micros(duration: Duration) -> Result<u64> {
    let rounded = duration
        .as_nanos()
        .checked_add(999)
        .context("duration overflow")?
        / 1000;
    Ok(u64::try_from(rounded)?)
}
async fn bounded<T, E: Into<anyhow::Error>>(
    future: impl Future<Output = std::result::Result<T, E>>,
) -> Result<T> {
    let began = Instant::now();
    let result = timeout(OPERATION_BUDGET, future)
        .await
        .context("workload operation expired")?;
    ensure!(began.elapsed() < OPERATION_BUDGET, "late workload result");
    result.map_err(Into::into)
}
fn input() -> String {
    format!(r#"{{"payload":"{}"}}"#, "x".repeat(1010))
}

async fn run(latency_workload: bool) -> Result<()> {
    let mut server = Server::start_performance().await?;
    let client = sfn(&server, super::process::SECRET);
    let began = Instant::now();
    let result = timeout(Duration::from_secs(120), async {
        let limits = Limits::read(&server).await?;
        let measurement = if latency_workload {
            latency::measure(&client).await?
        } else {
            load::measure(&client, &server).await?
        };
        Ok::<_, anyhow::Error>((limits, measurement))
    })
    .await
    .context("complete workload expired");
    let result = if began.elapsed() >= Duration::from_secs(120) {
        Err(anyhow::anyhow!("late complete workload result"))
    } else {
        result
    };
    let cleanup = server
        .shutdown(Instant::now() + Duration::from_secs(45))
        .await;
    let observed = cleanup?;
    let (limits, measurement) = result??;
    Report::emit(measurement, limits, observed)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real measured SDK workload; run make test-workflows-performance"]
async fn test_should_measure_one_hundred_fifty_state_signed_sdk_executions() -> Result<()> {
    run(true).await
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real loop/Stop workload; run make test-workflows-performance"]
async fn test_should_keep_health_and_stop_responsive_and_reuse_one_execution_slot() -> Result<()> {
    run(false).await
}
