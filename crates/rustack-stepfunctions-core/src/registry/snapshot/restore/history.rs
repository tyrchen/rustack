//! Validate lossless official history data without granting runtime capability.

use std::{collections::BTreeSet, fmt};

use chrono::{DateTime, Utc};
use rustack_core::json::{JsonLimits, validate};
use rustack_stepfunctions_model::types::{HistoryEvent, HistoryEventType};
use serde::{
    Deserialize, Deserializer,
    de::{Error, IgnoredAny, MapAccess, Visitor},
};
use serde_json::value::RawValue;

use super::{
    super::{HistoryRow, decode::Text},
    ProfileStates, RegistryError,
};
use crate::{
    ExecutionOutcome, FunctionArn, Payload, TenantScope, compiler::TASK_SECONDS_LIMIT,
    engine::HistoryRestorer,
};

#[derive(Debug)]
struct Details {
    fields: BTreeSet<String>,
    timeout: Option<i64>,
    heartbeat: Option<i64>,
    input_truncated: Option<bool>,
    output_truncated: Option<bool>,
    resource_type: Option<String>,
    resource: Option<String>,
    region: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DataDetails {
    truncated: bool,
}
#[derive(Debug)]
struct DetailVisitor;
impl<'de> Visitor<'de> for DetailVisitor {
    type Value = Details;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("bounded history details")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut result = Details {
            fields: BTreeSet::new(),
            timeout: None,
            heartbeat: None,
            input_truncated: None,
            output_truncated: None,
            resource_type: None,
            resource: None,
            region: None,
        };
        while let Some(key) = map.next_key::<String>()? {
            if !result.fields.insert(key.clone()) {
                return Err(A::Error::custom("duplicate history detail"));
            }
            match key.as_str() {
                "input" | "output" | "parameters" => {
                    if let Some(text) = map.next_value::<Option<Text<262_144>>>()? {
                        Payload::parse(text.0.as_bytes())
                            .map_err(|_| A::Error::custom("invalid history payload"))?;
                    }
                }
                "error" => {
                    map.next_value::<Option<Text<256>>>()?;
                }
                "cause" => {
                    map.next_value::<Option<Text<32768>>>()?;
                }
                "name" => {
                    map.next_value::<Option<Text<128>>>()?;
                }
                "roleArn" => {
                    map.next_value::<Option<Text<2048>>>()?;
                }
                "resource" => {
                    result.resource = map.next_value::<Option<Text<2048>>>()?.map(|value| value.0);
                }
                "resourceType" => {
                    result.resource_type =
                        map.next_value::<Option<Text<64>>>()?.map(|value| value.0);
                }
                "region" => {
                    result.region = map.next_value::<Option<Text<64>>>()?.map(|value| value.0);
                }
                "index" | "length" => {
                    if map
                        .next_value::<Option<i64>>()?
                        .is_some_and(|value| !(0..=1_000_000).contains(&value))
                    {
                        return Err(A::Error::custom("invalid history count"));
                    }
                }
                "timeoutInSeconds" | "heartbeatInSeconds" => {
                    let value = map.next_value::<Option<i64>>()?;
                    if value
                        .is_some_and(|value| !(1..=i64::from(TASK_SECONDS_LIMIT)).contains(&value))
                    {
                        return Err(A::Error::custom("invalid history timeout"));
                    }
                    if key == "timeoutInSeconds" {
                        result.timeout = value;
                    } else {
                        result.heartbeat = value;
                    }
                }
                "inputDetails" | "outputDetails" => {
                    let details = map.next_value::<DataDetails>()?;
                    if key == "inputDetails" {
                        result.input_truncated = Some(details.truncated);
                    } else {
                        result.output_truncated = Some(details.truncated);
                    }
                }
                _ => return Err(A::Error::custom("unsupported history detail field")),
            }
        }
        Ok(result)
    }
}
impl<'de> Deserialize<'de> for Details {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        decoder.deserialize_map(DetailVisitor)
    }
}
#[derive(Debug)]
struct Shape {
    timestamp: Box<RawValue>,
    detail: Option<(String, Details)>,
}
#[derive(Debug)]
struct ShapeVisitor;
impl<'de> Visitor<'de> for ShapeVisitor {
    type Value = Shape;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("one official history event")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut timestamp = None;
        let mut detail = None;
        while let Some(key) = map.next_key::<String>()? {
            if key == "timestamp" {
                if timestamp.is_some() {
                    return Err(A::Error::custom("duplicate history timestamp"));
                }
                timestamp = Some(map.next_value()?);
            } else if key.ends_with("EventDetails") {
                if detail.is_some() {
                    return Err(A::Error::custom("contradictory history details"));
                }
                detail = Some((key, map.next_value()?));
            } else if matches!(key.as_str(), "id" | "previousEventId" | "type") {
                map.next_value::<IgnoredAny>()?;
            } else {
                return Err(A::Error::custom("unsupported history event field"));
            }
        }
        Ok(Shape {
            timestamp: timestamp.ok_or_else(|| A::Error::custom("missing history timestamp"))?,
            detail,
        })
    }
}
impl<'de> Deserialize<'de> for Shape {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        decoder.deserialize_map(ShapeVisitor)
    }
}
fn projected_time(timestamp: DateTime<Utc>) -> Result<Box<RawValue>, RegistryError> {
    let event = HistoryEvent::builder()
        .id(0)
        .timestamp(timestamp)
        .r#type(HistoryEventType::ExecutionStarted)
        .build();
    let encoded = serde_json::to_vec(&event).map_err(|_| RegistryError::Validation)?;
    let shape: Shape = serde_json::from_slice(&encoded).map_err(|_| RegistryError::Validation)?;
    Ok(shape.timestamp)
}
fn detail(kind: &str) -> Result<Option<&'static str>, RegistryError> {
    let name = match kind {
        "ExecutionStarted" => "executionStartedEventDetails",
        "ExecutionSucceeded" => "executionSucceededEventDetails",
        "ExecutionFailed" => "executionFailedEventDetails",
        "ExecutionTimedOut" => "executionTimedOutEventDetails",
        "ExecutionAborted" => "executionAbortedEventDetails",
        "PassStateEntered"
        | "TaskStateEntered"
        | "WaitStateEntered"
        | "ChoiceStateEntered"
        | "SucceedStateEntered"
        | "FailStateEntered"
        | "ParallelStateEntered"
        | "MapStateEntered" => "stateEnteredEventDetails",
        "PassStateExited"
        | "TaskStateExited"
        | "WaitStateExited"
        | "ChoiceStateExited"
        | "SucceedStateExited"
        | "ParallelStateExited"
        | "MapStateExited" => "stateExitedEventDetails",
        "LambdaFunctionScheduled" => "lambdaFunctionScheduledEventDetails",
        "LambdaFunctionSucceeded" => "lambdaFunctionSucceededEventDetails",
        "LambdaFunctionFailed" => "lambdaFunctionFailedEventDetails",
        "LambdaFunctionTimedOut" => "lambdaFunctionTimedOutEventDetails",
        "LambdaFunctionScheduleFailed" => "lambdaFunctionScheduleFailedEventDetails",
        "TaskScheduled" => "taskScheduledEventDetails",
        "TaskStarted" => "taskStartedEventDetails",
        "TaskSubmitted" => "taskSubmittedEventDetails",
        "TaskSucceeded" => "taskSucceededEventDetails",
        "TaskFailed" => "taskFailedEventDetails",
        "TaskTimedOut" => "taskTimedOutEventDetails",
        "TaskStartFailed" => "taskStartFailedEventDetails",
        "MapStateStarted" => "mapStateStartedEventDetails",
        "MapIterationStarted" => "mapIterationStartedEventDetails",
        "MapIterationSucceeded" => "mapIterationSucceededEventDetails",
        "MapIterationFailed" => "mapIterationFailedEventDetails",
        "MapIterationAborted" => "mapIterationAbortedEventDetails",
        "LambdaFunctionStarted"
        | "ParallelStateStarted"
        | "ParallelStateSucceeded"
        | "ParallelStateFailed"
        | "ParallelStateAborted"
        | "MapStateSucceeded"
        | "MapStateFailed"
        | "MapStateAborted"
        | "TaskStateAborted"
        | "WaitStateAborted" => return Ok(None),
        _ => return Err(RegistryError::Validation),
    };
    Ok(Some(name))
}
fn terminal(kind: &str) -> bool {
    matches!(
        kind,
        "ExecutionSucceeded" | "ExecutionFailed" | "ExecutionTimedOut" | "ExecutionAborted"
    )
}
#[derive(Debug)]
pub(super) struct Profile<'a> {
    pub(super) states: ProfileStates,
    pub(super) lambda: bool,
    pub(super) callbacks: bool,
    pub(super) scope: &'a TenantScope,
}
fn fields(detail: &str) -> &'static [&'static str] {
    match detail {
        "executionStartedEventDetails" => &["input", "inputDetails", "roleArn"],
        "executionSucceededEventDetails" | "lambdaFunctionSucceededEventDetails" => {
            &["output", "outputDetails"]
        }
        "stateEnteredEventDetails" => &["name", "input", "inputDetails"],
        "stateExitedEventDetails" => &["name", "output", "outputDetails"],
        "lambdaFunctionScheduledEventDetails" => {
            &["resource", "input", "inputDetails", "timeoutInSeconds"]
        }
        "taskScheduledEventDetails" => &[
            "resourceType",
            "resource",
            "region",
            "parameters",
            "timeoutInSeconds",
            "heartbeatInSeconds",
        ],
        "taskStartedEventDetails" => &["resourceType", "resource"],
        "taskSubmittedEventDetails" | "taskSucceededEventDetails" => {
            &["resourceType", "resource", "output", "outputDetails"]
        }
        "taskFailedEventDetails" | "taskTimedOutEventDetails" | "taskStartFailedEventDetails" => {
            &["resourceType", "resource", "error", "cause"]
        }
        "mapStateStartedEventDetails" => &["length"],
        "mapIterationStartedEventDetails"
        | "mapIterationSucceededEventDetails"
        | "mapIterationFailedEventDetails"
        | "mapIterationAbortedEventDetails" => &["name", "index"],
        _ => &["error", "cause"],
    }
}
fn check_profile(kind: &str, shape: &Shape, profile: &Profile<'_>) -> Result<(), RegistryError> {
    if ((kind.starts_with("Map") || kind.starts_with("Parallel"))
        && matches!(profile.states, ProfileStates::Sequential))
        || (kind.starts_with("LambdaFunction") && !profile.lambda)
    {
        return Err(RegistryError::Validation);
    }
    if detail(kind)? != shape.detail.as_ref().map(|(name, _)| name.as_str()) {
        return Err(RegistryError::Validation);
    }
    if let Some((name, data)) = &shape.detail {
        if data
            .fields
            .iter()
            .any(|field| !fields(name).contains(&field.as_str()))
        {
            return Err(RegistryError::Validation);
        }
        if kind == "LambdaFunctionScheduled" {
            FunctionArn::try_from((
                data.resource.as_deref().ok_or(RegistryError::Validation)?,
                profile.scope,
            ))
            .map_err(|_| RegistryError::Validation)?;
        }
        let callback = data.resource_type.as_deref() == Some("lambda")
            && data.resource.as_deref() == Some("invoke.waitForTaskToken");
        if data.input_truncated == Some(true)
            || data
                .output_truncated
                .is_some_and(|value| value != (callback && kind == "TaskSubmitted"))
            || data.heartbeat.is_some_and(|value| {
                !callback || value >= data.timeout.unwrap_or(i64::from(TASK_SECONDS_LIMIT))
            })
        {
            return Err(RegistryError::Validation);
        }
        if data
            .region
            .as_ref()
            .is_some_and(|value| value != profile.scope.region())
        {
            return Err(RegistryError::Validation);
        }
        if let Some(resource_type) = &data.resource_type {
            let resource = data.resource.as_deref().ok_or(RegistryError::Validation)?;
            let valid = match resource_type.as_str() {
                "lambda" => {
                    profile.lambda
                        && (resource == "invoke"
                            || (resource == "invoke.waitForTaskToken" && profile.callbacks))
                }
                "states" => matches!(
                    resource,
                    "startExecution" | "startExecution.sync" | "startExecution.sync:2"
                ),
                _ => false,
            };
            if !valid {
                return Err(RegistryError::Validation);
            }
        }
    }
    Ok(())
}
pub(super) fn validate_rows(
    rows: Vec<HistoryRow>,
    input: &str,
    outcome: Option<&ExecutionOutcome>,
    stopped: Option<DateTime<Utc>>,
    profile: &Profile<'_>,
) -> Result<Vec<HistoryEvent>, RegistryError> {
    if rows.is_empty() || (outcome.is_some() && rows.len() < 2) {
        return Err(RegistryError::Validation);
    }
    let mut events = Vec::new();
    events
        .try_reserve_exact(rows.len())
        .map_err(|_| RegistryError::Capacity)?;
    let count = rows.len();
    for (index, row) in rows.into_iter().enumerate() {
        if row.timestamp.timestamp_subsec_nanos() >= 1_000_000_000 {
            return Err(RegistryError::Validation);
        }
        validate(
            row.event_json.as_bytes(),
            JsonLimits::new(2 * 1024 * 1024, 32, 65536, 1024 * 1024, 256)
                .map_err(|_| RegistryError::Validation)?,
        )
        .map_err(|_| RegistryError::Validation)?;
        let shape: Shape =
            serde_json::from_str(&row.event_json).map_err(|_| RegistryError::Validation)?;
        if shape.timestamp.get() != projected_time(row.timestamp)?.get() {
            return Err(RegistryError::Validation);
        }
        let mut event: HistoryEvent =
            serde_json::from_str(&row.event_json).map_err(|_| RegistryError::Validation)?;
        event.timestamp = row.timestamp;
        check_profile(event.r#type.as_str(), &shape, profile)?;
        let id = i64::try_from(index.checked_add(1).ok_or(RegistryError::Validation)?)
            .map_err(|_| RegistryError::Validation)?;
        if event.id != id
            || !event
                .previous_event_id
                .is_some_and(|prior| prior >= 0 && prior < id)
        {
            return Err(RegistryError::Validation);
        }
        if index == 0 {
            if event.r#type.as_str() != "ExecutionStarted"
                || event.previous_event_id != Some(0)
                || event
                    .execution_started_event_details
                    .as_ref()
                    .and_then(|details| details.input.as_deref())
                    != Some(input)
            {
                return Err(RegistryError::Validation);
            }
        } else if event.r#type.as_str() == "ExecutionStarted" {
            return Err(RegistryError::Validation);
        }
        if terminal(event.r#type.as_str()) != (index + 1 == count && outcome.is_some()) {
            return Err(RegistryError::Validation);
        }
        events.push(event);
    }
    match (outcome, stopped) {
        (Some(outcome), Some(stopped)) => HistoryRestorer::validate_terminal(
            events.last().ok_or(RegistryError::Validation)?,
            outcome,
            stopped,
        )?,
        (None, None) => {}
        _ => return Err(RegistryError::Validation),
    }
    Ok(events)
}
