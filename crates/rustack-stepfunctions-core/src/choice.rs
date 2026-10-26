//! Compiled short-circuit Choice predicates with shared evaluation budgets.

use std::{borrow::Cow, cmp::Ordering, fmt};

use chrono::{DateTime, Timelike};
use rustack_core::json::{JsonLimits, from_slice};
use serde_json::{Number, Value};

use crate::{AslError, EvaluationContext, Payload, ReferencePath, payload::validate_predicate};

mod glob;
use glob::Glob;

/// Immutable compiled Choice predicate; rule transition targets are compiler-owned.
#[derive(Clone)]
#[non_exhaustive]
pub struct Predicate(Node);

#[derive(Clone)]
enum Node {
    And(Vec<Predicate>),
    Or(Vec<Predicate>),
    Not(Box<Predicate>),
    Compare {
        variable: ReferencePath,
        comparison: Comparison,
    },
}

#[derive(Clone)]
enum Comparison {
    Test(Test, bool),
    String(Order, Operand<String>),
    Numeric(Order, Operand<f64>),
    Timestamp(Order, Operand<Timestamp>),
    Boolean(Operand<bool>),
    Matches(Glob),
}

#[derive(Clone)]
enum Operand<T> {
    Literal(T),
    Path(ReferencePath),
}

enum Resolved<'a, T> {
    Literal(&'a T),
    Value(&'a Value),
}

enum Family {
    String,
    Numeric,
    Timestamp,
}

#[derive(Clone, Copy)]
enum Test {
    Present,
    Null,
    String,
    Numeric,
    Boolean,
    Timestamp,
}

#[derive(Clone, Copy)]
enum Order {
    Equal,
    Less,
    Greater,
    LessEqual,
    GreaterEqual,
}

#[derive(Clone)]
struct Timestamp {
    seconds: i64,
    fraction: String,
    source_bytes: usize,
}

impl Timestamp {
    fn compare(&self, other: &Self) -> Ordering {
        self.seconds
            .cmp(&other.seconds)
            .then_with(|| self.fraction.cmp(&other.fraction))
    }
}

impl Order {
    fn accepts(self, ordering: Ordering) -> bool {
        match self {
            Self::Equal => ordering.is_eq(),
            Self::Less => ordering.is_lt(),
            Self::Greater => ordering.is_gt(),
            Self::LessEqual => !ordering.is_gt(),
            Self::GreaterEqual => !ordering.is_lt(),
        }
    }
}

impl fmt::Debug for Predicate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Predicate(<redacted>)")
    }
}

impl Predicate {
    /// Preflight source JSON before compiling a predicate without its Next field.
    ///
    /// # Errors
    /// Rejects duplicate/unknown fields, invalid operators and structural limits.
    ///
    /// ```
    /// use rustack_stepfunctions_core::Predicate;
    /// let predicate = Predicate::parse(br#"{"Variable":"$.n","NumericEquals":3}"#)?;
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    pub fn parse(input: &[u8]) -> Result<Self, AslError> {
        let limits = JsonLimits::new(1024 * 1024, 64, 4096, 262_144, 262_144)?;
        let value: Value = from_slice(input, limits)?;
        Self::compile(&value)
    }

    /// Compile an existing predicate tree, borrowing before validation/cloning.
    ///
    /// # Errors
    /// Rejects invalid predicates, more than1,024 nodes or32 predicate levels.
    ///
    /// ```
    /// use rustack_stepfunctions_core::Predicate;
    /// let predicate = Predicate::compile(&serde_json::json!({"Variable":"$.x","IsPresent":true}))?;
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    pub fn compile(value: &Value) -> Result<Self, AslError> {
        validate_predicate(value)?;
        Self::node(value, 0, &mut 1024)
    }

    /// Evaluate left-to-right against validated data and internal context.
    ///
    /// # Errors
    /// Missing references (except `IsPresent`), unsupported numeric ranges or
    /// shared work/string-byte exhaustion fail explicitly, without coercion.
    ///
    /// ```
    /// use rustack_stepfunctions_core::{Predicate, Payload, EvaluationContext};
    /// let data = Payload::parse(br#"{"n":3}"#)?;
    /// let predicate = Predicate::compile(&serde_json::json!({"Variable":"$.n","NumericEquals":3}))?;
    /// assert!(predicate.evaluate(&data,&EvaluationContext::from(&data))?);
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    pub fn evaluate(&self, data: &Payload, context: &EvaluationContext) -> Result<bool, AslError> {
        self.eval(
            data,
            context,
            &mut Budget {
                units: 10_000,
                bytes: 1024 * 1024,
            },
        )
    }

    fn node(value: &Value, depth: usize, remaining: &mut usize) -> Result<Self, AslError> {
        if depth >= 32 {
            return Err(AslError::InvalidPredicate);
        }
        *remaining = remaining.checked_sub(1).ok_or(AslError::InvalidPredicate)?;
        let object = value.as_object().ok_or(AslError::InvalidPredicate)?;
        for (key, is_and) in [("And", true), ("Or", false)] {
            if let Some(children) = object.get(key) {
                if object.len() != 1 {
                    return Err(AslError::InvalidPredicate);
                }
                let children = children
                    .as_array()
                    .filter(|v| !v.is_empty() && v.len() <= 128)
                    .ok_or(AslError::InvalidPredicate)?;
                let compiled = children
                    .iter()
                    .map(|v| Self::node(v, depth.saturating_add(1), remaining))
                    .collect::<Result<Vec<_>, _>>()?;
                return Ok(Self(if is_and {
                    Node::And(compiled)
                } else {
                    Node::Or(compiled)
                }));
            }
        }
        if let Some(child) = object.get("Not") {
            if object.len() != 1 {
                return Err(AslError::InvalidPredicate);
            }
            return Ok(Self(Node::Not(Box::new(Self::node(
                child,
                depth.saturating_add(1),
                remaining,
            )?))));
        }
        if object.len() != 2 {
            return Err(AslError::InvalidPredicate);
        }
        let variable = object
            .get("Variable")
            .and_then(Value::as_str)
            .ok_or(AslError::InvalidPredicate)?
            .parse()?;
        let (operator, value) = object
            .iter()
            .find(|(k, _)| k.as_str() != "Variable")
            .ok_or(AslError::InvalidPredicate)?;
        Ok(Self(Node::Compare {
            variable,
            comparison: comparison(operator, value)?,
        }))
    }

    fn eval(
        &self,
        data: &Payload,
        context: &EvaluationContext,
        budget: &mut Budget,
    ) -> Result<bool, AslError> {
        budget.step()?;
        match &self.0 {
            Node::And(children) => {
                for child in children {
                    if !child.eval(data, context, budget)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Node::Or(children) => {
                for child in children {
                    if child.eval(data, context, budget)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Node::Not(child) => Ok(!child.eval(data, context, budget)?),
            Node::Compare {
                variable,
                comparison,
            } => {
                let value = variable.lookup(data, context);
                if let Comparison::Test(Test::Present, expected) = comparison {
                    return Ok(value.is_some() == *expected);
                }
                evaluate(
                    comparison,
                    value.ok_or(AslError::Missing)?,
                    data,
                    context,
                    budget,
                )
            }
        }
    }
}

fn comparison(operator: &str, value: &Value) -> Result<Comparison, AslError> {
    let test = match operator {
        "IsPresent" => Some(Test::Present),
        "IsNull" => Some(Test::Null),
        "IsString" => Some(Test::String),
        "IsNumeric" => Some(Test::Numeric),
        "IsBoolean" => Some(Test::Boolean),
        "IsTimestamp" => Some(Test::Timestamp),
        _ => None,
    };
    if let Some(test) = test {
        return Ok(Comparison::Test(
            test,
            value.as_bool().ok_or(AslError::InvalidPredicate)?,
        ));
    }
    if operator == "StringMatches" {
        return Ok(Comparison::Matches(Glob::compile(
            value.as_str().ok_or(AslError::InvalidPredicate)?,
        )?));
    }
    let (name, path) = operator
        .strip_suffix("Path")
        .map_or((operator, false), |s| (s, true));
    if name == "BooleanEquals" {
        return Ok(Comparison::Boolean(operand(value, path, Value::as_bool)?));
    }
    let (family, suffix) = if let Some(s) = name.strip_prefix("String") {
        (Family::String, s)
    } else if let Some(s) = name.strip_prefix("Numeric") {
        (Family::Numeric, s)
    } else if let Some(s) = name.strip_prefix("Timestamp") {
        (Family::Timestamp, s)
    } else {
        return Err(AslError::InvalidPredicate);
    };
    let order = match suffix {
        "Equals" => Order::Equal,
        "LessThan" => Order::Less,
        "GreaterThan" => Order::Greater,
        "LessThanEquals" => Order::LessEqual,
        "GreaterThanEquals" => Order::GreaterEqual,
        _ => return Err(AslError::InvalidPredicate),
    };
    match family {
        Family::String => Ok(Comparison::String(
            order,
            operand(value, path, |v| v.as_str().map(str::to_owned))?,
        )),
        Family::Numeric => Ok(Comparison::Numeric(
            order,
            operand(value, path, |v| number(v).ok().flatten())?,
        )),
        Family::Timestamp => Ok(Comparison::Timestamp(
            order,
            operand(value, path, |v| v.as_str().and_then(timestamp))?,
        )),
    }
}

fn operand<T>(
    value: &Value,
    path: bool,
    literal: impl FnOnce(&Value) -> Option<T>,
) -> Result<Operand<T>, AslError> {
    if path {
        Ok(Operand::Path(
            value.as_str().ok_or(AslError::InvalidPredicate)?.parse()?,
        ))
    } else {
        Ok(Operand::Literal(
            literal(value).ok_or(AslError::InvalidPredicate)?,
        ))
    }
}

fn resolve<'a, T>(
    operand: &'a Operand<T>,
    data: &'a Payload,
    context: &'a EvaluationContext,
) -> Result<Resolved<'a, T>, AslError> {
    match operand {
        Operand::Literal(value) => Ok(Resolved::Literal(value)),
        Operand::Path(path) => Ok(Resolved::Value(
            path.lookup(data, context).ok_or(AslError::Missing)?,
        )),
    }
}

fn number(value: &Value) -> Result<Option<f64>, AslError> {
    const LIMIT: f64 = 9_007_199_254_740_992.0;
    if let Some(n) = value.as_u64() {
        if n > 9_007_199_254_740_992 {
            return Err(AslError::NumericRange);
        }
    }
    if let Some(n) = value.as_i64() {
        if !(-9_007_199_254_740_992..=9_007_199_254_740_992).contains(&n) {
            return Err(AslError::NumericRange);
        }
    }
    let result = value.as_f64();
    let floating = value.as_number().is_some_and(Number::is_f64);
    if result.is_some_and(|n| !n.is_finite() || n.abs() > LIMIT || (floating && n.abs() == LIMIT)) {
        return Err(AslError::NumericRange);
    }
    Ok(result)
}

fn timestamp(value: &str) -> Option<Timestamp> {
    let date = DateTime::parse_from_rfc3339(value).ok()?;
    if date.nanosecond() >= 1_000_000_000 {
        None
    } else {
        let fraction = if let Some(dot) = value.find('.') {
            let tail = value.get(dot.checked_add(1)?..)?;
            let end = tail
                .find(|ch: char| !ch.is_ascii_digit())
                .unwrap_or(tail.len());
            tail.get(..end)?.trim_end_matches('0')
        } else {
            ""
        };
        Some(Timestamp {
            seconds: date.timestamp(),
            fraction: fraction.to_owned(),
            source_bytes: value.len(),
        })
    }
}

fn evaluate(
    comparison: &Comparison,
    value: &Value,
    data: &Payload,
    context: &EvaluationContext,
    budget: &mut Budget,
) -> Result<bool, AslError> {
    match comparison {
        Comparison::Test(test, expected) => {
            let actual = match test {
                Test::Present => true,
                Test::Null => value.is_null(),
                Test::String => value.is_string(),
                Test::Numeric => value.is_number(),
                Test::Boolean => value.is_boolean(),
                Test::Timestamp => {
                    if let Some(text) = value.as_str() {
                        budget.text(text.len())?;
                        timestamp(text).is_some()
                    } else {
                        false
                    }
                }
            };
            Ok(actual == *expected)
        }
        Comparison::Boolean(operand) => {
            let expected = match resolve(operand, data, context)? {
                Resolved::Literal(value) => Some(*value),
                Resolved::Value(value) => value.as_bool(),
            };
            Ok(value.as_bool().zip(expected).is_some_and(|(a, b)| a == b))
        }
        Comparison::Numeric(order, operand) => {
            let expected = match resolve(operand, data, context)? {
                Resolved::Literal(value) => Some(*value),
                Resolved::Value(value) => number(value)?,
            };
            Ok(number(value)?
                .zip(expected)
                .and_then(|(a, b)| a.partial_cmp(&b))
                .is_some_and(|ordering| order.accepts(ordering)))
        }
        Comparison::String(order, operand) => {
            let expected = match resolve(operand, data, context)? {
                Resolved::Literal(value) => Some(value.as_str()),
                Resolved::Value(value) => value.as_str(),
            };
            if let Some((a, b)) = value.as_str().zip(expected) {
                budget.text(a.len().saturating_add(b.len()))?;
                Ok(order.accepts(a.cmp(b)))
            } else {
                Ok(false)
            }
        }
        Comparison::Timestamp(order, operand) => {
            let expected = match resolve(operand, data, context)? {
                Resolved::Literal(value) => {
                    budget.text(value.source_bytes)?;
                    Some(Cow::Borrowed(value))
                }
                Resolved::Value(value) => {
                    if let Some(text) = value.as_str() {
                        budget.text(text.len())?;
                        timestamp(text).map(Cow::Owned)
                    } else {
                        None
                    }
                }
            };
            let actual = if let Some(text) = value.as_str() {
                budget.text(text.len())?;
                timestamp(text)
            } else {
                None
            };
            Ok(actual
                .zip(expected)
                .is_some_and(|(a, b)| order.accepts(a.compare(&b))))
        }
        Comparison::Matches(glob) => {
            if let Some(text) = value.as_str() {
                budget.text(text.len())?;
                glob.matches(text, budget)
            } else {
                Ok(false)
            }
        }
    }
}

struct Budget {
    units: usize,
    bytes: usize,
}
impl Budget {
    fn step(&mut self) -> Result<(), AslError> {
        self.units = self.units.checked_sub(1).ok_or(AslError::WorkLimit)?;
        Ok(())
    }
    fn text(&mut self, bytes: usize) -> Result<(), AslError> {
        self.bytes = self.bytes.checked_sub(bytes).ok_or(AslError::WorkLimit)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
