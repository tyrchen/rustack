//! Ordered immutable retry/catch metadata and bounded error-name matching.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    num::NonZeroU32,
};

use serde_json::{Map, Value};

use super::{
    CatchResult, Code, CompileError, Location, StateId, fields, object_value, positive, required,
    target, text,
};
use crate::ResultPath;

/// Retry jitter policy; sampling belongs to the execution actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Jitter {
    /// Use the computed backoff delay.
    None,
    /// Uniformly sample zero through the computed cap.
    Full,
}

/// Validated retry timing metadata; no timer or randomness is created here.
#[derive(Debug)]
#[non_exhaustive]
pub struct RetryTiming {
    interval: NonZeroU32,
    rate: f64,
    max_delay: Option<NonZeroU32>,
    jitter: Jitter,
}
impl RetryTiming {
    read!(RetryTiming,interval_seconds,NonZeroU32,s=>s.interval,"Read positive initial delay seconds.");
    read!(RetryTiming,backoff_rate,f64,s=>s.rate,"Read the finite rate, at least one.");
    read!(RetryTiming,max_delay_seconds,Option<NonZeroU32>,s=>s.max_delay,"Read the optional positive delay cap.");
    read!(RetryTiming,jitter,Jitter,s=>s.jitter,"Read the selected jitter strategy.");
}

struct Matcher(Vec<String>);
impl Matcher {
    fn matches(&self, error: &str) -> bool {
        if error.len() > 256 || error == "States.Runtime" {
            return false;
        }
        self.0.iter().any(|name| {
            if name == error {
                return true;
            }
            if error == "States.DataLimitExceeded" {
                return false;
            }
            name == "States.ALL" || (name == "States.TaskFailed" && error != "States.Timeout")
        })
    }
}

/// Compiled retrier, preserving order and zero maximum attempts.
#[non_exhaustive]
pub struct Retrier {
    matcher: Matcher,
    attempts: u32,
    timing: RetryTiming,
}
impl fmt::Debug for Retrier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Retrier(<redacted>)")
    }
}
impl Retrier {
    read!(Retrier,error_names,&[String],s=>s.matcher.0.as_slice(),"Read validated matcher names.");
    read!(Retrier,max_attempts,u32,s=>s.attempts,"Read retries allowed after the first failure.");
    read!(Retrier,timing,&RetryTiming,s=>&s.timing,"Read validated timing metadata.");
    /// Test a bounded failure name using the selected fatal/wildcard policy.
    ///
    /// ```
    /// # fn matches(retrier:&rustack_stepfunctions_core::Retrier) {
    /// let accepted=retrier.matches("ExampleError");
    /// # }
    /// ```
    #[must_use]
    pub fn matches(&self, error: &str) -> bool {
        self.matcher.matches(error)
    }
}

enum Merge {
    Replace,
    Discard,
    Path(ResultPath),
}
/// Compiled catcher with a resolved same-scope target.
#[non_exhaustive]
pub struct Catcher {
    matcher: Matcher,
    next: StateId,
    result: Merge,
}
impl fmt::Debug for Catcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Catcher(<redacted>)")
    }
}
impl Catcher {
    read!(Catcher,error_names,&[String],s=>s.matcher.0.as_slice(),"Read validated matcher names.");
    read!(Catcher,next,StateId,s=>s.next,"Read the resolved continuation.");
    read!(Catcher,result_path,CatchResult<'_>,s=>match &s.result {Merge::Replace=>CatchResult::Replace,Merge::Discard=>CatchResult::Discard,Merge::Path(p)=>CatchResult::Path(p)},"Read validated error-object merge behavior.");
    /// Test a failure name without claiming execution cancellation handling.
    ///
    /// ```
    /// # fn matches(catcher:&rustack_stepfunctions_core::Catcher) {
    /// let accepted=catcher.matches("ExampleError");
    /// # }
    /// ```
    #[must_use]
    pub fn matches(&self, error: &str) -> bool {
        self.matcher.matches(error)
    }
}

/// Ordered, validated task recovery policies.
#[non_exhaustive]
pub struct RecoveryPolicy {
    retries: Vec<Retrier>,
    catches: Vec<Catcher>,
}
impl fmt::Debug for RecoveryPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RecoveryPolicy(<redacted>)")
    }
}
impl RecoveryPolicy {
    read!(RecoveryPolicy,retries,&[Retrier],s=>s.retries.as_slice(),"Read retriers in definition order.");
    read!(RecoveryPolicy,catches,&[Catcher],s=>s.catches.as_slice(),"Read catchers in definition order.");
    pub(super) fn units(&self) -> usize {
        self.retries
            .iter()
            .fold(0_usize, |n, r| {
                n.saturating_add(r.matcher.0.len()).saturating_add(1)
            })
            .saturating_add(self.catches.iter().fold(0_usize, |n, c| {
                n.saturating_add(c.matcher.0.len()).saturating_add(1)
            }))
    }
}

pub(super) fn compile(
    state: &Map<String, Value>,
    location: &Location,
    index: &BTreeMap<String, StateId>,
) -> Result<RecoveryPolicy, CompileError> {
    let retries = entries(state.get("Retry"), &location.field("Retry")?)?
        .iter()
        .enumerate()
        .map(|(i, value)| {
            let pos = location.field("Retry")?.field(&i.to_string())?;
            let object = object_value(value, &pos)?;
            fields(
                object,
                &[
                    "ErrorEquals",
                    "IntervalSeconds",
                    "MaxAttempts",
                    "BackoffRate",
                    "MaxDelaySeconds",
                    "JitterStrategy",
                ],
                &pos,
            )?;
            let matcher = matcher(object, &pos)?;
            let interval = match object.get("IntervalSeconds") {
                Some(v) => positive(v, 99_999_999, &pos.field("IntervalSeconds")?)?,
                None => NonZeroU32::MIN,
            };
            let attempts_pos = pos.field("MaxAttempts")?;
            let attempts = match object.get("MaxAttempts") {
                Some(v) => v
                    .as_u64()
                    .filter(|n| *n <= 99_999_999)
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| {
                        attempts_pos.error(Code::InvalidSchema, "invalid retry count")
                    })?,
                None => 3,
            };
            let rate = match object.get("BackoffRate") {
                Some(v) => v
                    .as_f64()
                    .filter(|n| n.is_finite() && *n >= 1.0)
                    .ok_or_else(|| pos.error(Code::InvalidSchema, "invalid retry backoff"))?,
                None => 2.0,
            };
            let max_delay = object
                .get("MaxDelaySeconds")
                .map(|v| positive(v, 31_622_400, &pos.field("MaxDelaySeconds")?))
                .transpose()?;
            let jitter = match object.get("JitterStrategy").map(Value::as_str) {
                None | Some(Some("NONE")) => Jitter::None,
                Some(Some("FULL")) => Jitter::Full,
                _ => return Err(pos.error(Code::InvalidSchema, "invalid retry jitter")),
            };
            Ok(Retrier {
                matcher,
                attempts,
                timing: RetryTiming {
                    interval,
                    rate,
                    max_delay,
                    jitter,
                },
            })
        })
        .collect::<Result<Vec<_>, CompileError>>()?;
    let catches = entries(state.get("Catch"), &location.field("Catch")?)?
        .iter()
        .enumerate()
        .map(|(i, value)| {
            let pos = location.field("Catch")?.field(&i.to_string())?;
            let object = object_value(value, &pos)?;
            fields(object, &["ErrorEquals", "Next", "ResultPath"], &pos)?;
            let matcher = matcher(object, &pos)?;
            let next = target(required(object, "Next", &pos)?, &pos.field("Next")?, index)?;
            let result = match object.get("ResultPath") {
                None => Merge::Replace,
                Some(Value::Null) => Merge::Discard,
                Some(Value::String(p)) => {
                    Merge::Path(p.parse().map_err(|e| CompileError::asl(&pos, e))?)
                }
                _ => return Err(pos.error(Code::InvalidSchema, "invalid catch result path")),
            };
            Ok(Catcher {
                matcher,
                next,
                result,
            })
        })
        .collect::<Result<Vec<_>, CompileError>>()?;
    last_all(
        retries.iter().map(|r| &r.matcher),
        retries.len(),
        &location.field("Retry")?,
    )?;
    last_all(
        catches.iter().map(|c| &c.matcher),
        catches.len(),
        &location.field("Catch")?,
    )?;
    Ok(RecoveryPolicy { retries, catches })
}

fn entries<'a>(value: Option<&'a Value>, location: &Location) -> Result<&'a [Value], CompileError> {
    match value {
        None => Ok(&[]),
        Some(value) => value
            .as_array()
            .filter(|v| v.len() <= 128)
            .map(Vec::as_slice)
            .ok_or_else(|| location.error(Code::InvalidSchema, "invalid recovery policy array")),
    }
}
fn matcher(object: &Map<String, Value>, location: &Location) -> Result<Matcher, CompileError> {
    let pos = location.field("ErrorEquals")?;
    let names = required(object, "ErrorEquals", location)?
        .as_array()
        .filter(|v| !v.is_empty() && v.len() <= 128)
        .ok_or_else(|| pos.error(Code::InvalidSchema, "invalid error matcher"))?;
    let mut unique = BTreeSet::new();
    let mut result = Vec::with_capacity(names.len());
    for value in names {
        let name = text(value, &pos)?;
        if name.is_empty()
            || name.len() > 256
            || name.chars().any(char::is_control)
            || !unique.insert(name)
        {
            return Err(pos.error(Code::InvalidSchema, "invalid or duplicate error name"));
        }
        result.push(name.to_owned());
    }
    if result.iter().any(|n| n == "States.ALL") && result.len() != 1 {
        return Err(pos.error(Code::InvalidSchema, "States.ALL must be the sole matcher"));
    }
    Ok(Matcher(result))
}
fn last_all<'a>(
    entries: impl Iterator<Item = &'a Matcher>,
    count: usize,
    location: &Location,
) -> Result<(), CompileError> {
    for (position, matcher) in entries.enumerate() {
        if matcher.0.iter().any(|n| n == "States.ALL") && position.saturating_add(1) != count {
            return Err(location.error(Code::InvalidSchema, "States.ALL policy must be last"));
        }
    }
    Ok(())
}
