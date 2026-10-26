//! Lexical graph compilation under one source allocation and aggregate budget.

use std::{collections::BTreeMap, sync::Arc};

use bytes::Bytes;
use serde_json::Value;

use super::{
    AslCompiler, Code, CompileError, CompiledMachine, Diagnostic, Location, Row, StateId, fields,
    graph, metadata, object, positive, required, state, state_name, text,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Root,
    Branch,
    Processor,
}

pub(super) struct Context<'a> {
    pub(super) compiler: &'a AslCompiler,
    original: Bytes,
    states: usize,
    units: usize,
    pub(super) warnings: Vec<Diagnostic>,
}
impl<'a> Context<'a> {
    pub(super) fn new(compiler: &'a AslCompiler, original: Bytes) -> Self {
        Self {
            compiler,
            original,
            states: 1000,
            units: 65_536,
            warnings: Vec::new(),
        }
    }

    pub(super) fn compile(
        &mut self,
        value: &Value,
        location: &Location,
        kind: Kind,
        depth: usize,
    ) -> Result<Arc<CompiledMachine>, CompileError> {
        if depth > 16 {
            return Err(location.error(Code::LimitExceeded, "nested machine depth exceeded"));
        }
        let object = object(value, location)?;
        let mut allowed = vec!["StartAt", "States", "Comment", "QueryLanguage"];
        match kind {
            Kind::Root => allowed.extend(["Version", "TimeoutSeconds"]),
            Kind::Processor => allowed.push("ProcessorConfig"),
            Kind::Branch => {}
        }
        fields(object, &allowed, location)?;
        metadata(object, location)?;
        if let Some(version) = object.get("Version") {
            if version.as_str() != Some("1.0") {
                return Err(location
                    .field("Version")?
                    .error(Code::InvalidSchema, "unsupported ASL version"));
            }
        }
        if let Some(config) = object.get("ProcessorConfig") {
            let pos = location.field("ProcessorConfig")?;
            let config = super::object_value(config, &pos)?;
            fields(config, &["Mode"], &pos)?;
            if config
                .get("Mode")
                .is_some_and(|mode| mode.as_str() != Some("INLINE"))
            {
                return Err(pos
                    .field("Mode")?
                    .error(Code::ProfileUnavailable, "only Inline Map is supported"));
            }
        }
        let timeout = object
            .get("TimeoutSeconds")
            .map(|v| positive(v, 31_536_000, &location.field("TimeoutSeconds")?))
            .transpose()?;
        let start_loc = location.field("StartAt")?;
        let start = text(required(object, "StartAt", location)?, &start_loc)?;
        state_name(start, &start_loc)?;
        let states_loc = location.field("States")?;
        let states = super::object_value(required(object, "States", location)?, &states_loc)?;
        if states.is_empty() {
            return Err(states_loc.error(Code::LimitExceeded, "state scope must not be empty"));
        }
        self.states = self.states.checked_sub(states.len()).ok_or_else(|| {
            states_loc.error(
                Code::LimitExceeded,
                "aggregate state count exceeds one thousand",
            )
        })?;
        let mut index = BTreeMap::new();
        for name in states.keys() {
            state_name(name, &states_loc)?;
            index.insert(name.clone(), StateId(0));
        }
        for (position, id) in index.values_mut().enumerate() {
            *id = StateId(position);
        }
        let start = *index.get(start).ok_or_else(|| {
            start_loc.error(Code::MissingTransitionTarget, "start target does not exist")
        })?;
        let mut rows = Vec::with_capacity(states.len());
        for name in index.keys() {
            let pos = states_loc.field(name)?;
            let value = states
                .get(name)
                .ok_or_else(|| pos.error(Code::InvalidSchema, "state definition missing"))?;
            let node = state::compile(value, &pos, &index, self, depth)?;
            self.units = self
                .units
                .checked_sub(node.units())
                .ok_or_else(|| pos.error(Code::LimitExceeded, "compiled node budget exceeded"))?;
            rows.push(Row {
                name: name.clone(),
                node,
            });
        }
        self.warnings.extend(graph(&rows, start, &states_loc)?);
        Ok(Arc::new(CompiledMachine {
            original: self.original.clone(),
            definition_root: kind == Kind::Root,
            scope: self.compiler.scope.clone(),
            rows,
            start,
            timeout,
        }))
    }
}
