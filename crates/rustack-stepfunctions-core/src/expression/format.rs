//! Escape-aware bounded AWS value formatting, independent of host formatters.

use std::{mem::take, str::from_utf8};

use serde_json::Value;

use super::Budget;
use crate::{AslError, MAX_PAYLOAD_BYTES, Payload, payload::encode};

#[derive(Clone)]
pub(super) struct Format(Vec<Part>);
#[derive(Clone)]
enum Part {
    Text(String),
    Slot,
}

impl Format {
    pub(super) fn compile(text: &str, slots: usize) -> Result<Self, AslError> {
        let mut parts = Vec::new();
        let mut literal = String::new();
        let mut chars = text.chars();
        let mut count = 0_usize;
        while let Some(ch) = chars.next() {
            match ch {
                '\\' => match chars.next() {
                    Some(ch @ ('\'' | '{' | '}' | '\\')) => literal.push(ch),
                    _ => return Err(AslError::InvalidExpression),
                },
                '{' => {
                    if chars.next() != Some('}') {
                        return Err(AslError::InvalidExpression);
                    }
                    count = count.checked_add(1).ok_or(AslError::InvalidExpression)?;
                    if count > slots {
                        return Err(AslError::InvalidExpression);
                    }
                    if !literal.is_empty() {
                        parts.push(Part::Text(take(&mut literal)));
                    }
                    parts.push(Part::Slot);
                }
                '}' => return Err(AslError::InvalidExpression),
                ch => literal.push(ch),
            }
        }
        if count != slots {
            return Err(AslError::InvalidExpression);
        }
        if !literal.is_empty() {
            parts.push(Part::Text(literal));
        }
        Ok(Self(parts))
    }

    pub(super) fn render(
        &self,
        args: &[Payload],
        budget: &mut Budget,
    ) -> Result<Payload, AslError> {
        let mut args = args.iter();
        let mut output = String::new();
        for part in &self.0 {
            budget.work(1)?;
            match part {
                Part::Text(text) => {
                    budget.text(text.len())?;
                    append(&mut output, text)?;
                }
                Part::Slot => {
                    let value = args.next().ok_or(AslError::IntrinsicValue)?.value();
                    render_value(&mut output, value, budget)?;
                }
            }
        }
        if args.next().is_some() {
            return Err(AslError::IntrinsicValue);
        }
        Payload::from_value(&Value::String(output))
    }
}

fn render_value(output: &mut String, value: &Value, budget: &mut Budget) -> Result<(), AslError> {
    budget.work(1)?;
    match value {
        Value::String(text) => emit(output, text, budget),
        Value::Null | Value::Bool(_) | Value::Number(_) => {
            let encoded = encode(value)?;
            emit(
                output,
                from_utf8(&encoded).map_err(|_| AslError::IntrinsicValue)?,
                budget,
            )
        }
        Value::Array(values) => {
            emit(output, "[", budget)?;
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    emit(output, ", ", budget)?;
                }
                render_value(output, value, budget)?;
            }
            emit(output, "]", budget)
        }
        Value::Object(values) => {
            budget.work(values.len())?;
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_unstable_by_key(|(key, _)| *key);
            emit(output, "{", budget)?;
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index != 0 {
                    emit(output, ", ", budget)?;
                }
                emit(output, key, budget)?;
                emit(output, "=", budget)?;
                render_value(output, value, budget)?;
            }
            emit(output, "}", budget)
        }
    }
}
fn emit(output: &mut String, text: &str, budget: &mut Budget) -> Result<(), AslError> {
    budget.text(text.len())?;
    append(output, text)
}

fn append(output: &mut String, text: &str) -> Result<(), AslError> {
    if output.len().saturating_add(text.len()) > MAX_PAYLOAD_BYTES {
        return Err(AslError::DataLimit);
    }
    output
        .try_reserve(text.len())
        .map_err(AslError::Allocation)?;
    output.push_str(text);
    Ok(())
}
