//! Reject complete malformed archives without publishing or retaining allocations.

use std::{str, sync::Arc};

use chrono::{DateTime, TimeDelta, Utc};
use rstest::rstest;

use super::*;
use crate::{
    RegistryError,
    registry::{
        snapshot::{Outcome, decode},
        tests::definition,
    },
};

fn replace(value: &mut Value, path: &str, new: Value) -> TestResult {
    *value.pointer_mut(path).ok_or("missing test field")? = new;
    Ok(())
}
fn event(value: &mut Value, index: usize, change: impl FnOnce(&mut Value)) -> TestResult {
    let path = format!("/executions/0/history/{index}/eventJson");
    let slot = value.pointer_mut(&path).ok_or("missing event")?;
    let mut data: Value = serde_json::from_str(slot.as_str().ok_or("missing JSON")?)?;
    change(&mut data);
    *slot = Value::String(serde_json::to_string(&data)?);
    Ok(())
}

#[rstest]
#[case("schema")]
#[case("profile")]
#[case("scope")]
#[case("future")]
#[case("callback")]
#[case("uuid")]
#[case("hash")]
#[case("reference")]
#[case("role")]
#[case("duplicate-resource")]
#[case("missing-name")]
#[case("orphan-name")]
#[case("name-too-long")]
#[case("name-at-capture")]
#[case("record-after-name")]
#[case("terminal-running")]
#[case("running-with-terminal")]
#[case("history-id")]
#[case("history-link")]
#[case("history-input")]
#[case("history-terminal")]
#[case("history-timestamp")]
#[case("unsupported-assignment")]
#[case("unsupported-credentials")]
#[case("unsupported-alias")]
#[case("unsupported-version")]
#[case("unsupported-truncation")]
#[case("wrong-detail-field")]
#[tokio::test]
async fn test_should_reject_archive_corruption_atomically(#[case] corruption: &str) -> TestResult {
    let mut data = serde_json::to_value(document().await?)?;
    let captured = data.get("capturedAt").cloned().ok_or("missing capture")?;
    match corruption {
        "schema" => replace(&mut data, "/schemaVersion", json!(2))?,
        "profile" => replace(&mut data, "/profileVersion", json!(2))?,
        "scope" => replace(&mut data, "/scope/account", json!("111111111111"))?,
        "future" => replace(
            &mut data,
            "/capturedAt",
            json!(Utc::now() + TimeDelta::days(1)),
        )?,
        "callback" => replace(&mut data, "/callbackSupport", json!(true))?,
        "uuid" => replace(
            &mut data,
            "/resources/0/resourceId",
            json!("00000000-0000-0000-0000-000000000000"),
        )?,
        "hash" => replace(&mut data, "/revisions/0/sourceHash", json!(vec![0; 32]))?,
        "reference" => replace(
            &mut data,
            "/executions/0/revisionId",
            json!("877c106f-0c19-41c1-8bee-742f6032c101"),
        )?,
        "role" => replace(
            &mut data,
            "/revisions/0/roleArn",
            json!("arn:aws:iam::111111111111:role/Worker"),
        )?,
        "duplicate-resource" => {
            let row = data
                .pointer("/resources/0")
                .cloned()
                .ok_or("missing resource")?;
            data.get_mut("resources")
                .and_then(Value::as_array_mut)
                .ok_or("missing resources")?
                .push(row);
        }
        "missing-name" => replace(&mut data, "/names", json!([]))?,
        "orphan-name" => replace(
            &mut data,
            "/names",
            json!([{"arn":"arn:aws:states:us-east-1:000000000000:execution:Machine:orphan"}]),
        )?,
        "name-too-long" => replace(
            &mut data,
            "/names/0/expiresAt",
            json!(Utc::now() + TimeDelta::days(91)),
        )?,
        "name-at-capture" => replace(&mut data, "/names/0/expiresAt", captured)?,
        "record-after-name" => replace(
            &mut data,
            "/names/0/expiresAt",
            json!(Utc::now() + TimeDelta::hours(1)),
        )?,
        "terminal-running" => replace(
            &mut data,
            "/executions/0/outcome",
            json!({"status":"RUNNING"}),
        )?,
        "running-with-terminal" => {
            replace(
                &mut data,
                "/executions/0/outcome",
                json!({"status":"RUNNING"}),
            )?;
            let row = data
                .pointer_mut("/executions/0")
                .and_then(Value::as_object_mut)
                .ok_or("missing execution")?;
            row.remove("stoppedAt");
            row.remove("expiresAt");
            data.pointer_mut("/names/0")
                .and_then(Value::as_object_mut)
                .ok_or("missing name")?
                .remove("expiresAt");
        }
        "history-id" => event(&mut data, 1, |row| row["id"] = json!(9))?,
        "history-link" => event(&mut data, 1, |row| row["previousEventId"] = json!(2))?,
        "history-input" => event(&mut data, 0, |row| {
            row["executionStartedEventDetails"]["input"] = json!("{}");
        })?,
        "history-terminal" => replace(&mut data, "/executions/0/outcome/output", json!("8"))?,
        "history-timestamp" => event(&mut data, 0, |row| row["timestamp"] = json!(0))?,
        "unsupported-assignment" => event(&mut data, 2, |row| {
            row["stateExitedEventDetails"]["assignedVariables"] =
                json!({"secret":"x".repeat(2048)});
        })?,
        "unsupported-credentials" => event(&mut data, 0, |row| {
            row["executionStartedEventDetails"]["taskCredentials"] =
                json!({"roleArn":"x".repeat(2049)});
        })?,
        "unsupported-alias" => event(&mut data, 0, |row| {
            row["executionStartedEventDetails"]["stateMachineAliasArn"] = json!("x".repeat(2049));
        })?,
        "unsupported-version" => event(&mut data, 0, |row| {
            row["executionStartedEventDetails"]["stateMachineVersionArn"] = json!("x".repeat(2049));
        })?,
        "unsupported-truncation" => event(&mut data, 0, |row| {
            row["executionStartedEventDetails"]["inputDetails"]["extra"] = json!(true);
        })?,
        "wrong-detail-field" => event(&mut data, 1, |row| {
            row["stateEnteredEventDetails"]["resource"] = json!("ignored");
        })?,
        _ => return Err("unknown case".into()),
    }
    let (target, mut owner) = registry(RegistryLimits::default(), None);
    // Some violations are rejected by bounded DTO decode, the rest by preparation.
    let rejected = match serde_json::from_value::<RegistrySnapshot>(data) {
        Ok(snapshot) => target
            .prepare_snapshot(snapshot, SnapshotLimits::default())
            .is_err(),
        Err(_) => true,
    };
    assert!(rejected, "accepted corruption: {corruption}");
    assert!(target.status().ready);
    assert_eq!(target.shared.restore.history.used(), 0);
    restored(&target, document().await?).await?;
    owner.shutdown(deadline()).await?;
    Ok(())
}

#[rstest]
#[case("/scope/partition", 17)]
#[case("/resources/0/arn", 257)]
#[case("/revisions/0/definition", 1_048_577)]
#[case("/revisions/0/roleArn", 2049)]
#[case("/executions/0/input", 262_145)]
#[case("/executions/0/traceHeader", 257)]
#[case("/executions/0/outcome/output", 262_145)]
#[case("/executions/0/history/0/eventJson", 2_097_153)]
#[tokio::test]
async fn test_should_bound_snapshot_text_during_decode(
    #[case] path: &str,
    #[case] length: usize,
) -> TestResult {
    let mut data = serde_json::to_value(document().await?)?;
    if path.ends_with("traceHeader") {
        data.pointer_mut("/executions/0")
            .and_then(Value::as_object_mut)
            .ok_or("missing execution")?
            .insert("traceHeader".into(), json!(""));
    }
    replace(&mut data, path, json!("x".repeat(length)))?;
    assert!(serde_json::from_value::<RegistrySnapshot>(data).is_err());
    Ok(())
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct InstantRow {
    #[serde(deserialize_with = "decode::time")]
    required: DateTime<Utc>,
    #[serde(default, deserialize_with = "decode::optional_time")]
    optional: Option<DateTime<Utc>>,
}
#[rstest]
#[case("2026-10-25T01:02:03Z")]
#[case("2026-10-25T01:02:03.123456789Z")]
#[case("1960-01-01T00:00:00.000000001Z")]
#[case("+12345-01-01T00:00:00Z")]
#[case("-0001-01-01T00:00:00Z")]
fn test_should_round_trip_exact_canonical_instant_cbor(#[case] text: &str) -> TestResult {
    let row: InstantRow = serde_json::from_value(json!({"required":text,"optional":text}))?;
    let mut bytes = Vec::new();
    ciborium::into_writer(&row, &mut bytes)?;
    let decoded: InstantRow = ciborium::from_reader(bytes.as_slice())?;
    assert_eq!(row.required, decoded.required);
    assert_eq!(row.optional, decoded.optional);
    assert_eq!(
        serde_json::to_value(decoded)?.get("required"),
        Some(&json!(text))
    );
    Ok(())
}
#[rstest]
#[case("2026-10-25T01:02:03.1234567891Z")]
#[case("2026-10-25T01:02:03.123456789000000000000000000000000000000000000000000000000Z")]
#[case("2026-10-25T01:02:03.1230Z")]
#[case("2026-10-25T01:02:03+00:00")]
#[case("2016-12-31T23:59:60Z")]
fn test_should_reject_lossy_or_noncanonical_required_and_optional_instants(#[case] text: &str) {
    for data in [
        json!({"required":text}),
        json!({"required":"2026-10-25T01:02:03Z","optional":text}),
    ] {
        assert!(serde_json::from_value::<InstantRow>(data).is_err());
    }
}

#[tokio::test]
async fn test_should_reject_copy_budget_without_closing_startup() -> TestResult {
    let (target, mut owner) = registry(RegistryLimits::default(), None);
    assert!(matches!(
        target.prepare_snapshot(document().await?, SnapshotLimits::try_from(1)?),
        Err(RegistryError::Capacity)
    ));
    restored(&target, document().await?).await?;
    owner.shutdown(deadline()).await?;
    Ok(())
}

#[rstest]
#[case(json!({"status":"RUNNING","output":"null"}))]
#[case(json!({"status":"TIMED_OUT","error":"foreign"}))]
#[case(json!({"status":"SUCCEEDED","output":"null","error":"foreign"}))]
#[case(json!({"status":"FAILED","output":"null"}))]
fn test_should_reject_contradictory_outcome_fields(#[case] value: Value) {
    assert!(serde_json::from_value::<Outcome>(value).is_err());
}

#[rstest]
#[case(false, "credentials")]
#[case(true, "credentials")]
#[case(false, "resource")]
#[case(false, "foreign")]
#[case(false, "timeout")]
#[case(true, "timeout")]
#[case(true, "heartbeat")]
#[case(true, "noncallback-heartbeat")]
#[tokio::test]
async fn test_should_reject_broad_task_fields_and_invalid_scoped_lambda_resources(
    #[case] optimized: bool,
    #[case] corruption: &str,
) -> TestResult {
    let mut data = serde_json::to_value(retention::lambda_document(optimized).await?)?;
    let detail = if optimized {
        "taskScheduledEventDetails"
    } else {
        "lambdaFunctionScheduledEventDetails"
    };
    event(&mut data, 2, |row| {
        let fields = &mut row[detail];
        match corruption {
            "credentials" => fields["taskCredentials"] = json!({"roleArn":"x".repeat(2049)}),
            "resource" => fields["resource"] = json!("not-an-arn"),
            "foreign" => {
                fields["resource"] = json!("arn:aws:lambda:us-west-2:000000000000:function:Worker");
            }
            "timeout" => fields["timeoutInSeconds"] = json!(0),
            "heartbeat" => fields["heartbeatInSeconds"] = json!(0),
            "noncallback-heartbeat" => fields["heartbeatInSeconds"] = json!(2),
            _ => {}
        }
    })?;
    let (target, mut owner) = registry(
        RegistryLimits::default(),
        Some(Arc::new(retention::CountingPort::default())),
    );
    assert!(
        target
            .prepare_snapshot(serde_json::from_value(data)?, SnapshotLimits::default())
            .is_err()
    );
    assert_eq!(target.shared.restore.history.used(), 0);
    owner.shutdown(deadline()).await?;
    Ok(())
}

#[tokio::test]
async fn test_should_validate_source_profile_even_when_target_is_stronger() -> TestResult {
    let (target, mut owner) = registry(
        RegistryLimits::default(),
        Some(Arc::new(retention::CountingPort::default())),
    );
    let mut data = serde_json::to_value(retention::lambda_document(false).await?)?;
    replace(&mut data, "/lambdaSupport", json!(false))?;
    assert!(
        target
            .prepare_snapshot(serde_json::from_value(data)?, SnapshotLimits::default())
            .is_err()
    );
    restored(&target, document().await?).await?;
    owner.shutdown(deadline()).await?;
    Ok(())
}

#[tokio::test]
async fn test_should_bound_snapshot_collections_and_reject_unknown_fields_during_decode()
-> TestResult {
    let data = serde_json::to_value(document().await?)?;
    let mut excess = data.clone();
    let row = data
        .pointer("/resources/0")
        .cloned()
        .ok_or("missing resource")?;
    replace(&mut excess, "/resources", json!(vec![row; 4097]))?;
    assert!(serde_json::from_value::<RegistrySnapshot>(excess).is_err());
    let mut unknown = data.clone();
    unknown
        .as_object_mut()
        .ok_or("invalid root")?
        .insert("secretField".into(), json!("private"));
    assert!(serde_json::from_value::<RegistrySnapshot>(unknown).is_err());
    // Embedding adapters redact serde diagnostics; domain debug never prints archive data.
    assert!(!format!("{:?}", document().await?).contains("stored"));
    Ok(())
}

#[tokio::test]
async fn test_should_validate_history_against_source_profile_independently_of_definition()
-> TestResult {
    let mut snapshot = retention::lambda_document(false).await?;
    snapshot.lambda_support = false;
    let pure = definition(42, false);
    let revision = snapshot.revisions.first_mut().ok_or("missing revision")?;
    revision.definition = str::from_utf8(pure.machine.original_bytes())?.to_owned();
    revision.source_hash = pure.hash;
    let (target, mut owner) = registry(
        RegistryLimits::default(),
        Some(Arc::new(retention::CountingPort::default())),
    );
    assert!(
        target
            .prepare_snapshot(snapshot, SnapshotLimits::default())
            .is_err()
    );
    assert!(target.status().ready);
    owner.shutdown(deadline()).await?;
    Ok(())
}

#[tokio::test]
async fn test_should_bound_name_only_expiry_and_reject_duplicate_tags() -> TestResult {
    let mut snapshot = document().await?;
    snapshot.resources.clear();
    snapshot.revisions.clear();
    snapshot.machines.clear();
    snapshot.executions.clear();
    snapshot.names.first_mut().ok_or("missing name")?.expires_at =
        Some(snapshot.captured_at + TimeDelta::days(91));
    let (target, mut owner) = registry(RegistryLimits::default(), None);
    assert!(
        target
            .prepare_snapshot(snapshot, SnapshotLimits::default())
            .is_err()
    );
    let encoded = serde_json::to_string(&document().await?)?;
    let duplicate = encoded.replace("\"tags\":{}", "\"tags\":{\"Team\":\"A\",\"Team\":\"B\"}");
    assert_ne!(encoded, duplicate);
    assert!(serde_json::from_str::<RegistrySnapshot>(&duplicate).is_err());
    restored(&target, document().await?).await?;
    owner.shutdown(deadline()).await?;
    Ok(())
}
