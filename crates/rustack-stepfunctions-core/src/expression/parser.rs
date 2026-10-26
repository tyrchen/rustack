//! Fully consumed bounded expression grammar; no input-bearing diagnostics.

use serde_json::{Value, from_str};
use winnow::{
    ModalResult, Parser,
    token::{any, take_while},
};

use super::{Function, Node, format::Format};
use crate::{AslError, Path, Payload};

pub(super) fn compile(text: &str) -> Result<Node, AslError> {
    if text.is_empty() || text.len() > 4096 || text.trim() != text {
        return Err(AslError::InvalidExpression);
    }
    if text.starts_with('$') {
        return text.parse::<Path>().map(Node::Path);
    }
    let mut input = text;
    let node = call(&mut input, 0, &mut 1024)?;
    if !input.is_empty() {
        return Err(AslError::InvalidExpression);
    }
    Ok(node)
}

fn consume(input: &mut &str, mut token: &'static str) -> Result<(), AslError> {
    let parsed: ModalResult<&str> = token.parse_next(input);
    parsed.map(|_| ()).map_err(|_| AslError::InvalidExpression)
}

fn space(input: &mut &str) -> Result<(), AslError> {
    let parsed: ModalResult<&str> =
        take_while(0.., |ch: char| matches!(ch, ' ' | '\t' | '\r' | '\n')).parse_next(input);
    parsed.map(|_| ()).map_err(|_| AslError::InvalidExpression)
}

fn unit(remaining: &mut usize) -> Result<(), AslError> {
    *remaining = remaining
        .checked_sub(1)
        .ok_or(AslError::InvalidExpression)?;
    Ok(())
}

fn call(input: &mut &str, depth: usize, remaining: &mut usize) -> Result<Node, AslError> {
    if depth >= 10 {
        return Err(AslError::InvalidExpression);
    }
    unit(remaining)?;
    let name: ModalResult<&str> = take_while(1.., |ch: char| {
        ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_')
    })
    .parse_next(input);
    let function = Function::parse(name.map_err(|_| AslError::InvalidExpression)?)?;
    consume(input, "(")?;
    space(input)?;
    let mut args = Vec::new();
    let mut format_text = None;
    if !input.starts_with(')') {
        loop {
            if args.len() >= 128 {
                return Err(AslError::InvalidExpression);
            }
            let is_format = function == Function::Format && args.is_empty();
            let (arg, raw) = argument(input, depth.saturating_add(1), remaining, is_format)?;
            if is_format {
                format_text = raw;
            }
            args.push(arg);
            space(input)?;
            if input.starts_with(')') {
                break;
            }
            consume(input, ",")?;
            space(input)?;
        }
    }
    consume(input, ")")?;
    if !function.arity(args.len()) {
        return Err(AslError::InvalidExpression);
    }
    if function == Function::JsonMerge {
        if let Some(Node::Literal(value)) = args.get(2) {
            if value.value() != &Value::Bool(false) {
                return Err(AslError::InvalidExpression);
            }
        }
    }
    let format = format_text
        .map(|text| Format::compile(&text, args.len().saturating_sub(1)))
        .transpose()?;
    Ok(Node::Call {
        function,
        args,
        format,
    })
}

fn argument(
    input: &mut &str,
    depth: usize,
    remaining: &mut usize,
    is_format: bool,
) -> Result<(Node, Option<String>), AslError> {
    if input.starts_with("States.") {
        return call(input, depth, remaining).map(|node| (node, None));
    }
    unit(remaining)?;
    if input.starts_with('\'') {
        let (value, raw) = string(input, is_format)?;
        return Ok((
            Node::Literal(Payload::from_value(&Value::String(value))?),
            Some(raw),
        ));
    }
    let token = argument_token(input)?;
    if token.starts_with('$') {
        return token.parse().map(|path| (Node::Path(path), None));
    }
    if token.is_empty() || token.len() > 128 {
        return Err(AslError::InvalidExpression);
    }
    let value: Value = from_str(token).map_err(|_| AslError::InvalidExpression)?;
    if !matches!(value, Value::Number(_) | Value::Bool(_) | Value::Null) {
        return Err(AslError::InvalidExpression);
    }
    Ok((Node::Literal(Payload::from_value(&value)?), None))
}

fn argument_token<'a>(input: &mut &'a str) -> Result<&'a str, AslError> {
    let source = *input;
    let mut brackets = 0_usize;
    let mut quoted = false;
    let mut escaped = false;
    let mut end = source.len();
    for (index, ch) in source.char_indices() {
        if quoted {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '\'' {
                quoted = false;
            }
        } else {
            match ch {
                '\'' => quoted = true,
                '[' => brackets = brackets.checked_add(1).ok_or(AslError::InvalidExpression)?,
                ']' => brackets = brackets.checked_sub(1).ok_or(AslError::InvalidExpression)?,
                ',' | ')' if brackets == 0 => {
                    end = index;
                    break;
                }
                _ => {}
            }
        }
    }
    if quoted || brackets != 0 {
        return Err(AslError::InvalidExpression);
    }
    let token = source.get(..end).ok_or(AslError::InvalidExpression)?;
    *input = source.get(end..).ok_or(AslError::InvalidExpression)?;
    Ok(token.trim_end_matches([' ', '\t', '\r', '\n']))
}

fn string(input: &mut &str, is_format: bool) -> Result<(String, String), AslError> {
    consume(input, "'")?;
    let mut value = String::new();
    let mut raw = String::new();
    loop {
        let parsed: ModalResult<char> = any.parse_next(input);
        match parsed.map_err(|_| AslError::InvalidExpression)? {
            '\'' => return Ok((value, raw)),
            '\\' => {
                let parsed: ModalResult<char> = any.parse_next(input);
                let ch = parsed.map_err(|_| AslError::InvalidExpression)?;
                if !matches!(ch, '\'' | '\\' | '{' | '}') {
                    return Err(AslError::InvalidExpression);
                }
                raw.push('\\');
                raw.push(ch);
                value.push(ch);
            }
            '{' | '}' if !is_format => return Err(AslError::InvalidExpression),
            ch => {
                raw.push(ch);
                value.push(ch);
            }
        }
    }
}
