//! Side-effect-free bounded machine compilation and immutable execution IR.

use std::{
    collections::{BTreeMap, VecDeque},
    fmt,
    num::NonZeroU32,
    sync::Arc,
};

use bytes::Bytes;
use rustack_core::json::{JsonLimits, from_slice};
use serde_json::{Map, Value};

macro_rules! read {
    ($owner:ident,$name:ident,$out:ty,$s:ident=>$body:expr,$doc:literal) => {
        #[doc=$doc]
        #[doc=concat!("\n```\n# fn inspect(value: &rustack_stepfunctions_core::",stringify!($owner),") {\nlet _ = value.",stringify!($name),"();\n# }\n```")]
        #[must_use]
        pub fn $name(&self)->$out { let $s=self; $body }
    };
}

mod diagnostic;
mod ir;
mod recovery;
mod state;
mod task;

use diagnostic::{Code, Location};
pub use diagnostic::{CompileError, Diagnostic, DiagnosticSeverity};
pub use ir::{
    CatchResult, Compilation, CompiledMachine, FailureView, StateId, StateView, TaskTimeout,
    Transition,
};
use ir::{Failure, Node, Row};
pub use recovery::{Catcher, Jitter, RecoveryPolicy, Retrier, RetryTiming};
pub use task::{FunctionArn, Integration, TaskSupport, TenantScope};

/// Synchronous bounded compiler; async owners provide bounded CPU admission.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct AslCompiler {
    scope: TenantScope,
    tasks: TaskSupport,
}

impl fmt::Debug for AslCompiler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AslCompiler(<redacted>)")
    }
}

impl AslCompiler {
    /// Configure trusted scope and support derived from the installed adapter.
    ///
    /// ```
    /// use rustack_stepfunctions_core::{AslCompiler,TenantScope,TaskSupport};
    /// let compiler=AslCompiler::new(TenantScope::default(),TaskSupport::Disabled);
    /// ```
    #[must_use]
    pub fn new(scope: TenantScope, tasks: TaskSupport) -> Self {
        Self { scope, tasks }
    }

    /// Compile all fields/targets before publishing an immutable artifact.
    ///
    /// # Errors
    /// Returns one bounded diagnostic with optional redacted typed JSON/ASL source.
    ///
    /// ```
    /// use rustack_stepfunctions_core::AslCompiler;
    /// let result=AslCompiler::default().compile(br#"{"StartAt":"Done","States":{"Done":{"Type":"Succeed"}}}"#)?;
    /// assert_eq!(result.machine().state_count(),1);
    /// # Ok::<(), rustack_stepfunctions_core::CompileError>(())
    /// ```
    pub fn compile(&self, input: &[u8]) -> Result<Compilation, CompileError> {
        let root = Location::root();
        let limits = JsonLimits::new(1024 * 1024, 64, 65_536, 1024 * 1024, 262_144)
            .map_err(|error| CompileError::json(&root, error))?;
        let value: Value =
            from_slice(input, limits).map_err(|error| CompileError::json(&root, error))?;
        let object = object(&value, &root)?;
        fields(
            object,
            &[
                "StartAt",
                "States",
                "Comment",
                "Version",
                "QueryLanguage",
                "TimeoutSeconds",
            ],
            &root,
        )?;
        metadata(object, &root)?;
        if let Some(version) = object.get("Version") {
            if version.as_str() != Some("1.0") {
                return Err(root
                    .field("Version")?
                    .error(Code::InvalidSchema, "unsupported ASL version"));
            }
        }
        let timeout = object
            .get("TimeoutSeconds")
            .map(|v| positive(v, 31_536_000, &root.field("TimeoutSeconds")?))
            .transpose()?;
        let start_loc = root.field("StartAt")?;
        let start = text(required(object, "StartAt", &root)?, &start_loc)?;
        state_name(start, &start_loc)?;
        let states_loc = root.field("States")?;
        let states = object_value(required(object, "States", &root)?, &states_loc)?;
        if states.is_empty() || states.len() > 1000 {
            return Err(states_loc.error(
                Code::LimitExceeded,
                "state count must be between one and one thousand",
            ));
        }
        let mut index = BTreeMap::new();
        for name in states.keys() {
            state_name(name, &states_loc)?;
            index.insert(name.clone(), StateId(0));
        }
        for (position, (_, id)) in index.iter_mut().enumerate() {
            *id = StateId(position);
        }
        let start = *index.get(start).ok_or_else(|| {
            start_loc.error(Code::MissingTransitionTarget, "start target does not exist")
        })?;
        let mut rows = Vec::with_capacity(states.len());
        let mut units = 65_536_usize;
        for name in index.keys() {
            let location = states_loc.field(name)?;
            let value = states
                .get(name)
                .ok_or_else(|| location.error(Code::InvalidSchema, "state definition missing"))?;
            let node = state::compile(value, &location, &index, &self.scope, self.tasks)?;
            units = units.checked_sub(node.units()).ok_or_else(|| {
                location.error(Code::LimitExceeded, "compiled node budget exceeded")
            })?;
            rows.push(Row {
                name: name.clone(),
                node,
            });
        }
        let warnings = graph(&rows, start, &states_loc)?;
        Ok(Compilation {
            machine: Arc::new(CompiledMachine {
                original: Bytes::copy_from_slice(input),
                rows,
                start,
                timeout,
            }),
            warnings,
        })
    }
}

fn graph(
    rows: &[Row],
    start: StateId,
    location: &Location,
) -> Result<Vec<Diagnostic>, CompileError> {
    let mut seen = vec![false; rows.len()];
    let mut queue = VecDeque::new();
    *seen
        .get_mut(start.0)
        .ok_or_else(|| location.error(Code::InvalidSchema, "invalid compiled start"))? = true;
    queue.push_back(start);
    let mut terminal = false;
    while let Some(id) = queue.pop_front() {
        let row = rows
            .get(id.0)
            .ok_or_else(|| location.error(Code::InvalidSchema, "invalid compiled target"))?;
        terminal |= row.node.terminal();
        for next in row.node.edges() {
            let mark = seen
                .get_mut(next.0)
                .ok_or_else(|| location.error(Code::InvalidSchema, "invalid compiled target"))?;
            if !*mark {
                *mark = true;
                queue.push_back(next);
            }
        }
    }
    if !terminal {
        return Err(location.error(
            Code::NoReachableTerminal,
            "no terminal state is reachable from the start",
        ));
    }
    rows.iter()
        .zip(seen)
        .filter(|(_, seen)| !*seen)
        .map(|(row, _)| {
            Ok(location.field(&row.name)?.warning(
                Code::UnreachableState,
                "state is unreachable from the start",
            ))
        })
        .collect()
}

fn object<'a>(
    value: &'a Value,
    location: &Location,
) -> Result<&'a Map<String, Value>, CompileError> {
    object_value(value, location)
}
fn object_value<'a>(
    value: &'a Value,
    location: &Location,
) -> Result<&'a Map<String, Value>, CompileError> {
    value
        .as_object()
        .ok_or_else(|| location.error(Code::InvalidSchema, "expected an object"))
}
fn required<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    location: &Location,
) -> Result<&'a Value, CompileError> {
    match object.get(key) {
        Some(value) => Ok(value),
        None => Err(location
            .field(key)?
            .error(Code::MissingField, "required field is absent")),
    }
}
fn text<'a>(value: &'a Value, location: &Location) -> Result<&'a str, CompileError> {
    value
        .as_str()
        .ok_or_else(|| location.error(Code::InvalidSchema, "expected a string"))
}
fn fields(
    object: &Map<String, Value>,
    allowed: &[&str],
    location: &Location,
) -> Result<(), CompileError> {
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(location.field(key)?.error(
                Code::UnsupportedField,
                "field is unavailable in the selected profile",
            ));
        }
    }
    Ok(())
}
fn metadata(object: &Map<String, Value>, location: &Location) -> Result<(), CompileError> {
    if let Some(comment) = object.get("Comment") {
        text(comment, &location.field("Comment")?)?;
    }
    if let Some(language) = object.get("QueryLanguage") {
        if language.as_str() != Some("JSONPath") {
            return Err(location
                .field("QueryLanguage")?
                .error(Code::ProfileUnavailable, "query language is unavailable"));
        }
    }
    Ok(())
}
fn state_name(value: &str, location: &Location) -> Result<(), CompileError> {
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        Err(location.error(Code::InvalidStateName, "invalid state name"))
    } else {
        Ok(())
    }
}
fn target(
    value: &Value,
    location: &Location,
    index: &BTreeMap<String, StateId>,
) -> Result<StateId, CompileError> {
    let name = text(value, location)?;
    state_name(name, location)?;
    index.get(name).copied().ok_or_else(|| {
        location.error(
            Code::MissingTransitionTarget,
            "transition target does not exist in this scope",
        )
    })
}
fn positive(value: &Value, max: u64, location: &Location) -> Result<NonZeroU32, CompileError> {
    value
        .as_u64()
        .filter(|n| *n <= max)
        .and_then(|n| u32::try_from(n).ok())
        .and_then(NonZeroU32::new)
        .ok_or_else(|| location.error(Code::InvalidSchema, "expected a supported positive integer"))
}

#[cfg(test)]
mod tests;
