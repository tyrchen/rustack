//! Side-effect-free bounded machine compilation and immutable execution IR.

use std::{
    collections::{BTreeMap, VecDeque},
    fmt,
    num::NonZeroU32,
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
mod scope;
mod state;
mod structured;
mod task;

use diagnostic::{Code, Location};
pub use diagnostic::{CompileError, Diagnostic, DiagnosticSeverity};
pub use ir::{
    CatchResult, Compilation, CompiledMachine, FailureView, MapConcurrency, StateId, StateView,
    TaskTimeout, Transition,
};
use ir::{Failure, Node, Row};
pub use recovery::{Catcher, Jitter, RecoveryPolicy, Retrier, RetryTiming};
pub use task::{FunctionArn, Integration, TaskSupport, TenantScope};

/// Trusted state-instruction profile selected by an execution composition root.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum StateSupport {
    /// The six sequential instruction forms used by the default library profile.
    #[default]
    Sequential,
    /// Also compile Parallel and Inline Map for a structured frame owner.
    Structured,
}

/// Synchronous bounded compiler; async owners provide bounded CPU admission.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct AslCompiler {
    scope: TenantScope,
    tasks: TaskSupport,
    states: StateSupport,
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
        Self {
            scope,
            tasks,
            states: StateSupport::Sequential,
        }
    }

    /// Select trusted IR support independently of installed Task adapters.
    ///
    /// Structured compilation does not enable sequential runtime admission.
    ///
    /// ```
    /// use rustack_stepfunctions_core::{AslCompiler, StateSupport};
    /// let compiler = AslCompiler::default().with_state_support(StateSupport::Structured);
    /// ```
    #[must_use]
    pub fn with_state_support(mut self, states: StateSupport) -> Self {
        self.states = states;
        self
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
        let mut context = scope::Context::new(self, Bytes::copy_from_slice(input));
        let machine = context.compile(&value, &root, scope::Kind::Root, 0)?;
        Ok(Compilation {
            machine,
            warnings: context.warnings,
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
