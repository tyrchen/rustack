//! Strict scoped state fields compiled once into immutable instructions.

use std::{collections::BTreeMap, sync::Arc};

use serde_json::{Map, Value};

use super::{
    Code, CompileError, Failure, Location, Node, StateId, StateSupport, TASK_SECONDS_LIMIT,
    TaskTimeout, Transition, fields, metadata, object_value, positive, recovery, required,
    scope::Context, structured, target, task, text,
};
use crate::{DataFlow, Expression, Predicate, ReferencePath, WaitSource};

const COMMON: &[&str] = &[
    "Type",
    "Comment",
    "QueryLanguage",
    "InputPath",
    "OutputPath",
];
const FLOW: &[&str] = &[
    "InputPath",
    "OutputPath",
    "Parameters",
    "ResultSelector",
    "ResultPath",
];

pub(super) fn compile(
    value: &Value,
    location: &Location,
    index: &BTreeMap<String, StateId>,
    context: &mut Context<'_>,
    depth: usize,
) -> Result<Node, CompileError> {
    let object = object_value(value, location)?;
    let kind = text(
        required(object, "Type", location)?,
        &location.field("Type")?,
    )?;
    validate_fields(kind, object, location, context.compiler.states)?;
    match kind {
        "Pass" => {
            if object.contains_key("Result") && object.contains_key("Parameters") {
                return Err(location.error(
                    Code::InvalidSchema,
                    "Pass Result and Parameters are mutually exclusive",
                ));
            }
            Ok(Node::Pass {
                flow: flow(object, location)?,
                result: object.get("Result").map(|v| Arc::new(v.clone())),
                next: transition(object, location, index)?,
            })
        }
        "Task" => task_state(object, location, index, context),
        "Parallel" | "Map" => structured::compile(kind, object, location, index, context, depth),
        "Choice" => choice(object, location, index),
        "Wait" => {
            let source = Value::Object(
                object
                    .iter()
                    .filter(|(key, _)| {
                        matches!(
                            key.as_str(),
                            "Seconds" | "SecondsPath" | "Timestamp" | "TimestampPath"
                        )
                    })
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            );
            Ok(Node::Wait {
                flow: flow(object, location)?,
                source: WaitSource::compile(&source).map_err(|e| CompileError::asl(location, e))?,
                next: transition(object, location, index)?,
            })
        }
        "Succeed" => Ok(Node::Succeed(flow(object, location)?)),
        "Fail" => Ok(Node::Fail {
            error: failure(object, "Error", "ErrorPath", 256, location)?,
            cause: failure(object, "Cause", "CausePath", 32_768, location)?,
        }),
        _ => Err(location.error(Code::InvalidSchema, "invalid state type")),
    }
}

fn choice(
    object: &Map<String, Value>,
    location: &Location,
    index: &BTreeMap<String, StateId>,
) -> Result<Node, CompileError> {
    let pos = location.field("Choices")?;
    let choices = required(object, "Choices", location)?
        .as_array()
        .filter(|v| !v.is_empty() && v.len() <= 128)
        .ok_or_else(|| pos.error(Code::InvalidSchema, "invalid Choice rule array"))?;
    let mut branches = Vec::with_capacity(choices.len());
    for (i, choice) in choices.iter().enumerate() {
        let pos = pos.field(&i.to_string())?;
        let rule = object_value(choice, &pos)?;
        let next = target(required(rule, "Next", &pos)?, &pos.field("Next")?, index)?;
        let predicate = Value::Object(
            rule.iter()
                .filter(|(key, _)| key.as_str() != "Next")
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        );
        branches.push((
            Predicate::compile(&predicate).map_err(|e| CompileError::asl(&pos, e))?,
            next,
        ));
    }
    let default = object
        .get("Default")
        .map(|v| target(v, &location.field("Default")?, index))
        .transpose()?;
    Ok(Node::Choice {
        flow: flow(object, location)?,
        branches,
        default,
    })
}

fn validate_fields(
    kind: &str,
    object: &Map<String, Value>,
    location: &Location,
    states: StateSupport,
) -> Result<(), CompileError> {
    if matches!(kind, "Parallel" | "Map") && states == StateSupport::Sequential {
        return Err(location.field("Type")?.error(
            Code::ProfileUnavailable,
            "state type requires structured execution support",
        ));
    }
    let extra: &[&str] = match kind {
        "Pass" => &["Result", "Parameters", "ResultPath", "Next", "End"],
        "Task" => &[
            "Resource",
            "Parameters",
            "ResultSelector",
            "ResultPath",
            "Retry",
            "Catch",
            "TimeoutSeconds",
            "TimeoutSecondsPath",
            "HeartbeatSeconds",
            "HeartbeatSecondsPath",
            "Next",
            "End",
        ],
        "Choice" => &["Choices", "Default"],
        "Wait" => &[
            "Seconds",
            "SecondsPath",
            "Timestamp",
            "TimestampPath",
            "Next",
            "End",
        ],
        "Succeed" => &[],
        "Fail" => &["Error", "Cause", "ErrorPath", "CausePath"],
        "Parallel" => &[
            "Branches",
            "Parameters",
            "ResultSelector",
            "ResultPath",
            "Retry",
            "Catch",
            "Next",
            "End",
        ],
        "Map" => &[
            "ItemsPath",
            "ItemSelector",
            "Parameters",
            "ItemProcessor",
            "Iterator",
            "MaxConcurrency",
            "MaxConcurrencyPath",
            "ResultSelector",
            "ResultPath",
            "Retry",
            "Catch",
            "Next",
            "End",
        ],
        _ => {
            return Err(location
                .field("Type")?
                .error(Code::InvalidSchema, "invalid state type"));
        }
    };
    let mut allowed = if kind == "Fail" {
        vec!["Type", "Comment", "QueryLanguage"]
    } else {
        COMMON.to_vec()
    };
    allowed.extend(extra);
    fields(object, &allowed, location)?;
    metadata(object, location)?;
    Ok(())
}

pub(super) fn flow(
    object: &Map<String, Value>,
    location: &Location,
) -> Result<DataFlow, CompileError> {
    compile_flow(object, location, true)
}

pub(super) fn map_flow(
    object: &Map<String, Value>,
    location: &Location,
) -> Result<DataFlow, CompileError> {
    compile_flow(object, location, false)
}

fn compile_flow(
    object: &Map<String, Value>,
    location: &Location,
    parameters: bool,
) -> Result<DataFlow, CompileError> {
    let value = Value::Object(
        object
            .iter()
            .filter(|(key, _)| {
                FLOW.contains(&key.as_str()) && (parameters || key.as_str() != "Parameters")
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    );
    match DataFlow::compile_located(&value) {
        Ok(flow) => Ok(flow),
        Err((field, error)) => {
            if field.is_empty() {
                Err(CompileError::asl(location, error))
            } else {
                Err(CompileError::asl(&location.field(field)?, error))
            }
        }
    }
}
pub(super) fn transition(
    object: &Map<String, Value>,
    location: &Location,
    index: &BTreeMap<String, StateId>,
) -> Result<Transition, CompileError> {
    match (object.get("Next"), object.get("End")) {
        (Some(value), None) => target(value, &location.field("Next")?, index).map(Transition::Next),
        (None, Some(Value::Bool(true))) => Ok(Transition::End),
        _ => Err(location.error(
            Code::InvalidSchema,
            "exactly one Next or End=true is required",
        )),
    }
}
fn task_state(
    object: &Map<String, Value>,
    location: &Location,
    index: &BTreeMap<String, StateId>,
    context: &Context<'_>,
) -> Result<Node, CompileError> {
    let resource = text(
        required(object, "Resource", location)?,
        &location.field("Resource")?,
    )?;
    let callback = resource
        == format!(
            "arn:{}:states:::lambda:invoke.waitForTaskToken",
            context.compiler.scope.partition()
        );
    let heartbeat = if callback {
        interval(object, "HeartbeatSeconds", "HeartbeatSecondsPath", location)?
    } else {
        for key in ["HeartbeatSeconds", "HeartbeatSecondsPath"] {
            if object.contains_key(key) {
                return Err(location.field(key)?.error(
                    Code::ProfileUnavailable,
                    "heartbeat requires callback task support",
                ));
            }
        }
        TaskTimeout::Omitted
    };
    let timeout = timeout(object, location)?;
    if matches!((&heartbeat, &timeout), (TaskTimeout::Seconds(h), TaskTimeout::Seconds(t)) if h >= t)
        || matches!((&heartbeat, &timeout), (TaskTimeout::Seconds(h), TaskTimeout::Omitted) if h.get() >= TASK_SECONDS_LIMIT)
    {
        return Err(location.field("HeartbeatSeconds")?.error(
            Code::InvalidSchema,
            "heartbeat must be smaller than Task timeout",
        ));
    }
    Ok(Node::Task {
        flow: flow(object, location)?,
        resource: task::compile(
            resource,
            object.get("Parameters"),
            context.compiler,
            heartbeat,
            location,
        )?,
        timeout,
        recovery: recovery::compile(object, location, index)?,
        next: transition(object, location, index)?,
    })
}

fn timeout(object: &Map<String, Value>, location: &Location) -> Result<TaskTimeout, CompileError> {
    interval(object, "TimeoutSeconds", "TimeoutSecondsPath", location)
}
fn interval(
    object: &Map<String, Value>,
    literal: &str,
    path: &str,
    location: &Location,
) -> Result<TaskTimeout, CompileError> {
    let pos = location.field(literal)?;
    match (object.get(literal), object.get(path)) {
        (None, None) => Ok(TaskTimeout::Omitted),
        (Some(value), None) => {
            positive(value, u64::from(TASK_SECONDS_LIMIT), &pos).map(TaskTimeout::Seconds)
        }
        (None, Some(value)) => {
            let pos = location.field(path)?;
            text(value, &pos)?
                .parse::<ReferencePath>()
                .map(TaskTimeout::Path)
                .map_err(|e| CompileError::asl(&pos, e))
        }
        _ => Err(pos.error(
            Code::InvalidSchema,
            "interval literal and path are mutually exclusive",
        )),
    }
}

fn failure(
    object: &Map<String, Value>,
    literal: &str,
    path: &str,
    max: usize,
    location: &Location,
) -> Result<Failure, CompileError> {
    match (object.get(literal), object.get(path)) {
        (None, None) => Ok(Failure::Omitted),
        (Some(value), None) => {
            let pos = location.field(literal)?;
            let value = text(value, &pos)?;
            if value.len() > max {
                return Err(pos.error(
                    Code::LimitExceeded,
                    "failure literal exceeds its byte limit",
                ));
            }
            Ok(Failure::Literal(value.into()))
        }
        (None, Some(value)) => {
            let pos = location.field(path)?;
            let value = text(value, &pos)?;
            if value.starts_with('$') {
                value
                    .parse::<ReferencePath>()
                    .map_err(|e| CompileError::asl(&pos, e))?;
            }
            value
                .parse::<Expression>()
                .map(Failure::Dynamic)
                .map_err(|e| CompileError::asl(&pos, e))
        }
        _ => Err(location.field(literal)?.error(
            Code::InvalidSchema,
            "failure literal and path are mutually exclusive",
        )),
    }
}
