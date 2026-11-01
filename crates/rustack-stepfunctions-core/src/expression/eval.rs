//! Bounded intrinsic evaluation with borrowed collection output views.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    num::NonZeroUsize,
};

use serde::Serialize;
use serde_json::Value;
use uuid::{Variant, Version};

use super::{Budget, Function, Node, UuidSource, format::Format};
use crate::{AslError, EvaluationContext, Payload, choice::number, payload::encode};

pub(super) fn node<S: UuidSource + ?Sized>(
    expression: &Node,
    data: &Payload,
    context: &EvaluationContext,
    source: &mut S,
    budget: &mut Budget,
) -> Result<Payload, AslError> {
    budget.work(1)?;
    let result = match expression {
        Node::Literal(value) => value.clone(),
        Node::Path(path) => path
            .select_with_work(data, context, &mut budget.units)?
            .to_payload()?,
        Node::Call {
            function,
            args,
            format,
        } => {
            let args = args
                .iter()
                .map(|arg| node(arg, data, context, source, budget))
                .collect::<Result<Vec<_>, _>>()?;
            call(*function, &args, format.as_ref(), source, budget)?
        }
    };
    budget.result(&result)?;
    Ok(result)
}

fn arg(args: &[Payload], index: usize) -> Result<&Value, AslError> {
    args.get(index)
        .map(Payload::value)
        .ok_or(AslError::IntrinsicValue)
}

fn output<T: Serialize + ?Sized>(value: &T) -> Result<Payload, AslError> {
    Payload::from_bytes(encode(value)?)
}

fn parse_json(text: &str, budget: &mut Budget) -> Result<Payload, AslError> {
    budget.text(text.len())?;
    if text
        .bytes()
        .all(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
    {
        return Payload::parse(b"null");
    }
    budget.text(text.len())?;
    let parsed = Payload::parse(text.as_bytes())?;
    budget.text(text.len())?;
    let bytes = text.as_bytes();
    let mut normalized = Vec::with_capacity(bytes.len());
    let mut quoted = false;
    let mut escaped = false;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
        } else if byte == b'"' {
            quoted = true;
        } else if byte == b'-'
            && (index == 0
                || bytes.get(index.saturating_sub(1)).is_some_and(|previous| {
                    matches!(previous, b' ' | b'\t' | b'\r' | b'\n' | b',' | b'[' | b':')
                }))
            && bytes.get(index.saturating_add(1)) == Some(&b'0')
            && bytes.get(index.saturating_add(2)).is_none_or(|next| {
                matches!(next, b' ' | b'\t' | b'\r' | b'\n' | b',' | b']' | b'}')
            })
        {
            continue;
        }
        normalized.push(byte);
    }
    if normalized.len() == bytes.len() {
        return Ok(parsed);
    }
    drop(parsed);
    budget.text(normalized.len())?;
    Payload::parse(&normalized)
}

fn call<S: UuidSource + ?Sized>(
    function: Function,
    args: &[Payload],
    format: Option<&Format>,
    source: &mut S,
    budget: &mut Budget,
) -> Result<Payload, AslError> {
    match function {
        Function::Format => {
            let (first, rest) = args.split_first().ok_or(AslError::IntrinsicValue)?;
            if let Some(format) = format {
                format.render(rest, budget)
            } else {
                let text = first.value().as_str().ok_or(AslError::IntrinsicValue)?;
                budget.text(text.len())?;
                Format::compile(text, rest.len())
                    .map_err(|_| AslError::IntrinsicValue)?
                    .render(rest, budget)
            }
        }
        Function::StringToJson => {
            let text = arg(args, 0)?.as_str().ok_or(AslError::IntrinsicValue)?;
            parse_json(text, budget)
        }
        Function::JsonToString => {
            let encoded = encode(arg(args, 0)?)?;
            budget.text(encoded.len())?;
            let text = String::from_utf8(encoded.to_vec()).map_err(|_| AslError::IntrinsicValue)?;
            Payload::from_value(&Value::String(text))
        }
        Function::Array => output(&args.iter().map(Payload::value).collect::<Vec<_>>()),
        Function::ArrayLength => output(
            &arg(args, 0)?
                .as_array()
                .ok_or(AslError::IntrinsicValue)?
                .len(),
        ),
        Function::ArrayGetItem => {
            let values = arg(args, 0)?.as_array().ok_or(AslError::IntrinsicValue)?;
            let index = arg(args, 1)?
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or(AslError::IntrinsicValue)?;
            output(values.get(index).ok_or(AslError::IntrinsicValue)?)
        }
        Function::ArrayContains => {
            let values = arg(args, 0)?.as_array().ok_or(AslError::IntrinsicValue)?;
            let needle = arg(args, 1)?;
            let mut found = false;
            for value in values {
                if equal(value, needle, budget)? {
                    found = true;
                    break;
                }
            }
            output(&found)
        }
        Function::ArrayPartition => {
            let values = arg(args, 0)?.as_array().ok_or(AslError::IntrinsicValue)?;
            let size = usize::try_from(rounded(arg(args, 1)?)?)
                .ok()
                .and_then(NonZeroUsize::new)
                .ok_or(AslError::IntrinsicValue)?;
            budget.work(values.len())?;
            output(&values.chunks(size.get()).collect::<Vec<_>>())
        }
        Function::StringSplit => split(args, budget),
        Function::MathAdd => {
            let left = rounded(arg(args, 0)?)?;
            let right = rounded(arg(args, 1)?)?;
            output(&left.checked_add(right).ok_or(AslError::IntrinsicValue)?)
        }
        Function::Uuid => {
            let id = source.next_uuid()?;
            if id.get_version() != Some(Version::Random) || id.get_variant() != Variant::RFC4122 {
                return Err(AslError::IntrinsicValue);
            }
            output(&id.to_string())
        }
        Function::JsonMerge => {
            let left = arg(args, 0)?.as_object().ok_or(AslError::IntrinsicValue)?;
            let right = arg(args, 1)?.as_object().ok_or(AslError::IntrinsicValue)?;
            if arg(args, 2)? != &Value::Bool(false) {
                return Err(AslError::IntrinsicValue);
            }
            budget.work(left.len().saturating_add(right.len()))?;
            let mut merged = BTreeMap::new();
            for (key, value) in left.iter().chain(right) {
                budget.text(key.len())?;
                merged.insert(key, value);
            }
            output(&merged)
        }
    }
}

fn split(args: &[Payload], budget: &mut Budget) -> Result<Payload, AslError> {
    let text = arg(args, 0)?.as_str().ok_or(AslError::IntrinsicValue)?;
    let separators = arg(args, 1)?.as_str().ok_or(AslError::IntrinsicValue)?;
    budget.text(text.len().saturating_add(separators.len()))?;
    let mut delimiters = BTreeSet::new();
    for ch in separators.chars() {
        budget.work(1)?;
        delimiters.insert(ch);
        if delimiters.len() > 1024 {
            return Err(AslError::WorkLimit);
        }
    }
    let mut parts = Vec::new();
    for part in text
        .split(|ch| delimiters.contains(&ch))
        .filter(|s| !s.is_empty())
    {
        budget.work(1)?;
        parts.push(part);
    }
    output(&parts)
}

fn rounded(value: &Value) -> Result<i32, AslError> {
    let value = value
        .as_f64()
        .filter(|n| n.is_finite())
        .ok_or(AslError::IntrinsicValue)?;
    let floor = value.floor();
    let rounded = if value - floor >= 0.5 {
        floor + 1.0
    } else {
        floor
    };
    if rounded < f64::from(i32::MIN) || rounded > f64::from(i32::MAX) {
        return Err(AslError::IntrinsicValue);
    }
    rounded
        .to_string()
        .parse()
        .map_err(|_| AslError::IntrinsicValue)
}

fn equal<'a>(left: &'a Value, right: &'a Value, budget: &mut Budget) -> Result<bool, AslError> {
    let mut stack = vec![(left, right)];
    while let Some((left, right)) = stack.pop() {
        budget.work(1)?;
        match (left, right) {
            (Value::Null, Value::Null) => {}
            (Value::Bool(a), Value::Bool(b)) if a == b => {}
            (Value::String(a), Value::String(b)) => {
                budget.text(a.len().saturating_add(b.len()))?;
                if a != b {
                    return Ok(false);
                }
            }
            (Value::Number(a), Value::Number(b)) => {
                if a.is_f64() || b.is_f64() {
                    if !number(left)?
                        .zip(number(right)?)
                        .is_some_and(|(a, b)| a.partial_cmp(&b).is_some_and(Ordering::is_eq))
                    {
                        return Ok(false);
                    }
                } else if a != b {
                    return Ok(false);
                }
            }
            (Value::Array(a), Value::Array(b)) => {
                if a.len() != b.len() {
                    return Ok(false);
                }
                if stack.len().saturating_add(a.len()) > budget.units {
                    return Err(AslError::WorkLimit);
                }
                stack.extend(a.iter().zip(b));
            }
            (Value::Object(a), Value::Object(b)) => {
                if a.len() != b.len() {
                    return Ok(false);
                }
                for (key, value) in a {
                    budget.work(1)?;
                    budget.text(key.len())?;
                    let Some(other) = b.get(key) else {
                        return Ok(false);
                    };
                    if stack.len() >= budget.units {
                        return Err(AslError::WorkLimit);
                    }
                    stack.push((value, other));
                }
            }
            _ => return Ok(false),
        }
    }
    Ok(true)
}
