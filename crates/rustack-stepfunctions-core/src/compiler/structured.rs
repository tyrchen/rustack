//! Parallel branches and normalized Inline Map definitions, compiled once.

use std::{collections::BTreeMap, num::NonZeroU8};

use serde_json::{Map, Value};

use super::{
    Code, CompileError, Location, MapConcurrency, Node, StateId, recovery, required,
    scope::{Context, Kind},
    state, text,
};
use crate::{ReferencePath, Template};

pub(super) fn compile(
    kind: &str,
    object: &Map<String, Value>,
    location: &Location,
    index: &BTreeMap<String, StateId>,
    context: &mut Context<'_>,
    depth: usize,
) -> Result<Node, CompileError> {
    let next = state::transition(object, location, index)?;
    let recovery = recovery::compile(object, location, index)?;
    if kind == "Parallel" {
        let pos = location.field("Branches")?;
        let branches = required(object, "Branches", location)?
            .as_array()
            .filter(|branches| !branches.is_empty() && branches.len() <= 1000)
            .ok_or_else(|| pos.error(Code::InvalidSchema, "invalid Parallel branch array"))?;
        let flow = state::flow(object, location)?;
        let mut machines = Vec::with_capacity(branches.len());
        for (i, branch) in branches.iter().enumerate() {
            machines.push(context.compile(
                branch,
                &pos.field(&i.to_string())?,
                Kind::Branch,
                depth.saturating_add(1),
            )?);
        }
        return Ok(Node::Parallel {
            flow,
            branches: machines,
            recovery,
            next,
        });
    }
    let processor_pos = location.field("ItemProcessor")?;
    let (key, value) = exclusive(object, "ItemProcessor", "Iterator", location)?
        .ok_or_else(|| processor_pos.error(Code::MissingField, "Map requires a processor"))?;
    let processor = context.compile(
        value,
        &location.field(key)?,
        Kind::Processor,
        depth.saturating_add(1),
    )?;
    let selector = exclusive(object, "ItemSelector", "Parameters", location)?
        .map(|(key, value)| {
            let pos = location.field(key)?;
            Template::compile(value).map_err(|error| CompileError::asl(&pos, error))
        })
        .transpose()?;
    let items = object.get("ItemsPath").map_or_else(
        || {
            "$".parse::<ReferencePath>()
                .map_err(|e| CompileError::asl(location, e))
        },
        |value| reference(value, &location.field("ItemsPath")?),
    )?;
    let concurrency = concurrency(object, location)?;
    let flow = state::map_flow(object, location)?;
    Ok(Node::Map {
        flow,
        items,
        selector,
        processor,
        concurrency,
        recovery,
        next,
    })
}

fn exclusive<'a>(
    object: &'a Map<String, Value>,
    current: &'a str,
    legacy: &'a str,
    location: &Location,
) -> Result<Option<(&'a str, &'a Value)>, CompileError> {
    match (object.get(current), object.get(legacy)) {
        (Some(_), Some(_)) => Err(location.field(current)?.error(
            Code::InvalidSchema,
            "current and legacy fields are mutually exclusive",
        )),
        (Some(value), None) => Ok(Some((current, value))),
        (None, Some(value)) => Ok(Some((legacy, value))),
        (None, None) => Ok(None),
    }
}

fn concurrency(
    object: &Map<String, Value>,
    location: &Location,
) -> Result<MapConcurrency, CompileError> {
    let Some((key, value)) = exclusive(object, "MaxConcurrencyPath", "MaxConcurrency", location)?
    else {
        return static_concurrency(40, location);
    };
    let pos = location.field(key)?;
    if key == "MaxConcurrencyPath" {
        return reference(value, &pos).map(MapConcurrency::Path);
    }
    let number = value.as_u64().filter(|n| *n <= 40).ok_or_else(|| {
        pos.error(
            Code::InvalidSchema,
            "Inline concurrency must be an integer from zero to forty",
        )
    })?;
    let number = u8::try_from(number)
        .map_err(|_| pos.error(Code::InvalidSchema, "invalid Inline concurrency"))?;
    static_concurrency(if number == 0 { 40 } else { number }, &pos)
}

fn static_concurrency(value: u8, location: &Location) -> Result<MapConcurrency, CompileError> {
    NonZeroU8::new(value)
        .map(MapConcurrency::Limit)
        .ok_or_else(|| location.error(Code::InvalidSchema, "invalid Inline concurrency"))
}

fn reference(value: &Value, location: &Location) -> Result<ReferencePath, CompileError> {
    text(value, location)?
        .parse()
        .map_err(|error| CompileError::asl(location, error))
}
