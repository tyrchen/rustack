//! Compiled bounded `JSONPath` subset, borrowed selection and result merging.

use std::{fmt, str::FromStr};

use serde_json::{Map, Value};
use winnow::{
    ModalResult, Parser,
    token::{any, take_while},
};

use crate::{AslError, EvaluationContext, Payload, payload::encode};

const MAX_PATH_BYTES: usize = 1024;
const MAX_SEGMENTS: usize = 128;
const MAX_WORK: usize = 10_000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Root {
    Data,
    Context,
}

#[derive(Clone)]
enum Segment {
    Member(String),
    Index(usize),
    Wildcard,
    Slice { start: usize, end: Option<usize> },
}

/// Immutable compiled path, with no runtime reparsing or unsupported operators.
#[derive(Clone)]
#[non_exhaustive]
pub struct Path {
    root: Root,
    segments: Vec<Segment>,
    selection: bool,
}

impl fmt::Debug for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Path(<redacted>)")
    }
}

impl FromStr for Path {
    type Err = AslError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.is_empty() || text.len() > MAX_PATH_BYTES {
            return Err(AslError::InvalidPath);
        }
        let mut input = text;
        literal(&mut input, "$")?;
        let root = if input.starts_with('$') {
            literal(&mut input, "$")?;
            Root::Context
        } else {
            Root::Data
        };
        let mut segments = Vec::new();
        let mut selection = false;
        while !input.is_empty() {
            if segments.len() >= MAX_SEGMENTS {
                return Err(AslError::InvalidPath);
            }
            let segment = if input.starts_with('.') {
                literal(&mut input, ".")?;
                let first: ModalResult<&str> =
                    take_while(1, identifier_first).parse_next(&mut input);
                let first = first.map_err(|_| AslError::InvalidPath)?;
                let rest: ModalResult<&str> =
                    take_while(0.., identifier_rest).parse_next(&mut input);
                let rest = rest.map_err(|_| AslError::InvalidPath)?;
                Segment::Member(format!("{first}{rest}"))
            } else {
                bracket(&mut input)?
            };
            selection |= matches!(segment, Segment::Wildcard | Segment::Slice { .. });
            segments.push(segment);
        }
        Ok(Self {
            root,
            segments,
            selection,
        })
    }
}

fn identifier_first(value: char) -> bool {
    value == '_' || value.is_ascii_alphabetic() || (!value.is_ascii() && !value.is_control())
}

fn identifier_rest(value: char) -> bool {
    identifier_first(value) || value.is_ascii_digit()
}

fn literal(input: &mut &str, mut token: &'static str) -> Result<(), AslError> {
    let parsed: ModalResult<&str> = token.parse_next(input);
    parsed.map(|_| ()).map_err(|_| AslError::InvalidPath)
}

fn number(input: &mut &str) -> Result<Option<usize>, AslError> {
    let parsed: ModalResult<&str> = take_while(0.., |c: char| c.is_ascii_digit()).parse_next(input);
    let text = parsed.map_err(|_| AslError::InvalidPath)?;
    if text.is_empty() {
        Ok(None)
    } else {
        text.parse().map(Some).map_err(|_| AslError::InvalidPath)
    }
}

fn quoted(input: &mut &str) -> Result<String, AslError> {
    literal(input, "'")?;
    let mut key = String::new();
    loop {
        let parsed: ModalResult<char> = any.parse_next(input);
        match parsed.map_err(|_| AslError::InvalidPath)? {
            '\'' => return Ok(key),
            '\\' => {
                let escaped: ModalResult<char> = any.parse_next(input);
                match escaped.map_err(|_| AslError::InvalidPath)? {
                    value @ ('\'' | '\\') => key.push(value),
                    _ => return Err(AslError::InvalidPath),
                }
            }
            value if value.is_control() => return Err(AslError::InvalidPath),
            value => key.push(value),
        }
    }
}

fn bracket(input: &mut &str) -> Result<Segment, AslError> {
    literal(input, "[")?;
    let segment = if input.starts_with('\'') {
        Segment::Member(quoted(input)?)
    } else if input.starts_with('*') {
        literal(input, "*")?;
        Segment::Wildcard
    } else {
        let lower = number(input)?;
        if input.starts_with(':') {
            literal(input, ":")?;
            Segment::Slice {
                start: lower.unwrap_or(0),
                end: number(input)?,
            }
        } else {
            Segment::Index(lower.ok_or(AslError::InvalidPath)?)
        }
    };
    literal(input, "]")?;
    Ok(segment)
}

/// Shape of a path result; JSON null is still a single selected value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MatchKind {
    /// No value matched a singular traversal.
    Missing,
    /// Exactly one value was selected without selection operators.
    Single,
    /// Wildcard/slice semantics, including zero or one selected value.
    Selection,
}

/// Borrowed immutable matches, constructible only through a bounded path.
#[non_exhaustive]
pub struct PathMatches<'a> {
    kind: MatchKind,
    values: Vec<&'a Value>,
}

impl fmt::Debug for PathMatches<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PathMatches")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl PathMatches<'_> {
    /// Inspect cardinality semantics without exposing business data.
    ///
    /// ```
    /// use rustack_stepfunctions_core::{Path, Payload, MatchKind};
    /// let data = Payload::parse(b"null")?;
    /// let context = rustack_stepfunctions_core::EvaluationContext::from(&data);
    /// let result = "$".parse::<Path>()?.select(&data, &context)?;
    /// assert_eq!(result.kind(), MatchKind::Single);
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    #[must_use]
    pub fn kind(&self) -> MatchKind {
        self.kind
    }

    /// Read a singular match; missing and selection results return None.
    ///
    /// ```
    /// use rustack_stepfunctions_core::{Path, Payload};
    /// let data = Payload::parse(b"null")?;
    /// assert!("$".parse::<Path>()?.select(&data, &rustack_stepfunctions_core::EvaluationContext::from(&data))?.single().is_some());
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    #[must_use]
    pub fn single(&self) -> Option<&Value> {
        if self.kind == MatchKind::Single {
            self.values.first().copied()
        } else {
            None
        }
    }

    /// Materialize a complete bounded result; selections always become arrays.
    ///
    /// # Errors
    /// Rejects missing matches, excessive encoded data or constructed depth.
    ///
    /// ```
    /// use rustack_stepfunctions_core::{Path, Payload};
    /// let data = Payload::parse(b"[7]")?;
    /// let result = "$[*]".parse::<Path>()?.select(&data, &rustack_stepfunctions_core::EvaluationContext::from(&data))?.to_payload()?;
    /// assert_eq!(result.original_bytes(), b"[7]");
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    pub fn to_payload(&self) -> Result<Payload, AslError> {
        let bytes = match self.kind {
            MatchKind::Missing => return Err(AslError::Missing),
            MatchKind::Single => encode(self.values.first().ok_or(AslError::Missing)?)?,
            MatchKind::Selection => encode(&self.values)?,
        };
        Payload::from_bytes(bytes)
    }
}

impl Path {
    /// Evaluate compiled segments against immutable data and context roots.
    ///
    /// # Errors
    /// Rejects evaluation beyond10,000 visited candidates/selected members.
    ///
    /// ```
    /// use rustack_stepfunctions_core::{Path, Payload};
    /// let data = Payload::parse(br#"{"items":[{"n":3}]}"#)?;
    /// let path: Path = "$.items[*].n".parse()?;
    /// assert_eq!(path.select(&data, &rustack_stepfunctions_core::EvaluationContext::from(&data))?.to_payload()?.value(), &serde_json::json!([3]));
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    pub fn select<'a>(
        &self,
        data: &'a Payload,
        context: &'a EvaluationContext,
    ) -> Result<PathMatches<'a>, AslError> {
        let mut remaining = MAX_WORK;
        self.select_with_work(data, context, &mut remaining)
    }

    pub(crate) fn select_with_work<'a>(
        &self,
        data: &'a Payload,
        context: &'a EvaluationContext,
        remaining: &mut usize,
    ) -> Result<PathMatches<'a>, AslError> {
        let root = match self.root {
            Root::Data => data.value(),
            Root::Context => context.value(),
        };
        let mut values = vec![root];
        for segment in &self.segments {
            let mut next = Vec::new();
            for value in values {
                spend(remaining, 1)?;
                select_segment(segment, value, &mut next, remaining)?;
            }
            values = next;
        }
        let kind = if self.selection {
            MatchKind::Selection
        } else if values.is_empty() {
            MatchKind::Missing
        } else {
            MatchKind::Single
        };
        Ok(PathMatches { kind, values })
    }
}

fn spend(remaining: &mut usize, amount: usize) -> Result<(), AslError> {
    *remaining = remaining.checked_sub(amount).ok_or(AslError::WorkLimit)?;
    Ok(())
}

fn select_segment<'a>(
    segment: &Segment,
    value: &'a Value,
    next: &mut Vec<&'a Value>,
    remaining: &mut usize,
) -> Result<(), AslError> {
    match segment {
        Segment::Member(key) => {
            if let Some(value) = value.as_object().and_then(|object| object.get(key)) {
                next.push(value);
            }
        }
        Segment::Index(index) => {
            if let Some(value) = value.as_array().and_then(|array| array.get(*index)) {
                next.push(value);
            }
        }
        Segment::Wildcard => match value {
            Value::Array(values) => {
                spend(remaining, values.len())?;
                next.extend(values);
            }
            Value::Object(values) => {
                spend(remaining, values.len())?;
                let mut ordered: Vec<_> = values.iter().collect();
                ordered.sort_unstable_by_key(|(key, _)| *key);
                next.extend(ordered.into_iter().map(|(_, value)| value));
            }
            _ => {}
        },
        Segment::Slice { start, end } => {
            if let Some(values) = value.as_array() {
                let lower = (*start).min(values.len());
                let upper = end.unwrap_or(values.len()).min(values.len());
                if upper > lower {
                    spend(remaining, upper.saturating_sub(lower))?;
                    next.extend(values.get(lower..upper).ok_or(AslError::InvalidPath)?);
                }
            }
        }
    }
    Ok(())
}

/// Compiled singular reference into either immutable root.
#[derive(Clone)]
#[non_exhaustive]
pub struct ReferencePath(Path);

impl fmt::Debug for ReferencePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ReferencePath(<redacted>)")
    }
}

impl TryFrom<Path> for ReferencePath {
    type Error = AslError;
    fn try_from(path: Path) -> Result<Self, Self::Error> {
        if path.selection {
            Err(AslError::InvalidReference)
        } else {
            Ok(Self(path))
        }
    }
}

impl FromStr for ReferencePath {
    type Err = AslError;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::try_from(text.parse::<Path>()?)
    }
}

impl ReferencePath {
    /// Look up a singular reference; missing/container mismatch returns None.
    ///
    /// ```
    /// use rustack_stepfunctions_core::{ReferencePath, Payload};
    /// let data = Payload::parse(br#"{"n":null}"#)?;
    /// assert!("$.n".parse::<ReferencePath>()?.lookup(&data, &rustack_stepfunctions_core::EvaluationContext::from(&data)).is_some());
    /// assert!("$.absent".parse::<ReferencePath>()?.lookup(&data, &rustack_stepfunctions_core::EvaluationContext::from(&data)).is_none());
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    #[must_use]
    pub fn lookup<'a>(
        &self,
        data: &'a Payload,
        context: &'a EvaluationContext,
    ) -> Option<&'a Value> {
        let mut value = match self.0.root {
            Root::Data => data.value(),
            Root::Context => context.value(),
        };
        for segment in &self.0.segments {
            value = match segment {
                Segment::Member(key) => value.as_object()?.get(key)?,
                Segment::Index(index) => value.as_array()?.get(*index)?,
                Segment::Wildcard | Segment::Slice { .. } => return None,
            };
        }
        Some(value)
    }
}

/// Data-root reference validated for result writes; context writes are rejected.
#[derive(Clone)]
#[non_exhaustive]
pub struct ResultPath(ReferencePath);

impl fmt::Debug for ResultPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ResultPath(<redacted>)")
    }
}

impl TryFrom<ReferencePath> for ResultPath {
    type Error = AslError;
    fn try_from(path: ReferencePath) -> Result<Self, Self::Error> {
        if path.0.root == Root::Context {
            Err(AslError::InvalidResultTarget)
        } else {
            Ok(Self(path))
        }
    }
}

impl FromStr for ResultPath {
    type Err = AslError;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::try_from(text.parse::<ReferencePath>()?)
    }
}

impl ResultPath {
    /// Merge into original state input without mutating either input on failure.
    ///
    /// # Errors
    /// Rejects scalar traversal, absent/out-of-range array slots and output limits.
    ///
    /// ```
    /// use rustack_stepfunctions_core::{ResultPath, Payload};
    /// let state = Payload::parse(br#"{"n":1}"#)?;
    /// let result = Payload::parse(b"2")?;
    /// let output = "$.nested.value".parse::<ResultPath>()?.merge(&state, &result)?;
    /// assert_eq!(output.value()["nested"]["value"], 2);
    /// assert_eq!(state.value(), &serde_json::json!({"n":1}));
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    pub fn merge(&self, original: &Payload, result: &Payload) -> Result<Payload, AslError> {
        if self.0.0.segments.is_empty() {
            return Ok(result.clone());
        }
        let mut output = original.value().clone();
        let mut destination = &mut output;
        let mut segments = self.0.0.segments.iter().peekable();
        while let Some(segment) = segments.next() {
            let last = segments.peek().is_none();
            destination = match segment {
                Segment::Member(key) => {
                    let object = destination.as_object_mut().ok_or(AslError::Merge)?;
                    if last {
                        object.insert(key.clone(), result.value().clone());
                        break;
                    }
                    if !object.contains_key(key) {
                        if !matches!(segments.peek(), Some(Segment::Member(_))) {
                            return Err(AslError::Merge);
                        }
                        object.insert(key.clone(), Value::Object(Map::new()));
                    }
                    object.get_mut(key).ok_or(AslError::Merge)?
                }
                Segment::Index(index) => {
                    let slot = destination
                        .as_array_mut()
                        .and_then(|values| values.get_mut(*index))
                        .ok_or(AslError::Merge)?;
                    if last {
                        *slot = result.value().clone();
                        break;
                    }
                    slot
                }
                Segment::Wildcard | Segment::Slice { .. } => {
                    return Err(AslError::InvalidResultTarget);
                }
            };
        }
        Payload::from_value(&output)
    }
}

#[cfg(test)]
mod tests;
