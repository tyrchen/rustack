//! Independent rank and microsecond-boundary controls for measurement reports.

use std::time::Duration;

use anyhow::{Context, Result};
use aws_sdk_sfn::{
    primitives::DateTime,
    types::{
        ExecutionStatus, HistoryEvent, HistoryEventType,
        HistoryEventType::{
            ChoiceStateEntered, ChoiceStateExited, PassStateEntered, PassStateExited,
            SucceedStateEntered, SucceedStateExited,
        },
        StateEnteredEventDetails, StateExitedEventDetails,
        builders::HistoryEventBuilder,
    },
};

use super::{SAMPLES, Samples, latency::validate_history, load::ExhaustionProbes, micros};

#[test]
fn test_should_use_exact_nearest_ranks_and_preserve_sample_order() -> Result<()> {
    let original: Vec<u64> = (1..=100).rev().collect();
    let samples = Samples::new(original.clone())?;
    assert_eq!(samples.observed_micros, original);
    assert_eq!(
        (samples.p50_micros, samples.p95_micros, samples.max_micros),
        (50, 95, 100)
    );
    assert!(Samples::new(vec![1; 99]).is_err());
    assert!(Samples::new(vec![1; 101]).is_err());
    Ok(())
}
#[test]
fn test_should_round_up_and_enforce_exact_p95_budget() -> Result<()> {
    assert_eq!(micros(Duration::from_nanos(1))?, 1);
    assert_eq!(micros(Duration::from_nanos(999))?, 1);
    assert_eq!(micros(Duration::from_nanos(1000))?, 1);
    assert_eq!(micros(Duration::from_nanos(1001))?, 2);
    assert!(micros(Duration::MAX).is_err());
    let mut values = vec![100_000; SAMPLES];
    Samples::new(values.clone())?.check_p95(100_000)?;
    for value in values.iter_mut().take(5) {
        *value = 100_001;
    }
    Samples::new(values.clone())?.check_p95(100_000)?;
    *values
        .get_mut(5)
        .ok_or_else(|| anyhow::anyhow!("missing sixth sample"))? = 100_001;
    assert!(Samples::new(values)?.check_p95(100_000).is_err());
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn test_should_reject_ready_results_at_the_expired_observation_boundary() -> Result<()> {
    let error = super::bounded(async {
        tokio::time::advance(super::OPERATION_BUDGET).await;
        Ok::<_, anyhow::Error>(())
    })
    .await
    .err()
    .ok_or_else(|| anyhow::anyhow!("accepted late ready result"))?;
    assert!(error.to_string().contains("late workload result"));
    super::bounded(async { Ok::<_, anyhow::Error>(()) }).await?;
    Ok(())
}

#[test]
fn test_should_reject_exhaustion_readiness_at_the_two_second_boundary() -> Result<()> {
    for worst in [1_999_999, 2_000_000, 2_000_001] {
        let mut measured = ExhaustionProbes::default();
        measured.record(&ExecutionStatus::Running, &ExecutionStatus::Running, worst)?;
        assert_eq!(measured.check().is_ok(), worst < 2_000_000);
    }
    Ok(())
}

fn event(kind: HistoryEventType, history: &[HistoryEvent]) -> Result<HistoryEventBuilder> {
    let previous = i64::try_from(history.len())?;
    Ok(HistoryEvent::builder()
        .timestamp(DateTime::from_secs(0))
        .r#type(kind)
        .id(previous.checked_add(1).context("event ID overflow")?)
        .previous_event_id(if history.len() == 1 { 0 } else { previous }))
}
fn complete_history() -> Result<Vec<HistoryEvent>> {
    let mut events = Vec::new();
    events.push(event(HistoryEventType::ExecutionStarted, &events)?.build()?);
    let alternating = [
        (PassStateEntered, PassStateExited),
        (ChoiceStateEntered, ChoiceStateExited),
    ];
    let states = alternating.into_iter().cycle().take(48).chain([
        (PassStateEntered, PassStateExited),
        (SucceedStateEntered, SucceedStateExited),
    ]);
    for (index, (entered, exited)) in states.enumerate() {
        let name = format!("S{index:02}");
        events.push(
            event(entered, &events)?
                .state_entered_event_details(
                    StateEnteredEventDetails::builder().name(&name).build()?,
                )
                .build()?,
        );
        events.push(
            event(exited, &events)?
                .state_exited_event_details(StateExitedEventDetails::builder().name(name).build()?)
                .build()?,
        );
    }
    events.push(event(HistoryEventType::ExecutionSucceeded, &events)?.build()?);
    Ok(events)
}
#[test]
fn test_should_reject_broken_predecessors_and_wrong_or_missing_state_exits() -> Result<()> {
    let complete = complete_history()?;
    validate_history(&complete)?;
    for mutation in [
        "predecessor",
        "root-predecessor",
        "missing-exit",
        "exit-kind",
        "exit-name",
        "first",
        "last",
    ] {
        let mut broken = complete.clone();
        match mutation {
            "root-predecessor" => {
                broken
                    .get_mut(1)
                    .context("missing first state entry")?
                    .previous_event_id = 1;
            }
            "predecessor" => {
                broken
                    .get_mut(50)
                    .context("missing causal event")?
                    .previous_event_id = 0;
            }
            "missing-exit" => broken.retain(|event| event.id() != 3),
            "exit-kind" => {
                broken.get_mut(2).context("missing exit")?.r#type =
                    HistoryEventType::ChoiceStateExited;
            }
            "exit-name" => {
                broken
                    .get_mut(2)
                    .context("missing exit")?
                    .state_exited_event_details
                    .as_mut()
                    .context("missing exit details")?
                    .name = "Wrong".to_owned();
            }
            "first" => {
                broken.first_mut().context("missing first")?.r#type =
                    HistoryEventType::ExecutionFailed;
            }
            "last" => {
                broken.last_mut().context("missing last")?.r#type =
                    HistoryEventType::ExecutionFailed;
            }
            _ => anyhow::bail!("unknown mutation"),
        }
        assert!(validate_history(&broken).is_err(), "accepted {mutation}");
    }
    Ok(())
}

#[test]
fn test_should_refuse_post_terminal_probes_as_live_load_evidence() -> Result<()> {
    let mut probes = ExhaustionProbes::default();
    probes.record(&ExecutionStatus::Failed, &ExecutionStatus::Failed, 1)?;
    assert!(probes.check().is_err());
    probes.record(&ExecutionStatus::Running, &ExecutionStatus::Failed, 1)?;
    assert!(probes.check().is_err());
    probes.record(&ExecutionStatus::Running, &ExecutionStatus::Running, 1)?;
    probes.check()
}
