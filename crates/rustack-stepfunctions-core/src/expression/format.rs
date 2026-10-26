//! Escape-aware scalar formatting, independent of host formatting expressions.

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
                    match value {
                        Value::String(text) => {
                            budget.text(text.len())?;
                            append(&mut output, text)?;
                        }
                        Value::Null | Value::Bool(_) | Value::Number(_) => {
                            let encoded = encode(value)?;
                            let text = from_utf8(&encoded).map_err(|_| AslError::IntrinsicValue)?;
                            append(&mut output, text)?;
                        }
                        _ => return Err(AslError::IntrinsicValue),
                    }
                }
            }
        }
        if args.next().is_some() {
            return Err(AslError::IntrinsicValue);
        }
        Payload::from_value(&Value::String(output))
    }
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
