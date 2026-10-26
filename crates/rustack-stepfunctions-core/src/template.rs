//! Compiled collision-free templates and bounded borrowed output preparation.

use std::{collections::BTreeMap, fmt};

use serde::{
    Serialize, Serializer,
    ser::{SerializeMap, SerializeSeq},
};
use serde_json::Value;

use crate::{
    AslError, EvaluationContext, Expression, Payload, SystemUuidSource, UuidSource,
    expression::Budget,
    payload::{encode, parse_component, validate_component},
};

/// Immutable compiled object template for Parameters/ResultSelector.
#[derive(Clone)]
#[non_exhaustive]
pub struct Template(Node);

#[derive(Clone)]
enum Node {
    Object(Vec<(String, Node)>),
    Array(Vec<Node>),
    Literal(Value),
    Dynamic(Expression),
}

enum Prepared<'a> {
    Object(Vec<(&'a str, Self)>),
    Array(Vec<Self>),
    Literal(&'a Value),
    Dynamic(Payload),
}

impl fmt::Debug for Template {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Template(<redacted>)")
    }
}

impl Template {
    /// Validate source JSON and compile an object template.
    ///
    /// # Errors
    /// Rejects duplicate fields, schema/compiled limits and invalid expressions.
    ///
    /// ```
    /// let template=rustack_stepfunctions_core::Template::parse(br#"{"value.$":"$.n"}"#)?;
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    pub fn parse(input: &[u8]) -> Result<Self, AslError> {
        Self::compile(&parse_component(input)?)
    }

    /// Compile a borrowed tree, rejecting collisions after one `.$` removal.
    ///
    /// # Errors
    /// Rejects nonobject roots, invalid fields, more than32 container levels or
    ///4,096 combined template/expression nodes.
    ///
    /// ```
    /// let template=rustack_stepfunctions_core::Template::compile(&serde_json::json!({"n.$":"States.MathAdd($.n,1)"}))?;
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    pub fn compile(value: &Value) -> Result<Self, AslError> {
        validate_component(value)?;
        if !value.is_object() {
            return Err(AslError::InvalidTemplate);
        }
        Ok(Self(compile(value, 0, &mut 4096)?))
    }

    /// Evaluate a template using fallible system UUID sourcing when needed.
    ///
    /// Async owners isolate entropy acquisition as documented by Expression.
    ///
    /// # Errors
    /// Returns expression, payload or shared resource-budget failures.
    ///
    /// ```
    /// use rustack_stepfunctions_core::{Template,Payload,EvaluationContext};
    /// let input=Payload::parse(br#"{"n":4}"#)?;
    /// let template=Template::parse(br#"{"value.$":"$.n"}"#)?;
    /// assert_eq!(template.evaluate(&input,&EvaluationContext::from(&input))?.value(),&serde_json::json!({"value":4}));
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    pub fn evaluate(
        &self,
        data: &Payload,
        context: &EvaluationContext,
    ) -> Result<Payload, AslError> {
        self.evaluate_with_source(data, context, &mut SystemUuidSource)
    }

    /// Evaluate all dynamic fields under one shared budget and UUID source.
    ///
    /// # Errors
    /// Returns typed data/expression/source failures without partial output.
    ///
    /// ```
    /// use rustack_stepfunctions_core::{Template,Payload,EvaluationContext,SystemUuidSource};
    /// let input=Payload::parse(b"null")?;
    /// let template=Template::parse(b"{}")?;
    /// let output=template.evaluate_with_source(&input,&EvaluationContext::from(&input),&mut SystemUuidSource::default())?;
    /// assert!(output.value().is_object());
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    pub fn evaluate_with_source<S: UuidSource + ?Sized>(
        &self,
        data: &Payload,
        context: &EvaluationContext,
        source: &mut S,
    ) -> Result<Payload, AslError> {
        self.evaluate_scoped(data, context, source, &mut Budget::new())
    }

    pub(crate) fn node_count(&self) -> usize {
        fn count(node: &Node) -> usize {
            match node {
                Node::Literal(_) => 1,
                Node::Dynamic(expression) => 1_usize.saturating_add(expression.node_count()),
                Node::Object(fields) => fields
                    .iter()
                    .fold(1_usize, |n, (_, node)| n.saturating_add(count(node))),
                Node::Array(values) => values
                    .iter()
                    .fold(1_usize, |n, node| n.saturating_add(count(node))),
            }
        }
        count(&self.0)
    }

    pub(crate) fn evaluate_scoped<S: UuidSource + ?Sized>(
        &self,
        data: &Payload,
        context: &EvaluationContext,
        source: &mut S,
        budget: &mut Budget,
    ) -> Result<Payload, AslError> {
        let prepared = prepare(&self.0, data, context, source, budget)?;
        let result = Payload::from_bytes(encode(&prepared)?)?;
        budget.result(&result)?;
        Ok(result)
    }
}

fn compile(value: &Value, depth: usize, remaining: &mut usize) -> Result<Node, AslError> {
    *remaining = remaining.checked_sub(1).ok_or(AslError::InvalidTemplate)?;
    if (value.is_object() || value.is_array()) && depth >= 32 {
        return Err(AslError::InvalidTemplate);
    }
    match value {
        Value::Object(fields) => {
            let mut result = BTreeMap::new();
            for (key, value) in fields {
                let (name, dynamic) = key
                    .strip_suffix(".$")
                    .map_or((key.as_str(), false), |name| (name, true));
                if result.contains_key(name) {
                    return Err(AslError::InvalidTemplate);
                }
                let node = if dynamic {
                    *remaining = remaining.checked_sub(1).ok_or(AslError::InvalidTemplate)?;
                    let expression: Expression =
                        value.as_str().ok_or(AslError::InvalidTemplate)?.parse()?;
                    *remaining = remaining
                        .checked_sub(expression.node_count())
                        .ok_or(AslError::InvalidTemplate)?;
                    Node::Dynamic(expression)
                } else {
                    compile(value, depth.saturating_add(1), remaining)?
                };
                result.insert(name.to_owned(), node);
            }
            Ok(Node::Object(result.into_iter().collect()))
        }
        Value::Array(values) => values
            .iter()
            .map(|value| compile(value, depth.saturating_add(1), remaining))
            .collect::<Result<Vec<_>, _>>()
            .map(Node::Array),
        value => Ok(Node::Literal(value.clone())),
    }
}

fn prepare<'a, S: UuidSource + ?Sized>(
    node: &'a Node,
    data: &Payload,
    context: &EvaluationContext,
    source: &mut S,
    budget: &mut Budget,
) -> Result<Prepared<'a>, AslError> {
    budget.work(1)?;
    match node {
        Node::Literal(value) => Ok(Prepared::Literal(value)),
        Node::Dynamic(expression) => expression
            .evaluate_scoped(data, context, source, budget)
            .map(Prepared::Dynamic),
        Node::Object(fields) => fields
            .iter()
            .map(|(key, value)| {
                prepare(value, data, context, source, budget).map(|value| (key.as_str(), value))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Prepared::Object),
        Node::Array(values) => values
            .iter()
            .map(|value| prepare(value, data, context, source, budget))
            .collect::<Result<Vec<_>, _>>()
            .map(Prepared::Array),
    }
}

impl Serialize for Prepared<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Literal(value) => value.serialize(serializer),
            Self::Dynamic(value) => value.value().serialize(serializer),
            Self::Object(fields) => {
                let mut output = serializer.serialize_map(Some(fields.len()))?;
                for (key, value) in fields {
                    output.serialize_entry(key, value)?;
                }
                output.end()
            }
            Self::Array(values) => {
                let mut output = serializer.serialize_seq(Some(values.len()))?;
                for value in values {
                    output.serialize_element(value)?;
                }
                output.end()
            }
        }
    }
}

#[cfg(test)]
mod tests;
