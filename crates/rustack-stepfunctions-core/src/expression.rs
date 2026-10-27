//! Compiled expressions with explicit UUID sourcing and shared evaluation budgets.

use std::{fmt, str::FromStr};

use getrandom::fill;
use uuid::{Builder, Uuid};

use crate::{AslError, EvaluationContext, Path, Payload};

mod eval;
mod format;
mod parser;

use format::Format;

/// Typed UUID source for deterministic/failure tests and scheduler-safe injection.
pub trait UuidSource {
    /// Produce one securely sourced version4 UUID in production.
    ///
    /// # Errors
    /// Returns entropy/admission failure; never silently substitutes a fixed ID.
    ///
    /// ```
    /// use rustack_stepfunctions_core::{SystemUuidSource, UuidSource};
    /// let id=SystemUuidSource::default().next_uuid()?;
    /// assert_eq!(id.get_version_num(),4);
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    fn next_uuid(&mut self) -> Result<Uuid, AslError>;
}

/// Fallible system entropy adapter; callers own any required blocking isolation.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct SystemUuidSource;

impl UuidSource for SystemUuidSource {
    fn next_uuid(&mut self) -> Result<Uuid, AslError> {
        let mut random = [0_u8; 16];
        fill(&mut random).map_err(AslError::Entropy)?;
        Ok(Builder::from_random_bytes(random).into_uuid())
    }
}

/// Immutable compiled path or allowlisted intrinsic, with redacted diagnostics.
#[derive(Clone)]
#[non_exhaustive]
pub struct Expression(Node);

#[derive(Clone)]
enum Node {
    Path(Path),
    Literal(Payload),
    Call {
        function: Function,
        args: Vec<Node>,
        format: Option<Format>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Function {
    Format,
    StringToJson,
    JsonToString,
    Array,
    ArrayLength,
    ArrayGetItem,
    ArrayContains,
    ArrayPartition,
    StringSplit,
    MathAdd,
    Uuid,
    JsonMerge,
}

impl Function {
    fn parse(name: &str) -> Result<Self, AslError> {
        match name {
            "States.Format" => Ok(Self::Format),
            "States.StringToJson" => Ok(Self::StringToJson),
            "States.JsonToString" => Ok(Self::JsonToString),
            "States.Array" => Ok(Self::Array),
            "States.ArrayLength" => Ok(Self::ArrayLength),
            "States.ArrayGetItem" => Ok(Self::ArrayGetItem),
            "States.ArrayContains" => Ok(Self::ArrayContains),
            "States.ArrayPartition" => Ok(Self::ArrayPartition),
            "States.StringSplit" => Ok(Self::StringSplit),
            "States.MathAdd" => Ok(Self::MathAdd),
            "States.UUID" => Ok(Self::Uuid),
            "States.JsonMerge" => Ok(Self::JsonMerge),
            _ => Err(AslError::InvalidExpression),
        }
    }

    fn arity(self, count: usize) -> bool {
        match self {
            Self::Format => (1..=128).contains(&count),
            Self::Array => count <= 128,
            Self::StringToJson | Self::JsonToString | Self::ArrayLength => count == 1,
            Self::ArrayGetItem
            | Self::ArrayContains
            | Self::ArrayPartition
            | Self::StringSplit
            | Self::MathAdd => count == 2,
            Self::Uuid => count == 0,
            Self::JsonMerge => count == 3,
        }
    }
}

impl fmt::Debug for Expression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Expression(<redacted>)")
    }
}

impl FromStr for Expression {
    type Err = AslError;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        parser::compile(text).map(Self)
    }
}

impl Expression {
    /// Evaluate with fallible OS entropy if a UUID call is reached.
    ///
    /// The OS may wait for entropy during early boot. Async execution owners use
    /// `evaluate_with_source` with pre-acquired IDs or a bounded blocking worker.
    ///
    /// # Errors
    /// Returns missing/type/format, resource-limit or entropy failure.
    ///
    /// ```
    /// use rustack_stepfunctions_core::{Expression,Payload,EvaluationContext};
    /// let data=Payload::parse(br#"{"n":2}"#)?;
    /// let expression:Expression="States.MathAdd($.n, 3)".parse()?;
    /// assert_eq!(expression.evaluate(&data,&EvaluationContext::from(&data))?.value(),&serde_json::json!(5));
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    pub fn evaluate(
        &self,
        data: &Payload,
        context: &EvaluationContext,
    ) -> Result<Payload, AslError> {
        self.evaluate_with_source(data, context, &mut SystemUuidSource)
    }

    /// Evaluate using an explicit UUID port under one shared set of budgets.
    ///
    /// # Errors
    /// Returns evaluation failures and rejects injected non-version4 UUIDs.
    ///
    /// ```
    /// use rustack_stepfunctions_core::{Expression,Payload,EvaluationContext,SystemUuidSource};
    /// let data=Payload::parse(b"null")?;
    /// let expression:Expression="States.Array()".parse()?;
    /// let result=expression.evaluate_with_source(&data,&EvaluationContext::from(&data),&mut SystemUuidSource::default())?;
    /// assert_eq!(result.value(),&serde_json::json!([]));
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    pub fn evaluate_with_source<S: UuidSource + ?Sized>(
        &self,
        data: &Payload,
        context: &EvaluationContext,
        source: &mut S,
    ) -> Result<Payload, AslError> {
        eval::node(&self.0, data, context, source, &mut Budget::new())
    }

    pub(crate) fn evaluate_scoped<S: UuidSource + ?Sized>(
        &self,
        data: &Payload,
        context: &EvaluationContext,
        source: &mut S,
        budget: &mut Budget,
    ) -> Result<Payload, AslError> {
        eval::node(&self.0, data, context, source, budget)
    }

    pub(crate) fn callback_sensitive(&self, input_sensitive: bool) -> bool {
        fn sensitive(node: &Node, input_sensitive: bool) -> bool {
            match node {
                Node::Path(path) => path.callback_sensitive(input_sensitive),
                Node::Literal(_) => false,
                Node::Call { args, .. } => args.iter().any(|arg| sensitive(arg, input_sensitive)),
            }
        }
        sensitive(&self.0, input_sensitive)
    }

    pub(crate) fn node_count(&self) -> usize {
        fn count(node: &Node) -> usize {
            match node {
                Node::Call { args, .. } => args
                    .iter()
                    .fold(1_usize, |sum, node| sum.saturating_add(count(node))),
                Node::Path(_) | Node::Literal(_) => 1,
            }
        }
        count(&self.0)
    }
}

pub(crate) struct Budget {
    units: usize,
    text: usize,
    materialized: usize,
}
impl Budget {
    pub(crate) fn new() -> Self {
        Self {
            units: 10_000,
            text: 4 * 1024 * 1024,
            materialized: 4 * 1024 * 1024,
        }
    }
    pub(crate) fn work(&mut self, count: usize) -> Result<(), AslError> {
        self.units = self.units.checked_sub(count).ok_or(AslError::WorkLimit)?;
        Ok(())
    }
    fn text(&mut self, count: usize) -> Result<(), AslError> {
        self.text = self.text.checked_sub(count).ok_or(AslError::WorkLimit)?;
        Ok(())
    }
    pub(crate) fn result(&mut self, result: &Payload) -> Result<(), AslError> {
        self.materialized = self
            .materialized
            .checked_sub(result.original_bytes().len())
            .ok_or(AslError::WorkLimit)?;
        Ok(())
    }

    pub(crate) fn path(
        &mut self,
        path: &Path,
        data: &Payload,
        context: &EvaluationContext,
    ) -> Result<Payload, AslError> {
        self.work(1)?;
        let result = path
            .select_with_work(data, context, &mut self.units)?
            .to_payload()?;
        self.result(&result)?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
